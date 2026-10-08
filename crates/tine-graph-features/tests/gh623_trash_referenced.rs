//! GH #623 (comment 17): trashing an image after dropping its reference from
//! one block. The backend keeps a file the published graph still references and
//! reports `TrashOutcome::Referenced`, never an error to match by text; once
//! the saves that dropped every reference have landed, the same call trashes
//! the file.
use std::fs;
use tine_core::model::PageKind;
use tine_graph_features::{
    assets::{self, TrashOutcome},
    pages,
};
use tine_store::{EditKind, SaveOutcome, Store};

fn fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (path, bytes) in files {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (root, store)
}

fn drop_reference(store: &Store, name: &str) {
    let read = pages::get_page(store, name, PageKind::Page)
        .unwrap()
        .unwrap();
    let mut doc = read.doc.clone();
    for block in &mut doc.blocks {
        block.raw = block
            .raw
            .replace("![a](../assets/x.png)", "")
            .trim()
            .to_string();
    }
    let saved = pages::save_page(
        store,
        EditKind::SaveBlock,
        &read.id,
        &doc,
        Some(String::from(read.rev)),
        false,
    )
    .unwrap();
    assert!(matches!(saved, SaveOutcome::Saved(_)), "{saved:?}");
}

#[test]
fn a_referenced_asset_is_kept_with_a_typed_outcome_until_every_reference_is_saved_away() {
    let (root, store) = fixture(&[
        ("pages/P.md", "- see ![a](../assets/x.png)\n"),
        ("pages/Q.md", "- also ![a](../assets/x.png)\n"),
        ("assets/x.png", "png"),
    ]);
    let asset = root.path().join("assets/x.png");

    assert_eq!(
        assets::trash_asset(&store, "x.png").unwrap(),
        TrashOutcome::Referenced
    );
    assert!(asset.exists());

    // Dropping the reference from P alone leaves Q's: kept, typed.
    drop_reference(&store, "P");
    assert_eq!(
        assets::trash_asset(&store, "x.png").unwrap(),
        TrashOutcome::Referenced
    );
    assert!(asset.exists());

    // The save that drops the last reference publishes before the trash runs.
    drop_reference(&store, "Q");
    assert_eq!(
        assets::trash_asset(&store, "x.png").unwrap(),
        TrashOutcome::Trashed
    );
    assert!(!asset.exists());
}

#[test]
fn other_trash_failures_stay_errors() {
    let (_root, store) = fixture(&[("pages/P.md", "- hi\n")]);
    assert!(assets::trash_asset(&store, "gone.png").is_err());
    assert!(assets::trash_asset(&store, "../pages/P.md").is_err());
}
