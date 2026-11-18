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
/// ordinary page beside the day it looks like, in either directory.
#[test]
fn a_year_less_title_format_leaves_a_label_named_page_ordinary() {
    let _guard = crate::integration_lock();
    for day_file in ["journals/2026_11_02.md", "pages/2026-11-02.md"] {
        year_less_label_page(day_file);
    }
}

fn year_less_label_page(day_file: &str) {
    let files = [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        ("pages/11-02.md", "- a label page\n"),
        (day_file, "- the day\n"),
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
    // Typed, the label is the ordinary page alone: "typing the ambiguous,
    // non-identifying label cannot select" the day (SPEC §2.1), so the day's
    // file is no second claimant of it (re-pinned: it was listed second).
    assert_eq!(existing(&view.resolve("11-02", false)), ["pages/11-02.md"]);
    assert_eq!(
        opens(&store, "11-02", PageKind::Page),
        Some(("pages/11-02.md".to_owned(), PageKind::Page))
    );
    // The day itself is still reached by a spelling that names it.
    assert_eq!(existing(&view.resolve("2026-11-02", false)), [day_file]);
    assert_eq!(store.journal_id(NOV_2, true).as_str(), day_file);
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

/// Re-scan until the incremental answers equal a cold open's, or fail with
/// both (an outside change may publish in parts; a wrong incremental answer
/// never converges).
fn settle_to_cold(store: &Store, root: &Path, rels: &[&str], days: &[Day], names: &[&str]) {
    let cold = cold_answers(root, rels, days, names);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let incremental = day_answers(store, days, names);
        if incremental == cold || std::time::Instant::now() >= deadline {
            assert_eq!(incremental, cold, "incremental equals a cold open");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// CM leftovers item 6 (SPEC §2.1, approved label limitation): with a
/// year-less title format, 2026-11-02 and 2027-11-02 both display `11-02`
/// and stay distinct days. An outside retitle from one to the other keeps
/// the displayed name and kind, yet moves the file to the other day; a
/// second file titled with the other year adds that day beside the first.
/// The incremental reading answers as a cold open does.
#[test]
fn a_year_less_retitle_to_another_year_moves_the_day() {
    let _guard = crate::integration_lock();
    let config = "{:journal/page-title-format \"MM-dd\"}\n";
    let nov_2_2027 = Day(20271102);
    let rels = ["logseq/config.edn", "pages/A.md", "pages/B.md"];
    let (days, names) = ([NOV_2, nov_2_2027], ["2026-11-02", "2027-11-02"]);
    // One file, retitled to the other year.
    let (dir, store) = graph(&[(rels[0], config), (rels[1], "title:: 2026-11-02\n\n- a\n")]);
    store.whole_graph().unwrap();
    assert_eq!(store.journal_id(NOV_2, false).as_str(), rels[1]);
    fs::write(
        dir.path().join(rels[1]),
        "title:: 2027-11-02\n\n- a, retitled\n",
    )
    .unwrap();
    settle_to_cold(&store, dir.path(), &rels, &days, &names);
    assert_eq!(store.journal_id(nov_2_2027, false).as_str(), rels[1]);
    // A second file whose title is the same label of another year.
    let (dir, store) = graph(&[(rels[0], config), (rels[1], "title:: 2026-11-02\n\n- a\n")]);
    store.whole_graph().unwrap();
    fs::write(dir.path().join(rels[2]), "title:: 2027-11-02\n\n- b\n").unwrap();
    settle_to_cold(&store, dir.path(), &rels, &days, &names);
    assert_eq!(store.journal_id(NOV_2, false).as_str(), rels[1]);
    assert_eq!(store.journal_id(nov_2_2027, false).as_str(), rels[2]);
}

/// Every block a page-reference query or a Linked References panel lists,
/// as raw text, sorted (`groups` name the source page only by its label).
fn raws(groups: &[tine_core::model::RefGroup]) -> Vec<String> {
    let mut out: Vec<String> = (groups.iter())
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
        .collect();
    out.sort();
    out
}

/// The block rows of the OG query `[[name]]`.
fn ref_query(store: &Store, name: &str) -> Vec<String> {
    use tine_core::query::ir::{ExecutionContext, QueryRows};
    let (query, view) = tine_core::query::parse_query_text(
        &format!("[[{name}]]"),
        tine_core::query::QueryDialect::Og,
        tine_core::date::JournalDate::today(),
    );
    let answer = (store.whole_graph().unwrap())
        .query_ir(
            tine_store::IrRequest::Run {
                query: &query,
                view: &view,
                context: &ExecutionContext::default(),
            },
            &tine_store::Cancel::default(),
        )
        .unwrap()
        .expect("never cancelled");
    let tine_store::IrAnswer::Result(result) = answer else {
        panic!("{name}: result rows")
    };
    match &result.rows {
        QueryRows::Block { groups } => raws(groups),
        other => panic!("{name}: block rows, got {other:?}"),
    }
}

/// What the store answers about the year-less label graph: each file's
/// conflict twins, the `[[name]]` query, the Linked References panel and the
/// incoming links of each name.
fn label_answers(store: &Store, names: &[&str]) -> Vec<String> {
    let view = store.whole_graph().unwrap();
    let mut out = Vec::new();
    for entry in view.inventory(InventoryScope::All).0.iter() {
        if let Resolved::Existing { id, others } = &entry.target {
            out.push(format!("{} {}: others {others:?}", entry.name, id.as_str()));
        }
    }
    for name in names {
        out.push(format!("[[{name}]] query: {:?}", ref_query(store, name)));
        out.push(format!(
            "{name} panel: {:?}",
            raws(&view.backlinks(name).unwrap())
        ));
    }
    out
}

/// CM SPEC §2.1 (approved label limitation) and §2.2: with a year-less title
/// format, the label `11-02` names the ordinary page `11-02`, and each day
/// is its own identity, reached by a spelling that names it. Conformance
/// fixture `d7-year-less-title-label` pins the model's answers for one day;
/// this adds another year and the incremental reading. Before the fix the
/// real-name index filed every day under its label: the ordinary page and
/// both days were one conflicted name, `[[11-02]]` matched the journals'
/// blocks, `[[2026-11-02]]` missed its own journal, and the day's panel was
/// the label page's.
#[test]
fn a_year_less_label_is_the_ordinary_page_and_each_day_is_its_own() {
    let _guard = crate::integration_lock();
    let files = [
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"MM-dd\"}\n",
        ),
        ("journals/2025_11_02.md", "- last year\n"),
        ("journals/2026_11_02.md", "- the day\n"),
        ("pages/11-02.md", "- a label page\n"),
        (
            "pages/Notes.md",
            "- label [[11-02]]\n- day [[2026-11-02]]\n- year before [[2025-11-02]]\n",
        ),
    ];
    let (dir, store) = graph(&files);
    let before = bytes(dir.path(), &files);
    let names = ["11-02", "2026-11-02", "2025-11-02"];
    let expected = vec![
        "11-02 journals/2025_11_02.md: others []".to_owned(),
        "11-02 journals/2026_11_02.md: others []".to_owned(),
        "11-02 pages/11-02.md: others []".to_owned(),
        "Notes pages/Notes.md: others []".to_owned(),
        r#"[[11-02]] query: ["a label page", "label [[11-02]]"]"#.to_owned(),
        r#"11-02 panel: ["label [[11-02]]"]"#.to_owned(),
        r#"[[2026-11-02]] query: ["day [[2026-11-02]]", "the day"]"#.to_owned(),
        r#"2026-11-02 panel: ["day [[2026-11-02]]"]"#.to_owned(),
        r#"[[2025-11-02]] query: ["last year", "year before [[2025-11-02]]"]"#.to_owned(),
        r#"2025-11-02 panel: ["year before [[2025-11-02]]"]"#.to_owned(),
    ];
    let mut answers = label_answers(&store, &names);
    answers[..4].sort();
    assert_eq!(answers, expected);
    // The incremental reading after an outside edit answers the same.
    fs::write(
        dir.path().join("journals/2026_11_02.md"),
        "- the day, edited\n",
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let edited: Vec<String> = (expected.iter())
        .map(|line| line.replace("\"the day\"", "\"the day, edited\""))
        .collect();
    loop {
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let mut answers = label_answers(&store, &names);
        answers[..4].sort();
        if answers == edited || std::time::Instant::now() >= deadline {
            assert_eq!(answers, edited, "incremental answers");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        bytes(dir.path(), &files)[..2],
        before[..2],
        "nothing else is written"
    );
}

/// REG-CM-YEARLESS-DAY-KEY (review item 4): a day's expansion holds only the
/// spellings that identify it. Under `MM-dd` the label `11-02` names no day,
/// so with no label page `[[11-02]]` is the reference-only ordinary page and
/// never the day's blocks, and `[[2026-11-02]]` never the label's links; the
/// same for an alias that is a date (`alias:: 2026-11-03`), whose label
/// `11-03` joins nothing. Before the fix the day's spelling keys included
/// its label, so both queries and panels mixed the two. Markdown and Org day
/// files, a day in journals/ and in pages/.
#[test]
fn a_year_less_label_without_its_page_joins_no_day() {
    let _guard = crate::integration_lock();
    for (day_rel, day_text) in [
        ("journals/2026_11_02.md", "- the day\n"),
        ("journals/2026_11_02.org", "* the day\n"),
        ("pages/2026-11-02.md", "- the day\n"),
    ] {
        let files = [
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"MM-dd\"}\n",
            ),
            (day_rel, day_text),
            ("pages/Meeting.md", "alias:: 2026-11-03\n\n- the meeting\n"),
            (
                "pages/Notes.md",
                "- label [[11-02]]\n- day [[2026-11-02]]\n- other label [[11-03]]\n- other day [[2026-11-03]]\n",
            ),
        ];
        let (_dir, store) = graph(&files);
        let view = store.whole_graph().unwrap();
        let panel = |name: &str| raws(&view.backlinks(name).unwrap());
        assert_eq!(
            ref_query(&store, "2026-11-02"),
            ["day [[2026-11-02]]", "the day"],
            "{day_rel}"
        );
        assert_eq!(ref_query(&store, "11-02"), ["label [[11-02]]"], "{day_rel}");
        assert_eq!(panel("2026-11-02"), ["day [[2026-11-02]]"], "{day_rel}");
        assert_eq!(panel("11-02"), ["label [[11-02]]"], "{day_rel}");
        assert_eq!(
            ref_query(&store, "11-03"),
            ["other label [[11-03]]"],
            "{day_rel}: an alias-date label"
        );
        assert!(
            !ref_query(&store, "2026-11-03").contains(&"other label [[11-03]]".to_owned()),
            "{day_rel}: the alias day holds no label link"
        );
        assert_eq!(panel("11-03"), ["other label [[11-03]]"], "{day_rel}");
    }
}

/// REG-CM-YEARLESS-DAY-KEY (review item 3): the page inventory partitions a
/// name's claimant bucket by identity once per question, so a graph of D
/// years of `11-02` under `MM-dd` costs O(D) claimant visits, not the D² of
/// reading every file that shares the label for each row.
#[test]
fn year_less_inventory_claimant_work_is_linear_in_the_days() {
    let _guard = crate::integration_lock();
    let visits = |years: i32| {
        let mut files: Vec<(String, String)> = vec![
            (
                "logseq/config.edn".into(),
                "{:journal/page-title-format \"MM-dd\"}\n".into(),
            ),
            ("pages/11-02.md".into(), "- a label page\n".into()),
        ];
        for year in 1990..1990 + years {
            let ext = if year % 2 == 0 { "md" } else { "org" };
            files.push((format!("journals/{year}_11_02.{ext}"), "- a day\n".into()));
        }
        let borrowed: Vec<(&str, &str)> = (files.iter())
            .map(|(rel, text)| (rel.as_str(), text.as_str()))
            .collect();
        let (_dir, store) = graph(&borrowed);
        let view = store.whole_graph().unwrap();
        let before = tine_store::cost_counters::claimant_visits();
        let rows = view.inventory(InventoryScope::All).0.len();
        assert_eq!(rows, years as usize + 1);
        tine_store::cost_counters::claimant_visits() - before
    };
    let (small, large) = (visits(8), visits(32));
    assert!(large <= 2 * 32 + 2, "claimant visits for 32 days: {large}");
    assert!(
        large <= 4 * small + 4,
        "claimant work grows linearly: 8 days {small}, 32 days {large}"
    );
}

/// REG-CM-YEARLESS-DAY-KEY (round 3, the owner fix): the page a read hands
/// the app is named by the spelling that identifies it (`name`, what the app
/// keys its working set, feed, sidebar, reference requests and saves by) and
/// titled by its name (`title`, what it shows). Under `MM-dd` two years of
/// `11-02` and the label page are three names; a day reached by its `title::`
/// in pages/ is named so its date can be read back, and its reference panel
/// asked by that name is its own. Every title format with a year names each
/// page by its title, as before. Before the fix every year read back as
/// `11-02`, the label page's name.
#[test]
fn a_page_read_names_a_year_less_day_by_the_spelling_that_identifies_it() {
    let _guard = crate::integration_lock();
    let config = "{:journal/page-title-format \"MM-dd\"}\n";
    for (ext, bullet) in [("md", "-"), ("org", "*")] {
        for label_page in [true, false] {
            let older = format!("journals/2025_11_02.{ext}");
            let newer = format!("journals/2026_11_02.{ext}");
            let (older_text, newer_text) = (
                format!("{bullet} last year\n"),
                format!("{bullet} the day\n"),
            );
            let mut files = vec![
                ("logseq/config.edn", config),
                (older.as_str(), older_text.as_str()),
                (newer.as_str(), newer_text.as_str()),
                ("pages/Meeting.md", "title:: 2026-11-04\n\n- the meeting\n"),
                (
                    "pages/Notes.md",
                    "- day [[2026-11-02]]\n- label [[11-02]]\n- meeting [[2026-11-04]]\n",
                ),
            ];
            if label_page {
                files.push(("pages/11-02.md", "- a label page\n"));
            }
            let (_dir, store) = graph(&files);
            let case = format!("{ext}, label page {label_page}");
            let read = |rel: &str| store.page(&PageId::from(rel)).unwrap().doc;
            let names = |rel: &str| {
                let doc = read(rel);
                (doc.name, doc.title, doc.kind)
            };
            assert_eq!(
                names(&newer),
                ("2026_11_02".into(), "11-02".into(), PageKind::Journal),
                "{case}"
            );
            assert_eq!(
                names(&older),
                ("2025_11_02".into(), "11-02".into(), PageKind::Journal),
                "{case}"
            );
            assert_eq!(
                names("pages/Meeting.md"),
                ("2026_11_04".into(), "11-04".into(), PageKind::Journal),
                "{case}"
            );
            // The name a read gives is the name the store opens it by.
            for rel in [newer.as_str(), older.as_str(), "pages/Meeting.md"] {
                let name = read(rel).name;
                assert_eq!(
                    opens(&store, &name, PageKind::Journal).map(|(id, _)| id),
                    Some(rel.to_owned()),
                    "{case}: {name}"
                );
            }
            let view = store.whole_graph().unwrap();
            assert_eq!(
                raws(&view.backlinks(&read(&newer).name).unwrap()),
                ["day [[2026-11-02]]"],
                "{case}"
            );
            assert_eq!(
                raws(&view.backlinks(&read("pages/Meeting.md").name).unwrap()),
                ["meeting [[2026-11-04]]"],
                "{case}"
            );
            if label_page {
                assert_eq!(
                    names("pages/11-02.md"),
                    ("11-02".into(), "11-02".into(), PageKind::Page),
                    "{case}"
                );
            }
        }
    }
    // A title format with a year: name and title are the title, as before.
    let (_dir, store) = graph(&[
        ("journals/2026_11_02.md", "- the day\n"),
        ("pages/Meeting.md", "title:: 2026-11-04\n\n- the meeting\n"),
    ]);
    for (rel, title) in [
        ("journals/2026_11_02.md", "Nov 2nd, 2026"),
        ("pages/Meeting.md", "Nov 4th, 2026"),
    ] {
        let doc = store.page(&PageId::from(rel)).unwrap().doc;
        assert_eq!(
            (doc.name.as_str(), doc.title.as_str()),
            (title, title),
            "{rel}"
        );
    }
}

/// REG-CM-YEARLESS-DAY-KEY (round 3): a reference panel, a block reference
/// and a query name each source page as a page read names it, so the app
/// loads and edits the source by that name. Under `MM-dd` the two years of
/// `11-02` and the label page are three groups, not one. Before the fix the
/// groups were merged and named `11-02`, which opens the label page (or one
/// year) and is not the name the day's own read gives.
#[test]
fn reference_groups_name_a_year_less_source_day_as_its_read_does() {
    let _guard = crate::integration_lock();
    for (ext, bullet) in [("md", "-"), ("org", "*")] {
        for label_page in [true, false] {
            let older = format!("journals/2025_11_02.{ext}");
            let newer = format!("journals/2026_11_02.{ext}");
            let older_text = format!("{bullet} last year [[Topic]]\n");
            let id = "6a000000-0000-4000-8000-000000000001";
            let newer_text = match ext {
                "md" => format!("- the day [[Topic]]\n  id:: {id}\n"),
                _ => format!("* the day [[Topic]]\n:PROPERTIES:\n:id: {id}\n:END:\n"),
            };
            let mut files = vec![
                (
                    "logseq/config.edn",
                    "{:journal/page-title-format \"MM-dd\"}\n",
                ),
                (older.as_str(), older_text.as_str()),
                (newer.as_str(), newer_text.as_str()),
                (
                    "pages/Meeting.md",
                    "title:: 2026-11-04\n\n- meeting [[Topic]]\n",
                ),
                ("pages/Topic.md", "- topic\n"),
            ];
            if label_page {
                files.push(("pages/11-02.md", "- a label page [[Topic]]\n"));
            }
            let (_dir, store) = graph(&files);
            let case = format!("{ext}, label page {label_page}");
            let view = store.whole_graph().unwrap();
            let mut expected = vec!["2026_11_04", "2026_11_02", "2025_11_02"];
            if label_page {
                expected.push("11-02");
            }
            let pages = |groups: &[tine_core::model::RefGroup]| {
                let mut pages: Vec<String> =
                    groups.iter().map(|group| group.page.clone()).collect();
                pages.sort_by(|a, b| b.cmp(a));
                pages
            };
            assert_eq!(
                pages(&view.backlinks("Topic").unwrap()),
                expected,
                "{case}: backlinks"
            );
            let tine_store::QueryResult::Simple(groups) = view
                .query("[[Topic]]", tine_store::QueryDialect::Simple)
                .unwrap()
            else {
                panic!("block query")
            };
            // The query also answers the topic page's own block, as before.
            assert_eq!(
                pages(&groups),
                [&["Topic"][..], &expected].concat(),
                "{case}: query"
            );
            let preview = view.preview_block(id, 8).unwrap().expect(&case);
            assert_eq!(preview.group.page, "2026_11_02", "{case}: block preview");
            let resolved = view.blocks(&[id.to_owned()], true).unwrap();
            assert_eq!(
                resolved[0].as_ref().map(|group| group.page.as_str()),
                Some("2026_11_02"),
                "{case}: block reference"
            );
            for group in view.backlinks("Topic").unwrap().iter() {
                let doc = store
                    .page_named(&group.page, group.kind)
                    .unwrap()
                    .unwrap()
                    .doc;
                assert_eq!(
                    (doc.name.as_str(), doc.kind),
                    (group.page.as_str(), group.kind),
                    "{case}"
                );
            }
        }
    }
}
