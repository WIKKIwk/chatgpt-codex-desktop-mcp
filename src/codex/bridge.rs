use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, oneshot},
    time::{sleep, timeout},
};
use uuid::Uuid;

use crate::config::Config;

use super::{
    model::{
        BridgeState, CodexSession, CodexSessionMode, CodexSessionSnapshot, CodexSessionStatus,
        snapshot,
    },
    protocol::reject_pending,
    runtime::spawn_server,
};

const MAX_PROMPT_BYTES: usize = 32_000;
const MAX_WAIT_SECONDS: u64 = 60;

pub struct CodexBridge {
    config: Config,
    state: Arc<Mutex<BridgeState>>,
    start_lock: Arc<Mutex<()>>,
    session_start_lock: Arc<Mutex<()>>,
}

impl CodexBridge {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: Arc::new(Mutex::new(BridgeState::new())),
            start_lock: Arc::new(Mutex::new(())),
            session_start_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn start_session(
        &self,
        workspace_root: &str,
        prompt: Option<String>,
        mode: CodexSessionMode,
    ) -> Result<CodexSessionSnapshot, String> {
        let _session_start_guard = self.session_start_lock.lock().await;
        if let Some(prompt) = prompt.as_deref() {
            assert_prompt(prompt, "prompt")?;
        }
        let active_count = {
            let state = self.state.lock().await;
            state
                .sessions
                .values()
                .filter(|session| session.status != CodexSessionStatus::Stopped)
                .count()
        };
        if active_count >= self.config.codex_max_sessions as usize {
            return Err(format!(
                "Codex session limit reached ({}). Stop an old session first.",
                self.config.codex_max_sessions
            ));
        }

        self.ensure_server().await?;
        let thread_result = self
            .request(
                "thread/start",
                json!({
                    "cwd": workspace_root,
                    "runtimeWorkspaceRoots": [workspace_root],
                    "approvalPolicy": "never",
                    "sandbox": mode.sandbox(),
                    "ephemeral": true,
                    "threadSource": "appServer",
                    "developerInstructions": developer_instructions(workspace_root, mode),
                }),
            )
            .await?;
        let thread_id = required_string(
            thread_result
                .get("thread")
                .and_then(Value::as_object)
                .and_then(|thread| thread.get("id")),
            "Codex did not return a thread id.",
        )?;
        let now = now_ms();
        let session_id = format!("codex_{}", Uuid::new_v4());
        {
            let mut state = self.state.lock().await;
            state.sessions.insert(
                session_id.clone(),
                CodexSession {
                    session_id: session_id.clone(),
                    thread_id,
                    workspace_root: workspace_root.to_owned(),
                    mode,
                    status: CodexSessionStatus::Starting,
                    active_turn_id: None,
                    turn_id: None,
                    turn_status: "starting".to_owned(),
                    response: String::new(),
                    events: Vec::new(),
                    error: None,
                    started_at: now,
                    last_event_at: now,
                },
            );
        }

        if let Some(prompt) = prompt {
            if let Err(error) = self.start_turn(&session_id, &prompt).await {
                self.mark_session_failed(&session_id, &error).await;
                return Err(error);
            }
        } else {
            self.set_idle(&session_id).await?;
        }
        self.snapshot(&session_id).await
    }

    pub async fn send_message(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<CodexSessionSnapshot, String> {
        assert_prompt(message, "message")?;
        let active_turn_id = {
            let mut state = self.state.lock().await;
            let session = state
                .sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("Unknown Codex session: {session_id}"))?;
            if session.status == CodexSessionStatus::Stopped {
                return Err(format!("Codex session is stopped: {session_id}"));
            }
            session.response.clear();
            session.events.clear();
            session.error = None;
            session.status = CodexSessionStatus::Running;
            session.turn_status = "inProgress".to_owned();
            session.last_event_at = now_ms();
            session.active_turn_id.clone()
        };

        let result = if let Some(active_turn_id) = active_turn_id.clone() {
            let thread_id = self.thread_id(session_id).await?;
            self.request(
                "turn/steer",
                json!({
                    "threadId": thread_id,
                    "expectedTurnId": active_turn_id,
                    "input": [{"type": "text", "text": message}],
                }),
            )
            .await
        } else {
            self.start_turn(session_id, message)
                .await
                .map(|_| Value::Null)
        };
        match result {
            Err(error) => {
                self.mark_session_failed(session_id, &error).await;
                return Err(error);
            }
            Ok(result) if active_turn_id.is_some() => {
                if let Some(turn_id) = result
                    .get("turnId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                {
                    let mut state = self.state.lock().await;
                    if let Some(session) = state.sessions.get_mut(session_id) {
                        session.turn_id = Some(turn_id);
                    }
                }
            }
            Ok(_) => {}
        }
        self.snapshot(session_id).await
    }

    pub async fn read_response(
        &self,
        session_id: &str,
        wait_seconds: u64,
    ) -> Result<CodexSessionSnapshot, String> {
        self.session_exists(session_id).await?;
        if wait_seconds > 0 && self.snapshot(session_id).await?.running {
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(wait_seconds.min(MAX_WAIT_SECONDS));
            while tokio::time::Instant::now() < deadline {
                if !self.snapshot(session_id).await?.running {
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }
        }
        self.snapshot(session_id).await
    }

    pub async fn stop_session(&self, session_id: &str) -> Result<CodexSessionSnapshot, String> {
        let (thread_id, active_turn_id) = {
            let state = self.state.lock().await;
            let session = state
                .sessions
                .get(session_id)
                .ok_or_else(|| format!("Unknown Codex session: {session_id}"))?;
            (session.thread_id.clone(), session.active_turn_id.clone())
        };
        if let Some(turn_id) = active_turn_id {
            let _ = self
                .request(
                    "turn/interrupt",
                    json!({"threadId": thread_id, "turnId": turn_id}),
                )
                .await;
        }
        let mut state = self.state.lock().await;
        let session = state
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Unknown Codex session: {session_id}"))?;
        session.active_turn_id = None;
        session.status = CodexSessionStatus::Stopped;
        session.turn_status = "interrupted".to_owned();
        session.last_event_at = now_ms();
        session.events.push("session_stopped".to_owned());
        if session.events.len() > 80 {
            session.events.drain(..session.events.len() - 80);
        }
        Ok(snapshot(session))
    }

    pub async fn close(&self) {
        let (kill_tx, mut state) = {
            let mut state = self.state.lock().await;
            for session in state.sessions.values_mut() {
                session.status = CodexSessionStatus::Stopped;
                session.active_turn_id = None;
            }
            (
                state.kill_tx.take(),
                std::mem::replace(&mut *state, BridgeState::new()),
            )
        };
        reject_pending(&mut state, "Codex bridge closed.");
        drop(state.stdin.take());
        if let Some(kill_tx) = kill_tx {
            let _ = kill_tx.send(());
        }
    }

    async fn ensure_server(&self) -> Result<(), String> {
        let _guard = self.start_lock.lock().await;
        if self.state.lock().await.stdin.is_some() {
            return Ok(());
        }
        spawn_server(
            &self.config.codex_command,
            self.state.clone(),
            self.config.max_output_bytes as usize,
        )
        .await?;
        if let Err(error) = self
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "forge-codex-bridge",
                        "title": "Forge Codex bridge",
                        "version": "1.0.0"
                    },
                    "capabilities": {"experimentalApi": true}
                }),
            )
            .await
        {
            self.close().await;
            return Err(format!(
                "Could not start the local Codex app-server: {error}"
            ));
        }
        self.notify("initialized", json!({})).await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let (id, receiver) = {
            let mut state = self.state.lock().await;
            let id = state.next_request_id;
            state.next_request_id += 1;
            let (sender, receiver) = oneshot::channel();
            state.pending.insert(id, sender);
            let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            let mut line = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
            line.push(b'\n');
            let write_result = state
                .stdin
                .as_mut()
                .ok_or_else(|| "Codex app-server is not running.".to_owned())?
                .write_all(&line)
                .await;
            if let Err(error) = write_result {
                state.pending.remove(&id);
                return Err(error.to_string());
            }
            (id, receiver)
        };
        match timeout(
            Duration::from_millis(self.config.codex_request_timeout_ms as u64),
            receiver,
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Codex app-server closed the response channel.".to_owned()),
            Err(_) => {
                self.state.lock().await.pending.remove(&id);
                Err(format!("Codex request timed out: {method}"))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut line = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
        line.push(b'\n');
        state
            .stdin
            .as_mut()
            .ok_or_else(|| "Codex app-server stdin is not writable.".to_owned())?
            .write_all(&line)
            .await
            .map_err(|error| error.to_string())
    }

    async fn start_turn(&self, session_id: &str, prompt: &str) -> Result<(), String> {
        let thread_id = self.thread_id(session_id).await?;
        let result = self
            .request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": prompt}],
                }),
            )
            .await?;
        let turn = result
            .get("turn")
            .and_then(Value::as_object)
            .ok_or_else(|| "Codex did not return a turn.".to_owned())?;
        let turn_id = required_string(turn.get("id"), "Codex did not return a turn id.")?;
        let mut state = self.state.lock().await;
        let session = state
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Unknown Codex session: {session_id}"))?;
        session.turn_id = Some(turn_id.clone());
        session.active_turn_id = Some(turn_id);
        session.status = CodexSessionStatus::Running;
        session.turn_status = string_or(turn.get("status"), "inProgress");
        session.last_event_at = now_ms();
        Ok(())
    }

    async fn thread_id(&self, session_id: &str) -> Result<String, String> {
        let state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .map(|session| session.thread_id.clone())
            .ok_or_else(|| format!("Unknown Codex session: {session_id}"))
    }

    async fn snapshot(&self, session_id: &str) -> Result<CodexSessionSnapshot, String> {
        let state = self.state.lock().await;
        state
            .sessions
            .get(session_id)
            .map(snapshot)
            .ok_or_else(|| format!("Unknown Codex session: {session_id}"))
    }

    async fn session_exists(&self, session_id: &str) -> Result<(), String> {
        self.thread_id(session_id).await.map(|_| ())
    }

    async fn set_idle(&self, session_id: &str) -> Result<(), String> {
        let mut state = self.state.lock().await;
        let session = state
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Unknown Codex session: {session_id}"))?;
        session.status = CodexSessionStatus::Idle;
        session.turn_status = "idle".to_owned();
        Ok(())
    }

    async fn mark_session_failed(&self, session_id: &str, error: &str) {
        let mut state = self.state.lock().await;
        if let Some(session) = state.sessions.get_mut(session_id) {
            session.status = CodexSessionStatus::Failed;
            session.error = Some(error.to_owned());
            session.active_turn_id = None;
            session.last_event_at = now_ms();
        }
    }
}

fn developer_instructions(root: &str, mode: CodexSessionMode) -> String {
    let editing = match mode {
        CodexSessionMode::Write => {
            "You may edit files only inside the selected workspace root when the delegated user request explicitly requires it."
        }
        CodexSessionMode::Review => {
            "This is a read-only review session; do not edit, create, rename, delete, commit, or push files."
        }
    };
    [
        "You are a delegated Codex worker controlled by a parent ChatGPT conversation through Forge.",
        &format!("Selected workspace root: {root}"),
        editing,
        "Never access secrets, credentials, private keys, .env files, or paths outside the selected workspace.",
        "Never deploy, publish, migrate real data, reboot the machine, or change system-wide settings.",
        "Return concise, evidence-based progress and final results to the parent conversation.",
    ]
    .join("\n")
}

fn assert_prompt(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty."));
    }
    if value.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "{label} is too large (max {MAX_PROMPT_BYTES} bytes)."
        ));
    }
    Ok(())
}

fn required_string(value: Option<&Value>, message: &str) -> Result<String, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| message.to_owned())
}

fn string_or(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
