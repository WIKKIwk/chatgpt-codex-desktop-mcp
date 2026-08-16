use serde_json::Value;

use crate::config::{Config, SearchProvider};

use super::{WebError, http::get_with_timeout, security::make_search_url};

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: Option<String>,
}

pub async fn web_search(
    config: &Config,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, WebError> {
    ensure_enabled(config)?;
    if config.search_provider != SearchProvider::Searxng {
        return Err(WebError::SearchProviderRequired);
    }
    if config.searxng_url.is_empty() {
        return Err(WebError::SearchUrlRequired);
    }
    let mut endpoint = make_search_url(&config.searxng_url)?;
    endpoint.query_pairs_mut().append_pair("q", query);
    endpoint.query_pairs_mut().append_pair("format", "json");
    let response = get_with_timeout(&endpoint, config.web_timeout_ms).await?;
    if !response.status().is_success() {
        return Err(WebError::SearchHttpStatus(response.status().as_u16()));
    }
    let data = response.json::<Value>().await?;
    let results = data
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(limit.clamp(1, 10))
        .map(search_result)
        .collect();
    Ok(results)
}

fn ensure_enabled(config: &Config) -> Result<(), WebError> {
    if config.web_tools_enabled {
        Ok(())
    } else {
        Err(WebError::Disabled)
    }
}

fn search_result(item: &Value) -> SearchResult {
    let engine = item
        .get("engines")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .or_else(|| {
            item.get("engine")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    SearchResult {
        title: item.get("title").map(string_value).unwrap_or_default(),
        url: item.get("url").map(string_value).unwrap_or_default(),
        snippet: item
            .get("content")
            .or_else(|| item.get("snippet"))
            .map(string_value)
            .unwrap_or_default(),
        engine,
    }
}

fn string_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}
