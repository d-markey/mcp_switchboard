//! Core routing and interception dispatch for tool call response payloads.
//! This handles applying the requested compacting strategy to tool outputs,
//! helping optimize the downstream LLM's context size.

use serde_json::Value;
use crate::config::RewriteMode;
use crate::transform_toon::try_convert_toon;
use crate::transform_md_tables::try_convert_json_table;
use crate::transform_csv::try_convert_json_csv;
use crate::jsonrpc::{is_tool_call_response, rewrite_tool_call_result};

/// Evaluates if an incoming byte slice represents a valid tool response matching the tracked `request_id`.
/// If it matches and compaction is configured, it unpacks the structure, runs the appropriate formatting algorithm,
/// and re-serializes the data transparently. Returns a tuple of (rewritten_string, whether_transformed).
pub fn maybe_rewrite_json_body(body_bytes: &[u8], mode: RewriteMode, request_id: &Value) -> (String, bool) {
    // If the body is malformed or not JSON, we forward it untouched to avoid breaking downstream pipelines.
    let Ok(mut json_val) = serde_json::from_slice::<Value>(body_bytes) else {
        return (String::from_utf8_lossy(body_bytes).into_owned(), false);
    };

    // Ensure we are only modifying the matching tool result to preserve other protocol events.
    if !is_tool_call_response(&json_val, request_id) {
        return (String::from_utf8_lossy(body_bytes).into_owned(), false);
    }

    let changed = rewrite_tool_call_result(&mut json_val, |text| match mode {
        RewriteMode::MdTables => try_convert_json_table(text),
        RewriteMode::Toon => try_convert_toon(text),
        RewriteMode::Csv => try_convert_json_csv(text),
    });

    let s = if changed {
        serde_json::to_string(&json_val).unwrap_or_else(|_| String::from_utf8_lossy(body_bytes).into_owned())
    } else {
        String::from_utf8_lossy(body_bytes).into_owned()
    };
    (s, changed)
}

/// Asynchronously evaluates and rewrites the JSON body, offloading to `spawn_blocking` if the body size exceeds 4096 bytes.
pub async fn maybe_rewrite_json_body_async(
    full_bytes: &[u8],
    mode: RewriteMode,
    request_id: &Value,
) -> (String, bool) {
    if full_bytes.len() > 4096 {
        let request_id = request_id.clone();
        let bytes_for_task = full_bytes.to_vec();
        tokio::task::spawn_blocking(move || maybe_rewrite_json_body(&bytes_for_task, mode, &request_id))
            .await
            .unwrap_or_else(|_| (String::from_utf8_lossy(full_bytes).into_owned(), false))
    } else {
        maybe_rewrite_json_body(full_bytes, mode, &request_id)
    }
}

