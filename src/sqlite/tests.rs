use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

use super::*;
use crate::config::{AccessMode, SearchProvider, ToolProfile};

fn test_config(database: &Path, enabled: bool) -> crate::config::Config {
    crate::config::Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![database.parent().expect("database parent").to_path_buf()],
        deny_globs: Vec::new(),
        access_mode: AccessMode::Coding,
        tool_profile: ToolProfile::Legacy,
        stateless_mcp_fallback: false,
        codex_bridge_enabled: false,
        codex_command: "codex".to_owned(),
        codex_max_sessions: 4,
        codex_request_timeout_ms: 120_000,
        max_read_bytes: 200_000,
        max_output_bytes: 200_000,
        web_tools_enabled: false,
        search_provider: SearchProvider::None,
        searxng_url: String::new(),
        web_max_bytes: 200_000,
        web_timeout_ms: 15_000,
        sqlite_tools_enabled: enabled,
        sqlite_allowed_dbs: vec![database.to_path_buf()],
        sqlite_max_rows: 100,
    }
}

fn database() -> (tempfile::TempDir, PathBuf) {
    let temp = tempdir().expect("temporary directory");
    let path = temp.path().join("test.sqlite");
    let connection = Connection::open(&path).expect("database");
    connection
        .execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, meta TEXT);
             INSERT INTO users (name, meta) VALUES ('Ada', '{\"enabled\":false}');
             INSERT INTO users (name, meta) VALUES ('Grace', '{\"enabled\":true}');",
        )
        .expect("schema and rows");
    (temp, path)
}

#[test]
fn sqlite_select_schema_and_read_only_rules_match_reference() {
    let (_temp, path) = database();
    let config = test_config(&path, true);

    let schema = sqlite_schema(&config, None).expect("schema");
    assert!(schema.iter().any(|row| row["name"] == "users"));
    let rows = sqlite_select(
        &config,
        None,
        "SELECT id, name FROM users ORDER BY id",
        &[],
        1,
    )
    .expect("select");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Ada");
    assert!(sqlite_select(&config, None, "DELETE FROM users", &[], 10).is_err());
}

#[test]
fn sqlite_select_rejects_write_statements_hidden_behind_with() {
    let (_temp, path) = database();
    let config = test_config(&path, true);

    let error = sqlite_select(
        &config,
        None,
        "WITH selected AS (SELECT id FROM users) DELETE FROM users",
        &[],
        10,
    )
    .expect_err("WITH DELETE must not be treated as a read-only query");
    assert!(matches!(error, SqliteError::InvalidQuery(message) if message.contains("read-only")));

    let rows = sqlite_select(
        &config,
        None,
        "SELECT COUNT(*) AS count FROM users",
        &[],
        10,
    )
    .expect("database remains readable");
    assert_eq!(rows[0]["count"], 2);
}

#[test]
fn preview_confirm_update_insert_delete_and_expected_guard() {
    let (_temp, path) = database();
    let config = test_config(&path, true);
    let mut store = SqliteChangeStore::new();

    let update = SqliteChange::Update {
        table: "users".to_owned(),
        set: serde_json::from_value(json!({
            "name": "Ada Lovelace",
            "meta.enabled": 1
        }))
        .expect("set map"),
        where_: vec![SqliteWhereCondition {
            column: "id".to_owned(),
            operator: SqliteOperator::Equal,
            value: json!(1),
        }],
        limit: Some(1),
        expected: Some(serde_json::from_value(json!({"name": "Ada"})).expect("expected map")),
    };
    let preview = sqlite_preview_change(&config, &mut store, None, update).expect("update preview");
    assert!(preview.diff.contains("Ada Lovelace"));
    sqlite_confirm_change(&mut store, &preview.action.id).expect("update confirm");

    let insert = SqliteChange::Insert {
        table: "users".to_owned(),
        columns: vec!["name".to_owned(), "meta".to_owned()],
        values: vec![json!("Lin"), json!("{}")],
    };
    let insert_preview =
        sqlite_preview_change(&config, &mut store, None, insert).expect("insert preview");
    sqlite_confirm_change(&mut store, &insert_preview.action.id).expect("insert confirm");

    let delete = SqliteChange::Delete {
        table: "users".to_owned(),
        where_: vec![SqliteWhereCondition {
            column: "name".to_owned(),
            operator: SqliteOperator::Equal,
            value: json!("Lin"),
        }],
        limit: Some(1),
        expected: None,
    };
    let delete_preview =
        sqlite_preview_change(&config, &mut store, None, delete).expect("delete preview");
    sqlite_confirm_change(&mut store, &delete_preview.action.id).expect("delete confirm");

    let rows = sqlite_select(
        &config,
        None,
        "SELECT name, meta FROM users WHERE id = 1",
        &[],
        1,
    )
    .expect("updated row");
    assert_eq!(rows[0]["name"], "Ada Lovelace");
    assert_eq!(rows[0]["meta"], "{\"enabled\":1}");
    let missing = sqlite_select(
        &config,
        None,
        "SELECT name FROM users WHERE name = 'Lin'",
        &[],
        1,
    )
    .expect("deleted row query");
    assert!(missing.is_empty());
}

#[test]
fn disabled_sqlite_tools_do_not_open_database() {
    let (_temp, path) = database();
    let config = test_config(&path, false);
    assert!(matches!(
        sqlite_schema(&config, None),
        Err(SqliteError::Disabled)
    ));
}
