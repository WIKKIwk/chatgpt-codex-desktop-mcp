use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::codex_results::{CodexSessionOutput, codex_session_output};
use super::core_results::Json;
use super::handler::ForgeHandler;
use super::tool_error::ToolError;
use crate::codex::CodexSessionMode;

#[derive(Debug, Deserialize, JsonSchema)]
struct StartCodexRequest {
    #[serde(rename = "workspaceId")]
    workspace_id: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SendCodexRequest {
    #[serde(rename = "sessionId")]
    session_id: String,
    message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadCodexRequest {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "waitSeconds", default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StopCodexRequest {
    #[serde(rename = "sessionId")]
    session_id: String,
}

#[tool_router(router = codex_tool_router, vis = "pub(crate)")]
impl ForgeHandler {
    #[tool(
        name = "codex_start_session",
        description = "Start a local delegated Codex app-server session scoped to an opened workspace."
    )]
    async fn codex_start_session(
        &self,
        Parameters(request): Parameters<StartCodexRequest>,
    ) -> Result<Json<CodexSessionOutput>, ToolError> {
        if !self.config.codex_bridge_enabled {
            return Err(invalid_params(
                "Codex bridge is disabled. Set CTM_CODEX_BRIDGE=1 to enable it.",
            ));
        }
        let mode = parse_mode(request.mode.as_deref())?;
        let workspace = self.resolve_workspace(&request.workspace_id, ".")?;
        let snapshot = self
            .codex
            .start_session(
                &workspace.workspace.root.to_string_lossy(),
                request.prompt,
                mode,
            )
            .await
            .map_err(bridge_error)?;
        Ok(codex_session_output(&snapshot))
    }

    #[tool(
        name = "codex_send_message",
        description = "Send a follow-up instruction to an existing delegated Codex session."
    )]
    async fn codex_send_message(
        &self,
        Parameters(request): Parameters<SendCodexRequest>,
    ) -> Result<Json<CodexSessionOutput>, ToolError> {
        let snapshot = self
            .codex
            .send_message(&request.session_id, &request.message)
            .await
            .map_err(bridge_error)?;
        Ok(codex_session_output(&snapshot))
    }

    #[tool(
        name = "codex_read_response",
        description = "Read the latest delegated Codex response and streamed progress."
    )]
    async fn codex_read_response(
        &self,
        Parameters(request): Parameters<ReadCodexRequest>,
    ) -> Result<Json<CodexSessionOutput>, ToolError> {
        let wait_seconds = request.wait_seconds.unwrap_or(30);
        if wait_seconds > 60 {
            return Err(invalid_params("waitSeconds must be between 0 and 60"));
        }
        let snapshot = self
            .codex
            .read_response(&request.session_id, wait_seconds)
            .await
            .map_err(bridge_error)?;
        Ok(codex_session_output(&snapshot))
    }

    #[tool(
        name = "codex_stop_session",
        description = "Interrupt the active turn and stop a delegated Codex session."
    )]
    async fn codex_stop_session(
        &self,
        Parameters(request): Parameters<StopCodexRequest>,
    ) -> Result<Json<CodexSessionOutput>, ToolError> {
        let snapshot = self
            .codex
            .stop_session(&request.session_id)
            .await
            .map_err(bridge_error)?;
        Ok(codex_session_output(&snapshot))
    }
}

fn parse_mode(value: Option<&str>) -> Result<CodexSessionMode, ToolError> {
    match value.unwrap_or("review") {
        "review" => Ok(CodexSessionMode::Review),
        "write" => Ok(CodexSessionMode::Write),
        other => Err(invalid_params(format!(
            "mode must be review or write, got {other}"
        ))),
    }
}

fn bridge_error(message: String) -> ToolError {
    if message.starts_with("Could not start")
        || message.starts_with("Codex app-server")
        || message.starts_with("Codex request timed out")
    {
        ToolError::internal_error(message, None)
    } else {
        invalid_params(message)
    }
}

fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::invalid_params(message.into(), None)
}
