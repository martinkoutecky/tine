//! The page-property header as parser-owned data, and the per-document block
//! reference counts that treat it as a referring block. Pure: no I/O, no
//! cache state; the store and every client ask these same functions.

use std::collections::HashMap;

use crate::doc::{DocBlock, Document};

#[cfg(test)]
mod empty_header_tests {
    use super::*;

    #[test]
    fn empty_org_page_drawers_are_metadata_and_keep_one_header_origin() {
        for ending in ["\n", "\r\n", "\r"] {
            for indent in ["", " \t", "\u{c}", "\u{1a}"] {
                let bytes = format!("{indent}:PROPERTIES:{ending}{indent}:END:{ending}");
                let doc = crate::org::parse_org(&bytes);
                assert!(page_document_is_org(&doc));
                let pre = doc.pre_block.as_deref().unwrap();
                let (header, prose) = pre_block_parts(pre, true);
                assert_eq!(header, ":PROPERTIES:\n:END:");
                assert!(prose.is_none(), "{bytes:?}");
                let rows = pre_block_projections(pre, true);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].0, PreBlockKind::Properties);
            }
        }
        let quote = "#+BEGIN_QUOTE\n:PROPERTIES:\n:END:\n#+END_QUOTE";
        let (header, prose) = pre_block_parts(quote, true);
        assert!(header.is_empty());
        assert!(prose.is_some(), "a quoted drawer remains quote content");
    }
}

pub use crate::page_header::page_property_lines;

/// The first root carries the format; an empty document's parser-owned Org
/// metadata distinguishes an Org drawer/directive preamble from Markdown.
pub fn page_document_is_org(doc: &Document) -> bool {
    doc.roots.first().map(DocBlock::is_org).unwrap_or_else(|| {
        doc.pre_block.as_deref().is_some_and(|pre| {
            let regions = crate::block_regions::parse_document(pre, true);
            regions.page_properties().next().is_some()
                || regions.find_org_preamble_property_region(pre).is_some()
        })
    })
}

/// Project only parser-owned page properties into native block syntax. Keeping
/// the whole Org drawer preserves parser ownership for reference evidence.
pub fn page_property_raw(pre: &str, is_org: bool) -> String {
    pre_block_parts(pre, is_org).0
}

fn raw_from_lines(entries: &[(String, String)], is_org: bool) -> String {
    if entries.is_empty() {
        return String::new();
    }
    if is_org {
        format!(
            ":PROPERTIES:\n{}\n:END:",
            entries
                .iter()
                .map(|(key, value)| format!(":{key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        entries
            .iter()
            .map(|(key, value)| format!("{key}:: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A block whose raw text is the projected page-property header.
pub fn property_projection(raw: &str, is_org: bool) -> DocBlock {
    let mut block = DocBlock::new(raw);
    block.set_org(is_org);
    block
}

/// The page-header block under the graph's property-reference policy.
pub fn property_projection_with_config(
    raw: &str,
    is_org: bool,
    config: &crate::query::atom::ParseConfig,
) -> DocBlock {
    let mut block = property_projection(raw, is_org);
    block.set_property_ref_config(config);
    block
}

/// The header pre-block as a block, from the document alone (no page entry).
/// I-12: the one projection of "the pre-block is a real block with `:block/refs`"
/// for the walkers that have only a `Document` (block-ref badge counts, scoped
/// referrer invalidation); the store's entry-taking `page_property_block` builds
/// the same block with a page-scoped identity for the DTO-producing walkers.
pub fn document_page_property_block(doc: &Document) -> Option<DocBlock> {
    let pre = doc.pre_block.as_deref()?;
    let is_org = page_document_is_org(doc);
    let raw = page_property_raw(pre, is_org);
    if raw.is_empty() {
        return None;
    }
    Some(property_projection(&raw, is_org))
}

/// Where the pre-block's prose is: the text before the page's first block that
/// is not its page properties, without the newlines at either edge.
///
/// OG makes everything before the first block one pre-block whose
/// `:block/refs` are its properties' and its body's (graph-parser
/// `block.cljs with-pre-block-if-exists`), for Markdown and Org alike, so a
/// link written there is a Linked Reference, counts, and is renamed
/// (`handler/page.cljs rename-update-refs!` walks `:block/_refs`). Tine keeps
/// the page properties as the header block ([`document_page_property_block`])
/// and reads this prose as the page's first block, which is also what the
/// page shows (`Page.tsx`) and what editing it promotes it to (GH #85). The
/// one answer (I-12; CM step 2, batch 8):
/// - Markdown: `tine_outline::page_preamble::page_preamble_content` (after the leading run
///   of page properties; the window asks the same function through wasm).
/// - Org: after the last line of the parser's page properties (directives and
///   the head drawer). Text between two directives is not read (recorded in
///   the batch 8 design note).
///
/// O(pre-block bytes); `None` when there is no prose.
pub fn preamble_content_range(pre: &str, is_org: bool) -> Option<std::ops::Range<usize>> {
    preamble_range_in(
        pre,
        is_org,
        &crate::block_regions::parse_document(pre, is_org),
    )
}

/// Both parts of a pre-block from one parse: the projected page-property
/// header text ([`page_property_raw`], empty when none) and the prose range
/// ([`preamble_content_range`]). For the per-page walkers (whole-graph build,
/// candidate signatures, reference questions), so the prose costs no second
/// parse of the pre-block.
pub fn pre_block_parts(pre: &str, is_org: bool) -> (String, Option<std::ops::Range<usize>>) {
    let regions = crate::block_regions::parse_document(pre, is_org);
    let mut raw = raw_from_lines(
        &crate::page_header::page_property_lines_in(&regions, is_org),
        is_org,
    );
    if raw.is_empty() && is_org && regions.find_org_preamble_property_region(pre).is_some() {
        raw = ":PROPERTIES:\n:END:".into();
    }
    (raw, preamble_range_in(pre, is_org, &regions))
}

fn preamble_range_in(
    pre: &str,
    is_org: bool,
    regions: &crate::block_regions::BlockRegions,
) -> Option<std::ops::Range<usize>> {
    tine_outline::page_preamble::preamble_content_range_in(pre, is_org, regions).map(|r| r.0..r.1)
}

/// The pre-block's prose as a block (see [`preamble_content_range`]): the
/// text a promoted preamble block would hold, so its reference answer is the
/// one that block will have.
pub fn preamble_block(pre: &str, is_org: bool) -> Option<DocBlock> {
    let range = preamble_content_range(pre, is_org)?;
    let mut block = DocBlock::new(&pre[range]);
    block.set_org(is_org);
    Some(block)
}

/// [`preamble_block`] under the graph's property-reference policy.
pub fn preamble_block_with_config(
    pre: &str,
    is_org: bool,
    config: &crate::query::atom::ParseConfig,
) -> Option<DocBlock> {
    let mut block = preamble_block(pre, is_org)?;
    block.set_property_ref_config(config);
    Some(block)
}

/// The pre-block's prose block from the document alone (no page entry); the
/// sibling of [`document_page_property_block`] for the same walkers.
pub fn document_preamble_block(doc: &Document) -> Option<DocBlock> {
    let pre = doc.pre_block.as_deref()?;
    preamble_block(pre, page_document_is_org(doc))
}

/// The pre-block's blocks every reference question reads, in page order:
/// the page-property header block, then the prose block.
pub fn document_pre_blocks(doc: &Document) -> impl Iterator<Item = DocBlock> {
    doc.pre_block
        .as_deref()
        .map_or_else(Vec::new, |pre| {
            pre_block_projections(pre, page_document_is_org(doc))
        })
        .into_iter()
        .map(|(_, block)| block)
}

/// Which parser-owned part of a page's pre-block a projection represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreBlockKind {
    /// The page-property header.
    Properties,
    /// Prose before the first outline block.
    Preamble,
}

/// Project both parts once, retaining their role for clients assigning result
/// identities. The document-only walker above drops the role and shares the
/// same parser boundary. Pure: no graph identities, I/O or persisted state.
pub fn pre_block_projections(pre: &str, is_org: bool) -> Vec<(PreBlockKind, DocBlock)> {
    let mut out = Vec::new();
    let (raw, prose) = pre_block_parts(pre, is_org);
    if !raw.is_empty() {
        out.push((PreBlockKind::Properties, property_projection(&raw, is_org)));
    }
    if let Some(range) = prose {
        let mut block = DocBlock::new(&pre[range]);
        block.set_org(is_org);
        out.push((PreBlockKind::Preamble, block));
    }
    out
}

/// Count each projected block reference once per referring block. The one
/// counter behind the graph's block-ref badges and a publication's counts.
/// Cost O(document blocks).
pub fn document_block_ref_counts(doc: &Document) -> HashMap<String, usize> {
    fn walk(blocks: &[DocBlock], counts: &mut HashMap<String, usize>) {
        for block in blocks {
            // projection().block_refs() is already de-duplicated per referrer block,
            // matching the badge's OG-compatible counting semantics.
            for id in block.projection().block_refs() {
                *counts.entry(id.clone()).or_insert(0) += 1;
            }
            walk(&block.children, counts);
        }
    }
    let mut counts = HashMap::new();
    // OG parity (#7): the header pre-block is a block with `:block/refs`, so a
    // `((uuid))` in a page property is one referrer of that block; so is one
    // in the preamble's prose (its own block, batch 8).
    for pre in document_pre_blocks(doc) {
        for id in pre.projection().block_refs() {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
    }
    walk(&doc.roots, &mut counts);
    counts
}

/// References a deletion leaves broken: the graph's referrers (`counts`, from
/// [`document_block_ref_counts`]) of the removed `id::`s, less those removed
/// with them. The one rule behind the app's reference notice and
/// `tine delete`.
pub fn refs_broken_by_removing(counts: &HashMap<String, usize>, removed: &[(&str, bool)]) -> usize {
    let refs = tine_outline::removal_refs::removal_refs(removed, true);
    let referrers: usize = refs
        .targets
        .iter()
        .map(|t| counts.get(t).copied().unwrap_or(0))
        .sum();
    referrers.saturating_sub(refs.removed_with)
}
