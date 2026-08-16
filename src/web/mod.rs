mod fetch;
mod http;
mod search;
mod security;

use serde::Serialize;
use thiserror::Error;

use crate::config::{Config, SearchProvider};

pub use fetch::{FetchResult, web_fetch};
pub use search::{SearchResult, web_search};

#[derive(Debug, Clone, Serialize)]
pub struct WebStatus {
    pub enabled: bool,
    #[serde(rename = "searchProvider")]
    pub search_provider: &'static str,
    #[serde(rename = "searxngConfigured")]
    pub searxng_configured: bool,
    #[serde(rename = "webMaxBytes")]
    pub web_max_bytes: u32,
    #[serde(rename = "webTimeoutMs")]
    pub web_timeout_ms: u32,
    #[serde(rename = "fetchPolicy")]
    pub fetch_policy: FetchPolicy,
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchPolicy {
    pub methods: [&'static str; 1],
    pub protocols: [&'static str; 2],
    pub credentials: &'static str,
    #[serde(rename = "privateNetworkTargets")]
    pub private_network_targets: &'static str,
    pub redirects: &'static str,
}

#[derive(Debug, Error)]
pub enum WebError {
    #[error("Web tools are disabled. Set CTM_WEB_TOOLS=1 to enable them.")]
    Disabled,
    #[error("web_search requires CTM_SEARCH_PROVIDER=searxng.")]
    SearchProviderRequired,
    #[error("web_search requires CTM_SEARXNG_URL.")]
    SearchUrlRequired,
    #[error("CTM_SEARXNG_URL must use http or https.")]
    InvalidSearchUrl,
    #[error("CTM_SEARXNG_URL must not include credentials.")]
    SearchUrlCredentials,
    #[error("web_fetch only supports http and https URLs.")]
    InvalidFetchProtocol,
    #[error("web_fetch URLs must not include credentials.")]
    FetchUrlCredentials,
    #[error("web_fetch cannot access localhost.")]
    LocalhostBlocked,
    #[error("web_fetch cannot access private or local network addresses.")]
    PrivateAddressBlocked,
    #[error("Unable to resolve host: {0}")]
    HostResolution(String),
    #[error("Redirect response missing Location header: HTTP {0}")]
    RedirectLocationMissing(u16),
    #[error("Too many redirects.")]
    TooManyRedirects,
    #[error("SearXNG search failed: HTTP {0}")]
    SearchHttpStatus(u16),
    #[error("web request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid web URL: {0}")]
    InvalidUrl(String),
    #[error("web URL has no host")]
    MissingHost,
    #[error("web response body error: {0}")]
    Body(String),
}

pub fn web_status(config: &Config) -> WebStatus {
    WebStatus {
        enabled: config.web_tools_enabled,
        search_provider: match config.search_provider {
            SearchProvider::None => "none",
            SearchProvider::Searxng => "searxng",
        },
        searxng_configured: !config.searxng_url.is_empty(),
        web_max_bytes: config.web_max_bytes,
        web_timeout_ms: config.web_timeout_ms,
        fetch_policy: FetchPolicy {
            methods: ["GET"],
            protocols: ["http", "https"],
            credentials: "not sent",
            private_network_targets: "blocked for web_fetch",
            redirects: "checked before each hop",
        },
    }
}

#[cfg(test)]
mod tests;
