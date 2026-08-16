use std::borrow::Cow;

use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::{
    process::{
        ManagedProcessSnapshot, ProcessResult, format_managed_process, format_process_result,
    },
    redaction::redact_text,
};

pub(crate) struct StructuredOutput<T> {
    text: String,
    value: T,
}

impl<T> StructuredOutput<T> {
    pub(crate) fn new(text: String, value: T) -> Self {
        Self { text, value }
    }
}

// rmcp's tool macro recognizes the `Json<T>` spelling when generating an
// output schema. This alias keeps that schema support while the custom result
// implementation preserves the reference server's human-readable text.
pub(crate) type Json<T> = StructuredOutput<T>;

impl<T: JsonSchema> JsonSchema for StructuredOutput<T> {
    fn schema_name() -> Cow<'static, str> {
        T::schema_name()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        T::json_schema(generator)
    }
}

impl<T: Serialize + JsonSchema + 'static> IntoCallToolResult for StructuredOutput<T> {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        let value = serde_json::to_value(self.value).map_err(|error| {
            ErrorData::internal_error(
                format!("Failed to serialize structured content: {error}"),
                None,
            )
        })?;
        let mut result = CallToolResult::structured(value);
        result.content = vec![ContentBlock::text(self.text)];
        Ok(result.into())
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct TextOutput {
    pub(crate) result: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct OpenWorkspaceOutput {
    pub(crate) result: String,
    #[serde(rename = "workspaceId")]
    pub(crate) workspace_id: String,
    pub(crate) root: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct ReadFileOutput {
    pub(crate) result: String,
    pub(crate) path: String,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct ProcessOutput {
    pub(crate) result: String,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    #[serde(rename = "exitCode")]
    pub(crate) exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    pub(crate) timed_out: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct ManagedProcessOutput {
    pub(crate) result: String,
    pub(crate) process_id: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: String,
    pub(crate) running: bool,
    #[serde(rename = "startedAt")]
    pub(crate) started_at: u64,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<u64>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    #[serde(rename = "exitCode")]
    pub(crate) exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    pub(crate) timed_out: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct GitDiffOutput {
    pub(crate) result: String,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    #[serde(rename = "exitCode")]
    pub(crate) exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    pub(crate) timed_out: bool,
    pub(crate) staged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(rename = "statOnly")]
    pub(crate) stat_only: bool,
}

fn text_value(text: impl AsRef<str>) -> TextOutput {
    TextOutput {
        result: redact_text(text.as_ref()),
    }
}

pub(crate) fn text_output(text: impl AsRef<str>) -> StructuredOutput<TextOutput> {
    let value = text_value(text);
    StructuredOutput::new(value.result.clone(), value)
}

pub(crate) fn open_workspace_output(
    id: String,
    root: String,
) -> StructuredOutput<OpenWorkspaceOutput> {
    let result = redact_text(&format!("Opened workspace {id}\nRoot: {root}"));
    let value = OpenWorkspaceOutput {
        result: result.clone(),
        workspace_id: redact_text(&id),
        root: redact_text(&root),
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn read_file_output(
    path: String,
    content: String,
    truncated: bool,
) -> StructuredOutput<ReadFileOutput> {
    let result = redact_text(&content);
    let value = ReadFileOutput {
        result: result.clone(),
        path: redact_text(&path),
        truncated,
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn process_output(result: &ProcessResult) -> StructuredOutput<ProcessOutput> {
    let value = ProcessOutput {
        result: redact_text(&format_process_result(result)),
        stdout: redact_text(&result.stdout),
        stderr: redact_text(&result.stderr),
        exit_code: result.exit_code,
        timed_out: result.timed_out,
    };
    StructuredOutput::new(value.result.clone(), value)
}

pub(crate) fn managed_process_output(
    snapshot: &ManagedProcessSnapshot,
) -> StructuredOutput<ManagedProcessOutput> {
    let value = ManagedProcessOutput {
        result: redact_text(&format_managed_process(snapshot)),
        process_id: redact_text(&snapshot.id),
        command: redact_text(&snapshot.command),
        args: snapshot
            .args
            .iter()
            .map(|value| redact_text(value))
            .collect(),
        cwd: redact_text(&snapshot.cwd),
        running: snapshot.running,
        started_at: snapshot.started_at,
        finished_at: snapshot.finished_at,
        stdout: redact_text(&snapshot.stdout),
        stderr: redact_text(&snapshot.stderr),
        exit_code: snapshot.exit_code,
        timed_out: snapshot.timed_out,
    };
    StructuredOutput::new(value.result.clone(), value)
}

pub(crate) fn git_diff_output(
    result: &ProcessResult,
    staged: bool,
    path: Option<String>,
    stat_only: bool,
) -> StructuredOutput<GitDiffOutput> {
    let value = GitDiffOutput {
        result: redact_text(&format_process_result(result)),
        stdout: redact_text(&result.stdout),
        stderr: redact_text(&result.stderr),
        exit_code: result.exit_code,
        timed_out: result.timed_out,
        staged,
        path: path.map(|value| redact_text(&value)),
        stat_only,
    };
    StructuredOutput::new(value.result.clone(), value)
}
