use std::fs;

use axum::body::Body;
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use super::build_router;
use crate::config::{AccessMode, Config, SearchProvider, ToolProfile};

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![std::env::temp_dir()],
        deny_globs: Vec::new(),
        access_mode: AccessMode::Coding,
        tool_profile: ToolProfile::Legacy,
        stateless_mcp_fallback: false,
        codex_bridge_enabled: false,
        codex_command: "codex".to_owned(),
        codex_max_sessions: 4,
        codex_request_timeout_ms: 120_000,
        max_read_bytes: 200_000,
        max_output_bytes: 200_000,
        web_tools_enabled: false,
        search_provider: SearchProvider::None,
        searxng_url: String::new(),
        web_max_bytes: 200_000,
        web_timeout_ms: 15_000,
        sqlite_tools_enabled: false,
        sqlite_allowed_dbs: Vec::new(),
        sqlite_max_rows: 100,
    }
}

fn initialize_request() -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("host", "127.0.0.1:3333")
        .header("MCP-Protocol-Version", "2025-11-25")
        .body(Body::from(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "edit-tool-test", "version": "0.1.0"}
                }
            })
            .to_string(),
        ))
        .expect("initialize request")
}

fn tool_call_request(
    session_id: &str,
    id: i32,
    name: &str,
    arguments: Value,
) -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("host", "127.0.0.1:3333")
        .header("MCP-Protocol-Version", "2025-11-25")
        .header("mcp-session-id", session_id)
        .body(Body::from(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            })
            .to_string(),
        ))
        .expect("tool call request")
}

async fn response_json(response: axum::response::Response) -> Value {
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    if content_type.starts_with("text/event-stream") {
        let text = String::from_utf8(body.to_vec()).expect("event stream utf8");
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: ").filter(|data| !data.is_empty()))
            .expect("event data");
        serde_json::from_str(data).expect("event json")
    } else {
        serde_json::from_slice(&body).expect("json response")
    }
}

fn text(result: &Value) -> &str {
    result["result"]["content"][0]["text"]
        .as_str()
        .expect("tool text")
}

#[tokio::test]
async fn preview_and_confirm_edit_write_only_after_confirmation() {
    let temp = tempdir().expect("temporary directory");
    fs::write(temp.path().join("sample.txt"), "before\n").expect("sample file");
    let router = build_router(test_config());
    let initialize = router
        .clone()
        .oneshot(initialize_request())
        .await
        .expect("initialize response");
    let session_id = initialize
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("session id")
        .to_owned();

    let opened = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            2,
            "open_workspace",
            serde_json::json!({"path": temp.path().to_string_lossy()}),
        ))
        .await
        .expect("open response");
    let opened = response_json(opened).await;
    let workspace_id = opened["result"]["structuredContent"]["workspaceId"]
        .as_str()
        .expect("workspace id")
        .to_owned();

    let preview = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            3,
            "preview_edit",
            serde_json::json!({
                "workspaceId": workspace_id,
                "changes": [{
                    "type": "replace_text",
                    "path": "sample.txt",
                    "oldText": "before",
                    "newText": "after"
                }]
            }),
        ))
        .await
        .expect("preview response");
    let preview_json = response_json(preview).await;
    let preview_text = text(&preview_json).to_owned();
    assert!(preview_text.starts_with("Pending edit: edit_"));
    assert!(preview_text.contains("-before"));
    let preview_data = &preview_json["result"]["structuredContent"];
    assert_eq!(preview_data["requires_approval"], true);
    assert_eq!(preview_data["changes"][0]["path"], "sample.txt");
    assert_eq!(preview_data["changes"][0]["type"], "replace_text");
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).expect("unchanged file"),
        "before\n"
    );
    let action_id = preview_data["action_id"]
        .as_str()
        .expect("action id")
        .to_owned();

    let confirm = router
        .oneshot(tool_call_request(
            &session_id,
            4,
            "confirm_edit",
            serde_json::json!({"actionId": action_id}),
        ))
        .await
        .expect("confirm response");
    let confirm_json = response_json(confirm).await;
    let confirm_text = text(&confirm_json);
    assert!(confirm_text.contains("Applied 1 change(s)"));
    let confirm_data = &confirm_json["result"]["structuredContent"];
    assert_eq!(confirm_data["applied"], true);
    assert_eq!(confirm_data["action_id"], action_id);
    assert_eq!(confirm_data["changeCount"], 1);
    assert_eq!(
        fs::read_to_string(temp.path().join("sample.txt")).expect("updated file"),
        "after\n"
    );
}
