use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use tokio::fs;

use super::core_results::{
    ManagedProcessOutput, ProcessOutput, StructuredOutput, managed_process_output, process_output,
};
use super::handler::ForgeHandler;
use super::tool_error::ToolError;
use crate::process::{ProcessInput, assert_process_allowed, run_process};

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExecProcessRequest {
    #[serde(rename = "workspaceId")]
    pub(crate) workspace_id: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(rename = "workingDirectory", default)]
    pub(crate) working_directory: Option<String>,
    #[serde(rename = "timeoutSeconds", default)]
    pub(crate) timeout_seconds: Option<usize>,
    #[serde(rename = "maxBytes", default)]
    pub(crate) max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ProcessIdRequest {
    #[serde(rename = "processId")]
    pub(crate) process_id: String,
}

pub(crate) async fn exec_process(
    handler: &ForgeHandler,
    request: ExecProcessRequest,
) -> Result<StructuredOutput<ProcessOutput>, ToolError> {
    let input = prepare_input(handler, request, 30, 300, "timeoutSeconds").await?;
    Ok(process_output(&run_process(input).await))
}

pub(crate) async fn process_start(
    handler: &ForgeHandler,
    request: ExecProcessRequest,
) -> Result<StructuredOutput<ManagedProcessOutput>, ToolError> {
    let input = prepare_input(handler, request, 300, 3_600, "timeoutSeconds").await?;
    let snapshot = handler
        .processes
        .lock()
        .map_err(|_| internal_error("process store is unavailable"))?
        .start(input);
    Ok(managed_process_output(&snapshot))
}

pub(crate) fn process_read(
    handler: &ForgeHandler,
    request: ProcessIdRequest,
) -> Result<StructuredOutput<ManagedProcessOutput>, ToolError> {
    let snapshot = handler
        .processes
        .lock()
        .map_err(|_| internal_error("process store is unavailable"))?
        .read(&request.process_id)
        .map_err(invalid_params)?;
    Ok(managed_process_output(&snapshot))
}

pub(crate) fn process_stop(
    handler: &ForgeHandler,
    request: ProcessIdRequest,
) -> Result<StructuredOutput<ManagedProcessOutput>, ToolError> {
    let snapshot = handler
        .processes
        .lock()
        .map_err(|_| internal_error("process store is unavailable"))?
        .stop(&request.process_id)
        .map_err(invalid_params)?;
    Ok(managed_process_output(&snapshot))
}

async fn prepare_input(
    handler: &ForgeHandler,
    request: ExecProcessRequest,
    timeout_default: usize,
    timeout_maximum: usize,
    timeout_name: &str,
) -> Result<ProcessInput, ToolError> {
    assert_process_allowed(&request.command, &request.args, handler.config.access_mode)
        .map_err(invalid_params)?;
    let working_directory = request.working_directory.as_deref().unwrap_or(".");
    let resolved = handler.resolve_workspace(&request.workspace_id, working_directory)?;
    ensure_directory(&resolved.absolute_path, working_directory).await?;
    let timeout_seconds = bounded_value(
        request.timeout_seconds,
        timeout_default,
        1,
        timeout_maximum,
        timeout_name,
    )?;
    let max_bytes = bounded_value(
        request.max_bytes,
        handler.config.max_output_bytes as usize,
        1,
        handler.config.max_output_bytes as usize,
        "maxBytes",
    )?;
    Ok(ProcessInput {
        command: request.command,
        args: request.args,
        cwd: resolved.absolute_path,
        timeout_ms: (timeout_seconds as u64) * 1_000,
        max_output_bytes: max_bytes,
    })
}

async fn ensure_directory(path: &Path, display_path: &str) -> Result<(), ToolError> {
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

fn bounded_value(
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

fn invalid_params(message: String) -> ToolError {
    ToolError::invalid_params(message, None)
}

fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::internal_error(message.into(), None)
}
