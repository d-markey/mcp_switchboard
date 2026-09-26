//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into dense GitHub Flavored Markdown (GFM) tables using the centralized tabular engine.

use serde_json::Value;
use crate::tabular::{try_extract_tabular, TabularDataset};

/// Entry point that attempts to transform a string block into a GFM table layout.
/// Markdown tables require at least 2 rows (`min_rows: 2`).
pub fn try_convert_json_table(text: &str) -> Option<String> {
    let dataset = try_extract_tabular(text, 2, 1)?;
    format_table_dataset(&dataset)
}

fn format_table_dataset(dataset: &TabularDataset) -> Option<String> {
    let mut md = String::new();
    md.push('|');
    for k in &dataset.columns {
        md.push_str(&format!(" {} |", escape_pipe_n_newline(k)));
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
            let val = escape_pipe_n_newline(cell);
            if val.is_empty() {
                md.push_str(" |");
            } else {
                md.push_str(&format!(" {} |", val));
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

fn is_empty_object(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn escape_pipe_n_newline(s: &str) -> String {
    s.replace('|', "\\|")
     .replace("\r\n", "\\n")
     .replace('\r', "\\n")
     .replace('\n', "\\n")
}
