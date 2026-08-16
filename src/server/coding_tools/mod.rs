mod edits;
mod execution;
mod project;
mod results;
mod shared;

use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use self::results::{
    ApplyPatchOutput, ManagedProcessOutput, OpenProjectOutput, ProcessOutput, ProjectStateOutput,
    ReadFilesOutput, TextOutput,
};
use super::core_results::{Json, StructuredOutput};
use super::handler::ForgeHandler;
use super::tool_error::ToolError;
use crate::{edit::Change, project::ProjectCheckKind};

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct OpenProjectRequest {
    pub(super) path: String,
    #[serde(rename = "treeDepth", default)]
    pub(super) tree_depth: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectStateRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct SearchCodeRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    pub(super) pattern: String,
    #[serde(default)]
    pub(super) path: Option<String>,
    #[serde(default)]
    pub(super) include: Option<String>,
    #[serde(default)]
    pub(super) exclude: Option<String>,
    #[serde(rename = "caseSensitive", default)]
    pub(super) case_sensitive: Option<bool>,
    #[serde(rename = "contextLines", default)]
    pub(super) context_lines: Option<usize>,
    #[serde(rename = "maxMatches", default)]
    pub(super) max_matches: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ReadFilesRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    pub(super) paths: Vec<String>,
    #[serde(rename = "maxBytesPerFile", default)]
    pub(super) max_bytes_per_file: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ApplyPatchRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    pub(super) changes: Vec<Change>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct RunProjectCheckRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) kind: Option<ProjectCheckKind>,
    #[serde(rename = "timeoutSeconds", default)]
    pub(super) timeout_seconds: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct RunProjectCommandRequest {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: String,
    pub(super) command: String,
    #[serde(default)]
    pub(super) args: Vec<String>,
    #[serde(rename = "workingDirectory", default)]
    pub(super) working_directory: Option<String>,
    #[serde(rename = "timeoutSeconds", default)]
    pub(super) timeout_seconds: Option<usize>,
    #[serde(rename = "maxBytes", default)]
    pub(super) max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum ManageProcessAction {
    Start,
    Read,
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ManageProcessRequest {
    pub(super) action: ManageProcessAction,
    #[serde(rename = "workspaceId", default)]
    pub(super) workspace_id: Option<String>,
    #[serde(rename = "processId", default)]
    pub(super) process_id: Option<String>,
    #[serde(default)]
    pub(super) command: Option<String>,
    #[serde(default)]
    pub(super) args: Vec<String>,
    #[serde(rename = "workingDirectory", default)]
    pub(super) working_directory: Option<String>,
    #[serde(rename = "timeoutSeconds", default)]
    pub(super) timeout_seconds: Option<usize>,
    #[serde(rename = "maxBytes", default)]
    pub(super) max_bytes: Option<usize>,
}

#[tool_router(router = coding_tool_router, vis = "pub(crate)")]
impl ForgeHandler {
    #[tool(
        name = "open_project",
        description = "Open a Desktop project and return its type, Git status, and a small tree."
    )]
    async fn open_project(
        &self,
        Parameters(request): Parameters<OpenProjectRequest>,
    ) -> Result<Json<OpenProjectOutput>, ToolError> {
        project::open_project(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "project_state",
        description = "Return Git status plus unstaged and staged diff summaries for a project."
    )]
    async fn project_state(
        &self,
        Parameters(request): Parameters<ProjectStateRequest>,
    ) -> Result<Json<ProjectStateOutput>, ToolError> {
        project::project_state(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "search_code",
        description = "Search project code with optional include, exclude, and nearby context."
    )]
    async fn search_code(
        &self,
        Parameters(request): Parameters<SearchCodeRequest>,
    ) -> Result<Json<TextOutput>, ToolError> {
        project::search_code(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "read_files",
        description = "Read several known project files in one bounded call."
    )]
    async fn read_files(
        &self,
        Parameters(request): Parameters<ReadFilesRequest>,
    ) -> Result<Json<ReadFilesOutput>, ToolError> {
        project::read_files(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "apply_patch",
        description = "Apply bounded create or edit changes in coding or full access mode."
    )]
    async fn apply_patch(
        &self,
        Parameters(request): Parameters<ApplyPatchRequest>,
    ) -> Result<Json<ApplyPatchOutput>, ToolError> {
        edits::apply_patch(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "run_project_check",
        description = "Run a safe project-native test, check, lint, build, or format check."
    )]
    async fn run_project_check(
        &self,
        Parameters(request): Parameters<RunProjectCheckRequest>,
    ) -> Result<Json<ProcessOutput>, ToolError> {
        execution::run_project_check(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "run_project_command",
        description = "Run a specific allowed project command with structured argv and no shell."
    )]
    async fn run_project_command(
        &self,
        Parameters(request): Parameters<RunProjectCommandRequest>,
    ) -> Result<Json<ProcessOutput>, ToolError> {
        execution::run_project_command(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }

    #[tool(
        name = "manage_process",
        description = "Start, read, or stop an allowed long-running development process."
    )]
    async fn manage_process(
        &self,
        Parameters(request): Parameters<ManageProcessRequest>,
    ) -> Result<Json<ManagedProcessOutput>, ToolError> {
        execution::manage_process(self, request)
            .await
            .map(|value| StructuredOutput::new(value.result.clone(), value))
    }
}
