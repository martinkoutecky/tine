//! Test-only synthetic-graph probe; production never includes this allocator.
use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

#[derive(Clone, Copy, Default, Debug)]
struct Work {
    calls: usize,
    bytes: usize,
    live: i64,
    peak: i64,
}
thread_local! { static METER: Cell<Option<Work>> = const { Cell::new(None) }; }
struct Allocator;
fn note(allocated: usize, freed: usize) {
    let _ = METER.try_with(|meter| {
        if let Some(mut work) = meter.get() {
            work.calls += usize::from(allocated > 0);
            work.bytes += allocated;
            work.live += allocated as i64 - freed as i64;
            work.peak = work.peak.max(work.live);
            meter.set(Some(work));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size(), 0);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(0, layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note(size, layout.size());
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn measured<T>(f: impl FnOnce() -> T) -> (T, Work, f64, usize) {
    COMPLETION_SCORES.with(|n| n.set(0));
    METER.with(|meter| meter.set(Some(Work::default())));
    let started = Instant::now();
    let out = f();
    let ms = started.elapsed().as_secs_f64() * 1000.;
    let work = METER.with(|meter| meter.take().unwrap());
    (out, work, ms, COMPLETION_SCORES.with(|n| n.get()))
}

#[test]
#[ignore = "explicit D-20261102-PERF completion timing/allocation probe; synthetic graphs only"]
fn completion_before_after_probe() {
    let root = std::path::PathBuf::from(
        std::env::var("TINE_COMPLETION_BENCH_ROOT").expect("synthetic fixture root"),
    );
    for size in [1000, 10000, 30000] {
        let store = crate::Store::open(&root.join(size.to_string()), Default::default())
            .unwrap()
            .0;
        let graph = store.whole_graph().unwrap().test_read_snapshot();
        // Warm the existing name tables, independently of the new pool.
        completion_tests::oracle(&QueryPlan::legacy_page_search("Topic", 100), &graph);
        for repeat in 0..5 {
            graph.completion_rank.release(&None, None);
            for q in [
                "T",
                "To",
                "Top",
                "Topi",
                "Topic",
                "Topic 0",
                "Topic 00",
                "Topic 000",
            ] {
                let plan = QueryPlan::legacy_page_search(q, 100);
                let (before, old, old_ms, old_scans) =
                    measured(|| completion_tests::oracle(&plan, &graph));
                let (after, new, new_ms, new_scans) =
                    measured(|| graph.completion_rank.execute(&plan, &graph, None));
                assert_eq!(
                    completion_tests::fingerprint(before),
                    completion_tests::fingerprint(after)
                );
                println!(
                    "{}",
                    serde_json::json!({ "size": size, "repeat": repeat, "query": q,
                    "before": {"ms":old_ms,"names":old_scans,"allocations":old.calls,"bytes":old.bytes},
                    "after": {"ms":new_ms,"names":new_scans,"allocations":new.calls,"bytes":new.bytes,"live_delta":new.live,"peak":new.peak} })
                );
            }
        }
        graph.completion_rank.release(&None, None);
        let (_, memory, ms, _) = measured(|| {
            drop(graph.completion_rank.execute(
                &QueryPlan::legacy_page_search("", 100),
                &graph,
                None,
            ));
        });
        println!(
            "{}",
            serde_json::json!({"size":size,"resident_pool_bytes":memory.live,"build_ms":ms,"peak_bytes":memory.peak})
        );
        store.close();
    }
}
