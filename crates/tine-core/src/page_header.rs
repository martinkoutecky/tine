//! The page header: which text holds a page's own properties, its property
//! lines, and the aliases they declare. Pure text functions over
//! `tine-outline`'s parser, with no `Document`, I/O or cache, so the store and
//! the frontend's wasm build (`lsdoc-wasm` includes this file) ask the same
//! code (I-12, census Q7). `tine-store` `query::document_alias_spellings` and
//! the merge (`tine-graph-features` `pages/merge_payload.rs`) call these;
//! `lsdoc-wasm` `page_alias_spellings` serves the page header's alias list.

use tine_outline::editor_text::property_key_norm;

/// Properties that describe the block they sit on, never a page. A first
/// bullet carrying one is an outline block even when it has no text yet: an
/// empty numbered-list item is `logseq.order-list-type:: number` and nothing
/// else. OG keeps such a bullet a bullet because the property sits below an
/// empty title line, and its serializer promotes only a first block whose FIRST
/// line holds `:: ` (`file/core.cljs` `transform-content`); Tine's raw has no
/// empty title line, so the key itself must decide (GH #540). `id` marks a
/// referenced outline block. `tine.view` is a Tine table/grid/board block whose
/// title is typed after its view is chosen (GH #210): its rows are children and
/// its first line is text, so it is never page properties. Mirrored by
/// `BLOCK_SCOPED_PROPERTY_KEYS` in `src/document/convert.ts`; `src/i12PageHeader.test.ts` keeps them equal
/// (it reads this file).
pub const BLOCK_SCOPED_PROPERTY_KEYS: &[&str] = &[
    "id",
    "heading",
    "collapsed",
    "background-color",
    "logseq.order-list-type",
    "tine.view",
    "tine.header",
    "tine.col-widths",
    "tine.col-aggregates",
    "tine.group-by",
    "tine.group-field",
    "tine.table-widths",
    "tine.columns",
    "tine.block-columns",
    "tine.filter",
    "tine.sort",
    "tine.sample",
    "tine.result-kinds",
];

/// The ONE predicate for "this first root's raw is a page header" (TS twin:
/// `isPropertiesOnly`, `src/editor/properties.ts`): canonical page-header
/// properties (the parser's `page_header_only`, I-12) and no block-scoped key.
/// Save-time promotion and alias reading both call it, so a block cannot be
/// promoted/read as the header in one place and kept an outline block in
/// another. O(raw bytes).
pub fn is_page_header_candidate(raw: &str) -> bool {
    tine_outline::block_regions::page_header_only(raw).is_some_and(|header| {
        !header.entries.iter().any(|entry| {
            BLOCK_SCOPED_PROPERTY_KEYS
                .iter()
                .any(|scoped| entry.key.eq_ignore_ascii_case(scoped))
        })
    })
}

/// The text a page's own properties, aliases included, are read from: the
/// pre-block when the page has one, else a properties-only FIRST root (the
/// page-properties block in OG, written back as a pre-block on save there;
/// GH #62), else none. The one header rule of the alias answer
/// ([`page_alias_spellings`]); a merge reads both pages' aliases and writes the
/// united header through it (REG-CM-MERGE-LOSES-ALIASES).
pub fn page_header_text<'a>(
    pre_block: Option<&'a str>,
    first_root: Option<&'a str>,
) -> Option<&'a str> {
    match pre_block {
        Some(pre) => Some(pre),
        // A first root's trailing line breaks are not part of its header.
        None => {
            first_root.filter(|raw| is_page_header_candidate(raw.trim_end_matches(['\n', '\r'])))
        }
    }
}

/// True for a page-header property key that declares aliases: `alias` and
/// `aliases`, in any case. OG reads only `alias::` as an alias
/// (`graph_parser/extract.cljc`, `(:alias properties)`) and `aliases::` only as
/// a reference-valued key; Tine and the conceptual model read both (named OG
/// difference, D-20261018-M2, K-Q1 2026-11-03).
pub fn is_alias_key(key: &str) -> bool {
    let key = property_key_norm(key);
    key == "alias" || key == "aliases"
}

/// Page properties owned by lsdoc, including Org directives and head drawers.
/// O(input bytes + AST nodes), without I/O; literal/prose entries are excluded.
pub fn page_property_lines(text: &str, is_org: bool) -> Vec<(String, String)> {
    tine_outline::block_regions::parse_document(text, is_org)
        .page_properties()
        .map(|p| {
            (
                if is_org {
                    p.key.to_ascii_lowercase()
                } else {
                    p.key.clone()
                },
                p.value.clone(),
            )
        })
        .collect()
}

/// The alias spellings one page header declares, as authored: every
/// [`is_alias_key`] line's value through `property_refs::linkable_names`, a
/// wholly quoted value skipped (literal text), one spelling per page identity
/// (the sorted-first). O(header bytes).
pub fn header_alias_spellings(text: &str, is_org: bool) -> Vec<String> {
    let mut aliases = Vec::new();
    for (key, value) in page_property_lines(text, is_org) {
        if !is_alias_key(&key) {
            continue;
        }
        let trimmed = value.trim();
        if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
            continue;
        }
        aliases.extend(tine_outline::property_refs::linkable_names(&value, is_org));
    }
    aliases.sort_unstable();
    let mut seen = std::collections::HashSet::new();
    aliases.retain(|alias| seen.insert(crate::page_identity::page_key(alias)));
    aliases
}

/// The alias spellings a page declares: [`header_alias_spellings`] of its
/// [`page_header_text`]. Empty when the page has no header.
pub fn page_alias_spellings(
    pre_block: Option<&str>,
    first_root: Option<&str>,
    is_org: bool,
) -> Vec<String> {
    page_header_text(pre_block, first_root)
        .map(|text| header_alias_spellings(text, is_org))
        .unwrap_or_default()
}
