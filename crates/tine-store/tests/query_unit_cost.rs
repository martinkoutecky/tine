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
            note_own(layout.size() as u64);
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
            note_own(new_size.saturating_sub(layout.size()) as u64);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// Bytes allocated by the measuring thread itself, to attribute the process-wide
// count: the rest comes from concurrent threads (the `tine-graph-load` warm
// cache, the watcher), which are not the measured operation's work.
thread_local! {
    static MEASURER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static OWN: AtomicU64 = AtomicU64::new(0);

fn note_own(bytes: u64) {
    if MEASURER.try_with(|m| m.get()).unwrap_or(false) {
        OWN.fetch_add(bytes, Ordering::Relaxed);
    }
}

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
    /// Whether the registry the edit left behind has a row for `rare`.
    rare_row: bool,
    save_bytes: u64,
    save: Counts,
    query_bytes: u64,
    query: Counts,
}

fn probe(pages: usize, blocks: usize, memo: bool) -> Probe {
    probe_edit(pages, blocks, memo, ["settle", "after"])
}

/// `edits` are the two texts written to the first block of `Page0000`: the
/// first settles one-time work, the second is measured.
fn probe_edit(pages: usize, blocks: usize, memo: bool, edits: [&str; 2]) -> Probe {
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
    for text in edits {
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
        if text == edits[1] {
            let rare_row = match graph.query_ir(IrRequest::Registry) {
                Ok(IrAnswer::Registry(registry)) => registry.row("rare").is_some(),
                _ => panic!("registry"),
            };
            store.close();
            let _ = fs::remove_dir_all(&root);
            return Probe {
                rare_row,
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
    let _case = CASE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// A property edit moves the registry, and the registry patch is per key:
/// changing a key held by one page reads that page, not the graph (the same
/// probe on a text edit reads no page at all).
#[test]
fn a_property_edit_patches_the_registry_per_page_not_per_graph() {
    let _case = CASE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for blocks in [1, 60] {
        let edits = ["before\nrare:: v1", "before\nrare:: v2"];
        let small = probe_edit(20, blocks, true, edits);
        let large = probe_edit(10_000, blocks, true, edits);
        let text = probe(10_000, blocks, true);
        for (pages, p) in [(20, &small), (10_000, &large), (10_000, &text)] {
            eprintln!(
                "I-25 registry patch: blocks={blocks} pages={pages} registry_pages_read={} \
                 query_bytes={}",
                p.save.query_registry_pages_read + p.query.query_registry_pages_read,
                p.query_bytes,
            );
        }
        assert!(
            small.rare_row && large.rare_row,
            "the patched registry lost the edited key"
        );
        let read = |p: &Probe| p.save.query_registry_pages_read + p.query.query_registry_pages_read;
        assert!(
            read(&large) <= 2,
            "I-25: a rare-property edit read {} pages' rows for the registry on a 10k graph; \
             exemplar query/index.rs QueryIndex::patched_registry (per-key patch)",
            read(&large)
        );
        assert_eq!(read(&text), 0, "a text-only edit read registry rows");
        assert!(
            large.query_bytes <= small.query_bytes * 2 + 64 * 1024,
            "I-25: registry patch bytes grew with the graph: {} (20 pages) -> {} (10k pages)",
            small.query_bytes,
            large.query_bytes
        );
    }
}

/// An edit that makes the page a holder of a key every other page already
/// holds moves one posting of that key. The posting sets are shared
/// structure, so the edit copies O(log holders) tree nodes, never every
/// holder's path (checkpoint-5 L02 B3, I-25; exemplar model/persistent.rs).
/// `status::` is held by all 9,999 other pages of this fixture.
#[test]
fn a_popular_property_edit_does_not_copy_the_key_postings() {
    let _case = CASE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for blocks in [1, 60] {
        let edits = ["settle", "before\nstatus:: s1"];
        let small = probe_edit(20, blocks, true, edits);
        let large = probe_edit(10_000, blocks, true, edits);
        let copies = |p: &Probe| p.save.query_facts_copies + p.query.query_facts_copies;
        eprintln!(
            "I-25 popular key: blocks={blocks} facts_copies 20 pages={} 10k pages={}; tree nodes {} / {}",
            copies(&small),
            copies(&large),
            small.save.shared_tree_node_copies + small.query.shared_tree_node_copies,
            large.save.shared_tree_node_copies + large.query.shared_tree_node_copies
        );
        assert!(
            copies(&large) <= 256,
            "I-25: adding a page to a 10k-holder property key copied {} index entries; \
             exemplar model/persistent.rs (shared postings, no per-edit set copy)",
            copies(&large)
        );
        let nodes = large.save.shared_tree_node_copies + large.query.shared_tree_node_copies;
        assert!(
            nodes <= 512,
            "I-25: the same edit copied {nodes} shared-tree nodes on a 10k graph; \
             O(log holders) is a few dozen"
        );
    }
}

/// The graph-wide alias list is carried across a save that moved no alias by
/// reference count, not by deep copy (checkpoint-5 L02 B3, I-25): with the
/// list built, a text edit allocates the same on a 10k-page graph whether or
/// not 10,000 aliases exist to carry (the control spells the same text under
/// a key that declares none).
#[test]
fn a_save_that_moves_no_alias_does_not_copy_the_alias_list() {
    let _case = CASE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    fn save_bytes(aliases: bool) -> (u64, u64) {
        static NEXT: AtomicU64 = AtomicU64::new(2_000_000);
        let root = std::env::temp_dir().join(format!(
            "tine-query-alias-carry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        for index in 0..10_000 {
            // The control carries the same text under a key that declares no
            // alias, so page-reference work is equal and only the alias list
            // differs.
            let key = if aliases { "alias" } else { "other" };
            let body = format!("{key}:: Alias-number-{index}\n\n- block {index}\n");
            let path = root.join("pages").join(format!("Page{index:05}.md"));
            fs::write(&path, body).unwrap();
            // Outside the 2 s racy window (§5.4): otherwise the watcher's racy
            // follow-up re-hashes all 10k fresh files on another thread inside a
            // measured save (the 1-6.5 MB Windows noise). Exemplar:
            // launch_one_read.rs `backdate`.
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
                .unwrap();
        }
        let store = Store::open(&root, Default::default()).unwrap().0;
        let id = PageId::from("pages/Page00000.md".to_string());
        // Build the alias list of the current generation.
        store.whole_graph().unwrap().backlinks("Page00003").unwrap();
        // Concurrent background threads allocate into the process-wide count
        // at random (Windows CI saw the same save measure 0.3 MB and 1.5 MB).
        // That noise only adds, while a real alias-list copy recurs in every
        // save, so the minimum over several saves is the save's own cost.
        let mut measured = u64::MAX;
        let mut own = u64::MAX;
        for (index, text) in ["settle", "a", "b", "c", "d", "e"].into_iter().enumerate() {
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = text.into();
            OWN.store(0, Ordering::Relaxed);
            MEASURER.with(|m| m.set(true));
            let (outcome, bytes, _) = measure(|| {
                store.save(
                    tine_store::EditKind::ReplacePage,
                    &id,
                    SaveBase::Existing(read.rev),
                    &doc,
                )
            });
            MEASURER.with(|m| m.set(false));
            let own_bytes = OWN.load(Ordering::Relaxed);
            assert!(matches!(outcome, SaveOutcome::Saved(_)), "{outcome:?}");
            eprintln!(
                "I-25 alias carry sample aliases={aliases} save={index}: process={bytes} \
                 measuring-thread={own_bytes} other-threads={}",
                bytes - own_bytes.min(bytes)
            );
            // Re-read so the next generation's list is built before the
            // measured save carries it.
            store.whole_graph().unwrap().backlinks("Page00003").unwrap();
            if index > 0 {
                measured = measured.min(bytes);
                own = own.min(own_bytes);
            }
        }
        store.close();
        let _ = fs::remove_dir_all(&root);
        (measured, own)
    }
    save_bytes(false); // one-time process work, not the edit's
    let (without, without_own) = save_bytes(false);
    let (with, with_own) = save_bytes(true);
    eprintln!(
        "I-25 alias carry: save bytes without aliases={without} with 10k aliases={with} \
         (measuring thread: {without_own} / {with_own})"
    );
    assert!(
        with <= without + 256 * 1024,
        "I-25: a save carrying a 10k-alias list allocated {with} bytes vs {without} without \
         aliases; exemplar model.rs carry_alias_list_from (Arc, not clone)"
    );
}
