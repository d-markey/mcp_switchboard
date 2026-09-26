//! Compaction strategy that rewrites JSON or JSONL arrays of structured objects
//! into CSV (Tab-Separated Values) format using the centralized tabular engine.

use serde_json::Value;
use crate::tabular::{try_extract_tabular, TabularDataset};

/// Entry point that attempts to transform a string block into a CSV (TSV) layout.
/// CSV requires at least 2 rows, or 1 row with at least 2 columns.
pub fn try_convert_json_csv(text: &str) -> Option<String> {
    let dataset = try_extract_tabular(text, 1, 2)?;
    format_csv_dataset(&dataset)
}

fn format_csv_dataset(dataset: &TabularDataset) -> Option<String> {
    let mut csv = String::new();

    // Header row
    let header_cells: Vec<String> = dataset.columns.iter().map(|k| format_string_cell(k)).collect();
    csv.push_str(&header_cells.join("\t"));
    csv.push('\n');

    // Data rows
    for row in &dataset.rows {
        let row_cells: Vec<String> = row.iter().map(|cell| format_string_cell(cell)).collect();
        csv.push_str(&row_cells.join("\t"));
        csv.push('\n');
    }

    if let Some(meta) = &dataset.metadata {
        if !is_empty_object(meta) {
            csv.push('\n');
            let meta_str = serde_json::to_string_pretty(meta).unwrap_or_default();
            let fence = get_fence_for_content(&meta_str);
            csv.push_str(&format!("{}json\n{}\n{}\n", fence, meta_str, fence));
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

fn format_string_cell(s: &str) -> String {
    let needs_quote = needs_quoting(s);

    let escaped = s
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t");

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
