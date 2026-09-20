use axum::{
    routing::post,
    extract::Request,
    response::{Response, IntoResponse},
    body::Body,
    http::{StatusCode, HeaderMap},
    Router,
};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::path::PathBuf;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use reqwest::Client;

// Helper to find a free port
fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

// ---------------------------
// Fake Backend Implementation
// ---------------------------

async fn fake_backend_endpoint(headers: HeaderMap, request: Request) -> Response {
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await.unwrap_or_default();
    let message: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));

    let method = message.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = message.get("id").cloned().unwrap_or(json!(1));

    if method == "initialize" {
        let resp = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "serverInfo": {"name": "fake-backend", "version": "0.0.1"}
            }
        });
        return axum::Json(resp).into_response();
    } else if method == "notifications/initialized" {
        return StatusCode::OK.into_response();
    } else if method == "tools/list" {
        let resp = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {"name": "get_table", "description": "returns a table", "inputSchema": {"type": "object"}},
                    {"name": "get_scalar", "description": "returns plain json", "inputSchema": {"type": "object"}},
                    {"name": "whoami", "description": "echoes the caller's Authorization header", "inputSchema": {"type": "object"}}
                ]
            }
        });
        return axum::Json(resp).into_response();
    } else if method == "tools/call" {
        let params = message.get("params");
        let name = params.and_then(|p| p.get("name")).and_then(|n| n.as_str()).unwrap_or("");
        let payload = if name == "get_table" {
            json!({
                "result": [
                    {"id": 1, "name": "alice", "score": 9.5},
                    {"id": 2, "name": "bob", "score": 7.1}
                ],
                "metadata": {"total": 2, "page": 1}
            })
        } else if name == "whoami" {
            let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
            json!({"authorization": auth})
        } else {
            json!({"ok": true})
        };
        let resp = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": serde_json::to_string(&payload).unwrap()}]
            }
        });
        return axum::Json(resp).into_response();
    }

    StatusCode::OK.into_response()
}

async fn origin_echo_endpoint(headers: HeaderMap, request: Request) -> Response {
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await.unwrap_or_default();
    let message: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
    let id = message.get("id").cloned().unwrap_or(json!(1));
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let payload = json!({"origin": origin});
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": serde_json::to_string(&payload).unwrap()}]
        }
    });
    axum::Json(resp).into_response()
}

async fn raw_sse_endpoint(request: Request) -> Response {
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await.unwrap_or_default();
    let message: Value = serde_json::from_slice(&body_bytes).unwrap_or(json!({}));
    let id = message.get("id").cloned().unwrap_or(json!(1));
    let table_rows = json!([{"id": 1, "name": "alice"}, {"id": 2, "name": "bob"}]);
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"content": [{"type": "text", "text": serde_json::to_string(&table_rows).unwrap()}]}
    });
    let pretty = serde_json::to_string_pretty(&envelope).unwrap();
    let sse_body: String = pretty.lines().map(|l| format!("data: {}\n", l)).collect::<String>() + "\n";

    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(sse_body))
        .unwrap()
}

// Helper to spawn a background Axum server
async fn spawn_server(router: Router) -> String {
    let port = find_free_port();
    let addr: SocketAddr = format!("127.0.0.1:{}", port).parse().unwrap();
    let listener = TcpListener::bind(addr).await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://127.0.0.1:{}", port)
}

// Spawn switchboard binary process
struct SwitchboardProcess {
    child: tokio::process::Child,
    pub base_url: String,
}

async fn spawn_switchboard(config_path: &std::path::Path) -> SwitchboardProcess {
    let port = find_free_port();
    let bin_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/mcp-switchboard.exe");
    let child = Command::new(&bin_path)
        .arg("--config")
        .arg(config_path)
        .arg("--port")
        .arg(port.to_string())
        .spawn()
        .expect("Failed to start mcp-switchboard binary");

    let base_url = format!("http://127.0.0.1:{}", port);
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            panic!("mcp-switchboard failed to start within timeout");
        }
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await.is_ok() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    SwitchboardProcess { child, base_url }
}

impl Drop for SwitchboardProcess {
    fn drop(&mut self) {
        drop(self.child.kill());
    }
}

// MCP JSON-RPC client helper over HTTP / SSE
async fn mcp_call_tool(proxy_url: &str, tool_name: &str, arguments: Value, authorization: Option<&str>) -> (bool, Value) {
    let client = Client::new();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Accept", "application/json, text/event-stream".parse().unwrap());
    headers.insert("MCP-Protocol-Version", "2025-06-18".parse().unwrap());
    if let Some(auth) = authorization {
        headers.insert("Authorization", auth.parse().unwrap());
    }

    // 1. Initialize
    let init_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "e2e-rust", "version": "0.1.0"}
        }
    });
    let resp = client.post(proxy_url).headers(headers.clone()).json(&init_body).send().await.unwrap();
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        panic!("Initialize failed with status {}: {}", status, body);
    }
    let session_id = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    if let Some(sid) = session_id {
        headers.insert("mcp-session-id", sid.parse().unwrap());
    }

    // 2. Initialized notification
    let notif_body = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let _ = client.post(proxy_url).headers(headers.clone()).json(&notif_body).send().await;

    // 3. Call tool
    let call_body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });
    let call_resp = client.post(proxy_url).headers(headers).json(&call_body).send().await.unwrap();
    if !call_resp.status().is_success() {
        let status = call_resp.status();
        let body = call_resp.text().await.unwrap_or_default();
        panic!("Call tool failed with status {}: {}", status, body);
    }
    let call_bytes = call_resp.bytes().await.unwrap();

    // Parse SSE or JSON response
    let text = String::from_utf8_lossy(&call_bytes);
    let message: Value = if text.contains("data:") {
        let data_lines: Vec<&str> = text.lines()
            .filter(|l| l.starts_with("data:"))
            .map(|l| l.strip_prefix("data:").unwrap().trim())
            .collect();
        serde_json::from_str(&data_lines.join("\n")).unwrap_or(json!({}))
    } else {
        serde_json::from_slice(&call_bytes).unwrap_or(json!({}))
    };

    let is_error = message.get("result").and_then(|r| r.get("isError")).and_then(|e| e.as_bool()).unwrap_or(false);
    let result = message.get("result").cloned().unwrap_or(message);
    (is_error, result)
}

async fn mcp_list_tools(proxy_url: &str) -> Vec<Value> {
    let client = Client::new();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("Accept", "application/json, text/event-stream".parse().unwrap());
    headers.insert("MCP-Protocol-Version", "2025-06-18".parse().unwrap());

    let init_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "e2e-rust", "version": "0.1.0"}
        }
    });
    let resp = client.post(proxy_url).headers(headers.clone()).json(&init_body).send().await.unwrap();
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        panic!("Initialize failed with status {}: {}", status, body);
    }
    if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        headers.insert("mcp-session-id", sid.parse().unwrap());
    }

    let _ = client.post(proxy_url).headers(headers.clone()).json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).send().await;

    let list_body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    let list_resp = client.post(proxy_url).headers(headers).json(&list_body).send().await.unwrap();
    if !list_resp.status().is_success() {
        let status = list_resp.status();
        let body = list_resp.text().await.unwrap_or_default();
        panic!("List tools failed with status {}: {}", status, body);
    }
    let bytes = list_resp.bytes().await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let message: Value = if text.contains("data:") {
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("data:")).map(|l| l.strip_prefix("data:").unwrap().trim()).collect();
        serde_json::from_str(&lines.join("\n")).unwrap_or(json!({}))
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };

    message.get("result").and_then(|r| r.get("tools")).and_then(|t| t.as_array()).cloned().unwrap_or_default()
}

// ----------------
// End-to-End Tests
// ----------------

#[tokio::test]
async fn test_tools_list_passes_through_unchanged() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    rewrite: md_tables\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let tools = mcp_list_tools(&proxy_url).await;
    let names: std::collections::HashSet<String> = tools.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect();
    assert_eq!(names, vec!["get_table", "get_scalar", "whoami"].into_iter().map(String::from).collect());
}

#[tokio::test]
async fn test_table_result_is_rendered_as_markdown() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    rewrite: md_tables\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let (_, result) = mcp_call_tool(&proxy_url, "get_table", json!({}), None).await;
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();

    assert!(text.contains("| id | name | score |"));
    assert!(text.contains("| 1 | alice | 9.5 |"));
    assert!(text.contains("| 2 | bob | 7.1 |"));
    assert!(text.contains("```json"));
    assert!(text.contains("\"total\": 2"));
    assert!(text.contains("\"page\": 1"));
}

#[tokio::test]
async fn test_scalar_result_passes_through_unchanged() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    rewrite: md_tables\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let (_, result) = mcp_call_tool(&proxy_url, "get_scalar", json!({}), None).await;
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert_eq!(text, "{\"ok\":true}");
}

#[tokio::test]
async fn test_authorization_header_is_forwarded_per_caller() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    rewrite: md_tables\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let (_, res_a) = mcp_call_tool(&proxy_url, "whoami", json!({}), Some("Bearer token-a")).await;
    let (_, res_b) = mcp_call_tool(&proxy_url, "whoami", json!({}), Some("Bearer token-b")).await;
    let text_a = res_a.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    let text_b = res_b.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();

    assert_eq!(text_a, "{\"authorization\":\"Bearer token-a\"}");
    assert_eq!(text_b, "{\"authorization\":\"Bearer token-b\"}");
}

#[tokio::test]
async fn test_configured_header_is_used_when_caller_sends_none() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    headers:\n      Authorization: Bearer proxy-configured-key\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let (_, result) = mcp_call_tool(&proxy_url, "whoami", json!({}), None).await;
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert_eq!(text, "{\"authorization\":\"Bearer proxy-configured-key\"}");
}

#[tokio::test]
async fn test_callers_own_header_overrides_configured_header() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n    headers:\n      Authorization: Bearer proxy-configured-key\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let (_, result) = mcp_call_tool(&proxy_url, "whoami", json!({}), Some("Bearer client-supplied-key")).await;
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert_eq!(text, "{\"authorization\":\"Bearer client-supplied-key\"}");
}

#[tokio::test]
async fn test_plain_tool_filter_hides_blacklisted_tool_from_list() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  plain:\n    url: {}/mcp\n    tools:\n      whitelist:\n        - '*'\n      blacklist:\n        - whoami\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/plain/mcp", switchboard.base_url);

    let tools = mcp_list_tools(&proxy_url).await;
    let names: std::collections::HashSet<String> = tools.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect();
    assert_eq!(names, vec!["get_table", "get_scalar"].into_iter().map(String::from).collect());
}

#[tokio::test]
async fn test_plain_tool_filter_blocks_calling_a_blacklisted_tool() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  plain:\n    url: {}/mcp\n    tools:\n      whitelist:\n        - '*'\n      blacklist:\n        - whoami\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/plain/mcp", switchboard.base_url);

    let (is_error, result) = mcp_call_tool(&proxy_url, "whoami", json!({}), None).await;
    assert!(is_error);
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert!(text.contains("not found or not allowed"));
}

#[tokio::test]
async fn test_plain_tool_filter_still_allows_whitelisted_tool() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  plain:\n    url: {}/mcp\n    tools:\n      whitelist:\n        - '*'\n      blacklist:\n        - whoami\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/plain/mcp", switchboard.base_url);

    let (_, result) = mcp_call_tool(&proxy_url, "get_scalar", json!({}), None).await;
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert_eq!(text, "{\"ok\":true}");
}

#[tokio::test]
async fn test_tool_search_filter_hides_blacklisted_tool_from_describe_tools() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  toolsearch:\n    url: {}/mcp\n    use_tool_search: true\n    tools:\n      whitelist:\n        - '*'\n      blacklist:\n        - whoami\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/toolsearch/mcp", switchboard.base_url);

    let tools = mcp_list_tools(&proxy_url).await;
    let describe_tool = tools.iter().find(|t| t.get("name").and_then(|n| n.as_str()) == Some("describe_tools")).unwrap();
    let desc = describe_tool.get("description").and_then(|d| d.as_str()).unwrap();

    assert!(desc.contains("toolsearch::get_table"));
    assert!(desc.contains("toolsearch::get_scalar"));
    assert!(!desc.contains("toolsearch::whoami"));
}

#[tokio::test]
async fn test_tool_search_filter_blocks_calling_a_blacklisted_tool() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  toolsearch:\n    url: {}/mcp\n    use_tool_search: true\n    tools:\n      whitelist:\n        - '*'\n      blacklist:\n        - whoami\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/toolsearch/mcp", switchboard.base_url);

    let (is_error, result) = mcp_call_tool(&proxy_url, "call_tool", json!({"name": "toolsearch::whoami", "arguments": {}}), None).await;
    assert!(is_error);
    let text = result.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert!(text.contains("not found or not allowed"));
}

#[tokio::test]
async fn test_cors_preflight_allows_configured_origin() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  backend:\n    url: {}/mcp\ncors:\n  allow_origins:\n    - https://allowed.example.com\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let client = Client::new();
    let resp = client.request(reqwest::Method::OPTIONS, &proxy_url)
        .header("Origin", "https://allowed.example.com")
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "content-type,mcp-session-id")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("https://allowed.example.com"));
}

#[tokio::test]
async fn test_cors_preflight_rejects_unconfigured_origin() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  backend:\n    url: {}/mcp\ncors:\n  allow_origins:\n    - https://allowed.example.com\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let client = Client::new();
    let resp = client.request(reqwest::Method::OPTIONS, &proxy_url)
        .header("Origin", "https://not-allowed.example.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    assert!(resp.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn test_cors_exposes_session_id_header_on_real_response() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  backend:\n    url: {}/mcp\ncors:\n  allow_origins:\n    - https://allowed.example.com\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let client = Client::new();
    let resp = client.post(&proxy_url)
        .header("Origin", "https://allowed.example.com")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "0"}}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("https://allowed.example.com"));
    let expose = resp.headers().get("access-control-expose-headers").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    assert!(expose.contains("mcp-session-id"));
}

#[tokio::test]
async fn test_proxy_without_cors_config_sends_no_cors_headers() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  backend:\n    url: {}/mcp\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let client = Client::new();
    let resp = client.request(reqwest::Method::OPTIONS, &proxy_url)
        .header("Origin", "https://anywhere.example.com")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .unwrap();

    assert!(resp.headers().get("access-control-allow-origin").is_none());
}

async fn post_with_origin(url: &str, origin: &str) -> Option<String> {
    let client = Client::new();
    let resp = client.post(url)
        .header("Origin", origin)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call"}))
        .send()
        .await
        .unwrap();
    let message: Value = resp.json().await.unwrap();
    let text = message.get("result")?.get("content")?.get(0)?.get("text")?.as_str()?;
    let payload: Value = serde_json::from_str(text).ok()?;
    payload.get("origin")?.as_str().map(|s| s.to_string())
}

#[tokio::test]
async fn test_origin_header_forwarding() {
    let backend_app = Router::new().route("/mcp", post(origin_echo_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  default:\n    url: {}/mcp\n  forwarding:\n    url: {}/mcp\n    forward_origin: true\n",
        backend_url, backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let default_url = format!("{}/default/mcp", switchboard.base_url);
    let forwarding_url = format!("{}/forwarding/mcp", switchboard.base_url);

    let received_default = post_with_origin(&default_url, "https://caller.example.com").await;
    assert!(received_default.is_none());

    let received_forwarding = post_with_origin(&forwarding_url, "https://caller.example.com").await;
    assert_eq!(received_forwarding, Some("https://caller.example.com".to_string()));
}

#[tokio::test]
async fn test_concentrator_routes_each_name_to_its_own_backend_session() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  alpha:\n    url: {}/mcp\n    rewrite: md_tables\n  beta:\n    url: {}/mcp\n",
        backend_url, backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let alpha_url = format!("{}/alpha/mcp", switchboard.base_url);
    let beta_url = format!("{}/beta/mcp", switchboard.base_url);

    let (_, res_alpha) = mcp_call_tool(&alpha_url, "get_scalar", json!({}), None).await;
    let (_, res_beta) = mcp_call_tool(&beta_url, "get_scalar", json!({}), None).await;

    assert_eq!(res_alpha.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()), Some("{\"ok\":true}"));
    assert_eq!(res_beta.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()), Some("{\"ok\":true}"));
}

#[tokio::test]
async fn test_concentrator_rejects_unconfigured_name() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!("servers:\n  alpha:\n    url: {}/mcp\n", backend_url)).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let client = Client::new();
    let resp = client.get(format!("{}/not-a-configured-name/mcp", switchboard.base_url)).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_tool_search_describe_tools_and_call_tool() {
    let backend_app = Router::new().route("/mcp", post(fake_backend_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  toolsearch:\n    url: {}/mcp\n    use_tool_search: true\n    rewrite: md_tables\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/toolsearch/mcp", switchboard.base_url);

    let tools = mcp_list_tools(&proxy_url).await;
    let names: std::collections::HashSet<String> = tools.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect();
    assert_eq!(names, vec!["describe_tools", "call_tool"].into_iter().map(String::from).collect());

    // Call describe_tools with valid and invalid
    let (_, desc_res) = mcp_call_tool(&proxy_url, "describe_tools", json!({"names": ["toolsearch::get_table", "toolsearch::not_a_real_tool"]}), None).await;
    // Note: describe_tools returns isError=true if any tool is unknown
    let content = desc_res.get("content").and_then(|c| c.as_array()).unwrap();
    let text_0 = content.first().and_then(|c| c.get("text")).and_then(|v| v.as_str()).unwrap();
    let described: Value = serde_json::from_str(text_0).unwrap();
    assert_eq!(described.get("name").and_then(|v| v.as_str()).unwrap(), "toolsearch::get_table");
    let text_1 = content.get(1).and_then(|c| c.get("text")).and_then(|v| v.as_str()).unwrap();
    assert_eq!(text_1, "unknown tool: toolsearch::not_a_real_tool");

    // Call call_tool
    let (_, call_res) = mcp_call_tool(&proxy_url, "call_tool", json!({"name": "toolsearch::get_table", "arguments": {}}), None).await;
    let text = call_res.get("content").and_then(|c| c.get(0)).and_then(|t| t.get("text")).and_then(|s| s.as_str()).unwrap();
    assert!(text.starts_with("| id | name | score |"));
}

#[tokio::test]
async fn test_multi_line_sse_data_from_a_pretty_printed_backend_is_rewritten() {
    let backend_app = Router::new().route("/mcp", post(raw_sse_endpoint));
    let backend_url = spawn_server(backend_app).await;

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.yaml");
    std::fs::write(&config_path, format!(
        "servers:\n  backend:\n    url: {}/mcp\n    rewrite: md_tables\n",
        backend_url
    )).unwrap();

    let switchboard = spawn_switchboard(&config_path).await;
    let proxy_url = format!("{}/backend/mcp", switchboard.base_url);

    let client = Client::new();
    let resp = client.post(&proxy_url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "get_table", "arguments": {}}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.bytes().await.unwrap();
    let text = String::from_utf8_lossy(&bytes);

    let data_lines: Vec<&str> = text.lines()
        .filter(|l| l.starts_with("data:"))
        .map(|l| l.strip_prefix("data:").unwrap().trim())
        .collect();
    let message: Value = serde_json::from_str(&data_lines.join("\n")).unwrap();
    let result_text = message.get("result").unwrap().get("content").unwrap().get(0).unwrap().get("text").unwrap().as_str().unwrap();
    assert!(result_text.starts_with("| id | name |"));
}
