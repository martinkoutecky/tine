//! I-25 unit-cost probe for the query index and answer memo (og 14 Q2 G3):
//! work and allocated bytes per 1-block and 60-block page edit, on a small and
//! a 10k-page graph, with a warmed property registry and a populated memo.
//! The query side of an edit (index patch, registry carry, memo carry,
//! re-answering warmed queries) must not grow with the graph.
use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use tine_core::date::JournalDate;
use tine_core::query::ir::{ExecutionContext, ViewSettings};
use tine_core::query::{parse_query_text, QueryDialect as IrDialect};
use tine_store::cost_counters::{self, Counts};
use tine_store::{
    IrAnswer, IrRequest, PageId, QueryDialect, SaveBase, SaveOutcome, Store, WholeGraph,
};

/// Counts bytes allocated while `COUNTING` is set (process-wide).
struct Counting;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(
                new_size.saturating_sub(layout.size()) as u64,
                Ordering::Relaxed,
            );
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

static CASE_LOCK: Mutex<()> = Mutex::new(());

fn measure<T>(f: impl FnOnce() -> T) -> (T, u64, Counts) {
    cost_counters::reset();
    ALLOCATED.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    let out = f();
    COUNTING.store(false, Ordering::Relaxed);
    (
        out,
        ALLOCATED.load(Ordering::Relaxed),
        cost_counters::snapshot(),
    )
}

const QUERIES: &[&str] = &[
    "(task TODO)",
    "(property status s1)",
    "[[Page0003]]",
    "(and (task TODO) [[Page0003]])",
    "(and (task TODO) (property status s2))",
];

fn warm(graph: &WholeGraph, memo: bool) {
    match graph.query_ir(IrRequest::Registry) {
        Ok(IrAnswer::Registry(_)) => {}
        _ => panic!("registry"),
    }
    if !memo {
        return;
    }
    for query in QUERIES {
        graph.query(query, QueryDialect::Simple).expect("query");
        let (ir, view) = parse_query_text(query, IrDialect::Og, JournalDate::today());
        run(graph, &ir, &view);
    }
}

/// After the edit: the registry (first use patches the index and carries the
/// registry) and warmed queries whose answers are page-sized, so the bytes
/// measured are the edit's, not a graph-sized answer's copy to the caller.
fn after_edit(graph: &WholeGraph) {
    match graph.query_ir(IrRequest::Registry) {
        Ok(IrAnswer::Registry(_)) => {}
        _ => panic!("registry"),
    }
    for query in ["[[Page0003]]", "(and (task TODO) [[Page0003]])"] {
        graph.query(query, QueryDialect::Simple).expect("query");
        let (ir, view) = parse_query_text(query, IrDialect::Og, JournalDate::today());
        run(graph, &ir, &view);
    }
}

fn run(graph: &WholeGraph, query: &tine_core::query::ir::Query, view: &ViewSettings) {
    match graph.query_ir(IrRequest::Run {
        query,
        view,
        context: &ExecutionContext::none(),
    }) {
        Ok(IrAnswer::Result(_)) => {}
        other => panic!("{other:?}"),
    }
}

struct Probe {
    save_bytes: u64,
    save: Counts,
    query_bytes: u64,
    query: Counts,
}

fn probe(pages: usize, blocks: usize, memo: bool) -> Probe {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-query-unit-cost-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    for index in 0..pages {
        let body = if index == 0 {
            "- before\n".repeat(blocks)
        } else {
            format!(
                "- TODO unrelated {index}\n  status:: s{}\n- see [[Page{:04}]]\n",
                index % 5,
                (index * 7) % pages
            )
        };
        fs::write(root.join("pages").join(format!("Page{index:04}.md")), body).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from("pages/Page0000.md".to_string());
    warm(&store.whole_graph().unwrap(), memo);
    // A first edit settles one-time work (the seed's first patch), so the
    // measured edit is the steady state.
    for text in ["settle", "after"] {
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = text.into();
        let (outcome, save_bytes, save) = measure(|| {
            store.save(
                tine_store::EditKind::ReplacePage,
                &id,
                SaveBase::Existing(read.rev),
                &doc,
            )
        });
        assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
        let graph = store.whole_graph().unwrap();
        let ((), query_bytes, query) = measure(|| after_edit(&graph));
        if text == "after" {
            store.close();
            let _ = fs::remove_dir_all(&root);
            return Probe {
                save_bytes,
                save,
                query_bytes,
                query,
            };
        }
    }
    unreachable!()
}

#[test]
fn a_query_side_edit_cost_does_not_grow_with_the_graph() {
    let _case = CASE_LOCK.lock().unwrap();
    for blocks in [1, 60] {
        let small = probe(20, blocks, true);
        let large = probe(10_000, blocks, true);
        let bare = probe(10_000, blocks, false);
        for (pages, memo, p) in [
            (20, true, &small),
            (10_000, true, &large),
            (10_000, false, &bare),
        ] {
            eprintln!(
                "I-25 query unit cost: blocks={blocks} pages={pages} memo={memo} save_bytes={} \
                 query_bytes={} facts_copies={} facts_derived={} carry_block_probes={} \
                 memo_page_probes={}",
                p.save_bytes,
                p.query_bytes,
                p.save.query_facts_copies + p.query.query_facts_copies,
                p.save.query_facts_derived + p.query.query_facts_derived,
                p.save.query_carry_block_probes + p.query.query_carry_block_probes,
                p.save.memo_page_probes,
            );
        }
        let facts_copies = large.save.query_facts_copies + large.query.query_facts_copies;
        assert!(
            facts_copies <= 256,
            "I-25: an edit copied {facts_copies} query-index facts entries on a 10k graph; \
             exemplar query/index.rs QueryIndex::patched (shared base + bounded delta)"
        );
        let derived = large.save.query_facts_derived + large.query.query_facts_derived;
        assert!(
            derived <= 4,
            "I-25: an edit re-derived {derived} pages' query facts; only the edited page may be"
        );
        let probes = large.save.query_carry_block_probes + large.query.query_carry_block_probes;
        assert!(
            probes as usize <= 2 * blocks * 2 * QUERIES.len() * 2,
            "I-25: memo carry evaluated {probes} blocks; only the edited page's blocks may be"
        );
        // A populated memo is carried across the edit by changed-page work
        // only: on a 10k graph it may add at most a small constant to the
        // save's bytes over a registry-only warm (the rest of the save's
        // bytes are the snapshot publication, which this lane does not own).
        assert!(
            large.save_bytes <= bare.save_bytes + 64 * 1024,
            "I-25: carrying the query memo across one edit cost {} bytes on a 10k graph \
             (memo) vs {} (registry only); exemplar query/memo.rs Entry::pages",
            large.save_bytes,
            bare.save_bytes
        );
        // Bytes on the query side follow the edited page, not the graph: the
        // 10k graph may cost at most a small constant more than 20 pages.
        assert!(
            large.query_bytes <= small.query_bytes * 2 + 64 * 1024,
            "I-25: query-side bytes per edit grew with the graph: {} (20 pages) → {} (10k pages)",
            small.query_bytes,
            large.query_bytes
        );
    }
}
