use super::core_results::{
    GitDiffOutput, Json, ManagedProcessOutput, OpenWorkspaceOutput, ProcessOutput, ReadFileOutput,
    TextOutput, git_diff_output, open_workspace_output, process_output, read_file_output,
    text_output,
};
use super::edit_results::{EditConfirmOutput, EditPreviewOutput};
use super::edit_tools::{ConfirmEditRequest, PreviewEditRequest};
use super::process_tools::{ExecProcessRequest, ProcessIdRequest};
use super::tool_error::ToolError;
use crate::config::{Config, ToolProfile};
use crate::edit::EditStore;
use crate::process::ManagedProcessStore;
use crate::workspace::{
    DenyRules, GitDiffOptions, ResolvedWorkspace, SearchError, SearchOptions, WorkspaceRegistry,
    assert_not_denied, find_files, git_diff as run_git_diff, git_status as run_git_status,
    list_directory, project_tree, read_file_capped, relative_display_path, search_text_files,
};
use crate::{codex::CodexBridge, sqlite::SqliteChangeStore};
use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Deserialize, JsonSchema)]
struct OpenWorkspaceRequest {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListDirectoryRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadFileRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchFilesRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(rename = "caseSensitive", default)]
    case_sensitive: Option<bool>,
    #[serde(rename = "contextLines", default)]
    context_lines: Option<usize>,
    #[serde(rename = "maxMatches", default)]
    max_matches: Option<usize>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    exclude: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindFilesRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(rename = "maxResults", default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectTreeRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    depth: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitStatusRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GitDiffRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    #[serde(default)]
    staged: Option<bool>,
    #[serde(default)]
    path: Option<String>,
    #[serde(rename = "statOnly", default)]
    stat_only: Option<bool>,
    #[serde(rename = "maxBytes", default)]
    max_bytes: Option<usize>,
}

#[derive(Clone)]
pub struct ForgeHandler {
    pub(crate) config: Config,
    pub(crate) workspaces: Arc<Mutex<WorkspaceRegistry>>,
    pub(crate) processes: Arc<Mutex<ManagedProcessStore>>,
    pub(crate) edits: Arc<Mutex<EditStore>>,
    pub(crate) sqlite_changes: Arc<Mutex<SqliteChangeStore>>,
    pub(crate) codex: Arc<CodexBridge>,
}

impl ForgeHandler {
    pub fn new(
        config: Config,
        workspaces: Arc<Mutex<WorkspaceRegistry>>,
        processes: Arc<Mutex<ManagedProcessStore>>,
        edits: Arc<Mutex<EditStore>>,
        sqlite_changes: Arc<Mutex<SqliteChangeStore>>,
        codex: Arc<CodexBridge>,
    ) -> Self {
        Self {
            config,
            workspaces,
            processes,
            edits,
            sqlite_changes,
            codex,
        }
    }
}

#[tool_router(router = core_tool_router)]
impl ForgeHandler {
    #[tool(
        name = "local_status",
        description = "Return server metadata and enabled feature status."
    )]
    async fn local_status(&self) -> Json<TextOutput> {
        text_output(super::status::local_status(&self.config))
    }

    #[tool(
        name = "open_workspace",
        description = "Open a local project directory under the configured allowed roots."
    )]
    async fn open_workspace(
        &self,
        Parameters(request): Parameters<OpenWorkspaceRequest>,
    ) -> Result<Json<OpenWorkspaceOutput>, ToolError> {
        let workspace = self
            .workspaces
            .lock()
            .map_err(|_| internal_error("workspace registry is unavailable"))?
            .open(&request.path)
            .map_err(|error| invalid_params(error.to_string()))?;
        Ok(open_workspace_output(
            workspace.id,
            workspace.root.display().to_string(),
        ))
    }

    #[tool(
        name = "list_dir",
        description = "List files and subdirectories in an opened workspace."
    )]
    async fn list_dir(
        &self,
        Parameters(request): Parameters<ListDirectoryRequest>,
    ) -> Result<Json<TextOutput>, ToolError> {
        let path = request.path.as_deref().unwrap_or(".");
        let resolved = self.resolve_workspace(&request.workspace_id, path)?;
        let output = list_directory(&resolved.absolute_path)
            .map_err(|error| internal_error(error.to_string()))?;
        Ok(text_output(output))
    }

    #[tool(
        name = "read_file",
        description = "Read a UTF-8 text file in an opened workspace with a byte cap."
    )]
    async fn read_file(
        &self,
        Parameters(request): Parameters<ReadFileRequest>,
    ) -> Result<Json<ReadFileOutput>, ToolError> {
        let resolved = self.resolve_workspace(&request.workspace_id, &request.path)?;
        let content =
            read_file_capped(&resolved.absolute_path, self.config.max_read_bytes as usize)
                .map_err(|error| internal_error(error.to_string()))?;
        let path = relative_display_path(&resolved.workspace.root, &resolved.absolute_path);
        Ok(read_file_output(path, content.content, content.truncated))
    }

    #[tool(
        name = "search_files",
        description = "Search text content inside an opened workspace."
    )]
    async fn search_files(
        &self,
        Parameters(request): Parameters<SearchFilesRequest>,
    ) -> Result<Json<TextOutput>, ToolError> {
        let path = request.path.as_deref().unwrap_or(".");
        let resolved = self.resolve_workspace(&request.workspace_id, path)?;
        let options = SearchOptions {
            pattern: request.pattern,
            case_sensitive: request.case_sensitive.unwrap_or(false),
            context_lines: bounded_value(request.context_lines, 0, 0, 20, "contextLines")?,
            max_matches: bounded_value(request.max_matches, 1_000, 1, 5_000, "maxMatches")?,
            include: request.include,
            exclude: request.exclude,
        };
        let deny_rules = DenyRules::new(&self.config.deny_globs)
            .map_err(|error| internal_error(error.to_string()))?;
        let output = search_text_files(
            &resolved.workspace.root,
            &resolved.absolute_path,
            &deny_rules,
            self.config.max_read_bytes as usize,
            self.config.max_output_bytes as usize,
            &options,
        )
        .map_err(search_error)?;
        let output = if output.is_empty() {
            "(no matches)".to_owned()
        } else {
            output
        };
        Ok(text_output(output))
    }

    #[tool(
        name = "find_files",
        description = "Find files by glob pattern in an opened workspace."
    )]
    async fn find_files(
        &self,
        Parameters(request): Parameters<FindFilesRequest>,
    ) -> Result<Json<TextOutput>, ToolError> {
        let path = request.path.as_deref().unwrap_or(".");
        let resolved = self.resolve_workspace(&request.workspace_id, path)?;
        let max_results = bounded_value(request.max_results, 100, 1, 500, "maxResults")?;
        let deny_rules = DenyRules::new(&self.config.deny_globs)
            .map_err(|error| internal_error(error.to_string()))?;
        let results = find_files(
            &resolved.workspace.root,
            &resolved.absolute_path,
            &deny_rules,
            &request.pattern,
            max_results,
        )
        .map_err(search_error)?;
        let output = if results.is_empty() {
            "(no matching files)".to_owned()
        } else {
            results.join("\n")
        };
        Ok(text_output(output))
    }

    #[tool(
        name = "project_tree",
        description = "Show a depth-limited project tree for an opened workspace."
    )]
    async fn project_tree(
        &self,
        Parameters(request): Parameters<ProjectTreeRequest>,
    ) -> Result<Json<TextOutput>, ToolError> {
        let path = request.path.as_deref().unwrap_or(".");
        let resolved = self.resolve_workspace(&request.workspace_id, path)?;
        let depth = bounded_value(request.depth, 3, 1, 5, "depth")?;
        let deny_rules = DenyRules::new(&self.config.deny_globs)
            .map_err(|error| internal_error(error.to_string()))?;
        let output = project_tree(
            &resolved.workspace.root,
            &resolved.absolute_path,
            &deny_rules,
            depth,
            self.config.max_output_bytes as usize,
        )
        .map_err(search_error)?;
        let output = if output.is_empty() {
            "(empty)".to_owned()
        } else {
            output
        };
        Ok(text_output(output))
    }

    #[tool(
        name = "git_status",
        description = "Run git status --short in an opened workspace."
    )]
    async fn git_status(
        &self,
        Parameters(request): Parameters<GitStatusRequest>,
    ) -> Result<Json<ProcessOutput>, ToolError> {
        let resolved = self.resolve_workspace(&request.workspace_id, ".")?;
        let result = run_git_status(
            &resolved.workspace.root,
            self.config.max_output_bytes as usize,
        )
        .await;
        Ok(process_output(&result))
    }

    #[tool(
        name = "git_diff",
        description = "Review staged or unstaged git diff output in an opened workspace."
    )]
    async fn git_diff(
        &self,
        Parameters(request): Parameters<GitDiffRequest>,
    ) -> Result<Json<GitDiffOutput>, ToolError> {
        let workspace = self.resolve_workspace(&request.workspace_id, ".")?;
        let path = request
            .path
            .map(|path| {
                self.resolve_workspace(&request.workspace_id, &path)
                    .map(|resolved| {
                        relative_display_path(&resolved.workspace.root, &resolved.absolute_path)
                    })
            })
            .transpose()?;
        let staged = request.staged.unwrap_or(false);
        let stat_only = request.stat_only.unwrap_or(false);
        let max_bytes = request
            .max_bytes
            .unwrap_or(self.config.max_output_bytes as usize);
        if max_bytes == 0 || max_bytes > self.config.max_output_bytes as usize {
            return Err(invalid_params(format!(
                "maxBytes must be between 1 and {}",
                self.config.max_output_bytes
            )));
        }
        let result = run_git_diff(
            &workspace.workspace.root,
            GitDiffOptions {
                staged,
                path: path.clone(),
                stat_only,
                max_bytes,
            },
        )
        .await;
        Ok(git_diff_output(&result, staged, path, stat_only))
    }

    #[tool(
        name = "preview_edit",
        description = "Preview one or more bounded file edits without writing them."
    )]
    async fn preview_edit(
        &self,
        Parameters(request): Parameters<PreviewEditRequest>,
    ) -> Result<Json<EditPreviewOutput>, ToolError> {
        super::edit_tools::preview_edit(self, request).await
    }

    #[tool(
        name = "confirm_edit",
        description = "Apply a pending file edit created by preview_edit."
    )]
    async fn confirm_edit(
        &self,
        Parameters(request): Parameters<ConfirmEditRequest>,
    ) -> Result<Json<EditConfirmOutput>, ToolError> {
        super::edit_tools::confirm_edit(self, request).await
    }

    #[tool(
        name = "exec_process",
        description = "Run a short foreground local executable using structured argv and no shell."
    )]
    async fn exec_process(
        &self,
        Parameters(request): Parameters<ExecProcessRequest>,
    ) -> Result<Json<ProcessOutput>, ToolError> {
        super::process_tools::exec_process(self, request).await
    }

    #[tool(
        name = "process_start",
        description = "Start a long-running local executable using structured argv and no shell."
    )]
    async fn process_start(
        &self,
        Parameters(request): Parameters<ExecProcessRequest>,
    ) -> Result<Json<ManagedProcessOutput>, ToolError> {
        super::process_tools::process_start(self, request).await
    }

    #[tool(
        name = "process_read",
        description = "Read the current output and exit state for a managed process."
    )]
    async fn process_read(
        &self,
        Parameters(request): Parameters<ProcessIdRequest>,
    ) -> Result<Json<ManagedProcessOutput>, ToolError> {
        super::process_tools::process_read(self, request)
    }

    #[tool(name = "process_stop", description = "Stop a running managed process.")]
    async fn process_stop(
        &self,
        Parameters(request): Parameters<ProcessIdRequest>,
    ) -> Result<Json<ManagedProcessOutput>, ToolError> {
        super::process_tools::process_stop(self, request)
    }

    pub(crate) fn resolve_workspace(
        &self,
        workspace_id: &str,
        path: &str,
    ) -> Result<ResolvedWorkspace, ToolError> {
        let resolved = self
            .workspaces
            .lock()
            .map_err(|_| internal_error("workspace registry is unavailable"))?
            .resolve(workspace_id, path)
            .map_err(|error| invalid_params(error.to_string()))?;
        assert_not_denied(
            &resolved.absolute_path,
            &resolved.workspace.root,
            &self.config.deny_globs,
        )
        .map_err(|error| invalid_params(error.to_string()))?;
        Ok(resolved)
    }
}

impl ForgeHandler {
    #[rustfmt::skip]
    pub(crate) fn tool_router(&self) -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        let mut router = if self.config.tool_profile == ToolProfile::Coding { Self::coding_tool_router() } else { Self::core_tool_router() + Self::sqlite_tool_router() + Self::web_tool_router() };
        if self.config.tool_profile == ToolProfile::Coding && self.config.codex_bridge_enabled { router += Self::codex_tool_router(); }
        if self.config.tool_profile == ToolProfile::Legacy && !self.config.sqlite_tools_enabled { for name in ["sqlite_schema", "sqlite_select", "sqlite_preview_change", "sqlite_confirm_change"] { router.remove_route(name); } }
        if self.config.tool_profile == ToolProfile::Legacy && !self.config.web_tools_enabled { for name in ["web_search", "web_fetch"] { router.remove_route(name); } }
        super::tool_metadata::apply(&mut router, self.config.max_output_bytes, self.config.sqlite_max_rows);
        router
    }
}

fn invalid_params(message: String) -> ToolError {
    ToolError::invalid_params(message, None)
}

fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::internal_error(message.into(), None)
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

fn search_error(error: SearchError) -> ToolError {
    let message = error.to_string();
    match error {
        SearchError::InvalidGlob { .. } | SearchError::BuildGlob(_) | SearchError::Matcher(_) => {
            invalid_params(message)
        }
        SearchError::Io(_) => internal_error(message),
    }
}
