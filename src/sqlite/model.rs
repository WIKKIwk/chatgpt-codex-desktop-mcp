use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

pub type JsonRow = Map<String, Value>;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SqliteStatus {
    pub enabled: bool,
    #[serde(rename = "allowedDbs")]
    pub allowed_dbs: Vec<String>,
    #[serde(rename = "maxRows")]
    pub max_rows: u32,
    #[serde(rename = "nodeSqlite")]
    pub node_sqlite: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SqliteChange {
    Insert {
        table: String,
        columns: Vec<String>,
        values: Vec<Value>,
    },
    Update {
        table: String,
        set: Map<String, Value>,
        #[serde(rename = "where", default)]
        where_: Vec<SqliteWhereCondition>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        expected: Option<Map<String, Value>>,
    },
    Delete {
        table: String,
        #[serde(rename = "where", default)]
        where_: Vec<SqliteWhereCondition>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        expected: Option<Map<String, Value>>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SqliteWhereCondition {
    pub column: String,
    pub operator: SqliteOperator,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub enum SqliteOperator {
    #[serde(rename = "=")]
    Equal,
    #[serde(rename = "!=")]
    NotEqual,
    #[serde(rename = ">")]
    Greater,
    #[serde(rename = "<")]
    Less,
    #[serde(rename = ">=")]
    GreaterOrEqual,
    #[serde(rename = "<=")]
    LessOrEqual,
    #[serde(rename = "LIKE")]
    Like,
    #[serde(rename = "IS")]
    Is,
    #[serde(rename = "IS NOT")]
    IsNot,
}

impl SqliteOperator {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::NotEqual => "!=",
            Self::Greater => ">",
            Self::Less => "<",
            Self::GreaterOrEqual => ">=",
            Self::LessOrEqual => "<=",
            Self::Like => "LIKE",
            Self::Is => "IS",
            Self::IsNot => "IS NOT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingSqliteChange {
    pub id: String,
    pub db_path: PathBuf,
    pub change: SqliteChange,
    pub before_rows: Vec<JsonRow>,
    pub diff: String,
}

#[derive(Debug, Default)]
pub struct SqliteChangeStore {
    changes: HashMap<String, PendingSqliteChange>,
}

impl SqliteChangeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &mut self,
        db_path: PathBuf,
        change: SqliteChange,
        before_rows: Vec<JsonRow>,
        diff: String,
    ) -> PendingSqliteChange {
        let entry = PendingSqliteChange {
            id: format!("sqlite_{}", Uuid::new_v4()),
            db_path,
            change,
            before_rows,
            diff,
        };
        self.changes.insert(entry.id.clone(), entry.clone());
        entry
    }

    pub fn take(&mut self, action_id: &str) -> Result<PendingSqliteChange, SqliteError> {
        self.changes
            .remove(action_id)
            .ok_or_else(|| SqliteError::UnknownAction(action_id.to_owned()))
    }
}

#[derive(Debug, Error)]
pub enum SqliteError {
    #[error("SQLite tools are disabled. Set CTM_SQLITE_TOOLS=1 to enable them.")]
    Disabled,
    #[error("No SQLite databases are allowed. Set CTM_SQLITE_ALLOWED_DBS.")]
    NoAllowedDatabases,
    #[error("SQLite database is not in CTM_SQLITE_ALLOWED_DBS: {0}")]
    DatabaseNotAllowed(PathBuf),
    #[error("SQLite database does not exist: {0}")]
    DatabaseMissing(PathBuf),
    #[error("SQLite path is not a file: {0}")]
    DatabaseNotFile(PathBuf),
    #[error("dbPath is required when multiple SQLite databases are allowed.")]
    DatabasePathRequired,
    #[error("Unknown sqlite actionId: {0}")]
    UnknownAction(String),
    #[error("invalid SQLite change: {0}")]
    InvalidChange(String),
    #[error("invalid SQLite query: {0}")]
    InvalidQuery(String),
    #[error("SQLite expected field mismatch: {0}")]
    ExpectedMismatch(String),
    #[error("SQLite database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("SQLite database path error: {0}")]
    Path(#[from] std::io::Error),
}
