use schemars::JsonSchema;
use serde::Serialize;

use super::core_results::{StructuredOutput, TextOutput};
use crate::{
    process::cap_text,
    redaction::redact_text,
    web::{FetchResult, SearchResult, WebStatus},
};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct WebSearchItemOutput {
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) engine: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct WebSearchOutput {
    pub(crate) result: String,
    pub(crate) results: Vec<WebSearchItemOutput>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct WebFetchOutput {
    pub(crate) result: String,
    #[serde(rename = "finalUrl")]
    pub(crate) final_url: String,
    pub(crate) status: u16,
    #[serde(rename = "contentType")]
    pub(crate) content_type: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

pub(crate) fn web_status_output(status: &WebStatus) -> StructuredOutput<TextOutput> {
    let result = serde_json::to_string_pretty(status).expect("web status is serializable");
    super::core_results::text_output(result)
}

pub(crate) fn web_search_output(results: &[SearchResult]) -> StructuredOutput<WebSearchOutput> {
    let results = results
        .iter()
        .map(|result| WebSearchItemOutput {
            title: redact_text(&result.title),
            url: redact_text(&result.url),
            snippet: redact_text(&result.snippet),
            engine: result.engine.as_ref().map(|value| redact_text(value)),
        })
        .collect::<Vec<_>>();
    let result_text = if results.is_empty() {
        "(no results)".to_owned()
    } else {
        results
            .iter()
            .map(|result| format!("- {}\n  {}\n  {}", result.title, result.url, result.snippet))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let result = redact_text(&result_text);
    let value = WebSearchOutput {
        result: result.clone(),
        results,
    };
    StructuredOutput::new(result, value)
}

pub(crate) fn web_fetch_output(
    result: &FetchResult,
    max_read_bytes: usize,
) -> StructuredOutput<WebFetchOutput> {
    let mut lines = vec![
        format!("final_url: {}", result.final_url),
        format!("status: {}", result.status),
        format!("content_type: {}", result.content_type),
    ];
    if result.truncated {
        lines.push("[output truncated]".to_owned());
    }
    lines.push(cap_text(result.text.clone(), max_read_bytes));
    let result_text = redact_text(&lines.join("\n"));
    let value = WebFetchOutput {
        result: result_text.clone(),
        final_url: redact_text(&result.final_url),
        status: result.status,
        content_type: redact_text(&result.content_type),
        title: redact_text(&result.title),
        text: redact_text(&result.text),
        truncated: result.truncated,
    };
    StructuredOutput::new(result_text, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{handler::server::tool::IntoCallToolResult, model::CallToolResponse};

    #[test]
    fn search_output_preserves_text_and_structured_results() {
        let response = web_search_output(&[SearchResult {
            title: "Rust docs".to_owned(),
            url: "https://example.com".to_owned(),
            snippet: "safe result".to_owned(),
            engine: Some("demo".to_owned()),
        }])
        .into_call_tool_result()
        .expect("search result");
        let CallToolResponse::Complete(result) = response else {
            panic!("expected complete tool result");
        };
        assert!(result.content[0].as_text().is_some());
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(structured["results"][0]["title"], "Rust docs");
        assert_eq!(structured["results"][0]["engine"], "demo");
    }

    #[test]
    fn fetch_output_keeps_metadata_and_caps_only_human_text() {
        let response = web_fetch_output(
            &FetchResult {
                final_url: "https://example.com/final".to_owned(),
                status: 200,
                content_type: "text/plain".to_owned(),
                title: "Example".to_owned(),
                text: "0123456789".to_owned(),
                truncated: true,
            },
            5,
        )
        .into_call_tool_result()
        .expect("fetch result");
        let CallToolResponse::Complete(result) = response else {
            panic!("expected complete tool result");
        };
        let structured = result.structured_content.as_ref().expect("structured");
        assert_eq!(structured["finalUrl"], "https://example.com/final");
        assert_eq!(structured["text"], "0123456789");
        assert_eq!(structured["truncated"], true);
        assert!(
            result.content[0]
                .as_text()
                .expect("human text")
                .text
                .contains("[output truncated]")
        );
    }
}
