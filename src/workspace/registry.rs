use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;
use uuid::Uuid;

use crate::config::Config;

use super::paths::{PathError, assert_allowed_path, resolve_workspace_path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkspace {
    pub workspace: Workspace,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("failed to inspect workspace {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("workspace path is not a directory: {0}")]
    NotDirectory(String),
    #[error("Unknown workspace id: {0}. Call open_workspace first.")]
    UnknownWorkspace(String),
}

pub struct WorkspaceRegistry {
    config: Config,
    workspaces: HashMap<String, Workspace>,
    workspace_ids_by_root: HashMap<PathBuf, String>,
}

impl WorkspaceRegistry {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            workspaces: HashMap::new(),
            workspace_ids_by_root: HashMap::new(),
        }
    }

    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<Workspace, WorkspaceError> {
        let requested_path = path.as_ref().to_path_buf();
        let root = assert_allowed_path(path, &self.config.allowed_roots)?;
        let metadata = fs::metadata(&root).map_err(|source| WorkspaceError::Metadata {
            path: requested_path.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(WorkspaceError::NotDirectory(
                requested_path.display().to_string(),
            ));
        }

        if let Some(existing_id) = self.workspace_ids_by_root.get(&root)
            && let Some(existing) = self.workspaces.get(existing_id)
        {
            return Ok(existing.clone());
        }

        let workspace = Workspace {
            id: format!("ws_{}", Uuid::new_v4()),
            root: root.clone(),
        };
        self.workspace_ids_by_root
            .insert(root, workspace.id.clone());
        self.workspaces
            .insert(workspace.id.clone(), workspace.clone());
        Ok(workspace)
    }

    pub fn get(&self, workspace_id: &str) -> Result<Workspace, WorkspaceError> {
        self.workspaces
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| WorkspaceError::UnknownWorkspace(workspace_id.to_owned()))
    }

    pub fn resolve(
        &self,
        workspace_id: &str,
        path: impl AsRef<Path>,
    ) -> Result<ResolvedWorkspace, WorkspaceError> {
        let workspace = self.get(workspace_id)?;
        let absolute_path = resolve_workspace_path(&workspace.root, path)?;
        Ok(ResolvedWorkspace {
            workspace,
            absolute_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AccessMode, SearchProvider, ToolProfile};
    use tempfile::tempdir;

    fn test_config(root: &Path) -> Config {
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
            sqlite_tools_enabled: false,
            sqlite_allowed_dbs: Vec::new(),
            sqlite_max_rows: 100,
        }
    }

    #[test]
    fn workspace_ids_are_cached_by_canonical_root() {
        let temp = tempdir().expect("temporary directory");
        let mut registry = WorkspaceRegistry::new(test_config(temp.path()));

        let first = registry.open(temp.path()).expect("first workspace");
        let second = registry
            .open(temp.path().join("."))
            .expect("second workspace");

        assert_eq!(first.id, second.id);
        assert_eq!(first.root, second.root);
    }

    #[test]
    fn unknown_workspace_ids_are_rejected() {
        let temp = tempdir().expect("temporary directory");
        let registry = WorkspaceRegistry::new(test_config(temp.path()));

        let error = registry.get("ws_missing").expect_err("missing workspace");
        assert!(error.to_string().contains("Unknown workspace"));
    }
}
