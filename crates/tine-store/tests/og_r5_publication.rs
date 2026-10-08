//! Neighbors of prepared-parse publication: physical ownership and parser shape.
use std::fs;
use tine_store::{EditKind, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn a_saved_shadow_journal_does_not_replace_the_canonical_published_day() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("journals")).unwrap();
    fs::create_dir(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(dir.path().join("journals/2026_06_26.md"), "- canonical\n").unwrap();
    fs::write(
        dir.path().join("journals/Friday, 26-06-2026.md"),
        "- shadow\n",
    )
    .unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let id = PageId::from("journals/Friday, 26-06-2026.md");
    let read = store.page(&id).unwrap();
    let mut page = read.doc;
    page.blocks[0].raw = "shadow edit".into();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &page
        ),
        SaveOutcome::Saved(_)
    ));
    let corpus = store.whole_graph().unwrap().corpus();
    assert_eq!(corpus.pages.len(), 1);
    assert_eq!(corpus.pages[0].id.as_str(), "journals/2026_06_26.md");
    assert_eq!(corpus.pages[0].document.roots[0].raw(), "canonical");
    assert_eq!(
        fs::read_to_string(dir.path().join(id.as_str())).unwrap(),
        "- shadow edit\n"
    );
    store.close();
}

#[test]
fn published_tree_matches_written_bytes_when_the_dto_does_not_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/A.md"), "- before\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let id = PageId::from("pages/A.md");
    let read = store.page(&id).unwrap();
    let mut page = read.doc;
    page.blocks[0].raw = "after\n- nested".into();
    let live_id = page.blocks[0].id.clone();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &id,
            SaveBase::Existing(read.rev),
            &page
        ),
        SaveOutcome::Saved(_)
    ));
    let bytes = fs::read_to_string(dir.path().join(id.as_str())).unwrap();
    let parsed = tine_core::doc::parse(&bytes);
    let view = store.whole_graph().unwrap().corpus();
    assert_eq!(*view.pages[0].document, parsed);
    assert_eq!(view.pages[0].document.roots[0].uuid, live_id);
    store.close();
}
