//! OG 0.10.9 graph-parser/text.cljs `get-ref-from-ast` and
//! `extract-refs-by-commas`: explicit refs in any parsed property; comma-split
//! top-level Plain nodes additionally for linkable properties. Never split the
//! whole raw value: spaces and commas inside a reference belong to its name.

use lsdoc::ast::{Inline, Url};
use std::ops::Range;

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
    let mut names = Vec::new();
    for_each_linkable_member(value, is_org, |name, _| names.push(name));
    names
}

/// Visit native members and their complete token ranges, including wrappers.
/// Ranges are absent when the parser cannot map decoded text to source bytes.
/// Names share the same producer as [`linkable_names`]. O(value bytes), no I/O.
pub fn for_each_linkable_member(
    value: &str,
    is_org: bool,
    mut visit: impl FnMut(String, Option<Range<usize>>),
) {
    let leading = value.len() - value.trim_start().len();
    let value = value.trim();
    if value.len() > 1 && value.starts_with('"') && value.ends_with('"') {
        return;
    }
    let nodes =
        crate::lsdoc_block_parse::parse_inline_bounded(value, if is_org { "org" } else { "md" })
            .unwrap_or_default();
    let mut names = Vec::new();
    for node in nodes {
        let token = match &node {
            Inline::Link { span, .. }
            | Inline::NestedLink { span, .. }
            | Inline::Tag { span, .. } => span.as_ref().map(|s| leading + s.0..leading + s.1),
            _ => None,
        };
        ref_from_inline(&node, &mut names);
        for name in names.drain(..) {
            visit(name, token.clone());
        }
        if let Inline::Plain { text, span, .. } = node {
            let start = span
                .filter(|s| value.get(s.0..s.1) == Some(text.as_str()))
                .map(|s| leading + s.0);
            let mut at = 0;
            for segment in text.split(crate::block_regions::is_linkable_property_separator) {
                let name = segment.trim();
                if !name.is_empty() {
                    let offset = at + segment.len() - segment.trim_start().len();
                    visit(
                        name.to_owned(),
                        start.map(|s| s + offset..s + offset + name.len()),
                    );
                }
                at += segment.len();
                at += text[at..].chars().next().map_or(0, char::len_utf8);
            }
        }
    }
}
