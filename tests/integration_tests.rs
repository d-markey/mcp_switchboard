use mcp_switchboard::config::{AppConfig, RewriteMode};
use mcp_switchboard::transform_md_tables::try_convert_json_table;
use mcp_switchboard::transform_toon::try_convert_toon;
use serde_json::json;

#[test]
fn test_config_parsing() {
    let yaml = r#"
servers:
  test_server:
    url: http://localhost:9000/mcp
    description: Test Server
    rewrite: md_tables
    use_tool_search: true
cors:
  allow_origins:
    - https://example.com
"#;
    let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(config.servers.contains_key("test_server"));
    let backend = config.servers.get("test_server").unwrap();
    assert_eq!(backend.url, "http://localhost:9000/mcp");
    assert_eq!(backend.rewrite, Some(RewriteMode::MdTables));
    assert!(backend.use_tool_search);
    assert_eq!(config.cors.unwrap().allow_origins, vec!["https://example.com"]);
}

#[test]
fn test_table_rewrite_min_fill() {
    // 2 rows, 3 columns: a, b, c. Total 6 cells.
    // Row 1: a, b (2)
    // Row 2: c (1)
    // Total filled: 3. Ratio: 3/6 = 0.5. Should pass.
    let data = json!([
        {"a": 1, "b": 2},
        {"c": 3}
    ]);
    let text = serde_json::to_string(&data).unwrap();
    let result = try_convert_json_table(&text);
    assert!(result.is_some());
    let table = result.unwrap();
    assert!(table.contains("| a | b | c |"));
    assert!(table.contains("| 1 | 2 |  |"));
    assert!(table.contains("|  |  | 3 |"));

    // Below 0.5 ratio
    // 2 rows, 4 columns: a, b, c, d. Total 8 cells.
    // Row 1: a (1)
    // Row 2: b (1)
    // Total filled: 2. Ratio: 2/8 = 0.25. Should fail.
    let sparse_data = json!([
        {"a": 1},
        {"b": 2},
        {"c": 3},
        {"d": 4}
    ]);
    // 4 rows, 4 columns. Total 16 cells.
    // Row 1: a (1)
    // Row 2: b (1)
    // Row 3: c (1)
    // Row 4: d (1)
    // Total filled: 4. Ratio: 4/16 = 0.25. Should fail.
    let text = serde_json::to_string(&sparse_data).unwrap();
    let result = try_convert_json_table(&text);
    assert!(result.is_none());
}

#[test]
fn test_json_repair() {
    // Over-escaped newlines: literal \n instead of control character
    let text = r#"[{"a": 1}\n,{"a": 2}]"#;
    let result = try_convert_json_table(text);
    assert!(result.is_some());
    assert!(result.unwrap().contains("| 1 |"));
}

#[test]
fn test_toon_conversion() {
    let data = json!({
        "users": [
            {"id": 1, "name": "Alice"},
            {"id": 2, "name": "Bob"}
        ],
        "meta": {"count": 2}
    });
    let text = serde_json::to_string(&data).unwrap();
    let result = try_convert_toon(&text);
    assert!(result.is_some());
    let toon = result.unwrap();
    assert!(toon.contains("users[2]{id,name}:"));
    assert!(toon.contains("1,Alice"));
    assert!(toon.contains("2,Bob"));
    assert!(toon.contains("meta:"));
    assert!(toon.contains("count: 2"));
}
