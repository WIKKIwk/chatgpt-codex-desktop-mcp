use std::collections::HashSet;

use super::super::tool_error::ToolError;
use super::results::{
    FileOutput, OpenProjectOutput, ProjectStateOutput, ReadFilesOutput, TextOutput,
};
use super::shared::{bounded, internal_error, process_stdout_or_error, search_error};
use super::{
    ForgeHandler, OpenProjectRequest, ProjectStateRequest, ReadFilesRequest, SearchCodeRequest,
};
use crate::{
    process::{ProcessInput, run_process},
    project::detect_project_type,
    redaction::redact_text,
    workspace::{
        DenyRules, GitDiffOptions, SearchOptions, git_diff, git_status, project_tree,
        read_file_capped, relative_display_path, search_text_files,
    },
};

pub(super) async fn open_project(
    handler: &ForgeHandler,
    request: OpenProjectRequest,
) -> Result<OpenProjectOutput, ToolError> {
    let depth = bounded(request.tree_depth, 2, 1, 4, "treeDepth")?;
    let workspace = handler
        .workspaces
        .lock()
        .map_err(|_| internal_error("workspace registry is unavailable"))?
        .open(&request.path)
        .map_err(|error| super::shared::invalid_params(error.to_string()))?;
    let deny_rules = DenyRules::new(&handler.config.deny_globs)
        .map_err(|error| internal_error(error.to_string()))?;
    let tree_root = workspace.root.clone();
    let tree_deny_rules = deny_rules.clone();
    let tree_max_output_bytes = handler.config.max_output_bytes as usize;
    let tree_task = tokio::task::spawn_blocking(move || {
        project_tree(
            &tree_root,
            &tree_root,
            &tree_deny_rules,
            depth,
            tree_max_output_bytes,
        )
    });
    let project_type_task = detect_project_type(&workspace.root);
    let git_task = run_process(ProcessInput {
        command: "git".to_owned(),
        args: vec!["status".to_owned(), "--short".to_owned()],
        cwd: workspace.root.clone(),
        timeout_ms: 15_000,
        max_output_bytes: handler.config.max_output_bytes.min(40_000) as usize,
    });
    let (tree_result, project_type, git) = tokio::join!(tree_task, project_type_task, git_task);
    let tree = tree_result
        .map_err(|error| internal_error(format!("tree worker failed: {error}")))?
        .map_err(search_error)?;
    let git_status = if git.exit_code == Some(0) {
        if git.stdout.trim().is_empty() {
            "(clean)".to_owned()
        } else {
            git.stdout.trim_end().to_owned()
        }
    } else {
        format!(
            "(not a Git worktree: {})",
            if git.stderr.trim().is_empty() {
                format!("exit {:?}", git.exit_code)
            } else {
                git.stderr.trim().to_owned()
            }
        )
    };
    let tree_text = if tree.is_empty() {
        "(empty)".to_owned()
    } else {
        tree
    };
    let summary = format!(
        "Workspace: {}\n\nRoot: {}\n\nProject type: {}\n\nGit status:\n{}\n\nProject tree:\n{}",
        workspace.id,
        workspace.root.display(),
        project_type,
        git_status,
        tree_text,
    );
    Ok(OpenProjectOutput {
        result: redact_text(&summary),
        workspace_id: workspace.id,
        root: workspace.root.display().to_string(),
        project_type,
        tree: redact_text(&tree_text),
        git_status: redact_text(&git_status),
    })
}

pub(super) async fn project_state(
    handler: &ForgeHandler,
    request: ProjectStateRequest,
) -> Result<ProjectStateOutput, ToolError> {
    let workspace = handler.resolve_workspace(&request.workspace_id, ".")?;
    let max_bytes = handler.config.max_output_bytes.min(80_000) as usize;
    let (status_result, unstaged_result, staged_result) = tokio::join!(
        git_status(&workspace.workspace.root, max_bytes),
        git_diff(
            &workspace.workspace.root,
            GitDiffOptions {
                staged: false,
                path: None,
                stat_only: true,
                max_bytes,
            }
        ),
        git_diff(
            &workspace.workspace.root,
            GitDiffOptions {
                staged: true,
                path: None,
                stat_only: true,
                max_bytes,
            }
        )
    );
    let status = process_stdout_or_error(&status_result, "(clean)");
    let unstaged = process_stdout_or_error(&unstaged_result, "(none)");
    let staged = process_stdout_or_error(&staged_result, "(none)");
    let result = format!(
        "Git status:\n{}\n\nUnstaged diff:\n{}\n\nStaged diff:\n{}",
        status, unstaged, staged
    );
    Ok(ProjectStateOutput {
        result: redact_text(&result),
        status: redact_text(&status),
        unstaged: redact_text(&unstaged),
        staged: redact_text(&staged),
    })
}

pub(super) async fn search_code(
    handler: &ForgeHandler,
    request: SearchCodeRequest,
) -> Result<TextOutput, ToolError> {
    if request.pattern.is_empty() {
        return Err(super::shared::invalid_params("pattern must not be empty"));
    }
    let path = request.path.as_deref().unwrap_or(".");
    let resolved = handler.resolve_workspace(&request.workspace_id, path)?;
    let options = SearchOptions {
        pattern: request.pattern,
        case_sensitive: request.case_sensitive.unwrap_or(false),
        context_lines: bounded(request.context_lines, 2, 0, 10, "contextLines")?,
        max_matches: bounded(request.max_matches, 200, 1, 1_000, "maxMatches")?,
        include: request.include,
        exclude: request.exclude,
    };
    let deny_rules = DenyRules::new(&handler.config.deny_globs)
        .map_err(|error| internal_error(error.to_string()))?;
    let workspace_root = resolved.workspace.root.clone();
    let absolute_path = resolved.absolute_path.clone();
    let max_read_bytes = handler.config.max_read_bytes as usize;
    let max_output_bytes = handler.config.max_output_bytes as usize;
    let output = tokio::task::spawn_blocking(move || {
        search_text_files(
            &workspace_root,
            &absolute_path,
            &deny_rules,
            max_read_bytes,
            max_output_bytes,
            &options,
        )
    })
    .await
    .map_err(|error| internal_error(format!("search worker failed: {error}")))?
    .map_err(search_error)?;
    let result = if output.is_empty() {
        "(no matches)".to_owned()
    } else {
        output
    };
    Ok(TextOutput {
        result: redact_text(&result),
    })
}

pub(super) async fn read_files(
    handler: &ForgeHandler,
    request: ReadFilesRequest,
) -> Result<ReadFilesOutput, ToolError> {
    if request.paths.is_empty() || request.paths.len() > 20 {
        return Err(super::shared::invalid_params(
            "paths must contain between 1 and 20 entries",
        ));
    }
    let max_per_file = bounded(
        request.max_bytes_per_file,
        50_000,
        1_000,
        100_000,
        "maxBytesPerFile",
    )?;
    let workspace_id = request.workspace_id;
    let mut seen = HashSet::new();
    let paths = request
        .paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect::<Vec<_>>();
    let fair_share = (handler.config.max_output_bytes as usize / paths.len()).max(1_000);
    let cap = max_per_file
        .min(handler.config.max_read_bytes as usize)
        .min(fair_share);
    let mut reads = Vec::with_capacity(paths.len());
    for path in paths {
        let resolved = handler.resolve_workspace(&workspace_id, &path)?;
        let display_path = relative_display_path(&resolved.workspace.root, &resolved.absolute_path);
        reads.push(tokio::task::spawn_blocking(move || {
            read_file_capped(&resolved.absolute_path, cap).map(|file| (display_path, file))
        }));
    }
    let mut sections = Vec::with_capacity(reads.len());
    let mut files = Vec::with_capacity(reads.len());
    for read in reads {
        let (display_path, file) = read
            .await
            .map_err(|error| internal_error(format!("file read worker failed: {error}")))?
            .map_err(|error| internal_error(error.to_string()))?;
        let content = redact_text(&file.content);
        sections.push(format!(
            "--- {}{} ---\n{}",
            display_path,
            if file.truncated { " [truncated]" } else { "" },
            content
        ));
        files.push(FileOutput {
            path: display_path,
            content,
            truncated: file.truncated,
        });
    }
    let result = redact_text(&sections.join("\n\n"));
    Ok(ReadFilesOutput { result, files })
}
