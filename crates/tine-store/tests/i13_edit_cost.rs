//! I-13/I-15/I-25: one page edit must not inherit a graph-sized disk bill.
use std::fs;
use std::sync::Mutex;

use tine_store::cost_counters::{self, Counts};
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

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
    let dir = tempfile::Builder::new().prefix("i13-cost-").tempdir().unwrap();
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
    (dir, store, PageId::from(format!("pages/Page{target:04}.md")))
}

fn edit(pages: usize, blocks: usize) -> (Counts, usize) {
    let (_dir, store, id) = graph(pages, blocks);
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = "after".into();
    cost_counters::reset();
    let outcome = store.save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(read.rev),
        &doc,
    );
    let counts = cost_counters::snapshot();
    assert!(
        matches!(outcome, SaveOutcome::Saved(_)),
        "I-13 exemplar transaction.rs:349 Step::Save: {outcome:?}"
    );
    let len = fs::metadata(store.path_for_os_handoff(&id.file(), false).unwrap())
        .unwrap()
        .len() as usize;
    store.close();
    (counts, len)
}

#[test]
fn edit_cost_is_page_bounded() {
    let _case = CASE_LOCK.lock().unwrap();
    for blocks in [1, 60] {
        let (small, small_len) = edit(20, blocks);
        let (large, large_len) = edit(2000, blocks);
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
        assert_eq!(
            small.parses, 1,
            "I-15: one parse per save; exemplar transaction.rs:349 Step::Save"
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
        assert!(
            small.bytes_written <= small_len as u64 + 16,
            "I-25: bytes written must be about page bytes; exemplar transaction.rs:349 Step::Save"
        );
    }
}

#[test]
fn memo_carry_checks_only_the_changed_page() {
    let _case = CASE_LOCK.lock().unwrap();
    let mut max_probes = 0;
    for blocks in [1, 60] {
        for pages in [1, 1000] {
            let (_dir, store, id) = graph_with_target(pages, blocks, pages - 1);
            let old = store.whole_graph().unwrap();
            let _ = old.backlinks("Target").unwrap();
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "after".into();
            cost_counters::reset();
            let outcome = store.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
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
    let _case = CASE_LOCK.lock().unwrap();
    for blocks in [1, 60] {
        for pages in [1, 1000] {
            let (_dir, store, id) = graph(pages, blocks);
            let old = store.whole_graph().unwrap();
            cost_counters::reset();
            let _icons = old.page_icons(&["Page0000".into()]);
            let icon = cost_counters::snapshot();
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = "after".into();
            cost_counters::reset();
            let outcome = store.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
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
fn full_publish_builds_one_corpus() {
    let _case = CASE_LOCK.lock().unwrap();
    let (_dir, store, _) = graph(20, 1);
    cost_counters::reset();
    let _ = tine_graph_features::publish::publish_html(&store).unwrap();
    let counts = cost_counters::snapshot();
    assert_eq!(counts.corpus, 1, "I-15: full publish enumerates from the view and builds one corpus; exemplar publish.rs:9 publish_html");
    store.close();
}
