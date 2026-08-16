use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{process::ChildStdin, sync::oneshot};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodexSessionMode {
    #[default]
    Review,
    Write,
}

impl CodexSessionMode {
    pub(crate) fn sandbox(self) -> &'static str {
        match self {
            Self::Review => "read-only",
            Self::Write => "workspace-write",
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Write => "write",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexSessionSnapshot {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "threadId")]
    pub thread_id: String,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    pub mode: String,
    pub status: String,
    pub running: bool,
    #[serde(rename = "turnId")]
    pub turn_id: Option<String>,
    #[serde(rename = "turnStatus")]
    pub turn_status: String,
    pub response: String,
    pub events: Vec<String>,
    pub error: Option<String>,
    #[serde(rename = "startedAt")]
    pub started_at: u64,
    #[serde(rename = "lastEventAt")]
    pub last_event_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSessionStatus {
    Starting,
    Running,
    Idle,
    Stopped,
    Failed,
}

impl CodexSessionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn running(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

pub(crate) struct CodexSession {
    pub session_id: String,
    pub thread_id: String,
    pub workspace_root: String,
    pub mode: CodexSessionMode,
    pub status: CodexSessionStatus,
    pub active_turn_id: Option<String>,
    pub turn_id: Option<String>,
    pub turn_status: String,
    pub response: String,
    pub events: Vec<String>,
    pub error: Option<String>,
    pub started_at: u64,
    pub last_event_at: u64,
}

pub(crate) struct BridgeState {
    pub stdin: Option<ChildStdin>,
    pub kill_tx: Option<oneshot::Sender<()>>,
    pub next_request_id: u64,
    pub pending: HashMap<u64, oneshot::Sender<Result<Value, String>>>,
    pub sessions: HashMap<String, CodexSession>,
}

impl BridgeState {
    pub fn new() -> Self {
        Self {
            stdin: None,
            kill_tx: None,
            next_request_id: 1,
            pending: HashMap::new(),
            sessions: HashMap::new(),
        }
    }
}

pub(crate) fn snapshot(session: &CodexSession) -> CodexSessionSnapshot {
    CodexSessionSnapshot {
        session_id: session.session_id.clone(),
        thread_id: session.thread_id.clone(),
        workspace_root: session.workspace_root.clone(),
        mode: session.mode.as_str().to_owned(),
        status: session.status.as_str().to_owned(),
        running: session.status.running(),
        turn_id: session.turn_id.clone(),
        turn_status: session.turn_status.clone(),
        response: session.response.clone(),
        events: session.events.clone(),
        error: session.error.clone(),
        started_at: session.started_at,
        last_event_at: session.last_event_at,
    }
}
