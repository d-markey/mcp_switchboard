//! Utilities for interacting with MCP tools list.

use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Fetches tools from a backend.
///
/// The `client` should be configured with appropriate timeouts externally,
/// or this function can be called within a `tokio::time::timeout` block.
pub async fn fetch_tools_list(
    client: &Client,
    url: &str,
    headers: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Result<Value, String> {
    let mut req_builder = client.post(url);
    for (k, v) in headers {
        req_builder = req_builder.header(k, v);
    }

    req_builder = req_builder.header("accept", "application/json, text/event-stream");
    if let Some(sid) = session_id {
        req_builder = req_builder.header("mcp-session-id", sid);
    }

    let list_req = crate::jsonrpc::jsonrpc_request(
        Some(&json!(1)),
        "tools/list",
        None
    );

    let resp = req_builder
        .json(&list_req)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Server returned status: {}", resp.status()));
    }

    resp.json::<Value>().await.map_err(|e| e.to_string())
}

/// Fetches tools from a backend and ensures the result is a clean tools array.
pub async fn fetch_and_filter_tools(
    client: &Client,
    url: &str,
    headers: &HashMap<String, String>,
    session_id: Option<&str>,
) -> Result<Vec<Value>, String> {
    let resp = fetch_tools_list(client, url, headers, session_id).await?;
    resp.get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .ok_or_else(|| "Invalid or missing tools in response".to_string())
}

/// Initializes a backend using MCP `initialize` handshake, returning the optional `mcp-session-id`.
pub async fn initialize_backend(
    client: &Client,
    url: &str,
    headers: &HashMap<String, String>,
) -> Option<String> {
    let init_req = crate::jsonrpc::jsonrpc_request(
        Some(&serde_json::json!(1)),
        "initialize",
        Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "mcp-switchboard",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    );

    let mut req_builder = client.post(url);
    for (k, v) in headers {
        req_builder = req_builder.header(k, v);
    }
    req_builder = req_builder.header("accept", "application/json, text/event-stream");

    let init_res = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        req_builder.json(&init_req).send()
    ).await;

    match init_res {
        Ok(Ok(resp)) if resp.status().is_success() => {
            resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(String::from)
        }
        _ => None,
    }
}

