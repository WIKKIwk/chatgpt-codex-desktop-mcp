use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt;

use super::model::{BridgeState, CodexSessionStatus};

const MAX_EVENT_COUNT: usize = 80;
const RESPONSE_MARKER: &str = "\n[response truncated]\n";

pub(crate) async fn handle_message(
    state: &mut BridgeState,
    message: Value,
    max_output_bytes: usize,
) {
    if let Some(id) = message.get("id").and_then(Value::as_u64)
        && (message.get("result").is_some() || message.get("error").is_some())
    {
        if let Some(sender) = state.pending.remove(&id) {
            if let Some(error) = message.get("error") {
                let text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Codex RPC error {}.", error));
                let _ = sender.send(Err(text));
            } else {
                let _ = sender.send(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
            }
        }
        return;
    }

    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    if message.get("id").is_some_and(is_rpc_id) {
        handle_server_request(state, &message, method).await;
        return;
    }
    handle_notification(state, method, message.get("params"), max_output_bytes);
}

pub(crate) fn reject_pending(state: &mut BridgeState, message: &str) {
    for (_, sender) in state.pending.drain() {
        let _ = sender.send(Err(message.to_owned()));
    }
}

pub(crate) fn mark_sessions_failed(state: &mut BridgeState, message: &str) {
    for session in state.sessions.values_mut() {
        if session.status == CodexSessionStatus::Stopped {
            continue;
        }
        session.status = CodexSessionStatus::Failed;
        session.active_turn_id = None;
        session.error = Some(message.to_owned());
        session.last_event_at = now_ms();
    }
}

async fn handle_server_request(state: &mut BridgeState, message: &Value, method: &str) {
    let Some(id) = message.get("id").filter(|value| is_rpc_id(value)) else {
        return;
    };
    let response = if method.ends_with("/requestApproval") {
        json!({"jsonrpc": "2.0", "id": id, "result": {"decision": "decline"}})
    } else if method.ends_with("/requestUserInput") {
        json!({"jsonrpc": "2.0", "id": id, "result": {"answers": {}}})
    } else if method.ends_with("/elicitation") {
        json!({"jsonrpc": "2.0", "id": id, "result": {"action": "decline", "content": null}})
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Forge bridge does not proxy Codex server request: {method}")
            }
        })
    };
    write_json(state, &response).await;
}

fn is_rpc_id(value: &Value) -> bool {
    value.is_string() || value.is_number()
}

fn handle_notification(
    state: &mut BridgeState,
    method: &str,
    params: Option<&Value>,
    max_output_bytes: usize,
) {
    let record = params.and_then(Value::as_object);
    let Some(thread_id) = record
        .and_then(|record| record.get("threadId"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(session) = state
        .sessions
        .values_mut()
        .find(|session| session.thread_id == thread_id)
    else {
        return;
    };
    session.last_event_at = now_ms();

    if method == "item/agentMessage/delta" {
        if let Some(delta) = record
            .and_then(|record| record.get("delta"))
            .and_then(Value::as_str)
        {
            session.response = append_capped(&session.response, delta, max_output_bytes);
        }
        return;
    }

    if matches!(method, "item/started" | "item/completed") {
        let item = record
            .and_then(|record| record.get("item"))
            .and_then(Value::as_object);
        let item_type = item
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        push_event(
            session,
            &format!(
                "{}:{}",
                if method == "item/started" {
                    "started"
                } else {
                    "completed"
                },
                item_type
            ),
        );
        if method == "item/completed"
            && item_type == "agentMessage"
            && let Some(text) = item
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
        {
            session.response = append_capped("", text, max_output_bytes);
        }
        return;
    }

    if method == "turn/completed" {
        let turn = record
            .and_then(|record| record.get("turn"))
            .and_then(Value::as_object);
        session.active_turn_id = None;
        session.turn_status = turn
            .and_then(|turn| turn.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_owned();
        if session.status != CodexSessionStatus::Stopped {
            session.status = CodexSessionStatus::Idle;
        }
        session.error = turn.and_then(|turn| turn.get("error")).map(compact_value);
        push_event(session, &format!("turn_completed:{}", session.turn_status));
        return;
    }

    if method == "thread/closed" {
        session.active_turn_id = None;
        session.status = CodexSessionStatus::Stopped;
        session.turn_status = "closed".to_owned();
        push_event(session, "thread_closed");
        return;
    }

    if method == "error" {
        let error = record
            .and_then(|record| record.get("message"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| compact_value(&Value::Object(record.cloned().unwrap_or_default())));
        session.error = Some(error);
        push_event(session, "codex_error");
    }
}

async fn write_json(state: &mut BridgeState, value: &Value) {
    let Some(stdin) = state.stdin.as_mut() else {
        return;
    };
    let Ok(mut line) = serde_json::to_vec(value) else {
        return;
    };
    line.push(b'\n');
    let _ = stdin.write_all(&line).await;
}

fn push_event(session: &mut super::model::CodexSession, event: &str) {
    session.events.push(event.to_owned());
    if session.events.len() > MAX_EVENT_COUNT {
        let remove = session.events.len() - MAX_EVENT_COUNT;
        session.events.drain(..remove);
    }
}

fn append_capped(current: &str, next: &str, max_bytes: usize) -> String {
    let combined = format!("{current}{next}");
    if combined.len() <= max_bytes {
        return combined;
    }
    let boundary = combined
        .char_indices()
        .take_while(|(index, _)| *index <= max_bytes)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    format!("{}{}", &combined[..boundary], RESPONSE_MARKER)
}

fn compact_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[allow(dead_code)]
fn _as_record(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use tokio::sync::oneshot;

    use super::*;
    use crate::codex::model::{CodexSession, CodexSessionMode};

    #[tokio::test]
    async fn json_rpc_response_resolves_pending_request() {
        let mut state = BridgeState::new();
        let (sender, receiver) = oneshot::channel();
        state.pending.insert(7, sender);
        handle_message(
            &mut state,
            json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}}),
            1_000,
        )
        .await;
        assert_eq!(
            receiver
                .await
                .expect("response channel")
                .expect("RPC result"),
            json!({"ok": true})
        );
    }

    #[tokio::test]
    async fn notifications_update_session_response_and_turn_status() {
        let mut state = BridgeState::new();
        state.sessions.insert(
            "session".to_owned(),
            CodexSession {
                session_id: "session".to_owned(),
                thread_id: "thread".to_owned(),
                workspace_root: "/tmp/project".to_owned(),
                mode: CodexSessionMode::Review,
                status: CodexSessionStatus::Running,
                active_turn_id: Some("turn".to_owned()),
                turn_id: Some("turn".to_owned()),
                turn_status: "inProgress".to_owned(),
                response: String::new(),
                events: Vec::new(),
                error: None,
                started_at: 0,
                last_event_at: 0,
            },
        );
        handle_message(
            &mut state,
            json!({
                "method": "item/agentMessage/delta",
                "params": {"threadId": "thread", "delta": "hello"}
            }),
            1_000,
        )
        .await;
        handle_message(
            &mut state,
            json!({
                "method": "turn/completed",
                "params": {"threadId": "thread", "turn": {"status": "completed"}}
            }),
            1_000,
        )
        .await;
        let session = state.sessions.get("session").expect("session");
        assert_eq!(session.response, "hello");
        assert_eq!(session.status, CodexSessionStatus::Idle);
        assert_eq!(session.turn_status, "completed");
        assert_eq!(session.events, vec!["turn_completed:completed"]);
    }
}
