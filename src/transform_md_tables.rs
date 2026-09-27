//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into dense GitHub Flavored Markdown (GFM) tables using the centralized tabular engine.

use serde_json::Value;
use crate::tabular::{try_extract_tabular, TabularDataset, is_empty_cell};

/// Entry point that attempts to transform a string block into a GFM table layout.
/// Markdown tables require at least 2 rows (`min_rows: 2`).
pub fn try_convert_json_table(text: &str) -> Option<String> {
    let dataset = try_extract_tabular(text, 2, 1, true)?;
    format_table_dataset(&dataset)
}

fn format_table_dataset(dataset: &TabularDataset) -> Option<String> {
    let mut md = String::new();
    md.push('|');
    for k in &dataset.columns {
        md.push_str(&format!(" {} |", escape_pipe_n_newline(&k)));
    }
    md.push('\n');

    md.push('|');
    for _ in &dataset.columns {
        md.push_str(" --- |");
    }
    md.push('\n');

    for row in &dataset.rows {
        md.push('|');
        for cell in row {
            let val_str = format_cell_value(cell);
            let escaped = escape_pipe_n_newline(&val_str);
            if escaped.is_empty() {
                md.push_str(" |");
            } else {
                md.push_str(&format!(" {} |", escaped));
            }
        }
        md.push('\n');
    }

    if let Some(meta) = &dataset.metadata {
        if !is_empty_object(meta) {
            md.push('\n');
            md.push_str(&serde_json::to_string(meta).unwrap_or_default());
        }
    }

    Some(md)
}

fn format_cell_value(val: &Value) -> String {
    if is_empty_cell(val) {
        return "".to_string();
    }
    match val {
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
    }
}

fn is_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn escape_pipe_n_newline(s: &str) -> String {
    s.replace('|', "\\|")
     .replace('\t', "\\t")
     .replace('\r', "\\r")
     .replace('\n', "\\n")
}
