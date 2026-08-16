use super::super::tool_error::ToolError;
use super::results::{ApplyPatchOutput, ChangeOutput};
use super::shared::{edit_error, invalid_params};
use super::{ApplyPatchRequest, ForgeHandler};
use crate::edit::{Change, EditType, apply_changes, preview_changes};
use crate::redaction::redact_text;

pub(super) async fn apply_patch(
    handler: &ForgeHandler,
    request: ApplyPatchRequest,
) -> Result<ApplyPatchOutput, ToolError> {
    if handler.config.access_mode == crate::AccessMode::Review {
        return Err(invalid_params(
            "apply_patch requires mcp.accessMode 'coding' or 'full'.",
        ));
    }
    assert_coding_payload(&request.changes)?;
    for change in &request.changes {
        if matches!(
            change.edit_type(),
            EditType::Overwrite | EditType::Rename | EditType::Delete
        ) {
            return Err(invalid_params(format!(
                "apply_patch does not support {} changes",
                change.edit_type().as_str()
            )));
        }
        validate_safe_shape(change)?;
        handler.resolve_workspace(&request.workspace_id, change.path())?;
        if let Some(new_path) = change.new_path() {
            handler.resolve_workspace(&request.workspace_id, new_path)?;
        }
    }
    let workspace = handler.resolve_workspace(&request.workspace_id, ".")?;
    let diffs = preview_changes(&workspace.workspace.root, &request.changes)
        .await
        .map_err(edit_error)?;
    apply_changes(&workspace.workspace.root, &request.changes)
        .await
        .map_err(edit_error)?;
    let changes = diffs
        .iter()
        .map(|entry| ChangeOutput {
            path: entry.path.clone(),
            edit_type: entry.edit_type.as_str().to_owned(),
            diff: redact_text(&entry.diff),
        })
        .collect::<Vec<_>>();
    let combined = changes
        .iter()
        .map(|entry| {
            format!(
                "--- {} ({}) ---\n{}",
                entry.path, entry.edit_type, entry.diff
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let result = redact_text(&format!(
        "Applied {} bounded change(s).\n\n{}",
        request.changes.len(),
        combined
    ));
    Ok(ApplyPatchOutput {
        result,
        applied: true,
        change_count: request.changes.len(),
        changes,
    })
}

fn validate_safe_shape(change: &Change) -> Result<(), ToolError> {
    let message = match change {
        Change::ReplaceText {
            old_text, new_text, ..
        } if old_text.is_some() && new_text.is_some() => None,
        Change::ReplaceRange {
            start_line,
            end_line,
            new_text,
            ..
        } if start_line.is_some_and(|line| line > 0)
            && end_line.is_some_and(|line| line > 0)
            && new_text.is_some() =>
        {
            None
        }
        Change::InsertBefore { anchor, text, .. }
            if anchor.as_deref().is_some_and(|value| !value.is_empty()) && text.is_some() =>
        {
            None
        }
        Change::InsertAfter {
            anchor_after, text, ..
        } if anchor_after
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            && text.is_some() =>
        {
            None
        }
        Change::Append { text, .. } | Change::Create { text, .. } if text.is_some() => None,
        _ => Some(format!(
            "{} change is missing one or more required fields",
            change.edit_type().as_str()
        )),
    };
    match message {
        Some(message) => Err(invalid_params(message)),
        None => Ok(()),
    }
}

fn assert_coding_payload(changes: &[Change]) -> Result<(), ToolError> {
    if changes.is_empty() || changes.len() > 20 {
        return Err(invalid_params(
            "apply_patch supports between 1 and 20 changes per call",
        ));
    }
    let mut total_bytes = 0usize;
    for (index, change) in changes.iter().enumerate() {
        for value in change.text_values() {
            if value.len() > 32_000 {
                return Err(invalid_params(format!(
                    "changes[{index}] contains text larger than 32000 bytes"
                )));
            }
            total_bytes = total_bytes.saturating_add(value.len());
        }
    }
    if total_bytes > 120_000 {
        return Err(invalid_params(
            "apply_patch payload is larger than 120000 bytes",
        ));
    }
    Ok(())
}
