//! This module implements the "Tool Search" mechanism, which is a key part of the Switchboard
//! architecture. Instead of exposing all backend tools directly to the LLM (which could
//! lead to context window exhaustion if there are hundreds of tools), Switchboard
//! exposes two meta-tools: `describe_tools` and `call_tool`.
//!
//! Tools from different backends are namespaced using a prefix (e.g., `my_backend::tool_name`)
//! to prevent name collisions. This module handles the prefixing, filtering, and rewriting
//! logic required to route these meta-calls to their actual backend implementations.

use serde_json::{json, Value};
use crate::config::ToolsFilter;
use crate::tool_filter::is_tool_allowed;

/// The name of the meta-tool used by the LLM to invoke a specific tool.
pub const CALL_TOOL_NAME: &str = "call_tool";
/// The name of the meta-tool used by the LLM to inspect tool definitions.
pub const DESCRIBE_TOOLS_NAME: &str = "describe_tools";
/// The character sequence used to namespace tool names by their backend prefix.
const PREFIX_SEPARATOR: &str = "::";

/// Combines a backend prefix with a tool name to create a unique, namespaced identifier.
pub fn prefixed_tool_name(prefix: &str, name: &str) -> String {
    format!("{}{}{}", prefix, PREFIX_SEPARATOR, name)
}

/// Attempts to remove the backend prefix from a namespaced tool name.
/// Returns the raw tool name if the prefix matches, otherwise None.
pub fn strip_prefix<'a>(prefix: &str, name: &'a str) -> Option<&'a str> {
    let marker = format!("{}{}", prefix, PREFIX_SEPARATOR);
    if name.starts_with(&marker) && name.len() > marker.len() {
        Some(&name[marker.len()..])
    } else {
        None
    }
}

/// Helper to get the filtered and prefixed list of tool names.
pub fn get_exposed_tool_names(backend_tools: &[Value], prefix: &str, filter: Option<&ToolsFilter>) -> Vec<String> {
    let mut names: Vec<String> = backend_tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .filter(|&name| is_tool_allowed(name, filter))
        .map(|name| prefixed_tool_name(prefix, name))
        .collect();
    names.sort();
    names
}

/// Constructs the list of tools that Switchboard will expose to the LLM for a given backend.
/// Instead of listing every tool from the backend, it returns the two meta-tools:
/// `describe_tools` and `call_tool`. The description of `describe_tools` contains
/// the list of available (and filtered) tool names from that backend.
pub fn build_tools_list_result(backend_tools: &[Value], prefix: &str, description: Option<&str>, filter: Option<&ToolsFilter>) -> Value {
    let prefixed_names = get_exposed_tool_names(backend_tools, prefix, filter);

    let names_line = if prefixed_names.is_empty() {
        "(no tools available)".to_string()
    } else {
        prefixed_names.join(", ")
    };

    let backend_summary = description.map(|d| format!("{}\n\n", d)).unwrap_or_default();

    json!({
        "tools": [
            {
                "name": DESCRIBE_TOOLS_NAME,
                "description": format!(
                    "{}Look up the description and input schema of one or more tools, before calling them with `{}`. Available tools: {}",
                    backend_summary, CALL_TOOL_NAME, names_line
                ),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "names": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Names of the tools to look up (including their prefix)."
                        }
                    },
                    "required": ["names"]
                }
            },
            {
                "name": CALL_TOOL_NAME,
                "description": format!(
                    "Call a tool by name. Look up its input schema with `{}` first if you haven't already done so in this conversation.",
                    DESCRIBE_TOOLS_NAME
                ),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "The tool to call (including its prefix)."},
                        "arguments": {"type": "object", "description": "Arguments to call it with."}
                    },
                    "required": ["name"]
                }
            }
        ]
    })
}

/// In-place modifies a `call_tool` request from the LLM into a direct tool call
/// that the target backend can process.
///
/// If the request is for `call_tool`, it extracts the actual tool name and arguments
/// from the `arguments` field, strips the prefix, and updates the request object
/// to look like a standard MCP `tools/call` request for that specific tool.
pub fn rewrite_call_tool_request(req_json: &mut Value, prefix: &str) -> Option<String> {
    let params = req_json.get_mut("params")?;
    let name = params.get("name").and_then(|v| v.as_str())?;

    // describe_tools is handled internally, not forwarded to the backend.
    if name == DESCRIBE_TOOLS_NAME {
        return None;
    }

    if name == CALL_TOOL_NAME {
        let (real_name_prefixed, real_args) = {
            let arguments = params.get("arguments")?;
            let real_name_prefixed = arguments.get("name").and_then(|v| v.as_str())?.to_string();
            let real_args = arguments.get("arguments").cloned().unwrap_or(json!({}));
            (real_name_prefixed, real_args)
        };

        let real_name = strip_prefix(prefix, &real_name_prefixed)?;

        if let Some(p_obj) = params.as_object_mut() {
            // Transform the meta-call into a direct call by replacing the method parameters.
            p_obj.insert("name".to_string(), Value::String(real_name.to_string()));
            p_obj.insert("arguments".to_string(), real_args);
        }
        return Some(real_name.to_string());
    }

    None
}

/// Builds the response for a `describe_tools` call.
/// It searches the `backend_tools` for the requested names, strips their prefixes
/// for the lookup, but restores the prefixed names in the returned schema so the
/// LLM maintains a consistent view of the namespaced tools.
pub fn build_describe_tools_result(
    backend_tools: &[Value],
    requested_names: &[String],
    prefix: &str,
    filter: Option<&ToolsFilter>,
) -> Value {
    let mut described = Vec::new();
    let mut errors = Vec::new();

    for req_name in requested_names {
        if let Some(real_name) = strip_prefix(prefix, req_name) {
            if is_tool_allowed(real_name, filter) {
                if let Some(tool) = backend_tools
                    .iter()
                    .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(real_name))
                {
                    let mut modified = tool.clone();
                    if let Some(obj) = modified.as_object_mut() {
                        // Put the prefixed name back so the LLM knows how to call it via call_tool.
                        obj.insert("name".to_string(), Value::String(req_name.clone()));
                    }
                    described.push(modified);
                    continue;
                }
            }
        }
        errors.push(format!("unknown tool: {}", req_name));
    }

    let mut content = Vec::new();
    for d in described {
        content.push(
            json!({"type": "text", "text": serde_json::to_string(&d).unwrap_or_default()}),
        );
    }
    for e in &errors {
        content.push(json!({"type": "text", "text": e}));
    }

    json!({
        "content": content,
        "isError": !errors.is_empty()
    })
}

/// Helper to extract the list of tools to describe from a `tools/call` message.
pub fn describe_tools_call_names(message: &Value) -> Option<Vec<String>> {
    if message.get("method").and_then(|v| v.as_str()) == Some("tools/call") {
        let params = message.get("params")?;
        if params.get("name").and_then(|v| v.as_str()) == Some(DESCRIBE_TOOLS_NAME) {
            return params
                .get("arguments")
                .and_then(|a| a.get("names"))
                .and_then(|n| n.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
        }
    }
    None
}

/// Helper to identify which tool is being targeted in a `call_tool` request.
pub fn call_tool_target_name(message: &Value, prefix: &str) -> Option<String> {
    if message.get("method").and_then(|v| v.as_str()) == Some("tools/call") {
        let params = message.get("params")?;
        if params.get("name").and_then(|v| v.as_str()) == Some(CALL_TOOL_NAME) {
            let prefixed_name = params.get("arguments")?.get("name")?.as_str()?;
            return strip_prefix(prefix, prefixed_name).map(String::from);
        }
    }
    None
}
