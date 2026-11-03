//! OG built-in property identity shared by reference evidence and browser tables.

use std::sync::OnceLock;
use tine_outline::editor_text::property_key_norm;

// Exact OG 1.0.0 property-page exclusions from
// `logseq.graph-parser.property/editable-built-in-properties` at 6e7afa8eb.
// Keep source spellings here; `property_key_norm` supplies Tine's canonical
// property identity (including underscore -> dash).
pub(super) const OG_EDITABLE_BUILT_IN_PROPERTIES: &[&str] = &[
    "title",
    "icon",
    "template",
    "template-including-parent",
    "public",
    "filters",
    "exclude-from-graph-view",
    "logseq.query/nlp-date",
    "macro",
    "filetags",
    "alias",
    "aliases",
    "tags",
    "logseq.color",
    "logseq.table.version",
    "logseq.table.compact",
    "logseq.table.headers",
    "logseq.table.hover",
    "logseq.table.borders",
    "logseq.table.stripes",
    "logseq.table.max-width",
];

// Exact base set from `hidden-built-in-properties`, plus the only registered
// extension set in that revision (`frontend.extensions.srs`).
pub(super) const OG_HIDDEN_BUILT_IN_PROPERTIES: &[&str] = &[
    "id",
    "custom-id",
    "background-color",
    "background_color",
    "heading",
    "collapsed",
    "created-at",
    "updated-at",
    "last-modified-at",
    "created_at",
    "last_modified_at",
    "query-table",
    "query-properties",
    "query-sort-by",
    "query-sort-desc",
    "ls-type",
    "hl-type",
    "hl-page",
    "hl-stamp",
    "hl-color",
    "logseq.macro-name",
    "logseq.macro-arguments",
    "logseq.order-list-type",
    "logseq.tldraw.page",
    "logseq.tldraw.shape",
    "todo",
    "doing",
    "now",
    "later",
    "done",
    "card-last-interval",
    "card-repeats",
    "card-last-reviewed",
    "card-next-schedule",
    "card-ease-factor",
    "card-last-score",
];

/// Whether a property key is in OG's editable or hidden built-in sets,
/// including registered SRS keys. O(key bytes), no I/O; the union is cached.
pub fn is_og_built_in_property(key: &str) -> bool {
    is_normalized_og_built_in_property(&property_key_norm(key))
}

pub(super) fn is_normalized_og_built_in_property(norm: &str) -> bool {
    static BUILT_IN: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    BUILT_IN
        .get_or_init(|| {
            OG_EDITABLE_BUILT_IN_PROPERTIES
                .iter()
                .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
                .map(|built_in| property_key_norm(built_in))
                .collect()
        })
        .contains(norm)
}

// Exact OG 1.0.0 `logseq.db.default/built-in-pages-names` at 6e7afa8eb
// (deps/db/src/logseq/db/default.cljs): the built-in markers, the priorities
// and three page names. `frontend.handler.page/get-all-pages` drops a page
// when `(built-in-pages-names (string/upper-case name))`, so only the
// all-capital members can ever match: Favorites, Contents and card stay
// listed in OG, and here (D-20261102-CM4 A11, follow OG).
const OG_BUILT_IN_PAGE_NAMES: &[&str] = &[
    "NOW",
    "LATER",
    "DOING",
    "DONE",
    "CANCELED",
    "CANCELLED",
    "IN-PROGRESS",
    "TODO",
    "WAIT",
    "WAITING",
    "A",
    "B",
    "C",
    "Favorites",
    "Contents",
    "card",
];

/// Whether All pages hides this page name as an OG built-in page
/// (`get-all-pages`' upper-case membership test). O(name bytes), no I/O.
pub fn is_og_built_in_page(name: &str) -> bool {
    let upper = name.to_uppercase();
    OG_BUILT_IN_PAGE_NAMES.contains(&upper.as_str())
}

/// Whether All pages leaves this page out: the one answer for its file rows
/// and its reference-only rows (I-12; the window asks it through wasm). OG
/// `frontend.handler.page/get-all-pages` (6e7afa8eb) removes a page whose
/// lower-cased `:block/name` is a UUID (`util/uuid-string?`, the
/// `exactly-uuid-pattern`), a drawing (`gp-config/draw?`: starts with
/// `default-draw-directory`), or an OG built-in page ([`is_og_built_in_page`]).
/// Intentional difference (CM step 2 batch 8): OG's `draw?` tests the bare
/// prefix `draws`, which also hides a page named `Drawstring`; here only the
/// drawing directory `draws/` is hidden. O(name bytes), no I/O.
pub fn hidden_from_all_pages(name: &str) -> bool {
    is_og_built_in_page(name) || is_og_uuid_string(name) || {
        let lower = name.to_lowercase();
        lower.starts_with("draws/")
    }
}

/// OG `util/uuid-string?`: `(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-5][0-9a-f]{3}-[089ab][0-9a-f]{3}-[0-9a-f]{12}$`.
fn is_og_uuid_string(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(at, &b)| match at {
        8 | 13 | 18 | 23 => b == b'-',
        14 => (b'0'..=b'5').contains(&b),
        19 => matches!(b.to_ascii_lowercase(), b'0' | b'8' | b'9' | b'a' | b'b'),
        _ => b.is_ascii_hexdigit(),
    })
}

#[cfg(test)]
mod built_in_page_tests {
    use super::{hidden_from_all_pages, is_og_built_in_page};

    #[test]
    fn all_pages_hides_uuid_names_and_drawings_as_og() {
        for name in [
            "6571a2b3-0000-4000-8000-00000000c8b8",
            "6571A2B3-0000-4000-B000-00000000C8B8",
            "draws/2026-11-03.excalidraw",
            "Draws/sketch",
            "TODO",
        ] {
            assert!(hidden_from_all_pages(name), "{name:?}");
        }
        for name in [
            // Version nibble 6 and variant c are outside OG's pattern.
            "6571a2b3-0000-6000-8000-00000000c8b8",
            "6571a2b3-0000-4000-c000-00000000c8b8",
            "6571a2b3-0000-4000-8000-00000000c8b8x",
            "draws",
            "Drawstring",
            "my draws/x",
            "Contents",
        ] {
            assert!(!hidden_from_all_pages(name), "{name:?}");
        }
    }

    #[test]
    fn og_upper_case_membership_hides_markers_and_priorities_only() {
        for name in ["TODO", "todo", "Doing", "in-progress", "a", "B", "WAIT"] {
            assert!(is_og_built_in_page(name), "{name:?}");
        }
        // OG upper-cases the name before the lookup, so its mixed-case
        // members never match; STARTED is a Tine marker OG does not list.
        for name in ["Favorites", "contents", "card", "STARTED", "D", "todo list"] {
            assert!(!is_og_built_in_page(name), "{name:?}");
        }
    }
}
