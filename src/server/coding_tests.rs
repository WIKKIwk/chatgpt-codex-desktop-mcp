use axum::body::Body;
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use super::app::build_router;
use crate::config::{AccessMode, Config, SearchProvider, ToolProfile};

fn config(root: &std::path::Path) -> Config {
    Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![root.to_path_buf()],
        deny_globs: Vec::new(),
        access_mode: AccessMode::Coding,
        tool_profile: ToolProfile::Coding,
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
            serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
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
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .filter(|value| !value.is_empty())
            })
            .expect("event data");
        serde_json::from_str(data).expect("event json")
    } else {
        serde_json::from_slice(&body).expect("json")
    }
}

#[tokio::test]
async fn coding_composite_tools_open_search_read_and_apply() {
    let temp = tempdir().expect("temporary directory");
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"coding-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest");
    std::fs::write(
        temp.path().join("sample.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .expect("source");
    let router = build_router(config(temp.path()));
    let initialized = router
        .clone()
        .oneshot(request(
            "initialize",
            1,
            None,
            serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"coding-test","version":"0.1"}}),
        ))
        .await
        .expect("initialize");
    let session_id = initialized
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("session id")
        .to_owned();

    let listed = response_json(
        router
            .clone()
            .oneshot(request(
                "tools/list",
                9,
                Some(&session_id),
                serde_json::json!({}),
            ))
            .await
            .expect("coding tools list"),
    )
    .await;
    let tool_names = listed["result"]["tools"]
        .as_array()
        .expect("coding tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"open_project"));
    assert!(tool_names.contains(&"apply_patch"));
    assert!(!tool_names.contains(&"open_workspace"));
    let open_tool = listed["result"]["tools"]
        .as_array()
        .expect("coding tools")
        .iter()
        .find(|tool| tool["name"] == "open_project")
        .expect("open_project schema");
    assert!(open_tool["outputSchema"].is_object());
    assert_eq!(open_tool["title"], "Open Desktop project");
    assert_eq!(open_tool["annotations"]["readOnlyHint"], true);
    assert_eq!(open_tool["annotations"]["openWorldHint"], false);
    assert_eq!(
        open_tool["inputSchema"]["properties"]["treeDepth"]["default"],
        2
    );

    let search_tool = listed["result"]["tools"]
        .as_array()
        .expect("coding tools")
        .iter()
        .find(|tool| tool["name"] == "search_code")
        .expect("search_code schema");
    assert_eq!(
        search_tool["inputSchema"]["properties"]["path"]["type"],
        "string"
    );
    assert_eq!(
        search_tool["inputSchema"]["properties"]["contextLines"]["default"],
        2
    );
    assert_eq!(
        search_tool["inputSchema"]["properties"]["maxMatches"]["maximum"],
        1_000
    );

    let apply_tool = listed["result"]["tools"]
        .as_array()
        .expect("coding tools")
        .iter()
        .find(|tool| tool["name"] == "apply_patch")
        .expect("apply_patch schema");
    assert_eq!(
        apply_tool["inputSchema"]["properties"]["changes"]["minItems"],
        1
    );
    assert_eq!(
        apply_tool["inputSchema"]["properties"]["changes"]["maxItems"],
        20
    );

    let opened = response_json(
        router
            .clone()
            .oneshot(request(
                "tools/call",
                2,
                Some(&session_id),
                serde_json::json!({"name":"open_project","arguments":{"path":temp.path().to_string_lossy(),"treeDepth":2}}),
            ))
            .await
            .expect("open project"),
    )
    .await;
    let opened_data = &opened["result"]["structuredContent"];
    let opened_text = opened_data["result"].as_str().expect("open result");
    let workspace_id = opened_data["workspaceId"]
        .as_str()
        .expect("workspace id")
        .to_owned();
    assert_eq!(opened_data["projectType"], "rust");
    assert!(opened_text.contains("Project type: rust"));
    assert_eq!(opened["result"]["content"][0]["text"], opened_text);

    let searched = response_json(
        router
            .clone()
            .oneshot(request(
                "tools/call",
                3,
                Some(&session_id),
                serde_json::json!({"name":"search_code","arguments":{"workspaceId":workspace_id,"pattern":"println"}}),
            ))
            .await
            .expect("search code"),
    )
    .await;
    assert!(
        searched["result"]["structuredContent"]["result"]
            .as_str()
            .expect("search result")
            .contains("sample.rs:1")
    );

    let read = response_json(
        router
            .clone()
            .oneshot(request(
                "tools/call",
                4,
                Some(&session_id),
                serde_json::json!({"name":"read_files","arguments":{"workspaceId":workspace_id,"paths":["sample.rs","sample.rs"]}}),
            ))
            .await
            .expect("read files"),
    )
    .await;
    let read_files = read["result"]["structuredContent"]["files"]
        .as_array()
        .expect("read files");
    assert_eq!(read_files.len(), 1);
    assert_eq!(read_files[0]["path"], "sample.rs");
    assert!(
        read_files[0]["content"]
            .as_str()
            .expect("file content")
            .contains("println")
    );

    let applied = response_json(
        router
            .oneshot(request(
                "tools/call",
                5,
                Some(&session_id),
                serde_json::json!({"name":"apply_patch","arguments":{"workspaceId":workspace_id,"changes":[{"type":"replace_text","path":"sample.rs","oldText":"hello","newText":"goodbye"}]}}),
            ))
            .await
            .expect("apply patch"),
    )
    .await;
    assert_eq!(
        applied["result"]["structuredContent"]["applied"],
        Value::Bool(true)
    );
    assert_eq!(
        applied["result"]["structuredContent"]["changeCount"],
        Value::from(1)
    );
    assert!(
        std::fs::read_to_string(temp.path().join("sample.rs"))
            .expect("updated source")
            .contains("goodbye")
    );
}
