use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::core_results::{Json, TextOutput};
use super::handler::ForgeHandler;
use super::tool_error::ToolError;
use super::web_results::{
    WebFetchOutput, WebSearchOutput, web_fetch_output, web_search_output, web_status_output,
};
use crate::web::{WebError, web_fetch, web_search, web_status};

#[derive(Debug, Deserialize, JsonSchema)]
struct WebSearchRequest {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebFetchRequest {
    url: String,
}

#[tool_router(router = web_tool_router, vis = "pub(crate)")]
impl ForgeHandler {
    #[tool(
        name = "web_status",
        description = "Show web tools configuration and public-fetch policy."
    )]
    async fn web_status(&self) -> Json<TextOutput> {
        web_status_output(&web_status(&self.config))
    }

    #[tool(
        name = "web_search",
        description = "Search the web through the configured SearXNG instance."
    )]
    async fn web_search(
        &self,
        Parameters(request): Parameters<WebSearchRequest>,
    ) -> Result<Json<WebSearchOutput>, ToolError> {
        let limit = request.limit.unwrap_or(5);
        if !(1..=10).contains(&limit) {
            return Err(invalid_params("limit must be between 1 and 10"));
        }
        let results = web_search(&self.config, &request.query, limit)
            .await
            .map_err(web_error)?;
        Ok(web_search_output(&results))
    }

    #[tool(
        name = "web_fetch",
        description = "Fetch a public HTTP(S) page without cookies or authorization headers."
    )]
    async fn web_fetch(
        &self,
        Parameters(request): Parameters<WebFetchRequest>,
    ) -> Result<Json<WebFetchOutput>, ToolError> {
        let result = web_fetch(&self.config, &request.url)
            .await
            .map_err(web_error)?;
        Ok(web_fetch_output(
            &result,
            self.config.max_read_bytes as usize,
        ))
    }
}

fn web_error(error: WebError) -> ToolError {
    let message = error.to_string();
    match error {
        WebError::Http(_) | WebError::Body(_) => ToolError::internal_error(message, None),
        _ => ToolError::invalid_params(message, None),
    }
}

fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::invalid_params(message.into(), None)
}
