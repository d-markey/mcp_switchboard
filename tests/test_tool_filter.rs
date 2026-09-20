use mcp_switchboard::config::ToolsFilter;
use mcp_switchboard::tool_filter::{is_tool_allowed, filter_tools};
use serde_json::json;

#[test]
fn test_default_filter_allows_everything() {
    assert!(is_tool_allowed("anything", None));
}

#[test]
fn test_whitelist_allows_only_matching_names() {
    let filter = ToolsFilter {
        whitelist: vec!["get_*".to_string()],
        blacklist: vec![],
    };
    assert!(is_tool_allowed("get_status", Some(&filter)));
    assert!(!is_tool_allowed("delete_project", Some(&filter)));
}

#[test]
fn test_blacklist_excludes_matching_names() {
    let filter = ToolsFilter {
        whitelist: vec![],
        blacklist: vec!["delete_*".to_string()],
    };
    assert!(is_tool_allowed("get_status", Some(&filter)));
    assert!(!is_tool_allowed("delete_project", Some(&filter)));
}

#[test]
fn test_blacklist_takes_precedence_over_whitelist() {
    let filter = ToolsFilter {
        whitelist: vec!["get_*".to_string()],
        blacklist: vec!["get_secrets".to_string()],
    };
    assert!(is_tool_allowed("get_status", Some(&filter)));
    assert!(!is_tool_allowed("get_secrets", Some(&filter)));
}

#[test]
fn test_filter_tools_drops_disallowed_entries() {
    let filter = ToolsFilter {
        whitelist: vec!["get_*".to_string()],
        blacklist: vec![],
    };
    let tools = vec![
        json!({"name": "get_status"}),
        json!({"name": "delete_project"}),
        json!({"name": "get_scalar"}),
    ];
    let filtered = filter_tools(tools, Some(&filter));
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0]["name"], "get_status");
    assert_eq!(filtered[1]["name"], "get_scalar");
}

#[test]
fn test_filter_tools_keeps_malformed_entries() {
    let filter = ToolsFilter {
        whitelist: vec!["get_*".to_string()],
        blacklist: vec![],
    };
    let malformed = vec![
        json!({"no_name": "here"}),
        json!("not-a-dict"),
    ];
    let filtered = filter_tools(malformed.clone(), Some(&filter));
    assert_eq!(filtered, malformed);
}
