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
    for_each_linkable_member(value, is_org, |name, _, _| names.push(name));
    names
}

/// Visit native members, each with its source byte range in `value` and
/// whether it is a plain member. A reference's range is its complete token,
/// wrappers included. A plain member's range is exactly the bytes its name
/// was decoded from, Markdown escapes included (`C\#` reads `C#`), from the
/// parser's text-to-source map. Ranges are absent when the parser cannot map
/// decoded text to source bytes. Names share the same producer as
/// [`linkable_names`]. O(value bytes), no I/O.
pub fn for_each_linkable_member(
    value: &str,
    is_org: bool,
    mut visit: impl FnMut(String, Option<Range<usize>>, bool),
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
            visit(name, token.clone(), false);
        }
        if let Inline::Plain {
            text,
            span,
            span_map,
        } = node
        {
            // lsdoc keeps the segment type crate-private; its fields are public.
            let map: Option<Vec<_>> = span_map.map(|m| m.iter().map(|s| (s.0, s.1, s.2)).collect());
            let source = PlainSource::new(value, &text, span, map.as_deref());
            let mut source = source.map(|s| (s, 0));
            let mut at = 0;
            for segment in text.split(crate::block_regions::is_linkable_property_separator) {
                let name = segment.trim();
                if !name.is_empty() {
                    let offset = at + segment.len() - segment.trim_start().len();
                    let range = source
                        .as_mut()
                        .and_then(|(s, cursor)| s.range(offset..offset + name.len(), cursor));
                    visit(
                        name.to_owned(),
                        range.map(|r| leading + r.start..leading + r.end),
                        true,
                    );
                }
                at += segment.len();
                at += text[at..].chars().next().map_or(0, char::len_utf8);
            }
        }
    }
}

/// Where a plain node's decoded text came from in the parsed value.
enum PlainSource<'a> {
    /// The text is its source slice, starting here.
    Verbatim(usize),
    /// The parser's segments `(text offset, source offset, length)`, in order;
    /// bytes outside them (an escape's backslash) belong to no text byte.
    Mapped(usize, &'a [(usize, usize, usize)]),
}

impl<'a> PlainSource<'a> {
    fn new(
        value: &str,
        text: &str,
        span: Option<lsdoc::ast::Span>,
        map: Option<&'a [(usize, usize, usize)]>,
    ) -> Option<Self> {
        let span = span?;
        if value.get(span.0..span.1) == Some(text) {
            return Some(Self::Verbatim(span.0));
        }
        map.map(|map| Self::Mapped(span.0, map))
    }

    /// Source bytes of decoded text bytes `range` (non-empty, after any
    /// earlier range): from the end of the byte before it (the node start
    /// for the first) through the end of its last byte, so a leading escape
    /// is included. `cursor` only moves forward: O(1) amortized per call.
    fn range(&self, range: Range<usize>, cursor: &mut usize) -> Option<Range<usize>> {
        let (start, map) = match self {
            Self::Verbatim(start) => return Some(start + range.start..start + range.end),
            Self::Mapped(start, map) => (*start, *map),
        };
        let mut end_of = |byte: usize| {
            while map.get(*cursor).is_some_and(|s| s.0 + s.2 <= byte) {
                *cursor += 1;
            }
            map.get(*cursor)
                .filter(|s| s.0 <= byte)
                .map(|s| s.1 + byte - s.0 + 1)
        };
        let from = match range.start {
            0 => start,
            byte => end_of(byte - 1)?,
        };
        Some(from..end_of(range.end - 1)?)
    }
}

#[cfg(test)]
mod member_range_tests {
    use super::for_each_linkable_member;

    fn members(value: &str, org: bool) -> Vec<(String, Option<&str>, bool)> {
        let mut out = Vec::new();
        for_each_linkable_member(value, org, |name, range, plain| {
            out.push((name, range.map(|r| &value[r]), plain))
        });
        out
    }

    #[test]
    fn plain_members_carry_the_bytes_the_parser_decoded_them_from() {
        let m = |name: &str, raw: &str| (name.to_owned(), Some(raw.to_owned()), true);
        let owned = |v: Vec<(String, Option<&str>, bool)>| {
            v.into_iter()
                .map(|(n, r, p)| (n, r.map(str::to_owned), p))
                .collect::<Vec<_>>()
        };
        assert_eq!(owned(members("C\\#", false)), [m("C#", "C\\#")]);
        assert_eq!(
            owned(members("foo\\*bar", false)),
            [m("foo*bar", "foo\\*bar")]
        );
        assert_eq!(
            owned(members(" a, \\#x, b\\,c, x\\\\", false)),
            [
                m("a", "a"),
                m("#x", "\\#x"),
                m("b", "b"),
                m("c", "c"),
                m("x\\", "x\\\\")
            ]
        );
        // Org keeps the backslash: the bytes are the name.
        assert_eq!(owned(members("C\\#", true)), [m("C\\#", "C\\#")]);
        // A reference's range is its whole token.
        assert_eq!(
            owned(members("[[C\\#]], b", false)),
            [
                ("C#".to_owned(), Some("[[C\\#]]".to_owned()), false),
                m("b", "b")
            ]
        );
    }
}
