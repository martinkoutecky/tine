use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs as disk, sync::Arc};

#[test]
fn rename_plan_keeps_its_view_during_concurrent_referrer_write() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-rename-plan-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- source\n").unwrap();
    disk::write(root.join("pages/Referrer.md"), "- [[Old]] before\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let written = AtomicBool::new(false);
    rename_page_after_inventory(&store, "Old", "New", None, || {
        if written.swap(true, Ordering::AcqRel) {
            return;
        }
        let writer = Arc::clone(&store);
        std::thread::spawn(move || {
            let id = PageId::from("pages/Referrer.md");
            let read = writer.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "[[Old]] concurrent".into();
            assert!(matches!(
                writer.save(&id, SaveBase::Existing(read.rev), &doc),
                SaveOutcome::Saved(_)
            ));
        })
        .join()
        .unwrap();
    })
    .unwrap();
    assert!(root.join("pages/New.md").is_file());
    assert!(!root.join("pages/Old.md").exists());
    assert!(disk::read_to_string(root.join("pages/Referrer.md"))
        .unwrap()
        .contains("[[New]] concurrent"));
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn delete_detects_an_external_twin_before_selecting_a_file() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-delete-external-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- markdown\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let _ = store.whole_graph().unwrap();
    disk::write(root.join("pages/Old.org"), "* org\n").unwrap();

    assert!(
        delete_page_expected(&store, "Old", tine_core::model::PageKind::Page, None, None).is_err()
    );
    assert!(root.join("pages/Old.md").exists());
    assert!(root.join("pages/Old.org").exists());
    store.close();
    disk::remove_dir_all(root).unwrap();
}
