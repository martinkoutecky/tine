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

/// What the store answers about `days` and `names`: the day index with and
/// without the active lookup, the indexed days, and every name's resolve as
/// a page and as a journal.
fn day_answers(store: &Store, days: &[Day], names: &[&str]) -> Vec<String> {
    let view = store.whole_graph().unwrap();
    let mut indexed = store.journal_days();
    indexed.sort();
    let mut out = vec![format!("indexed days {indexed:?}")];
    for day in days {
        out.push(format!(
            "{day:?}: active {}, index {}",
            store.journal_id(*day, true).as_str(),
            store.journal_id(*day, false).as_str()
        ));
    }
    for name in names {
        for is_journal in [false, true] {
            out.push(format!(
                "{name} ({is_journal}): {:?}",
                view.resolve(name, is_journal)
            ));
        }
    }
    out
}

/// The same answers from a cold open of a copy of `rels` under `root`.
fn cold_answers(root: &Path, rels: &[&str], days: &[Day], names: &[&str]) -> Vec<String> {
    let files: Vec<(&str, String)> = (rels.iter())
        .filter(|rel| root.join(rel).exists())
        .map(|rel| (*rel, fs::read_to_string(root.join(rel)).unwrap()))
        .collect();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, text)| (*rel, text.as_str()))
        .collect();
    let (_dir, store) = graph(&files);
    day_answers(&store, days, names)
}

/// Re-scan until `done` holds (an outside edit may publish in parts).
fn settle(store: &Store, what: &str, done: impl Fn(&Store) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        store.refresh(tine_store::Depth::Stamps).unwrap();
        if done(store) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: never converged"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A pages file named by one date and titled by another holds the title's
/// day (SPEC §2.1). Retitled outside the app to a third date, the
/// incremental update moves it off the day it held onto the new one, as a
/// cold open reads it: the old day has no file, so a write to it goes to a
/// new journals file, never to this one (CM-DAYFILE).
#[test]
fn an_outside_retitle_moves_a_file_off_the_day_it_held() {
    let _guard = crate::integration_lock();
    let file = "pages/2026-11-02.md";
    let (dir, store) = graph(&[(file, "title:: 2026-11-03\n\n- notes\n")]);
    let (nov_3, nov_4) = (Day(20261103), Day(20261104));
    // Ready first, so the retitle is an incremental update, not the load.
    store.whole_graph().unwrap();
    assert_eq!(store.journal_id(nov_3, false).as_str(), file);
    fs::write(
        dir.path().join(file),
        "title:: 2026-11-04\n\n- notes, retitled\n",
    )
    .unwrap();
    settle(&store, "outside retitle", |store| {
        let view = store.whole_graph().unwrap();
        matches!(
            view.resolve("Nov 4th, 2026", true),
            Resolved::Existing { .. }
        )
    });
    let (days, names) = (
        [NOV_2, nov_3, nov_4],
        ["Nov 2nd, 2026", "Nov 3rd, 2026", "Nov 4th, 2026"],
    );
    let incremental = day_answers(&store, &days, &names);
    assert_eq!(
        incremental,
        cold_answers(dir.path(), &[file], &days, &names),
        "incremental after the retitle equals a cold open"
    );
    assert_eq!(
        store.journal_id(nov_3, false).as_str(),
        "journals/2026_11_03.md"
    );
    assert_eq!(
        store.journal_id(nov_3, true).as_str(),
        "journals/2026_11_03.md"
    );
    assert_eq!(store.journal_id(nov_4, false).as_str(), file);
}

/// With a year-less title format two pages files titled by one full date
/// share that day; the winner deleted outside the app, the other takes the
/// day as a cold open gives it, though the remembered label `11-02` names
/// no day (the day is the entry's, never parsed back from its label).
#[test]
fn deleting_a_year_less_day_winner_promotes_its_twin() {
    let _guard = crate::integration_lock();
    let config = "{:journal/page-title-format \"MM-dd\"}\n";
    let rels = ["logseq/config.edn", "pages/A.md", "pages/B.md"];
    let (dir, store) = graph(&[
        (rels[0], config),
        (rels[1], "title:: 2026-11-02\n\n- a\n"),
        (rels[2], "title:: 2026-11-02\n\n- b\n"),
    ]);
    store.whole_graph().unwrap();
    assert_eq!(store.journal_id(NOV_2, false).as_str(), "pages/A.md");
    fs::remove_file(dir.path().join(rels[1])).unwrap();
    settle(&store, "outside delete", |store| {
        let view = store.whole_graph().unwrap();
        matches!(view.resolve("2026-11-02", true), Resolved::Existing { id, others }
            if id.as_str() == "pages/B.md" && others.is_empty())
    });
    let (days, names) = ([NOV_2], ["2026-11-02"]);
    assert_eq!(
        day_answers(&store, &days, &names),
        cold_answers(dir.path(), &rels, &days, &names),
        "incremental after the delete equals a cold open"
    );
    assert_eq!(store.journal_id(NOV_2, false).as_str(), "pages/B.md");
    assert!(!dir.path().join(rels[1]).exists(), "nothing recreates A");
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
