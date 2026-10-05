//! The page-property header as parser-owned data, and the per-document block
//! reference counts that treat it as a referring block. Pure: no I/O, no
//! cache state; the store and every client ask these same functions.

use std::collections::HashMap;

use crate::doc::{DocBlock, Document};

/// Page properties owned by lsdoc, including Org directives and head drawers.
/// O(input bytes + AST nodes), without I/O; literal/prose entries are excluded.
pub fn page_property_lines(text: &str, is_org: bool) -> Vec<(String, String)> {
    crate::block_regions::parse_document(text, is_org)
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

/// The first root carries the format; an empty document's parser-owned Org
/// metadata distinguishes an Org drawer/directive preamble from Markdown.
pub fn page_document_is_org(doc: &Document) -> bool {
    doc.roots.first().map(DocBlock::is_org).unwrap_or_else(|| {
        doc.pre_block.as_deref().is_some_and(|pre| {
            crate::block_regions::parse_document(pre, true)
                .page_properties()
                .next()
                .is_some()
        })
    })
}

/// Project only parser-owned page properties into native block syntax. Keeping
/// the whole Org drawer preserves parser ownership for reference evidence.
pub fn page_property_raw(pre: &str, is_org: bool) -> String {
    let entries = page_property_lines(pre, is_org);
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
    // `((uuid))` in a page property is one referrer of that block.
    if let Some(pre) = document_page_property_block(doc) {
        for id in pre.projection().block_refs() {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
    }
    walk(&doc.roots, &mut counts);
    counts
}
