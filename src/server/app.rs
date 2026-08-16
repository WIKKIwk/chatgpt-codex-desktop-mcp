use axum::http::Request;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    response::Response,
    routing::{any, get},
};
use serde::{Deserialize, Serialize};

use crate::config::Config;

use super::transport::McpTransport;

#[derive(Clone)]
struct AppState {
    config: Config,
    mcp: McpTransport,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub name: String,
    pub version: String,
    #[serde(rename = "toolProfile")]
    pub tool_profile: String,
    #[serde(rename = "accessMode")]
    pub access_mode: String,
    #[serde(rename = "statelessMcpFallback")]
    pub stateless_mcp_fallback: bool,
    #[serde(rename = "codexBridgeEnabled")]
    pub codex_bridge_enabled: bool,
}

pub fn build_router(config: Config) -> Router {
    let mcp_transport = McpTransport::new(config.clone());

    Router::new()
        .route("/healthz", get(health))
        .route("/mcp", any(mcp_endpoint))
        .with_state(AppState {
            config,
            mcp: mcp_transport,
        })
}

async fn mcp_endpoint(State(state): State<AppState>, request: Request<Body>) -> Response {
    state.mcp.handle(request).await
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        name: "forge".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        tool_profile: tool_profile_name(state.config.tool_profile).to_owned(),
        access_mode: access_mode_name(state.config.access_mode).to_owned(),
        stateless_mcp_fallback: state.config.stateless_mcp_fallback,
        codex_bridge_enabled: state.config.codex_bridge_enabled,
    })
}

fn access_mode_name(value: crate::config::AccessMode) -> &'static str {
    match value {
        crate::config::AccessMode::Review => "review",
        crate::config::AccessMode::Coding => "coding",
        crate::config::AccessMode::Full => "full",
    }
}

fn tool_profile_name(value: crate::config::ToolProfile) -> &'static str {
    match value {
        crate::config::ToolProfile::Legacy => "legacy",
        crate::config::ToolProfile::Coding => "coding",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::{AccessMode, SearchProvider, ToolProfile};

    fn test_config(stateless_mcp_fallback: bool) -> Config {
        Config {
            host: "127.0.0.1".to_owned(),
            port: 3333,
            allowed_roots: vec![std::env::temp_dir()],
            deny_globs: Vec::new(),
            access_mode: AccessMode::Coding,
            tool_profile: ToolProfile::Legacy,
            stateless_mcp_fallback,
            codex_bridge_enabled: true,
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

    #[tokio::test]
    async fn health_endpoint_reports_configured_modes() {
        let response = build_router(test_config(false))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let health: HealthResponse = serde_json::from_slice(&body).expect("health json");
        assert!(health.ok);
        assert_eq!(health.name, "forge");
        assert_eq!(health.access_mode, "coding");
        assert_eq!(health.tool_profile, "legacy");
    }

    fn initialize_request() -> axum::http::Request<Body> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {
                    "name": "rust-transport-test",
                    "version": "0.1.0"
                }
            }
        });
        axum::http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("host", "127.0.0.1:3333")
            .header("MCP-Protocol-Version", "2025-11-25")
            .body(Body::from(payload.to_string()))
            .expect("initialize request")
    }

    fn tools_list_request(session_id: Option<&str>) -> axum::http::Request<Body> {
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
                    "id": 2,
                    "method": "tools/list",
                    "params": {}
                })
                .to_string(),
            ))
            .expect("tools list request")
    }

    fn tool_call_request(
        session_id: &str,
        id: i32,
        name: &str,
        arguments: serde_json::Value,
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

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
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
    async fn stateless_mcp_initialize_and_list_tools_work() {
        let router = build_router(test_config(true));
        let initialize = router
            .clone()
            .oneshot(initialize_request())
            .await
            .expect("initialize response");
        assert_eq!(initialize.status(), axum::http::StatusCode::OK);
        let initialize_json = response_json(initialize).await;
        assert_eq!(
            initialize_json["result"]["serverInfo"]["name"],
            "chatgpt-codex-tools-mcp"
        );

        let list_request = tools_list_request(None);
        let list = router
            .oneshot(list_request)
            .await
            .expect("tools list response");
        assert_eq!(list.status(), axum::http::StatusCode::OK);
        let list_json = response_json(list).await;
        let tools = list_json["result"]["tools"]
            .as_array()
            .expect("tools array");
        let tool_names = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"local_status"));
        assert!(tool_names.contains(&"open_workspace"));
        assert!(tool_names.contains(&"list_dir"));
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"sqlite_status"));
        assert!(!tool_names.contains(&"open_project"));
        assert!(!tool_names.contains(&"sqlite_schema"));
        assert!(!tool_names.contains(&"web_search"));
        for tool in tools {
            assert!(tool["title"].is_string(), "missing title: {tool}");
            assert!(
                tool["annotations"]["readOnlyHint"].is_boolean(),
                "missing readOnlyHint: {tool}"
            );
        }
        let list_dir = tools
            .iter()
            .find(|tool| tool["name"] == "list_dir")
            .expect("list_dir schema");
        assert_eq!(
            list_dir["inputSchema"]["properties"]["path"]["type"],
            "string"
        );
        assert_eq!(
            list_dir["inputSchema"]["properties"]["path"]["default"],
            "."
        );
    }

    #[tokio::test]
    async fn stateful_initialize_returns_session_header() {
        let response = build_router(test_config(false))
            .oneshot(initialize_request())
            .await
            .expect("initialize response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert!(response.headers().contains_key("mcp-session-id"));
    }

    #[tokio::test]
    async fn workspace_tools_open_list_and_read_files() {
        let temp = tempdir().expect("temporary directory");
        std::fs::write(
            temp.path().join("sample.txt"),
            "hello workspace\ntoken: \"secret-value\"",
        )
        .expect("sample file");
        let router = build_router(test_config(false));
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
        let initialize_json = response_json(initialize).await;
        assert_eq!(
            initialize_json["result"]["serverInfo"]["name"],
            "chatgpt-codex-tools-mcp"
        );

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
        let opened_json = response_json(opened).await;
        let workspace_id = opened_json["result"]["structuredContent"]["workspaceId"]
            .as_str()
            .expect("workspace id")
            .to_owned();

        let listed = router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                3,
                "list_dir",
                serde_json::json!({"workspaceId": workspace_id, "path": "."}),
            ))
            .await
            .expect("list directory response");
        let listed_json = response_json(listed).await;
        assert_eq!(
            listed_json["result"]["structuredContent"]["result"],
            "file sample.txt"
        );

        let read = router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                4,
                "read_file",
                serde_json::json!({"workspaceId": workspace_id, "path": "sample.txt"}),
            ))
            .await
            .expect("read file response");
        let read_json = response_json(read).await;
        let read_text = read_json["result"]["structuredContent"]["result"]
            .as_str()
            .expect("read text");
        assert!(read_text.contains("hello workspace"));
        assert!(!read_text.contains("secret-value"));
        assert!(read_text.contains("[REDACTED]"));

        let searched = router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                5,
                "search_files",
                serde_json::json!({
                    "workspaceId": workspace_id,
                    "pattern": "workspace",
                    "path": "."
                }),
            ))
            .await
            .expect("search response");
        let searched_json = response_json(searched).await;
        assert!(
            searched_json["result"]["structuredContent"]["result"]
                .as_str()
                .expect("search text")
                .contains("sample.txt:1: hello workspace")
        );

        let found = router
            .clone()
            .oneshot(tool_call_request(
                &session_id,
                6,
                "find_files",
                serde_json::json!({
                    "workspaceId": workspace_id,
                    "pattern": "*.txt"
                }),
            ))
            .await
            .expect("find response");
        let found_json = response_json(found).await;
        assert_eq!(
            found_json["result"]["structuredContent"]["result"],
            "sample.txt"
        );

        let tree = router
            .oneshot(tool_call_request(
                &session_id,
                7,
                "project_tree",
                serde_json::json!({"workspaceId": workspace_id, "depth": 2}),
            ))
            .await
            .expect("tree response");
        let tree_json = response_json(tree).await;
        assert!(
            tree_json["result"]["structuredContent"]["result"]
                .as_str()
                .expect("tree text")
                .contains("📄 sample.txt")
        );
    }

    #[tokio::test]
    async fn missing_session_is_rejected_without_fallback() {
        let response = build_router(test_config(false))
            .oneshot(tools_list_request(None))
            .await
            .expect("tools list response");
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&body).expect("error json");
        assert_eq!(error["error"], "No valid MCP session");
    }

    #[tokio::test]
    async fn unknown_session_uses_stateless_fallback() {
        let response = build_router(test_config(true))
            .oneshot(tools_list_request(Some("unknown-session")))
            .await
            .expect("tools list response");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let response_json = response_json(response).await;
        let tool_names = response_json["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"local_status"));
    }

    #[tokio::test]
    async fn unknown_session_is_rejected_without_fallback() {
        let response = build_router(test_config(false))
            .oneshot(tools_list_request(Some("unknown-session")))
            .await
            .expect("tools list response");
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&body).expect("error json");
        assert_eq!(error["error"], "Unknown MCP session");
    }
}
