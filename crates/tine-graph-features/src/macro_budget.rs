//! Per-tree budget for the export's expanding macros (`{{embed}}`, `{{query}}`).
//! The depth cap (4) bounds nesting, not fan-out: a synced or imported block
//! holding N copies of `{{embed ((its-own-id))}}` rendered N^4 subtrees into
//! one String during print/publish (og C3 L04, I-22). Each top-level macro
//! (depth 0) opens a tree; past the budget the tree is cut with a visible
//! marker. Same expansion count as the app's `src/render/expansionBudget.ts`;
//! the byte bound here is on rendered HTML.

use std::cell::Cell;

const MAX_MACRO_EXPANSIONS: usize = 1000;
const MAX_MACRO_EXPANDED_BYTES: usize = 16 * 1024 * 1024;

thread_local! {
    /// (expansions, rendered bytes) of the tree being expanded on this thread.
    static BUDGET: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// Run `render` as one expansion of macro `name` at nesting `depth`, or return
/// the limit marker once the current tree's budget is spent.
pub(crate) fn within(name: &str, depth: u8, render: impl FnOnce() -> String) -> String {
    if depth == 0 {
        BUDGET.set((0, 0));
    }
    let (expansions, bytes) = BUDGET.get();
    if expansions >= MAX_MACRO_EXPANSIONS || bytes > MAX_MACRO_EXPANDED_BYTES {
        return format!(
            "<span class=\"macro-raw macro-expansion-limit\">{{{{{} …}}}} (macro expansion limit)</span>",
            crate::render::esc(name)
        );
    }
    BUDGET.set((expansions + 1, bytes));
    let out = render();
    let (expansions, bytes) = BUDGET.get();
    BUDGET.set((expansions, bytes + out.len()));
    out
}
