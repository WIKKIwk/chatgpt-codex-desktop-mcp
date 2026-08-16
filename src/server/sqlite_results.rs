use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value};

use super::core_results::{StructuredOutput, TextOutput};
use crate::{
    redaction::{redact_text, redact_value},
    sqlite::{
        JsonRow, SqliteStatus,
        change::{SqliteConfirmResult, SqlitePreviewResult},
    },
};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct SqliteRowsOutput {
    pub(crate) result: String,
    pub(crate) rows: Vec<JsonRow>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct SqlitePreviewOutput {
    pub(crate) result: String,
    pub(crate) action_id: String,
    pub(crate) requires_approval: bool,
    #[serde(rename = "beforeRows")]
    pub(crate) before_rows: Vec<JsonRow>,
    pub(crate) diff: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct SqliteConfirmOutput {
    pub(crate) result: String,
    pub(crate) applied: bool,
    pub(crate) action_id: String,
    pub(crate) change_type: String,
    pub(crate) table: String,
}

pub(crate) fn sqlite_status_output(status: &SqliteStatus) -> StructuredOutput<TextOutput> {
    let result = serde_json::to_string_pretty(status).expect("SQLite status is serializable");
    super::core_results::text_output(result)
}

pub(crate) fn sqlite_rows_output(rows: &[JsonRow]) -> StructuredOutput<SqliteRowsOutput> {
    let rows = redact_rows(rows);
    let result = serde_json::to_string_pretty(&rows).expect("SQLite rows are serializable");
    let value = SqliteRowsOutput {
        result: result.clone(),
        rows,
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn sqlite_preview_output(
    preview: &SqlitePreviewResult,
) -> StructuredOutput<SqlitePreviewOutput> {
    let action_id = redact_text(&preview.action.id);
    let diff = redact_text(&preview.diff);
    let before_rows = redact_rows(&preview.before_rows);
    let result = redact_text(&format!("Pending sqlite change: {}\n\n{}", action_id, diff));
    let value = SqlitePreviewOutput {
        result: result.clone(),
        action_id,
        requires_approval: true,
        before_rows,
        diff,
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn sqlite_confirm_output(
    result: &SqliteConfirmResult,
) -> StructuredOutput<SqliteConfirmOutput> {
    let action_id = redact_text(&result.action_id);
    let change_type = redact_text(result.change_type);
    let table = redact_text(&result.table);
    let result_text = redact_text(&format!(
        "Applied sqlite change: {} ({} on {})",
        action_id, change_type, table
    ));
    let value = SqliteConfirmOutput {
        result: result_text.clone(),
        applied: result.applied,
        action_id,
        change_type,
        table,
    };
    StructuredOutput::new(result_text, value)
}

fn redact_rows(rows: &[JsonRow]) -> Vec<JsonRow> {
    rows.iter()
        .map(|row| match redact_value(Value::Object(row.clone())) {
            Value::Object(redacted) => redacted,
            _ => Map::new(),
        })
        .collect()
}
