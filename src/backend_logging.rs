//! Utility for safely stripping or preserving specific HTTP header keys in log outputs.
//! This prevents leaking credentials or auth tokens into trace dumps while maintaining
//! debugging visibility.

use std::collections::{HashMap, HashSet};
use axum::http::HeaderMap;
use tracing::Level;

/// Processes a HeaderMap and redacts values whose keys are not explicitly whitelisted.
pub fn loggable_headers(headers: &HeaderMap, whitelist: &[String]) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let whitelist_set: HashSet<_> = whitelist.iter().map(|s| s.to_lowercase()).collect();

    for (k, v) in headers {
        let k_str = k.as_str();
        let k_lower = k_str.to_lowercase();
        let v_str = v.to_str().unwrap_or("<binary>");

        if whitelist_set.contains(&k_lower) {
            result.insert(k_str.to_string(), v_str.to_string());
        } else {
            result.insert(k_str.to_string(), "<redacted>".to_string());
        }
    }
    result
}

/// Logs an incoming proxy request based on the backend's configured log level.
pub fn log_proxy_request(
    level: Level,
    backend_name: &str,
    method: &str,
    url: &str,
    headers: &HeaderMap,
    whitelist: &[String],
) {
    let safe_headers = loggable_headers(headers, whitelist);
    match level {
        Level::TRACE => tracing::trace!(backend = %backend_name, method = %method, url = %url, headers = ?safe_headers, "Proxying request"),
        Level::DEBUG => tracing::debug!(backend = %backend_name, method = %method, url = %url, headers = ?safe_headers, "Proxying request"),
        Level::INFO => tracing::info!(backend = %backend_name, method = %method, url = %url, "Proxying request"),
        Level::WARN => tracing::warn!(backend = %backend_name, method = %method, url = %url, "Proxying request"),
        Level::ERROR => tracing::error!(backend = %backend_name, method = %method, url = %url, "Proxying request"),
    }
}

/// Logs an upstream proxy response based on the backend's configured log level.
pub fn log_proxy_response(
    level: Level,
    backend_name: &str,
    status: u16,
    headers: &HeaderMap,
    whitelist: &[String],
) {
    let safe_headers = loggable_headers(headers, whitelist);
    match level {
        Level::TRACE => tracing::trace!(backend = %backend_name, status = %status, headers = ?safe_headers, "Received response"),
        Level::DEBUG => tracing::debug!(backend = %backend_name, status = %status, headers = ?safe_headers, "Received response"),
        Level::INFO => tracing::info!(backend = %backend_name, status = %status, "Received response"),
        Level::WARN => tracing::warn!(backend = %backend_name, status = %status, "Received response"),
        Level::ERROR => tracing::error!(backend = %backend_name, status = %status, "Received response"),
    }
}
