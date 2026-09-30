//! I-25: compiled journal date formats parse without allocating per token.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use tine_core::date::{JournalDate, JournalFormat};

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
fn compiled_date_parse_allocates_only_its_input_character_buffer() {
    let default = JournalFormat::default();
    let custom = JournalFormat::new(Some("dd-MM-yyyy"), Some("MMM do, yyyy"));
    for (format, text, expected, limit) in [
        (&default, "2026_09_30", Some(20260930), 1),
        (&custom, "30-09-2026", Some(20260930), 1),
        (&custom, "sEp 30TH, 2026", Some(20260930), 2),
        (&custom, "Sep 31st, 2026", None, 4),
        (&custom, "2026_09_30", Some(20260930), 4),
    ] {
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let parsed = format.parse(text);
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(parsed.map(|date: JournalDate| date.ordinal_key()), expected);
        assert!(allocations <= limit,
            "I-25: the compiled date answerer must not allocate literal, digit or name copies; exemplar date.rs Format::parse: {text:?} used {allocations}, limit {limit}");
    }
}
