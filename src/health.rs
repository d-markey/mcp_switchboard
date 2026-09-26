//! Health check endpoint implementation.
//! Provides health status, version from TOML, and persistent backend info with status (waiting/online/offline),
//! all tool names exposed by the backend, and exposed tool names by the proxy.

use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};
use futures_util::future::join_all;
use tokio::time::timeout;
use std::time::Duration;
use crate::proxy::AppState;
use crate::config::BackendConfig;
use crate::tool_search::{prefixed_tool_name, DESCRIBE_TOOLS_NAME, CALL_TOOL_NAME, get_exposed_tool_names};
use crate::backend_stats::{BackendStatus, BackendHealthInfo, HealthResponse, BackendStatsRegistry};

/// Computes exposed tools based on backend config and raw tool names.
pub fn compute_exposed_tools(backend: &BackendConfig, raw_tools: &[serde_json::Value]) -> Vec<String> {
    if backend.use_tool_search {
        let mut exposed = vec![
            prefixed_tool_name(&backend.tool_prefix, DESCRIBE_TOOLS_NAME),
            prefixed_tool_name(&backend.tool_prefix, CALL_TOOL_NAME),
        ];
        exposed.sort();
        exposed
    } else {
        get_exposed_tool_names(raw_tools, &backend.tool_prefix, backend.tools.as_ref())
    }
}

async fn fetch_backend_info(
    name: String,
    backend: BackendConfig,
    client: reqwest::Client,
    backend_stats: BackendStatsRegistry,
) -> (String, BackendHealthInfo, bool) {
    let session_id = crate::tool_utils::initialize_backend(
        &client,
        &backend.url,
        &backend.headers,
    ).await;

    let call_res = timeout(
        Duration::from_secs(10),
        crate::tool_utils::fetch_tools_list(
            &client,
            &backend.url,
            &backend.headers,
            session_id.as_deref()
        )
    ).await;

    match call_res {
        Ok(Ok(json_resp)) => {
            let raw_tools_json = json_resp
                .get("result")
                .and_then(|r| r.get("tools"))
                .and_then(|t| t.as_array())
                .map(|arr| arr.as_slice())
                .unwrap_or(&[]);

            let raw_tools: Vec<String> = raw_tools_json
                .iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect();

            let mut tools_list = raw_tools.clone();
            tools_list.sort();

            let status = BackendStatus::Online;
            let exposed_tools = compute_exposed_tools(&backend, raw_tools_json);

            backend_stats.set_info(&name, status.clone(), tools_list.clone(), exposed_tools.clone());
            let info = BackendHealthInfo {
                status,
                tools: tools_list,
                exposed_tools,
            };
            (name, info, true)
        }
        _ => {
            let current_info = backend_stats.get_info(&name);
            let info = if let Some(inf) = current_info {
                let mut inf_offline = inf;
                inf_offline.status = BackendStatus::Offline;
                backend_stats.set_status(&name, BackendStatus::Offline);
                inf_offline
            } else {
                BackendHealthInfo {
                    status: BackendStatus::Offline,
                    tools: vec![],
                    exposed_tools: vec![],
                }
            };
            (name, info, false)
        }
    }
}

/// Axum route handler for returning proxy health status at `/health`.
pub async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let client = state.client.clone();
    let backend_stats = state.backend_stats.clone();

    let futures: Vec<_> = state.backends.iter().map(|(name, backend)| {
        tokio::spawn(
            fetch_backend_info(name.clone(), backend.clone(), client.clone(), backend_stats.clone())
        )
    }).collect();

    let results = join_all(futures).await;

    let mut backends_map = std::collections::HashMap::new();
    let mut overall_healthy = true;

    for res in results {
        if let Ok((name, info, is_healthy)) = res {
            overall_healthy &= is_healthy;
            backends_map.insert(name, info);
        } else {
            overall_healthy = false;
        }
    }

    let status = if overall_healthy { "healthy" } else { "unhealthy" }.to_string();

    Json(HealthResponse {
        version,
        status,
        backends: backends_map,
    })
}
