use std::{io::ErrorKind, path::Path};

use tokio::fs;

use super::model::{Change, DiffEntry, EditError, EditType, resolve_edit_path};

pub async fn preview_changes(root: &Path, changes: &[Change]) -> Result<Vec<DiffEntry>, EditError> {
    let mut diffs = Vec::with_capacity(changes.len());
    for change in changes {
        diffs.push(preview_one(root, change).await?);
    }
    Ok(diffs)
}

async fn preview_one(root: &Path, change: &Change) -> Result<DiffEntry, EditError> {
    let full_path = resolve_edit_path(root, change.path())?;
    match change {
        Change::ReplaceText {
            path,
            old_text,
            new_text,
        } => {
            let content = try_read(&full_path).await?;
            let replacement = new_text.as_deref().unwrap_or("");
            let Some(content) = content else {
                if old_text.as_deref().is_some_and(|value| !value.is_empty()) {
                    return Err(validation(format!("File does not exist: {path}")));
                }
                return Ok(new_file_diff(path, replacement));
            };
            let Some(old_text) = old_text.as_deref() else {
                return Err(validation(
                    "oldText is required for replace_text on an existing file.",
                ));
            };
            if !old_text.is_empty() && !content.contains(old_text) {
                return Err(validation(format!("oldText not found in {path}")));
            }
            Ok(DiffEntry {
                path: path.clone(),
                edit_type: EditType::ReplaceText,
                diff: line_change_diff(
                    path,
                    &format!(
                        "@@ replace {} line(s) with {} line(s) @@",
                        count_lines(old_text),
                        count_lines(replacement)
                    ),
                    old_text,
                    replacement,
                ),
            })
        }
        Change::ReplaceRange {
            path,
            start_line,
            end_line,
            new_text,
        } => {
            let content = assert_read(&full_path, path).await?;
            let lines = content.split('\n').collect::<Vec<_>>();
            let (start, end) = checked_range(start_line, end_line, lines.len())?;
            let old_text = lines[start..=end].join("\n");
            let replacement = new_text.as_deref().unwrap_or("");
            Ok(DiffEntry {
                path: path.clone(),
                edit_type: EditType::ReplaceRange,
                diff: line_change_diff(
                    path,
                    &format!(
                        "@@ L{}-L{}: {} \u{2192} {} line(s) @@",
                        start + 1,
                        end + 1,
                        count_lines(&old_text),
                        count_lines(replacement)
                    ),
                    &old_text,
                    replacement,
                ),
            })
        }
        Change::InsertBefore { path, anchor, text } => {
            let content = assert_read(&full_path, path).await?;
            let anchor = anchor.as_deref().unwrap_or("");
            if !content.contains(anchor) {
                return Err(validation(format!("anchor not found in {path}")));
            }
            Ok(insert_diff(
                path,
                EditType::InsertBefore,
                &format!("@@ insert before \"{}\" @@", truncate(anchor, 40)),
                text.as_deref().unwrap_or(""),
            ))
        }
        Change::InsertAfter {
            path,
            anchor_after,
            text,
        } => {
            let content = assert_read(&full_path, path).await?;
            let anchor = anchor_after.as_deref().unwrap_or("");
            if !content.contains(anchor) {
                return Err(validation(format!("anchor not found in {path}")));
            }
            Ok(insert_diff(
                path,
                EditType::InsertAfter,
                &format!("@@ insert after \"{}\" @@", truncate(anchor, 40)),
                text.as_deref().unwrap_or(""),
            ))
        }
        Change::Append { path, text } => Ok(insert_diff(
            path,
            EditType::Append,
            &format!(
                "@@ append {} line(s) @@",
                count_lines(text.as_deref().unwrap_or(""))
            ),
            text.as_deref().unwrap_or(""),
        )),
        Change::Create { path, text } => {
            if try_read(&full_path).await?.is_some() {
                return Err(validation(format!(
                    "File already exists: {path}. Use overwrite instead."
                )));
            }
            Ok(new_file_diff(path, text.as_deref().unwrap_or("")))
        }
        Change::Overwrite { path, new_text } => {
            let old_summary = try_read(&full_path)
                .await?
                .map(|content| format!("{} line(s)", count_lines(&content)))
                .unwrap_or_else(|| "(new file)".to_owned());
            let new_summary = format!("{} line(s)", count_lines(new_text.as_deref().unwrap_or("")));
            Ok(DiffEntry {
                path: path.clone(),
                edit_type: EditType::Overwrite,
                diff: format!(
                    "--- {path} ({old_summary})\n+++ {path}\n@@ overwrite: {old_summary} \u{2192} {new_summary} @@"
                ),
            })
        }
        Change::Rename { path, new_path } => {
            let target = new_path
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| validation("newPath is required for rename"))?;
            let target_full = resolve_edit_path(root, target)?;
            if try_read(&full_path).await?.is_none() {
                return Err(validation(format!("File does not exist: {path}")));
            }
            if try_read(&target_full).await?.is_some() {
                return Err(validation(format!(
                    "Target already exists: {target}. Use overwrite or delete first."
                )));
            }
            Ok(DiffEntry {
                path: path.clone(),
                edit_type: EditType::Rename,
                diff: format!("Rename: {path} \u{2192} {target}"),
            })
        }
        Change::Delete { path } => {
            assert_read(&full_path, path).await?;
            Ok(DiffEntry {
                path: path.clone(),
                edit_type: EditType::Delete,
                diff: format!("Delete: {path}"),
            })
        }
    }
}

async fn try_read(path: &Path) -> Result<Option<String>, EditError> {
    match fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(EditError::Io(error)),
    }
}

async fn assert_read(path: &Path, display_path: &str) -> Result<String, EditError> {
    try_read(path)
        .await?
        .ok_or_else(|| validation(format!("File does not exist: {display_path}")))
}

fn checked_range(
    start_line: &Option<usize>,
    end_line: &Option<usize>,
    line_count: usize,
) -> Result<(usize, usize), EditError> {
    let start = start_line.unwrap_or(1);
    let end = end_line.unwrap_or(start);
    if start == 0 || start > line_count {
        return Err(validation(format!(
            "startLine {start} out of range (file has {line_count} lines)"
        )));
    }
    if end < start || end > line_count {
        return Err(validation(format!("endLine {end} out of range")));
    }
    Ok((start - 1, end - 1))
}

fn line_change_diff(path: &str, header: &str, old_text: &str, new_text: &str) -> String {
    let mut lines = vec![
        format!("--- {path}"),
        format!("+++ {path}"),
        header.to_owned(),
    ];
    lines.extend(old_text.split('\n').take(20).map(|line| format!("-{line}")));
    lines.extend(new_text.split('\n').take(20).map(|line| format!("+{line}")));
    lines.join("\n")
}

fn insert_diff(path: &str, edit_type: EditType, header: &str, text: &str) -> DiffEntry {
    let mut lines = vec![
        format!("--- {path}"),
        format!("+++ {path}"),
        header.to_owned(),
    ];
    lines.extend(text.split('\n').take(20).map(|line| format!("+{line}")));
    DiffEntry {
        path: path.to_owned(),
        edit_type,
        diff: lines.join("\n"),
    }
}

fn new_file_diff(path: &str, text: &str) -> DiffEntry {
    DiffEntry {
        path: path.to_owned(),
        edit_type: EditType::Create,
        diff: format!(
            "--- (new file)\n+++ {path}\n@@ -0,0 +1,{} @@\n{}",
            count_lines(text),
            prepend_plus(text)
        ),
    }
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.split('\n').count()
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut value = text.chars().take(max).collect::<String>();
    if text.chars().count() > max {
        value.push('\u{2026}');
    }
    value
}

fn prepend_plus(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    text.split('\n')
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn validation(message: impl Into<String>) -> EditError {
    EditError::Validation(message.into())
}
