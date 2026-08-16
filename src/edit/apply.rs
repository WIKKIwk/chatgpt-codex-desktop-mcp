use std::{io::ErrorKind, path::Path};

use tokio::fs;

use super::model::{Change, EditError, resolve_edit_path};

pub async fn apply_changes(root: &Path, changes: &[Change]) -> Result<(), EditError> {
    for change in changes {
        apply_one(root, change).await?;
    }
    Ok(())
}

async fn apply_one(root: &Path, change: &Change) -> Result<(), EditError> {
    let full_path = resolve_edit_path(root, change.path())?;
    match change {
        Change::ReplaceText {
            path,
            old_text,
            new_text,
        } => {
            let replacement = new_text.as_deref().unwrap_or("");
            let Some(content) = try_read(&full_path).await? else {
                if old_text.as_deref().is_some_and(|value| !value.is_empty()) {
                    return Err(validation(format!("File does not exist: {path}")));
                }
                create_parent(&full_path).await?;
                fs::write(full_path, replacement).await?;
                return Ok(());
            };
            let Some(old_text) = old_text.as_deref() else {
                return Err(validation(
                    "oldText is required for replace_text on an existing file",
                ));
            };
            if old_text.is_empty() {
                return Ok(());
            }
            if !content.contains(old_text) {
                return Err(validation(format!("oldText not found in {path}")));
            }
            fs::write(full_path, content.replacen(old_text, replacement, 1)).await?;
        }
        Change::ReplaceRange {
            path,
            start_line,
            end_line,
            new_text,
        } => {
            let content = assert_read(&full_path, path).await?;
            let mut lines = content.split('\n').collect::<Vec<_>>();
            let (start, end) = checked_range(start_line, end_line, lines.len())?;
            let replacements = new_text
                .as_deref()
                .unwrap_or("")
                .split('\n')
                .collect::<Vec<_>>();
            lines.splice(start..=end, replacements);
            fs::write(full_path, lines.join("\n")).await?;
        }
        Change::InsertBefore { path, anchor, text } => {
            let content = assert_read(&full_path, path).await?;
            let anchor = anchor.as_deref().unwrap_or("");
            let index = content
                .find(anchor)
                .ok_or_else(|| validation(format!("anchor not found in {path}")))?;
            let updated = format!(
                "{}{}{}",
                &content[..index],
                text.as_deref().unwrap_or(""),
                &content[index..]
            );
            fs::write(full_path, updated).await?;
        }
        Change::InsertAfter {
            path,
            anchor_after,
            text,
        } => {
            let content = assert_read(&full_path, path).await?;
            let anchor = anchor_after.as_deref().unwrap_or("");
            let index = content
                .find(anchor)
                .ok_or_else(|| validation(format!("anchor not found in {path}")))?;
            let end = index + anchor.len();
            let updated = format!(
                "{}{}{}",
                &content[..end],
                text.as_deref().unwrap_or(""),
                &content[end..]
            );
            fs::write(full_path, updated).await?;
        }
        Change::Append { text, .. } => {
            create_parent(&full_path).await?;
            let content = try_read(&full_path).await?.unwrap_or_default();
            let needs_newline = !content.is_empty() && !content.ends_with('\n');
            let updated = format!(
                "{content}{}{}",
                if needs_newline { "\n" } else { "" },
                text.as_deref().unwrap_or("")
            );
            fs::write(full_path, updated).await?;
        }
        Change::Create { path, text } => {
            if try_read(&full_path).await?.is_some() {
                return Err(validation(format!("File already exists: {path}")));
            }
            create_parent(&full_path).await?;
            fs::write(full_path, text.as_deref().unwrap_or("")).await?;
        }
        Change::Overwrite { new_text, .. } => {
            create_parent(&full_path).await?;
            fs::write(full_path, new_text.as_deref().unwrap_or("")).await?;
        }
        Change::Rename { path, new_path } => {
            let target = new_path
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| validation("newPath is required for rename"))?;
            let target_path = resolve_edit_path(root, target)?;
            if try_read(&target_path).await?.is_some() {
                return Err(validation(format!("Target already exists: {target}")));
            }
            create_parent(&target_path).await?;
            fs::rename(full_path, target_path).await.map_err(|error| {
                if error.kind() == ErrorKind::NotFound {
                    validation(format!("File does not exist: {path}"))
                } else {
                    EditError::Io(error)
                }
            })?;
        }
        Change::Delete { path } => {
            assert_read(&full_path, path).await?;
            fs::remove_file(full_path).await?;
        }
    }
    Ok(())
}

async fn create_parent(path: &Path) -> Result<(), EditError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    Ok(())
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

fn validation(message: impl Into<String>) -> EditError {
    EditError::Validation(message.into())
}
