//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into CSV (Tab-Separated Values) format using the centralized tabular engine.

use serde_json::Value;
use crate::tabular::{try_extract_tabular, TabularDataset, is_empty_cell};

/// Entry point that attempts to transform a string block into a CSV (TSV) layout.
/// CSV requires at least 2 rows, or 1 row with at least 2 columns.
pub fn try_convert_json_csv(text: &str) -> Option<String> {
    let dataset = try_extract_tabular(text, 1, 2, true)?;
    format_csv_dataset(&dataset)
}

fn format_csv_dataset(dataset: &TabularDataset) -> Option<String> {
    let mut csv = String::new();

    // Header row
    let header_cells: Vec<String> = dataset.columns.iter().map(|k| format_string_cell_from_value(&serde_json::Value::String(k.clone()))).collect();
    csv.push_str(&header_cells.join("\t"));
    csv.push('\n');

    // Data rows
    for row in &dataset.rows {
        let row_cells: Vec<String> = row.iter().map(|cell| format_string_cell_from_value(cell)).collect();
        csv.push_str(&row_cells.join("\t"));
        csv.push('\n');
    }

    if let Some(meta) = &dataset.metadata {
        if !is_empty_object(meta) {
            csv.push('\n');
            csv.push_str(&serde_json::to_string_pretty(meta).unwrap_or_default());
        }
    }

    Some(csv)
}

fn is_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn format_string_cell_from_value(val: &Value) -> String {
    if is_empty_cell(val) {
        return "".to_string();
    }
    let s = match val {
        Value::Null => "".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(arr) => {
            let json_str = serde_json::to_string(arr).unwrap_or_default();
            if json_str.starts_with('[') && json_str.ends_with(']') {
                let trimmed = &json_str[1..json_str.len()-1];
                trimmed.trim().to_string()
            } else {
                json_str
            }
        }
        Value::Object(obj) => {
            serde_json::to_string(obj).unwrap_or_default()
        }
    };

    let escaped = s;

    if needs_quoting(&escaped) {
        format!("\"{}\"", escaped.replace('"', "\"\""))
    } else {
        escaped
    }
}

fn needs_quoting(text: &str) -> bool {
    text.chars().any(|c| c == '"' || c == ',' || c == ';' || c.is_whitespace() || c == '\n' || c == '\r' || c == '\t')
}
