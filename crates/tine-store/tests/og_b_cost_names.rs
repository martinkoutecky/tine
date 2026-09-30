//! Structural publication cost at the real guarded Store save door.
use std::fs;
use tine_store::{cost_counters, EditKind, OpenOptions, PageId, SaveBase, SaveOutcome, Store};
#[test]
fn created_page_patches_reference_tables() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    for i in 0..128 {
        fs::write(dir.path().join(format!("pages/P{i}.md")), "- unchanged\n").unwrap();
    }
    let store = Store::open(dir.path(), OpenOptions::default()).unwrap().0;
    let held = store.whole_graph().unwrap();
    let mut doc = store.page(&PageId::from("pages/P0.md")).unwrap().doc;
    doc.name = "Added".into();
    cost_counters::reset();
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &PageId::from("pages/Added.md"),
            SaveBase::CreateNew,
            &doc
        ),
        SaveOutcome::Saved(_)
    ));
    let counts = cost_counters::snapshot();
    assert_eq!(counts.snapshot_rebuilds,0,"I-25: patch new/remove/retitle signatures through stable page slots; exemplar model/persistent.rs: {counts:?}");
    assert!(
        counts.icon_page_probes <= 2,
        "I-25: patch affected icon names; exemplar model/page_icons.rs: {counts:?}"
    );
    assert_eq!(held.corpus().pages.len(), 128);
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 129);
    store.close();
}
