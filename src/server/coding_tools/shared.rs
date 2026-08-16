use std::path::Path;

use tokio::fs;

use super::super::tool_error::ToolError;
use crate::workspace::SearchError;

pub(super) fn bounded(
    value: Option<usize>,
    default: usize,
    minimum: usize,
    maximum: usize,
    name: &str,
) -> Result<usize, ToolError> {
    let value = value.unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        return Err(invalid_params(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    Ok(value)
}

pub(super) async fn ensure_directory(path: &Path, display_path: &str) -> Result<(), ToolError> {
    let metadata = fs::metadata(path)
        .await
        .map_err(|error| internal_error(error.to_string()))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(invalid_params(format!(
            "workingDirectory is not a directory: {display_path}"
        )))
    }
}

pub(super) fn process_stdout_or_error(
    result: &crate::process::ProcessResult,
    empty_fallback: &str,
) -> String {
    if result.exit_code == Some(0) {
        let stdout = result.stdout.trim_end();
        if stdout.is_empty() {
            empty_fallback.to_owned()
        } else {
            stdout.to_owned()
        }
    } else if result.stderr.trim().is_empty() {
        if result.stdout.trim().is_empty() {
            format!("(command failed with exit {:?})", result.exit_code)
        } else {
            result.stdout.trim_end().to_owned()
        }
    } else {
        result.stderr.trim_end().to_owned()
    }
}

pub(super) fn search_error(error: SearchError) -> ToolError {
    let message = error.to_string();
    match error {
        SearchError::InvalidGlob { .. } | SearchError::BuildGlob(_) | SearchError::Matcher(_) => {
            invalid_params(message)
        }
        SearchError::Io(_) => internal_error(message),
    }
}

pub(super) fn edit_error(error: crate::edit::EditError) -> ToolError {
    match error {
        crate::edit::EditError::Validation(message)
        | crate::edit::EditError::UnknownAction(message) => invalid_params(message),
        crate::edit::EditError::Io(error) => internal_error(error.to_string()),
    }
}

pub(super) fn is_codex_executable(command: &str) -> bool {
    command
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|value| value.eq_ignore_ascii_case("codex"))
}

pub(super) fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::invalid_params(message.into(), None)
}

pub(super) fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::internal_error(message.into(), None)
}
