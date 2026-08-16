use std::{io, path::PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum Change {
    #[serde(rename = "replace_text")]
    ReplaceText {
        path: String,
        #[serde(rename = "oldText")]
        #[schemars(required)]
        old_text: Option<String>,
        #[serde(rename = "newText")]
        #[schemars(required)]
        new_text: Option<String>,
    },
    #[serde(rename = "replace_range")]
    ReplaceRange {
        path: String,
        #[serde(rename = "startLine")]
        #[schemars(required, range(min = 1))]
        start_line: Option<usize>,
        #[serde(rename = "endLine")]
        #[schemars(required, range(min = 1))]
        end_line: Option<usize>,
        #[serde(rename = "newText")]
        #[schemars(required)]
        new_text: Option<String>,
    },
    #[serde(rename = "insert_before")]
    InsertBefore {
        path: String,
        #[serde(rename = "anchor")]
        #[schemars(required)]
        anchor: Option<String>,
        #[serde(rename = "text")]
        #[schemars(required)]
        text: Option<String>,
    },
    #[serde(rename = "insert_after")]
    InsertAfter {
        path: String,
        #[serde(rename = "anchorAfter")]
        #[schemars(required)]
        anchor_after: Option<String>,
        #[serde(rename = "text")]
        #[schemars(required)]
        text: Option<String>,
    },
    #[serde(rename = "append")]
    Append {
        path: String,
        #[serde(rename = "text")]
        #[schemars(required)]
        text: Option<String>,
    },
    #[serde(rename = "create")]
    Create {
        path: String,
        #[serde(rename = "text")]
        #[schemars(required)]
        text: Option<String>,
    },
    #[serde(rename = "overwrite")]
    Overwrite {
        path: String,
        #[serde(rename = "newText")]
        #[schemars(required)]
        new_text: Option<String>,
    },
    #[serde(rename = "rename")]
    Rename {
        path: String,
        #[serde(rename = "newPath")]
        #[schemars(required)]
        new_path: Option<String>,
    },
    #[serde(rename = "delete")]
    Delete { path: String },
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Self::ReplaceText { path, .. }
            | Self::ReplaceRange { path, .. }
            | Self::InsertBefore { path, .. }
            | Self::InsertAfter { path, .. }
            | Self::Append { path, .. }
            | Self::Create { path, .. }
            | Self::Overwrite { path, .. }
            | Self::Rename { path, .. }
            | Self::Delete { path } => path,
        }
    }

    pub fn new_path(&self) -> Option<&str> {
        match self {
            Self::Rename { new_path, .. } => new_path.as_deref(),
            _ => None,
        }
    }

    pub fn edit_type(&self) -> EditType {
        match self {
            Self::ReplaceText { .. } => EditType::ReplaceText,
            Self::ReplaceRange { .. } => EditType::ReplaceRange,
            Self::InsertBefore { .. } => EditType::InsertBefore,
            Self::InsertAfter { .. } => EditType::InsertAfter,
            Self::Append { .. } => EditType::Append,
            Self::Create { .. } => EditType::Create,
            Self::Overwrite { .. } => EditType::Overwrite,
            Self::Rename { .. } => EditType::Rename,
            Self::Delete { .. } => EditType::Delete,
        }
    }

    pub fn text_values(&self) -> Vec<&str> {
        match self {
            Self::ReplaceText {
                old_text, new_text, ..
            } => optional_values(old_text.as_ref(), new_text.as_ref()),
            Self::ReplaceRange { new_text, .. } | Self::Overwrite { new_text, .. } => {
                new_text.as_deref().into_iter().collect()
            }
            Self::InsertBefore { anchor, text, .. } => {
                optional_values(anchor.as_ref(), text.as_ref())
            }
            Self::InsertAfter {
                anchor_after, text, ..
            } => optional_values(anchor_after.as_ref(), text.as_ref()),
            Self::Append { text, .. } | Self::Create { text, .. } => {
                text.as_deref().into_iter().collect()
            }
            Self::Rename { .. } | Self::Delete { .. } => Vec::new(),
        }
    }
}

fn optional_values<'a>(first: Option<&'a String>, second: Option<&'a String>) -> Vec<&'a str> {
    [first, second]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditType {
    ReplaceText,
    ReplaceRange,
    InsertBefore,
    InsertAfter,
    Append,
    Create,
    Overwrite,
    Rename,
    Delete,
}

impl EditType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReplaceText => "replace_text",
            Self::ReplaceRange => "replace_range",
            Self::InsertBefore => "insert_before",
            Self::InsertAfter => "insert_after",
            Self::Append => "append",
            Self::Create => "create",
            Self::Overwrite => "overwrite",
            Self::Rename => "rename",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    pub path: String,
    pub edit_type: EditType,
    pub diff: String,
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Validation(String),
    #[error("unknown action_id: {0}")]
    UnknownAction(String),
}

pub(crate) fn resolve_edit_path(root: &std::path::Path, path: &str) -> Result<PathBuf, EditError> {
    crate::workspace::resolve_workspace_path(root, path)
        .map_err(|error| EditError::Validation(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn change_schema_requires_operation_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(Change)).expect("change schema");
        let variants = schema["oneOf"].as_array().expect("change variants");
        let required_fields = [
            ("replace_text", ["oldText", "newText"].as_slice()),
            (
                "replace_range",
                ["startLine", "endLine", "newText"].as_slice(),
            ),
            ("insert_before", ["anchor", "text"].as_slice()),
            ("insert_after", ["anchorAfter", "text"].as_slice()),
            ("append", ["text"].as_slice()),
            ("create", ["text"].as_slice()),
            ("overwrite", ["newText"].as_slice()),
            ("rename", ["newPath"].as_slice()),
        ];

        for (kind, fields) in required_fields {
            let variant = variants
                .iter()
                .find(|variant| variant["properties"]["type"]["const"] == kind)
                .expect("edit variant");
            let required = variant["required"]
                .as_array()
                .expect("required fields")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            for field in fields {
                assert!(required.contains(field), "{kind} must require {field}");
            }
        }
    }
}
