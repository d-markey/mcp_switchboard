use mcp_switchboard::backend_stats::BackendStatsRegistry;
use std::sync::Arc;
use std::thread;

#[test]
fn test_stats_registry_basic_counters() {
    let mut registry = BackendStatsRegistry::new();
    registry.init_backends(vec!["test_backend"]);
    let backend = "test_backend";

    assert_eq!(registry.get_all_stats().len(), 1);

    registry.inc_request(backend);
    registry.inc_request(backend);

    let stats_list = registry.get_all_stats();
    assert_eq!(stats_list.len(), 1);
    assert_eq!(stats_list[0].0, backend);
    assert_eq!(stats_list[0].1.pending_requests, 2);
    assert_eq!(stats_list[0].1.total_requests, 2);
}

#[test]
fn test_stats_payload_sizes_and_pending() {
    let mut registry = BackendStatsRegistry::new();
    registry.init_backends(vec!["api_server"]);
    let backend = "api_server";

    registry.inc_request(backend);
    registry.inc_request(backend);
    registry.dec_pending(backend);

    registry.add_tool_call_received_bytes(backend, 500);
    registry.add_tool_call_received_bytes(backend, 250);

    registry.add_tool_call_response_sent_bytes(backend, 300);

    let snapshots = registry.get_all_stats();
    assert_eq!(snapshots.len(), 1);
    let (_, stats) = snapshots[0];

    assert_eq!(stats.pending_requests, 1);
    assert_eq!(stats.total_requests, 2);
    assert_eq!(stats.total_tool_call_received_bytes, 750);
    assert_eq!(stats.total_tool_call_response_sent_bytes, 300);
}

#[test]
fn test_concurrent_stats_updates() {
    let mut registry = BackendStatsRegistry::new();
    registry.init_backends(vec!["concurrent_backend"]);
    let registry = Arc::new(registry);
    let backend = "concurrent_backend";

    let mut handles = vec![];
    for _ in 0..10 {
        let reg = registry.clone();
        let b = backend.to_string();
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                reg.inc_request(&b);
                reg.add_tool_call_received_bytes(&b, 10);
                reg.add_tool_call_response_sent_bytes(&b, 5);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let snapshots = registry.get_all_stats();
    assert_eq!(snapshots.len(), 1);
    let (_, stats) = snapshots[0];
    assert_eq!(stats.total_requests, 1000);
    assert_eq!(stats.total_tool_call_received_bytes, 10000);
    assert_eq!(stats.total_tool_call_response_sent_bytes, 5000);
}
