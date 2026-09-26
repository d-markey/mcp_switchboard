//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into CSV (Tab-Separated Values) format.

use serde_json::Value;
use regex::Regex;
use std::sync::OnceLock;

static OVER_ESCAPED_NEWLINE: OnceLock<Regex> = OnceLock::new();
const MIN_FILL_RATIO: f64 = 0.5;

/// Entry point that attempts to transform a string block into a CSV (TSV) layout.
/// Returns Some(String) if the shape matches the required criteria, or None if the text
/// should be passed downstream without any modifications.
pub fn try_convert_json_csv(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let val = crate::http_utils::try_parse_json_with_repair(trimmed);

    if let Some(v) = val {
        return convert_json_value_to_csv(v);
    }

    if let Some(rows) = parse_as_jsonl(trimmed) {
        return format_csv(&rows, None);
    }

    None
}



fn convert_json_value_to_csv(val: Value) -> Option<String> {
    match val {
        Value::Array(arr) => {
            if arr.is_empty() || !arr.iter().all(|v| v.is_object()) {
                return None;
            }
            let columns = get_all_keys(&arr);
            if !have_common_shape(&arr, &columns) {
                return None;
            }
            format_csv_with_columns(&arr, columns, None)
        }
        Value::Object(mut map) => {
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
            format_csv_with_columns(&rows, columns, Some(Value::Object(map)))
        }
        _ => None,
    }
}

fn try_parse_json(s: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        Some(v)
    } else {
        let re = OVER_ESCAPED_NEWLINE.get_or_init(|| Regex::new(r"\\r\\n|\\n").unwrap());
        let repaired = re.replace_all(s, "\n");
        serde_json::from_str(&repaired).ok()
    }
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

fn format_csv_with_columns(rows: &[Value], all_keys: Vec<String>, metadata: Option<Value>) -> Option<String> {
    let mut csv = String::new();

    // Header row
    let header_cells: Vec<String> = all_keys.iter().map(|k| format_cell(&Value::String(k.clone()))).collect();
    csv.push_str(&header_cells.join("\t"));
    csv.push('\n');

    // Data rows
    for row in rows {
        let mut row_cells = Vec::new();
        if let Some(obj) = row.as_object() {
            for k in &all_keys {
                let cell_str = obj.get(k).map(format_cell).unwrap_or_default();
                row_cells.push(cell_str);
            }
        } else {
            for _ in &all_keys {
                row_cells.push("".to_string());
            }
        }
        csv.push_str(&row_cells.join("\t"));
        csv.push('\n');
    }

    if let Some(meta) = metadata {
        if !is_empty_object(&meta) {
            csv.push('\n');
            let meta_str = serde_json::to_string_pretty(&meta).unwrap_or_default();
            let fence = get_fence_for_content(&meta_str);
            csv.push_str(&format!("{}json\n{}\n{}\n", fence, meta_str, fence));
        }
    }

    Some(csv)
}

fn format_csv(rows: &[Value], metadata: Option<Value>) -> Option<String> {
    let columns = get_all_keys(rows);
    format_csv_with_columns(rows, columns, metadata)
}

fn is_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn format_cell(val: &Value) -> String {
    match val {
        Value::Null => "".to_string(),
        Value::String(s) => {
            if s.trim().is_empty() {
                "".to_string()
            } else {
                format_string_cell(s)
            }
        }
        Value::Array(a) => {
            if a.is_empty() {
                "".to_string()
            } else {
                let json_str = serde_json::to_string(val).unwrap_or_default();
                let inner = if json_str.starts_with('[') && json_str.ends_with(']') && json_str.len() >= 2 {
                    &json_str[1..json_str.len() - 1]
                } else {
                    &json_str
                };
                format_string_cell(inner)
            }
        }
        Value::Object(m) => {
            if m.is_empty() {
                "".to_string()
            } else {
                let json_str = serde_json::to_string(val).unwrap_or_default();
                format_string_cell(&json_str)
            }
        }
        _ => {
            let s = val.to_string();
            format_string_cell(&s)
        }
    }
}

fn format_string_cell(s: &str) -> String {
    let needs_quote = needs_quoting(s);

    // First, convert CR, LF and TAB characters to escaped sequences \r, \n, \t
    let escaped = s
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t");

    // Then, if the string contains a '"', a ',', a ';', or a white space:
    // quote the string and replace '"' with '""' as per CSV spec.
    if needs_quote {
        format!("\"{}\"", escaped.replace('"', "\"\""))
    } else {
        escaped
    }
}

fn needs_quoting(text: &str) -> bool {
    text.chars().any(|c| c == '"' || c == ',' || c == ';' || c.is_whitespace())
}

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
