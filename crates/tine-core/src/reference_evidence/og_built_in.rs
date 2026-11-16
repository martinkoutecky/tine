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
