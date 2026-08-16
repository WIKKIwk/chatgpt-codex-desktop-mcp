use axum::body::Body;
use http_body_util::BodyExt;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use super::build_router;
use crate::config::{AccessMode, Config, SearchProvider, ToolProfile};

fn test_config(root: &std::path::Path, database: &std::path::Path) -> Config {
    Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![root.to_path_buf()],
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
        sqlite_tools_enabled: true,
        sqlite_allowed_dbs: vec![database.to_path_buf()],
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
                    "clientInfo": {"name": "sqlite-tool-test", "version": "0.1.0"}
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

fn tools_list_request(session_id: &str) -> axum::http::Request<Body> {
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
                "id": 7,
                "method": "tools/list",
                "params": {}
            })
            .to_string(),
        ))
        .expect("tools list request")
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
async fn sqlite_tools_return_structured_results_and_guard_writes() {
    let temp = tempdir().expect("temporary directory");
    let database = temp.path().join("sample.sqlite");
    let connection = Connection::open(&database).expect("database");
    connection
        .execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, token TEXT NOT NULL); INSERT INTO items (name, token) VALUES ('before', 'secret-value');",
        )
        .expect("seed database");
    drop(connection);

    let router = build_router(test_config(temp.path(), &database));
    let initialized = router
        .clone()
        .oneshot(initialize_request())
        .await
        .expect("initialize response");
    let session_id = initialized
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("session id")
        .to_owned();

    let listed = response_json(
        router
            .clone()
            .oneshot(tools_list_request(&session_id))
            .await
            .expect("tools list response"),
    )
    .await;
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    let select_tool = tools
        .iter()
        .find(|tool| tool["name"] == "sqlite_select")
        .expect("sqlite_select schema");
    assert_eq!(select_tool["title"], "SQLite select");
    assert_eq!(select_tool["annotations"]["readOnlyHint"], true);
    assert_eq!(
        select_tool["inputSchema"]["properties"]["limit"]["default"],
        100
    );
    assert_eq!(
        select_tool["inputSchema"]["properties"]["params"]["items"]["type"],
        serde_json::json!(["string", "number", "null"])
    );
    let preview_tool = tools
        .iter()
        .find(|tool| tool["name"] == "sqlite_preview_change")
        .expect("sqlite preview schema");
    assert_eq!(preview_tool["annotations"]["destructiveHint"], false);

    let status = response_json(
        router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                2,
                "sqlite_status",
                serde_json::json!({}),
            ))
            .await
            .expect("status response"),
    )
    .await;
    assert!(text(&status).contains("\"enabled\": true"));
    assert!(
        status["result"]["structuredContent"]["result"]
            .as_str()
            .expect("status structured result")
            .contains("\"maxRows\": 100")
    );

    let schema = response_json(
        router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                3,
                "sqlite_schema",
                serde_json::json!({"dbPath": database}),
            ))
            .await
            .expect("schema response"),
    )
    .await;
    assert!(
        schema["result"]["structuredContent"]["rows"]
            .as_array()
            .expect("schema rows")
            .iter()
            .any(|row| row["name"] == "items")
    );

    let selected = response_json(
        router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                4,
                "sqlite_select",
                serde_json::json!({
                    "dbPath": database,
                    "sql": "SELECT id, name, token FROM items",
                    "limit": 10
                }),
            ))
            .await
            .expect("select response"),
    )
    .await;
    let selected_row = &selected["result"]["structuredContent"]["rows"][0];
    assert_eq!(selected_row["name"], "before");
    assert_eq!(selected_row["token"], "[REDACTED]");

    let preview = response_json(
        router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                5,
                "sqlite_preview_change",
                serde_json::json!({
                    "dbPath": database,
                    "change": {
                        "type": "update",
                        "table": "items",
                        "set": {"name": "after"},
                        "where": [{"column": "id", "operator": "=", "value": 1}],
                        "limit": 1,
                        "expected": {"name": "before"}
                    }
                }),
            ))
            .await
            .expect("preview response"),
    )
    .await;
    let preview_data = &preview["result"]["structuredContent"];
    assert_eq!(preview_data["requires_approval"], true);
    assert_eq!(preview_data["beforeRows"][0]["name"], "before");
    let action_id = preview_data["action_id"]
        .as_str()
        .expect("sqlite action id")
        .to_owned();
    assert!(text(&preview).starts_with("Pending sqlite change: sqlite_"));

    let confirmed = response_json(
        router
            .oneshot(tool_call_request(
                &session_id,
                6,
                "sqlite_confirm_change",
                serde_json::json!({"actionId": action_id}),
            ))
            .await
            .expect("confirm response"),
    )
    .await;
    let confirmed_data = &confirmed["result"]["structuredContent"];
    assert_eq!(confirmed_data["applied"], true);
    assert_eq!(confirmed_data["action_id"], action_id);
    assert_eq!(confirmed_data["change_type"], "update");
    assert_eq!(confirmed_data["table"], "items");

    let connection = Connection::open(&database).expect("reopen database");
    let name: String = connection
        .query_row("SELECT name FROM items WHERE id = 1", [], |row| row.get(0))
        .expect("updated name");
    assert_eq!(name, "after");
}
