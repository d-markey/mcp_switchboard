//! Helper functions for validating, identifying, and transforming JSON-RPC payloads
//! used throughout the Model Context Protocol (MCP) message specification.

use serde_json::Value;

/// Determines if the current incoming conversation should be considered "modern"
/// based on the client protocol spec version date.
///
/// Versions equal to or newer than `2025-06-18` support certain structured schemas
/// or content handshakes. We look up this configuration either from individual custom headers
/// or extracted parameter payloads during the initialize handshake.
pub fn is_modern_request(message: &Value, protocol_version_header: Option<&str>) -> bool {
    // If header is present, it must be >= 2025-06-18
    if let Some(header) = protocol_version_header {
        return is_at_least_version(header, "2025-06-18");
    }

    // Fallback to initialize request body
    if message.get("method").and_then(|v| v.as_str()) == Some("initialize") {
        if let Some(version) = message.get("params").and_then(|p| p.get("protocolVersion")).and_then(|v| v.as_str()) {
             return is_at_least_version(version, "2025-06-18");
        }
    }

    false
}

fn is_at_least_version(actual: &str, required: &str) -> bool {
    actual >= required
}

/// Extracts the JSON-RPC request identifier if and only if the current payload represents a `tools/call`.
///
/// This is used by the proxy module to track request-response mapping pairs so that corresponding
/// tool execution results can be intercepted and rewritten cleanly on their return trip.
pub fn tool_call_request_id(message: &Value) -> Option<Value> {
    if message.get("method").and_then(|v| v.as_str()) == Some("tools/call") {
        return message.get("id").cloned();
    }
    None
}

/// Confirms whether a given JSON payload is the exact response block matching a specific tool invocation.
///
/// It validates that the transaction IDs match and that the package does not contain a nested `method` block,
/// indicating that it is a response payload instead of a brand new inbound call request.
pub fn is_tool_call_response(message: &Value, request_id: &Value) -> bool {
    if message.get("id") != Some(request_id) {
        return false;
    }
    // Responses do not have "method"
    if message.get("method").is_some() {
        return false;
    }
    // Must have result or error
    message.get("result").is_some() || message.get("error").is_some()
}

/// Walks through the array blocks inside an MCP `tools/call` response and applies a closure
/// to rewrite any nested string values.
///
/// This is the backbone utility used to apply compactors (like Markdown table conversion or TOON serialization)
/// to raw data responses, shrinking the raw size before sending them to the LLM client.
pub fn rewrite_tool_call_result<F>(message: &mut Value, rewriter: F) -> bool
where F: Fn(&str) -> Option<String>
{
    let mut changed = false;
    if let Some(result) = message.get_mut("result") {
        if let Some(content) = result.get_mut("content").and_then(|c| c.as_array_mut()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        if let Some(rewritten) = rewriter(text) {
                            block.as_object_mut().unwrap().insert("text".to_string(), Value::String(rewritten));
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
             // If the text description has been compacted, any parallel fields like `structuredContent`
             // become redundant or out of sync. We remove them to maximize token conservation.
             result.as_object_mut().unwrap().remove("structuredContent");
        }
    }
    changed
}
