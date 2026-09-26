//! Combined backend statistics and health registry.
//! Tracks backend status, tools list, exposed tools, pending requests, total requests,
//! total payload size received after a tool call, and payload size sent in response to tool calls.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use dashmap::DashMap;
use axum::{
    extract::State,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::proxy::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendStatus {
    Waiting,
    Online,
    Offline,
}

/// Statistics metrics tracker and health info for a single backend.
#[derive(Debug)]
pub struct BackendStats {
    pub status: RwLock<BackendStatus>,
    pub tools: RwLock<Vec<String>>,
    pub exposed_tools: RwLock<Vec<String>>,
    pub pending_requests: AtomicUsize,
    pub total_requests: AtomicU64,
    pub total_tool_call_received_bytes: AtomicU64,
    pub total_tool_call_response_sent_bytes: AtomicU64,
}

impl Default for BackendStats {
    fn default() -> Self {
        Self {
            status: RwLock::new(BackendStatus::Waiting),
            tools: RwLock::new(Vec::new()),
            exposed_tools: RwLock::new(Vec::new()),
            pending_requests: AtomicUsize::new(0),
            total_requests: AtomicU64::new(0),
            total_tool_call_received_bytes: AtomicU64::new(0),
            total_tool_call_response_sent_bytes: AtomicU64::new(0),
        }
    }
}

impl BackendStats {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Combined registry tracking health and statistics across all configured backends.
#[derive(Debug, Default, Clone)]
pub struct BackendStatsRegistry {
    pub inner: Arc<DashMap<String, Arc<BackendStats>>>,
}

impl BackendStatsRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Initializes backend stats entries at startup with status "waiting", 0 counters, and empty tools list.
    pub fn init_backends<I, S>(&mut self, backend_names: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in backend_names {
            self.inner.insert(name.as_ref().to_string(), Arc::new(BackendStats::new()));
        }
    }

    /// Gets or creates the stats/health tracker for a given backend name.
    pub fn get_or_create(&self, backend_name: &str) -> Arc<BackendStats> {
        self.inner
            .entry(backend_name.to_string())
            .or_insert_with(|| Arc::new(BackendStats::new()))
            .clone()
    }

    pub fn set_status(&self, name: &str, status: BackendStatus) {
        let stats = self.get_or_create(name);
        *stats.status.write().unwrap() = status;
    }

    pub fn get_info(&self, name: &str) -> Option<BackendHealthInfo> {
        self.inner.get(name).map(|v| {
            let stats = v.value();
            BackendHealthInfo {
                status: stats.status.read().unwrap().clone(),
                tools: stats.tools.read().unwrap().clone(),
                exposed_tools: stats.exposed_tools.read().unwrap().clone(),
            }
        })
    }

    pub fn set_info(&self, name: &str, status: BackendStatus, tools: Vec<String>, exposed_tools: Vec<String>) {
        let stats = self.get_or_create(name);
        *stats.status.write().unwrap() = status;
        *stats.tools.write().unwrap() = tools;
        *stats.exposed_tools.write().unwrap() = exposed_tools;
    }

    /// Increments pending requests and total requests for a backend.
    pub fn inc_request(&self, backend_name: &str) {
        let stats = self.get_or_create(backend_name);
        stats.pending_requests.fetch_add(1, Ordering::Relaxed);
        stats.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements pending requests for a backend upon request completion.
    pub fn dec_pending(&self, backend_name: &str) {
        if let Some(stats) = self.inner.get(backend_name) {
            let _ = stats.value().pending_requests.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |val| {
                if val > 0 { Some(val - 1) } else { Some(0) }
            });
        }
    }

    /// Records payload size received after a tool call to the backend.
    pub fn add_tool_call_received_bytes(&self, backend_name: &str, bytes: u64) {
        let stats = self.get_or_create(backend_name);
        stats.total_tool_call_received_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Records payload size sent in response to tool calls (transformed or original).
    pub fn add_tool_call_response_sent_bytes(&self, backend_name: &str, bytes: u64) {
        let stats = self.get_or_create(backend_name);
        stats.total_tool_call_response_sent_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Retrieves a snapshot of stats for all backends.
    pub fn get_all_stats(&self) -> Vec<(String, BackendStatsSnapshot)> {
        let mut result = Vec::new();
        for entry in self.inner.iter() {
            let name = entry.key().clone();
            let stats = entry.value();
            result.push((
                name,
                BackendStatsSnapshot {
                    pending_requests: stats.pending_requests.load(Ordering::Relaxed),
                    total_requests: stats.total_requests.load(Ordering::Relaxed),
                    total_tool_call_received_bytes: stats.total_tool_call_received_bytes.load(Ordering::Relaxed),
                    total_tool_call_response_sent_bytes: stats.total_tool_call_response_sent_bytes.load(Ordering::Relaxed),
                },
            ));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendHealthInfo {
    pub status: BackendStatus,
    pub tools: Vec<String>,
    pub exposed_tools: Vec<String>,
}

/// A read-only point-in-time snapshot of backend statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendStatsSnapshot {
    pub pending_requests: usize,
    pub total_requests: u64,
    pub total_tool_call_received_bytes: u64,
    pub total_tool_call_response_sent_bytes: u64,
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub version: String,
    pub status: String,
    pub backends: std::collections::HashMap<String, BackendHealthInfo>,
}

/// Axum route handler for returning proxy statistics at `/stats`.
pub async fn stats_handler(State(state): State<AppState>) -> impl IntoResponse {
    let stats = state.backend_stats.get_all_stats();
    let map: std::collections::HashMap<String, BackendStatsSnapshot> = stats.into_iter().collect();
    Json(map).into_response()
}
