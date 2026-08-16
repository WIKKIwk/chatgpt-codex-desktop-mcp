use super::super::tool_error::ToolError;
use super::results::{ManagedProcessOutput, ProcessOutput, managed_process_output, process_output};
use super::shared::{
    bounded, ensure_directory, internal_error, invalid_params, is_codex_executable,
};
use super::{
    ForgeHandler, ManageProcessAction, ManageProcessRequest, RunProjectCheckRequest,
    RunProjectCommandRequest,
};
use crate::{
    codex::{CodexBridge, CodexSessionMode},
    process::{ProcessInput, ProcessResult, assert_process_allowed, cap_text, run_process},
    project::select_project_check,
};

pub(super) async fn run_project_check(
    handler: &ForgeHandler,
    request: RunProjectCheckRequest,
) -> Result<ProcessOutput, ToolError> {
    let workspace = handler.resolve_workspace(&request.workspace_id, ".")?;
    let timeout_seconds = bounded(request.timeout_seconds, 300, 1, 900, "timeoutSeconds")?;
    let selected =
        select_project_check(&workspace.workspace.root, request.kind.unwrap_or_default())
            .await
            .map_err(invalid_params)?;
    assert_process_allowed(
        &selected.command,
        &selected.args,
        handler.config.access_mode,
    )
    .map_err(invalid_params)?;
    let command = selected.command.clone();
    let args = selected.args.clone();
    let label = selected.label.clone();
    let result = run_process(ProcessInput {
        command: selected.command,
        args: selected.args,
        cwd: workspace.workspace.root,
        timeout_ms: (timeout_seconds as u64) * 1_000,
        max_output_bytes: handler.config.max_output_bytes as usize,
    })
    .await;
    Ok(process_output(&result, command, args, Some(&label)))
}

pub(super) async fn run_project_command(
    handler: &ForgeHandler,
    request: RunProjectCommandRequest,
) -> Result<ProcessOutput, ToolError> {
    let timeout_seconds = bounded(request.timeout_seconds, 120, 1, 900, "timeoutSeconds")?;
    let max_bytes = bounded(
        request.max_bytes,
        handler.config.max_output_bytes as usize,
        1_000,
        handler.config.max_output_bytes as usize,
        "maxBytes",
    )?;
    let command = request.command.clone();
    let args = request.args.clone();
    let working_directory = request.working_directory.as_deref().unwrap_or(".");
    let resolved = handler.resolve_workspace(&request.workspace_id, working_directory)?;
    ensure_directory(&resolved.absolute_path, working_directory).await?;
    if handler.config.codex_bridge_enabled && is_codex_executable(&command) {
        let root = resolved.workspace.root.to_string_lossy().into_owned();
        return run_legacy_codex_command(
            &handler.codex,
            command,
            &root,
            &args,
            timeout_seconds,
            max_bytes,
        )
        .await;
    }
    assert_process_allowed(&command, &args, handler.config.access_mode).map_err(invalid_params)?;
    let result = run_process(ProcessInput {
        command: request.command,
        args: request.args,
        cwd: resolved.absolute_path,
        timeout_ms: (timeout_seconds as u64) * 1_000,
        max_output_bytes: max_bytes,
    })
    .await;
    Ok(process_output(&result, command, args, None))
}

pub(super) async fn manage_process(
    handler: &ForgeHandler,
    request: ManageProcessRequest,
) -> Result<ManagedProcessOutput, ToolError> {
    match request.action {
        ManageProcessAction::Read => {
            let process_id = request
                .process_id
                .ok_or_else(|| invalid_params("processId is required for action 'read'."))?;
            let snapshot = handler
                .processes
                .lock()
                .map_err(|_| internal_error("process store is unavailable"))?
                .read(&process_id)
                .map_err(invalid_params)?;
            Ok(managed_process_output(&snapshot))
        }
        ManageProcessAction::Stop => {
            let process_id = request
                .process_id
                .ok_or_else(|| invalid_params("processId is required for action 'stop'."))?;
            let snapshot = handler
                .processes
                .lock()
                .map_err(|_| internal_error("process store is unavailable"))?
                .stop(&process_id)
                .map_err(invalid_params)?;
            Ok(managed_process_output(&snapshot))
        }
        ManageProcessAction::Start => {
            let workspace_id = request
                .workspace_id
                .ok_or_else(|| invalid_params("workspaceId is required for action 'start'."))?;
            let command = request
                .command
                .ok_or_else(|| invalid_params("command is required for action 'start'."))?;
            let timeout_seconds =
                bounded(request.timeout_seconds, 600, 1, 3_600, "timeoutSeconds")?;
            let max_bytes = bounded(
                request.max_bytes,
                handler.config.max_output_bytes as usize,
                1_000,
                handler.config.max_output_bytes as usize,
                "maxBytes",
            )?;
            assert_process_allowed(&command, &request.args, handler.config.access_mode)
                .map_err(invalid_params)?;
            let working_directory = request.working_directory.as_deref().unwrap_or(".");
            let resolved = handler.resolve_workspace(&workspace_id, working_directory)?;
            ensure_directory(&resolved.absolute_path, working_directory).await?;
            let snapshot = handler
                .processes
                .lock()
                .map_err(|_| internal_error("process store is unavailable"))?
                .start(ProcessInput {
                    command,
                    args: request.args,
                    cwd: resolved.absolute_path,
                    timeout_ms: (timeout_seconds as u64) * 1_000,
                    max_output_bytes: max_bytes,
                });
            Ok(managed_process_output(&snapshot))
        }
    }
}

async fn run_legacy_codex_command(
    bridge: &CodexBridge,
    command: String,
    workspace_root: &str,
    args: &[String],
    timeout_seconds: usize,
    max_bytes: usize,
) -> Result<ProcessOutput, ToolError> {
    let (prompt, mode) = parse_legacy_codex_args(args)?;
    let started = bridge
        .start_session(workspace_root, prompt, mode)
        .await
        .map_err(bridge_error)?;
    let snapshot = if started.running {
        bridge
            .read_response(&started.session_id, timeout_seconds.min(60) as u64)
            .await
            .map_err(bridge_error)?
    } else {
        started
    };
    let stdout = [
        "Delegated to the local Codex bridge.".to_owned(),
        format!("session_id: {}", snapshot.session_id),
        format!("status: {}", snapshot.status),
        format!("mode: {}", snapshot.mode),
        if snapshot.response.is_empty() {
            "response: (session is ready; use codex_send_message or codex_read_response)".to_owned()
        } else {
            snapshot.response.clone()
        },
    ]
    .join("\n");
    let result = ProcessResult {
        stdout: cap_text(stdout, max_bytes),
        stderr: snapshot
            .error
            .as_deref()
            .map(|error| cap_text(error.to_owned(), max_bytes))
            .unwrap_or_default(),
        exit_code: if snapshot.error.is_some() {
            Some(1)
        } else if snapshot.running {
            None
        } else {
            Some(0)
        },
        timed_out: snapshot.running,
    };
    Ok(process_output(&result, command, args.to_vec(), None))
}

fn parse_legacy_codex_args(
    args: &[String],
) -> Result<(Option<String>, CodexSessionMode), ToolError> {
    let mut prompt = Vec::new();
    let mut mode = CodexSessionMode::Review;
    let mut saw_subcommand = false;
    for arg in args {
        if !saw_subcommand && matches!(arg.as_str(), "exec" | "e") {
            saw_subcommand = true;
            continue;
        }
        match arg.as_str() {
            "--write" | "--mode=write" => mode = CodexSessionMode::Write,
            "--review" | "--mode=review" => mode = CodexSessionMode::Review,
            "app-server" | "--listen" | "stdio://" => {
                return Err(invalid_params(
                    "Raw Codex app-server commands are not accepted through run_project_command. Use the Codex bridge tools.",
                ));
            }
            "--dangerously-bypass-approvals-and-sandbox" => {
                return Err(invalid_params(
                    "Unsafe Codex sandbox bypass is not accepted. The bridge scopes Codex to the selected workspace.",
                ));
            }
            _ => prompt.push(arg.clone()),
        }
    }
    let prompt = prompt.join(" ").trim().to_owned();
    Ok(((!prompt.is_empty()).then_some(prompt), mode))
}

fn bridge_error(message: String) -> ToolError {
    ToolError::internal_error(message, None)
}
