//! Render facts every renderer answers identically (I-12): which block
//! properties are chrome, and which checkbox a task marker draws. The live app (through the WASM bridge) and the static export ask
//! this one predicate, so a property chip hidden in the app is hidden in a
//! published page and the reverse.
//!
//! OG util/property.cljs `get-visible-ordered-properties`: built-ins shared
//! with the editor, plus exceptions for ordinary blocks and page pre-blocks.
//! Query filter availability remains a separate question.

/// Built-in keys that are never shown as a rendered chip (id/collapsed, Logseq
/// internals, display-only keys), already in `normalize` form.
pub use tine_outline::property_visibility::BUILTIN_HIDDEN as RENDER_HIDDEN;

/// A property key's comparison form: trimmed, ASCII-lowercased, spaces and
/// underscores as hyphens. Byte-for-byte `doc::property_key_norm`;
/// `tests/og_r_render_facets.rs` pins the two together because this file cannot
/// depend on `doc`.
pub fn normalize(key: &str) -> String {
    tine_outline::property_visibility::normalize(key)
}

/// Whether a block property key is hidden from the rendered chips: a built-in
/// internal key, a `tine.*` view setting, a known `logseq.table.*` setting (OG
/// resolves those from the block, `shui/table/v2.cljs`), or a key the graph
/// lists in `:block-hidden-properties`. Case- and separator-insensitive.
/// O(key bytes + hidden keys), no allocation beyond the normalized key.
pub fn is_render_hidden_prop(key: &str, user_hidden: &[String]) -> bool {
    let key = normalize(key);
    key.starts_with("tine.")
        || matches!(
            key.as_str(),
            "logseq.color"
                | "logseq.query/nlp-date"
                | "logseq.table.version"
                | "logseq.table.compact"
                | "logseq.table.headers"
                | "logseq.table.hover"
                | "logseq.table.borders"
                | "logseq.table.stripes"
                | "logseq.table.max-width"
        )
        || RENDER_HIDDEN.contains(&key.as_str())
        || user_hidden.iter().any(|hidden| normalize(hidden) == key)
}

/// Page pre-blocks hide title/icon/filters but show editable table/view keys.
/// Tine additionally surfaces aliases in AliasGroupHeader and hides its page
/// markers. Unlike OG's empty-properties-order fallthrough, filtering always
/// applies, including to properties created by external tools.
pub fn is_render_hidden_page_prop(key: &str, user_hidden: &[String]) -> bool {
    let key = normalize(key);
    RENDER_HIDDEN.contains(&key.as_str())
        || matches!(
            key.as_str(),
            "title" | "icon" | "filters" | "alias" | "tine.tag-table" | "tine/exports"
        )
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
