use mcp_switchboard::jsonrpc::{
    is_modern_request,
    is_tool_call_response,
    rewrite_tool_call_result,
    tool_call_request_id,
};
use serde_json::json;

#[test]
fn test_modern_request_recognized_by_header() {
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(is_modern_request(&message, Some("2025-06-18")));
    assert!(is_modern_request(&message, Some("2026-01-01")));
}

#[test]
fn test_legacy_request_recognized_by_missing_header() {
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(!is_modern_request(&message, None));
}

#[test]
fn test_legacy_header_value_is_legacy() {
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    assert!(!is_modern_request(&message, Some("2024-11-05")));
}

#[test]
fn test_initialize_request_falls_back_to_body_protocol_version() {
    let modern_init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}});
    let legacy_init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}});
    assert!(is_modern_request(&modern_init, None));
    assert!(!is_modern_request(&legacy_init, None));
}

#[test]
fn test_tool_call_request_id_matches_only_tools_call() {
    assert_eq!(tool_call_request_id(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call"})), Some(json!(7)));
    assert_eq!(tool_call_request_id(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list"})), None);
    assert_eq!(tool_call_request_id(&json!({"jsonrpc": "2.0", "method": "notifications/progress"})), None);
}

#[test]
fn test_is_tool_call_response_matches_by_id_and_shape() {
    assert!(is_tool_call_response(&json!({"jsonrpc": "2.0", "id": 7, "result": {}}), &json!(7)));
    assert!(is_tool_call_response(&json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -1}}), &json!(7)));
    assert!(!is_tool_call_response(&json!({"jsonrpc": "2.0", "id": 8, "result": {}}), &json!(7)));
    // a request/notification (has "method") is never a response, even with a matching id
    assert!(!is_tool_call_response(&json!({"jsonrpc": "2.0", "id": 7, "method": "sampling/createMessage"}), &json!(7)));
}

#[test]
fn test_rewrite_tool_call_result_rewrites_matching_text_block() {
    let mut message = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {
            "content": [{"type": "text", "text": "[{\"a\":1},{\"a\":2}]"}],
        },
    });
    let changed = rewrite_tool_call_result(&mut message, |text| {
        if text.contains("\"a\"") { Some("TABLE".to_string()) } else { None }
    });
    assert!(changed);
    assert_eq!(message["result"]["content"][0]["text"], "TABLE");
}

#[test]
fn test_rewrite_tool_call_result_leaves_non_table_text_untouched() {
    let mut message = json!({"jsonrpc": "2.0", "id": 7, "result": {"content": [{"type": "text", "text": "plain"}]}});
    let changed = rewrite_tool_call_result(&mut message, |_text| None);
    assert!(!changed);
    assert_eq!(message["result"]["content"][0]["text"], "plain");
}

#[test]
fn test_rewrite_tool_call_result_ignores_non_text_blocks() {
    let mut message = json!({"jsonrpc": "2.0", "id": 7, "result": {"content": [{"type": "image", "data": "..."}]}});
    let changed = rewrite_tool_call_result(&mut message, |_text| Some("TABLE".to_string()));
    assert!(!changed);
}

#[test]
fn test_rewrite_tool_call_result_ignores_error_response() {
    let mut message = json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -1, "message": "boom"}});
    let changed = rewrite_tool_call_result(&mut message, |_text| Some("TABLE".to_string()));
    assert!(!changed);
}

#[test]
fn test_rewrite_drops_structured_content_when_text_is_rewritten() {
    let mut message = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {
            "content": [{"type": "text", "text": "[{\"a\":1}]"}],
            "structuredContent": {"a": [{"a": 1}]},
        },
    });
    let changed = rewrite_tool_call_result(&mut message, |_text| Some("TABLE".to_string()));
    assert!(changed);
    assert!(message["result"].get("structuredContent").is_none());
    assert_eq!(message["result"]["content"][0]["text"], "TABLE");
}

#[test]
fn test_rewrite_keeps_structured_content_when_text_is_not_rewritten() {
    let mut message = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {
            "content": [{"type": "text", "text": "plain"}],
            "structuredContent": {"a": 1},
        },
    });
    let changed = rewrite_tool_call_result(&mut message, |_text| None);
    assert!(!changed);
    assert_eq!(message["result"]["structuredContent"], json!({"a": 1}));
}
