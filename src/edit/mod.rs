mod apply;
mod model;
mod preview;
mod store;

pub use apply::apply_changes;
pub use model::{Change, DiffEntry, EditError, EditType};
pub use preview::preview_changes;
pub use store::{EditStore, PendingEdit};

#[cfg(test)]
mod tests;
