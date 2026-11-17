//! The members of one property value: the ONE answer to "which names does a
//! property value contain?", read by the reference walk (index, aliases, model
//! observation, rename, merge), the window's property chips and page tags,
//! and query atoms.
//!
//! OG 0.10.9 graph-parser `text.cljs:100-131,148-185` (`parse-property`):
//! the value's top-level reference forms from mldoc's property parse
//! (`Property.property_references`: macros skipped, so a macro's text is read
//! in place) after `mldoc.cljc:117-132` `ast-link?` removal, united, for a
//! comma-separated key, with the comma members of the Plain children of the
//! value's first DOCUMENT node. Nothing nested in another inline form
//! (emphasis, sub/superscript, a link label) is a member. Never split the
//! whole raw value: spaces and commas inside a reference belong to its name.

use lsdoc::ast::{Inline, Url};
use std::ops::Range;

/// How a value is read, by its key (the caller decides from the graph config).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grammar {
    /// A comma-separated key (`tags`, `alias`, `aliases`, configured keys):
    /// references plus comma members.
    Linkable,
    /// Every other parsed key: references only.
    Parsed,
    /// A key in `:ignored-page-references-keywords`: OG keeps the string and
    /// `block.cljs` `extract-refs-from-property-value` reads its ordinary
    /// inline parse, so macros stay macros.
    Literal,
}

/// The syntactic form a member was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Link,
    NestedLink,
    Tag,
    /// A comma member of a Plain node.
    Plain,
}

/// Where a member stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Within {
    Value,
    /// Inside the top-level `{{embed …}}` at this range.
    Embed(Range<usize>),
    /// Inside another top-level macro at this range.
    Macro(Range<usize>),
}

/// One member with its native coordinates, byte offsets in the value as given
/// (untrimmed). `token` is the whole reference token, wrappers included; for a
/// plain member it is the bytes its name was decoded from (escapes included).
/// `run` is the name's own bytes, absent when no contiguous run spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub token: Range<usize>,
    pub run: Option<Range<usize>>,
    pub kind: Kind,
    pub within: Within,
}

/// The members of `value` in source order, before identity deduplication. A
/// wholly quoted value has none. O(value bytes): one inline parse, one more
/// per top-level macro (its own bytes), and for [`Grammar::Linkable`] one
/// bounded document parse. No I/O.
pub fn members(value: &str, is_org: bool, grammar: Grammar) -> Vec<Member> {
    read(value, is_org, grammar).members
}

/// [`members`] with the inline parse they were read from, so a caller that
/// also walks the value's nodes (the reference walk's rename targets) parses
/// it once.
pub struct Reading {
    pub members: Vec<Member>,
    /// The trimmed value's inline parse; empty when it has no members to read
    /// (blank or quoted) or is deeper than the parser's bound.
    pub nodes: Vec<Inline>,
    /// The trimmed value's offset in `value`: node spans start here.
    pub lead: usize,
}

/// [`members`] and the parse behind them. O(value bytes): one inline parse,
/// one in-place parse per top-level macro and, for [`Grammar::Linkable`], one
/// document parse.
pub fn read(value: &str, is_org: bool, grammar: Grammar) -> Reading {
    let lead = value.len() - value.trim_start().len();
    let text = value.trim();
    let mut out = Vec::new();
    if text.is_empty() || (text.len() > 1 && text.starts_with('"') && text.ends_with('"')) {
        return Reading {
            members: out,
            nodes: Vec::new(),
            lead,
        };
    }
    let nodes =
        crate::lsdoc_block_parse::parse_inline_bounded(text, format(is_org)).unwrap_or_default();
    let reader = Read {
        text,
        org: is_org,
        skip_macros: grammar != Grammar::Literal,
    };
    reader.references(&nodes, 0, &Within::Value, &mut out);
    if grammar == Grammar::Linkable {
        plain_members(text, is_org, &mut out);
    }
    out.sort_by_key(|m| m.token.start);
    let shift = |r: &mut Range<usize>| *r = r.start + lead..r.end + lead;
    for member in &mut out {
        shift(&mut member.token);
        if let Some(run) = member.run.as_mut() {
            shift(run);
        }
        if let Within::Embed(r) | Within::Macro(r) = &mut member.within {
            shift(r);
        }
    }
    Reading {
        members: out,
        nodes,
        lead,
    }
}

fn format(is_org: bool) -> &'static str {
    if is_org {
        "org"
    } else {
        "md"
    }
}

/// The name run of an accepted page link node (`[[x]]`, Org `[[x][label]]`,
/// Markdown `[label]([[x]])`), relative to the node. O(node bytes).
pub fn page_link_run(node: &str, org: bool) -> Option<Range<usize>> {
    if node.len() < 4 {
        return None;
    }
    if org {
        return Some(2..node.find("][").unwrap_or(node.len() - 2));
    }
    if node.starts_with("[[") {
        return Some(2..node.len() - 2);
    }
    let split = node.rfind("](")?;
    let target = node.get(split + 2..node.len() - 1)?;
    let lead = target.len() - target.trim_start().len();
    let start = split + 2 + lead + 2;
    Some(start..start + target.trim().len().checked_sub(4)?)
}

struct Read<'a> {
    text: &'a str,
    org: bool,
    skip_macros: bool,
}

impl Read<'_> {
    /// The top-level reference forms of `nodes`, parsed from `text[base..]`.
    fn references(&self, nodes: &[Inline], base: usize, within: &Within, out: &mut Vec<Member>) {
        for node in nodes {
            if let Some((name, token, run, kind)) = self.reference(node, base, false) {
                if !name.is_empty() {
                    out.push(Member {
                        name,
                        token,
                        run,
                        kind,
                        within: within.clone(),
                    });
                }
                continue;
            }
            let Inline::Macro {
                name,
                span: Some(span),
                ..
            } = node
            else {
                continue;
            };
            // mldoc reads property references with macros skipped: `{{` is
            // text and the macro's bytes after it (closing braces included)
            // are read in place, so its top-level links are the value's.
            let whole = base + span.0..base + span.1;
            let Some(inner) = self.text.get(whole.start + 2..whole.end) else {
                continue;
            };
            if !self.skip_macros || !self.text[whole.clone()].starts_with("{{") {
                continue;
            }
            let within = match within {
                Within::Value if name.eq_ignore_ascii_case("embed") => Within::Embed(whole.clone()),
                Within::Value => Within::Macro(whole.clone()),
                outer => outer.clone(),
            };
            if let Some(nodes) =
                crate::lsdoc_block_parse::parse_inline_bounded(inner, format(self.org))
            {
                self.references(&nodes, whole.start + 2, &within, out);
            }
        }
    }

    /// OG `get-ref-from-ast` on one node: a page link (no asset or drawing,
    /// `ast-link?`), a nested link, or a tag named by its first child.
    /// Returns (trimmed name, token, run, kind).
    fn reference(
        &self,
        node: &Inline,
        base: usize,
        tag_child: bool,
    ) -> Option<(String, Range<usize>, Option<Range<usize>>, Kind)> {
        let token = |span: &Option<lsdoc::ast::Span>| {
            span.as_ref()
                .map(|s| base + s.0..base + s.1)
                .filter(|r| r.start <= r.end && r.end <= self.text.len())
        };
        match node {
            // A tag's first child is read by `get-ref-from-ast` without the
            // `ast-link?` filter, so a Search target names it as well.
            Inline::Link {
                url: Url::PageRef { v },
                span,
                ..
            }
            | Inline::Link {
                url: Url::Search { v },
                span,
                ..
            } if tag_child
                || matches!(
                    node,
                    Inline::Link {
                        url: Url::PageRef { .. },
                        ..
                    }
                ) && !crate::block_regions::local_asset_target(v) =>
            {
                let token = token(span)?;
                let run = page_link_run(&self.text[token.clone()], self.org)
                    .map(|r| token.start + r.start..token.start + r.end);
                Some((v.trim().to_owned(), token, run, Kind::Link))
            }
            // `page-ref/get-page-name`: the text between the outermost brackets.
            Inline::NestedLink { content, span } => {
                let token = token(span)?;
                let name = content.strip_prefix("[[")?.strip_suffix("]]")?;
                let node = &self.text[token.clone()];
                let run = (node.starts_with("[[") && node.ends_with("]]") && node.len() >= 4)
                    .then(|| token.start + 2..token.end - 2);
                Some((name.trim().to_owned(), token, run, Kind::NestedLink))
            }
            Inline::Tag { children, span } => {
                let token = token(span)?;
                let (name, run) = match children.first()? {
                    Inline::Plain { text, .. } => {
                        let at = token.start + 1;
                        let run = self.text[at..]
                            .starts_with(text.as_str())
                            .then(|| at..at + text.len());
                        (text.trim().to_owned(), run)
                    }
                    child => {
                        let (name, _, run, _) = self.reference(child, base, true)?;
                        (name, run)
                    }
                };
                Some((name, token, run, Kind::Tag))
            }
            _ => None,
        }
    }
}

/// OG `extract-refs-by-commas`: it parses the value with the DOCUMENT entry
/// point (`gp-mldoc/->edn`) and splits the Plain children of the first node.
/// Only a paragraph has inline children there; any other first node (heading,
/// list, quote, table, rule, directive, comment, example, math, HTML, hiccup)
/// gives none, so `# x` has no comma member while `\# x` has `# x`. The parser
/// decides the form and the source of each decoded byte; nothing here looks
/// at the text. O(value bytes), one bounded document parse.
fn plain_members(text: &str, is_org: bool, out: &mut Vec<Member>) {
    let nodes = match crate::lsdoc_block_parse::parse_text_bounded(text, format(is_org))
        .and_then(|doc| doc.blocks.into_iter().next())
    {
        Some(lsdoc::ast::Block::Paragraph { inline, .. }) => inline,
        _ => return,
    };
    for node in nodes {
        let Inline::Plain {
            text: plain,
            span,
            span_map,
        } = node
        else {
            continue;
        };
        // lsdoc keeps the segment type crate-private; its fields are public.
        let map: Option<Vec<_>> = span_map.map(|m| m.iter().map(|s| (s.0, s.1, s.2)).collect());
        let whole = span.as_ref().map_or(0..0, |s| s.0..s.1);
        let source = PlainSource::new(text, &plain, span, map.as_deref());
        let mut source = source.map(|s| (s, 0));
        let mut at = 0;
        for segment in plain.split(crate::block_regions::is_linkable_property_separator) {
            let name = segment.trim();
            if !name.is_empty() {
                let offset = at + segment.len() - segment.trim_start().len();
                let run = source
                    .as_mut()
                    .and_then(|(s, cursor)| s.range(offset..offset + name.len(), cursor));
                out.push(Member {
                    name: name.to_owned(),
                    token: run.clone().unwrap_or(whole.clone()),
                    run,
                    kind: Kind::Plain,
                    within: Within::Value,
                });
            }
            at += segment.len();
            at += plain[at..].chars().next().map_or(0, char::len_utf8);
        }
    }
}

/// The names of a comma-separated key's value, in source order, before
/// identity deduplication. O(value bytes), no I/O.
pub fn linkable_names(value: &str, is_org: bool) -> Vec<String> {
    members(value, is_org, Grammar::Linkable)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// [`members`] of a comma-separated key as (name, range, plain): a reference's
/// range is its complete token, a plain member's the bytes its name was
/// decoded from, absent when the parser cannot map them. O(value bytes).
pub fn for_each_linkable_member(
    value: &str,
    is_org: bool,
    mut visit: impl FnMut(String, Option<Range<usize>>, bool),
) {
    for m in members(value, is_org, Grammar::Linkable) {
        let plain = m.kind == Kind::Plain;
        let range = if plain { m.run } else { Some(m.token) };
        visit(m.name, range, plain);
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

    #[test]
    fn mixed_quoted_members_keep_their_complete_native_name_ranges() {
        for org in [false, true] {
            assert!(members("\"literal\"", org).is_empty());
            let value = "\"literal\", p\\\"q";
            let parsed = members(value, org);
            assert_eq!(parsed[0], ("\"literal\"".into(), Some("\"literal\""), true));
            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[1].1, Some("p\\\"q"));
        }
    }

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

    #[test]
    fn members_are_top_level_and_read_macros_in_place() {
        use super::{members, Grammar, Kind, Within};
        let names = |value: &str, org: bool| -> Vec<String> {
            members(value, org, Grammar::Linkable)
                .into_iter()
                .map(|m| m.name)
                .collect()
        };
        // Emphasis, sub/superscript and link labels hold no member.
        assert!(names("**[[a]]**", false).is_empty());
        assert_eq!(names("[[a [[b]]]]", false), ["a [[b]]"]);
        assert_eq!(names("{{renderer [[m]]}}", false), ["m"]);
        let embed = members("x, {{embed [[a]]}}", false, Grammar::Linkable);
        let e = &embed[1];
        assert_eq!((e.name.as_str(), e.kind), ("a", Kind::Link));
        assert_eq!(e.within, Within::Embed(3..18));
        assert_eq!(e.run, Some(13..14));
        // A literal key reads macros as macros.
        assert!(members("{{embed [[a]]}}", false, Grammar::Literal).is_empty());
        // Tag runs: bare and bracketed.
        let t = &members("#a, #[[b c]]", false, Grammar::Parsed);
        assert_eq!(
            (t[0].run.clone(), t[1].run.clone()),
            (Some(1..2), Some(7..10))
        );
    }
}
