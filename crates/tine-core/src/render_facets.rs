//! Render facts every renderer answers identically (I-12): which block
//! properties are chrome, and which checkbox a task marker draws. The live app (through the WASM bridge) and the static export ask
//! this one predicate, so a property chip hidden in the app is hidden in a
//! published page and the reverse.
//!
//! Deliberately SEPARATE concepts (do not merge): the editor's textarea hide
//! list (`BUILTIN_HIDDEN` in `src/editor/properties.ts`), the query filter
//! blacklist (`INTERNAL_PROPS` in `query.rs`), and the page-property area
//! (`PAGE_PROPS_HIDDEN` in `components/Page.tsx`).
//!
//! Dependency-free on purpose: `lsdoc-wasm` includes this file by path.

/// Built-in keys that are never shown as a rendered chip (id/collapsed, Logseq
/// internals, display-only keys), already in `normalize` form.
pub const RENDER_HIDDEN: &[&str] = &[
    "id",
    "collapsed",
    "hl-page",
    "hl-color",
    "hl-type",
    "ls-type",
    "background-color",
    "logseq.order-list-type",
    "heading",
    "title",
    "filters",
    "created-at",
    "updated-at",
    "last-modified-at",
    "query-table",
    "query-properties",
    "query-sort-by",
    "query-sort-desc",
    "logseq.tldraw.shape",
];

/// A property key's comparison form: trimmed, ASCII-lowercased, spaces and
/// underscores as hyphens. Byte-for-byte `doc::property_key_norm`;
/// `tests/og_r_render_facets.rs` pins the two together because this file cannot
/// depend on `doc`.
pub fn normalize(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

/// Whether a block property key is hidden from the rendered chips: a built-in
/// internal key, a `tine.*` view setting, a `logseq.table.*` table setting (OG
/// resolves those from the block, `shui/table/v2.cljs`), or a key the graph
/// lists in `:block-hidden-properties`. Case- and separator-insensitive.
/// O(key bytes + hidden keys), no allocation beyond the normalized key.
pub fn is_render_hidden_prop(key: &str, user_hidden: &[String]) -> bool {
    let key = normalize(key);
    key.starts_with("tine.")
        || key.starts_with("logseq.table.")
        || RENDER_HIDDEN.contains(&key.as_str())
        || user_hidden.iter().any(|hidden| normalize(hidden) == key)
}

/// The checkbox a task marker draws: `Some(true)` checked (`DONE`),
/// `Some(false)` an empty box (every open marker), `None` no box (`CANCELED`,
/// `CANCELLED`, anything that is not a task marker). OG `block-checkbox`.
/// `tests/og_r_render_facets.rs` pins the marker set to `doc::MARKERS` and the
/// app's `taskCheckboxState` (`src/markers.ts`) tests assert the same table.
pub fn task_checkbox_state(marker: &str) -> Option<bool> {
    match marker {
        "DONE" => Some(true),
        "TODO" | "DOING" | "NOW" | "LATER" | "WAITING" | "WAIT" | "STARTED" | "IN-PROGRESS" => {
            Some(false)
        }
        _ => None,
    }
}
