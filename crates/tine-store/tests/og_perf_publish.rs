//! Cost-model audit 2026-11-02 rows 3 and 4 (D-10): creating a page, the
//! first save of a day's journal and a favourite toggle cost the names they
//! change, not the graph. Before, each one listed the graph again and opened
//! every page file for its title (a create or a journal create dropped the
//! name memos; any config.edn byte change rebuilt the name index).
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::test_host_writer::HostWriter;
use tine_core::date::JournalDate;
use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_store::{cost_counters, EditKind, PageId, Resolved, SaveBase, Store};

const PAGES: usize = 40;

fn graph() -> (PathBuf, Arc<Store>) {
    let root = crate::test_support::fresh_dir("tine-og-perf-publish-");
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), "{}\n").unwrap();
    let mut files: Vec<(String, String)> = (0..PAGES)
        .map(|i| {
            (
                format!("pages/P{i}.md"),
                format!("title:: Name {i}\n\n- body {i}\n"),
            )
        })
        .collect();
    files.push(("journals/2026_01_15.md".into(), "- an old day\n".into()));
    files.push(("journals/Jan 15th, 2026.md".into(), "- its twin\n".into()));
    for (rel, text) in files {
        let path = root.join(rel);
        fs::write(&path, text).unwrap();
        // Outside the racy window (§5.4), so no settling re-read is counted.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (root, store)
}

fn doc(name: &str, kind: PageKind, title: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind,
        title: title.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "b".into(),
            raw: "new".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    }
}

/// The user's path: resolve the name, then save the proposed id as CreateNew.
fn create_page(store: &Store, writer: &HostWriter, name: &str) {
    let id = match store.whole_graph().unwrap().resolve(name, false) {
        Resolved::Absent { id } => id,
        _ => panic!("{name} should resolve as absent"),
    };
    writer
        .save(
            EditKind::CreatePage,
            &id,
            SaveBase::CreateNew,
            &doc(name, PageKind::Page, name),
        )
        .unwrap();
}

/// A day's first save: its journal file is created.
fn create_journal(writer: &HostWriter, stem: &str, title: &str) -> PageId {
    let id = PageId::from(format!("journals/{stem}.md"));
    writer
        .save(
            EditKind::CreatePage,
            &id,
            SaveBase::CreateNew,
            &doc(stem, PageKind::Journal, title),
        )
        .unwrap();
    id
}

/// Work since the last reset: page files opened for a title, and
/// directories listed (a name walk lists every page directory). A created
/// page's own file is read once for its title; any other is graph-wide work.
fn names_read(label: &str) -> (u64, u64) {
    let counts = cost_counters::snapshot();
    eprintln!(
        "{label}: preamble reads={}, readdir={}, full reads={}, parses={}",
        counts.preamble_reads, counts.readdir, counts.full_reads, counts.parses
    );
    (counts.preamble_reads, counts.readdir)
}

#[test]
fn creates_and_a_favourite_toggle_list_and_open_no_other_page() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (root, store) = graph();
    let writer = HostWriter::start(&store);
    // The first lookup of an absent name of each kind after launch still
    // lists once: the launch index stays unvouched for names it lacks
    // (`snapshot_name_index`).
    create_page(&store, &writer, "Warm");
    create_journal(&writer, "2026_01_16", "Jan 16th, 2026");

    cost_counters::reset();
    create_page(&store, &writer, "Fresh");
    assert_eq!(
        names_read("page create"),
        (1, 0),
        "a page create lists and names only the page it creates"
    );

    cost_counters::reset();
    let today = JournalDate::today();
    let id = create_journal(&writer, &today.file_stem(), &today.title());
    assert_eq!(
        names_read("journal create"),
        (0, 0),
        "the first save of a day's journal lists and names only that journal"
    );

    cost_counters::reset();
    tine_graph_features::config::set_favorites(&store, &["Fresh".into()], None).unwrap();
    assert_eq!(
        names_read("favourite toggle"),
        (0, 0),
        "a favourite toggle lists and names no page"
    );
    assert_eq!(store.config().favorites, ["Fresh"]);

    // Same answers as a fresh listing: the created page and journal resolve,
    // the duplicate day keeps its date-stem winner, every title still names.
    let view = store.whole_graph().unwrap();
    let existing = |name: &str, journal: bool| match view.resolve(name, journal) {
        Resolved::Existing { id, others } => (id.as_str().to_owned(), others.len()),
        _ => panic!("{name} should resolve to its page"),
    };
    assert_eq!(existing("Fresh", false), ("pages/Fresh.md".into(), 0));
    assert_eq!(existing("Name 7", false), ("pages/P7.md".into(), 0));
    assert_eq!(existing(&today.title(), true), (id.as_str().to_owned(), 0));
    assert_eq!(
        existing("Jan 15th, 2026", true),
        ("journals/2026_01_15.md".into(), 1)
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

/// The watcher's path: an outside favourite edit (a sync delivery of another
/// device's star) lists no directory and names no page; a build-relevant one
/// (`:hidden`) still rescans and drops the hidden page.
#[test]
fn an_outside_favourite_edit_rescans_nothing_but_a_hidden_edit_does() {
    let _integration_guard = crate::exclusive_integration_lock();
    let (root, store) = graph();
    let subscription = store.subscribe();
    // Let the watcher install its OS subscription before the outside write.
    std::thread::sleep(Duration::from_millis(200));
    let deliver = |text: &str| {
        let temp = root.join("logseq/.delivery");
        fs::write(&temp, text).unwrap();
        fs::rename(temp, root.join("logseq/config.edn")).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(change) = subscription.try_recv().unwrap() {
                if change.origin == tine_store::Origin::External
                    && (change.files.iter()).any(|(id, _, _)| id.as_str() == "logseq/config.edn")
                {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("no External publication of logseq/config.edn");
    };

    cost_counters::reset();
    deliver("{:favorites [\"Name 1\"]}\n");
    assert_eq!(store.config().favorites, ["Name 1"]);
    assert_eq!(
        names_read("outside favourite"),
        (0, 0),
        "an outside favourite edit takes no full scan and names no page"
    );

    cost_counters::reset();
    deliver("{:hidden [\"pages/P0.md\"]}\n");
    assert!(
        cost_counters::snapshot().readdir > 0,
        "a :hidden edit rescans"
    );
    let view = store.whole_graph().unwrap();
    assert!(matches!(
        view.resolve("Name 0", false),
        Resolved::Absent { .. }
    ));
    assert!(matches!(
        view.resolve("Name 1", false),
        Resolved::Existing { .. }
    ));
    store.close();
    let _ = fs::remove_dir_all(root);
}
