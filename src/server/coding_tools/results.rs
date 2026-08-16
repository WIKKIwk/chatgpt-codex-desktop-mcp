use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    process::{
        ManagedProcessSnapshot, ProcessResult, format_managed_process, format_process_result,
    },
    redaction::redact_text,
};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct TextOutput {
    pub(super) result: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct OpenProjectOutput {
    pub(super) result: String,
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    pub(super) root: String,
    #[serde(rename = "projectType")]
    pub(super) project_type: String,
    pub(super) tree: String,
    #[serde(rename = "gitStatus")]
    pub(super) git_status: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ProjectStateOutput {
    pub(super) result: String,
    pub(super) status: String,
    pub(super) unstaged: String,
    pub(super) staged: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct FileOutput {
    pub(super) path: String,
    pub(super) content: String,
    pub(super) truncated: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ReadFilesOutput {
    pub(super) result: String,
    pub(super) files: Vec<FileOutput>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ChangeOutput {
    pub(super) path: String,
    #[serde(rename = "type")]
    pub(super) edit_type: String,
    pub(super) diff: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ApplyPatchOutput {
    pub(super) result: String,
    pub(super) applied: bool,
    #[serde(rename = "changeCount")]
    pub(super) change_count: usize,
    pub(super) changes: Vec<ChangeOutput>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ProcessOutput {
    pub(super) result: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    #[serde(rename = "exitCode")]
    pub(super) exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    pub(super) timed_out: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(super) struct ManagedProcessOutput {
    pub(super) result: String,
    pub(super) process_id: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: String,
    pub(super) running: bool,
    #[serde(rename = "startedAt")]
    pub(super) started_at: u64,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    pub(super) finished_at: Option<u64>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    #[serde(rename = "exitCode")]
    pub(super) exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    pub(super) timed_out: bool,
}

pub(super) fn process_output(
    result: &ProcessResult,
    command: String,
    args: Vec<String>,
    label: Option<&str>,
) -> ProcessOutput {
    let formatted = format_process_result(result);
    let result_text = label.map_or(formatted.clone(), |label| format!("{label}\n{formatted}"));
    ProcessOutput {
        result: redact_text(&result_text),
        command,
        args,
        stdout: redact_text(&result.stdout),
        stderr: redact_text(&result.stderr),
        exit_code: result.exit_code,
        timed_out: result.timed_out,
    }
}

pub(super) fn managed_process_output(snapshot: &ManagedProcessSnapshot) -> ManagedProcessOutput {
    ManagedProcessOutput {
        result: redact_text(&format_managed_process(snapshot)),
        process_id: snapshot.id.clone(),
        command: snapshot.command.clone(),
        args: snapshot.args.clone(),
        cwd: snapshot.cwd.clone(),
        running: snapshot.running,
        started_at: snapshot.started_at,
        finished_at: snapshot.finished_at,
        stdout: redact_text(&snapshot.stdout),
        stderr: redact_text(&snapshot.stderr),
        exit_code: snapshot.exit_code,
        timed_out: snapshot.timed_out,
    }
}
