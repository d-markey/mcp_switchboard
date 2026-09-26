//! Configuration management for the Switchboard application.
//! It defines structures for mapping servers, handling CORS policies, and specifies rules
//! for rewriting data payloads or filtering out tools. It also handles environmental variable
//! substitution so that sensitive credentials or dynamic hosts don't need to be hardcoded in YAML files.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;
use regex::Regex;
use tracing::Level;

/// Defines explicit tool whitelists or blacklists for a backend server.
/// This gives fine-grained security or context control over which tools are visible to the LLM.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct ToolsFilter {
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub blacklist: Vec<String>,
}

/// Supported custom rewrite rules for transforming response payloads before they reach the LLM client.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewriteMode {
    /// Compresses Markdown tables or formats data explicitly to save precious tokens.
    MdTables,
    /// Custom token conservation mode or target format transformation.
    Toon,
    /// Compresses tabular data into CSV format with tab separators.
    Csv,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RawBackendConfig {
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub rewrite: Option<RewriteMode>,
    #[serde(default, rename = "use_tool_search")]
    pub use_tool_search: bool,
    #[serde(default)]
    pub tool_prefix: Option<String>,
    #[serde(default, rename = "forward_origin")]
    pub forward_origin: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub tools: Option<ToolsFilter>,
    #[serde(default, rename = "log_level")]
    pub log_level: Option<String>,
    #[serde(default, rename = "log_headers")]
    pub log_headers: Vec<String>,
}

/// Holds proxy, authentication, filtering, and rewriting details for a single upstream MCP backend.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BackendConfig {
    /// Placed dynamically from the HashMap key for ease of identification in log traces.
    #[serde(skip)]
    pub name: String,
    /// The real destination URL where MCP requests should be forwarded.
    pub url: String,
    /// An optional custom overview displayed inside the `describe_tools` description text.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional payload rewrite mode to save context or token size.
    #[serde(default)]
    pub rewrite: Option<RewriteMode>,
    /// Enables the token-saving Two-Step Tool Search optimization (`describe_tools` + `call_tool`).
    #[serde(default, rename = "use_tool_search")]
    pub use_tool_search: bool,
    /// Prefix to namespace all tools from this backend. Defaults to backend's name if omitted.
    #[serde(default, rename = "tool_prefix")]
    pub tool_prefix: String,
    /// Controls whether the incoming `origin` HTTP header is forwarded to prevent CORS issues upstream.
    #[serde(default, rename = "forward_origin")]
    pub forward_origin: bool,
    /// Custom static HTTP headers (e.g. `Authorization: Bearer ...`) injected on downstream requests.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Rules determining which subset of tools from this backend are allowed to be seen or called.
    #[serde(default)]
    pub tools: Option<ToolsFilter>,
    /// Restricts or standardizes logging levels for requests routed to this specific server.
    #[serde(default, rename = "log_level")]
    pub log_level: Option<String>,
    /// Specific header keys whose values should be outputted into logging/tracing channels.
    #[serde(default, rename = "log_headers")]
    pub log_headers: Vec<String>,
    /// Resolved tracing level for this backend.
    #[serde(skip)]
    pub resolved_log_level: Option<Level>,
}

/// Global Cross-Origin Resource Sharing configuration rules.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct CorsConfig {
    #[serde(default, rename = "allow_origins")]
    pub allow_origins: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RawAppConfig {
    #[serde(default)]
    pub servers: HashMap<String, RawBackendConfig>,
    #[serde(default)]
    pub cors: Option<CorsConfig>,
}

/// The main application configuration model, mirroring the YAML file schema structure.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub servers: HashMap<String, BackendConfig>,
    #[serde(default)]
    pub cors: Option<CorsConfig>,
}

impl AppConfig {
    /// Loads a YAML configuration file, sanitizes empty strings, expands environment variables,
    /// and validates configuration invariants (like avoiding duplicate prefixes or missing URLs).
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let raw_config: RawAppConfig = serde_yaml::from_str(&content)?;

        // A gateway without any backends is a user misconfiguration; we stop early.
        if raw_config.servers.is_empty() {
            return Err("Config must contain at least one server in 'servers'".into());
        }

        // Regex to find variable interpolation markers like `${MY_ENV_VAR}`.
        let env_regex = Regex::new(r"\$\{([^}]+)\}")?;
        let mut used_prefixes = HashSet::new();
        let mut servers = HashMap::new();

        let expand_env_vars = |val: &str| -> Result<String, Box<dyn std::error::Error>> {
            let mut new_val = val.to_string();
            for cap in env_regex.captures_iter(val) {
                let var_name = &cap[1];
                let env_val = env::var(var_name).map_err(|_| format!("Undefined environment variable: {}", var_name))?;
                new_val = new_val.replace(&cap[0], &env_val);
            }
            Ok(new_val)
        };

        for (name, raw_backend) in raw_config.servers {
            // Substitute dynamic environment variables inside the backend URL.
            let url = expand_env_vars(&raw_backend.url)?;

            // Fail early if target address is missing to avoid routing requests to nowhere.
            if url.is_empty() {
                return Err(format!("Backend '{}' missing 'url'", name).into());
            }

            let tool_prefix = match raw_backend.tool_prefix {
                Some(p) => {
                    if p.trim().is_empty() {
                        return Err(format!("Backend '{}' has empty 'tool_prefix'", name).into());
                    }
                    p
                }
                None => name.clone(),
            };

            // If two distinct backends use the same tool_prefix with tool_search enabled,
            // routing a tool call would become ambiguous or impossible.
            if raw_backend.use_tool_search && !used_prefixes.insert(tool_prefix.clone()) {
                return Err(format!("Duplicate tool_prefix '{}' across tool_search backends", tool_prefix).into());
            }

            let mut headers = raw_backend.headers;
            // Substitute dynamic environment variables inside the header values.
            for value in headers.values_mut() {
                *value = expand_env_vars(value)?;
            }

            let mut resolved_log_level = None;
            let mut log_level = raw_backend.log_level;
            if let Some(level) = &log_level {
                let lower = level.to_lowercase();
                match lower.as_str() {
                    "trace" => {
                        log_level = Some(lower);
                        resolved_log_level = Some(Level::TRACE);
                    }
                    "debug" => {
                        log_level = Some(lower);
                        resolved_log_level = Some(Level::DEBUG);
                    }
                    "info" => {
                        log_level = Some(lower);
                        resolved_log_level = Some(Level::INFO);
                    }
                    "warn" => {
                        log_level = Some(lower);
                        resolved_log_level = Some(Level::WARN);
                    }
                    "error" => {
                        log_level = Some(lower);
                        resolved_log_level = Some(Level::ERROR);
                    }
                    _ => return Err(format!("Backend '{}' has invalid log_level: {}", name, level).into()),
                }
            }

            let mut log_headers = raw_backend.log_headers;
            for h in log_headers.iter_mut() {
                *h = h.to_lowercase();
            }

            if let Some(filter) = &raw_backend.tools {
                for b in &filter.blacklist {
                    if b.is_empty() {
                         return Err(format!("Backend '{}' has empty string in tools blacklist", name).into());
                    }
                }
            }

            servers.insert(
                name.clone(),
                BackendConfig {
                    name,
                    url,
                    description: raw_backend.description,
                    rewrite: raw_backend.rewrite,
                    use_tool_search: raw_backend.use_tool_search,
                    tool_prefix,
                    forward_origin: raw_backend.forward_origin,
                    headers,
                    tools: raw_backend.tools,
                    log_level,
                    log_headers,
                    resolved_log_level,
                },
            );
        }

        // Validate that CORS, if enabled, specifies actual origins rather than being empty.
        if let Some(cors) = &raw_config.cors {
            if cors.allow_origins.is_empty() {
                return Err("CORS 'allow_origins' must not be empty if 'cors' key is present".into());
            }
        }

        Ok(AppConfig {
            servers,
            cors: raw_config.cors,
        })
    }
}
