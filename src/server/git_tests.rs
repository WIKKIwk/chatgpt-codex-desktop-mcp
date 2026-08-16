use std::{fs, path::Path, process::Command};

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
        access_mode: AccessMode::Review,
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

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
                    "clientInfo": {"name": "git-tool-test", "version": "0.1.0"}
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
async fn git_tools_report_status_and_diff_through_mcp() {
    let temp = tempdir().expect("temporary directory");
    git(temp.path(), &["init", "--quiet"]);
    git(temp.path(), &["config", "user.email", "test@example.com"]);
    git(temp.path(), &["config", "user.name", "Rust Test"]);
    fs::write(temp.path().join("sample.txt"), "before\n").expect("sample file");
    fs::write(temp.path().join("search.txt"), "Needle\nneedle\ncontext\n").expect("search file");
    git(temp.path(), &["add", "sample.txt", "search.txt"]);
    git(temp.path(), &["commit", "--quiet", "-m", "initial"]);
    fs::write(temp.path().join("sample.txt"), "after\n").expect("updated file");

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
        .expect("open workspace response");
    let opened = response_json(opened).await;
    let opened_data = &opened["result"]["structuredContent"];
    let workspace_id = opened_data["workspaceId"]
        .as_str()
        .expect("workspace id")
        .to_owned();
    assert_eq!(opened_data["workspaceId"], workspace_id);
    let canonical_root = fs::canonicalize(temp.path()).expect("canonical root");
    assert_eq!(
        opened_data["root"],
        canonical_root.to_string_lossy().to_string()
    );

    let read = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            3,
            "read_file",
            serde_json::json!({"workspaceId": workspace_id, "path": "sample.txt"}),
        ))
        .await
        .expect("read file response");
    let read = response_json(read).await;
    let read_data = &read["result"]["structuredContent"];
    assert_eq!(read_data["path"], "sample.txt");
    assert_eq!(read_data["truncated"], false);
    assert_eq!(read_data["result"], "after\n");

    let searched = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            4,
            "search_files",
            serde_json::json!({
                "workspaceId": workspace_id,
                "pattern": "Needle",
                "path": ".",
                "caseSensitive": true,
                "contextLines": 1,
                "maxMatches": 1
            }),
        ))
        .await
        .expect("search files response");
    let searched = response_json(searched).await;
    let search_text = searched["result"]["structuredContent"]["result"]
        .as_str()
        .expect("search text");
    assert!(search_text.contains("search.txt:1: Needle"));
    assert!(!search_text.contains("search.txt:2: needle"));

    let found = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            5,
            "find_files",
            serde_json::json!({
                "workspaceId": workspace_id,
                "pattern": "*.txt",
                "maxResults": 1
            }),
        ))
        .await
        .expect("find files response");
    let found = response_json(found).await;
    let found_text = found["result"]["structuredContent"]["result"]
        .as_str()
        .expect("find text");
    assert_eq!(found_text.lines().count(), 1);

    let status = router
        .clone()
        .oneshot(tool_call_request(
            &session_id,
            6,
            "git_status",
            serde_json::json!({"workspaceId": workspace_id}),
        ))
        .await
        .expect("git status response");
    let status = response_json(status).await;
    let status_text = status["result"]["content"][0]["text"]
        .as_str()
        .expect("status text");
    assert!(status_text.contains("sample.txt"));
    assert!(status_text.contains("exit_code: 0"));
    let status_data = &status["result"]["structuredContent"];
    assert!(
        status_data["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("sample.txt")
    );
    assert_eq!(status_data["exitCode"], 0);
    assert_eq!(status_data["timedOut"], false);

    let diff = router
        .oneshot(tool_call_request(
            &session_id,
            7,
            "git_diff",
            serde_json::json!({
                "workspaceId": workspace_id,
                "path": "sample.txt",
                "maxBytes": 10_000
            }),
        ))
        .await
        .expect("git diff response");
    let diff = response_json(diff).await;
    let diff_text = diff["result"]["content"][0]["text"]
        .as_str()
        .expect("diff text");
    assert!(diff_text.contains("sample.txt"));
    assert!(diff_text.contains("+after"));
    let diff_data = &diff["result"]["structuredContent"];
    assert!(
        diff_data["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("+after")
    );
    assert_eq!(diff_data["exitCode"], 0);
    assert_eq!(diff_data["staged"], false);
    assert_eq!(diff_data["path"], "sample.txt");
    assert_eq!(diff_data["statOnly"], false);
}
