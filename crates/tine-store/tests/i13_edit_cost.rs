//! I-13/I-15/I-25: one page edit must not inherit a graph-sized disk bill.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tine_store::cost_counters::{self, Counts};
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

static CASE_LOCK: Mutex<()> = Mutex::new(());

fn graph(pages: usize, blocks: usize) -> (Store, PageId) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!(
            "i13-cost-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    for index in 0..pages {
        let body = if index == 0 {
            "- before\n".repeat(blocks)
        } else {
            format!("- unrelated {index}\n")
        };
        fs::write(root.join("pages").join(format!("Page{index:04}.md")), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    (store, PageId::from("pages/Page0000.md"))
}

fn edit(pages: usize, blocks: usize) -> (Counts, usize) {
    let (store, id) = graph(pages, blocks);
    let read = store.page(&id).unwrap();
    let mut doc = read.doc;
    doc.blocks[0].raw = "after".into();
    cost_counters::reset();
    let outcome = store.save(&id, SaveBase::Existing(read.rev), &doc);
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
fn single_page_print_builds_one_corpus() {
    let _case = CASE_LOCK.lock().unwrap();
    for pages in [20, 2000] {
        let (store, _) = graph(pages, 1);
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
    let (store, _) = graph(20, 1);
    cost_counters::reset();
    let _ = tine_graph_features::publish::publish_html(&store).unwrap();
    let counts = cost_counters::snapshot();
    assert_eq!(counts.corpus, 1, "I-15: full publish enumerates from the view and builds one corpus; exemplar publish.rs:9 publish_html");
    store.close();
}
