use mcp_switchboard::config::{AppConfig, RewriteMode};
use tempfile::NamedTempFile;
use std::io::Write;
use std::env;

fn write_config(text: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", text).unwrap();
    file
}

#[test]
fn test_loads_multiple_named_backends() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    description: Jira issue tracker
  expert:
    url: https://expert.example.com/mcp
    rewrite: md_tables
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();

    assert_eq!(config.servers.len(), 2);

    let jira = config.servers.get("jira").unwrap();
    assert_eq!(jira.name, "jira");
    assert_eq!(jira.url, "https://jira.example.com/mcp");
    assert_eq!(jira.description.as_deref(), Some("Jira issue tracker"));
    assert_eq!(jira.tool_prefix.as_deref(), Some("jira"));

    let expert = config.servers.get("expert").unwrap();
    assert_eq!(expert.name, "expert");
    assert_eq!(expert.url, "https://expert.example.com/mcp");
    assert_eq!(expert.rewrite, Some(RewriteMode::MdTables));
    assert_eq!(expert.tool_prefix.as_deref(), Some("expert"));
}

#[test]
fn test_rewrite_defaults_to_none() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().rewrite.is_none());
}

#[test]
fn test_loads_rewrite_md_tables() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    rewrite: md_tables\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().rewrite, Some(RewriteMode::MdTables));
}

#[test]
fn test_loads_rewrite_toon() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    rewrite: toon\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().rewrite, Some(RewriteMode::Toon));
}

#[test]
fn test_forward_origin_defaults_to_false() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(!config.servers.get("jira").unwrap().forward_origin);
}

#[test]
fn test_loads_forward_origin() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    forward_origin: true\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().forward_origin);
}

#[test]
fn test_use_tool_search_defaults_to_false() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(!config.servers.get("jira").unwrap().use_tool_search);
}

#[test]
fn test_loads_use_tool_search() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    use_tool_search: true\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().use_tool_search);
}

#[test]
fn test_tool_prefix_defaults_to_backend_name() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    use_tool_search: true\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().tool_prefix.as_deref(), Some("jira"));
}

#[test]
fn test_loads_explicit_tool_prefix() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    use_tool_search: true
    tool_prefix: j
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().tool_prefix.as_deref(), Some("j"));
}

#[test]
fn test_rejects_empty_tool_prefix() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    tool_prefix: ''\n");
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty 'tool_prefix'"));
}

#[test]
fn test_rejects_duplicate_tool_prefix_across_tool_search_backends() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    use_tool_search: true
    tool_prefix: shared
  expert:
    url: https://expert.example.com/mcp
    use_tool_search: true
    tool_prefix: shared
"#);
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Duplicate tool_prefix 'shared'"));
}

#[test]
fn test_allows_duplicate_name_derived_prefix_when_tool_search_disabled() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
  expert:
    url: https://expert.example.com/mcp
    use_tool_search: true
    tool_prefix: jira
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("expert").unwrap().tool_prefix.as_deref(), Some("jira"));
}

#[test]
fn test_headers_defaults_to_empty() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().headers.is_empty());
}

#[test]
fn test_loads_headers() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    headers:
      Authorization: Bearer secret
      X-Api-Key: abc123
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let headers = &config.servers.get("jira").unwrap().headers;
    assert_eq!(headers.get("Authorization").unwrap(), "Bearer secret");
    assert_eq!(headers.get("X-Api-Key").unwrap(), "abc123");
}

#[test]
fn test_substitutes_env_var_in_header_value() {
    env::set_var("JIRA_TOKEN", "secret-token");
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    headers:
      Authorization: Bearer ${JIRA_TOKEN}
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().headers.get("Authorization").unwrap(), "Bearer secret-token");
}

#[test]
fn test_substitutes_multiple_env_vars_in_one_header_value() {
    env::set_var("SCHEME", "Bearer");
    env::set_var("TOKEN", "abc123");
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    headers:
      Authorization: ${SCHEME} ${TOKEN}
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert_eq!(config.servers.get("jira").unwrap().headers.get("Authorization").unwrap(), "Bearer abc123");
}

#[test]
fn test_rejects_undefined_env_var_in_header_value() {
    env::remove_var("MISSING_TOKEN");
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    headers:
      Authorization: Bearer ${MISSING_TOKEN}
"#);
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("MISSING_TOKEN"));
}

#[test]
fn test_log_level_defaults_to_none() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().log_level.is_none());
}

#[test]
fn test_loads_log_level() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    log_level: DEBUG\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let backend = config.servers.get("jira").unwrap();
    assert_eq!(backend.log_level.as_deref(), Some("debug"));
    assert_eq!(backend.resolved_log_level, Some(tracing::Level::DEBUG));
}

#[test]
fn test_loads_log_level_trace() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    log_level: trace\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let backend = config.servers.get("jira").unwrap();
    assert_eq!(backend.log_level.as_deref(), Some("trace"));
    assert_eq!(backend.resolved_log_level, Some(tracing::Level::TRACE));
}

#[test]
fn test_rejects_invalid_log_level() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n    log_level: verbose\n");
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid log_level"));
}

#[test]
fn test_log_headers_defaults_to_empty() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().log_headers.is_empty());
}

#[test]
fn test_loads_log_headers_lowercased() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    log_headers:
      - X-Request-Id
      - X-Trace-Id
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let log_headers = &config.servers.get("jira").unwrap().log_headers;
    assert!(log_headers.contains(&"x-request-id".to_string()));
    assert!(log_headers.contains(&"x-trace-id".to_string()));
}

#[test]
fn test_tool_filter_defaults_to_none() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.servers.get("jira").unwrap().tools.is_none());
}

#[test]
fn test_loads_tool_filter_whitelist_and_blacklist() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    tools:
      whitelist:
        - get_*
      blacklist:
        - get_secrets
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let filter = config.servers.get("jira").unwrap().tools.as_ref().unwrap();
    assert_eq!(filter.whitelist, vec!["get_*".to_string()]);
    assert_eq!(filter.blacklist, vec!["get_secrets".to_string()]);
}

#[test]
fn test_rejects_empty_string_in_tools_blacklist() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
    tools:
      blacklist:
        - ''
"#);
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty string in tools blacklist"));
}

#[test]
fn test_cors_allow_origins_defaults_to_empty() {
    let file = write_config("servers:\n  jira:\n    url: https://jira.example.com/mcp\n");
    let config = AppConfig::load_from_file(file.path()).unwrap();
    assert!(config.cors.is_none());
}

#[test]
fn test_loads_cors_allow_origins() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
cors:
  allow_origins:
    - https://a.example.com
    - https://b.example.com
"#);
    let config = AppConfig::load_from_file(file.path()).unwrap();
    let cors = config.cors.as_ref().unwrap();
    assert_eq!(cors.allow_origins, vec!["https://a.example.com", "https://b.example.com"]);
}

#[test]
fn test_rejects_empty_allow_origins_list() {
    let file = write_config(r#"
servers:
  jira:
    url: https://jira.example.com/mcp
cors:
  allow_origins: []
"#);
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("allow_origins' must not be empty"));
}

#[test]
fn test_rejects_empty_servers() {
    let file = write_config("servers: {}\n");
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one server"));
}

#[test]
fn test_rejects_entry_without_url() {
    let file = write_config("servers:\n  jira:\n    description: no url here\n");
    // serde_yaml might fail during deserialization if URL is missing and not optional
    let result = AppConfig::load_from_file(file.path());
    assert!(result.is_err());
}
