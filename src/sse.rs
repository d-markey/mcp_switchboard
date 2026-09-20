//! Server-Sent Events (SSE) streaming proxy relay.
//! This module implements standard buffering and un-framing of reactive event streams,
//! allowing Switchboard to rewrite stream payloads chunk-by-chunk in real time.

use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use axum::body::Bytes;

/// Custom stream wrapper that hooks into downstream SSE providers.
/// It buffers chunked data blocks to re-assemble separate `data: ...` event text sequences
/// completely before invoking a translation closure over them.
pub struct SseRelayStream<S, F> {
    inner: S,
    rewriter: F,
    buffer: Vec<u8>,
}

impl<S, F> SseRelayStream<S, F> {
    pub fn new(inner: S, rewriter: F) -> Self {
        Self {
            inner,
            rewriter,
            buffer: Vec::new(),
        }
    }
}

impl<S, F> Stream for SseRelayStream<S, F>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
    F: Fn(&[u8]) -> String + Unpin,
{
    type Item = Result<Bytes, reqwest::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    self.buffer.extend_from_slice(&chunk);

                    let mut output = Vec::new();
                    let mut start = 0;

                    // Locate separate event frames within the aggregated binary buffer.
                    while let Some(end) = find_event_end(&self.buffer[start..]) {
                        let event_end = start + end;
                        let event_data = &self.buffer[start..event_end];

                        let processed_event = process_event(event_data, &self.rewriter);
                        output.extend_from_slice(&processed_event);

                        start = event_end;
                    }

                    // Remove processed chunks to conserve memory.
                    if start > 0 {
                        self.buffer.drain(0..start);
                    }

                    if !output.is_empty() {
                        return Poll::Ready(Some(Ok(Bytes::from(output))));
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    // Flush out any dangling data fragments at stream termination.
                    if !self.buffer.is_empty() {
                        let last = self.buffer.split_off(0);
                        let processed = process_event(&last, &self.rewriter);
                        return Poll::Ready(Some(Ok(Bytes::from(processed))));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Identifies event demarcations according to the official SSE protocol specification (`\n\n` or `\r\n\r\n`).
fn find_event_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len() {
        if buf[i..].starts_with(b"\n\n") {
            return Some(i + 2);
        }
        if buf[i..].starts_with(b"\r\n\r\n") {
            return Some(i + 4);
        }
    }
    None
}

/// Splits the SSE frames apart, extracts the payload data field lines, executes the rewriter,
/// and packages the results back together into valid standard SSE wire format.
fn process_event<F>(event: &[u8], rewriter: &F) -> Vec<u8>
where
    F: Fn(&[u8]) -> String,
{
    let mut data_payload = String::new();
    let mut has_data = false;

    let event_str = String::from_utf8_lossy(event);
    let mut lines = Vec::new();

    for line in event_str.split('\n') {
        let mut trimmed = line;
        if trimmed.ends_with('\r') {
            trimmed = &trimmed[..trimmed.len() - 1];
        }
        if let Some(stripped) = trimmed.strip_prefix("data: ") {
            if has_data {
                data_payload.push('\n');
            }
            data_payload.push_str(stripped);
            has_data = true;
        } else {
            lines.push(trimmed);
        }
    }

    // If this specific frame isn't a data packet (e.g. an initialization comment or heartbeat ping),
    // forward it untouched to prevent breaking protocol sync.
    if !has_data {
        return event.to_vec();
    }

    let rewritten_data = rewriter(data_payload.as_bytes());
    if rewritten_data == data_payload {
        return event.to_vec();
    }

    let mut out = String::new();
    let ends_with_two_newlines = event.ends_with(b"\n\n") || event.ends_with(b"\r\n\r\n")
        || event.ends_with(b"\n\r\n") || event.ends_with(b"\r\n\n");

    for line in lines {
        if line.is_empty() && !ends_with_two_newlines {
             continue;
        }
        if !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }

    for data_line in rewritten_data.lines() {
        out.push_str("data: ");
        out.push_str(data_line);
        out.push('\n');
    }

    if ends_with_two_newlines {
        out.push('\n');
    }

    out.into_bytes()
}
