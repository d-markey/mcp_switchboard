use mcp_switchboard::transform_csv::try_convert_json_csv;
use serde_json::json;

#[test]
fn test_csv_array_of_objects() {
    let payload = json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]).to_string();
    let csv = try_convert_json_csv(&payload).unwrap();
    assert_eq!(
        csv,
        "id\tname\n1\ta\n2\tb\n"
    );
}

#[test]
fn test_csv_non_informative_data() {
    let payload = json!([
        {"dict": {}, "id": 1, "list": [], "txt": "", "val": null},
        {"dict": {}, "id": 2, "list": [], "txt": "hello", "val": "world"}
    ]).to_string();
    let csv = try_convert_json_csv(&payload).unwrap();
    assert_eq!(
        csv,
        "dict\tid\tlist\ttxt\tval\n\t1\t\t\t\n\t2\t\thello\tworld\n"
    );
}

#[test]
fn test_csv_array_cell_formatting() {
    let payload = json!([{"id": 1, "tags": ["x", "y"]}]).to_string();
    let csv = try_convert_json_csv(&payload).unwrap();
    assert!(csv.contains("\"\"x\",\"y\"\"") || csv.contains("\"x,y\"") || csv.contains("\"\""));
}

#[test]
fn test_csv_string_escaping_and_quoting() {
    let payload = json!([
        {"note": "hello world"},
        {"note": "a,b"},
        {"note": "a;b"},
        {"note": "say \"hi\""},
        {"note": "line1\nline2\r\ntab\there"}
    ]).to_string();
    let csv = try_convert_json_csv(&payload).unwrap();
    assert!(csv.contains("\"hello world\""));
    assert!(csv.contains("\"a,b\""));
    assert!(csv.contains("\"a;b\""));
    assert!(csv.contains("\"say \"\"hi\"\"\""));
    assert!(csv.contains("\"line1\\nline2\\r\\ntab\\there\""));
}

#[test]
fn test_csv_jsonl() {
    let payload = format!("{}\n{}\n{}",
        json!({"id": 1, "name": "a"}),
        json!({"id": 2, "name": "b"}),
        json!({"id": 3, "name": "c"})
    );
    let csv = try_convert_json_csv(&payload).unwrap();
    assert_eq!(
        csv,
        "id\tname\n1\ta\n2\tb\n3\tc\n"
    );
}

#[test]
fn test_csv_envelope_with_metadata() {
    let payload = json!({
        "result": [{"id": 1, "name": "a"}],
        "metadata": {"page": 1}
    }).to_string();
    let csv = try_convert_json_csv(&payload).unwrap();
    assert!(csv.starts_with("id\tname\n1\ta\n"));
    assert!(csv.contains("\"metadata\": {"));
}

#[test]
fn test_csv_non_table_returns_none() {
    assert!(try_convert_json_csv(&json!({"a": 1, "b": "text"}).to_string()).is_none());
}
