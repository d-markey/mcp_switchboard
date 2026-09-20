use mcp_switchboard::transform_md_tables::try_convert_json_table;
use serde_json::json;

#[test]
fn test_level0_array_of_objects() {
    let payload = json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(
        markdown,
        "| id | name |\n| --- | --- |\n| 1 | a |\n| 2 | b |\n"
    );
}

#[test]
fn test_level1_object_with_result_array_and_metadata() {
    let payload = json!({
        "result": [{"id": 1, "name": "a"}, {"id": 2, "name": "b"}],
        "metadata": {"page": 1},
        "other_attribute": "x",
    }).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.starts_with("| id | name |\n| --- | --- |\n| 1 | a |\n| 2 | b |"));
    assert!(markdown.contains("\"metadata\": {"));
    assert!(markdown.contains("\"other_attribute\": \"x\""));
    assert!(markdown.trim().ends_with("```"));
}

#[test]
fn test_level1_double_encoded_array_string() {
    let inner = json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]).to_string();
    let payload = json!({
        "content": inner,
        "metadata": {"total_items": 2},
        "description": "some rows",
    }).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.starts_with("| id | name |\n| --- | --- |\n| 1 | a |\n| 2 | b |"));
    assert!(markdown.contains("\"metadata\": {"));
    assert!(markdown.contains("\"description\": \"some rows\""));
    assert!(markdown.trim().ends_with("```"));
}

#[test]
fn test_over_escaped_newlines_in_double_encoded_field_are_repaired() {
    let pretty = serde_json::to_string_pretty(&json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}])).unwrap();
    let real_lines: Vec<&str> = pretty.split('\n').collect();
    let corrupted_inner = format!("{}\n{}", real_lines[..2].join("\n"), real_lines[2..].join("\\n"));
    assert!(corrupted_inner.contains("\\n"));

    let payload = json!({"content": corrupted_inner, "metadata": {"items_per_page": 100}}).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.starts_with("| id | name |\n| --- | --- |\n| 1 | a |\n| 2 | b |"));
}

#[test]
fn test_repair_is_not_attempted_when_json_is_otherwise_valid() {
    let payload = json!({"content": json!([{"a": 1}]).to_string(), "metadata": {"page": 1}}).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.contains("| a |"));
}

#[test]
fn test_unrepairable_malformed_json_still_returns_none() {
    let payload = json!({"content": "[{\"a\": 1}, {bad json here}]", "metadata": {"page": 1}}).to_string();
    assert!(try_convert_json_table(&payload).is_none());
}

#[test]
fn test_native_array_preferred_over_double_encoded_string() {
    let inner = json!([{"b": 2}]).to_string();
    let payload = json!({"result": [{"a": 1}], "content": inner}).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.contains("| a |"));
    assert!(!markdown.contains("| b |"));
}

#[test]
fn test_string_field_that_is_not_json_is_kept_as_metadata_not_rows() {
    let payload = json!({"result": [{"a": 1}], "notes": "just some text, not json"}).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.contains("\"notes\": \"just some text, not json\""));
}

#[test]
fn test_jsonl_rows() {
    let payload = format!("{}\n{}\n{}",
        json!({"id": 1, "name": "a"}),
        json!({"id": 2, "name": "b"}),
        json!({"id": 3, "name": "c"})
    );
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(
        markdown,
        "| id | name |\n| --- | --- |\n| 1 | a |\n| 2 | b |\n| 3 | c |\n"
    );
}

#[test]
fn test_missing_keys_become_empty_cells() {
    let payload = json!([{"id": 1, "name": "a"}, {"id": 2}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(
        markdown,
        "| id | name |\n| --- | --- |\n| 1 | a |\n| 2 |  |\n"
    );
}

#[test]
fn test_empty_collections_render_as_empty_cells() {
    let payload = json!([{"id": 1, "tags": []}, {"id": 2, "tags": {}}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(markdown, "| id | tags |\n| --- | --- |\n| 1 |  |\n| 2 |  |\n");
}

#[test]
fn test_whitespace_only_strings_render_as_empty_cells() {
    let payload = json!([{"id": 1, "note": "   "}, {"id": 2, "note": "\t\n"}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(markdown, "| id | note |\n| --- | --- |\n| 1 |  |\n| 2 |  |\n");
}

#[test]
fn test_pipe_and_newline_escaping() {
    let payload = json!([{"note": "a|b\nc"}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(markdown, "| note |\n| --- |\n| a\\|b<br>c |\n");
}

#[test]
fn test_column_names_with_pipes_and_newlines_are_escaped() {
    let payload = json!([{"a|b": 1, "c\nd": 2}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(markdown, "| a\\|b | c<br>d |\n| --- | --- |\n| 1 | 2 |\n");
}

#[test]
fn test_crlf_and_lone_cr_are_normalized_like_newlines() {
    let payload = json!([{"a": "x\r\ny", "b": "p\rq"}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(markdown, "| a | b |\n| --- | --- |\n| x<br>y | p<br>q |\n");
}

#[test]
fn test_nested_value_rendered_as_compact_json() {
    let payload = json!([{"id": 1, "tags": ["x", "y"]}]).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.contains("| [\"x\",\"y\"] |"));
}

#[test]
fn test_non_table_json_returns_none() {
    assert!(try_convert_json_table(&json!({"a": 1, "b": "text"}).to_string()).is_none());
}

#[test]
fn test_plain_text_returns_none() {
    assert!(try_convert_json_table("just some plain text response").is_none());
}

#[test]
fn test_empty_array_returns_none() {
    assert!(try_convert_json_table("[]").is_none());
}

#[test]
fn test_array_of_scalars_returns_none() {
    assert!(try_convert_json_table(&json!([1, 2, 3]).to_string()).is_none());
}

#[test]
fn test_single_json_object_is_not_a_jsonl_table() {
    assert!(try_convert_json_table(&json!({"id": 1, "name": "a"}).to_string()).is_none());
}

#[test]
fn test_ambiguous_multiple_array_fields_without_preferred_key_returns_none() {
    let payload = json!({
        "foo": [{"a": 1}],
        "bar": [{"b": 2}],
    }).to_string();
    assert!(try_convert_json_table(&payload).is_none());
}

#[test]
fn test_preferred_key_wins_when_multiple_array_fields_present() {
    let payload = json!({
        "result": [{"a": 1}],
        "extra_list": [{"b": 2}],
    }).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert!(markdown.contains("| a |"));
    assert!(markdown.contains("\"extra_list\""));
}

#[test]
fn test_invalid_jsonl_line_returns_none() {
    let payload = format!("{}\nnot json", json!({"id": 1}));
    assert!(try_convert_json_table(&payload).is_none());
}

#[test]
fn test_metadata_containing_backticks_does_not_break_code_fence() {
    let payload = json!({
        "result": [{"id": 1}],
        "metadata": {"note": "see ```python\nprint(1)\n``` for details"},
    }).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    let fence_line = markdown.lines().last().unwrap();
    assert_eq!(fence_line, "````");
    assert!(markdown.contains("````json"));
    assert!(markdown.trim().ends_with("````"));
}

#[test]
fn test_disjoint_row_shapes_return_none() {
    let payload = json!([{"a": 1}, {"b": 2}, {"c": 3}]).to_string();
    assert!(try_convert_json_table(&payload).is_none());
}

#[test]
fn test_common_shape_with_one_row_missing_several_keys_still_converts() {
    let payload = json!(
        [
            {"id": 1, "name": "a", "score": 9.5},
            {"id": 2, "name": "b", "score": 7.1},
            {"id": 3, "name": "c", "score": 8.0},
            {"id": 4},
        ]
    ).to_string();
    let markdown = try_convert_json_table(&payload).unwrap();
    assert_eq!(
        markdown,
        "| id | name | score |\n| --- | --- | --- |\n| 1 | a | 9.5 |\n| 2 | b | 7.1 |\n| 3 | c | 8.0 |\n| 4 |  |  |\n"
    );
}
