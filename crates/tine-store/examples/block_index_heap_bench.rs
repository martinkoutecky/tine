//! Linux heap probe; run on a graph copy under the campaign's 2 GiB cap.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; }
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static USABLE: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counted;
unsafe fn usable_size(ptr: *mut u8, _layout: Layout) -> usize {
    #[cfg(target_os = "linux")]
    {
        libc::malloc_usable_size(ptr.cast())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ptr;
        _layout.size()
    }
}
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() && TRACK.try_with(Cell::get).unwrap_or(false) {
            REQUESTED.fetch_add(layout.size(), Relaxed);
            USABLE.fetch_add(usable_size(ptr, layout), Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            REQUESTED.fetch_sub(layout.size(), Relaxed);
            USABLE.fetch_sub(usable_size(ptr, layout), Relaxed);
            ALLOCS.fetch_sub(1, Relaxed);
        }
        System.dealloc(ptr, layout);
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;

fn main() {
    let root = std::env::args().nth(1).expect("graph copy");
    let (store, _, _) =
        tine_store::Store::open(std::path::Path::new(&root), Default::default()).unwrap();
    let view = store.whole_graph_reconciled().unwrap();
    let pages = view.corpus().pages.len();
    TRACK.set(true);
    let mut measurement = (0, 0, 0, 0);
    let start = std::time::Instant::now();
    view.with_rebuilt_block_index(|ids| {
        measurement = (
            ids,
            REQUESTED.load(Relaxed),
            USABLE.load(Relaxed),
            ALLOCS.load(Relaxed),
        );
    });
    let rebuild_ms = start.elapsed().as_secs_f64() * 1000.;
    TRACK.set(false);
    assert_eq!(
        REQUESTED.load(Relaxed),
        0,
        "the temporary index was released"
    );
    assert_eq!(USABLE.load(Relaxed), 0);
    assert_eq!(ALLOCS.load(Relaxed), 0);
    println!(
        "{}",
        serde_json::json!({"pages": pages, "ids": measurement.0,
        "requestedBytes": measurement.1, "allocatorUsableBytes": measurement.2,
        "liveAllocations": measurement.3, "rebuildAndDropMs": rebuild_ms})
    );
    store.close();
}
