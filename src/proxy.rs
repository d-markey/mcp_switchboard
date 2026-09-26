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
use serde_json::json;
use std::sync::Arc;

use crate::config::BackendConfig;
use crate::transform::{maybe_rewrite_json_body, maybe_rewrite_json_body_async};
use crate::tool_filter::is_tool_allowed;
use crate::tool_search::{build_tools_list_result, rewrite_call_tool_request, CALL_TOOL_NAME, DESCRIBE_TOOLS_NAME};
use crate::backend_logging::{log_proxy_request, log_proxy_response};
use crate::backend_stats::BackendStatsRegistry;
use crate::jsonrpc::{get_method, get_params, get_arguments, McpMethod};
use crate::http_utils::is_excluded_header;

#[derive(Clone)]
pub struct AppState {
    pub backends: Arc<std::collections::HashMap<String, BackendConfig>>,
    pub client: Client,
    pub backend_stats: BackendStatsRegistry,
}

/// The main router dispatcher endpoint.
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

    state.backend_stats.inc_request(&name);
    struct PendingGuard {
        backend_stats: BackendStatsRegistry,
        backend_name: String,
    }
    impl Drop for PendingGuard {
        fn drop(&mut self) {
            self.backend_stats.dec_pending(&self.backend_name);
        }
    }
    let _pending_guard = PendingGuard {
        backend_stats: state.backend_stats.clone(),
        backend_name: name.clone(),
    };

    let url = &backend.url;

    if let Some(level) = backend.resolved_log_level {
        log_proxy_request(level, &name, method.as_str(), url, &headers, &backend.log_headers);
    }

    let mut req_builder = state.client.request(method.clone(), url);
    for (key, val) in &headers {
        let key_str = key.as_str();
        if is_excluded_header(key_str) || (key_str.eq_ignore_ascii_case("origin") && !backend.forward_origin) {
            continue;
        }
        req_builder = req_builder.header(key, val);
    }

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
    let mut is_tool_call = false;
    let final_body = if method == Method::POST {
        if let Ok(mut json_val) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            tool_call_id = crate::jsonrpc::tool_call_request_id(&json_val);
            is_tool_call = tool_call_id.is_some();

            match get_method(&json_val) {
                Some(McpMethod::ToolsList) => {
                    if backend.use_tool_search {
                        return handle_tools_list_synthetic(&state.client, &backend, &name, &json_val).await;
                    } else if backend.tools.is_some() {
                        return handle_tools_list_filtered(&state.client, &backend, &json_val).await;
                    }
                }
                Some(McpMethod::ToolsCall) => {
                    if let Some(params) = get_params(&json_val) {
                        if let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) {
                            if tool_name == DESCRIBE_TOOLS_NAME {
                                return handle_describe_tools(&state.client, &backend, &name, &json_val).await;
                            } else if tool_name == CALL_TOOL_NAME {
                                let prefix = &backend.tool_prefix;
                                let inner_name = get_arguments(params).and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                                if let Some(real_name) = crate::tool_search::strip_prefix(prefix, inner_name) {
                                    if !is_tool_allowed(real_name, backend.tools.as_ref()) {
                                        return handle_tool_not_found(&json_val, real_name);
                                    }
                                } else {
                                    return handle_tool_not_found(&json_val, inner_name);
                                }
                                rewrite_call_tool_request(&mut json_val, prefix);
                            } else {
                                if !is_tool_allowed(tool_name, backend.tools.as_ref()) {
                                    return handle_tool_not_found(&json_val, tool_name);
                                }
                            }
                        }
                    }
                }
                _ => {}
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
        Ok(r) => {
            if r.status().is_success() || r.status().is_client_error() || r.status().is_server_error() {
                state.backend_stats.set_status(&name, crate::backend_stats::BackendStatus::Online);
            }
            r
        }
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status = resp.status();
    let resp_headers = resp.headers().clone();

    if let Some(level) = backend.resolved_log_level {
        log_proxy_response(level, &name, status.as_u16(), &resp_headers, &backend.log_headers);
    }

    let content_type = resp_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let backend_stats = state.backend_stats.clone();
    let backend_name_for_body = name.clone();

    let body = if let (Some(mode), Some(request_id)) = (backend.rewrite, tool_call_id) {
        if crate::http_utils::is_json_content_type(&content_type) {
            let full_bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
            };
            if is_tool_call { backend_stats.add_tool_call_received_bytes(&backend_name_for_body, full_bytes.len() as u64); }

            let (rewritten, _) = maybe_rewrite_json_body_async(&full_bytes, mode, &request_id).await;

            if is_tool_call { backend_stats.add_tool_call_response_sent_bytes(&backend_name_for_body, rewritten.len() as u64); }
            Body::from(rewritten)
        } else if crate::http_utils::is_sse_content_type(&content_type) {
            let stats_reg_clone = backend_stats.clone();
            let b_name = backend_name_for_body.clone();
            let is_tc = is_tool_call;
            let stream = crate::sse::SseRelayStream::new(resp.bytes_stream(), move |data: &[u8]| {
                if is_tc { stats_reg_clone.add_tool_call_received_bytes(&b_name, data.len() as u64); }
                let (rewritten, _) = maybe_rewrite_json_body(data, mode, &request_id);
                if is_tc { stats_reg_clone.add_tool_call_response_sent_bytes(&b_name, rewritten.len() as u64); }
                rewritten
            });
            Body::from_stream(stream)
        } else {
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
            };
            let size = bytes.len() as u64;
            if is_tool_call {
                backend_stats.add_tool_call_received_bytes(&backend_name_for_body, size);
                backend_stats.add_tool_call_response_sent_bytes(&backend_name_for_body, size);
            }
            Body::from(bytes)
        }
    } else {
        if crate::http_utils::is_sse_content_type(&content_type) && is_tool_call {
            let stats_reg_clone = backend_stats.clone();
            let b_name = backend_name_for_body.clone();
            let stream = crate::sse::SseRelayStream::new(resp.bytes_stream(), move |data: &[u8]| {
                stats_reg_clone.add_tool_call_received_bytes(&b_name, data.len() as u64);
                String::from_utf8_lossy(data).into_owned()
            });
            Body::from_stream(stream)
        } else {
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
            };
            let size = bytes.len() as u64;
            if is_tool_call { backend_stats.add_tool_call_received_bytes(&backend_name_for_body, size); }
            Body::from(bytes)
        }
    };

    let mut response_builder = Response::builder().status(status);
    for (k, v) in &resp_headers {
        if !is_excluded_header(k.as_str()) {
            response_builder = response_builder.header(k, v);
        }
    }
    response_builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn handle_tool_not_found(req_json: &serde_json::Value, tool_name: &str) -> Response {
    let result = crate::jsonrpc::jsonrpc_tool_error(
        req_json.get("id"),
        &format!("Error: Tool '{}' not found or not allowed", tool_name),
    );
    Json(result).into_response()
}

async fn handle_tools_list_synthetic(client: &Client, backend: &BackendConfig, _name: &str, req_json: &serde_json::Value) -> Response {
    let json_resp = match crate::tool_utils::fetch_tools_list(client, &backend.url, &backend.headers, None).await {
        Ok(j) => j,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let tools = json_resp.get("result").and_then(|r| r.get("tools")).and_then(|t| t.as_array()).cloned().unwrap_or_default();
    let result = crate::jsonrpc::jsonrpc_response(
        req_json.get("id"),
        build_tools_list_result(&tools, &backend.tool_prefix, backend.description.as_deref(), backend.tools.as_ref())
    );
    Json(result).into_response()
}

async fn handle_tools_list_filtered(client: &Client, backend: &BackendConfig, req_json: &serde_json::Value) -> Response {
    let tools = match crate::tool_utils::fetch_and_filter_tools(client, &backend.url, &backend.headers, None).await {
        Ok(t) => t,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let result = crate::jsonrpc::jsonrpc_response(
        req_json.get("id"),
        json!({ "tools": crate::tool_filter::filter_tools(tools, backend.tools.as_ref()) })
    );
    Json(result).into_response()
}

async fn handle_describe_tools(client: &Client, backend: &BackendConfig, _name: &str, req_json: &serde_json::Value) -> Response {
    let tools = match crate::tool_utils::fetch_and_filter_tools(client, &backend.url, &backend.headers, None).await {
        Ok(t) => t,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let requested_names: Vec<String> = get_params(req_json)
        .and_then(|p| get_arguments(p))
        .and_then(|a| a.get("names"))
        .and_then(|n| n.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let result = crate::jsonrpc::jsonrpc_response(
        req_json.get("id"),
        crate::tool_search::build_describe_tools_result(&tools, &requested_names, &backend.tool_prefix, backend.tools.as_ref())
    );
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
            .await.map_err(|_| StatusCode::BAD_REQUEST.into_response())?;
        Ok(BytesOrString::Bytes(bytes.to_vec()))
    }
}
