use rusqlite::{Connection, TransactionBehavior, params_from_iter, types::Value as SqlValue};
use serde_json::{Map, Value};

use super::super::{
    model::{PendingSqliteChange, SqliteChange, SqliteChangeStore, SqliteError},
    query::{fetch_rows_by_rowids, fetch_target_rowids_for_where, json_to_sql_value, quote_ident},
};
use super::validation::{validate_change, validate_expected};

#[derive(Debug, Clone)]
pub struct SqliteConfirmResult {
    pub applied: bool,
    pub action_id: String,
    pub change_type: &'static str,
    pub table: String,
}

pub fn sqlite_confirm_change(
    store: &mut SqliteChangeStore,
    action_id: &str,
) -> Result<SqliteConfirmResult, SqliteError> {
    let pending = store.take(action_id)?;
    validate_change(&pending.change)?;
    validate_expected(change_expected(&pending.change))?;
    let mut connection = Connection::open_with_flags(
        &pending.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    apply_change(&transaction, &pending)?;
    transaction.commit()?;
    Ok(SqliteConfirmResult {
        applied: true,
        action_id: action_id.to_owned(),
        change_type: pending.change.type_name(),
        table: pending.change.table().to_owned(),
    })
}

fn apply_change(
    connection: &rusqlite::Transaction<'_>,
    pending: &PendingSqliteChange,
) -> Result<(), SqliteError> {
    match &pending.change {
        SqliteChange::Insert {
            table,
            columns,
            values,
        } => {
            let placeholders = std::iter::repeat_n("?", columns.len())
                .collect::<Vec<_>>()
                .join(", ");
            let columns = columns
                .iter()
                .map(|column| quote_ident(column))
                .collect::<Result<Vec<_>, _>>()?;
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({placeholders})",
                quote_ident(table)?,
                columns.join(", ")
            );
            let params = values
                .iter()
                .map(json_to_sql_value)
                .collect::<Result<Vec<_>, _>>()?;
            connection.execute(&sql, params_from_iter(params.iter()))?;
        }
        SqliteChange::Update {
            table,
            set,
            where_,
            limit,
            expected,
        } => {
            let rowids =
                fetch_target_rowids_for_where(connection, table, where_, limit.unwrap_or(1))?;
            verify_expected(connection, table, &rowids, expected.as_ref())?;
            if !rowids.is_empty() {
                let (set_sql, mut params) = build_set_clause(set)?;
                let placeholders = std::iter::repeat_n("?", rowids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "UPDATE {} SET {} WHERE rowid IN ({placeholders})",
                    quote_ident(table)?,
                    set_sql
                );
                params.extend(rowids.iter().copied().map(SqlValue::Integer));
                connection.execute(&sql, params_from_iter(params.iter()))?;
            }
        }
        SqliteChange::Delete {
            table,
            where_,
            limit,
            expected,
        } => {
            let rowids =
                fetch_target_rowids_for_where(connection, table, where_, limit.unwrap_or(1))?;
            verify_expected(connection, table, &rowids, expected.as_ref())?;
            if !rowids.is_empty() {
                let placeholders = std::iter::repeat_n("?", rowids.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "DELETE FROM {} WHERE rowid IN ({placeholders})",
                    quote_ident(table)?
                );
                let params = rowids
                    .iter()
                    .copied()
                    .map(SqlValue::Integer)
                    .collect::<Vec<_>>();
                connection.execute(&sql, params_from_iter(params.iter()))?;
            }
        }
    }
    Ok(())
}

fn change_expected(change: &SqliteChange) -> Option<&Map<String, Value>> {
    match change {
        SqliteChange::Insert { .. } => None,
        SqliteChange::Update { expected, .. } | SqliteChange::Delete { expected, .. } => {
            expected.as_ref()
        }
    }
}

fn build_set_clause(set: &Map<String, Value>) -> Result<(String, Vec<SqlValue>), SqliteError> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for (key, value) in set {
        let Some((column, path)) = key.split_once('.') else {
            clauses.push(format!("{} = ?", quote_ident(key)?));
            params.push(json_to_sql_value(value)?);
            continue;
        };
        let json_path = format!("$.{path}");
        clauses.push(format!(
            "{} = json_set({}, '{}', ?)",
            quote_ident(column)?,
            quote_ident(column)?,
            json_path
        ));
        params.push(json_to_sql_value(value)?);
    }
    Ok((clauses.join(", "), params))
}

fn verify_expected(
    connection: &rusqlite::Transaction<'_>,
    table: &str,
    rowids: &[i64],
    expected: Option<&Map<String, Value>>,
) -> Result<(), SqliteError> {
    let Some(expected) = expected.filter(|expected| !expected.is_empty()) else {
        return Ok(());
    };
    if rowids.is_empty() {
        return Err(SqliteError::ExpectedMismatch(
            "rows not found (no rows match WHERE clause)".to_owned(),
        ));
    }
    let current = fetch_rows_by_rowids(connection, table, rowids)?;
    for (key, expected_value) in expected {
        for row in &current {
            let actual = row.get(key).unwrap_or(&Value::Null);
            if actual != expected_value {
                return Err(SqliteError::ExpectedMismatch(format!(
                    "field {key} expected {}, got {}. The row changed since preview. Run preview again.",
                    serde_json::to_string(expected_value).unwrap_or_else(|_| "null".to_owned()),
                    serde_json::to_string(actual).unwrap_or_else(|_| "null".to_owned())
                )));
            }
        }
    }
    Ok(())
}

trait SqliteChangeDetails {
    fn table(&self) -> &str;
    fn type_name(&self) -> &'static str;
}

impl SqliteChangeDetails for SqliteChange {
    fn table(&self) -> &str {
        match self {
            SqliteChange::Insert { table, .. }
            | SqliteChange::Update { table, .. }
            | SqliteChange::Delete { table, .. } => table,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            SqliteChange::Insert { .. } => "insert",
            SqliteChange::Update { .. } => "update",
            SqliteChange::Delete { .. } => "delete",
        }
    }
}
