//! What a write allocates, counted by a global allocator of this test
//! binary: `apply` copies the output once, into a buffer of its exact length.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use tine_text::{apply, Format, PageText, Splice};

struct Counting;

thread_local! {
    /// While measuring: this thread's (allocations, reallocations).
    static COUNTS: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

fn count(realloc: bool) {
    let _ = COUNTS.try_with(|counts| {
        if let Some((allocs, reallocs)) = counts.get() {
            counts.set(Some((allocs + 1, reallocs + usize::from(realloc))));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(false);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(true);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Appending 1 MiB to a 1 MiB block, with a line break in the middle of the
/// appended text, in each line ending: no buffer is regrown, and the output
/// buffer is exactly the output's length.
#[test]
fn apply_writes_the_output_once_into_its_exact_length() {
    let mib = 1 << 20;
    let body = "a".repeat(mib);
    let mut appended = "b".repeat(mib / 2);
    appended.push('\n');
    appended += &"c".repeat(mib / 2);
    for ending in ["\n", "\r\n", "\r", ""] {
        let text = format!("- {body}{ending}");
        let page = PageText::read_text(&text, Format::Markdown).unwrap();
        let splice = Splice {
            slot: 0,
            range: mib..mib,
            text: &appended,
        };
        COUNTS.with(|c| c.set(Some((0, 0))));
        let out = apply(&page, &[splice]).unwrap();
        let (allocs, reallocs) = COUNTS.with(|c| c.replace(None).unwrap());
        // The break is written as the line's (CRLF or LF) and the `"  "` layout.
        let written = appended.len() + 2 + usize::from(ending == "\r\n");
        assert_eq!(out.len(), text.len() + written, "{ending:?}");
        assert_eq!(reallocs, 0, "{ending:?}: {allocs} allocations");
        assert_eq!(out.capacity(), out.len(), "{ending:?}");
        // The plan, its order, the slot's group list, its continuation layout,
        // and the output.
        assert!(allocs <= 6, "{ending:?}: {allocs} allocations");
    }
}
