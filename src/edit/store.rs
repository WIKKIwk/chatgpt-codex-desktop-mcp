use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::model::{Change, DiffEntry, EditError};

#[derive(Debug, Clone)]
pub struct PendingEdit {
    pub id: String,
    pub workspace_id: String,
    pub changes: Vec<Change>,
    pub diffs: Vec<DiffEntry>,
    pub created_at: u64,
}

#[derive(Debug, Default)]
pub struct EditStore {
    edits: HashMap<String, PendingEdit>,
}

impl EditStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &mut self,
        workspace_id: String,
        changes: Vec<Change>,
        diffs: Vec<DiffEntry>,
    ) -> PendingEdit {
        let edit = PendingEdit {
            id: format!("edit_{}", Uuid::new_v4()),
            workspace_id,
            changes,
            diffs,
            created_at: now_millis(),
        };
        self.edits.insert(edit.id.clone(), edit.clone());
        edit
    }

    pub fn take(&mut self, action_id: &str) -> Result<PendingEdit, EditError> {
        self.edits
            .remove(action_id)
            .ok_or_else(|| EditError::UnknownAction(action_id.to_owned()))
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
