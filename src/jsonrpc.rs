//! JSON-RPC 2.0 message parsing and inspection helpers.

use serde_json::Value;

/// Known MCP methods
#[derive(Debug, PartialEq, Eq)]
pub enum McpMethod {
    Initialize,
    ToolsList,
    ToolsCall,
    Unknown(String),
}

impl McpMethod {
    pub fn from_str(s: &str) -> Self {
        match s {
            "initialize" => McpMethod::Initialize,
            "tools/list" => McpMethod::ToolsList,
            "tools/call" => McpMethod::ToolsCall,
            _ => McpMethod::Unknown(s.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            McpMethod::Initialize => "initialize",
            McpMethod::ToolsList => "tools/list",
            McpMethod::ToolsCall => "tools/call",
            McpMethod::Unknown(s) => s,
        }
    }
}

/// Helper to get method from a JSON-RPC value
pub fn get_method(val: &Value) -> Option<McpMethod> {
    val.get("method").and_then(|v| v.as_str()).map(McpMethod::from_str)
}

/// Helper to get params from a JSON-RPC value
pub fn get_params(val: &Value) -> Option<&Value> {
    val.get("params")
}

/// Helper to get arguments from params
pub fn get_arguments(params: &Value) -> Option<&Value> {
    params.get("arguments")
}

pub fn jsonrpc_request(id: Option<&Value>, method: &str, params: Option<Value>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), serde_json::Value::String("2.0".to_string()));
    if let Some(id) = id {
        if !id.is_null() {
            obj.insert("id".to_string(), id.clone());
        }
    }
    obj.insert("method".to_string(), serde_json::Value::String(method.to_string()));
    if let Some(params) = params {
        obj.insert("params".to_string(), params);
    }
    Value::Object(obj)
}

pub fn jsonrpc_response(id: Option<&Value>, result: Value) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), serde_json::Value::String("2.0".to_string()));
    if let Some(id) = id {
        if !id.is_null() {
            obj.insert("id".to_string(), id.clone());
        }
    }
    obj.insert("result".to_string(), result);
    Value::Object(obj)
}

pub fn jsonrpc_error(id: Option<&Value>, code: i64, message: &str) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), serde_json::Value::String("2.0".to_string()));
    if let Some(id) = id {
        if !id.is_null() {
            obj.insert("id".to_string(), id.clone());
        }
    }
    let mut error = serde_json::Map::new();
    error.insert("code".to_string(), serde_json::Value::Number(code.into()));
    error.insert("message".to_string(), serde_json::Value::String(message.to_string()));
    obj.insert("error".to_string(), Value::Object(error));
    Value::Object(obj)
}

/// Standardizes tool error responses with `isError: true` and appropriate content formatting.
pub fn jsonrpc_tool_error(id: Option<&Value>, message: &str) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".to_string(), serde_json::Value::String("2.0".to_string()));
    if let Some(id) = id {
        if !id.is_null() {
            obj.insert("id".to_string(), id.clone());
        }
    }
    let mut res_obj = serde_json::Map::new();
    res_obj.insert("content".to_string(), serde_json::json!([{ "type": "text", "text": message }]));
    res_obj.insert("isError".to_string(), Value::Bool(true));
    obj.insert("result".to_string(), Value::Object(res_obj));
    Value::Object(obj)
}

/// Checks if a JSON-RPC request is a tool call request (i.e., method == "tools/call").
pub fn is_tool_call_request(val: &Value) -> bool {
    matches!(get_method(val), Some(McpMethod::ToolsCall))
}

/// Checks if a JSON-RPC request uses a modern protocol version based on header or params.
pub fn is_modern_request(val: &Value, protocol_version_header: Option<&str>) -> bool {
    if let Some(version) = protocol_version_header {
        if version >= "2025-06-18" {
            return true;
        }
    }
    if get_method(val) == Some(McpMethod::Initialize) {
        if let Some(params) = get_params(val) {
            if let Some(v) = params.get("protocolVersion").and_then(|s| s.as_str()) {
                return v >= "2025-06-18";
            }
        }
    }
    false
}

/// Extracts the request ID from a JSON-RPC request value if present.
pub fn tool_call_request_id(val: &Value) -> Option<Value> {
    if is_tool_call_request(val) {
        val.get("id").cloned()
    } else {
        None
    }
}

/// Checks if a JSON-RPC response corresponds to a tool call response matching the given request ID.
pub fn is_tool_call_response(val: &Value, request_id: &Value) -> bool {
    // Must have matching id
    if val.get("id") != Some(request_id) {
        return false;
    }
    // Must not be a request or notification (must not have method)
    if val.get("method").is_some() {
        return false;
    }
    // Must have result or error
    val.get("result").is_some() || val.get("error").is_some()
}

/// Rewrites tool call result text content blocks using the provided rewriter closure.
pub fn rewrite_tool_call_result<F>(val: &mut Value, rewriter: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    let Some(result) = val.get_mut("result") else {
        return false;
    };
    let Some(content) = result.get_mut("content").and_then(|c| c.as_array_mut()) else {
        return false;
    };

    let mut changed = false;
    for item in content {
        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(text_item) = item.get_mut("text") {
                if let Some(text_val) = text_item.as_str() {
                    if let Some(rewritten) = rewriter(text_val) {
                        *text_item = Value::String(rewritten);
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        // If content was transformed, drop structuredContent to prevent LLM client confusion
        if let Some(obj) = result.as_object_mut() {
            obj.remove("structuredContent");
        }
    }

    changed
}
