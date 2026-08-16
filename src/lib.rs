pub mod codex;
pub mod config;
#[cfg(test)]
mod config_tests;
pub mod edit;
pub mod process;
pub mod project;
pub mod redaction;
pub mod server;
pub mod sqlite;
pub mod web;
pub mod workspace;

pub use config::{AccessMode, Config, ConfigError, SearchProvider, ToolProfile};
pub use workspace::{
    DenyRules, PathError, ResolvedWorkspace, Workspace, WorkspaceError, WorkspaceRegistry,
    assert_allowed_path, assert_not_denied, is_inside_root, relative_display_path,
    resolve_workspace_path,
};
