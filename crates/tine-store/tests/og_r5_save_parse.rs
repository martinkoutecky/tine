//! I-15: a host save plans with one old-source parse and one splice check, then
//! publishes by reparsing the written bytes.
use std::fs;
use tine_store::{cost_counters, EditKind, PageId, SaveBase, Store};

#[test]
fn ordinary_host_save_plans_two_parses_and_publishes_the_written_tree() {
    let _integration_guard = crate::exclusive_integration_lock();
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(
            dir.path().join("pages/Page.md"),
            "- before\n".repeat(blocks),
        )
        .unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let store = std::sync::Arc::new(store);
        let host_writer = crate::test_host_writer::HostWriter::start(&store);
        let held = store.whole_graph().unwrap();
        let id = PageId::from("pages/Page.md");
        let read = store.page(&id).unwrap();
        let mut page = read.doc;
        page.blocks[0].raw = "after".into();
        cost_counters::reset();
        let kind = EditKind::ReplacePage;
        let mut plan = store.page_plan(Some(kind));
        plan.save_page(&[kind], &id, SaveBase::Existing(read.rev), &page);
        let op = plan.plan().unwrap();
        let planned = cost_counters::snapshot();
        host_writer.apply(op).unwrap();
        let cost = cost_counters::snapshot();
        eprintln!("R5 blocks={blocks}: {cost:?}");
        assert_eq!(
            planned.parses, 2,
            "I-15: one old-source parse and one splice check"
        );
        assert_eq!(
            cost.parses, 3,
            "I-15: the host adds one reparse of the bytes it publishes"
        );
        assert_eq!(store.page(&id).unwrap().doc.blocks[0].raw, "after");
        assert_eq!(
            held.corpus(None, false).pages[0].document.roots[0].raw(),
            "before"
        );
        store.close();
    }
}
