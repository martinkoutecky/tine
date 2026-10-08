//! Linux paired save/rename probe. Run on a disposable COPY of a generator graph.
//! Saves run before rename scans; polling keeps notification bursts out of the
//! timed foreground operation. Allocations are counted on the caller's thread.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tine_store::{EditKind, OpenOptions, PageId, SaveBase, SaveOutcome, Store, WatchMode};

struct Counting;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
}
fn count(bytes: usize) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = BYTES.try_with(|v| v.set(v.get() + bytes as u64));
        let _ = CALLS.try_with(|v| v.set(v.get() + 1));
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn proc_number(path: &str, key: &str) -> u64 {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
fn cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
            0
        );
        time.tv_sec as f64 + time.tv_nsec as f64 / 1e9
    }
    #[cfg(not(target_os = "linux"))]
    panic!("save_rename_cost uses Linux /proc and thread CPU clocks");
}
fn measure(label: &str, f: impl FnOnce()) {
    let read = proc_number("/proc/self/io", "rchar:");
    #[cfg(feature = "test-faults")]
    tine_store::cost_counters::reset();
    BYTES.with(|v| v.set(0));
    CALLS.with(|v| v.set(0));
    ACTIVE.with(|v| v.set(true));
    let cpu = cpu_seconds();
    let start = Instant::now();
    f();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let cpu_ms = (cpu_seconds() - cpu) * 1000.0;
    ACTIVE.with(|v| v.set(false));
    let bytes = BYTES.with(Cell::get);
    let calls = CALLS.with(Cell::get);
    let reads = proc_number("/proc/self/io", "rchar:") - read;
    let rss = proc_number("/proc/self/status", "VmRSS:");
    println!("{label} ms={ms:.3} cpu_ms={cpu_ms:.3} allocated_bytes={bytes} allocations={calls} read_bytes={reads} rss_kib={rss}");
    #[cfg(feature = "test-faults")]
    println!("{label} {:?}", tine_store::cost_counters::snapshot());
}

fn main() {
    let root = std::env::args().nth(1).expect("disposable graph path");
    let root = Path::new(&root);
    for blocks in [1, 60] {
        fs::write(
            root.join(format!("pages/R5Cost{blocks}.md")),
            "- before\n".repeat(blocks),
        )
        .unwrap();
    }
    let store = Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let held = store.whole_graph().unwrap();
    for blocks in [1, 60] {
        for round in 0..100 {
            let id = PageId::from(format!("pages/R5Cost{blocks}.md"));
            let read = store.page(&id).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("after {round}");
            measure(&format!("save-{blocks}-{round}"), || {
                assert!(matches!(
                    store.save(
                        EditKind::ReplacePage,
                        &id,
                        SaveBase::Existing(read.rev),
                        &doc
                    ),
                    SaveOutcome::Saved(_)
                ));
            });
        }
    }
    for round in 0..5 {
        measure(&format!("rename-{round}"), || {
            tine_graph_features::pages::rename_page_expected(&store, "R5Cost1", "R5Renamed", None)
                .unwrap();
        });
        tine_graph_features::pages::rename_page_expected(&store, "R5Renamed", "R5Cost1", None)
            .unwrap();
    }
    std::hint::black_box(held);
    store.close();
}
