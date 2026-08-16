use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use super::{GlobMatcher, IGNORED_DIRECTORIES, is_safe_path};
use crate::workspace::DenyRules;

pub(super) fn collect_files(
    workspace_root: &Path,
    start_path: &Path,
    deny_rules: &DenyRules,
    include: Option<&GlobMatcher>,
    exclude: Option<&GlobMatcher>,
) -> Vec<PathBuf> {
    let filter_deny_rules = deny_rules.clone();
    let filter_root = workspace_root.to_path_buf();
    let mut walker = WalkBuilder::new(start_path);
    walker
        .hidden(false)
        .follow_links(false)
        .sort_by_file_path(|left, right| left.cmp(right))
        .filter_entry(move |entry| entry_allowed(&filter_root, &filter_deny_rules, entry.path()));

    walker
        .build()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| path.is_file())
        .filter(|path| is_safe_path(workspace_root, path, deny_rules))
        .filter(|path| {
            let relative = super::relative_display_path(workspace_root, path);
            !exclude.is_some_and(|matcher| matcher.matches(&relative))
                && include.is_none_or(|matcher| matcher.matches(&relative))
        })
        .collect()
}

fn entry_allowed(workspace_root: &Path, deny_rules: &DenyRules, path: &Path) -> bool {
    let Ok(metadata) = path.symlink_metadata() else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    if metadata.is_dir()
        && path
            .file_name()
            .is_some_and(|name| IGNORED_DIRECTORIES.contains(&name.to_string_lossy().as_ref()))
    {
        return false;
    }
    is_safe_path(workspace_root, path, deny_rules)
}
