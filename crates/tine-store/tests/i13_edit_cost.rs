//! I-13/I-15/I-25: one page edit must not inherit a graph-sized disk bill.
use std::fs;
use std::sync::Mutex;

use tine_store::cost_counters::{self, Counts};
use tine_store::{PageId, SaveBase, Store};

static CASE_LOCK: Mutex<()> = Mutex::new(());

/// A fixture graph. Callers bind it as `let (_dir, store, id) = graph(..)`: the
/// self-deleting `TempDir` is declared first, so it drops last (after the store
/// releases the files) and the graph never outlives the test, panic included.
fn graph(pages: usize, blocks: usize) -> (tempfile::TempDir, Store, PageId) {
    graph_with_target(pages, blocks, 0)
}

fn graph_with_target(
    pages: usize,
    blocks: usize,
    target: usize,
) -> (tempfile::TempDir, Store, PageId) {
    let dir = tempfile::Builder::new()
        .prefix("i13-cost-")
        .tempdir()
        .unwrap();
    let root = dir.path().to_path_buf();
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    for index in 0..pages {
        let body = if index == target {
            "- before\n".repeat(blocks)
        } else {
            format!("- unrelated {index}\n")
        };
        fs::write(root.join("pages").join(format!("Page{index:04}.md")), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (
        dir,
        store,
        PageId::from(format!("pages/Page{target:04}.md")),
    )
}

/// The cost of one host save, with the part spent planning it on the caller's
/// thread and the whole save through the host's publication.
fn edit(pages: usize, blocks: usize) -> (Counts, Counts, usize) {
    let (_dir, store, id) = graph(pages, blocks);
    let store = std::sync::Arc::new(store);
    let host_writer = crate::test_host_writer::HostWriter::start(&store);
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = "after".into();
    cost_counters::reset();
    let kind = tine_store::EditKind::ReplacePage;
    let mut plan = store.page_plan(Some(kind));
    plan.save_page(&[kind], &id, SaveBase::Existing(read.rev), &doc);
    let op = plan.plan().unwrap();
    let planned = cost_counters::snapshot();
    host_writer.apply(op).unwrap();
    let counts = cost_counters::snapshot();
    let len = fs::metadata(store.path_for_os_handoff(&id.file(), false).unwrap())
        .unwrap()
        .len() as usize;
    store.close();
    (planned, counts, len)
}

#[test]
fn edit_cost_is_page_bounded() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap();
    for blocks in [1, 60] {
        let (small_planned, small, small_len) = edit(20, blocks);
        let (large_planned, large, large_len) = edit(2000, blocks);
        eprintln!("I-25 unit cost: blocks={blocks}, 20 pages={small:?}, 2000 pages={large:?}, page bytes={small_len}/{large_len}");
        if std::env::var_os("TINE_I13_OBSERVE").is_some() {
            continue;
        }
        assert_eq!(
            small.readdir, 0,
            "I-13: a save must not walk directories; exemplar transaction.rs:349 Step::Save"
        );
        assert_eq!(
            large.readdir, 0,
            "I-13: a save must not walk directories; exemplar transaction.rs:349 Step::Save"
        );
        // The counter counts every document/outline parse since OG-B-COST (it
        // counted only the publication parse before, and read 1 while a save
        // really parsed 11 times). Planning ratchet: 2 (one authoritative
        // old-source parse, one retained-layout reparse); I-15's target is 1.
        // The host then reparses the bytes it publishes, so a whole save is 3.
        // Both may only fall.
        assert!(
            small_planned.parses <= 2,
            "I-15: parses per planned save may only fall (ratchet 2, target 1): {}; exemplar transaction.rs:349 Step::Save",
            small_planned.parses
        );
        assert_eq!(
            large_planned.parses, small_planned.parses,
            "I-13/I-15: planning parses must not grow with graph pages"
        );
        assert!(
            small.parses <= 3,
            "I-15: parses per host save may only fall (plan 2 + publication 1): {}",
            small.parses
        );
        assert_eq!(large.parses, small.parses, "I-13/I-15: parse count must not grow with graph pages; exemplar transaction.rs:349 Step::Save");
        assert_eq!(large.full_reads, small.full_reads, "I-15: full reads must not grow with graph pages; exemplar transaction.rs:349 Step::Save");
        assert_eq!(
            large.files_written, small.files_written,
            "I-25: one page file plus one temp; exemplar transaction.rs:349 Step::Save"
        );
        assert_eq!(
            large.bytes_written, small.bytes_written,
            "I-25: bytes written must follow page bytes; exemplar transaction.rs:349 Step::Save"
        );
        assert_eq!(small.snapshot_rebuilds, 0, "I-13: a content edit must not rebuild the P-entry snapshot index; exemplar store.rs:259 Snapshot::capture");
        assert_eq!(large.snapshot_rebuilds, 0, "I-13: a content edit must not rebuild the P-entry snapshot index; exemplar store.rs:259 Snapshot::capture");
        // An applied HostOp also drafts (contract: "only a conflict, a failed
        // save or a host operation drafts"). The exact one-file, page-bytes
        // proof of an ordinary edit is
        // host_edit_cost::an_ordinary_edit_is_one_guarded_save_with_no_draft.
        assert_eq!(
            (small_planned.files_written, small_planned.bytes_written),
            (0, 0),
            "I-25: planning a save writes nothing"
        );
    }
}

#[test]
fn memo_carry_checks_only_the_changed_page() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap();
    let mut max_probes = 0;
    for blocks in [1, 60] {
        for pages in [1, 1000] {
            let (_dir, store, id) = graph_with_target(pages, blocks, pages - 1);
            let store = std::sync::Arc::new(store);
            let host_writer = crate::test_host_writer::HostWriter::start(&store);
            let old = store.whole_graph().unwrap();
            let _ = old.backlinks("Target").unwrap();
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "after".into();
            cost_counters::reset();
            let outcome = host_writer.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(outcome, Ok(_)), "{outcome:?}");
            let counts = cost_counters::snapshot();
            eprintln!(
                "memo carry: blocks={blocks} pages={pages} probes={} bytes_written={}",
                counts.memo_page_probes, counts.bytes_written
            );
            max_probes = max_probes.max(counts.memo_page_probes);
            store.close();
        }
    }
    assert!(max_probes <= 2, "I-13: carrying a memo must inspect only the edited page; exemplar model.rs:850 carry_memos_from, observed {max_probes} page probes");
}

#[test]
fn held_snapshot_edit_and_icon_request_have_bounded_page_work() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap();
    for blocks in [1, 60] {
        for pages in [1, 1000] {
            let (_dir, store, id) = graph(pages, blocks);
            let store = std::sync::Arc::new(store);
            let host_writer = crate::test_host_writer::HostWriter::start(&store);
            let old = store.whole_graph().unwrap();
            cost_counters::reset();
            let _icons = old.page_icons(&["Page0000".into()]);
            let icon = cost_counters::snapshot();
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "after".into();
            cost_counters::reset();
            let outcome = host_writer.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(outcome, Ok(_)), "{outcome:?}");
            let edit = cost_counters::snapshot();
            eprintln!("remaining I-13/I-25: blocks={blocks} pages={pages} icon_probes={} cache_copies={} signature_blocks={} bytes_written={} files_written={}", icon.icon_page_probes, edit.cache_page_copies, edit.signature_block_probes, edit.bytes_written, edit.files_written);
            if std::env::var_os("TINE_I13_OBSERVE").is_none() {
                assert!(icon.icon_page_probes <= 2, "I-13: page_icons must inspect only requested names; exemplar model.rs:990 page_icons, observed {}", icon.icon_page_probes);
                assert_eq!(edit.signature_block_probes, blocks as u64, "I-25: reference signature may scan the changed page only; exemplar model.rs:1353 reference_signature");
            }
            store.close();
        }
    }
}

#[test]
fn single_page_print_builds_one_corpus() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap();
    for pages in [20, 2000] {
        let (_dir, store, _) = graph(pages, 1);
        cost_counters::reset();
        let html =
            tine_graph_features::print::page_print_html(&store, "Page0000", Default::default())
                .unwrap();
        let counts = cost_counters::snapshot();
        eprintln!("I-13 print: pages={pages}, counts={counts:?}");
        assert!(html.is_some());
        if std::env::var_os("TINE_I13_OBSERVE").is_none() {
            assert!(
                counts.corpus <= 1,
                "I-15: one corpus for single-page print; exemplar print.rs:29 page_print_html"
            );
        }
        store.close();
    }
}

#[test]
fn full_publish_builds_a_constant_number_of_corpora() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap();
    // The site `tine publish` writes: one corpus of the graph view to select
    // from, and one of the private build graph it renders from
    // (`build_graph.rs`), whatever the number of pages.
    let corpora = |pages: usize| {
        let (_dir, store, _) = graph(pages, 1);
        let out = tempfile::tempdir().unwrap();
        cost_counters::reset();
        let _ = tine_graph_features::publish_query::publish_static(
            &store,
            out.path(),
            "site",
            true,
            true,
        )
        .unwrap();
        let counts = cost_counters::snapshot();
        store.close();
        counts.corpus
    };
    let (small, large) = (corpora(20), corpora(40));
    assert_eq!(small, 2, "I-15: full publish builds the view's corpus and the build graph's, no more; exemplar publish_query.rs publish_static");
    assert_eq!(
        large, small,
        "I-15: the number of corpora does not grow with the pages"
    );
}
