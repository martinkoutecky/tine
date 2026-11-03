//! The reference walker: which lsdoc nodes of one block source are page
//! references, and the source span of each (moved out of
//! `reference_evidence.rs` unchanged, conceptual model step 2 batch 6).
//!
//! This file also compiles into lsdoc-wasm (`crates/lsdoc-wasm/src/lib.rs`),
//! so the window's swipe check reads the same answer. Every name from outside
//! comes through `super::`: tine-core's `reference_evidence` and the wasm
//! crate root both provide `block_regions`, `og_built_in`,
//! `property_key_norm`, `is_query_macro_name`, `parse_inline_bounded`,
//! `ParseConfig`, `separated_by_commas` and `reference_parsing_suppressed`.

use lsdoc::ast::{Block, Inline, ListItem, Span, Url};
use std::ops::Range;

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
    pub rule: super::block_regions::StaticStr,
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
        "explicit_task_marker",
        "explicit_priority",
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
    /// Collected only for a rename ([`rename_targets`]); never stored.
    #[serde(skip)]
    pub(super) rewrite_targets: Option<Vec<RewriteTarget>>,
}

/// A source span a rename rewrites although it is no page reference
/// (conceptual model step 2 batch 6, class RewriteTarget; D-20261031-CMQ and
/// the manager ruling Q2 of 2026-11-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteTarget {
    /// An Org `[[file:…]]` link node: its path stem names a page.
    FileLink(Range<usize>),
    /// A `[[x]]` page link (rule `explicit_link`) or a nested link (rule
    /// `explicit_nested_link`) inside the argument of a macro other than
    /// `embed` (an embed is a reference) and the query macros (whose
    /// arguments the query parsers read): cloze, renderer, plugin macros.
    MacroLink(ProjectedPageRef),
}

/// The page links of one macro's argument text, as [`RewriteTarget::MacroLink`]s.
fn collect_macro_links(
    name: &str,
    span: Option<&Span>,
    mapper: SpanMapper,
    raw: &str,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    let Some(targets) = projection.rewrite_targets.as_mut() else {
        return;
    };
    if name.eq_ignore_ascii_case("embed") || super::is_query_macro_name(name) {
        return;
    }
    let Some(range) = span.and_then(|span| mapper.map(span, raw.len())) else {
        return;
    };
    // `{{name argument}}`: parse the argument text in place, so its link
    // spans are offsets in `raw`.
    let node = &raw[range.clone()];
    let start = 2 + name.len();
    let Some(argument) = node
        .get(start..node.len().saturating_sub(2))
        .filter(|_| node.starts_with("{{") && node.ends_with("}}"))
    else {
        return;
    };
    let Some(nodes) =
        super::parse_inline_bounded(argument, if is_org { "org" } else { "markdown" })
    else {
        return;
    };
    let mut inner = ReferenceSourceProjection::default();
    walk_inlines(
        &nodes,
        SpanMapper::direct(range.start + start),
        raw,
        is_org,
        &mut inner,
    );
    targets.extend(
        inner
            .explicit
            .into_iter()
            .filter(|r| r.rule == "explicit_link" || r.rule == "explicit_nested_link")
            .map(RewriteTarget::MacroLink),
    );
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

use super::block_regions::{nested_reference_names as nested_names, unbracket};

fn link_page_name(url: &Url, label: &[Inline], is_org: bool) -> Option<String> {
    let mut text = String::new();
    flatten_inlines(label, &mut text);
    let (kind, value) = match url {
        Url::PageRef { v } => ("page_ref", v.as_str()),
        Url::Search { v } => ("search", v.as_str()),
        Url::File { v } => ("file", v.as_str()),
        _ => return None,
    };
    // Evidence names a file link by its label, never its file name, so the
    // file-name format is not consulted.
    super::block_regions::reference_target_name(kind, value, &text, is_org, false, false)
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
    raw: &str,
    is_org: bool,
    projection: &mut ReferenceSourceProjection,
) {
    let raw_len = raw.len();
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
                if is_org && matches!(url, Url::File { .. }) {
                    if let (Some(targets), Some(range)) = (
                        projection.rewrite_targets.as_mut(),
                        span.as_ref().and_then(|span| mapper.map(span, raw_len)),
                    ) {
                        targets.push(RewriteTarget::FileLink(range));
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
                walk_inlines(label, mapper, raw, is_org, projection);
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
                walk_inlines(children, mapper, raw, is_org, projection);
            }
            Inline::Macro { name, span, .. } if name != "embed" => {
                collect_macro_links(name, span.as_ref(), mapper, raw, is_org, projection)
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
                walk_inlines(children, mapper, raw, is_org, projection)
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
    walk_inlines(&item.name, mapper, raw, is_org, projection);
    walk_blocks(&item.content, mapper, raw, is_org, projection);
    for child in &item.items {
        walk_list_item(child, mapper, raw, is_org, projection);
    }
}

/// Whether a normalized property key names a page (OG: every nonempty key
/// except built-ins). Reuses the same cached union as query-table discovery.
pub(super) fn property_key_eligible(norm: &str) -> bool {
    !norm.is_empty() && !super::og_built_in::is_normalized_og_built_in_property(norm)
}

fn project_property_key(
    projection: &mut ReferenceSourceProjection,
    key: &str,
    key_range: Range<usize>,
    raw_len: usize,
) {
    let norm = super::property_key_norm(key);
    if property_key_eligible(&norm) {
        push_explicit_range(
            projection,
            norm,
            key_range,
            raw_len,
            "explicit_property_key",
        );
    }
}

pub(super) fn quoted_linkable_value(value: &str) -> bool {
    let whole = value.trim();
    whole.len() >= 2 && whole.starts_with('"') && whole.ends_with('"')
}

fn project_implicit_linkable_property(
    projection: &mut ReferenceSourceProjection,
    key: &str,
    value_offset: usize,
    value: &str,
    raw_len: usize,
    nodes: &[Inline],
    config: &super::ParseConfig,
) {
    let key = super::property_key_norm(key);
    if !super::separated_by_commas(&key, config)
        || super::reference_parsing_suppressed(&key, config)
    {
        return;
    }
    if quoted_linkable_value(value) {
        return;
    }

    // OG extract-refs-by-commas reads only top-level Plain nodes. lsdoc owns
    // syntax and spans, so code, emphasis and link labels cannot become atoms.
    for node in nodes {
        let Inline::Plain {
            text,
            span: Some(span),
            ..
        } = node
        else {
            continue;
        };
        let Some(range) = SpanMapper::direct(value_offset).map(&span, raw_len) else {
            continue;
        };
        let mut segment_start = 0;
        for (index, separator) in text
            .char_indices()
            .filter(|(_, ch)| super::block_regions::is_linkable_property_separator(*ch))
            .map(|(index, ch)| (index, ch.len_utf8()))
            .chain(std::iter::once((text.len(), 0)))
        {
            let segment = &text[segment_start..index];
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
                let start = range.start + segment_start + leading;
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
}

fn structural_property(key: &str, raw: &str, is_org: bool) -> bool {
    (key.eq_ignore_ascii_case("id") && super::block_regions::parse(raw, is_org).id.is_some())
        || key.eq_ignore_ascii_case("collapsed")
        || key.to_ascii_lowercase().starts_with("logseq.")
}

// lsdoc has already recognized and removed these tags from the inline title.
// Locate only that accepted suffix inside its header span; this does not
// recognize tag syntax in arbitrary content or reparse the document.
fn project_headline_tags(
    tags: &[String],
    span: Option<&Span>,
    mapper: SpanMapper,
    raw: &str,
    projection: &mut ReferenceSourceProjection,
) {
    if tags.is_empty() {
        return;
    }
    let Some(span) = span else { return };
    // The first header starts before the synthetic "* " prefix. Its source
    // begins at raw byte zero; later headers use their ordinary mapped span.
    let Some(range) = mapper.map(&Span(span.0.max(mapper.prefix), span.1), raw.len()) else {
        return;
    };
    let header = raw[range.clone()]
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end();
    let suffix = format!(":{}:", tags.join(":"));
    let Some(before) = header.strip_suffix(&suffix) else {
        return;
    };
    let mut start = range.start + before.len() + 1;
    for tag in tags {
        let end = start + tag.len();
        push_explicit_range(
            projection,
            tag.clone(),
            start..end,
            raw.len(),
            "explicit_tag",
        );
        start = end + 1;
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
            Block::Heading {
                inline,
                htags,
                span,
                ..
            }
            | Block::Bullet {
                inline,
                htags,
                span,
                ..
            } => {
                walk_inlines(inline, mapper, raw, is_org, projection);
                project_headline_tags(htags, span.as_ref(), mapper, raw, projection);
            }
            Block::Paragraph { inline, .. } | Block::FootnoteDef { inline, .. } => {
                walk_inlines(inline, mapper, raw, is_org, projection)
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
                        walk_inlines(cell, mapper, raw, is_org, projection);
                    }
                }
                for row in rows {
                    for cell in row {
                        walk_inlines(cell, mapper, raw, is_org, projection);
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

/// Shared evidence projection using the graph's linkable-property policy.
/// `regions` is `block_regions::from_blocks(raw, is_org, blocks)`, which the
/// caller has already computed for the same block (GJ5 S3: not twice).
pub fn project_with_config(
    raw: &str,
    is_org: bool,
    blocks: &[Block],
    regions: &super::block_regions::BlockRegions,
    config: &super::ParseConfig,
) -> ReferenceSourceProjection {
    let mut projection = ReferenceSourceProjection::default();
    project_into(&mut projection, raw, is_org, blocks, regions, config);
    projection
}

pub(super) fn project_into(
    projection: &mut ReferenceSourceProjection,
    raw: &str,
    is_org: bool,
    blocks: &[Block],
    regions: &super::block_regions::BlockRegions,
    config: &super::ParseConfig,
) {
    let mapper = SpanMapper::block(raw);
    walk_blocks(blocks, mapper, raw, is_org, projection);
    // OG graph-parser/block.cljs `with-page-refs` (0.10.9 and 1.0.0) concatenates
    // `[marker priority]` into the BLOCK's refs, not task-looking headings
    // nested in its body. The first parser header owns both, just as it owns the
    // DTO's task and priority facets. A priority `[#A]` references the page "A"
    // (OG lowercases to `:block/name "a"`), as TODO references the page "todo".
    if let Some(
        Block::Heading {
            marker,
            priority,
            span: Some(span),
            ..
        }
        | Block::Bullet {
            marker,
            priority,
            span: Some(span),
            ..
        },
    ) = blocks.first()
    {
        if marker.is_some() || priority.is_some() {
            if let Some(range) = mapper.map(&Span(span.0.max(mapper.prefix), span.1), raw.len()) {
                let header = raw[range.clone()].lines().next().unwrap_or_default();
                let mut after_marker = 0;
                if let Some(marker) = marker {
                    if let Some(offset) = header.find(marker.as_str()) {
                        let start = range.start + offset;
                        push_explicit_range(
                            projection,
                            marker.to_owned(),
                            start..start + marker.len(),
                            raw.len(),
                            "explicit_task_marker",
                        );
                        after_marker = offset + marker.len();
                    }
                }
                if let Some(priority) = priority {
                    // The parser's accepted header token is `[#P]`; it follows
                    // the marker, so search from there.
                    let token = format!("[#{priority}]");
                    if let Some(offset) = header[after_marker..].find(&token) {
                        let start = range.start + after_marker + offset;
                        push_explicit_range(
                            projection,
                            priority.to_owned(),
                            start..start + token.len(),
                            raw.len(),
                            "explicit_priority",
                        );
                    }
                }
            }
        }
    }
    for property in regions.properties.iter().filter(|p| p.applicable) {
        let key = &property.key;
        let key_range = property.key_range.0..property.key_range.1;
        let offset = property.value_range.0;
        let value = property.value_range.slice(raw);
        project_property_key(projection, key, key_range.clone(), raw.len());
        if structural_property(key, raw, is_org) {
            projection
                .withheld_ranges
                .push(key_range.start..property.value_range.1);
            continue;
        }
        if let Some(nodes) =
            super::parse_inline_bounded(value, if is_org { "org" } else { "markdown" })
        {
            walk_inlines(&nodes, SpanMapper::direct(offset), raw, is_org, projection);
            project_implicit_linkable_property(
                projection,
                key,
                offset,
                value,
                raw.len(),
                &nodes,
                config,
            );
        }
    }

    projection.explicit.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then_with(|| a.range.end.cmp(&b.range.end))
            .then_with(|| a.name.cmp(&b.name))
    });
    projection.explicit.dedup();
    projection.plain_ranges = plain_search_ranges(raw.len(), projection);
}
