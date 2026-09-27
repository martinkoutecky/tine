//! Closed page-content edit vocabulary. Kinds travel with requests and are never persisted.

use serde::{Deserialize, Serialize};

/// The user's page-content intent. Composite edits carry several values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditKind {
    /// Insert new blocks or a copied subtree.
    InsertBlocks,
    /// Change one block's text or properties, including the page pre-block.
    SaveBlock,
    /// Move existing blocks while keeping their identity.
    MoveBlocks,
    /// Remove blocks.
    DeleteBlocks,
    /// Create a page file.
    CreatePage,
    /// Delete a page file.
    DeletePage,
    /// Rename a page file and its references.
    RenamePage,
    /// Replace whole-page text without a finer intent.
    ReplacePage,
}

#[cfg(test)]
mod tests {
    use super::EditKind;

    #[test]
    fn fixture_pins_closed_vocabulary() {
        let fixture: Vec<String> =
            serde_json::from_str(include_str!("../../../edit-kinds.json")).unwrap();
        let actual = [
            EditKind::InsertBlocks,
            EditKind::SaveBlock,
            EditKind::MoveBlocks,
            EditKind::DeleteBlocks,
            EditKind::CreatePage,
            EditKind::DeletePage,
            EditKind::RenamePage,
            EditKind::ReplacePage,
        ];
        // An added enum variant must be considered here even if the fixture is unchanged.
        let name = |kind: EditKind| match kind {
            EditKind::InsertBlocks => "insert-blocks",
            EditKind::SaveBlock => "save-block",
            EditKind::MoveBlocks => "move-blocks",
            EditKind::DeleteBlocks => "delete-blocks",
            EditKind::CreatePage => "create-page",
            EditKind::DeletePage => "delete-page",
            EditKind::RenamePage => "rename-page",
            EditKind::ReplacePage => "replace-page",
        };
        let encoded: Vec<String> = actual
            .iter()
            .map(|kind| {
                serde_json::to_value(kind)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            actual
                .iter()
                .map(|kind| name(*kind).to_owned())
                .collect::<Vec<_>>(),
            encoded
        );
        assert_eq!(encoded, fixture, "a new edit kind needs an ADR arguing why the existing kinds cannot express it, plus Martin's approval (OG-RULES Rule 8); exemplar src/document/save/engine.ts");
        assert_eq!(actual.len(), 8, "a new edit kind needs an ADR arguing why the existing kinds cannot express it, plus Martin's approval (OG-RULES Rule 8); exemplar src/document/save/engine.ts");
        assert!(serde_json::from_str::<EditKind>("\"unlisted-kind\"").is_err());
    }
}
