//! The rest of "one bad file never stops the graph load" (D-20261019-QC), on the
//! doors `load_panic_tests.rs` leaves open: a whole-graph index build (cold load,
//! Settings rescan, config edit), undecodable or oversized bytes a transaction
//! observed, a held page's parse panic, and a page a transient panic retired.
use super::load_panic_tests::{
    assert_rest_of_graph_answers, cached_doc, edit_good, graph_with_bad_page, others_cached,
    publish_held, ready_graph_without_watcher, settled, NoRowExplains,
};
use super::*;
use crate::model::index_probe::TEST_ROW_PANIC_PATHS;
use crate::model::{
    TEST_FULL_INDEX_PANIC_ROOTS, TEST_INDEX_PANIC_SENTINEL, TEST_PAGE_PARSE_PANIC_SENTINEL,
};
use std::fs;
use std::sync::Arc;

const JOURNAL: &str = "journals/2026_01_05.md";

fn listed(store: &Store) -> Vec<String> {
    let view = store.whole_graph().expect("one bad file fails no graph");
    let files = view.unreadable_files();
    files.iter().map(|(id, _)| id.as_str().to_owned()).collect()
}

/// A panic on one row of the cold load's whole-graph build costs that row: the
/// graph is Ready, the page is listed and still named and openable, and the
/// other pages answer and save.
#[test]
fn a_cold_load_lists_the_page_an_index_stage_panics_on() {
    for fillers in [2, 80] {
        let tag = format!("cold-{fillers}");
        let root = graph_with_bad_page(&tag, &format!("- {TEST_INDEX_PANIC_SENTINEL}\n"), fillers);
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let outcome = settled(&store);
        assert!(matches!(outcome, Ok(true)), "{tag}: {outcome:?}");
        assert_eq!(listed(&store), ["pages/Bad.md"], "{tag}");
        assert_rest_of_graph_answers(&store, &tag, fillers + 1);
        assert!(
            store.page_named("Bad", PageKind::Page).unwrap().is_some(),
            "{tag}: the page keeps its name and opens"
        );
        let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
        assert!(edit_good(&store, &good).is_ok(), "{tag}");
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// Several bad files, a journal among them: each is listed, the rest work.
#[test]
fn a_cold_load_lists_every_page_an_index_stage_panics_on() {
    let bad = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    let root = graph_with_bad_page("cold-many", &bad, 20);
    fs::write(root.join(JOURNAL), &bad).unwrap();
    fs::write(root.join("pages/Filler 07.md"), &bad).unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    let mut files = listed(&store);
    files.sort();
    assert_eq!(files, [JOURNAL, "pages/Bad.md", "pages/Filler 07.md"]);
    assert_rest_of_graph_answers(&store, "cold-many", 20);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A repair brings a page the cold load contained back, with no rescan.
#[test]
fn a_page_a_cold_load_contained_returns_when_repaired() {
    let bad = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    let root = graph_with_bad_page("cold-repair", &bad, 2);
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    assert!(matches!(settled(&store), Ok(true)));
    store.watch.stop_background_for_test();
    fs::write(root.join("pages/Bad.md"), "- repaired [[Target]]\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(listed(&store), Vec::<String>::new());
    assert_rest_of_graph_answers(&store, "cold-repair", 4);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The review's F4: a Settings rescan in a working session with one contained
/// page took the whole graph down.
#[test]
fn a_settings_rescan_keeps_the_graph_with_a_contained_page() {
    let (root, store) = ready_graph_without_watcher("rescan-contained");
    let bad = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    fs::write(root.join("pages/Filler 00.md"), &bad).unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_eq!(listed(&store), ["pages/Filler 00.md"]);
    for pass in 0..2 {
        let rescanned = store.refresh(Depth::Rebuild);
        assert!(rescanned.is_ok(), "rescan {pass}: {rescanned:?}");
        assert_eq!(listed(&store), ["pages/Filler 00.md"], "rescan {pass}");
        assert_rest_of_graph_answers(&store, "rescan-contained", 2);
    }
    assert!(!store.writer.is_poisoned());
    let good = store.page_named("Good", PageKind::Page).unwrap().unwrap();
    assert!(edit_good(&store, &good).is_ok());
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A build-relevant config edit rebuilds every index and names no page: a row
/// only that build reaches costs that row.
#[test]
fn a_config_edit_contains_a_row_its_rebuild_panics_on() {
    let (root, store) = ready_graph_without_watcher("config-row");
    let path = store.graph.root.join("pages/Filler 00.md");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:property-pages/excludelist #{:unused}}\n",
    )
    .unwrap();
    TEST_ROW_PANIC_PATHS.lock().unwrap().push(path.clone());
    let refreshed = store.refresh(Depth::Bytes);
    let files = store
        .whole_graph()
        .map(|view| view.unreadable_files().len());
    TEST_ROW_PANIC_PATHS.lock().unwrap().retain(|p| *p != path);
    assert!(refreshed.is_ok(), "{refreshed:?}");
    assert!(matches!(files, Ok(1)), "{files:?}");
    assert_eq!(listed(&store), ["pages/Filler 00.md"]);
    assert_rest_of_graph_answers(&store, "config-row", 2);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The backstop stays: a panic no row reproduces fails the graph, blames no page,
/// and a retry recovers it with nothing listed.
#[test]
fn a_panic_no_row_reproduces_fails_the_load_and_lists_nothing() {
    let root = graph_with_bad_page("no-row", "- fine\n", 2);
    let canonical = fs::canonicalize(&root).unwrap();
    TEST_FULL_INDEX_PANIC_ROOTS
        .lock()
        .unwrap()
        .push(canonical.clone());
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let outcome = settled(&store);
    TEST_FULL_INDEX_PANIC_ROOTS
        .lock()
        .unwrap()
        .retain(|r| *r != canonical);
    assert!(
        matches!(outcome, Err(LoadError::Failed { .. })),
        "{outcome:?}"
    );
    store.refresh(Depth::Stamps).unwrap();
    assert!(matches!(settled(&store), Ok(true)));
    assert_eq!(listed(&store), Vec::<String>::new());
    assert_rest_of_graph_answers(&store, "no-row", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// Many bad files, a regression on something common rather than a
/// bad file, cost one panicking build per probe without bound, under the
/// writer and at every launch. They cost the probe budget, then the load fails
/// with no page blamed, as before probing; a repair recovers it.
#[test]
fn many_bad_pages_fail_the_load_within_the_probe_budget() {
    let bad = format!("- {TEST_INDEX_PANIC_SENTINEL}\n");
    let root = graph_with_bad_page("probe-budget", &bad, 40);
    for i in (0..40).step_by(2) {
        fs::write(root.join(format!("pages/Filler {i:02}.md")), &bad).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let outcome = settled(&store);
    assert!(
        matches!(outcome, Err(LoadError::Failed { .. })),
        "{outcome:?}"
    );
    assert!(
        store.graph.unreadable_pages().is_empty(),
        "no page is blamed"
    );
    let (builds, _) = crate::model::index_probe::probe_cost(&store.graph.root);
    assert!(
        (1..=crate::model::index_probe::PROBE_BUILDS).contains(&builds),
        "{builds} probe builds"
    );
    store.watch.stop_background_for_test();
    for i in (0..40).step_by(2) {
        fs::write(root.join(format!("pages/Filler {i:02}.md")), "- fixed\n").unwrap();
    }
    fs::write(root.join("pages/Bad.md"), "- fixed\n").unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert!(matches!(settled(&store), Ok(true)));
    assert_eq!(listed(&store), Vec::<String>::new());
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A row that panics when built alone, while the build without it
/// panics all the same, is not shown to be the bad file. It was listed
/// unreadable and kept out of a recovered graph; now the load fails with no
/// page blamed, and a recovery brings every page back.
#[test]
fn a_row_that_does_not_explain_the_panic_is_not_blamed() {
    let root = graph_with_bad_page("probe-unexplained", "- fine\n", 2);
    let path = fs::canonicalize(&root).unwrap().join("pages/Filler 00.md");
    let no_row = NoRowExplains::arm(&root);
    TEST_ROW_PANIC_PATHS.lock().unwrap().push(path.clone());
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let outcome = settled(&store);
    TEST_ROW_PANIC_PATHS.lock().unwrap().retain(|p| *p != path);
    drop(no_row);
    assert!(
        matches!(outcome, Err(LoadError::Failed { .. })),
        "{outcome:?}"
    );
    assert!(
        store.graph.unreadable_pages().is_empty(),
        "{:?}",
        store.graph.unreadable_pages()
    );
    store.watch.stop_background_for_test();
    store.refresh(Depth::Stamps).unwrap();
    assert!(matches!(settled(&store), Ok(true)));
    assert_eq!(listed(&store), Vec::<String>::new());
    assert_rest_of_graph_answers(&store, "probe-unexplained", 3);
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// Item 2: a move publishes bytes no parse may take. That one file is listed
/// and no other page is read again; before, the whole page cache was discarded
/// and rebuilt, and every other file's listed state went with it.
#[test]
fn a_move_of_undecodable_bytes_costs_that_file_only() {
    let bytes = b"- a \xff\xfe\n".to_vec();
    let (root, store) = ready_graph_without_watcher("move-undecodable");
    let others = others_cached(&store);
    fs::write(root.join("pages/Odd.md"), &bytes).unwrap();
    let from = FileId::from("pages/Odd.md".to_owned());
    let to = FileId::from("pages/Moved.md".to_owned());
    let mut plan = store.page_plan(Some(crate::EditKind::ReplacePage));
    plan.move_file(&from, FileRev::from_bytes(&bytes), &to, None);
    let writer = crate::test_host_writer::HostWriter::start(&store);
    let outcome = writer.apply(plan.plan().unwrap());
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(fs::read(root.join("pages/Moved.md")).unwrap(), bytes);
    writer.settle();
    assert_eq!(listed(&store), ["pages/Moved.md"]);
    for (rel, before) in &others {
        assert!(Arc::ptr_eq(before, &cached_doc(&store, rel)), "{rel}");
    }
    assert_rest_of_graph_answers(&store, "move-undecodable", 3);
    drop((writer, store));
    let _ = fs::remove_dir_all(&root);
}

/// Item 3: a held page whose owner's bytes do not parse, or do not decode, has
/// no row. It was gone from the index with no report until its host let go.
#[test]
fn a_held_page_with_no_row_for_its_bytes_is_listed_while_held() {
    let key = "pages/Filler 00.md";
    for (tag, bad) in [
        (
            "parse",
            format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n").into_bytes(),
        ),
        ("undecodable", b"- a \xff\xfe\n".to_vec()),
    ] {
        let (root, store) = ready_graph_without_watcher(&format!("held-{tag}"));
        let others = others_cached(&store);
        store.hold_page(&PageId::from(key)).unwrap();
        {
            let _writer = store.writer.lock().unwrap();
            assert!(store
                .graph
                .publish_owned(key, Some(Arc::from(&bad[..])), None)
                .is_ok());
            let file = FileId::from(key.to_owned());
            let files = vec![(file, ChangeKind::Modified, None)];
            store.changes.publish(Origin::Own, files, false, Vec::new());
        }
        assert_eq!(listed(&store), [key], "{tag}");
        assert_rest_of_graph_answers(&store, tag, 2);
        for (rel, before) in &others {
            assert!(
                Arc::ptr_eq(before, &cached_doc(&store, rel)),
                "{tag}: {rel}"
            );
        }
        // A rebuild of the cache around the held page reports it too.
        store.refresh(Depth::Rebuild).unwrap();
        assert_eq!(listed(&store), [key], "{tag}: after a rescan");
        publish_held(&store, key, "- back, mentions [[Target]]\n".to_owned());
        assert_eq!(
            listed(&store),
            Vec::<String>::new(),
            "{tag}: good bytes clear it"
        );
        assert_rest_of_graph_answers(&store, tag, 3);
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// Item 3 on the host's own route: a save whose bytes do not parse lands and
/// stays `Saved`, and the page is never missing unreported. While the host
/// holds it, it is listed; the host lets go on its own, and the release's
/// reread installs the saved document's row. Read under the writer, which
/// the release holds, so no state in between is seen.
#[test]
fn an_own_save_whose_bytes_do_not_parse_is_listed_after_it_lands() {
    let (root, store) = ready_graph_without_watcher("held-save");
    let key = "pages/Filler 00.md";
    let path = store.graph.root.join(key);
    let filler = store.page(&PageId::from(key)).unwrap();
    let mut doc = filler.doc.clone();
    doc.blocks[0].raw = TEST_PAGE_PARSE_PANIC_SENTINEL.into();
    let writer = crate::test_host_writer::HostWriter::start(&store);
    let saved = writer.save(
        crate::EditKind::SaveBlock,
        &filler.id,
        SaveBase::Existing(filler.rev.clone()),
        &doc,
    );
    assert!(saved.is_ok());
    assert!(fs::read_to_string(&path)
        .unwrap()
        .contains(TEST_PAGE_PARSE_PANIC_SENTINEL));
    let accounted = || {
        let _writer = store.writer.lock().unwrap();
        let listed = store.graph.unreadable_pages();
        let listed = listed.iter().any(|(id, _)| id.as_str() == key);
        if store.watch.holds(&path) {
            listed
        } else {
            !listed && store.graph.cached_rev(&path).is_some()
        }
    };
    assert!(accounted(), "listed while held, indexed once released");
    writer.settle();
    assert!(accounted(), "after the host let go");
    assert_rest_of_graph_answers(&store, "held-save", 2);
    drop((writer, store));
    let _ = fs::remove_dir_all(&root);
}

/// Item 5: a panic its bytes do not explain retired a good page, and only other
/// bytes or a rescan returned it. A refresh gives it another chance.
#[test]
fn a_refresh_returns_a_good_page_a_transient_panic_retired() {
    let (root, store) = ready_graph_without_watcher("transient");
    let path = store.graph.root.join("pages/Filler 00.md");
    fs::write(&path, "- edited, mentions [[Target]]\n").unwrap();
    TEST_ROW_PANIC_PATHS.lock().unwrap().push(path.clone());
    let refreshed = store.refresh(Depth::Bytes);
    let during = store.whole_graph().unwrap();
    assert!(refreshed.is_ok());
    assert_eq!(during.unreadable_files().len(), 1);
    // Still panicking: a retry leaves it out and publishes nothing.
    let rev = store.changes.rev();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(store.changes.rev(), rev, "a failed retry publishes nothing");
    TEST_ROW_PANIC_PATHS.lock().unwrap().retain(|p| *p != path);
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(listed(&store), Vec::<String>::new());
    assert_rest_of_graph_answers(&store, "transient", 3);
    assert!(store.changes.rev() > rev, "the return is published");
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// Item 5's bound: a page that really panics costs a few retries per revision of
/// its bytes, then nothing, however often the window takes focus.
#[test]
fn a_page_that_keeps_panicking_stops_being_retried() {
    let (root, store) = ready_graph_without_watcher("retry-bound");
    let path = store.graph.root.join("pages/Filler 00.md");
    fs::write(&path, format!("- {TEST_INDEX_PANIC_SENTINEL}\n")).unwrap();
    store.refresh(Depth::Bytes).unwrap();
    let rev = store.changes.rev();
    for _ in 0..10 {
        store.refresh(Depth::Stamps).unwrap();
    }
    assert_eq!(
        store.changes.rev(),
        rev,
        "bad bytes never pass the probe, and publish nothing"
    );
    assert_eq!(store.graph.contained_retries_left(&path), Some(0));
    assert_eq!(listed(&store), ["pages/Filler 00.md"]);
    fs::write(&path, format!("- {TEST_INDEX_PANIC_SENTINEL} other\n")).unwrap();
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(listed(&store), ["pages/Filler 00.md"]);
    assert!(
        store
            .graph
            .contained_retries_left(&path)
            .is_some_and(|left| left > 0),
        "other bytes get their own retries"
    );
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// A retired page open in an editor has its bytes from its owner,
/// so a retry skips it, yet every focus took the writer to learn that.
#[test]
fn a_held_retired_page_costs_a_focus_nothing() {
    let (root, store) = ready_graph_without_watcher("retry-held");
    let key = "pages/Filler 00.md";
    let path = store.graph.root.join(key);
    fs::write(&path, format!("- {TEST_INDEX_PANIC_SENTINEL}\n")).unwrap();
    store.refresh(Depth::Bytes).unwrap();
    assert_eq!(listed(&store), [key]);
    assert!(store.graph.contained_retry_due());
    store.hold_page(&PageId::from(key)).unwrap();
    assert!(
        !store.graph.contained_retry_due(),
        "a focus would take the writer for a page no retry may read"
    );
    store.refresh(Depth::Stamps).unwrap();
    assert_eq!(store.graph.contained_retries_left(&path), Some(3));
    drop(store);
    let _ = fs::remove_dir_all(&root);
}

/// The probe's cost, measured by hand over a generated graph (never a real one):
/// `TINE_PROBE_GRAPH=<generate-realistic-graph.mjs output> cargo test -p tine-store
/// --lib measure_the_probe -- --ignored --nocapture`. A scratch copy per case
/// gets the sentinel appended to its bad pages.
#[test]
#[ignore = "a measurement over a generated graph, run by hand"]
fn measure_the_probe_on_a_generated_graph() {
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let Some(source) = std::env::var_os("TINE_PROBE_GRAPH") else {
        return;
    };
    let source = std::path::PathBuf::from(source);
    let mut files: Vec<_> = ["pages", "journals"]
        .iter()
        .flat_map(|dir| fs::read_dir(source.join(dir)).unwrap())
        .map(|entry| entry.unwrap().path())
        .map(|path| path.strip_prefix(&source).unwrap().to_path_buf())
        .collect();
    files.sort();
    let cases: [(&str, fn(usize) -> bool); 4] = [
        ("clean", |_| false),
        ("one bad page", |i| i == 4321),
        ("30% bad", |i| i % 10 < 3),
        ("every page bad", |_| true),
    ];
    for (n, (case, bad)) in cases.into_iter().enumerate() {
        let root = crate::test_support::fresh_dir(&format!("tine-probe-measure-{n}-"));
        copy(&source, &root);
        let mut bad_pages = 0;
        for (_, rel) in files.iter().enumerate().filter(|(i, _)| bad(*i)) {
            let path = root.join(rel);
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str(&format!("\n- {TEST_INDEX_PANIC_SENTINEL}\n"));
            fs::write(&path, text).unwrap();
            bad_pages += 1;
        }
        let started = std::time::Instant::now();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let outcome = settled(&store);
        let load = started.elapsed();
        let listed = store.graph.unreadable_pages().len();
        let (builds, probing) =
            crate::model::index_probe::probe_cost(&fs::canonicalize(&root).unwrap());
        println!(
            "PROBE-MEASURE case={case:?} files={} bad={bad_pages} outcome={} listed={listed} \
             probe_builds={builds} probe_ms={} load_ms={}",
            files.len(),
            if outcome.is_ok() { "Ready" } else { "Failed" },
            probing.as_millis(),
            load.as_millis(),
        );
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// Review F1 of og-perf-publish: an own create whose capture panicked (on
/// its row, contained; or with no row to blame, failing the graph), then
/// `refresh()`. The claimant index, warm and complete before the create,
/// names the created page afterwards, and a fresh view resolves it.
#[test]
fn an_own_create_a_capture_missed_is_named_after_refresh() {
    for failed in [false, true] {
        let tag = format!("create-missed-{failed}");
        let (root, store) = ready_graph_without_watcher(&tag);
        assert_eq!(store.graph.find_claimants("Good", PageKind::Page).len(), 1);
        assert!(store
            .graph
            .find_claimants("Fresh", PageKind::Page)
            .is_empty());
        let rel = "pages/Fresh.md";
        let path = store.graph.root.join(rel);
        let canonical = fs::canonicalize(&root).unwrap();
        if failed {
            fs::create_dir_all(root.join("logseq")).unwrap();
            let config = "{:property-pages/excludelist #{:unused}}\n";
            fs::write(root.join("logseq/config.edn"), config).unwrap();
            TEST_FULL_INDEX_PANIC_ROOTS
                .lock()
                .unwrap()
                .push(canonical.clone());
            let refreshed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.refresh(Depth::Bytes)
            }));
            assert!(refreshed.is_ok(), "{tag}: the refresh must not unwind");
            assert!(matches!(store.whole_graph(), Err(LoadError::Failed { .. })));
        } else {
            TEST_ROW_PANIC_PATHS.lock().unwrap().push(path.clone());
        }
        let dto = (store.graph.page_dto_for_bytes(&path, b"- fresh\n"))
            .unwrap()
            .unwrap();
        let saved = crate::test_host_writer::HostWriter::start(&store).save(
            crate::EditKind::CreatePage,
            &PageId::from(rel),
            SaveBase::CreateNew,
            &dto,
        );
        TEST_ROW_PANIC_PATHS.lock().unwrap().retain(|p| *p != path);
        TEST_FULL_INDEX_PANIC_ROOTS
            .lock()
            .unwrap()
            .retain(|r| *r != canonical);
        assert!(saved.is_ok(), "{tag}");
        if !failed {
            assert_eq!(
                listed(&store),
                [rel],
                "{tag}: the capture panicked on its row"
            );
        }
        store.refresh(Depth::Stamps).unwrap();
        assert!(matches!(settled(&store), Ok(true)), "{tag}");
        let claimants = store.graph.find_claimants("Fresh", PageKind::Page);
        assert_eq!(
            claimants
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            [&path],
            "{tag}"
        );
        assert!(
            matches!(
                store.whole_graph().unwrap().resolve("Fresh", false),
                crate::Resolved::Existing { .. }
            ),
            "{tag}"
        );
        drop(store);
        let _ = fs::remove_dir_all(&root);
    }
}

/// A writer that panicked poisons the writer lock. `close` still completes
/// (the store reads as closed and refuses I/O), and `Drop`, which closes
/// again, neither panics nor aborts (review R1 of audit-storage-1109).
#[test]
fn close_after_a_writer_panicked_completes_and_drops() {
    let (root, store) = ready_graph_without_watcher("close-poisoned-writer");
    store.poison_writer_for_tests();
    assert!(store.writer.is_poisoned());
    store.close();
    assert!(store.is_closed());
    assert!(matches!(store.is_graph_ready(), Err(LoadError::Closed)));
    store.close();
    drop(Arc::try_unwrap(store).ok().expect("the only reference"));
    let _ = fs::remove_dir_all(&root);
}
