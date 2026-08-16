use std::path::Path;

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
                    "clientInfo": {"name": "process-tool-test", "version": "0.1.0"}
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

#[tokio::test]
async fn process_tools_execute_and_manage_structured_commands() {
    let temp = tempdir().expect("temporary directory");
    let root = Path::new(temp.path());
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
            serde_json::json!({"path": root.to_string_lossy()}),
        ))
        .await
        .expect("open workspace response");
    let opened = response_json(opened).await;
    let workspace_id = opened["result"]["structuredContent"]["workspaceId"]
        .as_str()
        .expect("workspace id")
        .to_owned();

    let executed = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            3,
            "exec_process",
            serde_json::json!({
                "workspaceId": workspace_id,
                "command": "git",
                "args": ["--version"],
                "workingDirectory": "."
            }),
        ))
        .await
        .expect("exec response");
    let executed_json = response_json(executed).await;
    let executed_data = &executed_json["result"]["structuredContent"];
    assert!(
        executed_data["result"]
            .as_str()
            .expect("process result")
            .contains("exit_code: 0")
    );
    assert!(
        executed_data["stdout"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("git")
    );
    assert_eq!(executed_data["exitCode"], 0);
    assert_eq!(executed_data["timedOut"], false);

    let started = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            4,
            "process_start",
            serde_json::json!({
                "workspaceId": workspace_id,
                "command": "git",
                "args": ["--version"],
                "timeoutSeconds": 30
            }),
        ))
        .await
        .expect("process start response");
    let started_json = response_json(started).await;
    let started_data = &started_json["result"]["structuredContent"];
    let process_id = started_data["process_id"]
        .as_str()
        .expect("process id")
        .to_owned();
    assert!(process_id.starts_with("proc_"));
    assert_eq!(started_data["command"], "git");
    assert_eq!(started_data["args"], serde_json::json!(["--version"]));

    let read = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            5,
            "process_read",
            serde_json::json!({"processId": process_id}),
        ))
        .await
        .expect("process read response");
    let read_json = response_json(read).await;
    let read_data = &read_json["result"]["structuredContent"];
    assert_eq!(read_data["process_id"], process_id);

    let stopped = router
        .oneshot(tool_call_request(
            &session_id,
            6,
            "process_stop",
            serde_json::json!({"processId": process_id}),
        ))
        .await
        .expect("process stop response");
    let stopped_json = response_json(stopped).await;
    let stopped_data = &stopped_json["result"]["structuredContent"];
    assert_eq!(stopped_data["process_id"], process_id);
    assert_eq!(stopped_data["running"], false);
}
