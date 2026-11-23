//! The page payload of the desktop's `graph-changed` window event, which the
//! browser core's edits answer in too: one per active page a publication
//! moved (`Change::pages`).

use tine_core::model::PageKind;
use tine_store::{ChangeKind, FileId};

/// One active page a publication created, changed or removed.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GraphChange {
    /// The page file's graph-relative path.
    pub path: String,
    /// The page's name.
    pub name: String,
    /// Journal or ordinary page.
    pub kind: PageKind,
    /// The file is new.
    pub created: bool,
    /// The page is gone.
    pub removed: bool,
}

impl GraphChange {
    /// One `Change::pages` row as its event. O(name bytes).
    pub fn of(id: &FileId, change: ChangeKind, kind: PageKind, name: &str) -> Self {
        Self {
            path: id.as_str().to_owned(),
            name: name.to_owned(),
            kind,
            created: matches!(change, ChangeKind::Created),
            removed: matches!(change, ChangeKind::Removed),
        }
    }
}
