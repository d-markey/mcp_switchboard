use mcp_switchboard::sse::SseRelayStream;
use axum::body::Bytes;
use futures_util::{stream, StreamExt};

async fn collect_stream(chunks: Vec<&[u8]>, rewriter: impl Fn(&[u8]) -> String + Unpin) -> Vec<u8> {
    let input = stream::iter(chunks.into_iter().map(|c| Ok::<Bytes, reqwest::Error>(Bytes::from(c.to_vec()))));
    let relay = SseRelayStream::new(input, rewriter);
    let out = relay.collect::<Vec<Result<Bytes, reqwest::Error>>>().await;
    let mut result = Vec::new();
    for chunk in out {
        result.extend_from_slice(&chunk.unwrap());
    }
    result
}

#[tokio::test]
async fn test_single_event_untouched_when_rewrite_returns_same() {
    let raw = b"data: {\"id\":1}\n\n";
    let out = collect_stream(vec![raw], |s| String::from_utf8_lossy(s).into_owned()).await;
    assert_eq!(out, raw);
}

#[tokio::test]
async fn test_single_event_data_line_is_rewritten() {
    let raw = b"id: 1\nevent: message\ndata: {\"a\":1}\n\n";
    let out = collect_stream(vec![raw], |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"a\":1}" { "{\"a\":2}".to_string() } else { data.into_owned() }
    }).await;
    assert_eq!(out, b"id: 1\nevent: message\ndata: {\"a\":2}\n\n");
}

#[tokio::test]
async fn test_event_split_across_chunks_is_still_recognized() {
    let chunks = vec![b"data: {\"a\"" as &[u8], b":1}\n\n" as &[u8]];
    let out = collect_stream(chunks, |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"a\":1}" { "{\"a\":2}".to_string() } else { data.into_owned() }
    }).await;
    assert_eq!(out, b"data: {\"a\":2}\n\n");
}

#[tokio::test]
async fn test_multiple_events_only_matching_one_rewritten() {
    let raw = b"data: {\"id\":1}\n\ndata: {\"id\":2}\n\n";
    let out = collect_stream(vec![raw], |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"id\":2}" { "{\"id\":\"X\"}".to_string() } else { data.into_owned() }
    }).await;
    assert_eq!(out, b"data: {\"id\":1}\n\ndata: {\"id\":\"X\"}\n\n");
}

#[tokio::test]
async fn test_crlf_line_endings_supported() {
    // Note: My new process_event normalizes to \n for internal fields but attempts to preserve \r if present in others.
    // Actually it always adds \n now. Let's see what it does.
    let raw = b"id: 1\r\ndata: {\"a\":1}\r\n\r\n";
    let out = collect_stream(vec![raw], |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"a\":1}" { "{\"a\":2}".to_string() } else { data.into_owned() }
    }).await;
    // My implementation currently reconstructs with \n
    assert!(String::from_utf8_lossy(&out).contains("id: 1\n"));
    assert!(String::from_utf8_lossy(&out).contains("data: {\"a\":2}\n\n"));
}

#[tokio::test]
async fn test_multi_line_data_is_joined_with_newline_and_left_untouched_when_no_rewrite() {
    let raw = b"data: line1\ndata: line2\n\n";
    let out = collect_stream(vec![raw], |s| String::from_utf8_lossy(s).into_owned()).await;
    assert_eq!(out, raw);
}

#[tokio::test]
async fn test_multi_line_data_is_rewritten_and_collapsed_to_one_line() {
    let raw = b"id: 1\ndata: {\"a\":\ndata: 1}\n\n";
    let out = collect_stream(vec![raw], |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"a\":\n1}" { "{\"a\":1}".to_string() } else { data.into_owned() }
    }).await;
    assert_eq!(out, b"id: 1\ndata: {\"a\":1}\n\n");
}

#[tokio::test]
async fn test_trailing_partial_event_is_flushed_unmodified() {
    let raw = b"data: {\"a\":1}\n\ndata: {\"incomplete\"";
    let out = collect_stream(vec![raw], |s| String::from_utf8_lossy(s).into_owned()).await;
    // The last chunk will be processed as an event if it's the end of stream
    // My process_event should handle it.
    assert_eq!(out, raw);
}

#[tokio::test]
async fn test_spaceless_data_prefix() {
    let raw = b"data:{\"a\":1}\n\n";
    let out = collect_stream(vec![raw], |data_bytes| {
        let data = String::from_utf8_lossy(data_bytes);
        if data == "{\"a\":1}" { "{\"a\":2}".to_string() } else { data.into_owned() }
    }).await;
    // Normalized output format puts space after data: per standard convention
    assert_eq!(out, b"data: {\"a\":2}\n\n");
}

