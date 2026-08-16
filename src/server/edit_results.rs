use schemars::JsonSchema;
use serde::Serialize;

use super::core_results::StructuredOutput;
use crate::{edit::PendingEdit, redaction::redact_text};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct EditChangeOutput {
    pub(crate) path: String,
    #[serde(rename = "type")]
    pub(crate) edit_type: String,
    pub(crate) diff: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct EditPreviewOutput {
    pub(crate) result: String,
    #[serde(rename = "action_id")]
    pub(crate) action_id: String,
    pub(crate) requires_approval: bool,
    pub(crate) changes: Vec<EditChangeOutput>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct EditConfirmOutput {
    pub(crate) result: String,
    pub(crate) applied: bool,
    #[serde(rename = "action_id")]
    pub(crate) action_id: String,
    #[serde(rename = "changeCount")]
    pub(crate) change_count: usize,
}

pub(crate) fn preview_edit_output(pending: &PendingEdit) -> StructuredOutput<EditPreviewOutput> {
    let changes = pending
        .diffs
        .iter()
        .map(|entry| EditChangeOutput {
            path: redact_text(&entry.path),
            edit_type: entry.edit_type.as_str().to_owned(),
            diff: redact_text(&entry.diff),
        })
        .collect::<Vec<_>>();
    let combined = format_combined_diff(&changes);
    let result = redact_text(&format!("Pending edit: {}\n\n{}", pending.id, combined));
    let value = EditPreviewOutput {
        result: result.clone(),
        action_id: redact_text(&pending.id),
        requires_approval: true,
        changes,
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn confirm_edit_output(
    action_id: &str,
    change_count: usize,
) -> StructuredOutput<EditConfirmOutput> {
    let result = redact_text(&format!(
        "Applied {change_count} change(s) from {action_id}"
    ));
    let value = EditConfirmOutput {
        result: result.clone(),
        applied: true,
        action_id: redact_text(action_id),
        change_count,
    };
    StructuredOutput::new(result, value)
}

fn format_combined_diff(changes: &[EditChangeOutput]) -> String {
    changes
        .iter()
        .map(|entry| {
            format!(
                "--- {} ({}) ---\n{}",
                entry.path, entry.edit_type, entry.diff
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
