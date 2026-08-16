use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::config::Config;

use super::super::{
    connection::{open_database, resolve_allowed_db},
    model::{JsonRow, PendingSqliteChange, SqliteChange, SqliteChangeStore, SqliteError},
    query::{fetch_rows_for_where, quote_ident},
};
use super::validation::{validate_change, validate_expected};

pub struct SqlitePreviewResult {
    pub action: PendingSqliteChange,
    pub before_rows: Vec<JsonRow>,
    pub diff: String,
}

pub fn sqlite_preview_change(
    config: &Config,
    store: &mut SqliteChangeStore,
    db_path: Option<&str>,
    change: SqliteChange,
) -> Result<SqlitePreviewResult, SqliteError> {
    let resolved = resolve_allowed_db(config, db_path)?;
    validate_change(&change)?;
    validate_expected(change_expected(&change))?;
    let connection = open_database(&resolved, true)?;
    let (before_rows, diff) = match &change {
        SqliteChange::Insert {
            table,
            columns,
            values,
        } => (Vec::new(), format_insert(table, columns, values)?),
        SqliteChange::Update {
            table,
            set,
            where_,
            limit,
            ..
        } => {
            let before = fetch_rows_for_where(&connection, table, where_, limit.unwrap_or(1))?;
            let after = before
                .iter()
                .map(|row| apply_set_to_row(row, set))
                .collect::<Vec<_>>();
            (
                before.clone(),
                format_before_after(&before, &after, table, "UPDATE", set)?,
            )
        }
        SqliteChange::Delete {
            table,
            where_,
            limit,
            ..
        } => {
            let before = fetch_rows_for_where(&connection, table, where_, limit.unwrap_or(1))?;
            let diff = format_delete_preview(&before, table)?;
            (before, diff)
        }
    };
    let action = store.create(resolved, change, before_rows.clone(), diff.clone());
    Ok(SqlitePreviewResult {
        action,
        before_rows,
        diff,
    })
}

fn change_expected(change: &SqliteChange) -> Option<&Map<String, Value>> {
    match change {
        SqliteChange::Insert { .. } => None,
        SqliteChange::Update { expected, .. } | SqliteChange::Delete { expected, .. } => {
            expected.as_ref()
        }
    }
}

fn format_insert(table: &str, columns: &[String], values: &[Value]) -> Result<String, SqliteError> {
    let quoted_columns = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Result<Vec<_>, _>>()?;
    let mut row = Map::new();
    for (column, value) in columns.iter().zip(values) {
        row.insert(column.clone(), value.clone());
    }
    Ok(format!(
        "+ INSERT INTO {} ({})\n  VALUES\n{}",
        quote_ident(table)?,
        quoted_columns.join(", "),
        serde_json::to_string_pretty(&row).expect("JSON row is serializable")
    ))
}

fn format_before_after(
    before: &[JsonRow],
    after: &[JsonRow],
    table: &str,
    operation: &str,
    set: &Map<String, Value>,
) -> Result<String, SqliteError> {
    let mut lines = Vec::new();
    for (index, (before_row, after_row)) in before.iter().zip(after).enumerate() {
        lines.push(format!("--- {table} row {} (before)", index + 1));
        lines.push(format!("+++ {table} row {} (after)", index + 1));
        let keys = before_row
            .keys()
            .chain(after_row.keys())
            .chain(set.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in keys {
            let before_value = before_row.get(&key).unwrap_or(&Value::Null);
            let after_value = after_row.get(&key).unwrap_or(&Value::Null);
            if before_value != after_value {
                lines.push(format!("- {key}: {}", json_string(before_value)?));
                lines.push(format!("+ {key}: {}", json_string(after_value)?));
            }
        }
        if index + 1 < before.len() {
            lines.push("---".to_owned());
        }
    }
    if lines.is_empty() {
        lines.push(format!(
            "(no rows match the WHERE condition for {operation})"
        ));
    }
    Ok(lines.join("\n"))
}

fn format_delete_preview(rows: &[JsonRow], table: &str) -> Result<String, SqliteError> {
    if rows.is_empty() {
        return Ok(format!(
            "(no rows match the WHERE condition. Nothing will be deleted from {table})"
        ));
    }
    let mut lines = vec![format!(
        "DELETE from {table}: {} row(s) will be removed",
        rows.len()
    )];
    for (index, row) in rows.iter().enumerate() {
        lines.push(format!("  row {}: {}", index + 1, json_string(row)?));
    }
    Ok(lines.join("\n"))
}

fn apply_set_to_row(row: &JsonRow, set: &Map<String, Value>) -> JsonRow {
    let mut result = row.clone();
    for (key, value) in set {
        let Some((column, path)) = key.split_once('.') else {
            result.insert(key.clone(), value.clone());
            continue;
        };
        let Some(Value::String(raw)) = result.get(column) else {
            result.insert(column.to_owned(), value.clone());
            continue;
        };
        let Ok(mut parsed) = serde_json::from_str::<Value>(raw) else {
            result.insert(column.to_owned(), value.clone());
            continue;
        };
        let parts = path.split('.').collect::<Vec<_>>();
        if set_json_path(&mut parsed, &parts, value.clone()) {
            result.insert(
                column.to_owned(),
                Value::String(serde_json::to_string(&parsed).expect("JSON value is serializable")),
            );
        } else {
            result.insert(column.to_owned(), value.clone());
        }
    }
    result
}

fn set_json_path(current: &mut Value, parts: &[&str], value: Value) -> bool {
    let Some((head, tail)) = parts.split_first() else {
        return false;
    };
    let Value::Object(object) = current else {
        return false;
    };
    if tail.is_empty() {
        object.insert((*head).to_owned(), value);
        return true;
    }
    let child = object
        .entry((*head).to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if child.is_null() {
        *child = Value::Object(Map::new());
    }
    set_json_path(child, tail, value)
}

fn json_string(value: &impl serde::Serialize) -> Result<String, SqliteError> {
    serde_json::to_string(value).map_err(|error| SqliteError::InvalidChange(error.to_string()))
}
