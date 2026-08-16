use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("filesystem error while resolving path: {0}")]
    Io(#[from] io::Error),
    #[error("path is outside allowed roots: {0}")]
    OutsideAllowedRoots(String),
    #[error("path is outside workspace: {0}")]
    OutsideWorkspace(String),
    #[error("path resolves outside workspace through a symlink: {0}")]
    SymlinkEscape(String),
    #[error("path is blocked by deny glob ({pattern}): {path}")]
    Denied { pattern: String, path: String },
    #[error("invalid deny glob '{pattern}': {source}")]
    InvalidDenyGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("could not build deny glob set: {0}")]
    BuildDenyGlobs(#[from] globset::Error),
}

#[derive(Debug, Clone)]
pub struct DenyRules {
    patterns: Vec<String>,
    set: GlobSet,
}

impl DenyRules {
    pub fn new(patterns: &[String]) -> Result<Self, PathError> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .literal_separator(true)
                .build()
                .map_err(|source| PathError::InvalidDenyGlob {
                    pattern: pattern.clone(),
                    source,
                })?;
            builder.add(glob);
        }

        Ok(Self {
            patterns: patterns.to_vec(),
            set: builder.build()?,
        })
    }

    pub fn check(&self, path: &Path, workspace_root: &Path) -> Result<(), PathError> {
        let relative = relative_display_path(workspace_root, path);
        let candidates = [relative.clone(), format!("{relative}/")];

        for candidate in candidates {
            if let Some(index) = self.set.matches(&candidate).first() {
                let pattern = self
                    .patterns
                    .get(*index)
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned());
                return Err(PathError::Denied {
                    pattern,
                    path: relative,
                });
            }
        }

        Ok(())
    }

    pub(crate) fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

pub fn assert_allowed_path(
    path: impl AsRef<Path>,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, PathError> {
    let resolved = absolute_path(path.as_ref())?;
    let canonical_path = canonicalize_existing_prefix(&resolved)?;

    for root in allowed_roots {
        let canonical_root = canonicalize_existing_prefix(root)?;
        if is_inside_absolute(&canonical_path, &canonical_root) {
            return if resolved.exists() {
                Ok(fs::canonicalize(resolved)?)
            } else {
                Ok(resolved)
            };
        }
    }

    Err(PathError::OutsideAllowedRoots(
        path.as_ref().display().to_string(),
    ))
}

pub fn resolve_workspace_path(
    root: impl AsRef<Path>,
    input_path: impl AsRef<Path>,
) -> Result<PathBuf, PathError> {
    let root = absolute_path(root.as_ref())?;
    let input = input_path.as_ref();
    let resolved = if input.is_absolute() {
        absolute_path(input)?
    } else {
        absolute_path(&root.join(input))?
    };

    if !is_inside_absolute(&resolved, &root) {
        return Err(PathError::OutsideWorkspace(input.display().to_string()));
    }

    let canonical_root = canonicalize_existing_prefix(&root)?;
    let canonical_path = canonicalize_existing_prefix(&resolved)?;
    if !is_inside_absolute(&canonical_path, &canonical_root) {
        return Err(PathError::SymlinkEscape(input.display().to_string()));
    }

    Ok(resolved)
}

pub fn is_inside_root(path: impl AsRef<Path>, root: impl AsRef<Path>) -> bool {
    match (absolute_path(path.as_ref()), absolute_path(root.as_ref())) {
        (Ok(path), Ok(root)) => is_inside_absolute(&path, &root),
        _ => false,
    }
}

pub fn relative_display_path(root: &Path, path: &Path) -> String {
    let root = absolute_path(root).unwrap_or_else(|_| root.to_path_buf());
    let path = absolute_path(path).unwrap_or_else(|_| path.to_path_buf());

    match path.strip_prefix(&root) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => normalize_separators(&relative.to_string_lossy()),
        Err(_) => normalize_separators(&path.to_string_lossy()),
    }
}

pub fn assert_not_denied(
    path: impl AsRef<Path>,
    workspace_root: impl AsRef<Path>,
    deny_globs: &[String],
) -> Result<(), PathError> {
    DenyRules::new(deny_globs)?.check(path.as_ref(), workspace_root.as_ref())
}

fn absolute_path(path: &Path) -> Result<PathBuf, PathError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    Ok(normalize_path(&path))
}

fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf, PathError> {
    let mut probe = absolute_path(path)?;
    let mut suffix = Vec::new();

    while !probe.exists() {
        let Some(name) = probe.file_name() else {
            break;
        };
        suffix.push(name.to_owned());
        let parent = probe.parent().unwrap_or_else(|| Path::new("/"));
        if parent == probe {
            break;
        }
        probe = parent.to_path_buf();
    }

    let canonical_base = if probe.exists() {
        fs::canonicalize(&probe)?
    } else {
        probe
    };

    let mut result = canonical_base;
    for name in suffix.iter().rev() {
        result.push(name);
    }
    Ok(normalize_path(&result))
}

fn is_inside_absolute(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }

    normalized
}

fn normalize_separators(value: &str) -> String {
    value.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_DENY_GLOBS;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn path_boundaries_and_parent_escape_are_rejected() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root");

        assert!(is_inside_root(&root, &root));
        assert!(is_inside_root(root.join("child"), &root));
        assert!(!is_inside_root(root.join("../outside"), &root));
        assert!(matches!(
            resolve_workspace_path(&root, "../outside"),
            Err(PathError::OutsideWorkspace(_))
        ));
    }

    #[test]
    fn custom_and_default_style_deny_globs_are_enforced() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root");

        let patterns = vec![
            "**/.env".to_owned(),
            "private/**".to_owned(),
            "**/*token*".to_owned(),
        ];
        let rules = DenyRules::new(&patterns).expect("deny rules");

        assert!(matches!(
            rules.check(&root.join(".env"), &root),
            Err(PathError::Denied { .. })
        ));
        assert!(matches!(
            rules.check(&root.join("private/data.txt"), &root),
            Err(PathError::Denied { .. })
        ));
        assert!(matches!(
            rules.check(&root.join("nested/API_TOKEN.txt"), &root),
            Err(PathError::Denied { .. })
        ));
        rules
            .check(&root.join("public/readme.md"), &root)
            .expect("public path");
    }

    #[test]
    fn all_reference_default_sensitive_patterns_are_enforced() {
        let temp = tempdir().expect("temporary directory");
        let root = temp.path().join("root");
        fs::create_dir(&root).expect("root");
        let patterns = DEFAULT_DENY_GLOBS
            .iter()
            .map(|pattern| (*pattern).to_owned())
            .collect::<Vec<_>>();
        let rules = DenyRules::new(&patterns).expect("default deny rules");

        for path in [
            ".env",
            "nested/.env.local",
            "nested/id_rsa",
            "nested/id_ed25519",
            "nested/access_token.txt",
            "nested/client_secret.json",
            "nested/key.txt",
            "nested/private.key",
            "nested/certificate.pem",
            ".git/config",
            "AppData/Local/state.json",
        ] {
            assert!(
                matches!(
                    rules.check(&root.join(path), &root),
                    Err(PathError::Denied { .. })
                ),
                "sensitive path was not denied: {path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("temporary directory");
        let allowed = temp.path().join("allowed");
        let outside = temp.path().join("outside");
        fs::create_dir(&allowed).expect("allowed");
        fs::create_dir(&outside).expect("outside");
        fs::write(outside.join("private.txt"), "private").expect("private file");
        symlink(&outside, allowed.join("escape")).expect("symlink");

        assert!(matches!(
            resolve_workspace_path(&allowed, "escape/private.txt"),
            Err(PathError::SymlinkEscape(_))
        ));
    }
}
