use std::{
    fs::{self, DirEntry},
    io,
    path::Path,
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use thiserror::Error;

use super::{DenyRules, relative_display_path, resolve_workspace_path};

const IGNORED_DIRECTORIES: [&str; 3] = ["node_modules", "dist", ".git"];

mod in_process;
mod index;
mod walker;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid glob '{pattern}': {source}")]
    InvalidGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("could not build glob set: {0}")]
    BuildGlob(#[from] globset::Error),
    #[error("search matcher error: {0}")]
    Matcher(String),
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub pattern: String,
    pub case_sensitive: bool,
    pub context_lines: usize,
    pub max_matches: usize,
    pub include: Option<String>,
    pub exclude: Option<String>,
}

pub fn search_text_files(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    max_read_bytes: usize,
    max_output_bytes: usize,
    options: &SearchOptions,
) -> Result<String, SearchError> {
    fs::metadata(start_path)?;
    let include = GlobMatcher::new(options.include.as_deref())?;
    let exclude = GlobMatcher::new(options.exclude.as_deref())?;
    in_process::search(
        workspace_root,
        start_path,
        deny_rules,
        max_read_bytes,
        max_output_bytes,
        options,
        include.as_ref(),
        exclude.as_ref(),
    )
}

pub fn find_files(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<String>, SearchError> {
    fs::metadata(start_path)?;
    let matcher = GlobMatcher::new(Some(pattern))?.expect("pattern matcher");
    let mut results = Vec::new();
    walk_find(
        workspace_root,
        start_path,
        deny_rules,
        &matcher,
        max_results,
        &mut results,
    );
    Ok(results)
}

pub fn project_tree(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    depth: usize,
    max_output_bytes: usize,
) -> Result<String, SearchError> {
    fs::metadata(start_path)?;
    let mut lines = Vec::new();
    walk_tree(
        workspace_root,
        start_path,
        0,
        depth,
        "",
        deny_rules,
        &mut lines,
    );
    Ok(cap_text(lines.join("\n"), max_output_bytes))
}

fn walk_find(
    workspace_root: &Path,
    path: &Path,
    deny_rules: &DenyRules,
    matcher: &GlobMatcher,
    max_results: usize,
    results: &mut Vec<String>,
) {
    if results.len() >= max_results || !is_safe_path(workspace_root, path, deny_rules) {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.is_dir() {
        let Ok(entries) = sorted_entries(path) else {
            return;
        };
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry
                .file_type()
                .map(|kind| kind.is_symlink())
                .unwrap_or(true)
            {
                continue;
            }
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                && IGNORED_DIRECTORIES.contains(&name.as_str())
            {
                continue;
            }
            walk_find(
                workspace_root,
                &entry.path(),
                deny_rules,
                matcher,
                max_results,
                results,
            );
            if results.len() >= max_results {
                break;
            }
        }
        return;
    }
    if metadata.is_file() {
        let relative_path = relative_display_path(workspace_root, path);
        if matcher.matches(&relative_path) {
            results.push(relative_path);
        }
    }
}

fn walk_tree(
    workspace_root: &Path,
    path: &Path,
    current_depth: usize,
    max_depth: usize,
    prefix: &str,
    deny_rules: &DenyRules,
    lines: &mut Vec<String>,
) {
    if current_depth > max_depth || !is_safe_path(workspace_root, path, deny_rules) {
        return;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if !metadata.is_dir() {
        lines.push(format!(
            "📄 {}",
            relative_display_path(workspace_root, path)
        ));
        return;
    }
    let Ok(entries) = sorted_entries(path) else {
        return;
    };
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            if !IGNORED_DIRECTORIES.contains(&name.as_str()) {
                directories.push((name, entry.path()));
            }
        } else if kind.is_file() {
            files.push(name);
        }
    }
    if current_depth > 0 {
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default();
        lines.push(format!("{prefix}📁 {name}/"));
    }
    let child_prefix = if current_depth == 0 {
        String::new()
    } else {
        format!("{prefix}  ")
    };
    for (_, directory) in directories {
        walk_tree(
            workspace_root,
            &directory,
            current_depth + 1,
            max_depth,
            &format!("{child_prefix}  "),
            deny_rules,
            lines,
        );
    }
    for file in files {
        lines.push(format!("{child_prefix}📄 {file}"));
    }
}

fn sorted_entries(path: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn is_safe_path(workspace_root: &Path, path: &Path, deny_rules: &DenyRules) -> bool {
    resolve_workspace_path(workspace_root, path).is_ok()
        && deny_rules.check(path, workspace_root).is_ok()
}

fn cap_text(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    text.truncate(valid_utf8_boundary(&text, max_bytes));
    text.push_str("\n[output truncated]\n");
    text
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn valid_utf8_boundary(value: &str, max_bytes: usize) -> usize {
    value
        .char_indices()
        .take_while(|(index, _)| *index <= max_bytes)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0)
}

struct GlobMatcher {
    set: GlobSet,
}

impl GlobMatcher {
    fn new(pattern: Option<&str>) -> Result<Option<Self>, SearchError> {
        let Some(pattern) = pattern else {
            return Ok(None);
        };
        let mut builder = GlobSetBuilder::new();
        for raw in split_glob_patterns(pattern) {
            let glob = GlobBuilder::new(&raw)
                .literal_separator(true)
                .case_insensitive(true)
                .build()
                .map_err(|source| SearchError::InvalidGlob {
                    pattern: raw,
                    source,
                })?;
            builder.add(glob);
        }
        Ok(Some(Self {
            set: builder.build()?,
        }))
    }

    fn matches(&self, path: &str) -> bool {
        self.set.is_match(path)
            || Path::new(path)
                .file_name()
                .is_some_and(|name| self.set.is_match(name.to_string_lossy().as_ref()))
    }
}

pub(super) fn split_glob_patterns(value: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                let item = value[start..index].trim();
                if !item.is_empty() {
                    patterns.push(item.to_owned());
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    let item = value[start..].trim();
    if !item.is_empty() {
        patterns.push(item.to_owned());
    }
    patterns
}

#[cfg(test)]
mod tests;
