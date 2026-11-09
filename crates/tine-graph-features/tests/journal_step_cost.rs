//! Synthetic loaded-feed journey: bounded day work, unchanged day union and cursor.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};
use tine_core::date::JournalDate;
use tine_graph_features::journals::feed_page_through;
use tine_store::{cost_counters, Store};

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
struct CountingAllocator;
fn allocated() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocated();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc_zeroed(layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn scrolling_three_days_bounds_work_and_preserves_referenced_only_days() {
    measure(true);
}

#[test]
#[ignore = "before/after measurement without the candidate cost ceiling"]
fn measure_journal_scroll_work() {
    measure(false);
}

fn measure(enforce_cost: bool) {
    // The allocation count and journal work are per thread; the I/O snapshot is process-wide, so no other
    // case may run beside it (every case holds the shared lock; scripts/test-rust-integration-layout.mjs).
    let _guard = crate::exclusive_integration_lock();
    for journals in [300_i64, 3000] {
        // The timing guard uses a small reference fixture; the measurement
        // journey separately exercises R=J and its retained reference scan.
        for references in [0, if enforce_cost { 30 } else { journals }] {
            let root = tempfile::tempdir().unwrap();
            for dir in ["pages", "journals"] {
                std::fs::create_dir(root.path().join(dir)).unwrap();
            }
            let date = |n: i64| JournalDate::from_days(10_000_i64 + n);
            for n in 0..journals {
                let path = root
                    .path()
                    .join(format!("journals/{}.md", date(n * 2).file_stem()));
                std::fs::write(&path, "- synthetic body\n").unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(
                        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
                    ))
                    .unwrap();
            }
            if references > 0 {
                let body = (0..references)
                    .map(|n| {
                        format!(
                            "- [[{}]]\n",
                            date(journals - references + n * 2 + 1).file_stem()
                        )
                    })
                    .collect::<String>();
                let plan = root.path().join("pages/Plan.md");
                std::fs::write(&plan, body).unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(plan)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(
                        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
                    ))
                    .unwrap();
            }
            let store = Store::open(root.path(), Default::default()).unwrap().0;
            store.stop_watcher_for_tests();
            store.whole_graph().unwrap();
            let before = date(journals).ordinal_key();
            let warm = feed_page_through(&store, 3, Some(before), None, true).unwrap();
            let expected: Vec<_> = (0..journals * 2)
                .rev()
                .filter(|n| {
                    date(*n).ordinal_key() < before
                        && (n % 2 == 0
                            || (references > 0
                                && *n >= journals - references
                                && *n < journals + references))
                })
                .take(4)
                .map(|n| date(n).ordinal_key())
                .collect();
            let days = |page: &tine_graph_features::journals::FeedPage<
                tine_graph_features::journals::FeedRead,
            >| {
                page.pages
                    .iter()
                    .map(|p| {
                        tine_core::date::JournalFormat::new(None, None)
                            .parse(&p.doc.title)
                            .unwrap()
                            .ordinal_key()
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(days(&warm), expected[..3]);
            assert_eq!(warm.next_before_day, Some(expected[2]));
            assert!(!warm.done);
            let counts = cost_counters::journal_work();
            let io = cost_counters::snapshot();
            ALLOCATIONS.with(|count| count.set(Some(0)));
            let started = Instant::now();
            let page = feed_page_through(&store, 3, Some(before), None, true).unwrap();
            let elapsed = started.elapsed();
            let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
            let after = cost_counters::journal_work();
            let io_after = cost_counters::snapshot();
            let work = (after.0 - counts.0, after.1 - counts.1, after.2 - counts.2);
            println!("J={journals} R={references}: days={} sorts={} reference_names={} allocations={allocations} step={elapsed:?}", work.0, work.1, work.2);
            if !enforce_cost {
                let mut timings: Vec<_> = (0..9)
                    .map(|_| {
                        let started = Instant::now();
                        std::hint::black_box(
                            feed_page_through(&store, 3, Some(before), None, true).unwrap(),
                        );
                        started.elapsed()
                    })
                    .collect();
                timings.sort();
                let graph = store.whole_graph().unwrap();
                ALLOCATIONS.with(|count| count.set(Some(0)));
                let started = Instant::now();
                std::hint::black_box(
                    graph.inventory(tine_store::InventoryScope::ReferencedJournals),
                );
                let reference_time = started.elapsed();
                let reference_allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
                println!("J={journals} R={references}: median_step={:?} reference_inventory={reference_time:?} reference_allocations={reference_allocations}", timings[4]);
            }
            assert_eq!(days(&page), expected[..3]);
            if enforce_cost {
                assert!(
                    work.0 <= 4,
                    "three-day step walked unrelated journal days: {work:?}"
                );
                assert_eq!(work.1, 1, "only the retained reference inventory may sort");
            }
            assert_eq!(work.2, references as usize);
            assert_eq!(io_after.readdir - io.readdir, 0);
            assert_eq!(
                io_after.full_reads - io.full_reads,
                if references == 0 { 3 } else { 1 }
            );
            assert_eq!(io_after.files_written - io.files_written, 0);
            assert_eq!(io_after.fsyncs - io.fsyncs, 0);
            if enforce_cost && references == 0 {
                assert!(
                    allocations < 1000,
                    "step allocated with history size: {allocations}"
                );
            }
            if enforce_cost {
                assert!(
                    elapsed < Duration::from_millis(100),
                    "three-day step exceeded timing budget: {elapsed:?}"
                );
            }
            let extended =
                feed_page_through(&store, 1, Some(before), Some(expected[2]), true).unwrap();
            assert_eq!(days(&extended), expected[..3]);
            let zero = feed_page_through(&store, 0, Some(before), None, true).unwrap();
            assert!(zero.pages.is_empty() && !zero.done);
            let end =
                feed_page_through(&store, 3, Some(date(0).ordinal_key()), None, true).unwrap();
            assert!(end.pages.is_empty() && end.done);
            store.close();
        }
    }
}
