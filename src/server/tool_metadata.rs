use std::sync::Arc;

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{Tool, ToolAnnotations},
};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
struct ToolMetadata {
    title: &'static str,
    read_only_hint: Option<bool>,
    destructive_hint: Option<bool>,
    open_world_hint: Option<bool>,
}

pub(crate) fn apply<S>(router: &mut ToolRouter<S>, max_output_bytes: u32, sqlite_max_rows: u32) {
    for route in router.map.values_mut() {
        let Some(metadata) = metadata_for(&route.attr.name) else {
            continue;
        };
        apply_definition(&mut route.attr, metadata, max_output_bytes, sqlite_max_rows);
    }
}

fn apply_definition(
    tool: &mut Tool,
    metadata: ToolMetadata,
    max_output_bytes: u32,
    sqlite_max_rows: u32,
) {
    tool.title = Some(metadata.title.to_owned());
    tool.annotations = Some(ToolAnnotations::from_raw(
        None,
        metadata.read_only_hint,
        metadata.destructive_hint,
        None,
        metadata.open_world_hint,
    ));

    let mut schema = Value::Object(tool.input_schema.as_ref().clone());
    normalize_schema(&mut schema);
    customize_schema(
        &mut schema,
        tool.name.as_ref(),
        max_output_bytes,
        sqlite_max_rows,
    );
    tool.input_schema = Arc::new(schema.as_object().cloned().unwrap_or_default());
}

fn metadata_for(name: &str) -> Option<ToolMetadata> {
    let (title, read_only_hint, destructive_hint, open_world_hint) = match name {
        "local_status" => ("Local status", Some(true), None, None),
        "open_workspace" => ("Open workspace", Some(true), None, None),
        "list_dir" => ("List directory", Some(true), None, None),
        "read_file" => ("Read file", Some(true), None, None),
        "search_files" => ("Search files", Some(true), None, None),
        "find_files" => ("Find files", Some(true), None, None),
        "project_tree" => ("Project tree", Some(true), None, None),
        "git_status" => ("Git status", Some(true), None, None),
        "git_diff" => ("Git diff", Some(true), None, None),
        "preview_edit" => ("Preview edit", Some(false), Some(false), None),
        "confirm_edit" => ("Confirm edit", Some(false), Some(true), None),
        "exec_process" => ("Exec process", Some(false), Some(false), Some(true)),
        "process_start" => ("Start process", Some(false), Some(false), Some(true)),
        "process_read" => ("Read process", Some(true), None, None),
        "process_stop" => ("Stop process", Some(false), Some(true), None),
        "sqlite_status" => ("SQLite status", Some(true), None, None),
        "sqlite_schema" => ("SQLite schema", Some(true), None, None),
        "sqlite_select" => ("SQLite select", Some(true), None, None),
        "sqlite_preview_change" => ("SQLite preview change", Some(false), Some(false), None),
        "sqlite_confirm_change" => ("SQLite confirm change", Some(false), Some(true), None),
        "web_status" => ("Web status", Some(true), None, None),
        "web_search" => ("Web search", Some(true), None, None),
        "web_fetch" => ("Web fetch", Some(true), None, None),
        "open_project" => ("Open Desktop project", Some(true), None, Some(false)),
        "project_state" => ("Inspect project state", Some(true), None, Some(false)),
        "search_code" => ("Search project code", Some(true), None, Some(false)),
        "read_files" => ("Read project files", Some(true), None, Some(false)),
        "apply_patch" => (
            "Apply bounded project edits",
            Some(false),
            Some(false),
            Some(false),
        ),
        "run_project_check" => ("Run project check", Some(false), Some(false), Some(false)),
        "run_project_command" => (
            "Run allowed project command",
            Some(false),
            Some(false),
            Some(true),
        ),
        "manage_process" => (
            "Manage development process",
            Some(false),
            Some(false),
            Some(true),
        ),
        "codex_start_session" => (
            "Start delegated Codex session",
            Some(false),
            Some(false),
            Some(false),
        ),
        "codex_send_message" => (
            "Send message to Codex",
            Some(false),
            Some(false),
            Some(false),
        ),
        "codex_read_response" => ("Read Codex response", Some(true), None, Some(false)),
        "codex_stop_session" => ("Stop Codex session", Some(false), Some(false), Some(false)),
        _ => return None,
    };
    Some(ToolMetadata {
        title,
        read_only_hint,
        destructive_hint,
        open_world_hint,
    })
}

fn normalize_schema(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };

    if object.get("default").is_some_and(Value::is_null) {
        object.remove("default");
    }
    if let Some(Value::Array(types)) = object.get_mut("type") {
        let without_null = types
            .iter()
            .filter(|value| value != &&Value::String("null".to_owned()))
            .cloned()
            .collect::<Vec<_>>();
        if without_null.len() == 1 {
            object.insert("type".to_owned(), without_null[0].clone());
        } else if without_null.len() < types.len() {
            object.insert("type".to_owned(), Value::Array(without_null));
        }
    }

    for child in object.values_mut() {
        normalize_schema(child);
    }
}

fn customize_schema(
    schema: &mut Value,
    tool_name: &str,
    max_output_bytes: u32,
    sqlite_max_rows: u32,
) {
    match tool_name {
        "list_dir" => set_default(schema, "path", json!(".")),
        "search_files" => {
            set_default(schema, "path", json!("."));
            set_default(schema, "caseSensitive", json!(false));
            set_default(schema, "contextLines", json!(0));
            set_default(schema, "maxMatches", json!(1_000));
            set_range(schema, "contextLines", Some(0), Some(20));
            set_range(schema, "maxMatches", Some(1), Some(5_000));
        }
        "find_files" => {
            set_default(schema, "path", json!("."));
            set_default(schema, "maxResults", json!(100));
            set_range(schema, "maxResults", Some(1), Some(500));
        }
        "project_tree" => {
            set_default(schema, "path", json!("."));
            set_default(schema, "depth", json!(3));
            set_range(schema, "depth", Some(1), Some(5));
        }
        "git_diff" => {
            set_default(schema, "staged", json!(false));
            set_default(schema, "statOnly", json!(false));
            set_default(schema, "maxBytes", json!(max_output_bytes));
            set_range(schema, "maxBytes", Some(1), Some(max_output_bytes.into()));
        }
        "exec_process" => customize_process_schema(schema, 30, 300, max_output_bytes),
        "process_start" => customize_process_schema(schema, 300, 3_600, max_output_bytes),
        "open_project" => {
            set_default(schema, "treeDepth", json!(2));
            set_range(schema, "treeDepth", Some(1), Some(4));
        }
        "search_code" => {
            set_default(schema, "path", json!("."));
            set_default(schema, "caseSensitive", json!(false));
            set_default(schema, "contextLines", json!(2));
            set_default(schema, "maxMatches", json!(200));
            set_range(schema, "contextLines", Some(0), Some(10));
            set_range(schema, "maxMatches", Some(1), Some(1_000));
        }
        "read_files" => {
            set_array_range(schema, "paths", Some(1), Some(20));
            set_default(schema, "maxBytesPerFile", json!(50_000));
            set_range(schema, "maxBytesPerFile", Some(1_000), Some(100_000));
        }
        "apply_patch" => {
            set_array_range(schema, "changes", Some(1), Some(20));
            retain_safe_edit_variants(schema);
        }
        "preview_edit" => set_array_range(schema, "changes", Some(1), None),
        "run_project_check" => {
            set_default(schema, "kind", json!("auto"));
            set_default(schema, "timeoutSeconds", json!(300));
            set_range(schema, "timeoutSeconds", Some(1), Some(900));
        }
        "run_project_command" => {
            set_default(schema, "args", json!([]));
            set_default(schema, "workingDirectory", json!("."));
            set_default(schema, "timeoutSeconds", json!(120));
            set_default(schema, "maxBytes", json!(max_output_bytes));
            set_range(schema, "timeoutSeconds", Some(1), Some(900));
            set_range(
                schema,
                "maxBytes",
                Some(1_000),
                Some(max_output_bytes.into()),
            );
        }
        "manage_process" => {
            set_default(schema, "args", json!([]));
            set_default(schema, "workingDirectory", json!("."));
            set_default(schema, "timeoutSeconds", json!(600));
            set_default(schema, "maxBytes", json!(max_output_bytes));
            set_range(schema, "timeoutSeconds", Some(1), Some(3_600));
            set_range(
                schema,
                "maxBytes",
                Some(1_000),
                Some(max_output_bytes.into()),
            );
        }
        "sqlite_select" => {
            set_default(schema, "params", json!([]));
            set_default(schema, "limit", json!(sqlite_max_rows));
            set_range(schema, "limit", Some(1), Some(sqlite_max_rows.into()));
            set_scalar_array_items(schema, "params", None);
        }
        "sqlite_preview_change" => {
            normalize_sqlite_value_schemas(schema);
            remove_nested_default(schema, "where");
        }
        "web_search" => {
            set_default(schema, "limit", json!(5));
            set_range(schema, "limit", Some(1), Some(10));
        }
        "codex_start_session" => {
            set_default(schema, "mode", json!("review"));
            set_string_length(schema, "prompt", Some(1), Some(32_000));
        }
        "codex_send_message" => set_string_length(schema, "message", Some(1), Some(32_000)),
        "codex_read_response" => {
            set_default(schema, "waitSeconds", json!(30));
            set_range(schema, "waitSeconds", Some(0), Some(60));
        }
        _ => {}
    }
}

fn customize_process_schema(
    schema: &mut Value,
    timeout_default: u64,
    timeout_maximum: u64,
    max_output_bytes: u32,
) {
    set_default(schema, "args", json!([]));
    set_default(schema, "workingDirectory", json!("."));
    set_default(schema, "timeoutSeconds", json!(timeout_default));
    set_default(schema, "maxBytes", json!(max_output_bytes));
    set_range(schema, "timeoutSeconds", Some(1), Some(timeout_maximum));
    set_range(
        schema,
        "maxBytes",
        Some(1_000),
        Some(max_output_bytes.into()),
    );
}

fn properties_mut(schema: &mut Value) -> Option<&mut Map<String, Value>> {
    schema.get_mut("properties")?.as_object_mut()
}

fn property_mut<'a>(schema: &'a mut Value, name: &str) -> Option<&'a mut Map<String, Value>> {
    properties_mut(schema)?.get_mut(name)?.as_object_mut()
}

fn set_default(schema: &mut Value, name: &str, default: Value) {
    if let Some(property) = property_mut(schema, name) {
        property.insert("default".to_owned(), default);
    }
}

fn set_range(schema: &mut Value, name: &str, minimum: Option<u64>, maximum: Option<u64>) {
    let Some(property) = property_mut(schema, name) else {
        return;
    };
    if let Some(minimum) = minimum {
        property.insert("minimum".to_owned(), json!(minimum));
    }
    if let Some(maximum) = maximum {
        property.insert("maximum".to_owned(), json!(maximum));
    }
}

fn set_string_length(schema: &mut Value, name: &str, minimum: Option<u64>, maximum: Option<u64>) {
    let Some(property) = property_mut(schema, name) else {
        return;
    };
    if let Some(minimum) = minimum {
        property.insert("minLength".to_owned(), json!(minimum));
    }
    if let Some(maximum) = maximum {
        property.insert("maxLength".to_owned(), json!(maximum));
    }
}

fn set_array_range(schema: &mut Value, name: &str, minimum: Option<u64>, maximum: Option<u64>) {
    let Some(property) = property_mut(schema, name) else {
        return;
    };
    if let Some(minimum) = minimum {
        property.insert("minItems".to_owned(), json!(minimum));
    }
    if let Some(maximum) = maximum {
        property.insert("maxItems".to_owned(), json!(maximum));
    }
}

fn set_scalar_array_items(schema: &mut Value, name: &str, minimum: Option<u64>) {
    let Some(property) = property_mut(schema, name) else {
        return;
    };
    property.insert(
        "items".to_owned(),
        json!({"type": ["string", "number", "null"]}),
    );
    if let Some(minimum) = minimum {
        property.insert("minItems".to_owned(), json!(minimum));
    }
}

fn retain_safe_edit_variants(schema: &mut Value) {
    retain_tagged_edit_variants(schema);
}

fn retain_tagged_edit_variants(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };
    if let Some(one_of) = object.get_mut("oneOf").and_then(Value::as_array_mut) {
        let is_edit_union = one_of.iter().any(|variant| {
            variant
                .pointer("/properties/type/const")
                .and_then(Value::as_str)
                .is_some()
        });
        if is_edit_union {
            one_of.retain(|variant| {
                variant
                    .pointer("/properties/type/const")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(
                            kind,
                            "replace_text"
                                | "replace_range"
                                | "insert_before"
                                | "insert_after"
                                | "append"
                                | "create"
                        )
                    })
            });
        }
    }
    for child in object.values_mut() {
        retain_tagged_edit_variants(child);
    }
}

fn normalize_sqlite_value_schemas(value: &mut Value) {
    let Value::Object(object) = value else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for (name, property) in properties {
            match name.as_str() {
                "value" => *property = json!({"type": ["string", "number", "null"]}),
                "values" => {
                    set_scalar_array_items_in_property(property, Some(1));
                }
                "params" => set_scalar_array_items_in_property(property, None),
                "set" => {
                    if let Some(set) = property.as_object_mut() {
                        set.insert(
                            "additionalProperties".to_owned(),
                            json!({"type": ["string", "number", "null"]}),
                        );
                    }
                }
                _ => {}
            }
            normalize_sqlite_value_schemas(property);
        }
    }
    for child in object.values_mut() {
        normalize_sqlite_value_schemas(child);
    }
}

fn set_scalar_array_items_in_property(property: &mut Value, minimum: Option<u64>) {
    let Some(property) = property.as_object_mut() else {
        return;
    };
    property.insert(
        "items".to_owned(),
        json!({"type": ["string", "number", "null"]}),
    );
    if let Some(minimum) = minimum {
        property.insert("minItems".to_owned(), json!(minimum));
    }
}

fn remove_nested_default(value: &mut Value, property_name: &str) {
    let Value::Object(object) = value else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut)
        && let Some(property) = properties
            .get_mut(property_name)
            .and_then(Value::as_object_mut)
    {
        property.remove("default");
    }
    for child in object.values_mut() {
        remove_nested_default(child, property_name);
    }
}
