use serde_json::{Map, Value};

use super::super::{
    model::{SqliteChange, SqliteError, SqliteWhereCondition},
    query::{is_identifier, json_to_sql_value},
};

pub(crate) fn validate_change(change: &SqliteChange) -> Result<(), SqliteError> {
    match change {
        SqliteChange::Insert {
            table,
            columns,
            values,
        } => {
            validate_table(table)?;
            if columns.is_empty() {
                return Err(invalid("insert requires at least one column"));
            }
            if values.len() != columns.len() {
                return Err(invalid(
                    "insert values array length must match columns array length",
                ));
            }
            for column in columns {
                validate_column(column, "insert")?;
            }
            for value in values {
                validate_scalar(value)?;
            }
        }
        SqliteChange::Update {
            table,
            set,
            where_,
            limit,
            ..
        } => {
            validate_table(table)?;
            if set.is_empty() {
                return Err(invalid("update requires at least one set field"));
            }
            for (key, value) in set {
                validate_set_key(key)?;
                validate_scalar(value)?;
            }
            validate_where(where_)?;
            validate_limit(*limit)?;
        }
        SqliteChange::Delete {
            table,
            where_,
            limit,
            ..
        } => {
            validate_table(table)?;
            validate_where(where_)?;
            validate_limit(*limit)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_where(where_: &[SqliteWhereCondition]) -> Result<(), SqliteError> {
    for condition in where_ {
        validate_column(&condition.column, "WHERE")?;
        validate_scalar(&condition.value)?;
    }
    Ok(())
}

pub(crate) fn validate_expected(_expected: Option<&Map<String, Value>>) -> Result<(), SqliteError> {
    // `expected` is only compared with the row returned during confirm; unlike
    // SET/WHERE values it is never bound into SQL. The reference contract
    // intentionally accepts unknown JSON values here (including blob-shaped
    // objects), so do not restrict it to SQLite scalar parameters.
    Ok(())
}

fn validate_table(table: &str) -> Result<(), SqliteError> {
    if is_identifier(table) {
        Ok(())
    } else {
        Err(invalid(format!("invalid or unsafe table name: {table:?}")))
    }
}

fn validate_column(column: &str, context: &str) -> Result<(), SqliteError> {
    if is_identifier(column) {
        Ok(())
    } else {
        Err(invalid(format!(
            "invalid column name in {context}: {column:?}"
        )))
    }
}

fn validate_set_key(key: &str) -> Result<(), SqliteError> {
    let mut parts = key.split('.');
    let column = parts.next().unwrap_or_default();
    validate_column(column, "set")?;
    for part in parts {
        if !is_identifier(part) {
            return Err(invalid(format!(
                "invalid JSON path segment in set key: {key:?}"
            )));
        }
    }
    Ok(())
}

fn validate_scalar(value: &Value) -> Result<(), SqliteError> {
    json_to_sql_value(value)
        .map(|_| ())
        .map_err(|error| invalid(error.to_string()))
}

fn validate_limit(limit: Option<usize>) -> Result<(), SqliteError> {
    if limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
        return Err(invalid("SQLite change limit must be between 1 and 100"));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> SqliteError {
    SqliteError::InvalidChange(message.into())
}
