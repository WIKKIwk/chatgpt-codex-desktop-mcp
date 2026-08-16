use rmcp::{
    ErrorData,
    handler::server::tool::IntoCallToolResult,
    model::{CallToolResponse, CallToolResult, ContentBlock},
};

use crate::redaction::redact_text;

#[derive(Debug)]
pub(crate) struct ToolError(ErrorData);

impl ToolError {
    pub(crate) fn invalid_params(
        message: impl Into<std::borrow::Cow<'static, str>>,
        _data: Option<serde_json::Value>,
    ) -> Self {
        Self(ErrorData::invalid_params(message, None))
    }

    pub(crate) fn internal_error(
        message: impl Into<std::borrow::Cow<'static, str>>,
        _data: Option<serde_json::Value>,
    ) -> Self {
        Self(ErrorData::internal_error(message, None))
    }
}

impl From<ErrorData> for ToolError {
    fn from(error: ErrorData) -> Self {
        Self(error)
    }
}

impl IntoCallToolResult for ToolError {
    fn into_call_tool_result(self) -> Result<CallToolResponse, ErrorData> {
        let message = redact_text(self.0.message.as_ref());
        Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into())
    }
}
