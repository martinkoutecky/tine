//! Canonical, source-addressed page-reference evidence.
//!
//! lsdoc decides which syntax is a reference and which source ranges are plain
//! visible text.  This module maps those parser spans back from Tine's
//! re-bulleted parse input to `DocBlock::raw`; query surfaces then select a
//! canonical page/alias set without reparsing or inventing another matcher.

use crate::model::{ReferenceKind, ReferenceOccurrence, ReferenceSpan};
use crate::refs;
use lsdoc::ast::{Block, Inline, ListItem, Span, Url};
use std::ops::Range;

pub const ENGINE_VERSION: &str = "reference-evidence/v1";
const MAX_OCCURRENCES_PER_BLOCK: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectedPageRef {
    pub name: String,
    pub range: Range<usize>,
    pub rule: &'static str,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReferenceSourceProjection {
    pub explicit: Vec<ProjectedPageRef>,
    pub plain_ranges: Vec<Range<usize>>,
}

#[derive(Clone, Copy)]
struct SpanMapper {
    prefix: usize,
    raw_offset: usize,
    leading_trim: usize,
}

impl SpanMapper {
    fn block(raw: &str) -> Self {
        Self {
            prefix: 2,
            raw_offset: 0,
            leading_trim: raw.len() - raw.trim_start().len(),
        }
    }

    fn direct(raw_offset: usize) -> Self {
        Self {
            prefix: 0,
            raw_offset,
            leading_trim: 0,
        }
    }

    fn map(self, span: &Span, raw_len: usize) -> Option<Range<usize>> {
        let start = span
            .0
            .checked_sub(self.prefix)?
            .checked_add(self.leading_trim)?
            .checked_add(self.raw_offset)?;
        let end = span
            .1
            .checked_sub(self.prefix)?
            .checked_add(self.leading_trim)?
            .checked_add(self.raw_offset)?;
        (start <= end && end <= raw_len).then_some(start..end)
    }
}

fn flatten_inlines(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Plain { text, .. }
            | Inline::Code { text, .. }
            | Inline::Verbatim { text, .. } => out.push_str(text),
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => flatten_inlines(children, out),
            Inline::Link { url, label, .. } => {
                if label.is_empty() {
                    match url {
                        Url::PageRef { v }
                        | Url::BlockRef { v }
                        | Url::Search { v }
                        | Url::File { v }
                        | Url::EmbedData { v } => out.push_str(v),
                        Url::Complex { link, .. } => {
                            if let Some(link) = link {
                                out.push_str(link);
                            }
                        }
                    }
                } else {
                    flatten_inlines(label, out);
                }
            }
            Inline::NestedLink { content, .. } => out.push_str(content),
            Inline::Target { text, .. } => out.push_str(text),
            Inline::Entity { unicode, .. } => out.push_str(unicode),
            Inline::Latex { body, .. } => out.push_str(body),
            Inline::Hiccup { v, .. } => out.push_str(v),
            _ => {}
        }
    }
}

fn local_asset(value: &str) -> bool {
    value.trim_start_matches(['.', '/']).starts_with("assets") || value.starts_with("draws")
}

fn unbracket(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("[[")
        .and_then(|rest| rest.strip_suffix("]]"))
        .unwrap_or(value)
}

fn link_page_name(url: &Url, label: &[Inline], is_org: bool) -> Option<String> {
    match url {
        Url::PageRef { v } if !local_asset(v) => Some(v.clone()),
        Url::Search { v } if v.trim().starts_with("[[") && v.trim().ends_with("]]") => {
            Some(unbracket(v).to_string())
        }
        Url::Search { v } if is_org && !local_asset(v) => Some(v.clone()),
        Url::File { .. } if !label.is_empty() => {
            let mut value = String::new();
            flatten_inlines(label, &mut value);
            (!value.trim().is_empty()).then_some(value)
        }
        _ => None,
    }
}

fn tag_name(children: &[Inline]) -> String {
    let mut value = String::new();
    flatten_inlines(children, &mut value);
    value
}

fn push_explicit(
    projection: &mut ReferenceSourceProjection,
    name: String,
    span: Option<&Span>,
    mapper: SpanMapper,
    raw_len: usize,
    rule: &'static str,
) {
    let Some(range) = span.and_then(|span| mapper.map(span, raw_len)) else {
        return;
    };
    push_explicit_range(projection, name, range, raw_len, rule);
}

fn push_explicit_range(
    projection: &mut ReferenceSourceProjection,
    name: String,
    range: Range<usize>,
    raw_len: usize,
    rule: &'static str,
) {
    if range.start > range.end || range.end > raw_len {
        return;
    }
    if !name.trim().is_empty() {
        projection
            .explicit
            .push(ProjectedPageRef { name, range, rule });
    }
}

fn nested_names(content: &str) -> Vec<String> {
    let mut starts = Vec::new();
    let mut out = Vec::new();
    let bytes = content.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1] == b'[' {
            starts.push(index + 2);
            index += 2;
        } else if bytes[index] == b']' && bytes[index + 1] == b']' {
            if let Some(start) = starts.pop() {
                if start <= index {
                    out.push(content[start..index].to_string());
                }
            }
            index += 2;
        } else {
            index += content[index..].chars().next().map_or(1, char::len_utf8);
        }
    }
    if out.is_empty() && !content.trim().is_empty() {
        out.push(unbracket(content).to_string());
    }
    out
}

fn walk_inlines(
    inlines: &[Inline],
    mapper: SpanMapper,
    raw_len: usize,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    for inline in inlines {
        match inline {
            Inline::Plain {
                span: Some(span), ..
            } => {
                if let Some(range) = mapper.map(span, raw_len) {
                    projection.plain_ranges.push(range);
                }
            }
            Inline::Link {
                url, label, span, ..
            } => {
                if let Some(name) = link_page_name(url, label, is_org) {
                    push_explicit(
                        projection,
                        name,
                        span.as_ref(),
                        mapper,
                        raw_len,
                        "explicit_link",
                    );
                }
                walk_inlines(label, mapper, raw_len, is_org, projection);
            }
            Inline::NestedLink { content, span } => {
                for name in nested_names(content) {
                    push_explicit(
                        projection,
                        name,
                        span.as_ref(),
                        mapper,
                        raw_len,
                        "explicit_nested_link",
                    );
                }
            }
            Inline::Tag { children, span } => {
                push_explicit(
                    projection,
                    tag_name(children),
                    span.as_ref(),
                    mapper,
                    raw_len,
                    "explicit_tag",
                );
                walk_inlines(children, mapper, raw_len, is_org, projection);
            }
            Inline::Macro { name, args, span } if name == "embed" => {
                let value = if args.len() <= 1 {
                    args.first().cloned().unwrap_or_default()
                } else {
                    args.join(", ")
                };
                push_explicit(
                    projection,
                    unbracket(&value).trim().to_string(),
                    span.as_ref(),
                    mapper,
                    raw_len,
                    "explicit_embed",
                );
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. } => {
                walk_inlines(children, mapper, raw_len, is_org, projection)
            }
            // Code/verbatim and the remaining opaque inline forms are deliberately
            // not plain-reference search ranges.
            _ => {}
        }
    }
}

fn walk_list_item(
    item: &ListItem,
    mapper: SpanMapper,
    raw: &str,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    walk_inlines(&item.name, mapper, raw.len(), is_org, projection);
    walk_blocks(&item.content, mapper, raw, is_org, projection);
    for child in &item.items {
        walk_list_item(child, mapper, raw, is_org, projection);
    }
}

fn property_values(
    span: Option<&Span>,
    mapper: SpanMapper,
    raw: &str,
) -> Vec<(String, usize, String)> {
    let Some(range) = span.and_then(|span| mapper.map(span, raw.len())) else {
        return Vec::new();
    };
    let Some(source) = raw.get(range.clone()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut line_offset = range.start;
    for line in source.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        if let Some((key, value)) = crate::doc::parse_property_line(line_without_newline) {
            if let Some(value_at) = line_without_newline.rfind(&value) {
                out.push((key, line_offset + value_at, value));
            }
        }
        line_offset += line.len();
    }
    out
}

fn project_implicit_linkable_property(
    projection: &mut ReferenceSourceProjection,
    key: &str,
    value_offset: usize,
    value: &str,
    raw_len: usize,
) {
    if !(key.eq_ignore_ascii_case("tags")
        || key.eq_ignore_ascii_case("alias")
        || key.eq_ignore_ascii_case("aliases"))
    {
        return;
    }
    let whole = value.trim();
    if whole.len() >= 2 && whole.starts_with('"') && whole.ends_with('"') {
        return;
    }

    let mut segment_start = 0;
    for (index, separator) in value
        .char_indices()
        .filter(|(_, ch)| *ch == ',' || *ch == '，')
        .map(|(index, ch)| (index, ch.len_utf8()))
        .chain(std::iter::once((value.len(), 0)))
    {
        let segment = &value[segment_start..index];
        let leading = segment.len() - segment.trim_start().len();
        let name = segment.trim();
        // Wrapped page refs and tags are already parser-owned explicit
        // occurrences. Mixed syntax is likewise left to the parser rather than
        // inventing a second interpretation for one property member.
        if !name.is_empty()
            && !name.contains("[[")
            && !name.contains("]]")
            && !name.starts_with('#')
        {
            let start = value_offset + segment_start + leading;
            let end = start + name.len();
            push_explicit_range(
                projection,
                name.to_string(),
                start..end,
                raw_len,
                "implicit_linkable_property",
            );
        }
        segment_start = index + separator;
    }
}

fn walk_blocks(
    blocks: &[Block],
    mapper: SpanMapper,
    raw: &str,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    for block in blocks {
        match block {
            Block::Paragraph { inline, .. }
            | Block::Heading { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::FootnoteDef { inline, .. } => {
                walk_inlines(inline, mapper, raw.len(), is_org, projection)
            }
            Block::Quote { children, .. } | Block::Custom { children, .. } => {
                walk_blocks(children, mapper, raw, is_org, projection)
            }
            Block::List { items, .. } => {
                for item in items {
                    walk_list_item(item, mapper, raw, is_org, projection);
                }
            }
            Block::Table { header, rows, .. } => {
                if let Some(header) = header {
                    for cell in header {
                        walk_inlines(cell, mapper, raw.len(), is_org, projection);
                    }
                }
                for row in rows {
                    for cell in row {
                        walk_inlines(cell, mapper, raw.len(), is_org, projection);
                    }
                }
            }
            Block::Properties { span, .. } => {
                for (key, offset, value) in property_values(span.as_ref(), mapper, raw) {
                    let parsed = lsdoc::parse_format(&value, if is_org { "org" } else { "md" });
                    walk_blocks(
                        &parsed.blocks,
                        SpanMapper::direct(offset),
                        raw,
                        is_org,
                        projection,
                    );
                    project_implicit_linkable_property(projection, &key, offset, &value, raw.len());
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn project(raw: &str, is_org: bool, blocks: &[Block]) -> ReferenceSourceProjection {
    let mut projection = ReferenceSourceProjection::default();
    walk_blocks(blocks, SpanMapper::block(raw), raw, is_org, &mut projection);
    projection.explicit.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then_with(|| a.range.end.cmp(&b.range.end))
            .then_with(|| a.name.cmp(&b.name))
    });
    projection.explicit.dedup();
    projection
        .plain_ranges
        .sort_by_key(|range| (range.start, range.end));
    projection.plain_ranges.dedup();
    projection
}

fn byte_to_utf16(raw: &str, byte: usize) -> usize {
    raw.get(..byte)
        .map(|prefix| prefix.encode_utf16().count())
        .unwrap_or_else(|| raw.encode_utf16().count())
}

fn is_og_edge_alphanumeric(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric())
}

fn overlaps(range: &Range<usize>, other: &Range<usize>) -> bool {
    range.start < other.end && other.start < range.end
}

/// Visit source-order matches with memory bounded by the target name, not the
/// number or size of matches in the block.
fn visit_plain_matches(
    raw: &str,
    range: &Range<usize>,
    needle: &str,
    mut visit: impl FnMut(Range<usize>) -> bool,
) {
    let Some(source) = raw.get(range.clone()) else {
        return;
    };
    if needle.is_empty() {
        return;
    }
    let first_requires_boundary = needle.chars().next().is_some_and(|ch| ch.is_alphanumeric());
    let last_requires_boundary = needle
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_alphanumeric());
    for (offset, _) in source.char_indices() {
        let start = range.start + offset;
        let mut candidate = source[offset..].char_indices().flat_map(|(relative, ch)| {
            let end = start + relative + ch.len_utf8();
            ch.to_lowercase().map(move |lower| (lower, end))
        });
        let mut end = start;
        let matched = needle.chars().flat_map(char::to_lowercase).all(|expected| {
            candidate.next().is_some_and(|(actual, actual_end)| {
                end = actual_end;
                actual == expected
            })
        });
        if !matched {
            continue;
        }
        let before = raw
            .get(..start)
            .and_then(|prefix| prefix.chars().next_back());
        let after = raw.get(end..).and_then(|suffix| suffix.chars().next());
        // Exact OG edge semantics: only adjacent ASCII alphanumerics exclude
        // an unlinked match. `_` and continuous CJK are valid boundaries.
        if (!first_requires_boundary || !is_og_edge_alphanumeric(before))
            && (!last_requires_boundary || !is_og_edge_alphanumeric(after))
            && !visit(start..end)
        {
            return;
        }
    }
}

fn push_unique_bounded(
    out: &mut Vec<ReferenceOccurrence>,
    matched_name: &str,
    canonical: &str,
    kind: ReferenceKind,
    span: ReferenceSpan,
    rule: &str,
) -> bool {
    if out.iter().any(|existing| {
        existing.span == span
            && existing.kind == kind
            && refs::same_page(&existing.matched_name, matched_name)
    }) {
        return true;
    }
    if out.len() >= MAX_OCCURRENCES_PER_BLOCK {
        return false;
    }
    #[cfg(test)]
    OCCURRENCE_CONSTRUCTIONS.with(|count| count.set(count.get().saturating_add(1)));
    out.push(ReferenceOccurrence {
        matched_name: matched_name.to_string(),
        canonical: canonical.to_string(),
        kind,
        span,
        rule: rule.to_string(),
    });
    true
}

#[cfg(test)]
thread_local! {
    static OCCURRENCE_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_occurrence_constructions() {
    OCCURRENCE_CONSTRUCTIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn occurrence_constructions() -> usize {
    OCCURRENCE_CONSTRUCTIONS.with(std::cell::Cell::get)
}

pub(crate) fn occurrences_of_kind(
    raw: &str,
    projection: &ReferenceSourceProjection,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
) -> Vec<ReferenceOccurrence> {
    let mut out = Vec::with_capacity(MAX_OCCURRENCES_PER_BLOCK.min(8));
    if kind == ReferenceKind::Explicit {
        for reference in &projection.explicit {
            if !names_norm
                .iter()
                .any(|name| refs::same_page(name, &reference.name))
            {
                continue;
            }
            if out.len() >= MAX_OCCURRENCES_PER_BLOCK {
                break;
            }
            if !push_unique_bounded(
                &mut out,
                reference.name.trim(),
                canonical,
                kind,
                ReferenceSpan {
                    start: byte_to_utf16(raw, reference.range.start),
                    end: byte_to_utf16(raw, reference.range.end),
                },
                reference.rule,
            ) {
                break;
            }
        }
        return out;
    }

    'names: for name in names_norm {
        for eligible in &projection.plain_ranges {
            let mut full = false;
            visit_plain_matches(raw, eligible, name, |range| {
                if projection
                    .explicit
                    .iter()
                    .any(|reference| overlaps(&range, &reference.range))
                {
                    return true;
                }
                if out.len() >= MAX_OCCURRENCES_PER_BLOCK {
                    full = true;
                    return false;
                }
                let keep_scanning = push_unique_bounded(
                    &mut out,
                    raw.get(range.clone()).unwrap_or(name),
                    canonical,
                    kind,
                    ReferenceSpan {
                        start: byte_to_utf16(raw, range.start),
                        end: byte_to_utf16(raw, range.end),
                    },
                    "plain_og_boundary",
                );
                full = !keep_scanning;
                keep_scanning
            });
            if full {
                break 'names;
            }
        }
    }
    out.sort_by_key(|occurrence| (occurrence.span.start, occurrence.span.end));
    out
}

/// Cheap membership path used once a result construction budget is closed.
/// It performs no occurrence/string construction and stops at the first hit.
pub(crate) fn has_occurrence_kind(
    raw: &str,
    projection: &ReferenceSourceProjection,
    names_norm: &[String],
    kind: ReferenceKind,
) -> bool {
    if kind == ReferenceKind::Explicit {
        return projection.explicit.iter().any(|reference| {
            names_norm
                .iter()
                .any(|name| refs::same_page(name, &reference.name))
        });
    }
    for name in names_norm {
        for eligible in &projection.plain_ranges {
            let mut found = false;
            visit_plain_matches(raw, eligible, name, |range| {
                found = !projection
                    .explicit
                    .iter()
                    .any(|reference| overlaps(&range, &reference.range));
                !found
            });
            if found {
                return true;
            }
        }
    }
    false
}

pub(crate) fn occurrences(
    raw: &str,
    projection: &ReferenceSourceProjection,
    canonical: &str,
    names_norm: &[String],
) -> Vec<ReferenceOccurrence> {
    let mut out = occurrences_of_kind(
        raw,
        projection,
        canonical,
        names_norm,
        ReferenceKind::Explicit,
    );
    out.extend(occurrences_of_kind(
        raw,
        projection,
        canonical,
        names_norm,
        ReferenceKind::Plain,
    ));
    out.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then_with(|| a.span.end.cmp(&b.span.end))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.matched_name.cmp(&b.matched_name))
    });
    out.truncate(MAX_OCCURRENCES_PER_BLOCK);
    out
}

/// Deliberately uncached parser path used by diagnostics/tests as a drift
/// oracle for the memoized `DocBlock::projection` integration.
pub(crate) fn slow_occurrences(
    raw: &str,
    is_org: bool,
    canonical: &str,
    names_norm: &[String],
) -> Vec<ReferenceOccurrence> {
    let parsed = crate::render::parse_projection(raw, is_org);
    let source = project(raw, is_org, &parsed.blocks);
    occurrences(raw, &source, canonical, names_norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(raw: &str, names: &[&str]) -> Vec<ReferenceOccurrence> {
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        occurrences(
            raw,
            &projected,
            "Target",
            &names
                .iter()
                .map(|name| refs::normalize(name))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn mixed_explicit_and_plain_occurrences_stay_independent() {
        let raw = "[[Target]] then Target and `Target`";
        let got = evidence(raw, &["Target"]);
        assert_eq!(
            got.iter()
                .filter(|hit| hit.kind == ReferenceKind::Explicit)
                .count(),
            1
        );
        assert_eq!(
            got.iter()
                .filter(|hit| hit.kind == ReferenceKind::Plain)
                .count(),
            1
        );
        let plain = got
            .iter()
            .find(|hit| hit.kind == ReferenceKind::Plain)
            .unwrap();
        assert_eq!(&raw[plain.span.start..plain.span.end], "Target");
    }

    #[test]
    fn unicode_boundaries_and_properties_use_parser_ranges() {
        let got = evidence("note:: Target\nŽTargetX Target", &["Target"]);
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
    }

    #[test]
    fn escaped_and_code_links_do_not_become_explicit_or_plain() {
        let got = evidence("\\[[Target]] and `Target`\n```\nTarget\n```", &["Target"]);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].kind, ReferenceKind::Plain);
    }

    #[test]
    fn bare_linkable_property_values_project_exact_explicit_evidence() {
        for raw in [
            "tags:: Target",
            "alias:: Target",
            "aliases:: Other, Target, Third",
            "tags:: Other，Target，Third",
        ] {
            let got = evidence(raw, &["Target"]);
            let explicit = got
                .iter()
                .filter(|hit| hit.kind == ReferenceKind::Explicit)
                .collect::<Vec<_>>();
            assert_eq!(explicit.len(), 1, "{raw}: {got:?}");
            assert_eq!(explicit[0].rule, "implicit_linkable_property");
            let start = raw.find("Target").unwrap();
            assert_eq!(explicit[0].span.start, byte_to_utf16(raw, start));
            assert_eq!(explicit[0].span.end, byte_to_utf16(raw, start + 6));
        }
    }

    #[test]
    fn explicit_property_syntax_is_not_duplicated_or_promoted_from_custom_values() {
        let raw = "tags:: [[Target]]\nalias:: #Target\ncustom:: Target\naliases:: \"Target\"";
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        let target = projected
            .explicit
            .iter()
            .filter(|reference| refs::same_page(&reference.name, "Target"))
            .collect::<Vec<_>>();
        assert_eq!(target.len(), 2, "{target:?}");
        assert!(target
            .iter()
            .all(|reference| reference.rule != "implicit_linkable_property"));
    }

    #[test]
    fn occurrence_construction_is_capped_while_scanning_many_matches() {
        let raw = "Target ".repeat(50_000);
        let parsed = crate::render::parse_projection(&raw, false);
        let projected = project(&raw, false, &parsed.blocks);
        reset_occurrence_constructions();
        let got = occurrences_of_kind(
            &raw,
            &projected,
            "Target",
            &[refs::normalize("Target")],
            ReferenceKind::Plain,
        );
        assert_eq!(got.len(), MAX_OCCURRENCES_PER_BLOCK);
        assert_eq!(occurrence_constructions(), MAX_OCCURRENCES_PER_BLOCK);
        assert!(got.capacity() <= MAX_OCCURRENCES_PER_BLOCK);
    }

    #[test]
    fn plain_boundaries_match_logseq_ascii_edge_rules() {
        let got = evidence("北京Target北京 foo_Target_bar aTargetz", &["Target"]);
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
        assert!(plains.iter().all(|hit| hit.matched_name == "Target"));
    }
}
