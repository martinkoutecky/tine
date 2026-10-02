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
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

pub const ENGINE_VERSION: &str = "reference-evidence/v1";
const MAX_OCCURRENCES_PER_BLOCK: usize = 64;

// Exact OG 1.0.0 property-page exclusions from
// `logseq.graph-parser.property/editable-built-in-properties` at 6e7afa8eb.
// Keep source spellings here; `property_key_norm` supplies Tine's canonical
// property identity (including underscore -> dash).
const OG_EDITABLE_BUILT_IN_PROPERTIES: &[&str] = &[
    "title",
    "icon",
    "template",
    "template-including-parent",
    "public",
    "filters",
    "exclude-from-graph-view",
    "logseq.query/nlp-date",
    "macro",
    "filetags",
    "alias",
    "aliases",
    "tags",
    "logseq.color",
    "logseq.table.version",
    "logseq.table.compact",
    "logseq.table.headers",
    "logseq.table.hover",
    "logseq.table.borders",
    "logseq.table.stripes",
    "logseq.table.max-width",
];

// Exact base set from `hidden-built-in-properties`, plus the only registered
// extension set in that revision (`frontend.extensions.srs`).
const OG_HIDDEN_BUILT_IN_PROPERTIES: &[&str] = &[
    "id",
    "custom-id",
    "background-color",
    "background_color",
    "heading",
    "collapsed",
    "created-at",
    "updated-at",
    "last-modified-at",
    "created_at",
    "last_modified_at",
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

/// One parser-recognized page reference and its source span.
#[deny(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectedPageRef {
    /// Reference target spelling.
    pub name: String,
    /// Byte range in the block's raw source. Browser-facing
    /// `ReferenceSpan` values convert these offsets to UTF-16 code units.
    pub range: Range<usize>,
    /// Parser rule that recognized this reference.
    #[serde(with = "reference_rule")]
    pub rule: crate::block_regions::StaticStr,
}

/// `ProjectedPageRef::rule` over serde (the launch checkpoint): the rules are a
/// closed set, so a deserialized rule maps back to its static spelling and an
/// unknown one is an error. `rules_are_a_closed_set` pins the list.
pub mod reference_rule {
    use serde::{Deserialize, Deserializer, Serializer};
    /// Every rule this module's `push_explicit*` callers name.
    pub const RULES: &[&str] = &[
        "explicit_link",
        "explicit_nested_link",
        "explicit_tag",
        "explicit_embed",
        "explicit_property_key",
        "implicit_linkable_property",
    ];
    pub fn serialize<S: Serializer>(value: &&'static str, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(value)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<&'static str, D::Error> {
        let value = String::deserialize(d)?;
        RULES
            .iter()
            .copied()
            .find(|rule| *rule == value)
            .ok_or_else(|| serde::de::Error::custom("unknown reference rule"))
    }
}

/// Borrowed view of the reference spans retained with a block projection.
/// The stored form is split (sparse vectors live behind the projection's
/// optional box); every reader works through this one view.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy)]
pub struct ReferenceSource<'a> {
    /// Explicit page references recognized by the parser.
    pub explicit: &'a [ProjectedPageRef],
    /// Byte ranges in the raw source eligible for plain-text matching.
    pub plain_ranges: &'a [Range<usize>],
    /// Structural source ranges excluded from plain-text matching.
    pub withheld_ranges: &'a [Range<usize>],
}

/// Reference spans produced by one projection build, owned. A block stores
/// them split (see [`ReferenceSource`]); this form is the build result.
#[deny(missing_docs)]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ReferenceSourceProjection {
    /// Explicit page references recognized by the parser.
    pub explicit: Vec<ProjectedPageRef>,
    /// Byte ranges in the raw source left after explicit links and structural
    /// bookkeeping are removed; literal code, math, and HTML remain eligible.
    pub plain_ranges: Vec<Range<usize>>,
    /// Structural source ranges excluded from plain-text matching.
    pub withheld_ranges: Vec<Range<usize>>,
}

impl ReferenceSourceProjection {
    /// Borrow this build result as a [`ReferenceSource`].
    pub fn as_source(&self) -> ReferenceSource<'_> {
        ReferenceSource {
            explicit: &self.explicit,
            plain_ranges: &self.plain_ranges,
            withheld_ranges: &self.withheld_ranges,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoundedOccurrences {
    pub occurrences: Vec<ReferenceOccurrence>,
    pub total: usize,
    pub truncated: bool,
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

use crate::block_regions::{nested_reference_names as nested_names, unbracket};

fn link_page_name(url: &Url, label: &[Inline], is_org: bool) -> Option<String> {
    let mut text = String::new();
    flatten_inlines(label, &mut text);
    let (kind, value) = match url {
        Url::PageRef { v } => ("page_ref", v.as_str()),
        Url::Search { v } => ("search", v.as_str()),
        Url::File { v } => ("file", v.as_str()),
        _ => return None,
    };
    crate::block_regions::reference_target_name(kind, value, &text, is_org, false)
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

fn walk_inlines(
    inlines: &[Inline],
    mapper: SpanMapper,
    raw_len: usize,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    for inline in inlines {
        match inline {
            Inline::Link {
                url, label, span, ..
            } => {
                // A block UUID link is parser-owned syntax. It does not name a
                // page and must not become a plain page mention either.
                if matches!(url, Url::BlockRef { .. }) {
                    if let Some(range) = span.as_ref().and_then(|span| mapper.map(span, raw_len)) {
                        projection.withheld_ranges.push(range);
                    }
                }
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

fn property_key_eligible(key: &str) -> bool {
    let key = crate::doc::property_key_norm(key);
    !key.is_empty()
        && !OG_EDITABLE_BUILT_IN_PROPERTIES
            .iter()
            .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
            .any(|built_in| crate::doc::property_key_norm(built_in) == key)
}

fn project_property_key(
    projection: &mut ReferenceSourceProjection,
    key: &str,
    key_range: Range<usize>,
    raw_len: usize,
) {
    if property_key_eligible(key) {
        push_explicit_range(
            projection,
            crate::doc::property_key_norm(key),
            key_range,
            raw_len,
            "explicit_property_key",
        );
    }
}

/// Page candidates in accepted tags/alias/aliases values. Quoted values stay
/// literal; explicit and implicit references share the existing evidence.
/// O(properties + references), zero parses or allocation; inputs must be the
/// memoized regions and evidence of the same raw block.
pub fn linkable_property_names<'a>(
    projection: ReferenceSource<'a>,
    regions: &'a crate::block_regions::BlockRegions,
) -> impl Iterator<Item = &'a str> {
    let mut properties = regions
        .properties
        .iter()
        .filter(|p| {
            p.applicable
                && (p.key.eq_ignore_ascii_case("tags")
                    || p.key.eq_ignore_ascii_case("alias")
                    || p.key.eq_ignore_ascii_case("aliases"))
                && !quoted_linkable_value(&p.value)
        })
        .peekable();
    projection.explicit.iter().filter_map(move |reference| {
        while properties
            .peek()
            .is_some_and(|p| p.value_range.1 <= reference.range.start)
        {
            properties.next();
        }
        properties
            .peek()
            .filter(|p| {
                p.value_range.0 <= reference.range.start && reference.range.end <= p.value_range.1
            })
            .map(|_| reference.name.as_str())
    })
}

fn quoted_linkable_value(value: &str) -> bool {
    let whole = value.trim();
    whole.len() >= 2 && whole.starts_with('"') && whole.ends_with('"')
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
    if quoted_linkable_value(value) {
        return;
    }

    let mut segment_start = 0;
    for (index, separator) in value
        .char_indices()
        .filter(|(_, ch)| refs::is_linkable_property_separator(*ch))
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

fn structural_property(key: &str, raw: &str, is_org: bool) -> bool {
    (key.eq_ignore_ascii_case("id") && refs::block_id(raw, is_org).is_some())
        || key.eq_ignore_ascii_case("collapsed")
        || key.to_ascii_lowercase().starts_with("logseq.")
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
            Block::Drawer {
                name,
                span: Some(span),
            } if name.eq_ignore_ascii_case("logbook") => {
                if let Some(range) = mapper.map(span, raw.len()) {
                    projection.withheld_ranges.push(range);
                }
            }
            _ => {}
        }
    }
}

fn plain_search_ranges(
    raw_len: usize,
    projection: &ReferenceSourceProjection,
) -> Vec<Range<usize>> {
    let mut claimed: Vec<_> = projection
        .explicit
        .iter()
        .map(|reference| reference.range.clone())
        .chain(projection.withheld_ranges.iter().cloned())
        .filter(|range| range.start < range.end && range.end <= raw_len)
        .collect();
    claimed.sort_by_key(|range| (range.start, range.end));
    let mut out = Vec::new();
    let mut cursor = 0;
    for range in claimed {
        if range.start > cursor {
            out.push(cursor..range.start);
        }
        cursor = cursor.max(range.end);
    }
    if cursor < raw_len {
        out.push(cursor..raw_len);
    }
    out
}

/// Project parser-claimed references and the remaining plain-search spans for
/// one block source. Work and allocation are bounded by that source's spans;
/// malformed ranges are omitted, and no graph state or files are touched.
pub fn project(raw: &str, is_org: bool, blocks: &[Block]) -> ReferenceSourceProjection {
    let mut projection = ReferenceSourceProjection::default();
    walk_blocks(blocks, SpanMapper::block(raw), raw, is_org, &mut projection);
    let regions = crate::block_regions::from_blocks(raw, is_org, blocks);
    for property in regions.properties.iter().filter(|p| p.applicable) {
        let key = &property.key;
        let key_range = property.key_range.0..property.key_range.1;
        let offset = property.value_range.0;
        let value = property.value_range.slice(raw);
        project_property_key(&mut projection, key, key_range.clone(), raw.len());
        if structural_property(key, raw, is_org) {
            projection
                .withheld_ranges
                .push(key_range.start..property.value_range.1);
            continue;
        }
        if let Some(parsed) = crate::render::parse_text_bounded(value, is_org) {
            walk_blocks(
                &parsed.blocks,
                SpanMapper::direct(offset),
                raw,
                is_org,
                &mut projection,
            );
        }
        project_implicit_linkable_property(&mut projection, key, offset, value, raw.len());
    }

    projection.explicit.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then_with(|| a.range.end.cmp(&b.range.end))
            .then_with(|| a.name.cmp(&b.name))
    });
    projection.explicit.dedup();
    projection.plain_ranges = plain_search_ranges(raw.len(), &projection);
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

fn og_prefix_allows(raw: &str, start: usize) -> bool {
    let mut preceding = raw.get(..start).unwrap_or_default().chars().rev();
    match preceding.next() {
        None => true,
        Some('[') => preceding.next() != Some('['),
        Some('#') => false,
        Some(_) => true,
    }
}

fn overlaps(range: &Range<usize>, other: &Range<usize>) -> bool {
    range.start < other.end && other.start < range.end
}

/// The first character of `nfd(needle)` when it is a starter (combining class
/// 0), else `None` (no start-character pre-rejection is sound then).
fn needle_base_starter(needle: &str) -> Option<char> {
    let base = needle.chars().next()?.nfd().next()?;
    (canonical_combining_class(base) == 0).then_some(base)
}

/// Necessary condition for a match to start at a source character `first`: a
/// match requires `nfc(lower(span)) == needle`, hence
/// `nfd(lower(span)) == nfd(needle)`. Canonical reordering never moves a
/// starter, so when the first character of `nfd(lower(first))` is a starter it
/// is also the first character of `nfd(lower(span))` and must equal the
/// needle's base. A non-starter there (an orphan combining mark) keeps the
/// candidate, so only provably impossible starts are skipped and the accepted
/// matches are exactly those of the unfiltered scan. Allocation-free: this is
/// the per-character cost of every unlinked-references scan (GH #623).
fn match_may_start_with(first: char, needle_base: Option<char>) -> bool {
    let Some(base) = needle_base else {
        return true;
    };
    if first.is_ascii() {
        // ASCII has no decompositions and lowercases within ASCII.
        return first.to_ascii_lowercase() == base;
    }
    match first.to_lowercase().nfd().next() {
        Some(starter) => canonical_combining_class(starter) != 0 || starter == base,
        None => true,
    }
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
    let needle: String = needle.to_lowercase().nfc().collect();
    let first_requires_boundary = needle.chars().next().is_some_and(|ch| ch.is_alphanumeric());
    let last_requires_boundary = needle
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_alphanumeric());
    let needle_base = needle_base_starter(&needle);
    for (offset, grapheme) in source.grapheme_indices(true) {
        let first = grapheme.chars().next().expect("nonempty grapheme");
        if !match_may_start_with(first, needle_base) {
            continue;
        }
        let start = range.start + offset;
        let mut end = start;
        let mut candidate_raw = String::new();
        let mut matched = false;
        let mut boundaries = source[offset..]
            .grapheme_indices(true)
            .map(|(offset, grapheme)| start + offset + grapheme.len());
        let mut boundary = boundaries.next().expect("nonempty suffix");
        for (relative, ch) in source[offset..].char_indices() {
            candidate_raw.push(ch);
            end = start + relative + ch.len_utf8();
            if end > boundary {
                boundary = boundaries.next().expect("next grapheme");
            }
            let candidate: String = candidate_raw.to_lowercase().nfc().collect();
            if candidate == needle && end == boundary {
                matched = true;
                break;
            }
            // Accept only at a grapheme edge (I-4), while rejecting incompatible
            // prefixes early without allocating an arbitrarily long grapheme.
            let without_last = candidate
                .char_indices()
                .next_back()
                .map_or("", |(index, _)| &candidate[..index]);
            if !needle.starts_with(&candidate) && !needle.starts_with(without_last) {
                break;
            }
        }
        if !matched {
            continue;
        }
        let before = raw
            .get(..start)
            .and_then(|prefix| prefix.chars().next_back());
        let after = raw.get(end..).and_then(|suffix| suffix.chars().next());
        // Exact OG edge semantics: only adjacent ASCII alphanumerics exclude
        // an unlinked match. `_` and continuous CJK are valid boundaries.
        if og_prefix_allows(raw, start)
            && (!first_requires_boundary || !is_og_edge_alphanumeric(before))
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
    #[cfg(any(test, feature = "test-counters"))]
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

#[cfg(any(test, feature = "test-counters"))]
thread_local! {
    static OCCURRENCE_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-counters"))]
pub fn reset_occurrence_constructions() {
    OCCURRENCE_CONSTRUCTIONS.with(|count| count.set(0));
}

#[cfg(any(test, feature = "test-counters"))]
pub fn occurrence_constructions() -> usize {
    OCCURRENCE_CONSTRUCTIONS.with(std::cell::Cell::get)
}

fn projected_reference_matches(
    reference: &ProjectedPageRef,
    names_norm: &[String],
    config: &crate::config::Config,
) -> bool {
    if reference.rule == "explicit_property_key" {
        return config.property_page_key_enabled(&reference.name)
            && names_norm.iter().any(|name| {
                crate::doc::property_key_norm(name)
                    == crate::doc::property_key_norm(&reference.name)
            });
    }
    names_norm
        .iter()
        .any(|name| refs::same_page(name, &reference.name))
}

pub fn occurrences_of_kind_bounded(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> BoundedOccurrences {
    let mut out = Vec::with_capacity(MAX_OCCURRENCES_PER_BLOCK.min(8));
    let mut total = 0usize;
    if kind == ReferenceKind::Explicit {
        for reference in projection.explicit {
            if !projected_reference_matches(reference, names_norm, config) {
                continue;
            }
            total = total.saturating_add(1);
            if out.len() < MAX_OCCURRENCES_PER_BLOCK {
                let _ = push_unique_bounded(
                    &mut out,
                    reference.name.trim(),
                    canonical,
                    kind,
                    ReferenceSpan {
                        start: byte_to_utf16(raw, reference.range.start),
                        end: byte_to_utf16(raw, reference.range.end),
                    },
                    reference.rule,
                );
            }
        }
        return BoundedOccurrences {
            truncated: total > out.len(),
            occurrences: out,
            total,
        };
    }

    for name in names_norm {
        for eligible in projection.plain_ranges {
            visit_plain_matches(raw, eligible, name, |range| {
                if projection
                    .explicit
                    .iter()
                    .any(|reference| overlaps(&range, &reference.range))
                {
                    return true;
                }
                total = total.saturating_add(1);
                if out.len() < MAX_OCCURRENCES_PER_BLOCK {
                    let _ = push_unique_bounded(
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
                }
                true
            });
        }
    }
    out.sort_by_key(|occurrence| (occurrence.span.start, occurrence.span.end));
    BoundedOccurrences {
        truncated: total > out.len(),
        occurrences: out,
        total,
    }
}

pub fn occurrences_of_kind(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> Vec<ReferenceOccurrence> {
    occurrences_of_kind_bounded(raw, projection, canonical, names_norm, kind, config).occurrences
}

/// Cheap membership path used once a result construction budget is closed.
/// It performs no occurrence/string construction and stops at the first hit.
pub fn has_occurrence_kind(
    raw: &str,
    projection: ReferenceSource<'_>,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> bool {
    if kind == ReferenceKind::Explicit {
        return projection
            .explicit
            .iter()
            .any(|reference| projected_reference_matches(reference, names_norm, config));
    }
    for name in names_norm {
        for eligible in projection.plain_ranges {
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

pub fn occurrences(
    raw: &str,
    projection: ReferenceSource<'_>,
    canonical: &str,
    names_norm: &[String],
    config: &crate::config::Config,
) -> Vec<ReferenceOccurrence> {
    let mut out = occurrences_of_kind(
        raw,
        projection,
        canonical,
        names_norm,
        ReferenceKind::Explicit,
        config,
    );
    out.extend(occurrences_of_kind(
        raw,
        projection,
        canonical,
        names_norm,
        ReferenceKind::Plain,
        config,
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
pub fn slow_occurrences(
    raw: &str,
    is_org: bool,
    canonical: &str,
    names_norm: &[String],
    config: &crate::config::Config,
) -> Vec<ReferenceOccurrence> {
    let parsed = crate::render::parse_projection(raw, is_org);
    let source = project(raw, is_org, &parsed.blocks);
    occurrences(raw, source.as_source(), canonical, names_norm, config)
}

#[cfg(test)]
mod tests {

    // GH #623: the start-character pre-rejection must not change which spans
    // match. `reference_visit_plain_matches` is the matcher as it was before the
    // pre-rejection (verbatim); every (text, needle) pair over an alphabet of
    // the awkward cases (case pairs, composed/decomposed accents, final sigma,
    // dotted I, sharp s, Hangul jamo, orphan combining marks, CJK, ZWJ emoji)
    // must give the identical match list.
    fn reference_visit_plain_matches(
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
        let needle: String = needle.to_lowercase().nfc().collect();
        let first_requires_boundary = needle.chars().next().is_some_and(|ch| ch.is_alphanumeric());
        let last_requires_boundary = needle
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric());
        let ascii_first = needle
            .chars()
            .next()
            .filter(char::is_ascii)
            .map(|ch| (ch.to_ascii_lowercase(), ch.to_ascii_uppercase()));
        for (offset, grapheme) in source.grapheme_indices(true) {
            let first = grapheme.chars().next().expect("nonempty grapheme");
            if let Some((lower, upper)) = ascii_first {
                if first.is_ascii() && first != lower && first != upper {
                    continue;
                }
            }
            let start = range.start + offset;
            let mut end = start;
            let mut candidate_raw = String::new();
            let mut matched = false;
            let mut boundaries = source[offset..]
                .grapheme_indices(true)
                .map(|(offset, grapheme)| start + offset + grapheme.len());
            let mut boundary = boundaries.next().expect("nonempty suffix");
            for (relative, ch) in source[offset..].char_indices() {
                candidate_raw.push(ch);
                end = start + relative + ch.len_utf8();
                if end > boundary {
                    boundary = boundaries.next().expect("next grapheme");
                }
                let candidate: String = candidate_raw.to_lowercase().nfc().collect();
                if candidate == needle && end == boundary {
                    matched = true;
                    break;
                }
                // Accept only at a grapheme edge (I-4), while rejecting incompatible
                // prefixes early without allocating an arbitrarily long grapheme.
                let without_last = candidate
                    .char_indices()
                    .next_back()
                    .map_or("", |(index, _)| &candidate[..index]);
                if !needle.starts_with(&candidate) && !needle.starts_with(without_last) {
                    break;
                }
            }
            if !matched {
                continue;
            }
            let before = raw
                .get(..start)
                .and_then(|prefix| prefix.chars().next_back());
            let after = raw.get(end..).and_then(|suffix| suffix.chars().next());
            // Exact OG edge semantics: only adjacent ASCII alphanumerics exclude
            // an unlinked match. `_` and continuous CJK are valid boundaries.
            if og_prefix_allows(raw, start)
                && (!first_requires_boundary || !is_og_edge_alphanumeric(before))
                && (!last_requires_boundary || !is_og_edge_alphanumeric(after))
                && !visit(start..end)
            {
                return;
            }
        }
    }

    #[test]
    fn start_character_prerejection_matches_exactly_what_the_unfiltered_scan_matches() {
        const ALPHABET: &[&str] = &[
            "a",
            "A",
            "e",
            "E",
            "r",
            "R",
            "i",
            "I",
            "s",
            "S",
            "k",
            "K",
            "\u{212A}",
            "\u{e9}",
            "\u{c9}",
            "e\u{301}",
            "E\u{301}",
            "\u{159}",
            "r\u{30c}",
            "\u{158}",
            "\u{3a3}",
            "\u{3c3}",
            "\u{3c2}",
            "\u{130}",
            "i\u{307}",
            "\u{df}",
            "\u{1e9e}",
            "ss",
            "\u{1100}",
            "\u{1161}",
            "\u{11a8}",
            "\u{ac00}",
            "\u{301}",
            "\u{30a}",
            "\u{4e2d}",
            "\u{6587}",
            "\u{1f468}\u{200d}\u{1f469}",
            "\u{f8}",
            "\u{d8}",
            "o\u{338}",
            "\u{c5}",
            "A\u{30a}",
            "\u{212b}",
            " ",
            "-",
            "_",
            "1",
            "/",
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let build = |next: &mut dyn FnMut() -> u64, max: u64| -> String {
            (0..=next() % max)
                .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
                .collect()
        };
        let collect =
            |matcher: &dyn Fn(&str, &Range<usize>, &str, &mut dyn FnMut(Range<usize>) -> bool),
             raw: &str,
             needle: &str| {
                let mut found = Vec::new();
                matcher(raw, &(0..raw.len()), needle, &mut |range| {
                    found.push(range);
                    true
                });
                found
            };
        let mut matched_any = 0usize;
        for _ in 0..40_000 {
            let raw = build(&mut next, 14);
            let needle = build(&mut next, 3);
            let new = collect(
                &|raw, range, needle, visit| visit_plain_matches(raw, range, needle, visit),
                &raw,
                &needle,
            );
            let old = collect(
                &|raw, range, needle, visit| {
                    reference_visit_plain_matches(raw, range, needle, visit)
                },
                &raw,
                &needle,
            );
            assert_eq!(new, old, "raw={raw:?} needle={needle:?}");
            matched_any += usize::from(!old.is_empty());
        }
        assert!(
            matched_any > 500,
            "the alphabet must actually produce matches: {matched_any}"
        );
    }
    use super::*;

    fn evidence(raw: &str, names: &[&str]) -> Vec<ReferenceOccurrence> {
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        occurrences(
            raw,
            projected.as_source(),
            "Target",
            &names
                .iter()
                .map(|name| refs::normalize(name))
                .collect::<Vec<_>>(),
            &crate::config::Config::default(),
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
        let plains = got
            .iter()
            .filter(|hit| hit.kind == ReferenceKind::Plain)
            .collect::<Vec<_>>();
        assert_eq!(plains.len(), 2);
        assert!(plains
            .iter()
            .all(|hit| &raw[hit.span.start..hit.span.end] == "Target"));
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
    fn escaped_link_is_not_plain_but_code_mentions_are() {
        let got = evidence("\\[[Target]] and `Target`\n```\nTarget\n```", &["Target"]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.iter().all(|hit| hit.kind == ReferenceKind::Plain));
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
    fn property_key_projection_uses_canonical_span_and_exact_og_built_ins() {
        let raw = "  Done_At:: today";
        let parsed = crate::render::parse_projection(raw, false);
        let projected = project(raw, false, &parsed.blocks);
        let key = projected
            .explicit
            .iter()
            .find(|reference| reference.rule == "explicit_property_key")
            .unwrap();
        assert_eq!(key.name, "done-at");
        assert_eq!(&raw[key.range.clone()], "Done_At");

        for built_in in OG_EDITABLE_BUILT_IN_PROPERTIES
            .iter()
            .chain(OG_HIDDEN_BUILT_IN_PROPERTIES)
        {
            let raw = format!("{built_in}:: value");
            let parsed = crate::render::parse_projection(&raw, false);
            let projected = project(&raw, false, &parsed.blocks);
            assert!(
                projected
                    .explicit
                    .iter()
                    .all(|reference| reference.rule != "explicit_property_key"),
                "built-in key projected: {built_in}: {:?}",
                projected.explicit
            );
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
            projected.as_source(),
            "Target",
            &[refs::normalize("Target")],
            ReferenceKind::Plain,
            &crate::config::Config::default(),
        );
        assert_eq!(got.len(), MAX_OCCURRENCES_PER_BLOCK);
        assert_eq!(occurrence_constructions(), MAX_OCCURRENCES_PER_BLOCK);
        assert!(got.capacity() <= MAX_OCCURRENCES_PER_BLOCK);
    }

    #[test]
    fn occurrence_cap_reports_total_and_truncation() {
        let raw = "Target ".repeat(70);
        let parsed = crate::render::parse_projection(&raw, false);
        let projected = project(&raw, false, &parsed.blocks);
        let got = occurrences_of_kind_bounded(
            &raw,
            projected.as_source(),
            "Target",
            &[refs::normalize("Target")],
            ReferenceKind::Plain,
            &crate::config::Config::default(),
        );
        assert_eq!(got.occurrences.len(), MAX_OCCURRENCES_PER_BLOCK);
        assert_eq!(got.total, 70);
        assert!(got.truncated);
    }

    #[test]
    fn structural_id_property_is_not_plain_reference_text() {
        let got = evidence("id:: 6a55b643-1234-5678-9abc-def012345678", &["6a55b643"]);
        assert!(
            got.is_empty(),
            "structural id leaked into evidence: {got:?}"
        );
    }

    #[test]
    fn block_reference_uuid_is_not_a_plain_page_mention() {
        let got = evidence(
            "((11111111-1111-4111-8111-111111111111))",
            &["11111111-1111-4111-8111-111111111111"],
        );
        assert!(
            got.is_empty(),
            "block link leaked into page evidence: {got:?}"
        );
    }

    #[test]
    fn unlinked_mentions_include_literal_regions_without_counting_link_syntax() {
        for raw in [
            "`Target`",
            "```\nTarget\n```",
            "$$\nTarget\n$$",
            "<div>Target</div>",
        ] {
            let got = evidence(raw, &["Target"]);
            assert_eq!(
                got.iter()
                    .filter(|hit| hit.kind == ReferenceKind::Plain)
                    .count(),
                1,
                "{raw}: {got:?}"
            );
        }
        for raw in ["```\n[[Target]]\n```", "`#Target`", "\\[[Target]]"] {
            assert!(evidence(raw, &["Target"]).is_empty(), "{raw}");
        }
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
