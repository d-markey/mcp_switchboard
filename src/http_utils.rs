//! HTTP and JSON utility helpers for MIME type inspection and JSON payload repair.

use serde_json::Value;

/// Hop-by-hop headers that should not be forwarded to upstreams per RFC 2616 specification rules.
pub const EXCLUDED_HEADERS: &[&str] = &[
    "content-length",
    "content-encoding",
    "connection",
    "transfer-encoding",
    "host",
];

/// Checks if a header name (case-insensitive) is a hop-by-hop header that should be excluded.
pub fn is_excluded_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXCLUDED_HEADERS.contains(&lower.as_str())
}

/// Checks if a Content-Type header value indicates JSON data.
pub fn is_json_content_type(content_type: &str) -> bool {
    content_type.to_lowercase().contains("application/json")
}

/// Checks if a Content-Type header value indicates Server-Sent Events (SSE).
pub fn is_sse_content_type(content_type: &str) -> bool {
    content_type.to_lowercase().contains("text/event-stream")
}

/// Attempts to repair over-escaped or malformed escape sequences outside of strings
/// (such as `\n`, `\r`, `\t`, `\"` at the structural level) character-by-character.
/// Allocation-friendly: allocates a `String` buffer only upon encountering the first repairable sequence.
pub fn try_repair_over_escaped_newlines(text: &str) -> Option<Value> {
    let mut chars = text.char_indices().peekable();
    let mut in_string = false;
    let mut escaped = false;
    let mut repaired: Option<String> = None;

    while let Some((i, c)) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            if c == '"' {
                in_string = true;
            } else if c == '\\' {
                // Found a backslash outside a string! Check the next character.
                if let Some(&(_, next_c)) = chars.peek() {
                    let replacement = match next_c {
                        'r' => Some('\r'),
                        'n' => Some('\n'),
                        't' => Some('\t'),
                        _ => None,
                    };

                    if let Some(rep) = replacement {
                        // First repairable sequence: initialize allocated String buffer
                        let buf = repaired.get_or_insert_with(|| {
                            let mut s = String::with_capacity(text.len());
                            s.push_str(&text[..i]);
                            s
                        });
                        buf.push(rep);
                        // Consume the next character (`next_c`)
                        chars.next();
                        continue;
                    } else {
                        // Unexpected backslash followed by non-repairable char outside string
                        return None;
                    }
                } else {
                    // Trailing backslash at the end of input
                    return None;
                }
            }
        }

        // If we have an active repaired buffer, push the current character
        if let Some(buf) = &mut repaired {
            buf.push(c);
        }
    }

    let final_text = repaired?;
    serde_json::from_str(&final_text).ok()
}

/// Attempts to parse JSON, falling back to over-escaped newline repair if initial parsing fails.
pub fn try_parse_json_with_repair(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        Some(v)
    } else {
        try_repair_over_escaped_newlines(text)
    }
}
