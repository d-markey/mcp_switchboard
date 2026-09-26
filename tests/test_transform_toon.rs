use mcp_switchboard::transform_toon::try_convert_toon;
use serde_json::json;

#[test]
fn test_scalars() {
    assert_eq!(try_convert_toon("42").unwrap(), "42\n");
    assert_eq!(try_convert_toon("\"hello\"").unwrap(), "hello\n");
    assert_eq!(try_convert_toon("true").unwrap(), "true\n");
}

#[test]
fn test_simple_object() {
    let input = "{\"name\": \"Alice\", \"age\": 30}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("name: Alice"));
    assert!(output.contains("age: 30"));
}

#[test]
fn test_nested_object() {
    let input = "{\"user\": {\"name\": \"Bob\", \"active\": true}}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("user:"));
    assert!(output.contains("  name: Bob"));
    assert!(output.contains("  active: true"));
}

#[test]
fn test_tabular_array() {
    let input = "[{\"id\": 1, \"role\": \"admin\"}, {\"id\": 2, \"role\": \"user\"}]";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("[2]{id,role}:"));
    assert!(output.contains("1,admin"));
    assert!(output.contains("2,user"));
}

#[test]
fn test_tabular_array_with_key() {
    let input = "{\"users\": [{\"id\": 1, \"role\": \"admin\"}, {\"id\": 2, \"role\": \"user\"}]}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("users[2]{id,role}:"));
    assert!(output.contains("  1,admin"));
}

#[test]
fn test_scalar_list_with_key() {
    let input = "{\"tags\": [\"rust\", \"proxy\"]}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("tags[2]: rust,proxy"));
}

#[test]
fn test_non_tabular_array_scalars() {
    let input = "[1, 2, 3]";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("[3]:"));
    assert!(output.contains("1"));
}

#[test]
fn test_non_tabular_array_mixed() {
    let input = "[{\"a\": 1}, 2]";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("[2]:"));
}

#[test]
fn test_empty_collections() {
    assert_eq!(try_convert_toon("[]").unwrap(), "[0]:\n");
    assert_eq!(try_convert_toon("{}").unwrap(), "");
}

#[test]
fn test_jsonl_input() {
    let input = "{\"a\": 1}\n{\"a\": 2}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("[2]{a}:"));
    assert!(output.contains("1"));
    assert!(output.contains("2"));
}

#[test]
fn test_invalid_input() {
    assert!(try_convert_toon("invalid json").is_none());
}

#[test]
fn test_quoting_rules() {
    let input = "{\"msg\": \"hello: world, test\"}";
    let output = try_convert_toon(input).unwrap();
    assert!(output.contains("msg: \"hello: world, test\""));
}

#[test]
fn test_complex_nesting() {
    let input = serde_json::json!({
        "project": "mcp-switchboard",
        "stats": {
            "tests": 104,
            "passed": true
        },
        "files": [
            {"name": "main.rs", "size": 2800},
            {"name": "lib.rs", "size": 100}
        ],
        "tags": ["proxy", "rust"]
    }).to_string();

    let output = try_convert_toon(&input).unwrap();
    assert!(output.contains("project: mcp-switchboard"));
    assert!(output.contains("files[2]{name,size}:"));
    assert!(output.contains("tags[2]:"));
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
