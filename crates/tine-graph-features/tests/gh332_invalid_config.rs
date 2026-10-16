use std::fs;
use tine_store::{Area, Store};

#[test]
fn malformed_graph_view_config_keeps_pages_readable_at_graph_open() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir(root.path().join(dir)).unwrap();
    }
    let config = "{:graph-view {:layout : grid}}\n";
    let page = "- existing page\n";
    let journal = "- existing journal\n";
    fs::write(root.path().join("logseq/config.edn"), config).unwrap();
    fs::write(root.path().join("pages/Existing.md"), page).unwrap();
    fs::write(root.path().join("journals/2026_09_01.md"), journal).unwrap();
    let (store, _, _) = Store::open(root.path(), Default::default()).unwrap();
    let view = store.whole_graph().unwrap();
    assert_eq!(
        view.corpus().pages.len(),
        2,
        "invalid config must never erase the inventory"
    );
    for (area, name) in [
        (Area::Pages, "Existing.md"),
        (Area::Journals, "2026_09_01.md"),
    ] {
        let id = store.file_id(area, name).unwrap();
        let read = store.page(&id.as_str().into()).unwrap();
        assert!(!read.doc.blocks.is_empty(), "{name} remains readable");
    }
    assert_eq!(
        fs::read_to_string(root.path().join("logseq/config.edn")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(root.path().join("pages/Existing.md")).unwrap(),
        page
    );
    assert_eq!(
        fs::read_to_string(root.path().join("journals/2026_09_01.md")).unwrap(),
        journal
    );
    // Live redelivery of the same shape keeps the inventory too.
    store.refresh(tine_store::Depth::Stamps).unwrap();
    assert_eq!(store.whole_graph().unwrap().corpus().pages.len(), 2);
    store.close();
}
