use rusqlite::{
    Connection, params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde_json::{Map, Value, json};

use crate::config::Config;

use super::{
    connection::{open_database, resolve_allowed_db},
    model::{JsonRow, SqliteError, SqliteOperator, SqliteWhereCondition},
};

pub fn sqlite_schema(config: &Config, db_path: Option<&str>) -> Result<Vec<JsonRow>, SqliteError> {
    let path = resolve_allowed_db(config, db_path)?;
    let connection = open_database(&path, true)?;
    query_rows(
        &connection,
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('table','view','index','trigger') ORDER BY type, name",
        &[],
        None,
    )
}

pub fn sqlite_select(
    config: &Config,
    db_path: Option<&str>,
    sql: &str,
    params: &[Value],
    limit: usize,
) -> Result<Vec<JsonRow>, SqliteError> {
    validate_read_only_sql_shape(sql)?;
    let path = resolve_allowed_db(config, db_path)?;
    let connection = open_database(&path, true)?;
    assert_read_only_statement(&connection, sql)?;
    let sql_params = params
        .iter()
        .map(json_to_sql_value)
        .collect::<Result<Vec<_>, _>>()?;
    query_rows(
        &connection,
        sql,
        &sql_params,
        Some(limit.min(config.sqlite_max_rows as usize)),
    )
}

pub(crate) fn fetch_rows_for_where(
    connection: &Connection,
    table: &str,
    conditions: &[SqliteWhereCondition],
    limit: usize,
) -> Result<Vec<JsonRow>, SqliteError> {
    let resolved = build_where_clause(conditions)?;
    let where_clause = if resolved.clause.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", resolved.clause)
    };
    let sql = format!(
        "SELECT * FROM {}{} LIMIT {}",
        quote_ident(table)?,
        where_clause,
        limit.min(100)
    );
    query_rows(connection, &sql, &resolved.params, None)
}

pub(crate) fn fetch_target_rowids_for_where(
    connection: &Connection,
    table: &str,
    conditions: &[SqliteWhereCondition],
    limit: usize,
) -> Result<Vec<i64>, SqliteError> {
    let resolved = build_where_clause(conditions)?;
    let where_clause = if resolved.clause.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", resolved.clause)
    };
    let sql = format!(
        "SELECT rowid AS __ctm_rowid FROM {}{} LIMIT {}",
        quote_ident(table)?,
        where_clause,
        limit.min(100)
    );
    let rows = query_rows(connection, &sql, &resolved.params, None)?;
    rows.into_iter()
        .map(|row| match row.get("__ctm_rowid") {
            Some(Value::Number(value)) => {
                value.as_i64().filter(|value| *value > 0).ok_or_else(|| {
                    SqliteError::InvalidChange("unable to resolve SQLite rowid".to_owned())
                })
            }
            _ => Err(SqliteError::InvalidChange(
                "unable to resolve SQLite rowid".to_owned(),
            )),
        })
        .collect()
}

pub(crate) fn fetch_rows_by_rowids(
    connection: &Connection,
    table: &str,
    rowids: &[i64],
) -> Result<Vec<JsonRow>, SqliteError> {
    if rowids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", rowids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT * FROM {} WHERE rowid IN ({placeholders})",
        quote_ident(table)?
    );
    let params = rowids
        .iter()
        .copied()
        .map(SqlValue::Integer)
        .collect::<Vec<_>>();
    query_rows(connection, &sql, &params, None)
}

pub(crate) fn query_rows(
    connection: &Connection,
    sql: &str,
    params: &[SqlValue],
    limit: Option<usize>,
) -> Result<Vec<JsonRow>, SqliteError> {
    let mut statement = connection.prepare(sql)?;
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = statement.query(params_from_iter(params.iter()))?;
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        let mut object = Map::new();
        for (index, column) in columns.iter().enumerate() {
            object.insert(column.clone(), value_ref_to_json(row.get_ref(index)?));
        }
        result.push(object);
        if limit.is_some_and(|limit| result.len() >= limit) {
            break;
        }
    }
    Ok(result)
}

pub(crate) fn json_to_sql_value(value: &Value) -> Result<SqlValue, SqliteError> {
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::String(value) => Ok(SqlValue::Text(value.clone())),
        Value::Number(value) if value.as_i64().is_some() => {
            Ok(SqlValue::Integer(value.as_i64().expect("checked integer")))
        }
        Value::Number(value) => value
            .as_f64()
            .map(SqlValue::Real)
            .ok_or_else(|| SqliteError::InvalidQuery("number is outside SQLite range".to_owned())),
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => Err(SqliteError::InvalidQuery(
            "SQLite values must be strings, numbers, or null".to_owned(),
        )),
    }
}

pub(crate) fn quote_ident(name: &str) -> Result<String, SqliteError> {
    if !is_identifier(name) {
        return Err(SqliteError::InvalidChange(format!(
            "unsafe SQLite identifier: {name:?}"
        )));
    }
    Ok(format!("\"{name}\""))
}

pub(crate) fn build_where_clause(
    conditions: &[SqliteWhereCondition],
) -> Result<ResolvedWhere, SqliteError> {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for condition in conditions {
        let column = quote_ident(&condition.column)?;
        match condition.operator {
            SqliteOperator::Is | SqliteOperator::IsNot => {
                clauses.push(format!("{column} {} NULL", condition.operator.as_sql()));
            }
            _ => {
                clauses.push(format!("{column} {} ?", condition.operator.as_sql()));
                params.push(json_to_sql_value(&condition.value)?);
            }
        }
    }
    Ok(ResolvedWhere {
        clause: clauses.join(" AND "),
        params,
    })
}

pub(crate) struct ResolvedWhere {
    pub clause: String,
    pub params: Vec<SqlValue>,
}

pub(crate) fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|ch| matches!(ch, '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
}

fn value_ref_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => json!({"type": "Buffer", "data": value}),
    }
}

fn validate_read_only_sql_shape(sql: &str) -> Result<(), SqliteError> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(SqliteError::InvalidQuery(
            "SQL must not be empty.".to_owned(),
        ));
    }
    if trimmed.contains(';') {
        return Err(SqliteError::InvalidQuery(
            "Only one SQL statement is allowed.".to_owned(),
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    if starts_with_keyword(&lower, "select") || starts_with_keyword(&lower, "with") {
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("pragma") {
        let name = rest
            .trim_start()
            .split([' ', '\t', '\n', '\r', '(', '='])
            .next()
            .unwrap_or_default();
        if matches!(
            name,
            "table_info" | "table_list" | "index_list" | "index_info" | "foreign_key_list"
        ) {
            return Ok(());
        }
    }
    Err(SqliteError::InvalidQuery(
        "Only SELECT/WITH and safe PRAGMA statements are allowed.".to_owned(),
    ))
}

fn assert_read_only_statement(connection: &Connection, sql: &str) -> Result<(), SqliteError> {
    let statement = connection.prepare(sql)?;
    if statement.readonly() {
        Ok(())
    } else {
        Err(SqliteError::InvalidQuery(
            "Only read-only SQL statements are allowed.".to_owned(),
        ))
    }
}

fn starts_with_keyword(value: &str, keyword: &str) -> bool {
    value
        .strip_prefix(keyword)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}
