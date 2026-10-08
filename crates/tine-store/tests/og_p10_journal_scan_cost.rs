//! I-25: a live journal inventory validates paths without allocating directory
//! prefixes or absolute copies for every eligibility check.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fs;
use tine_store::{Area, OpenOptions, Store, WatchMode};

struct Counting;
thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
fn count() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn journal_inventory_keeps_path_validation_allocation_bounded() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("journals")).unwrap();
    for index in 0..300 {
        let date = tine_core::date::JournalDate::from_days(20_000 + index);
        fs::write(
            root.path()
                .join("journals")
                .join(format!("{}.md", date.file_stem())),
            "- body\n",
        )
        .unwrap();
    }
    let store = Store::open(
        root.path(),
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    ALLOCATIONS.with(|count| count.set(Some(0)));
    let listing = store.scan_area(Area::Journals, None).unwrap();
    let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
    assert_eq!(listing.files.len(), 300);
    assert!(listing.unreadable.is_empty());
    assert!(listing
        .files
        .iter()
        .all(|file| file.page.is_some() && file.day.is_some()));
    assert!(allocations <= 24 * listing.files.len() + 64,
        "I-25: journal inventory reuses configured path prefixes and relative eligibility; exemplar store/page_identity.rs: {allocations} allocations for {} files", listing.files.len());
    store.close();
}
