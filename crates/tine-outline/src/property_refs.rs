//! OG 0.10.9 graph-parser/text.cljs `get-ref-from-ast` and
//! `extract-refs-by-commas`: explicit refs in any parsed property; comma-split
//! top-level Plain nodes additionally for linkable properties. Never split the
//! whole raw value: spaces and commas inside a reference belong to its name.

use lsdoc::ast::{Inline, Url};

/// Append the explicit page reference carried by one top-level inline token.
pub fn ref_from_inline(node: &Inline, out: &mut Vec<String>) {
    match node {
        Inline::Link {
            url: Url::PageRef { v } | Url::Search { v },
            ..
        } => out.push(v.trim().to_string()),
        Inline::NestedLink { content, .. } => out.push(content.trim().to_string()),
        Inline::Tag { children, .. } => {
            let mut text = String::new();
            tag_plain_text(children, &mut text);
            if !text.trim().is_empty() {
                out.push(text.trim().to_string());
            }
        }
        _ => {}
    }
}

fn tag_plain_text(children: &[Inline], out: &mut String) {
    for child in children {
        match child {
            Inline::Plain { text, .. } => out.push_str(text),
            Inline::Link {
                url: Url::PageRef { v } | Url::Search { v },
                ..
            } => out.push_str(v),
            Inline::NestedLink { content, .. } => out.push_str(content),
            _ => {}
        }
    }
}

/// Read exactly the top-level Plain nodes, as OG's comma extraction does.
pub fn plain_segments(nodes: &[Inline], out: &mut Vec<String>) {
    for node in nodes {
        if let Inline::Plain { text, .. } = node {
            out.push(text.clone());
        }
    }
}

/// Linkable-property names in source order, before identity deduplication.
/// Quoted values have no references. Cost O(value bytes), no I/O.
pub fn linkable_names(value: &str, is_org: bool) -> Vec<String> {
    let value = value.trim();
    if value.len() > 1 && value.starts_with('"') && value.ends_with('"') {
        return Vec::new();
    }
    let nodes =
        crate::lsdoc_block_parse::parse_inline_bounded(value, if is_org { "org" } else { "md" })
            .unwrap_or_default();
    let mut names = Vec::new();
    for node in nodes {
        ref_from_inline(&node, &mut names);
        if let Inline::Plain { text, .. } = node {
            names.extend(
                text.split(crate::block_regions::is_linkable_property_separator)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    names
}
