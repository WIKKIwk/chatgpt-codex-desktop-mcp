use axum::body::Body;
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use super::build_router;
use crate::config::{AccessMode, Config, SearchProvider, ToolProfile};

fn test_config(root: &std::path::Path) -> Config {
    Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![root.to_path_buf()],
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

fn request(
    method: &str,
    id: i32,
    session_id: Option<&str>,
    params: Value,
) -> axum::http::Request<Body> {
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("host", "127.0.0.1:3333")
        .header("MCP-Protocol-Version", "2025-11-25");
    if let Some(session_id) = session_id {
        builder = builder.header("mcp-session-id", session_id);
    }
    builder
        .body(Body::from(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            })
            .to_string(),
        ))
        .expect("request")
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
        .expect("body")
        .to_bytes();
    if content_type.starts_with("text/event-stream") {
        let text = String::from_utf8(body.to_vec()).expect("event stream");
        let data = text
            .lines()
            .find_map(|line| line.strip_prefix("data: ").filter(|data| !data.is_empty()))
            .expect("event data");
        serde_json::from_str(data).expect("event json")
    } else {
        serde_json::from_slice(&body).expect("json")
    }
}

#[tokio::test]
async fn tool_failures_are_visible_is_error_results() {
    let allowed = tempdir().expect("allowed root");
    let outside = tempdir().expect("outside root");
    let router = build_router(test_config(allowed.path()));
    let initialized = router
        .clone()
        .oneshot(request(
            "initialize",
            1,
            None,
            serde_json::json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "error-test", "version": "0.1"}
            }),
        ))
        .await
        .expect("initialize");
    let session_id = initialized
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("session id")
        .to_owned();

    let failed = response_json(
        router
            .oneshot(request(
                "tools/call",
                2,
                Some(&session_id),
                serde_json::json!({
                    "name": "open_workspace",
                    "arguments": {"path": outside.path()}
                }),
            ))
            .await
            .expect("tool response"),
    )
    .await;
    assert!(failed.get("error").is_none());
    assert_eq!(failed["result"]["isError"], true);
    assert!(
        failed["result"]["content"][0]["text"]
            .as_str()
            .expect("error text")
            .contains("allowed")
    );
}
