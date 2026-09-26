use mcp_switchboard::backend_stats::{BackendStatsRegistry, BackendStatus, HealthResponse};
use mcp_switchboard::health::{health_handler, compute_exposed_tools};
use mcp_switchboard::proxy::AppState;
use mcp_switchboard::config::BackendConfig;
use std::collections::HashMap;
use std::sync::Arc;
use axum::{
    extract::State,
    response::IntoResponse,
    body::to_bytes,
};

#[test]
fn test_backend_stats_registry_operations() {
    let mut registry = BackendStatsRegistry::new();
    registry.init_backends(vec!["backend1"]);
    let info = registry.get_info("backend1").unwrap();
    assert_eq!(info.status, BackendStatus::Waiting);

    registry.set_status("backend1", BackendStatus::Online);
    let info = registry.get_info("backend1").unwrap();
    assert_eq!(info.status, BackendStatus::Online);
}

#[test]
fn test_compute_exposed_tools() {
    let backend = BackendConfig {
        name: "my_server".to_string(),
        url: "http://localhost/mcp".to_string(),
        description: None,
        rewrite: None,
        use_tool_search: false,
        tool_prefix: "custom".to_string(),
        forward_origin: false,
        headers: HashMap::new(),
        tools: None,
        log_level: None,
        log_headers: vec![],
        resolved_log_level: None,
    };

    let raw = vec![
        serde_json::json!({"name": "foo"}),
        serde_json::json!({"name": "bar"})
    ];
    let exposed = compute_exposed_tools(&backend, &raw);
    assert_eq!(exposed, vec!["custom::bar".to_string(), "custom::foo".to_string()]);
}

#[tokio::test]
async fn test_health_handler_waiting_and_unhealthy() {
    let mut servers = HashMap::new();
    servers.insert(
        "backend_waiting".to_string(),
        BackendConfig {
            name: "backend_waiting".to_string(),
            url: "http://localhost:9999/mcp".to_string(),
            description: None,
            rewrite: None,
            use_tool_search: false,
            tool_prefix: "".to_string(),
            forward_origin: false,
            headers: HashMap::new(),
            tools: None,
            log_level: None,
            log_headers: vec![],
            resolved_log_level: None,
        },
    );

    servers.insert(
        "backend_offline".to_string(),
        BackendConfig {
            name: "backend_offline".to_string(),
            url: "http://localhost:9998/mcp".to_string(),
            description: None,
            rewrite: None,
            use_tool_search: false,
            tool_prefix: "".to_string(),
            forward_origin: false,
            headers: HashMap::new(),
            tools: None,
            log_level: None,
            log_headers: vec![],
            resolved_log_level: None,
        },
    );

    let mut backend_stats = BackendStatsRegistry::new();
    backend_stats.init_backends(servers.keys());
    backend_stats.set_status("backend_waiting", BackendStatus::Waiting);

    let state = AppState {
        backends: Arc::new(servers),
        client: reqwest::Client::new(),
        backend_stats,
    };

    let response = health_handler(State(state)).await.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let health_resp: HealthResponse = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(health_resp.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(health_resp.status, "unhealthy"); // because backend_offline call fails & status is offline

    let waiting_info = health_resp.backends.get("backend_waiting").unwrap();
    // Since backend_waiting has an invalid URL (localhost:9999), its tools/list call fails and it becomes offline in the handler
    assert_eq!(waiting_info.status, BackendStatus::Offline);

    let offline_info = health_resp.backends.get("backend_offline").unwrap();
    assert_eq!(offline_info.status, BackendStatus::Offline);
}
