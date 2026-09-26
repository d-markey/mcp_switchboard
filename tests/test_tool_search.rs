use mcp_switchboard::tool_search::{
    build_describe_tools_result, build_tools_list_result, call_tool_target_name,
    describe_tools_call_names, rewrite_call_tool_request, CALL_TOOL_NAME, DESCRIBE_TOOLS_NAME,
};
use mcp_switchboard::jsonrpc::jsonrpc_request;
use serde_json::{json, Value};

fn backend_tools() -> Vec<Value> {
    vec![
        json!({"name": "get_table", "description": "returns a table", "inputSchema": {"type": "object"}}),
        json!({"name": "get_scalar", "description": "returns plain json", "inputSchema": {"type": "object"}}),
    ]
}

#[test]
fn test_tools_list_result_exposes_only_the_two_proxy_tools() {
    let result = build_tools_list_result(&backend_tools(), "jira", None, None);
    let tools = result["tools"].as_array().unwrap();
    let names: std::collections::HashSet<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(DESCRIBE_TOOLS_NAME));
    assert!(names.contains(CALL_TOOL_NAME));
}

#[test]
fn test_tools_list_result_embeds_prefixed_backend_tool_names_once_in_describe_tools_description() {
    let result = build_tools_list_result(&backend_tools(), "jira", None, None);
    let tools = result["tools"].as_array().unwrap();
    let describe_tool = tools.iter().find(|t| t["name"] == DESCRIBE_TOOLS_NAME).unwrap();
    let description = describe_tool["description"].as_str().unwrap();
    assert!(description.contains("jira::get_table"));
    assert!(description.contains("jira::get_scalar"));

    let call_tool = tools.iter().find(|t| t["name"] == CALL_TOOL_NAME).unwrap();
    let call_desc = call_tool["description"].as_str().unwrap();
    assert!(!call_desc.contains("get_table"));
    assert!(!call_desc.contains("get_scalar"));
}

#[test]
fn test_tools_list_result_prepends_backend_description_to_describe_tools() {
    let result = build_tools_list_result(&backend_tools(), "jira", Some("Jira issue tracker"), None);
    let tools = result["tools"].as_array().unwrap();
    let describe_tool = tools.iter().find(|t| t["name"] == DESCRIBE_TOOLS_NAME).unwrap();
    let description = describe_tool["description"].as_str().unwrap();
    assert!(description.starts_with("Jira issue tracker\n\n"));
}

#[test]
fn test_tools_list_result_omits_backend_description_when_none() {
    let result = build_tools_list_result(&backend_tools(), "jira", None, None);
    let tools = result["tools"].as_array().unwrap();
    let describe_tool = tools.iter().find(|t| t["name"] == DESCRIBE_TOOLS_NAME).unwrap();
    let description = describe_tool["description"].as_str().unwrap();
    assert!(description.starts_with("Look up the description"));
}

#[test]
fn test_describe_tools_result_returns_schema_for_known_prefixed_name() {
    let result = build_describe_tools_result(&backend_tools(), &["jira::get_table".to_string()], "jira", None);
    assert_eq!(result["isError"], false);
    let described: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(described["name"], "jira::get_table");
    assert_eq!(described["description"], "returns a table");
}

#[test]
fn test_describe_tools_result_flags_unknown_name() {
    let result = build_describe_tools_result(&backend_tools(), &["jira::get_table".to_string(), "jira::not_a_tool".to_string()], "jira", None);
    assert_eq!(result["isError"], true);
    let texts: Vec<_> = result["content"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap()).collect();
    assert!(texts.contains(&"unknown tool: jira::not_a_tool"));
    let described: Value = serde_json::from_str(texts[0]).unwrap();
    assert_eq!(described["name"], "jira::get_table");
}

#[test]
fn test_describe_tools_result_flags_wrong_prefix_as_unknown() {
    let result = build_describe_tools_result(&backend_tools(), &["expert::get_table".to_string(), "get_table".to_string()], "jira", None);
    assert_eq!(result["isError"], true);
    let texts: Vec<_> = result["content"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap()).collect();
    assert_eq!(texts, vec!["unknown tool: expert::get_table", "unknown tool: get_table"]);
}

#[test]
fn test_describe_tools_result_filters_blacklisted_tool() {
    let filter = mcp_switchboard::config::ToolsFilter {
        whitelist: vec![],
        blacklist: vec!["get_table".to_string()],
    };
    let result = build_describe_tools_result(&backend_tools(), &["jira::get_table".to_string()], "jira", Some(&filter));
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("unknown tool: jira::get_table"));
}

#[test]
fn test_describe_tools_call_names_extracts_names() {
    let message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": DESCRIBE_TOOLS_NAME, "arguments": {"names": ["jira::get_table"]}}))
    );
    assert_eq!(describe_tools_call_names(&message), Some(vec!["jira::get_table".to_string()]));
}

#[test]
fn test_describe_tools_call_names_ignores_other_tools() {
    let message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {}}))
    );
    assert_eq!(describe_tools_call_names(&message), None);
}

#[test]
fn test_rewrite_call_tool_request_substitutes_real_name_and_arguments() {
    let mut message = jsonrpc_request(
        Some(&json!(42)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "jira::get_table", "arguments": {"x": 1}}}))
    );
    let real_name = rewrite_call_tool_request(&mut message, "jira");
    assert_eq!(real_name, Some("get_table".to_string()));
    assert_eq!(message["params"]["name"], "get_table");
    assert_eq!(message["params"]["arguments"]["x"], 1);
}

#[test]
fn test_rewrite_call_tool_request_defaults_missing_arguments_to_empty_dict() {
    let mut message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "jira::get_scalar"}}))
    );
    rewrite_call_tool_request(&mut message, "jira");
    assert_eq!(message["params"]["arguments"], json!({}));
}

#[test]
fn test_rewrite_call_tool_request_ignores_other_tools() {
    let mut message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": DESCRIBE_TOOLS_NAME, "arguments": {"names": []}}))
    );
    assert_eq!(rewrite_call_tool_request(&mut message, "jira"), None);
}

#[test]
fn test_rewrite_call_tool_request_rejects_name_with_wrong_prefix() {
    let mut message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "expert::get_table", "arguments": {}}}))
    );
    assert_eq!(rewrite_call_tool_request(&mut message, "jira"), None);
}

#[test]
fn test_call_tool_target_name_returns_unprefixed_name() {
    let message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "jira::get_table", "arguments": {}}}))
    );
    assert_eq!(call_tool_target_name(&message, "jira"), Some("get_table".to_string()));
}

#[test]
fn test_call_tool_target_name_rejects_wrong_prefix() {
    let message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "expert::get_table", "arguments": {}}}))
    );
    assert_eq!(call_tool_target_name(&message, "jira"), None);
}

#[test]
fn test_call_tool_target_name_ignores_other_tools() {
    let message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": DESCRIBE_TOOLS_NAME, "arguments": {}}))
    );
    assert_eq!(call_tool_target_name(&message, "jira"), None);
}

#[test]
fn test_rewrite_call_tool_request_rejects_unprefixed_name() {
    let mut message = jsonrpc_request(
        Some(&json!(1)),
        "tools/call",
        Some(json!({"name": CALL_TOOL_NAME, "arguments": {"name": "get_table", "arguments": {}}}))
    );
    assert_eq!(rewrite_call_tool_request(&mut message, "jira"), None);
}
