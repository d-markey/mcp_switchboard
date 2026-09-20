//! Primary reverse proxy handler for routing incoming MCP requests.
//! It intercept requests, implements dynamic whitelisting/blacklists, unpacks synthetic Two-Step
//! tool routing meta-calls, and transparently passes the modified requests upstream.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use reqwest::Client;
use std::sync::Arc;

use crate::config::BackendConfig;
use crate::transform::maybe_rewrite_json_body;
use crate::tool_filter::is_tool_allowed;
use crate::tool_search::{build_tools_list_result, rewrite_call_tool_request, CALL_TOOL_NAME, DESCRIBE_TOOLS_NAME};
use crate::backend_logging::{log_proxy_request, log_proxy_response};

#[derive(Clone)]
pub struct AppState {
    pub backends: Arc<std::collections::HashMap<String, BackendConfig>>,
    pub client: Client,
}

// Hop-by-hop headers that should not be forwarded to upstreams per RFC 2616 specification rules.
const EXCLUDED_HEADERS: &[&str] = &[
    "content-length",
    "content-encoding",
    "connection",
    "transfer-encoding",
    "host",
];

/// The main router dispatcher endpoint.
/// Extracts target backend from URL path segments, sanitizes incoming transport headers,
/// detects if the payload is a special tool inquiry method, and processes stream or static results.
pub async fn proxy_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
    method: Method,
    headers: HeaderMap,
    body: BytesOrString,
) -> impl IntoResponse {
    let backend = match state.backends.get(&name) {
        Some(b) => b.clone(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let url = &backend.url;

    if let Some(level) = backend.resolved_log_level {
        log_proxy_request(
            level,
            &name,
            method.as_str(),
            url,
            &headers,
            &backend.log_headers,
        );
    }

    let mut req_builder = state.client.request(method.clone(), url);
    for (key, val) in &headers {
        let key_str = key.as_str().to_lowercase();
        if EXCLUDED_HEADERS.contains(&key_str.as_str()) {
            continue;
        }
        // Conditionally exclude origin tracking headers based on server config constraints to handle strict CORS.
        if key_str == "origin" && !backend.forward_origin {
            continue;
        }
        req_builder = req_builder.header(key, val);
    }

    // Inject static custom authentication keys declared in our config file if not already provided by the client.
    for (k, v) in &backend.headers {
        if !headers.contains_key(k) {
            req_builder = req_builder.header(k, v);
        }
    }

    let body_bytes = match body {
        BytesOrString::Bytes(b) => b,
        BytesOrString::String(s) => s.into_bytes(),
    };

    let mut tool_call_id = None;
    let final_body = if method == Method::POST {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            tool_call_id = crate::jsonrpc::tool_call_request_id(&json_val);

            if let Some(method_str) = json_val.get("method").and_then(|v| v.as_str()) {
                if method_str == "tools/list" {
                    // Intercept tool enumeration: if tool search mode is active, return the two meta-tools.
                    if backend.use_tool_search {
                        return handle_tools_list_synthetic(&state.client, &backend, &name, &json_val).await;
                    } else if backend.tools.is_some() {
                        // Otherwise apply typical flat whitelist/blacklist filtering before returning schemas.
                        return handle_tools_list_filtered(&state.client, &backend, &json_val).await;
                    }
                } else if method_str == "tools/call" {
                    if let Some(params) = json_val.get("params") {
                        if let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) {
                            if tool_name == DESCRIBE_TOOLS_NAME {
                                // Handled locally inside Switchboard, never forwarded upstream.
                                return handle_describe_tools(&state.client, &backend, &name, &json_val).await;
                            } else if tool_name == CALL_TOOL_NAME {
                                if let Some(prefix) = &backend.tool_prefix.clone().or_else(|| Some(name.clone())) {
                                    if let Some(real_name) = crate::tool_search::strip_prefix(prefix, params.get("arguments").and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or("")) {
                                        // Enforce security boundaries over namespaced tool requests.
                                        if !is_tool_allowed(real_name, backend.tools.as_ref()) {
                                            return handle_tool_not_found(&json_val, real_name);
                                        }
                                    }
                                    rewrite_call_tool_request(&mut json_val, prefix);
                                }
                            } else {
                                // Flat non-prefixed tool execution boundary validation check.
                                if !is_tool_allowed(tool_name, backend.tools.as_ref()) {
                                    return handle_tool_not_found(&json_val, tool_name);
                                }
                            }
                        }
                    }
                }
            }
            serde_json::to_vec(&json_val).unwrap_or(body_bytes)
        } else {
            body_bytes
        }
    } else {
        body_bytes
    };

    req_builder = req_builder.body(final_body);

    let resp = match req_builder.send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = resp.status();
    let resp_headers = resp.headers().clone();

    if let Some(level) = backend.resolved_log_level {
        log_proxy_response(
            level,
            &name,
            status.as_u16(),
            &resp_headers,
            &backend.log_headers,
        );
    }

    let content_type = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Check if the response returned data requires text formatting/compaction (JSON or SSE stream).
    let body = if let (Some(mode), Some(request_id)) = (backend.rewrite, tool_call_id) {
        if content_type.contains("application/json") {
            let full_bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
            };

            let rewritten = if full_bytes.len() > 4096 {
                let request_id = request_id.clone();
                let bytes_for_task = full_bytes.clone();
                tokio::task::spawn_blocking(move || {
                    maybe_rewrite_json_body(&bytes_for_task, mode, &request_id)
                })
                .await
                .unwrap_or_else(|_| String::from_utf8_lossy(&full_bytes).into_owned())
            } else {
                maybe_rewrite_json_body(&full_bytes, mode, &request_id)
            };
            Body::from(rewritten)
        } else if content_type.contains("text/event-stream") {
            // Use active reactive buffering stream to process SSE text lines chunk-by-chunk.
            let stream = crate::sse::SseRelayStream::new(resp.bytes_stream(), move |data: &[u8]| {
                maybe_rewrite_json_body(data, mode, &request_id)
            });
            Body::from_stream(stream)
        } else {
            Body::from_stream(resp.bytes_stream())
        }
    } else {
        Body::from_stream(resp.bytes_stream())
    };

    let mut response_builder = Response::builder().status(status);
    for (k, v) in &resp_headers {
        let k_str = k.as_str().to_lowercase();
        if EXCLUDED_HEADERS.contains(&k_str.as_str()) {
            continue;
        }
        response_builder = response_builder.header(k, v);
    }

    response_builder
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn handle_tool_not_found(req_json: &serde_json::Value, tool_name: &str) -> Response {
    let result = serde_json::json!({
        "jsonrpc": "2.0",
        "id": req_json.get("id"),
        "result": {
            "content": [
                {
                    "type": "text",
                    "text": format!("Error: Tool '{}' not found or not allowed", tool_name)
                }
            ],
            "isError": true
        }
    });
    Json(result).into_response()
}

async fn handle_tools_list_synthetic(
    client: &Client,
    backend: &BackendConfig,
    name: &str,
    req_json: &serde_json::Value,
) -> Response {
    let prefix = backend.tool_prefix.clone().unwrap_or_else(|| name.to_string());
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/list",
        "id": req_json.get("id").unwrap_or(&serde_json::json!(1))
    });

    let Ok(resp) = client.post(&backend.url).json(&list_req).send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let Ok(json_resp) = resp.json::<serde_json::Value>().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let tools = json_resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    let synthetic_result = build_tools_list_result(
        &tools,
        &prefix,
        backend.description.as_deref(),
        backend.tools.as_ref(),
    );

    let result = serde_json::json!({
        "jsonrpc": "2.0",
        "id": req_json.get("id"),
        "result": synthetic_result
    });

    Json(result).into_response()
}

async fn handle_tools_list_filtered(
    client: &Client,
    backend: &BackendConfig,
    req_json: &serde_json::Value,
) -> Response {
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/list",
        "id": req_json.get("id").unwrap_or(&serde_json::json!(1))
    });

    let Ok(resp) = client.post(&backend.url).json(&list_req).send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let Ok(mut json_resp) = resp.json::<serde_json::Value>().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    if let Some(result) = json_resp.get_mut("result") {
        if let Some(tools) = result.get_mut("tools").and_then(|t| t.as_array_mut()) {
            tools.retain(|t| {
                if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                    is_tool_allowed(name, backend.tools.as_ref())
                } else {
                    true
                }
            });
        }
    }

    let result = serde_json::json!({
        "jsonrpc": "2.0",
        "id": req_json.get("id"),
        "result": json_resp.get("result")
    });

    Json(result).into_response()
}

async fn handle_describe_tools(
    client: &Client,
    backend: &BackendConfig,
    name: &str,
    req_json: &serde_json::Value,
) -> Response {
    let prefix = backend.tool_prefix.clone().unwrap_or_else(|| name.to_string());
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/list",
        "id": req_json.get("id").unwrap_or(&serde_json::json!(1))
    });

    let Ok(resp) = client.post(&backend.url).json(&list_req).send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let Ok(json_resp) = resp.json::<serde_json::Value>().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };

    let tools = json_resp
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    let requested_names: Vec<String> = req_json
        .get("params")
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("names"))
        .and_then(|n| n.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let synthetic_result = crate::tool_search::build_describe_tools_result(
        &tools,
        &requested_names,
        &prefix,
    );

    let result = serde_json::json!({
        "jsonrpc": "2.0",
        "id": req_json.get("id"),
        "result": synthetic_result
    });

    Json(result).into_response()
}

pub enum BytesOrString {
    Bytes(Vec<u8>),
    String(String),
}

#[axum::async_trait]
impl<S> axum::extract::FromRequest<S> for BytesOrString
where
    S: Send + Sync,
{
    type Rejection = axum::response::Response;

    async fn from_request(req: axum::extract::Request, _state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST.into_response())?;
        Ok(BytesOrString::Bytes(bytes.to_vec()))
    }
}
