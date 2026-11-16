//! CM step 2 day files (D-20261110-CM CM-DATE / CM-ORDER, D-20261111-CMR14
//! CM-DAYFILE; conceptual model K, SPEC §§2.1–2.2): a file whose effective
//! name is a date is that day's journal wherever it lies, the journals
//! directory ranks first among a day's files, and nothing is moved,
//! rewritten or merged on discovery. Each case drives the store's public
//! entry points: opening by name, the day index, links and the conflict
//! inventory.

use std::fs;
use std::path::Path;

use tine_core::model::PageKind;
use tine_store::{Day, InventoryScope, PageId, Resolved, Store};

const NOV_2: Day = Day(20261102);

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("journals")).unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    (dir, store)
}

/// Every graph file's bytes, so a case can show nothing was written.
fn bytes(root: &Path, files: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
    (files.iter())
        .map(|(rel, _)| (rel.to_string(), fs::read(root.join(rel)).unwrap()))
        .collect()
}

fn existing(target: &Resolved) -> Vec<String> {
    match target {
        Resolved::Existing { id, others } => std::iter::once(id)
            .chain(others)
            .map(|id| id.as_str().to_owned())
            .collect(),
        other => panic!("expected an existing page, got {other:?}"),
    }
}

/// The file `name` opens through `page_named`, with the kind it reads as.
fn opens(store: &Store, name: &str, kind: PageKind) -> Option<(String, PageKind)> {
    (store.page_named(name, kind).unwrap()).map(|read| (read.id.as_str().to_owned(), read.doc.kind))
}

#[test]
fn a_date_named_pages_file_alone_is_the_day_and_stays_where_it_is() {
    let _guard = crate::integration_lock();
    let files = [
        ("pages/2026-11-02.md", "- the day's notes\n"),
        (
            "pages/Notes.md",
            "- see [[2026-11-02]] and [[Nov 2nd, 2026]]\n",
        ),
    ];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let view = store.whole_graph().unwrap();
    let day = "pages/2026-11-02.md".to_owned();
    // Every accepted spelling, typed as a page or as a journal, is the day.
    for (name, is_journal) in [
        ("2026-11-02", false),
        ("2026-11-02", true),
        ("Nov 2nd, 2026", true),
        ("Nov 2nd, 2026", false),
    ] {
        assert_eq!(
            existing(&view.resolve(name, is_journal)),
            [day.clone()],
            "{name}"
        );
    }
    // The day index and opening by name use the file that holds the day.
    assert_eq!(store.journal_id(NOV_2, true).as_str(), day);
    assert_eq!(store.journal_id(NOV_2, false).as_str(), day);
    assert_eq!(
        opens(&store, "Nov 2nd, 2026", PageKind::Journal),
        Some((day.clone(), PageKind::Journal))
    );
    // It is a journal, named by its day's title; no ordinary `2026-11-02`.
    let row = view.inventory(InventoryScope::File(&PageId::from(day.as_str())));
    assert!(
        (row.0.iter()).all(|entry| entry.is_journal && entry.name == "Nov 2nd, 2026"),
        "{:?}",
        row.0
            .iter()
            .map(|e| (&e.name, e.is_journal))
            .collect::<Vec<_>>()
    );
    // Both links in Notes reach the day's journal.
    for name in ["2026-11-02", "Nov 2nd, 2026"] {
        let link = view.link_target(name);
        assert!(link.is_journal, "{name}: {link:?}");
        assert_eq!(link.name, "Nov 2nd, 2026", "{name}");
    }
    let refs = view.backlinks("Nov 2nd, 2026").unwrap();
    assert_eq!(
        refs.iter()
            .map(|group| group.page.as_str())
            .collect::<Vec<_>>(),
        ["Notes"]
    );
    assert_eq!(bytes(dir.path(), &files), before, "nothing is written");
}

#[test]
fn with_both_files_the_journals_one_opens_and_the_other_is_a_visible_conflict() {
    let _guard = crate::integration_lock();
    let files = [
        ("journals/2026_11_02.md", "- journal side\n"),
        (
            "pages/2026-11-02.md",
            "alias:: Loser Alias\n\n- page side [[Only From The Loser]]\n",
        ),
        ("pages/Notes.md", "- see [[2026-11-02]]\n"),
    ];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let view = store.whole_graph().unwrap();
    let both = ["journals/2026_11_02.md", "pages/2026-11-02.md"];
    for (name, is_journal) in [("2026-11-02", false), ("Nov 2nd, 2026", true)] {
        assert_eq!(existing(&view.resolve(name, is_journal)), both, "{name}");
    }
    assert_eq!(store.journal_id(NOV_2, true).as_str(), both[0]);
    assert_eq!(store.journal_id(NOV_2, false).as_str(), both[0]);
    assert_eq!(
        opens(&store, "2026-11-02", PageKind::Page),
        Some((both[0].to_owned(), PageKind::Journal))
    );
    // The losing file is listed as a conflict with the file that opens.
    let conflicts = view.inventory(InventoryScope::ConflictingFiles(None));
    let mut listed: Vec<_> = (conflicts.0.iter())
        .filter_map(|entry| match &entry.target {
            Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
            _ => None,
        })
        .collect();
    listed.sort();
    assert_eq!(listed, both, "the same-day twins form one conflict");
    // Only the opening file contributes: the loser's alias names nothing and
    // its reference counts for no page.
    assert!(
        matches!(view.resolve("Loser Alias", false), Resolved::Absent { .. }),
        "{:?}",
        view.resolve("Loser Alias", false)
    );
    assert!(view.backlinks("Only From The Loser").unwrap().is_empty());
    assert_eq!(
        (view.backlinks("Nov 2nd, 2026").unwrap().iter())
            .map(|group| group.page.clone())
            .collect::<Vec<_>>(),
        ["Notes"]
    );
    assert_eq!(
        bytes(dir.path(), &files),
        before,
        "neither file is rewritten"
    );
}

/// Before the first view exists, opening a day reads the file listing: the
/// same day files, in the same order, as the ready view.
#[test]
fn before_the_first_view_a_day_opens_the_same_file() {
    let _guard = crate::integration_lock();
    for journal in [false, true] {
        let mut files = vec![("pages/2026-11-02.md", "- page side\n")];
        if journal {
            files.push(("journals/2026_11_02.md", "- journal side\n"));
        }
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("journals")).unwrap();
        for (rel, text) in &files {
            fs::write(dir.path().join(rel), text).unwrap();
        }
        let pause = dir.path().join(".tine-test-pause-load");
        fs::write(&pause, "").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let expected = if journal {
            "journals/2026_11_02.md"
        } else {
            "pages/2026-11-02.md"
        };
        let early = (
            opens(&store, "Nov 2nd, 2026", PageKind::Journal),
            opens(&store, "2026-11-02", PageKind::Page),
            store.journal_id(NOV_2, true).as_str().to_owned(),
            store.journal_id(NOV_2, false).as_str().to_owned(),
        );
        fs::remove_file(pause).unwrap();
        let day = Some((expected.to_owned(), PageKind::Journal));
        assert_eq!(
            early,
            (day.clone(), day, expected.to_owned(), expected.to_owned()),
            "journal file present: {journal}"
        );
        let view = store.whole_graph().unwrap();
        assert_eq!(
            existing(&view.resolve("Nov 2nd, 2026", true))[0],
            expected,
            "journal file present: {journal}"
        );
    }
}

#[test]
fn a_title_that_is_a_date_makes_the_file_that_day() {
    let _guard = crate::integration_lock();
    let files = [("pages/meeting.md", "title:: 2026-11-02\n\n- minutes\n")];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let view = store.whole_graph().unwrap();
    assert_eq!(
        existing(&view.resolve("Nov 2nd, 2026", true)),
        ["pages/meeting.md"]
    );
    assert_eq!(
        existing(&view.resolve("2026-11-02", false)),
        ["pages/meeting.md"]
    );
    assert_eq!(store.journal_id(NOV_2, true).as_str(), "pages/meeting.md");
    assert!(matches!(
        view.resolve("meeting", false),
        Resolved::Absent { .. }
    ));
    assert_eq!(bytes(dir.path(), &files), before);
}

#[test]
fn a_date_named_org_file_is_the_day() {
    let _guard = crate::integration_lock();
    let files = [("pages/2026-11-02.org", "* org day\n")];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let view = store.whole_graph().unwrap();
    assert_eq!(
        existing(&view.resolve("Nov 2nd, 2026", true)),
        ["pages/2026-11-02.org"]
    );
    assert_eq!(
        opens(&store, "Nov 2nd, 2026", PageKind::Journal),
        Some(("pages/2026-11-02.org".to_owned(), PageKind::Journal))
    );
    assert_eq!(bytes(dir.path(), &files), before);
}

/// The approved label limitation (SPEC §2.1): with a title format without a
/// year, a label like `11-02` is no date, so a page of that name stays an
/// ordinary page beside the day it looks like.
#[test]
fn a_year_less_title_format_leaves_a_label_named_page_ordinary() {
    let _guard = crate::integration_lock();
    let files = [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        ("pages/11-02.md", "- a label page\n"),
        ("journals/2026_11_02.md", "- the day\n"),
    ];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let view = store.whole_graph().unwrap();
    let row = view.inventory(InventoryScope::File(&PageId::from("pages/11-02.md")));
    assert!(
        (row.0.iter()).any(|entry| !entry.is_journal && entry.name == "11-02"),
        "{:?}",
        row.0
            .iter()
            .map(|e| (&e.name, e.is_journal))
            .collect::<Vec<_>>()
    );
    // Pinned as it is: the label page opens, and the journal whose displayed
    // title is the same label is listed as a second physical file of that
    // name (K says the label selects no day; see the receipt's question).
    assert_eq!(
        existing(&view.resolve("11-02", false)),
        ["pages/11-02.md", "journals/2026_11_02.md"]
    );
    // The day itself is still reached by a spelling that names it.
    assert_eq!(
        existing(&view.resolve("2026-11-02", false)),
        ["journals/2026_11_02.md"]
    );
    assert_eq!(
        store.journal_id(NOV_2, true).as_str(),
        "journals/2026_11_02.md"
    );
    assert_eq!(bytes(dir.path(), &files), before);
}

#[test]
fn a_day_with_no_file_is_created_in_the_journals_directory() {
    let _guard = crate::integration_lock();
    let files = [("pages/Notes.md", "- see [[2026-11-02]]\n")];
    let (_dir, store) = graph(&files);
    let view = store.whole_graph().unwrap();
    for (name, is_journal) in [("2026-11-02", false), ("Nov 2nd, 2026", true)] {
        match view.resolve(name, is_journal) {
            Resolved::Absent { id } => assert_eq!(id.as_str(), "journals/2026_11_02.md", "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(
        store.journal_id(NOV_2, true).as_str(),
        "journals/2026_11_02.md"
    );
}
