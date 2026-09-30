//! I-15: keep the splice reparse, share its result with publication.
use std::fs;
use tine_store::{cost_counters, EditKind, PageId, SaveBase, SaveOutcome, Store};

#[test]
fn ordinary_save_parses_old_and_output_once_and_publishes_the_written_tree() {
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(
            dir.path().join("pages/Page.md"),
            "- before\n".repeat(blocks),
        )
        .unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let held = store.whole_graph().unwrap();
        let id = PageId::from("pages/Page.md");
        let read = store.page(&id).unwrap();
        let mut page = read.doc;
        page.blocks[0].raw = "after".into();
        cost_counters::reset();
        assert!(matches!(
            store.save(
                EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &page
            ),
            SaveOutcome::Saved(_)
        ));
        let cost = cost_counters::snapshot();
        eprintln!("R5 blocks={blocks}: {cost:?}");
        assert_eq!(cost.parses, 2, "I-15: one old-source parse and one splice check, shared with publication; exemplar model.rs transaction_publish_page");
        assert_eq!(store.page(&id).unwrap().doc.blocks[0].raw, "after");
        assert_eq!(held.corpus().pages[0].document.roots[0].raw(), "before");
        store.close();
    }
}
