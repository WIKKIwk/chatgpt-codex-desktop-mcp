use std::collections::HashMap;
use std::fs;

use tempfile::tempdir;

use crate::config::{AccessMode, SearchProvider, ToolProfile, load_config_from};

fn env(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn file_config_and_environment_precedence_match_reference() {
    let temp = tempdir().expect("temporary directory");
    let config_path = temp.path().join("config.json");
    fs::write(
        &config_path,
        r#"{
            "mcp": {
                "port": 3334,
                "allowedRoots": ["./workspace"],
                "accessMode": "full",
                "statelessFallback": true
            },
            "web": {
                "enabled": true,
                "searchProvider": "searxng",
                "searxngUrl": "https://example.com"
            }
        }"#,
    )
    .expect("config file");

    let variables = env(&[
        (
            "CTM_CONFIG_PATH",
            config_path.to_str().expect("config path"),
        ),
        ("PORT", "4444"),
    ]);
    let config = load_config_from(&variables, temp.path()).expect("config");
    assert_eq!(config.port, 4444);
    assert_eq!(config.access_mode, AccessMode::Full);
    assert!(config.stateless_mcp_fallback);
    assert_eq!(config.allowed_roots, vec![temp.path().join("workspace")]);
    assert!(config.web_tools_enabled);
    assert_eq!(config.search_provider, SearchProvider::Searxng);
}

#[test]
fn coding_profile_enables_bridge_by_default_and_allows_override() {
    let temp = tempdir().expect("temporary directory");
    let variables = env(&[
        ("CTM_TOOL_PROFILE", "coding"),
        ("CTM_ACCESS_MODE", "coding"),
    ]);
    let config = load_config_from(&variables, temp.path()).expect("config");
    assert_eq!(config.tool_profile, ToolProfile::Coding);
    assert_eq!(config.access_mode, AccessMode::Coding);
    assert!(config.codex_bridge_enabled);

    let mut disabled = variables;
    disabled.insert("CTM_CODEX_BRIDGE".to_owned(), "false".to_owned());
    let config = load_config_from(&disabled, temp.path()).expect("config");
    assert!(!config.codex_bridge_enabled);
}

#[test]
fn empty_file_values_fall_back_like_environment_values() {
    let temp = tempdir().expect("temporary directory");
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, r#"{"mcp":{"codexCommand":"","host":""}}"#).expect("config file");
    let variables = env(&[(
        "CTM_CONFIG_PATH",
        config_path.to_str().expect("config path"),
    )]);

    let config = load_config_from(&variables, temp.path()).expect("config");
    assert!(!config.codex_command.is_empty());
    assert_eq!(config.host, "127.0.0.1");
}

#[test]
fn invalid_values_fail_with_their_field_name() {
    let temp = tempdir().expect("temporary directory");
    let variables = env(&[("PORT", "not-a-port")]);
    let error = load_config_from(&variables, temp.path()).expect_err("invalid port");
    assert!(error.to_string().contains("PORT"));
}
