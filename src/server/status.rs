use serde_json::json;

use crate::config::{AccessMode, Config, SearchProvider, ToolProfile};

pub(super) fn local_status(config: &Config) -> String {
    json!({
        "ok": true,
        "name": "chatgpt-codex-tools-mcp",
        "version": env!("CARGO_PKG_VERSION"),
        "accessMode": access_mode_name(config.access_mode),
        "allowedRoots": config.allowed_roots,
        "toolProfile": tool_profile_name(config.tool_profile),
        "toolGroups": tool_groups(),
        "statelessMcpFallback": config.stateless_mcp_fallback,
        "maxReadBytes": config.max_read_bytes,
        "maxOutputBytes": config.max_output_bytes,
        "webToolsEnabled": config.web_tools_enabled,
        "searchProvider": search_provider_name(config.search_provider),
        "searxngConfigured": !config.searxng_url.is_empty(),
        "webMaxBytes": config.web_max_bytes,
        "webTimeoutMs": config.web_timeout_ms,
        "sqliteToolsEnabled": config.sqlite_tools_enabled,
        "sqliteAllowedDbs": config.sqlite_allowed_dbs,
        "sqliteMaxRows": config.sqlite_max_rows,
        "codexBridgeEnabled": config.codex_bridge_enabled,
        "codexMaxSessions": config.codex_max_sessions,
    })
    .to_string()
}

fn tool_groups() -> serde_json::Value {
    json!([
        {"type": "meta", "tools": ["local_status"]},
        {"type": "workspace", "tools": ["open_workspace"]},
        {"type": "read", "tools": ["list_dir", "read_file", "search_files", "find_files", "project_tree"]},
        {"type": "git", "tools": ["git_status", "git_diff"]},
        {"type": "edit_write", "tools": ["preview_edit", "confirm_edit"]},
        {"type": "exec", "tools": ["exec_process"]},
        {"type": "process", "tools": ["process_start", "process_read", "process_stop"]},
        {"type": "sqlite", "tools": ["sqlite_status", "sqlite_schema", "sqlite_select", "sqlite_preview_change", "sqlite_confirm_change"]},
        {"type": "web", "tools": ["web_status", "web_search", "web_fetch"]}
    ])
}

fn access_mode_name(value: AccessMode) -> &'static str {
    match value {
        AccessMode::Review => "review",
        AccessMode::Coding => "coding",
        AccessMode::Full => "full",
    }
}

fn tool_profile_name(value: ToolProfile) -> &'static str {
    match value {
        ToolProfile::Legacy => "legacy",
        ToolProfile::Coding => "coding",
    }
}

fn search_provider_name(value: SearchProvider) -> &'static str {
    match value {
        SearchProvider::None => "none",
        SearchProvider::Searxng => "searxng",
    }
}
