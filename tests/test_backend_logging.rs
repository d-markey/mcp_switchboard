use mcp_switchboard::backend_logging::loggable_headers;
use axum::http::{HeaderMap, HeaderValue};

#[test]
fn test_loggable_headers_redaction() {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Authorization", HeaderValue::from_static("Bearer secret"));
    headers.insert("X-Custom-ID", HeaderValue::from_static("12345"));

    let whitelist = vec!["content-type".to_string(), "x-custom-id".to_string()];
    let safe = loggable_headers(&headers, &whitelist);

    assert_eq!(safe.get("content-type").unwrap(), "application/json");
    assert_eq!(safe.get("x-custom-id").unwrap(), "12345");
    assert_eq!(safe.get("authorization").unwrap(), "<redacted>");
}

#[test]
fn test_loggable_headers_empty_whitelist() {
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));

    let whitelist = vec![];
    let safe = loggable_headers(&headers, &whitelist);

    assert_eq!(safe.get("content-type").unwrap(), "<redacted>");
}

#[test]
fn test_loggable_headers_case_insensitivity() {
    let mut headers = HeaderMap::new();
    headers.insert("X-Custom-ID", HeaderValue::from_static("12345"));

    let whitelist = vec!["X-CUSTOM-ID".to_string()];
    let safe = loggable_headers(&headers, &whitelist);

    assert_eq!(safe.get("x-custom-id").unwrap(), "12345");
}
