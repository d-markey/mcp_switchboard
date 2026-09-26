//! Centralized tabular data detection, cleanup, and rendering module.
//!
//! Handles parsing arrays, JSONL, and wrapped envelope objects into tabular rows,
//! identifying uninformative cells (null, empty strings, empty arrays/objects, whitespace),
//! dropping uninformative rows and columns (where all cells are empty), and formatting
//! scalar values without enforcing strict roundtrip data-type quoting.

use serde_json::Value;

/// Represents a cleaned tabular dataset ready for formatting as Markdown, CSV, or TOON.
#[derive(Debug, Clone)]
pub struct TabularDataset {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub metadata: Option<Value>,
}

/// Attempts to extract a tabular dataset from a parsed JSON value or raw text (JSON / JSONL).
pub fn try_extract_tabular(text: &str, min_rows: usize, min_cols: usize) -> Option<TabularDataset> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(val) = crate::http_utils::try_parse_json_with_repair(trimmed) {
        if let Some(dataset) = extract_from_value(val, min_rows, min_cols) {
            return Some(dataset);
        }
    }

    if let Some(rows) = parse_as_jsonl_raw(trimmed) {
        return build_dataset_from_rows(rows, None, min_rows, min_cols);
    }

    None
}

fn extract_from_value(val: Value, min_rows: usize, min_cols: usize) -> Option<TabularDataset> {
    match val {
        Value::Array(arr) => {
            if arr.is_empty() || !arr.iter().all(|v| v.is_object()) {
                return None;
            }
            let columns = get_raw_keys(&arr);
            if !have_common_shape(&arr, &columns) {
                return None;
            }
            build_dataset_from_rows(arr, None, min_rows, min_cols)
        }
        Value::Object(mut map) => {
            let preferred_keys = ["result", "results", "data", "items", "rows", "content"];
            let mut table_key = None;

            for key in &preferred_keys {
                if let Some(v) = map.get(*key) {
                    if is_eligible_table_array(v) {
                        table_key = Some(key.to_string());
                        break;
                    }
                }
            }

            // Fallback: If no preferred key matches, but the map contains exactly one inner array,
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

            let Some(key) = table_key else {
                return None;
            };

            let table_val = map.remove(&key).unwrap();
            let rows = match table_val {
                Value::Array(arr) => arr,
                Value::String(s) => {
                    let inner = crate::http_utils::try_parse_json_with_repair(&s)?;
                    inner.as_array()?.clone()
                }
                _ => return None,
            };

            let columns = get_raw_keys(&rows);
            if !have_common_shape(&rows, &columns) {
                return None;
            }

            let metadata = if map.is_empty() { None } else { Some(Value::Object(map)) };
            build_dataset_from_rows(rows, metadata, min_rows, min_cols)
        }
        _ => None,
    }
}

fn is_eligible_table_array(v: &Value) -> bool {
    match v {
        Value::Array(arr) => {
            if arr.is_empty() || !arr.iter().all(|item| item.is_object()) {
                return false;
            }
            let columns = get_raw_keys(arr);
            have_common_shape(arr, &columns)
        }
        Value::String(s) => {
            if let Some(inner) = crate::http_utils::try_parse_json_with_repair(s) {
                is_eligible_table_array(&inner)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn get_raw_keys(rows: &[Value]) -> Vec<String> {
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
    // Allow single-row tables (common in tests/assertions where fill ratio might be 1.0)
    if rows.len() == 1 {
        return true;
    }
    let filled_cells: usize = rows.iter().filter_map(|r| r.as_object()).map(|o| o.len()).sum();
    let total_cells = rows.len() * columns.len();
    (filled_cells as f64 / total_cells as f64) >= 0.5
}

fn parse_as_jsonl_raw(text: &str) -> Option<Vec<Value>> {
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
    if rows.is_empty() {
        None
    } else {
        Some(rows)
    }
}

/// Checks if a value is uninformative (null, empty string, whitespace-only, empty array, empty object).
pub fn is_empty_cell(val: &Value) -> bool {
    match val {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(arr) => arr.is_empty() || arr.iter().all(is_empty_cell),
        Value::Object(map) => map.is_empty() || map.values().all(is_empty_cell),
        _ => false,
    }
}

/// Renders a JSON value to a clean string representation, collapsing uninformative cells to `""`
/// and avoiding unnecessary strict type-distinguishing quotes for LLMs.
pub fn format_cell_value(val: &Value) -> String {
    if is_empty_cell(val) {
        return "".to_string();
    }
    match val {
        Value::Null => "".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(arr) => {
            serde_json::to_string(arr).unwrap_or_default()
        }
        Value::Object(obj) => {
            serde_json::to_string(obj).unwrap_or_default()
        }
    }
}

fn build_dataset_from_rows(raw_rows: Vec<Value>, metadata: Option<Value>, min_rows: usize, _min_cols: usize) -> Option<TabularDataset> {
    if raw_rows.is_empty() {
        return None;
    }

    if !raw_rows.iter().all(|r| r.is_object()) {
        return None;
    }

    let all_keys = get_raw_keys(&raw_rows);
    if all_keys.is_empty() || !have_common_shape(&raw_rows, &all_keys) {
        return None;
    }

    // Build initial matrix of formatted strings
    let mut matrix: Vec<Vec<String>> = Vec::with_capacity(raw_rows.len());
    for row in &raw_rows {
        let mut formatted_row = Vec::with_capacity(all_keys.len());
        if let Some(obj) = row.as_object() {
            for k in &all_keys {
                let cell_val = obj.get(k).unwrap_or(&Value::Null);
                formatted_row.push(format_cell_value(cell_val));
            }
        } else {
            for _ in &all_keys {
                formatted_row.push("".to_string());
            }
        }
        matrix.push(formatted_row);
    }

    let kept_columns = all_keys;
    let kept_col_indices: Vec<usize> = (0..kept_columns.len()).collect();

    // Project matrix onto kept columns
    let mut filtered_matrix = Vec::new();
    for row in matrix {
        let filtered_row: Vec<String> = kept_col_indices.iter().map(|&idx| row[idx].clone()).collect();
        // Drop uninformative rows where every cell in that row is empty
        let is_row_empty = filtered_row.iter().all(|cell| cell.is_empty());
        if !is_row_empty {
            filtered_matrix.push(filtered_row);
        }
    }

    if filtered_matrix.is_empty() {
        return None;
    }

    let row_count = filtered_matrix.len();
    let col_count = kept_columns.len();

    if min_rows >= 2 {
        if row_count < min_rows {
            return None;
        }
    } else {
        if row_count == 1 && col_count < 2 {
            return None;
        }
        if row_count == 0 {
            return None;
        }
    }

    Some(TabularDataset {
        columns: kept_columns,
        rows: filtered_matrix,
        metadata,
    })
}
