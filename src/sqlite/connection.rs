use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags};

use crate::config::Config;

use super::model::{SqliteError, SqliteStatus};

pub fn sqlite_status(config: &Config) -> SqliteStatus {
    SqliteStatus {
        enabled: config.sqlite_tools_enabled,
        allowed_dbs: config
            .sqlite_allowed_dbs
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        max_rows: config.sqlite_max_rows,
        node_sqlite: true,
    }
}

pub(crate) fn resolve_allowed_db(
    config: &Config,
    db_path: Option<&str>,
) -> Result<PathBuf, SqliteError> {
    if !config.sqlite_tools_enabled {
        return Err(SqliteError::Disabled);
    }
    if config.sqlite_allowed_dbs.is_empty() {
        return Err(SqliteError::NoAllowedDatabases);
    }

    let requested = match db_path {
        Some(path) => absolute_path(path)?,
        None => {
            if config.sqlite_allowed_dbs.len() != 1 {
                return Err(SqliteError::DatabasePathRequired);
            }
            absolute_path(&config.sqlite_allowed_dbs[0].to_string_lossy())?
        }
    };
    let requested_for_compare = comparable_path(&requested);
    let allowed = config
        .sqlite_allowed_dbs
        .iter()
        .map(|path| comparable_path(path))
        .any(|path| path == requested_for_compare);
    if !allowed {
        return Err(SqliteError::DatabaseNotAllowed(requested));
    }
    if !requested.exists() {
        return Err(SqliteError::DatabaseMissing(requested));
    }
    if !requested.is_file() {
        return Err(SqliteError::DatabaseNotFile(requested));
    }
    Ok(requested)
}

pub(crate) fn open_database(path: &Path, read_only: bool) -> Result<Connection, SqliteError> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

fn absolute_path(path: &str) -> Result<PathBuf, SqliteError> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn comparable_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        return PathBuf::from(canonical.to_string_lossy().to_ascii_lowercase());
    }

    #[cfg(not(windows))]
    {
        canonical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AccessMode, SearchProvider, ToolProfile};
    use tempfile::tempdir;

    fn config(root: &Path, enabled: bool, allowed: Vec<PathBuf>) -> Config {
        Config {
            host: "127.0.0.1".to_owned(),
            port: 3333,
            allowed_roots: vec![root.to_path_buf()],
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
            web_tools_enabled: false,
            search_provider: SearchProvider::None,
            searxng_url: String::new(),
            web_max_bytes: 200_000,
            web_timeout_ms: 15_000,
            sqlite_tools_enabled: enabled,
            sqlite_allowed_dbs: allowed,
            sqlite_max_rows: 100,
        }
    }

    #[test]
    fn allowed_database_resolution_requires_exact_configured_file() {
        let temp = tempdir().expect("temporary directory");
        let database = temp.path().join("data.sqlite");
        fs::write(&database, b"not yet a database").expect("database placeholder");
        let config = config(temp.path(), true, vec![database.clone()]);

        assert_eq!(
            resolve_allowed_db(&config, None).expect("allowed path"),
            database
        );
        assert!(matches!(
            resolve_allowed_db(
                &config,
                Some(&temp.path().join("other.sqlite").to_string_lossy())
            ),
            Err(SqliteError::DatabaseNotAllowed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn case_sensitive_filesystems_do_not_fold_database_paths() {
        let temp = tempdir().expect("temporary directory");
        let lower = temp.path().join("data.sqlite");
        let upper = temp.path().join("DATA.sqlite");
        fs::write(&lower, b"lower").expect("lower database placeholder");
        if upper.exists() {
            return;
        }
        fs::write(&upper, b"upper").expect("upper database placeholder");
        let config = config(temp.path(), true, vec![lower]);

        assert!(matches!(
            resolve_allowed_db(&config, Some(&upper.to_string_lossy())),
            Err(SqliteError::DatabaseNotAllowed(_))
        ));
    }
}
