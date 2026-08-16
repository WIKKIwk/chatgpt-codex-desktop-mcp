mod files;
mod git;
mod paths;
mod registry;
mod search;

pub use files::{FILE_TRUNCATION_MARKER, ReadFileResult, list_directory, read_file_capped};
pub use git::{GitDiffOptions, git_diff, git_status};
pub use paths::{
    DenyRules, PathError, assert_allowed_path, assert_not_denied, is_inside_root,
    relative_display_path, resolve_workspace_path,
};
pub use registry::{ResolvedWorkspace, Workspace, WorkspaceError, WorkspaceRegistry};
pub use search::{SearchError, SearchOptions, find_files, project_tree, search_text_files};
