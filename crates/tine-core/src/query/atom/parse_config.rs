//! The parse-relevant config slice and the two key tests the reference
//! walker reads (moved out of `query/atom.rs` unchanged, conceptual model
//! step 2 batch 6). This file also compiles into lsdoc-wasm
//! (`crates/lsdoc-wasm/src/lib.rs`); `property_key_norm` comes through
//! `super::`, which `query::atom` and the wasm crate root both provide.

use super::property_key_norm;

/// The parse-relevant slice of the graph config the atomizer and registry
/// read (master `config::ParseConfig`, SPEC §5.8), filled from `config.edn`
/// by [`ParseConfig::from_config`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParseConfig {
    pub separated_by_commas: Vec<String>,
    pub ignored_page_references_keywords: Vec<String>,
    pub hidden_properties: Vec<String>,
    pub journal_page_title_format: Option<String>,
    pub journal_file_name_format: Option<String>,
}

impl ParseConfig {
    pub(crate) fn default_ref() -> &'static Self {
        static DEFAULT: std::sync::OnceLock<ParseConfig> = std::sync::OnceLock::new();
        DEFAULT.get_or_init(Self::default)
    }
}

/// OG `gp-property/unparsed-built-in-properties` (`property.cljs:110-121`):
/// `(hidden-built-in ∪ editable-built-in) − built-in-extended(∅) −
/// editable-linkable − keys(built-in-property-types)`, expanded here because
/// Tine has no Clojure set algebra at runtime. Names are `(name k)` of the
/// keyword, i.e. what a `key::` line spells.
const UNPARSED_BUILT_IN_PROPERTIES: &[&str] = &[
    // from hidden-built-in-properties (`property.cljs:68-79`)
    "id",
    "custom-id",
    "background-color",
    "background_color",
    "query-properties",
    "query-sort-by",
    "ls-type",
    "hl-type",
    "hl-color",
    "logseq.macro-name",
    "logseq.macro-arguments",
    "logseq.order-list-type",
    "logseq.tldraw.page",
    "logseq.tldraw.shape",
    // from editable-built-in-properties (`property.cljs:58-66`)
    "title",
    "icon",
    "template",
    "filters",
    "macro",
    "filetags",
    "logseq.color",
    "logseq.table.version",
    "logseq.table.compact",
    "logseq.table.headers",
    "logseq.table.hover",
    "logseq.table.borders",
    "logseq.table.stripes",
    "logseq.table.max-width",
];

/// OG `gp-property/editable-linkable-built-in-properties` (`property.cljs:46-48`).
pub(super) const EDITABLE_LINKABLE_BUILT_IN_PROPERTIES: &[&str] = &["alias", "aliases", "tags"];

fn contains_key(list: &[&str], key: &str) -> bool {
    list.iter()
        .any(|candidate| property_key_norm(candidate) == key)
}

fn contains_config_key(list: &[String], key: &str) -> bool {
    list.iter()
        .any(|candidate| property_key_norm(candidate.trim_start_matches(':')) == key)
}

/// OG `separated-by-commas?` (`text.cljs:141-146`).
pub(crate) fn separated_by_commas(key: &str, config: &ParseConfig) -> bool {
    contains_key(EDITABLE_LINKABLE_BUILT_IN_PROPERTIES, key)
        || contains_config_key(&config.separated_by_commas, key)
}

/// Step 1's key test: OG `parse-property`'s first `cond` branch
/// (`text.cljs:169-176`).
pub(crate) fn reference_parsing_suppressed(key: &str, config: &ParseConfig) -> bool {
    contains_key(UNPARSED_BUILT_IN_PROPERTIES, key)
        || contains_config_key(&config.ignored_page_references_keywords, key)
}
