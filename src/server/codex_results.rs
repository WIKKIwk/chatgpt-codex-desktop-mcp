use schemars::JsonSchema;
use serde::Serialize;

use super::core_results::StructuredOutput;
use crate::{codex::CodexSessionSnapshot, redaction::redact_text};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct CodexSessionOutput {
    pub(crate) result: String,
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
    #[serde(rename = "threadId")]
    pub(crate) thread_id: String,
    #[serde(rename = "workspaceRoot")]
    pub(crate) workspace_root: String,
    pub(crate) mode: String,
    pub(crate) status: String,
    pub(crate) running: bool,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: Option<String>,
    #[serde(rename = "turnStatus")]
    pub(crate) turn_status: String,
    pub(crate) response: String,
    pub(crate) events: Vec<String>,
    pub(crate) error: Option<String>,
    #[serde(rename = "startedAt")]
    pub(crate) started_at: u64,
    #[serde(rename = "lastEventAt")]
    pub(crate) last_event_at: u64,
}

pub(crate) fn codex_session_output(
    snapshot: &CodexSessionSnapshot,
) -> StructuredOutput<CodexSessionOutput> {
    let result = format_snapshot(snapshot);
    let value = CodexSessionOutput {
        result: result.clone(),
        session_id: redact_text(&snapshot.session_id),
        thread_id: redact_text(&snapshot.thread_id),
        workspace_root: redact_text(&snapshot.workspace_root),
        mode: redact_text(&snapshot.mode),
        status: redact_text(&snapshot.status),
        running: snapshot.running,
        turn_id: snapshot.turn_id.as_ref().map(|value| redact_text(value)),
        turn_status: redact_text(&snapshot.turn_status),
        response: redact_text(&snapshot.response),
        events: snapshot
            .events
            .iter()
            .map(|value| redact_text(value))
            .collect(),
        error: snapshot.error.as_ref().map(|value| redact_text(value)),
        started_at: snapshot.started_at,
        last_event_at: snapshot.last_event_at,
    };
    StructuredOutput::new(result, value)
}

fn format_snapshot(snapshot: &CodexSessionSnapshot) -> String {
    let mut lines = vec![
        format!("session_id: {}", snapshot.session_id),
        format!("thread_id: {}", snapshot.thread_id),
        format!("workspace: {}", snapshot.workspace_root),
        format!("mode: {}", snapshot.mode),
        format!("status: {}", snapshot.status),
        format!("running: {}", snapshot.running),
        format!("turn_status: {}", snapshot.turn_status),
    ];
    lines.push(if snapshot.response.is_empty() {
        "response: (not available yet; call codex_read_response)".to_owned()
    } else {
        format!("response:\n{}", snapshot.response)
    });
    if !snapshot.events.is_empty() {
        lines.push(format!("events:\n{}", snapshot.events.join("\n")));
    }
    if let Some(error) = &snapshot.error {
        lines.push(format!("error: {error}"));
    }
    let text = lines.join("\n");
    redact_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{handler::server::tool::IntoCallToolResult, model::CallToolResponse};

    #[test]
    fn session_output_matches_reference_fields_and_text() {
        let response = codex_session_output(&CodexSessionSnapshot {
            session_id: "codex_session".to_owned(),
            thread_id: "thread_1".to_owned(),
            workspace_root: "/tmp/project".to_owned(),
            mode: "review".to_owned(),
            status: "idle".to_owned(),
            running: false,
            turn_id: None,
            turn_status: "completed".to_owned(),
            response: "done".to_owned(),
            events: vec!["turn_completed".to_owned()],
            error: None,
            started_at: 10,
            last_event_at: 20,
        })
        .into_call_tool_result()
        .expect("Codex result");
        let CallToolResponse::Complete(result) = response else {
            panic!("expected complete tool result");
        };
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(structured["sessionId"], "codex_session");
        assert_eq!(structured["turnStatus"], "completed");
        assert_eq!(structured["response"], "done");
        assert!(
            result.content[0]
                .as_text()
                .expect("human text")
                .text
                .contains("response:\ndone")
        );
    }
}
