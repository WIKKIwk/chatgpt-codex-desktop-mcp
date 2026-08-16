use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::core_results::{Json, TextOutput};
use super::handler::ForgeHandler;
use super::sqlite_results::{
    SqliteConfirmOutput, SqlitePreviewOutput, SqliteRowsOutput, sqlite_confirm_output,
    sqlite_preview_output, sqlite_rows_output, sqlite_status_output,
};
use super::tool_error::ToolError;
use crate::sqlite::{
    SqliteChange, SqliteError, sqlite_confirm_change, sqlite_preview_change, sqlite_schema,
    sqlite_select, sqlite_status,
};

#[derive(Debug, Deserialize, JsonSchema)]
struct SqliteSchemaRequest {
    #[serde(rename = "dbPath", default)]
    db_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SqliteSelectRequest {
    #[serde(rename = "dbPath", default)]
    db_path: Option<String>,
    sql: String,
    #[serde(default)]
    params: Vec<Value>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SqlitePreviewRequest {
    #[serde(rename = "dbPath", default)]
    db_path: Option<String>,
    change: SqliteChange,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SqliteConfirmRequest {
    #[serde(rename = "actionId")]
    action_id: String,
}

#[tool_router(router = sqlite_tool_router, vis = "pub(crate)")]
impl ForgeHandler {
    #[tool(
        name = "sqlite_status",
        description = "Show SQLite tool configuration and runtime availability."
    )]
    async fn sqlite_status(&self) -> Json<TextOutput> {
        sqlite_status_output(&sqlite_status(&self.config))
    }

    #[tool(
        name = "sqlite_schema",
        description = "Inspect tables, views, indexes, and triggers for an allowed database."
    )]
    async fn sqlite_schema(
        &self,
        Parameters(request): Parameters<SqliteSchemaRequest>,
    ) -> Result<Json<SqliteRowsOutput>, ToolError> {
        let rows = sqlite_schema(&self.config, request.db_path.as_deref()).map_err(sqlite_error)?;
        Ok(sqlite_rows_output(&rows))
    }

    #[tool(
        name = "sqlite_select",
        description = "Run one read-only SELECT/WITH or safe PRAGMA statement."
    )]
    async fn sqlite_select(
        &self,
        Parameters(request): Parameters<SqliteSelectRequest>,
    ) -> Result<Json<SqliteRowsOutput>, ToolError> {
        let limit = request
            .limit
            .unwrap_or(self.config.sqlite_max_rows as usize);
        if limit == 0 || limit > self.config.sqlite_max_rows as usize {
            return Err(invalid_params(format!(
                "limit must be between 1 and {}",
                self.config.sqlite_max_rows
            )));
        }
        let rows = sqlite_select(
            &self.config,
            request.db_path.as_deref(),
            &request.sql,
            &request.params,
            limit,
        )
        .map_err(sqlite_error)?;
        Ok(sqlite_rows_output(&rows))
    }

    #[tool(
        name = "sqlite_preview_change",
        description = "Preview a bounded structured SQLite insert, update, or delete without writing."
    )]
    async fn sqlite_preview_change(
        &self,
        Parameters(request): Parameters<SqlitePreviewRequest>,
    ) -> Result<Json<SqlitePreviewOutput>, ToolError> {
        let payload = serde_json::to_vec(&request.change).expect("SQLite change is serializable");
        if payload.len() > 32_000 {
            return Err(invalid_params(
                "sqlite_preview_change payload is larger than 32000 bytes",
            ));
        }
        let preview = self
            .sqlite_changes
            .lock()
            .map_err(|_| internal_error("SQLite change store is unavailable"))
            .and_then(|mut store| {
                sqlite_preview_change(
                    &self.config,
                    &mut store,
                    request.db_path.as_deref(),
                    request.change,
                )
                .map_err(sqlite_error)
            })?;
        Ok(sqlite_preview_output(&preview))
    }

    #[tool(
        name = "sqlite_confirm_change",
        description = "Apply a pending SQLite change created by sqlite_preview_change."
    )]
    async fn sqlite_confirm_change(
        &self,
        Parameters(request): Parameters<SqliteConfirmRequest>,
    ) -> Result<Json<SqliteConfirmOutput>, ToolError> {
        let result = self
            .sqlite_changes
            .lock()
            .map_err(|_| internal_error("SQLite change store is unavailable"))
            .and_then(|mut store| {
                sqlite_confirm_change(&mut store, &request.action_id).map_err(sqlite_error)
            })?;
        Ok(sqlite_confirm_output(&result))
    }
}

fn sqlite_error(error: SqliteError) -> ToolError {
    let message = error.to_string();
    match error {
        SqliteError::Database(_) | SqliteError::Path(_) => ToolError::internal_error(message, None),
        _ => ToolError::invalid_params(message, None),
    }
}

fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::invalid_params(message.into(), None)
}

fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::internal_error(message.into(), None)
}
