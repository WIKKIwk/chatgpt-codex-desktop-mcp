use std::path::PathBuf;

use super::*;
use crate::config::{AccessMode, ToolProfile};

fn config(enabled: bool) -> Config {
    Config {
        host: "127.0.0.1".to_owned(),
        port: 3333,
        allowed_roots: vec![PathBuf::from(".")],
        deny_globs: Vec::new(),
        access_mode: AccessMode::Review,
        tool_profile: ToolProfile::Legacy,
        stateless_mcp_fallback: false,
        codex_bridge_enabled: false,
        codex_command: "codex".to_owned(),
        codex_max_sessions: 4,
        codex_request_timeout_ms: 120_000,
        max_read_bytes: 200_000,
        max_output_bytes: 200_000,
        web_tools_enabled: enabled,
        search_provider: crate::config::SearchProvider::Searxng,
        searxng_url: "http://127.0.0.1:8888".to_owned(),
        web_max_bytes: 100,
        web_timeout_ms: 1_000,
        sqlite_tools_enabled: false,
        sqlite_allowed_dbs: Vec::new(),
        sqlite_max_rows: 100,
    }
}

#[test]
fn status_reports_fetch_policy_and_search_configuration() {
    let status = web_status(&config(true));
    assert!(status.enabled);
    assert_eq!(status.search_provider, "searxng");
    assert!(status.searxng_configured);
    assert_eq!(status.fetch_policy.methods, ["GET"]);
    assert_eq!(status.fetch_policy.protocols, ["http", "https"]);
}

#[tokio::test]
async fn disabled_web_tools_fail_before_network_access() {
    let config = config(false);
    assert!(matches!(
        web_search(&config, "test", 5).await,
        Err(WebError::Disabled)
    ));
    assert!(matches!(
        web_fetch(&config, "http://127.0.0.1").await,
        Err(WebError::Disabled)
    ));
}

#[tokio::test]
async fn public_fetch_rejects_credentials_localhost_and_private_addresses() {
    let config = config(true);
    assert!(matches!(
        web_fetch(&config, "http://user:pass@example.com").await,
        Err(WebError::FetchUrlCredentials)
    ));
    assert!(matches!(
        web_fetch(&config, "http://localhost:8080").await,
        Err(WebError::LocalhostBlocked)
    ));
    assert!(matches!(
        web_fetch(&config, "http://127.0.0.1:8080").await,
        Err(WebError::PrivateAddressBlocked)
    ));
}

#[test]
fn searxng_url_is_normalized_without_credentials_or_old_query() {
    let url = security::make_search_url("http://127.0.0.1:8888/base/?old=1#fragment")
        .expect("search URL");
    assert_eq!(url.as_str(), "http://127.0.0.1:8888/base/search");
    assert!(matches!(
        security::make_search_url("http://user@example.com"),
        Err(WebError::SearchUrlCredentials)
    ));
}
