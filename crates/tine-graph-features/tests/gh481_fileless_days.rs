use tine_graph_features::journals;
use tine_store::{Depth, Store};

#[test]
fn referenced_fileless_past_days_are_paged_without_writing_files() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("pages")).unwrap();
    std::fs::create_dir(root.path().join("journals")).unwrap();
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-03]] and [[Jan 3rd, 2000]]\n- [[2000-01-01]]\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let first = journals::feed_page(&store, 1, None).unwrap();
    assert_eq!(
        first.pages.len(),
        1,
        "referenced fileless past day must appear"
    );
    assert_eq!(first.pages[0].doc.name, "Jan 3rd, 2000");
    assert!(first.pages[0].doc.blocks.is_empty());
    assert!(first.pages[0].doc.rev.is_none());
    let rest = journals::feed_page(&store, 5, first.next_before_day).unwrap();
    assert_eq!(
        rest.pages
            .iter()
            .map(|p| p.doc.name.as_str())
            .collect::<Vec<_>>(),
        ["Jan 1st, 2000"]
    );
    assert!(rest.done);
    assert_eq!(
        store
            .whole_graph()
            .unwrap()
            .backlinks("Jan 3rd, 2000")
            .unwrap()[0]
            .page,
        "Plan"
    );
    assert_eq!(
        std::fs::read_dir(root.path().join("journals"))
            .unwrap()
            .count(),
        0
    );
    // Removing the last references removes the virtual rows in a fresh publication.
    std::fs::write(root.path().join("pages/Plan.md"), "- no dates\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert!(journals::feed_page(&store, 5, None)
        .unwrap()
        .pages
        .is_empty());
    store.close();
}

#[test]
fn future_references_are_upcoming_and_deleting_a_referenced_file_keeps_the_day() {
    let _integration_guard = crate::integration_lock();
    use tine_store::{EditKind, SaveBase, SaveOutcome};
    let root = tempfile::tempdir().unwrap();
    for directory in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(directory)).unwrap();
    }
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2099-01-03]] [[2099-01-01]] [[2000-01-01]]\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("journals/2000_01_01.md"),
        "- existing day\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("journals/2099_01_03.md"),
        "alias:: 2099-01-03\n- future file\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let first = journals::feed_page(&store, 1, None).unwrap();
    assert_eq!(first.pages[0].doc.name, "Jan 1st, 2000");
    assert_eq!(
        first
            .upcoming
            .iter()
            .map(|p| p.doc.name.as_str())
            .collect::<Vec<_>>(),
        ["Jan 1st, 2099", "Jan 3rd, 2099"]
    );
    assert!(journals::feed_page(&store, 5, Some(20000101))
        .unwrap()
        .upcoming
        .is_empty());
    journals::trash_journal_file(&store, None, "2000_01_01.md").unwrap();
    let after = journals::feed_page(&store, 5, None).unwrap();
    assert_eq!(after.pages.len(), 1);
    assert!(after.pages[0].doc.rev.is_none());
    assert!(after.pages[0].doc.blocks.is_empty());
    assert!(!root.path().join("journals/2000_01_01.md").exists());
    // The usual CreateNew guard is sufficient for the virtual body's first edit.
    let mut body = after.pages[0].doc.clone();
    body.blocks.push(serde_json::from_value(serde_json::json!({"id":"new-body", "raw":"first edit", "children":[], "collapsed":false})).unwrap());
    assert!(matches!(
        store.save(
            EditKind::ReplacePage,
            &after.pages[0].id,
            SaveBase::CreateNew,
            &body
        ),
        SaveOutcome::Saved(_)
    ));
    assert_eq!(
        std::fs::read_to_string(root.path().join("journals/2000_01_01.md")).unwrap(),
        "- first edit\n"
    );
    store.close();
}

#[test]
fn ordinary_date_pages_and_query_only_dates_do_not_become_journal_rows() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    for directory in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(directory)).unwrap();
    }
    std::fs::write(
        root.path().join("pages/2000-01-01.md"),
        "- ordinary date page\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-01]]\n- {{query [[2000-01-02]]}}\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    assert!(journals::feed_page(&store, 5, None)
        .unwrap()
        .pages
        .is_empty());
    assert!(store
        .whole_graph()
        .unwrap()
        .backlinks("Jan 1st, 2000")
        .unwrap()
        .is_empty());
    store.close();
}
