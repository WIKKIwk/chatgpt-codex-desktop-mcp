use rmcp::{ServerHandler, model::ServerCapabilities, tool_handler};

use super::handler::ForgeHandler;
use crate::config::ToolProfile;

const CODING_INSTRUCTIONS: &str = "Use open_project first for local coding work. Prefer search_code, read_files, project_state, and run_project_check over repeated low-level calls. Use apply_patch for bounded edits, then run the closest check. Never commit, push, deploy, migrate real data, access blocked secrets, or leave allowed roots.";
const LEGACY_INSTRUCTIONS: &str = "Open a workspace before using workspace-scoped tools. Use preview_edit then confirm_edit for file writes, structured argv for processes, and the configured safety boundaries for SQLite and web tools.";

#[tool_handler(router = self.tool_router())]
impl ServerHandler for ForgeHandler {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let instructions = match self.config.tool_profile {
            ToolProfile::Coding => CODING_INSTRUCTIONS,
            ToolProfile::Legacy => LEGACY_INSTRUCTIONS,
        };
        rmcp::model::ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "chatgpt-codex-tools-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(instructions)
    }
}
