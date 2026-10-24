//! OG 0.10.9 graph-parser/property.cljs `hidden-built-in-properties`, plus
//! extensions/srs.cljs's six registered keys. Editor and renderers share this
//! list; render-only exceptions belong to their surface predicate.
pub const BUILTIN_HIDDEN: &[&str] = &[
    "id",
    "custom-id",
    "background-color",
    "heading",
    "collapsed",
    "created-at",
    "updated-at",
    "last-modified-at",
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

/// These keep dedicated controls in Tine's properties form. The remaining
/// hidden built-ins still have that form as their edit/remove surface.
pub const FORM_MANAGED: &[&str] = &["id", "collapsed", "logseq.order-list-type"];

/// OG lists legacy underscore spellings explicitly; Tine keeps its existing
/// comparison contract instead, sharing normalization across all surfaces.
pub fn normalize(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}
