use schemars::JsonSchema;
use serde::Deserialize;

use super::core_results::StructuredOutput;
use super::edit_results::{
    EditConfirmOutput, EditPreviewOutput, confirm_edit_output, preview_edit_output,
};
use super::handler::ForgeHandler;
use super::tool_error::ToolError;
use crate::edit::{Change, EditError, apply_changes, preview_changes};

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PreviewEditRequest {
    #[serde(rename = "workspaceId")]
    pub(crate) workspace_id: String,
    pub(crate) changes: Vec<Change>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ConfirmEditRequest {
    #[serde(rename = "actionId")]
    pub(crate) action_id: String,
}

pub(crate) async fn preview_edit(
    handler: &ForgeHandler,
    request: PreviewEditRequest,
) -> Result<StructuredOutput<EditPreviewOutput>, ToolError> {
    assert_payload(&request.changes)?;
    assert_change_paths(handler, &request.workspace_id, &request.changes)?;
    let workspace = handler.resolve_workspace(&request.workspace_id, ".")?;
    let diffs = preview_changes(&workspace.workspace.root, &request.changes)
        .await
        .map_err(edit_error)?;
    let pending = handler
        .edits
        .lock()
        .map_err(|_| internal_error("edit store is unavailable"))?
        .create(request.workspace_id, request.changes, diffs);
    Ok(preview_edit_output(&pending))
}

pub(crate) async fn confirm_edit(
    handler: &ForgeHandler,
    request: ConfirmEditRequest,
) -> Result<StructuredOutput<EditConfirmOutput>, ToolError> {
    let pending = handler
        .edits
        .lock()
        .map_err(|_| internal_error("edit store is unavailable"))?
        .take(&request.action_id)
        .map_err(edit_error)?;
    assert_change_paths(handler, &pending.workspace_id, &pending.changes)?;
    let workspace = handler.resolve_workspace(&pending.workspace_id, ".")?;
    apply_changes(&workspace.workspace.root, &pending.changes)
        .await
        .map_err(edit_error)?;
    Ok(confirm_edit_output(&pending.id, pending.changes.len()))
}

fn assert_change_paths(
    handler: &ForgeHandler,
    workspace_id: &str,
    changes: &[Change],
) -> Result<(), ToolError> {
    for change in changes {
        handler.resolve_workspace(workspace_id, change.path())?;
        if let Some(new_path) = change.new_path() {
            handler.resolve_workspace(workspace_id, new_path)?;
        }
    }
    Ok(())
}

fn assert_payload(changes: &[Change]) -> Result<(), ToolError> {
    if changes.is_empty() {
        return Err(invalid_params("preview_edit requires at least one change"));
    }
    if changes.len() > 20 {
        return Err(invalid_params(
            "preview_edit supports at most 20 changes per call",
        ));
    }
    let mut total_bytes = 0;
    for (index, change) in changes.iter().enumerate() {
        validate_change_shape(index, change)?;
        for value in change.text_values() {
            let bytes = value.len();
            total_bytes += bytes;
            if bytes > 32_000 {
                return Err(invalid_params(format!(
                    "preview_edit changes[{index}] contains text larger than 32000 bytes"
                )));
            }
        }
    }
    if total_bytes > 120_000 {
        return Err(invalid_params(
            "preview_edit payload is larger than 120000 bytes",
        ));
    }
    Ok(())
}

fn validate_change_shape(index: usize, change: &Change) -> Result<(), ToolError> {
    let complete = match change {
        Change::ReplaceText {
            old_text, new_text, ..
        } => old_text.is_some() && new_text.is_some(),
        Change::ReplaceRange {
            start_line,
            end_line,
            new_text,
            ..
        } => {
            start_line.is_some_and(|line| line > 0)
                && end_line.is_some_and(|line| line > 0)
                && new_text.is_some()
        }
        Change::InsertBefore { anchor, text, .. } => anchor.is_some() && text.is_some(),
        Change::InsertAfter {
            anchor_after, text, ..
        } => anchor_after.is_some() && text.is_some(),
        Change::Append { text, .. } | Change::Create { text, .. } => text.is_some(),
        Change::Overwrite { new_text, .. } => new_text.is_some(),
        Change::Rename { new_path, .. } => new_path.is_some(),
        Change::Delete { .. } => true,
    };
    if complete {
        Ok(())
    } else {
        Err(invalid_params(format!(
            "preview_edit changes[{index}] ({}) is missing one or more required fields",
            change.edit_type().as_str()
        )))
    }
}

fn edit_error(error: EditError) -> ToolError {
    match error {
        EditError::Validation(message) | EditError::UnknownAction(message) => {
            invalid_params(message)
        }
        EditError::Io(error) => internal_error(error.to_string()),
    }
}

fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::invalid_params(message.into(), None)
}

fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::internal_error(message.into(), None)
}
