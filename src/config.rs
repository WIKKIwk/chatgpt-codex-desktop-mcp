use std::{
    collections::HashMap,
    env as process_env, fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Review,
    Coding,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProfile {
    Legacy,
    Coding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchProvider {
    None,
    Searxng,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub host: String,
    pub port: u32,
    pub allowed_roots: Vec<PathBuf>,
    pub deny_globs: Vec<String>,
    pub access_mode: AccessMode,
    pub tool_profile: ToolProfile,
    pub stateless_mcp_fallback: bool,
    pub codex_bridge_enabled: bool,
    pub codex_command: String,
    pub codex_max_sessions: u32,
    pub codex_request_timeout_ms: u32,
    pub max_read_bytes: u32,
    pub max_output_bytes: u32,
    pub web_tools_enabled: bool,
    pub search_provider: SearchProvider,
    pub searxng_url: String,
    pub web_max_bytes: u32,
    pub web_timeout_ms: u32,
    pub sqlite_tools_enabled: bool,
    pub sqlite_allowed_dbs: Vec<PathBuf>,
    pub sqlite_max_rows: u32,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse config file {path}: {message}")]
    ParseFile { path: PathBuf, message: String },
    #[error("invalid {field}: {value}")]
    InvalidValue { field: String, value: String },
    #[error("could not determine current directory: {0}")]
    CurrentDirectory(#[source] io::Error),
}

pub const DEFAULT_DENY_GLOBS: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/id_rsa",
    "**/id_ed25519",
    "**/*token*",
    "**/*secret*",
    "**/key.txt",
    "**/*.key",
    "**/*.pem",
    "**/AppData/**",
    "**/.git/**",
    "**/.git/config",
];

pub fn load_config() -> Result<Config, ConfigError> {
    let env = process_env::vars().collect::<HashMap<_, _>>();
    let cwd = process_env::current_dir().map_err(ConfigError::CurrentDirectory)?;
    load_config_from(&env, &cwd)
}

pub fn load_config_from(env: &HashMap<String, String>, cwd: &Path) -> Result<Config, ConfigError> {
    let config_path = configured_config_path(env, cwd);
    let file_config = read_file_config(&config_path)?;
    let mcp = object_section(&file_config, "mcp");
    let web = object_section(&file_config, "web");
    let sqlite = object_section(&file_config, "sqlite");

    let access_mode = parse_access_mode(
        configured(env, "CTM_ACCESS_MODE", mcp, &["accessMode"]),
        "mcp.accessMode",
    )?;
    let tool_profile = parse_tool_profile(
        configured(env, "CTM_TOOL_PROFILE", mcp, &["toolProfile"]),
        "mcp.toolProfile",
    )?;
    let default_bridge = tool_profile == ToolProfile::Coding;

    Ok(Config {
        host: parse_string(configured(env, "HOST", mcp, &["host"]), "127.0.0.1"),
        port: parse_integer(configured(env, "PORT", mcp, &["port"]), 3333, "PORT")?,
        allowed_roots: parse_path_list(
            configured(env, "CTM_ALLOWED_ROOTS", mcp, &["allowedRoots"]),
            &[cwd.to_path_buf()],
            cwd,
        ),
        deny_globs: parse_list(
            configured(env, "CTM_DENY_GLOBS", mcp, &["denyGlobs"]),
            &DEFAULT_DENY_GLOBS
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        ),
        access_mode,
        tool_profile,
        stateless_mcp_fallback: parse_boolean(
            configured(
                env,
                "CTM_STATELESS_MCP_FALLBACK",
                mcp,
                &["statelessFallback"],
            ),
            false,
        ),
        codex_bridge_enabled: parse_boolean(
            configured(
                env,
                "CTM_CODEX_BRIDGE",
                mcp,
                &["codexBridgeEnabled", "codexBridge"],
            ),
            default_bridge,
        ),
        codex_command: parse_string(
            configured(env, "CTM_CODEX_COMMAND", mcp, &["codexCommand"]),
            default_codex_command(),
        ),
        codex_max_sessions: parse_integer(
            configured(env, "CTM_CODEX_MAX_SESSIONS", mcp, &["codexMaxSessions"]),
            4,
            "CTM_CODEX_MAX_SESSIONS",
        )?,
        codex_request_timeout_ms: parse_integer(
            configured(
                env,
                "CTM_CODEX_REQUEST_TIMEOUT_MS",
                mcp,
                &["codexRequestTimeoutMs"],
            ),
            120_000,
            "CTM_CODEX_REQUEST_TIMEOUT_MS",
        )?,
        max_read_bytes: parse_integer(
            configured(env, "CTM_MAX_READ_BYTES", mcp, &["maxReadBytes"]),
            200_000,
            "CTM_MAX_READ_BYTES",
        )?,
        max_output_bytes: parse_integer(
            configured(env, "CTM_MAX_OUTPUT_BYTES", mcp, &["maxOutputBytes"]),
            200_000,
            "CTM_MAX_OUTPUT_BYTES",
        )?,
        web_tools_enabled: parse_boolean(
            configured(env, "CTM_WEB_TOOLS", web, &["enabled"]),
            false,
        ),
        search_provider: parse_search_provider(
            configured(env, "CTM_SEARCH_PROVIDER", web, &["searchProvider"]),
            "web.searchProvider",
        )?,
        searxng_url: parse_string(configured(env, "CTM_SEARXNG_URL", web, &["searxngUrl"]), ""),
        web_max_bytes: parse_integer(
            configured(env, "CTM_WEB_MAX_BYTES", web, &["maxBytes"]),
            200_000,
            "CTM_WEB_MAX_BYTES",
        )?,
        web_timeout_ms: parse_integer(
            configured(env, "CTM_WEB_TIMEOUT_MS", web, &["timeoutMs"]),
            15_000,
            "CTM_WEB_TIMEOUT_MS",
        )?,
        sqlite_tools_enabled: parse_boolean(
            configured(env, "CTM_SQLITE_TOOLS", sqlite, &["enabled"]),
            false,
        ),
        sqlite_allowed_dbs: parse_path_list(
            configured(env, "CTM_SQLITE_ALLOWED_DBS", sqlite, &["allowedDbs"]),
            &[],
            cwd,
        ),
        sqlite_max_rows: parse_integer(
            configured(env, "CTM_SQLITE_MAX_ROWS", sqlite, &["maxRows"]),
            100,
            "CTM_SQLITE_MAX_ROWS",
        )?,
    })
}

fn configured_config_path(env: &HashMap<String, String>, cwd: &Path) -> PathBuf {
    let raw = env
        .get("CTM_CONFIG_PATH")
        .filter(|value| !value.is_empty())
        .or_else(|| env.get("CONFIG_PATH").filter(|value| !value.is_empty()))
        .map(|value| PathBuf::from(expand_home(value)))
        .unwrap_or_else(|| cwd.join("config.json"));

    if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    }
}

fn read_file_config(path: &Path) -> Result<Map<String, Value>, ConfigError> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let raw = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }

    let parsed = serde_json::from_str::<Value>(&raw).map_err(|error| ConfigError::ParseFile {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| ConfigError::ParseFile {
            path: path.to_path_buf(),
            message: "top-level JSON value must be an object".to_owned(),
        })
}

fn object_section<'a>(root: &'a Map<String, Value>, name: &str) -> Option<&'a Map<String, Value>> {
    root.get(name).and_then(Value::as_object)
}

fn configured(
    env: &HashMap<String, String>,
    env_name: &str,
    section: Option<&Map<String, Value>>,
    json_names: &[&str],
) -> Option<Value> {
    if let Some(value) = env.get(env_name).filter(|value| !value.is_empty()) {
        return Some(Value::String(value.clone()));
    }

    json_names.iter().find_map(|name| {
        section
            .and_then(|values| values.get(*name))
            .filter(|value| !value.is_null())
            .filter(|value| !matches!(value, Value::String(value) if value.is_empty()))
            .cloned()
    })
}

fn parse_access_mode(value: Option<Value>, field: &str) -> Result<AccessMode, ConfigError> {
    match parse_string(value, "review").as_str() {
        "review" => Ok(AccessMode::Review),
        "coding" => Ok(AccessMode::Coding),
        "full" => Ok(AccessMode::Full),
        value => Err(invalid_value(field, value)),
    }
}

fn parse_tool_profile(value: Option<Value>, field: &str) -> Result<ToolProfile, ConfigError> {
    match parse_string(value, "legacy").as_str() {
        "legacy" => Ok(ToolProfile::Legacy),
        "coding" => Ok(ToolProfile::Coding),
        value => Err(invalid_value(field, value)),
    }
}

fn parse_search_provider(value: Option<Value>, field: &str) -> Result<SearchProvider, ConfigError> {
    match parse_string(value, "none").as_str() {
        "none" => Ok(SearchProvider::None),
        "searxng" => Ok(SearchProvider::Searxng),
        value => Err(invalid_value(field, value)),
    }
}

fn parse_boolean(value: Option<Value>, fallback: bool) -> bool {
    match value {
        None | Some(Value::Null) => fallback,
        Some(Value::Bool(value)) => value,
        Some(Value::Number(value)) => value.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Some(_) => false,
    }
}

fn parse_integer(value: Option<Value>, fallback: u32, field: &str) -> Result<u32, ConfigError> {
    let Some(value) = value else {
        return Ok(fallback);
    };

    let text = match value {
        Value::Number(value) => value.to_string(),
        Value::String(value) => value,
        other => value_to_string(&other),
    };
    let parsed = text
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|value| u32::try_from(value).ok().filter(|value| *value >= 1));
    parsed.ok_or_else(|| invalid_value(field, &text))
}

fn parse_string(value: Option<Value>, fallback: impl Into<String>) -> String {
    let fallback = fallback.into();
    value.map_or(fallback, |value| value_to_string(&value))
}

fn parse_list(value: Option<Value>, fallback: &[String]) -> Vec<String> {
    let entries = match value {
        Some(Value::Array(values)) => values
            .iter()
            .map(value_to_string)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
        Some(Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    if entries.is_empty() {
        fallback.to_vec()
    } else {
        entries
    }
}

fn parse_path_list(value: Option<Value>, fallback: &[PathBuf], cwd: &Path) -> Vec<PathBuf> {
    let entries = parse_list(value, &[])
        .into_iter()
        .map(|value| {
            let expanded = expand_home(&value);
            let path = PathBuf::from(expanded);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .collect::<Vec<_>>();

    if entries.is_empty() {
        fallback.to_vec()
    } else {
        entries
    }
}

fn expand_home(value: &str) -> String {
    let home = process_env::var_os("HOME")
        .or_else(|| process_env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    match (home, value) {
        (Some(home), "~") => home.to_string_lossy().into_owned(),
        (Some(home), value) if value.starts_with("~/") || value.starts_with("~\\") => {
            home.join(&value[2..]).to_string_lossy().into_owned()
        }
        (_, value) => value.to_owned(),
    }
}

fn default_codex_command() -> String {
    #[cfg(target_os = "macos")]
    {
        let bundled = Path::new("/Applications/ChatGPT.app/Contents/Resources/codex");
        if bundled.exists() {
            return bundled.to_string_lossy().into_owned();
        }
    }

    "codex".to_owned()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn invalid_value(field: &str, value: &str) -> ConfigError {
    ConfigError::InvalidValue {
        field: field.to_owned(),
        value: value.to_owned(),
    }
}
