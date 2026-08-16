use std::sync::{Arc, Mutex};

use axum::http::{Method, Request, StatusCode, header::CONTENT_TYPE};
use axum::{
    body::{Body, to_bytes},
    response::Response,
};
use bytes::Bytes;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;
use tower::ServiceExt;

use crate::codex::CodexBridge;
use crate::config::Config;
use crate::edit::EditStore;
use crate::process::ManagedProcessStore;
use crate::sqlite::SqliteChangeStore;
use crate::workspace::WorkspaceRegistry;

use super::handler::ForgeHandler;
use super::session::{SESSION_TTL, TtlSessionManager};

const MAX_REQUEST_BODY_BYTES: usize = 4 * 1024 * 1024;
type McpService = StreamableHttpService<ForgeHandler, TtlSessionManager>;

#[derive(Clone)]
pub struct McpTransport {
    stateful: McpService,
    stateless: McpService,
    stateless_fallback: bool,
}

impl McpTransport {
    pub fn new(config: Config) -> Self {
        let stateful_handler_config = config.clone();
        let stateless_handler_config = config.clone();
        let workspaces = Arc::new(Mutex::new(WorkspaceRegistry::new(config.clone())));
        let processes = Arc::new(Mutex::new(ManagedProcessStore::new()));
        let edits = Arc::new(Mutex::new(EditStore::new()));
        let sqlite_changes = Arc::new(Mutex::new(SqliteChangeStore::new()));
        let codex = Arc::new(CodexBridge::new(config.clone()));
        let stateful_workspaces = workspaces.clone();
        let stateless_workspaces = workspaces;
        let stateful_processes = processes.clone();
        let stateless_processes = processes;
        let stateful_edits = edits.clone();
        let stateless_edits = edits;
        let stateful_sqlite_changes = sqlite_changes.clone();
        let stateless_sqlite_changes = sqlite_changes;
        let stateful_codex = codex.clone();
        let stateless_codex = codex;
        let stateful_config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(true)
            .with_json_response(true)
            .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES);
        let stateless_config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES);

        let stateful = StreamableHttpService::new(
            move || {
                Ok(ForgeHandler::new(
                    stateful_handler_config.clone(),
                    stateful_workspaces.clone(),
                    stateful_processes.clone(),
                    stateful_edits.clone(),
                    stateful_sqlite_changes.clone(),
                    stateful_codex.clone(),
                ))
            },
            TtlSessionManager::new(SESSION_TTL),
            stateful_config,
        );
        let stateless = StreamableHttpService::new(
            move || {
                Ok(ForgeHandler::new(
                    stateless_handler_config.clone(),
                    stateless_workspaces.clone(),
                    stateless_processes.clone(),
                    stateless_edits.clone(),
                    stateless_sqlite_changes.clone(),
                    stateless_codex.clone(),
                ))
            },
            TtlSessionManager::new(SESSION_TTL),
            stateless_config,
        );

        Self {
            stateful,
            stateless,
            stateless_fallback: config.stateless_mcp_fallback,
        }
    }

    pub async fn handle(&self, request: Request<Body>) -> Response {
        let (parts, body) = request.into_parts();
        let body = match to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
            Ok(body) => body,
            Err(_) => return json_error(StatusCode::PAYLOAD_TOO_LARGE, "Request body too large"),
        };
        let method = request_method(&body);
        let is_post = parts.method == Method::POST;
        let initialize_request = is_post && method.as_deref() == Some("initialize");
        let stateless_request = self.stateless_fallback && is_post && method.is_some();
        let has_session = parts.headers.contains_key("mcp-session-id");

        if has_session {
            let response = self.call(&self.stateful, parts.clone(), body.clone()).await;
            if response.status() == StatusCode::NOT_FOUND {
                if stateless_request {
                    return self.call(&self.stateless, parts, body).await;
                }
                return json_error(StatusCode::NOT_FOUND, "Unknown MCP session");
            }
            return response;
        }

        if initialize_request {
            return self.call(&self.stateful, parts, body).await;
        }
        if stateless_request {
            return self.call(&self.stateless, parts, body).await;
        }
        json_error(StatusCode::BAD_REQUEST, "No valid MCP session")
    }

    async fn call(
        &self,
        service: &McpService,
        parts: axum::http::request::Parts,
        body: Bytes,
    ) -> Response {
        let request = Request::from_parts(parts, Body::from(body));
        let response = service
            .clone()
            .oneshot(request)
            .await
            .expect("MCP service has an infallible error type");
        let (parts, body) = response.into_parts();
        Response::from_parts(parts, Body::new(body))
    }
}

fn request_method(body: &Bytes) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value.get("method")?.as_str().map(str::to_owned)
}

fn json_error(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({ "error": message }).to_string();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("valid error response")
}
