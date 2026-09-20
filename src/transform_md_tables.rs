//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into dense GitHub Flavored Markdown (GFM) tables.
//!
//! Large JSON arrays are highly token-inefficient because object keys are repeated
//! on every single record. By translating the structure into a Markdown table, we declare the keys
//! exactly once as headers, eliminating syntax repetition and significantly extending
//! the usable window of any downstream LLM conversation.

use serde_json::Value;
use regex::Regex;
use std::sync::OnceLock;

static OVER_ESCAPED_NEWLINE: OnceLock<Regex> = OnceLock::new();
/// The required ratio of populated columns to ensure the payload is tabular.
/// We use 0.5 to make sure the dataset isn't overly sparse, which would look poor as a table.
const MIN_FILL_RATIO: f64 = 0.5;

/// Entry point that attempts to transform a string block into a GFM table layout.
/// Returns Some(String) if the shape matches the required criteria, or None if the text
/// should be passed downstream without any modifications.
pub fn try_convert_json_table(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Try parsing as standard JSON first.
    let val = if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        Some(v)
    } else {
        // Many backends emit double-escaped or raw newlines inside serialized text.
        // We attempt to repair these to remain resilient against uneven payload sanitization.
        try_repair_over_escaped_newlines(trimmed)
    };

    if let Some(v) = val {
        return convert_json_value_to_table(v);
    }

    // Try processing as JSON Lines (JSONL), which is common for log outputs or database dumps.
    if let Some(rows) = parse_as_jsonl(trimmed) {
        return format_table(&rows, None);
    }

    None
}

/// Normalizes raw or double-escaped newline symbols back into valid standard breaks.
fn try_repair_over_escaped_newlines(text: &str) -> Option<Value> {
    let re = OVER_ESCAPED_NEWLINE.get_or_init(|| Regex::new(r"\\r\\n|\\n").unwrap());
    let repaired = re.replace_all(text, "\n");
    if repaired == text {
        return None;
    }
    serde_json::from_str(&repaired).ok()
}

/// Evaluates the top-level structure of a JSON Value to determine if it is eligible for table compaction.
fn convert_json_value_to_table(val: Value) -> Option<String> {
    match val {
        Value::Array(arr) => {
            // Arrays must consist entirely of object records to form consistent table rows.
            if arr.is_empty() || !arr.iter().all(|v| v.is_object()) {
                return None;
            }
            let columns = get_all_keys(&arr);
            if !have_common_shape(&arr, &columns) {
                return None;
            }
            format_table_with_columns(&arr, columns, None)
        }
        Value::Object(mut map) => {
            // Frequently, a response wraps the tabular array inside a generic envelope wrapper
            // under fields like "result", "data", or "rows". We look for these known envelopes.
            let preferred_keys = ["result", "results", "data", "items", "rows", "content"];
            let mut table_key = None;

            for key in preferred_keys {
                if let Some(v) = map.get(key) {
                    if is_eligible_table_array(v) {
                        table_key = Some(key.to_string());
                        break;
                    }
                }
            }

            // Fallback: If no preferred key matches, but the envelope contains exactly one inner array,
            // we assume that single array is the primary target payload.
            if table_key.is_none() {
                let mut eligible_keys = Vec::new();
                for (k, v) in map.iter() {
                    if is_eligible_table_array(v) {
                        eligible_keys.push(k.clone());
                    }
                }
                if eligible_keys.len() == 1 {
                    table_key = Some(eligible_keys[0].clone());
                }
            }

            let key = table_key?;

            let table_val = map.remove(&key).unwrap();
            let rows = match table_val {
                Value::Array(arr) => arr,
                Value::String(s) => {
                    let inner = try_parse_json(&s)?;
                    inner.as_array()?.clone()
                }
                _ => unreachable!(),
            };

            let columns = get_all_keys(&rows);
            if !have_common_shape(&rows, &columns) {
                return None;
            }
            // Preserve the other metadata fields from the envelope underneath the table block
            // so that any supplementary tracking tags aren't lost completely.
            format_table_with_columns(&rows, columns, Some(Value::Object(map)))
        }
        _ => None,
    }
}

fn try_parse_json(s: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str(s) {
        return Some(v);
    }
    try_repair_over_escaped_newlines(s)
}

fn is_eligible_table_array(v: &Value) -> bool {
    match v {
        Value::Array(arr) => {
            if arr.is_empty() || !arr.iter().all(|item| item.is_object()) {
                return false;
            }
            let columns = get_all_keys(arr);
            have_common_shape(arr, &columns)
        }
        Value::String(s) => {
            if let Some(inner) = try_parse_json(s) {
                is_eligible_table_array(&inner)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Aggregates all distinct property keys across every object record to define the union table header.
fn get_all_keys(rows: &[Value]) -> Vec<String> {
    let mut all_keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for k in obj.keys() {
                if seen.insert(k.clone()) {
                    all_keys.push(k.clone());
                }
            }
        }
    }
    all_keys
}

/// Computes the ratio of populated fields to make sure the structure is uniform enough
/// to justify a table presentation, filtering out deeply nested irregular structures.
fn have_common_shape(rows: &[Value], columns: &[String]) -> bool {
    if rows.is_empty() || columns.is_empty() {
        return false;
    }
    let filled_cells: usize = rows.iter().filter_map(|r| r.as_object()).map(|o| o.len()).sum();
    let total_cells = rows.len() * columns.len();
    (filled_cells as f64 / total_cells as f64) >= MIN_FILL_RATIO
}

fn parse_as_jsonl(text: &str) -> Option<Vec<Value>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let val: Value = serde_json::from_str(trimmed).ok()?;
        if !val.is_object() {
            return None;
        }
        rows.push(val);
    }
    let columns = get_all_keys(&rows);
    if !rows.is_empty() && have_common_shape(&rows, &columns) {
        Some(rows)
    } else {
        None
    }
}

/// Generates the GFM text syntax, escaping inner pipes to prevent fracturing the markdown column layout.
fn format_table_with_columns(rows: &[Value], all_keys: Vec<String>, metadata: Option<Value>) -> Option<String> {
    let mut md = String::new();
    md.push('|');
    for k in &all_keys {
        md.push_str(&format!(" {} |", escape_pipe_n_newline(k)));
    }
    md.push('\n');

    md.push('|');
    for _ in &all_keys {
        md.push_str(" --- |");
    }
    md.push('\n');

    for row in rows {
        md.push('|');
        if let Some(obj) = row.as_object() {
            for k in &all_keys {
                let cell = obj.get(k).map(format_cell).unwrap_or_default();
                md.push_str(&format!(" {} |", cell));
            }
        }
        md.push('\n');
    }

    if let Some(meta) = metadata {
        if !is_empty_object(&meta) {
            md.push('\n');
            let meta_str = serde_json::to_string_pretty(&meta).unwrap_or_default();
            let fence = get_fence_for_content(&meta_str);
            md.push_str(&format!("{}json\n{}\n{}\n", fence, meta_str, fence));
        }
    }

    Some(md)
}

fn format_table(rows: &[Value], metadata: Option<Value>) -> Option<String> {
    let columns = get_all_keys(rows);
    format_table_with_columns(rows, columns, metadata)
}

fn is_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

/// Escapes standard Markdown structural indicators to prevent layout corruption inside table cells.
fn escape_pipe_n_newline(s: &str) -> String {
    s.replace('|', "\\|")
     .replace("\r\n", "<br>")
     .replace(['\n', '\r'], "<br>")
}

fn format_cell(val: &Value) -> String {
    match val {
        Value::Null => "".to_string(),
        Value::String(s) => {
            if s.trim().is_empty() {
                "".to_string()
            } else {
                escape_pipe_n_newline(s)
            }
        }
        Value::Array(a) => {
            if a.is_empty() {
                "".to_string()
            } else {
                serde_json::to_string(a).unwrap_or_default()
            }
        }
        Value::Object(m) => {
            if m.is_empty() {
                "".to_string()
            } else {
                serde_json::to_string(m).unwrap_or_default()
            }
        }
        _ => val.to_string(),
    }
}

/// Intelligently computes the number of backticks required to wrap metadata blocks safely,
/// preventing accidental fence termination if the inner JSON payload already contains backtick symbols.
fn get_fence_for_content(content: &str) -> String {
    let mut max_backticks = 0;
    let mut current_backticks = 0;
    for c in content.chars() {
        if c == '`' {
            current_backticks += 1;
        } else {
            if current_backticks > max_backticks {
                max_backticks = current_backticks;
            }
            current_backticks = 0;
        }
    }
    if current_backticks > max_backticks {
        max_backticks = current_backticks;
    }

    let n = std::cmp::max(3, max_backticks + 1);
    "`".repeat(n)
}
