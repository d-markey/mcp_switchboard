//! Compactor that reformats raw JSON or JSONL content into the concise TOON format notation.
//!
//! TOON minimizes character overhead by stripping noisy standard braces, quotation punctuation, and repetitive structural
//! separators where possible, or compiling arrays into standard comma-separated sequences. This lowers context utilization
//! without losing semantic object structure visibility for the LLM.

use serde_json::Value;

const INDENT: &str = "  ";

/// Return a TOON rendering of `text` if it is valid JSON or JSONL.
///
/// Returns None when `text` is neither, in which case the caller should
/// forward the original text unchanged.
pub fn try_convert_toon(text: &str) -> Option<String> {
    let data = parse(text)?;
    let mut lines = Vec::new();
    encode_root(&data, &mut lines);

    if lines.is_empty() {
        Some("".to_string())
    } else {
        let mut out = lines.join("\n");
        out.push('\n');
        Some(out)
    }
}

fn parse(text: &str) -> Option<Value> {
    let stripped = text.trim();
    if stripped.is_empty() {
        return None;
    }

    if let Some(val) = crate::http_utils::try_parse_json_with_repair(stripped) {
        return Some(val);
    }

    parse_jsonl(stripped)
}

fn parse_jsonl(text: &str) -> Option<Value> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }

    let mut rows = Vec::new();
    for line in lines {
        let val: Value = serde_json::from_str(line).ok()?;
        rows.push(val);
    }
    Some(Value::Array(rows))
}

fn encode_root(data: &Value, lines: &mut Vec<String>) {
    match data {
        Value::Object(obj) => {
            encode_object_fields(obj, 0, lines);
        }
        Value::Array(arr) => {
            encode_list_body("", arr, 0, lines);
        }
        _ => {
            lines.push(scalar(data));
        }
    }
}

fn encode_object_fields(obj: &serde_json::Map<String, Value>, depth: usize, lines: &mut Vec<String>) {
    for (key, value) in obj {
        encode_pair(&format_key(key), value, depth, lines);
    }
}

fn encode_pair(key: &str, value: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = INDENT.repeat(depth);
    match value {
        Value::Object(obj) => {
            if obj.is_empty() {
                lines.push(format!("{}: {{}}", indent + key));
            } else {
                lines.push(format!("{}:", indent + key));
                encode_object_fields(obj, depth + 1, lines);
            }
        }
        Value::Array(arr) => {
            encode_list_body(key, arr, depth, lines);
        }
        _ => {
            lines.push(format!("{}: {}", indent + key, scalar(value)));
        }
    }
}

fn encode_list_body(key: &str, items: &[Value], depth: usize, lines: &mut Vec<String>) {
    let indent = INDENT.repeat(depth);
    let prefix = if key.is_empty() { "".to_string() } else { key.to_string() };

    if items.is_empty() {
        let suffix = if prefix.is_empty() { "[0]:" } else { &format!("{}[0]:", prefix) };
        lines.push(format!("{}{}", indent, suffix));
        return;
    }

    if let Some(columns) = tabular_columns(items) {
        let header_parts: Vec<String> = columns.iter().map(|c| format_key(c)).collect();
        let header = format!("{{{}}}", header_parts.join(","));
        let lead = if prefix.is_empty() {
            format!("[{}]{}:", items.len(), header)
        } else {
            format!("{}[{}]{}:", prefix, items.len(), header)
        };
        lines.push(format!("{}{}", indent, lead));

        let row_indent = INDENT.repeat(depth + 1);
        for item in items {
            if let Some(obj) = item.as_object() {
                let row_values: Vec<String> = columns.iter()
                    .map(|col| scalar(obj.get(col).unwrap_or(&Value::Null)))
                    .collect();
                lines.push(format!("{}{}", row_indent, row_values.join(",")));
            }
        }
        return;
    }

    if items.iter().all(is_scalar) {
        let lead = if prefix.is_empty() {
            format!("[{}]:", items.len())
        } else {
            format!("{}[{}]:", prefix, items.len())
        };
        let values: Vec<String> = items.iter().map(scalar).collect();
        lines.push(format!("{}{} {}", indent, lead, values.join(",")));
        return;
    }

    let lead = if prefix.is_empty() {
        format!("[{}]:", items.len())
    } else {
        format!("{}[{}]:", prefix, items.len())
    };
    lines.push(format!("{}{}", indent, lead));
    for item in items {
        encode_list_item(item, depth + 1, lines);
    }
}

fn encode_list_item(item: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = INDENT.repeat(depth);
    match item {
        Value::Object(obj) if !obj.is_empty() => {
            let mut item_lines = Vec::new();
            encode_object_fields(obj, depth + 1, &mut item_lines);
            if let Some(first) = item_lines.first() {
                lines.push(format!("{}- {}", indent, first.trim_start()));
                lines.extend(item_lines.iter().skip(1).cloned());
            }
        }
        Value::Array(arr) => {
            let mut item_lines = Vec::new();
            encode_list_body("", arr, depth + 1, &mut item_lines);
            if let Some(first) = item_lines.first() {
                lines.push(format!("{}- {}", indent, first.trim_start()));
                lines.extend(item_lines.iter().skip(1).cloned());
            } else {
                lines.push(format!("{}-", indent));
            }
        }
        _ => {
            lines.push(format!("{}- {}", indent, scalar(item)));
        }
    }
}

fn is_scalar(value: &Value) -> bool {
    !value.is_object() && !value.is_array()
}

fn tabular_columns(items: &[Value]) -> Option<Vec<String>> {
    if items.is_empty() || !items.iter().all(|v| v.is_object()) {
        return None;
    }

    let first_obj = items[0].as_object().unwrap();
    let first_keys: Vec<String> = first_obj.keys().cloned().collect();
    let key_set: std::collections::HashSet<_> = first_keys.iter().collect();

    for item in items {
        let obj = item.as_object().unwrap();
        if obj.len() != key_set.len() || obj.keys().any(|k| !key_set.contains(k)) {
            return None;
        }
        if obj.values().any(|v| !is_scalar(v)) {
            return None;
        }
    }

    Some(first_keys)
}

fn format_key(key: &str) -> String {
    if needs_quoting(key) {
        serde_json::to_string(key).unwrap_or_else(|_| key.to_string())
    } else {
        key.to_string()
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if needs_quoting(s) {
                serde_json::to_string(s).unwrap_or_else(|_| s.to_string())
            } else {
                s.clone()
            }
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn needs_quoting(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    if text.trim() != text {
        return true;
    }
    match text {
        "true" | "false" | "null" => return true,
        _ => {}
    }
    if text.chars().any(|c| ",:{}[]\"\n".contains(c)) {
        return true;
    }

    let first = text.chars().next().unwrap();
    if (first == '-' || first.is_ascii_digit()) && looks_numeric(text) {
        return true;
    }

    false
}

fn looks_numeric(text: &str) -> bool {
    text.parse::<f64>().is_ok()
}
