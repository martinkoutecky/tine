use tine_graph_features::journals;
use tine_store::{Depth, Store};

fn completed_feed(
    store: &Store,
    limit: usize,
    before: Option<i64>,
) -> std::io::Result<journals::FeedPage<journals::FeedRead>> {
    journals::feed_page_through(store, limit, before, None, true)
}

#[test]
fn gf4_feed_returns_before_ready() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    for directory in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(directory)).unwrap();
    }
    std::fs::write(root.path().join("journals/2000_01_01.md"), "- early body\n").unwrap();
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-03]] [[2099-01-01]]\n",
    )
    .unwrap();
    let pause = root.path().join(".tine-test-pause-load");
    std::fs::write(&pause, "").unwrap();
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    assert!(matches!(store.is_graph_ready(), Ok(false)));
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::sync::Arc::clone(&store);
    let worker =
        std::thread::spawn(move || send.send(journals::feed_page(&reader, 3, None)).unwrap());
    let early = receive.recv_timeout(std::time::Duration::from_secs(1));
    // Always release the worker, including against the pre-fix implementation.
    std::fs::remove_file(pause).unwrap();
    worker.join().unwrap();
    store.whole_graph().unwrap();
    let early = early.expect("journal body waited for graph Ready").unwrap();
    assert!(early.references_pending);
    assert_eq!(early.pages.len(), 1);
    assert!(early.pages[0].doc.blocks[0].raw.contains("early body"));
    let complete = completed_feed(&store, 3, None).unwrap();
    assert!(!complete.references_pending);
    assert_eq!(
        complete
            .pages
            .iter()
            .map(|p| p.doc.name.as_str())
            .collect::<Vec<_>>(),
        ["Jan 3rd, 2000", "Jan 1st, 2000"]
    );
    assert_eq!(complete.upcoming[0].doc.name, "Jan 1st, 2099");
    assert!(complete.pages[0].doc.rev.is_none());
    assert!(complete.pages[1].doc.rev.is_some());
    assert_eq!(
        std::fs::read_dir(root.path().join("journals"))
            .unwrap()
            .count(),
        1
    );
    store.close();
}

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
    store.whole_graph().unwrap(); // This case asserts the final referenced-day inventory.
    let first = completed_feed(&store, 1, None).unwrap();
    assert_eq!(
        first.pages.len(),
        1,
        "referenced fileless past day must appear"
    );
    assert_eq!(first.pages[0].doc.name, "Jan 3rd, 2000");
    assert!(first.pages[0].doc.blocks.is_empty());
    assert!(first.pages[0].doc.rev.is_none());
    let rest = completed_feed(&store, 5, first.next_before_day).unwrap();
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
    assert!(completed_feed(&store, 5, None).unwrap().pages.is_empty());
    store.close();
}

#[test]
fn future_references_are_upcoming_and_deleting_a_referenced_file_keeps_the_day() {
    let _integration_guard = crate::integration_lock();
    use crate::test_host_writer::HostWriter;
    use tine_store::{EditKind, SaveBase};
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
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    store.whole_graph().unwrap(); // Upcoming is a graph-wide referenced-day answer.
    let host = crate::test_host(&store);
    let first = completed_feed(&store, 1, None).unwrap();
    assert_eq!(first.pages[0].doc.name, "Jan 1st, 2000");
    assert_eq!(
        first
            .upcoming
            .iter()
            .map(|p| p.doc.name.as_str())
            .collect::<Vec<_>>(),
        ["Jan 1st, 2099", "Jan 3rd, 2099"]
    );
    assert!(completed_feed(&store, 5, Some(20000101))
        .unwrap()
        .upcoming
        .is_empty());
    journals::trash_journal_file(&store, &host, "2000_01_01.md").unwrap();
    let after = completed_feed(&store, 5, None).unwrap();
    assert_eq!(after.pages.len(), 1);
    assert!(after.pages[0].doc.rev.is_none());
    assert!(after.pages[0].doc.blocks.is_empty());
    assert!(!root.path().join("journals/2000_01_01.md").exists());
    // The usual CreateNew guard is sufficient for the virtual body's first edit.
    let mut body = after.pages[0].doc.clone();
    body.blocks.push(serde_json::from_value(serde_json::json!({"id":"new-body", "raw":"first edit", "children":[], "collapsed":false})).unwrap());
    HostWriter::for_store(&store)
        .save(
            EditKind::ReplacePage,
            &after.pages[0].id,
            SaveBase::CreateNew,
            &body,
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("journals/2000_01_01.md")).unwrap(),
        "- first edit\n"
    );
    store.close();
}

/// A query-only date is no journal row. A date-named pages file is its day's
/// journal (CM-DAYFILE, D-20261111-CMR14), shown once in the feed with its
/// references. (Superseded: before CM step 2 `pages/2000-01-01.md` was an
/// ordinary page and the feed was empty.)
#[test]
fn a_date_named_pages_file_is_a_journal_row_and_query_only_dates_are_not() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    for directory in ["pages", "journals"] {
        std::fs::create_dir(root.path().join(directory)).unwrap();
    }
    std::fs::write(root.path().join("pages/2000-01-01.md"), "- date page\n").unwrap();
    std::fs::write(
        root.path().join("pages/Plan.md"),
        "- [[2000-01-01]]\n- {{query [[2000-01-02]]}}\n",
    )
    .unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let feed = completed_feed(&store, 5, None).unwrap();
    assert_eq!(feed.pages.len(), 1);
    assert_eq!(feed.pages[0].doc.name, "Jan 1st, 2000");
    assert_eq!(feed.pages[0].doc.blocks[0].raw, "date page");
    let backlinks = store
        .whole_graph()
        .unwrap()
        .backlinks("Jan 1st, 2000")
        .unwrap();
    let blocks: usize = backlinks.iter().map(|row| row.blocks.len()).sum();
    assert_eq!(blocks, 1, "{backlinks:?}");
    assert!(!root.path().join("journals/2000_01_01.md").exists());
    store.close();
}

/// REG-CM-YEARLESS-DAY-KEY (round 3): under a year-less title format the
/// feed names each day by the spelling that identifies it and titles it by
/// its label, for a day file (Markdown and Org) and a referenced day with no
/// file alike, so two years of `11-02` are two feed days the app keys apart.
/// Before the fix both read back as `11-02`.
#[test]
fn a_year_less_feed_names_each_day_by_the_spelling_that_identifies_it() {
    let _integration_guard = crate::integration_lock();
    for ext in ["md", "org"] {
        let root = tempfile::tempdir().unwrap();
        for directory in ["pages", "journals", "logseq"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
        }
        let file = |rel: &str, text: &str| std::fs::write(root.path().join(rel), text).unwrap();
        file(
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        );
        file(&format!("journals/2026_11_02.{ext}"), "- the day\n");
        file(&format!("journals/2025_11_02.{ext}"), "- last year\n");
        file("pages/11-02.md", "- a label page\n");
        file("pages/Plan.md", "- [[2024-11-02]]\n");
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.whole_graph().unwrap();
        let feed = completed_feed(&store, 10, None).unwrap();
        let names: Vec<_> = (feed.pages.iter())
            .map(|p| (p.doc.name.as_str(), p.doc.title.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("2026_11_02", "11-02"),
                ("2025_11_02", "11-02"),
                ("2024_11_02", "11-02")
            ],
            "{ext}"
        );
        store.close();
    }
}
