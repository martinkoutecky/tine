//! Parser-owned regions and source edits for ONE raw outline block (I-4/I-12).
//!
//! `parse` answers header, literal, property, planning, drawer and identity ownership
//! in raw UTF-8 byte coordinates. `from_blocks` answers the same questions from an
//! existing single-block AST, without parsing. Both cost O(bytes in this block +
//! AST nodes), never O(page/graph). `BlockRegions` can be retained while raw is
//! unchanged; callers must not know parser preparation, wrapper syntax or placement.
//!
//! `edit` / `apply` set/remove properties and planning, strip copy metadata, project
//! the visible body, normalize planning, and insert drawer rows. Only parser-owned
//! ranges can be replaced; new metadata is placed outside literals. No file I/O.
//! An invalid request returns an error; a quarantined parse refuses edits. Callers
//! must surface that refusal. Debug builds reparse and verify literal preservation.
//! Sub-token scans below are confined to regions lsdoc has ALREADY accepted.
//! Org properties are primary only in the canonical head drawer; `Visible`
//! removes that same region, retaining property-shaped drawers in the body.
//! `parse_document` answers literal and page-property ownership in whole-file
//! coordinates, O(file bytes + AST nodes). Org directives are readable page
//! properties, but never primary block properties for structural editing.

use lsdoc::ast::{Block, Inline, ListItem, Span};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// OG linkable-property member separator, shared by native references and wasm.
/// `graph_parser/text.cljs::sep-by-comma` at Logseq c67b8b5fa splits both commas.
pub fn is_linkable_property_separator(ch: char) -> bool {
    ch == ',' || ch == '，'
}

#[path = "page_filename.rs"]
mod reference_filename;

/// Interpret an already accepted link target. Evidence uses File labels;
/// filename candidates use decoded page filenames, preserving the existing
/// rename-candidate policy. Unlabeled local assets never name pages. O(target).
pub fn reference_target_name(
    kind: &str,
    value: &str,
    label: &str,
    org: bool,
    filename_candidates: bool,
) -> Option<String> {
    let path = value.strip_prefix("file:").unwrap_or(value);
    let local_asset =
        path.trim_start_matches(['.', '/']).starts_with("assets") || path.starts_with("draws");
    match kind {
        "page_ref" if !local_asset => Some(value.to_owned()),
        "search" if value.trim().starts_with("[[") && value.trim().ends_with("]]") => {
            Some(unbracket(value).to_owned())
        }
        "search" if org && !local_asset => Some(value.to_owned()),
        "file" if filename_candidates && !local_asset => {
            let file = path.rsplit('/').next().unwrap_or(value);
            let stem = file
                .rfind('.')
                .filter(|&i| i > 0)
                .map_or(file, |i| &file[..i]);
            Some(reference_filename::decode_page_name(stem, false))
        }
        "file" if !label.trim().is_empty() && label != value => Some(label.to_owned()),
        _ => None,
    }
}

pub fn unbracket(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("[[")
        .and_then(|rest| rest.strip_suffix("]]"))
        .unwrap_or(value)
}

pub fn nested_reference_names(content: &str) -> Vec<String> {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Range(pub usize, pub usize);
impl Range {
    pub fn slice<'a>(&self, raw: &'a str) -> &'a str {
        &raw[self.0..self.1]
    }
    pub fn contains(&self, at: usize) -> bool {
        self.0 <= at && at < self.1
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Header {
    pub marker: Option<String>,
    pub priority: Option<String>,
    pub heading: Option<u32>,
    /// Source bytes of the accepted marker / `[#X]` priority token (I-12: one
    /// answer for readers and writers). Located inside the accepted header only.
    pub marker_range: Option<Range>,
    pub priority_range: Option<Range>,
}

impl Header {
    /// lsdoc accepted `marker`/`priority` as the first tokens of the block, so
    /// the first match of each accepted text IS that token: only skipped
    /// whitespace (and a Markdown heading prefix) can precede it. O(block bytes).
    fn locate(&mut self, raw: &str) {
        let mut from = 0;
        if let Some(marker) = &self.marker {
            if let Some(at) = raw.find(marker.as_str()) {
                from = at + marker.len();
                self.marker_range = Some(Range(at, from));
            }
        }
        if let Some(priority) = &self.priority {
            self.priority_range = raw[from..].match_indices("[#").find_map(|(at, _)| {
                let rest = &raw[from + at + 2..];
                let end = rest.find(']')?;
                rest[..end]
                    .eq_ignore_ascii_case(priority)
                    .then(|| Range(from + at, from + at + 2 + end + 1))
            });
        }
    }
}

/// A block-level literal container lsdoc accepted (fence, `#+BEGIN_SRC`,
/// example, export, comment, `$$` math, LaTeX environment, raw HTML, custom).
/// `range` is the whole container including any trailing blank lines lsdoc
/// folded in; `open_end` ends its first (opener) line and `close_start` begins
/// its last non-blank (closer) line. Line arithmetic only, never fence syntax.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiteralBlock {
    pub kind: &'static str,
    pub lang: String,
    pub range: Range,
    pub open_end: usize,
    pub close_start: usize,
    /// End of the opener's delimiter token (the backtick/tilde run, or
    /// `#+BEGIN_X`); the info string / language follows it.
    pub delim_end: usize,
}

/// The editor-state policy for a fence still being typed: lsdoc forms no
/// container without a closer, but Tine's editor treats the first fence-looking
/// line outside every accepted literal as an open fence running to the end of
/// the text. It is the only fence-shaped recognition outside the parser and is
/// computed only here (guarded by `src/ogP11Parser.guard.test.ts`, I-12).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenFence {
    pub lang: String,
    pub start: usize,
    pub open_end: usize,
    pub delim_end: usize,
}

fn opener_delim_end(opener: &str) -> usize {
    let t = opener.trim_start();
    let lead = opener.len() - t.len();
    let c = t.as_bytes().first().copied();
    lead + if matches!(c, Some(b'`' | b'~')) {
        t.bytes().take_while(|&b| Some(b) == c).count()
    } else {
        t.find(char::is_whitespace).unwrap_or(t.len())
    }
}

impl LiteralBlock {
    fn new(raw: &str, kind: &'static str, lang: &str, range: Range) -> Self {
        let text = range.slice(raw);
        let open_end = range.0 + text.find('\n').map_or(text.len(), |at| at + 1);
        let body = text.trim_end();
        let close_start = range.0 + body.rfind('\n').map_or(0, |at| at + 1);
        let delim_end = range.0 + opener_delim_end(&raw[range.0..open_end]);
        LiteralBlock {
            kind,
            lang: lang.to_owned(),
            range,
            open_end,
            close_start,
            delim_end,
        }
    }
}

fn open_fence(raw: &str, literals: &[Range]) -> Option<OpenFence> {
    if !(raw.contains("```") || raw.contains("~~~") || raw.contains("#+")) {
        return None;
    }
    let (mut at, mut cursor) = (0, 0);
    for line in raw.split_inclusive('\n') {
        let start = at;
        at += line.len();
        while literals.get(cursor).is_some_and(|r| r.1 <= start) {
            cursor += 1;
        }
        if literals.get(cursor).is_some_and(|r| r.contains(start)) {
            continue;
        }
        let t = line.trim_start();
        let opens = t.starts_with("```")
            || t.starts_with("~~~")
            || ["#+begin_src", "#+begin_example"]
                .iter()
                .any(|n| t.get(..n.len()).is_some_and(|h| h.eq_ignore_ascii_case(n)));
        if opens {
            let delim_end = start + opener_delim_end(line);
            let fence_start = start + (line.len() - t.len());
            let info = raw[delim_end..start + line.trim_end().len()].trim();
            return Some(OpenFence {
                lang: info.split_whitespace().next().unwrap_or("").to_owned(),
                start: fence_start,
                open_end: start + line.len(),
                delim_end,
            });
        }
    }
    None
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Property {
    pub key: String,
    pub value: String,
    pub line: Range,
    pub key_range: Range,
    pub value_range: Range,
    pub region: usize,
    pub primary: bool,
    #[serde(skip_serializing)]
    pub directive: bool,
    #[serde(skip_serializing)]
    pub applicable: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Planning {
    pub kind: String,
    pub line: Range,
    pub timestamp: Range,
    pub date: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drawer {
    pub name: String,
    pub range: Range,
    pub close: usize,
    pub clocks: Vec<Range>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BlockRegions {
    pub header: Header,
    pub literals: Vec<Range>,
    pub property_regions: Vec<Range>,
    pub properties: Vec<Property>,
    pub planning: Vec<Planning>,
    pub drawers: Vec<Drawer>,
    pub id: Option<Property>,
    pub quarantined: bool,
    /// Block-level literal containers in source order (see [`LiteralBlock`]).
    pub literal_blocks: Vec<LiteralBlock>,
    /// A fence being typed, when no accepted container closes it.
    pub open_fence: Option<OpenFence>,
}

// The only conversion from prepared single-block coordinates to raw coordinates.
fn raw_range(raw: &str, span: &Option<Span>, document: bool) -> Option<Range> {
    let Span(start, end) = span.as_ref()?;
    let lead = if document {
        0
    } else {
        raw.len() - raw.trim_start().len()
    };
    let wrapper = if document { 0 } else { 2 };
    let r = Range(
        (start.saturating_sub(wrapper) + lead).min(raw.len()),
        (end.saturating_sub(wrapper) + lead).min(raw.len()),
    );
    (r.0 < r.1 && raw.is_char_boundary(r.0) && raw.is_char_boundary(r.1)).then_some(r)
}
fn whole_line(raw: &str, r: Range) -> Range {
    let start = raw[..r.0].rfind('\n').map_or(0, |p| p + 1);
    let end = if r.1 > start && raw.as_bytes()[r.1 - 1] == b'\n' {
        r.1
    } else {
        raw[r.1..].find('\n').map_or(raw.len(), |p| r.1 + p + 1)
    };
    Range(start, end)
}
fn line_ranges(raw: &str, range: Range) -> Vec<Range> {
    let mut at = range.0;
    range
        .slice(raw)
        .split_inclusive('\n')
        .map(|s| {
            let r = Range(at, at + s.len());
            at += s.len();
            r
        })
        .collect()
}
fn trimmed_range(raw: &str, range: Range) -> Range {
    let s = range.slice(raw);
    Range(
        range.0 + s.len() - s.trim_start().len(),
        range.1 - (s.len() - s.trim_end().len()),
    )
}

/// Parse one raw block with the same boundary used by render. No page work.
pub fn parse(raw: &str, is_org: bool) -> BlockRegions {
    #[cfg(not(target_arch = "wasm32"))]
    let blocks = match std::panic::catch_unwind(|| crate::render::parse_block(raw, is_org)) {
        Ok(blocks) => blocks,
        Err(_) => {
            return BlockRegions {
                literals: vec![Range(0, raw.len())],
                quarantined: true,
                ..BlockRegions::default()
            }
        }
    };
    #[cfg(target_arch = "wasm32")]
    let blocks = crate::render::parse_block(raw, is_org);
    from_blocks(raw, is_org, &blocks)
}

/// Derive regions from render's existing AST; O(block bytes), zero parses.
pub fn from_blocks(raw: &str, is_org: bool, blocks: &[Block]) -> BlockRegions {
    mapped_regions(raw, is_org, blocks, false)
}

/// Parser-owned regions for a whole file or preamble, without a block wrapper.
/// Refused parser input is quarantined and wholly literal; O(source bytes).
pub fn parse_document(raw: &str, is_org: bool) -> BlockRegions {
    let Some(parsed) = crate::render::parse_text_bounded(raw, is_org) else {
        return BlockRegions {
            literals: vec![Range(0, raw.len())],
            quarantined: true,
            ..Default::default()
        };
    };
    mapped_regions(raw, is_org, &parsed.blocks, true)
}

/// The accepted header of a block's first node, with token spans when `locate`.
fn header_of(raw: &str, blocks: &[Block], locate: bool) -> Header {
    let Some(
        Block::Bullet {
            marker,
            priority,
            size,
            ..
        }
        | Block::Heading {
            marker,
            priority,
            size,
            ..
        },
    ) = blocks.first()
    else {
        return Header::default();
    };
    let mut header = Header {
        marker: marker.clone(),
        priority: priority.clone(),
        heading: *size,
        ..Header::default()
    };
    if locate {
        header.locate(raw);
    }
    header
}

/// Only the accepted header (marker, priority, their spans) of one raw block: the same
/// answer as `parse(raw, is_org).header`, without the regions walk or its JSON. Typing paths
/// that need just the task marker use this lighter door. O(block bytes).
pub fn header_tokens(raw: &str, is_org: bool) -> Header {
    header_of(raw, &crate::render::parse_block(raw, is_org), true)
}

fn mapped_regions(raw: &str, is_org: bool, blocks: &[Block], document: bool) -> BlockRegions {
    let mut result = BlockRegions::default();
    result.header = header_of(raw, blocks, !document);
    visit_blocks(raw, is_org, blocks, document, &mut result);
    result.literals.sort_by_key(|r| r.0);
    let mut merged: Vec<Range> = Vec::new();
    for r in result.literals.drain(..) {
        if let Some(last) = merged.last_mut().filter(|last| r.0 <= last.1) {
            last.1 = last.1.max(r.1);
        } else {
            merged.push(r);
        }
    }
    result.literals = merged;
    result.literal_blocks.sort_by_key(|b| b.range.0);
    if !document {
        result.open_fence = open_fence(raw, &result.literals);
    }
    // Nested content under a literal Custom belongs to that container, even if
    // lsdoc emitted child Properties or timestamps there.
    exclude_literals(&mut result.properties, &result.literals, |p| p.line.0);
    exclude_literals(&mut result.planning, &result.literals, |p| p.line.0);
    exclude_literals(&mut result.drawers, &result.literals, |p| p.range.0);
    let own = result.own_org_region(raw);
    for p in &mut result.properties {
        p.primary = p.primary && (!is_org || Some(p.region) == own);
    }
    result.id = result
        .properties
        .iter()
        .find(|p| p.primary && p.key.eq_ignore_ascii_case("id"))
        .cloned();
    result
}

// AST visitors append entries in source order. A single interval cursor per
// entry kind keeps whole-file ownership filtering linear in entries + literals.
fn exclude_literals<T>(entries: &mut Vec<T>, literals: &[Range], start: impl Fn(&T) -> usize) {
    let mut cursor = 0;
    entries.retain(|entry| {
        let at = start(entry);
        while literals.get(cursor).is_some_and(|r| r.1 <= at) {
            cursor += 1;
        }
        !literals.get(cursor).is_some_and(|r| r.contains(at))
    });
}

fn visit_blocks(raw: &str, org: bool, blocks: &[Block], document: bool, out: &mut BlockRegions) {
    for block in blocks {
        match block {
            Block::Src { span, .. }
            | Block::Example { span, .. }
            | Block::Export { span, .. }
            | Block::CommentBlock { span, .. }
            | Block::DisplayedMath { span, .. }
            | Block::LatexEnv { span, .. }
            | Block::RawHtml { span, .. } => {
                if let Some(r) = raw_range(raw, span, document) {
                    out.literals.push(r);
                    let (kind, lang) = match block {
                        Block::Src { lang, .. } => ("src", lang.as_str()),
                        Block::Example { .. } => ("example", ""),
                        _ => ("other", ""),
                    };
                    out.literal_blocks
                        .push(LiteralBlock::new(raw, kind, lang, r));
                }
            }
            Block::Custom { span, children, .. } => {
                if let Some(r) = raw_range(raw, span, document) {
                    out.literals.push(r);
                    out.literal_blocks
                        .push(LiteralBlock::new(raw, "other", "", r));
                }
                visit_blocks(raw, org, children, document, out);
            }
            Block::Quote { children, .. } => visit_blocks(raw, org, children, document, out),
            Block::Paragraph { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::Heading { inline, .. }
            | Block::FootnoteDef { inline, .. } => visit_inline(raw, inline, document, out),
            Block::List { items, .. } => visit_items(raw, org, items, document, out),
            Block::Table { header, rows, .. } => {
                for row in header.iter().chain(rows) {
                    for cell in row {
                        visit_inline(raw, cell, document, out);
                    }
                }
            }
            Block::Properties { props, span } => {
                if let Some(r) = raw_range(raw, span, document) {
                    let r = whole_line(raw, r);
                    let index = out.property_regions.len();
                    out.property_regions.push(r);
                    // Index accepted parser keys once, rather than searching
                    // the entire folded property node for every source entry.
                    let mut names = HashMap::new();
                    for property in props {
                        let name = &property.0;
                        names.entry(name.to_ascii_lowercase()).or_insert(name);
                    }
                    for line in line_ranges(raw, r) {
                        let t = trimmed_range(raw, line);
                        let s = t.slice(raw);
                        // A Properties node can fold Markdown lines, a drawer
                        // and directives together. Split only this accepted node;
                        // preserve each entry's source spelling and syntax.
                        let markdown = s.split_once("::").map(|(k, v)| (k, v, 0, 2, !org));
                        let directive = s
                            .strip_prefix("#+")
                            .and_then(|s| s.split_once(':'))
                            .map(|(k, v)| (k, v, 2, 1, false));
                        let drawer = s
                            .strip_prefix(':')
                            .and_then(|s| s.split_once(':'))
                            .map(|(k, v)| (k, v, 1, 1, org));
                        let candidates = if org {
                            [directive, drawer, markdown]
                        } else {
                            [markdown, directive, drawer]
                        };
                        // Accepted entries of the other format still own spans
                        // (e.g. template id removal), but are not active metadata.
                        let Some((key, value, prefix, delim, appropriate)) = candidates
                            .into_iter()
                            .flatten()
                            .find(|(key, ..)| names.contains_key(&key.to_ascii_lowercase()))
                        else {
                            continue;
                        };
                        let name = names[&key.to_ascii_lowercase()];
                        let ks = t.0 + prefix;
                        let vs = ks + key.len() + delim;
                        let value_range = trimmed_range(raw, Range(vs, t.1));
                        out.properties.push(Property {
                            key: name.clone(),
                            value: value.trim().to_string(),
                            line,
                            key_range: Range(ks, ks + key.len()),
                            value_range,
                            region: index,
                            primary: appropriate,
                            directive: org && prefix == 2,
                            applicable: appropriate || (org && prefix == 2),
                        });
                    }
                }
            }
            Block::Directive { name, value, span } if org => {
                if let Some(r) = raw_range(raw, span, document) {
                    let line = whole_line(raw, r);
                    let region = out.property_regions.len();
                    out.property_regions.push(line);
                    // Locate sub-tokens only inside lsdoc's accepted directive.
                    let t = trimmed_range(raw, line);
                    if let Some(colon) = t.slice(raw).find(':') {
                        let key_range = Range(t.0 + 2, t.0 + colon);
                        let value_range = trimmed_range(raw, Range(t.0 + colon + 1, t.1));
                        out.properties.push(Property {
                            key: name.clone(),
                            value: value.clone(),
                            line,
                            key_range,
                            value_range,
                            region,
                            primary: false,
                            directive: true,
                            applicable: true,
                        });
                    }
                }
            }
            Block::Drawer { name, span } => {
                if let Some(r) = raw_range(raw, span, document) {
                    let r = whole_line(raw, r);
                    let lines = line_ranges(raw, r);
                    if let Some(last) = lines.last() {
                        let clocks = if name.eq_ignore_ascii_case("LOGBOOK") {
                            lines
                                .iter()
                                .skip(1)
                                .take(lines.len().saturating_sub(2))
                                .filter(|r| r.slice(raw).trim_start().starts_with("CLOCK:"))
                                .copied()
                                .collect()
                        } else {
                            Vec::new()
                        };
                        out.drawers.push(Drawer {
                            name: name.clone(),
                            range: r,
                            close: last.0,
                            clocks,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}
fn visit_items(raw: &str, org: bool, items: &[ListItem], document: bool, out: &mut BlockRegions) {
    for item in items {
        visit_blocks(raw, org, &item.content, document, out);
        visit_inline(raw, &item.name, document, out);
        visit_items(raw, org, &item.items, document, out);
    }
}
fn visit_inline(raw: &str, inline: &[Inline], document: bool, out: &mut BlockRegions) {
    for i in inline {
        match i {
            Inline::Code { span, .. } | Inline::Verbatim { span, .. } => {
                if let Some(r) = raw_range(raw, span, document) {
                    out.literals.push(r);
                }
            }
            Inline::Timestamp { ts, date, span }
                if matches!(ts.as_str(), "Scheduled" | "Deadline" | "Closed") =>
            {
                if let Some(r) = raw_range(raw, span, document) {
                    let line = whole_line(raw, r);
                    // The Timestamp owns the planning prefix. A glued suffix
                    // remains body text; only whitespace may precede the token.
                    if raw[line.0..r.0].trim().is_empty() {
                        out.planning.push(Planning {
                            kind: ts.clone(),
                            line,
                            timestamp: r,
                            date: date.clone(),
                        });
                    }
                }
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => visit_inline(raw, children, document, out),
            Inline::Link { label, .. } => visit_inline(raw, label, document, out),
            _ => {}
        }
    }
}

/// One Markdown page-header property: the parser's accepted key and value, its
/// 0-based line index, and the byte range of its line without the newline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Leading Markdown page-header properties (see [`page_header`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PageHeader {
    /// Byte end of the last header line, newline excluded; 0 with no entries.
    pub end: usize,
    pub entries: Vec<HeaderEntry>,
}

/// The page header of a Markdown page preamble or first root block: the parser's
/// accepted properties (`parse_document`; literals and no-space `key::value`
/// excluded) that form the leading run of the text, joined only by empty lines.
/// Named Tine policies over that answer: the run starts at byte 0, every line
/// starts at column zero, and a key may not start with `#` (a `#tag::` line is
/// prose that merely parses). A parser-refused input has no header. O(bytes).
pub fn page_header(raw: &str) -> PageHeader {
    let regions = parse_document(raw, false);
    let mut header = PageHeader::default();
    if regions.quarantined {
        return header;
    }
    let (mut at, mut line_no) = (0, 0);
    for p in regions.properties.iter().filter(|p| p.primary) {
        let gap = &raw[at..p.line.0.max(at)];
        let eligible = p.line.0 >= at
            && gap.bytes().all(|b| b == b'\n' || b == b'\r')
            && p.key_range.0 == p.line.0
            && !p.key.starts_with('#')
            && (!header.entries.is_empty() || p.line.0 == 0);
        if !eligible {
            break;
        }
        line_no += gap.matches('\n').count();
        let end = p.line.1 - usize::from(raw.as_bytes()[p.line.1 - 1] == b'\n');
        header.entries.push(HeaderEntry {
            key: p.key.clone(),
            value: p.value.clone(),
            line: line_no,
            start: p.line.0,
            end,
        });
        header.end = end;
        at = p.line.1;
        line_no += 1;
    }
    header
}

/// The page header of `raw` when the text is nothing but header properties (no
/// leading or trailing newline, at least one entry). Single answerer (I-12) for
/// "is this first block a page-properties block": a header line is a parser
/// property, so `key::value` (no space) and literal-fence content are prose.
pub fn page_header_only(raw: &str) -> Option<PageHeader> {
    if raw.is_empty() || raw.starts_with('\n') || raw.ends_with('\n') {
        return None;
    }
    let header = page_header(raw);
    (!header.entries.is_empty() && header.end == raw.len()).then_some(header)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edit {
    Property {
        key: String,
        value: Option<String>,
    },
    Properties {
        values: Vec<(String, String)>,
    },
    Planning {
        which: String,
        value: Option<String>,
    },
    StripCopy {
        template: bool,
    },
    Visible,
    NormalizePlanning,
    DrawerRow {
        name: String,
        value: String,
    },
    ClockOut {
        value: String,
    },
}

fn splice(raw: &str, mut edits: Vec<(Range, String)>) -> String {
    edits.sort_by_key(|(r, _)| r.0);
    let mut merged: Vec<(Range, String)> = Vec::new();
    for (r, text) in edits {
        if let Some((previous, replacement)) = merged.last_mut() {
            if replacement.is_empty() && text.is_empty() && previous.1 == r.0 {
                previous.1 = r.1;
                continue;
            }
        }
        merged.push((r, text));
    }
    let mut edits = merged;
    let preceding_end = edits.iter().rev().nth(1).map_or(0, |(r, _)| r.1);
    if let Some((r, _)) = edits
        .last_mut()
        .filter(|(r, text)| text.is_empty() && r.1 == raw.len() && r.0 > 0 && !raw.ends_with('\n'))
    {
        if raw[..r.0].ends_with("\r\n") && r.0 >= preceding_end + 2 {
            r.0 -= 2;
        } else if raw[..r.0].ends_with('\n') && r.0 > preceding_end {
            r.0 -= 1;
        }
    }
    let mut out = String::with_capacity(raw.len());
    let mut at = 0;
    for (r, text) in edits {
        assert!(r.0 >= at);
        out.push_str(&raw[at..r.0]);
        out.push_str(&text);
        at = r.1;
    }
    out.push_str(&raw[at..]);
    out
}
fn newline(raw: &str) -> &'static str {
    if raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}
impl BlockRegions {
    /// Page metadata in source order: native Markdown properties, or the head
    /// Org drawer and Org directives. Literal and body-drawer entries excluded.
    pub fn page_properties(&self) -> impl Iterator<Item = &Property> {
        self.properties.iter().filter(|p| p.primary || p.directive)
    }
    fn planning_removal(raw: &str, p: &Planning) -> Range {
        if raw[p.timestamp.1..p.line.1].trim().is_empty() {
            p.line
        } else {
            Range(p.line.0, p.timestamp.1)
        }
    }
    pub fn property(&self, key: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.key.eq_ignore_ascii_case(key))
    }
    pub fn literal_at(&self, at: usize) -> bool {
        self.literals.iter().any(|r| r.contains(at))
    }
    fn hoist_planning(&self, raw: &str, after: &str) -> String {
        let mut entries: Vec<&Planning> = self
            .planning
            .iter()
            .filter(|p| p.kind != "Closed")
            .collect();
        entries.sort_by_key(|p| if p.kind == "Scheduled" { 0 } else { 1 });
        let at = self.title_end(raw);
        if entries.is_empty() || entries.iter().any(|p| p.line.0 < at) {
            return raw.to_string();
        }
        let mut text = entries
            .iter()
            .map(|p| p.timestamp.slice(raw))
            .collect::<Vec<_>>()
            .join(newline(raw));
        if !after.is_empty() {
            text.push_str(newline(raw));
            text.push_str(after);
        }
        text.push_str(newline(raw));
        let mut edits: Vec<_> = entries
            .iter()
            .map(|p| (Self::planning_removal(raw, p), String::new()))
            .collect();
        if let Some((_, replacement)) = edits.iter_mut().find(|(r, _)| r.0 == at) {
            *replacement = text;
        } else {
            edits.push((Range(at, at), text));
        }
        splice(raw, edits)
    }
    fn title_end(&self, raw: &str) -> usize {
        // If the first thing is a literal container, insert after it, never into it.
        if let Some(r) = self
            .literals
            .iter()
            .find(|r| r.0 == raw.len() - raw.trim_start().len())
        {
            whole_line(raw, *r).1
        } else {
            raw.find('\n').map_or(raw.len(), |p| p + 1)
        }
    }
    fn head_end(&self, raw: &str) -> usize {
        let mut at = self.title_end(raw);
        loop {
            if let Some(p) = self.planning.iter().find(|p| p.line.0 == at) {
                at = p.line.1;
            } else {
                break;
            }
        }
        at
    }
    fn own_org_region(&self, raw: &str) -> Option<usize> {
        let at = self.head_end(raw);
        self.property_regions.iter().position(|r| {
            (r.0 == at || r.0 == 0)
                && r.slice(raw)
                    .lines()
                    .next()
                    .is_some_and(|line| line.trim().eq_ignore_ascii_case(":PROPERTIES:"))
        })
    }
    fn inserted(&self, raw: &str, at: usize, text: &str) -> String {
        let nl = newline(raw);
        let prefix = if at > 0 && !raw[..at].ends_with('\n') {
            nl
        } else {
            ""
        };
        let suffix = if at < raw.len() { nl } else { "" };
        splice(
            raw,
            vec![(Range(at, at), format!("{prefix}{text}{suffix}"))],
        )
    }
    fn remove_properties(
        &self,
        raw: &str,
        org: bool,
        remove: impl Fn(&Property) -> bool,
    ) -> String {
        let selected: Vec<&Property> = self.properties.iter().filter(|p| remove(p)).collect();
        let mut edits = Vec::new();
        for (index, r) in self.property_regions.iter().enumerate() {
            let all: Vec<&Property> = self
                .properties
                .iter()
                .filter(|p| p.region == index)
                .collect();
            if org && !all.is_empty() && all.iter().all(|p| remove(p)) {
                edits.push((*r, String::new()));
            } else {
                for p in selected.iter().filter(|p| p.region == index) {
                    edits.push((p.line, String::new()));
                }
            }
        }
        splice(raw, edits)
    }
    fn set_values(
        &self,
        raw: &str,
        org: bool,
        values: Vec<(String, String)>,
    ) -> Result<String, String> {
        let own = self.own_org_region(raw);
        let mut edits = Vec::new();
        let mut add = Vec::new();
        for (key, value) in values {
            if key.is_empty() || key.contains(['\n', '\r', ':']) || value.contains(['\n', '\r']) {
                return Err("Invalid property edit".into());
            }
            let matching: Vec<_> = self
                .properties
                .iter()
                .filter(|p| p.key.eq_ignore_ascii_case(&key) && (!org || Some(p.region) == own))
                .collect();
            if let Some(first) = matching.first() {
                let trailer = matches!(key.as_str(), "id" | "collapsed" | "logseq.order-list-type");
                let region = self.property_regions[first.region];
                if !org && !trailer && region.0 != 0 && region.0 != self.head_end(raw) {
                    // Move legacy trailing user properties to the existing head,
                    // using accepted property spans rather than a second scanner.
                    edits.extend(matching.iter().map(|p| (p.line, String::new())));
                    add.push((key, value));
                } else {
                    // An accepted empty Markdown property can end immediately
                    // after `::`. Refilling it must supply the grammar's space;
                    // the accepted key/value ranges establish the delimiter slot.
                    let value = if !org
                        && !value.is_empty()
                        && first.value_range.0 == first.key_range.1 + 2
                    {
                        format!(" {value}")
                    } else {
                        value
                    };
                    edits.push((first.value_range, value));
                    edits.extend(matching.iter().skip(1).map(|p| (p.line, String::new())));
                }
            } else {
                add.push((key, value));
            }
        }
        if !add.is_empty() {
            let lines = add
                .iter()
                .map(|(key, value)| {
                    if org {
                        format!(":{key}: {value}")
                    } else {
                        format!("{key}:: {value}")
                    }
                })
                .collect::<Vec<_>>()
                .join(newline(raw));
            return self.add_properties(
                raw,
                org,
                &lines,
                add.iter().any(|(key, _)| {
                    matches!(key.as_str(), "id" | "collapsed" | "logseq.order-list-type")
                }),
                edits,
            );
        }
        Ok(splice(raw, edits))
    }
    // One placement policy for edited values and opaque, already-owned hidden rows.
    fn add_properties(
        &self,
        raw: &str,
        org: bool,
        lines: &str,
        trailer: bool,
        mut edits: Vec<(Range, String)>,
    ) -> Result<String, String> {
        let own = self.own_org_region(raw);
        let (at, text) = if org {
            if let Some(index) = own {
                let body_lines = line_ranges(raw, self.property_regions[index]);
                let close = body_lines
                    .iter()
                    .find(|r| r.slice(raw).trim().eq_ignore_ascii_case(":END:"))
                    .ok_or("Properties drawer has no parser-owned closer")?;
                (close.0, lines.to_string())
            } else {
                let at = self.head_end(raw);
                let text = format!(":PROPERTIES:{}{lines}{}:END:", newline(raw), newline(raw));
                // OG places accepted planning above a new drawer even when
                // authored below body content. Literal lookalikes are absent.
                if self
                    .planning
                    .iter()
                    .any(|p| p.kind != "Closed" && p.line.0 >= at)
                {
                    return Ok(self.hoist_planning(raw, &text));
                }
                (at, text)
            }
        } else {
            let at = if trailer || self.literal_at(0) {
                raw.len()
            } else if let Some(r) = self
                .property_regions
                .iter()
                .find(|r| r.0 == self.head_end(raw) || r.0 == 0)
            {
                r.1
            } else {
                self.head_end(raw)
            };
            (at, lines.to_string())
        };
        let prefix = if at > 0 && !raw[..at].ends_with('\n') {
            newline(raw)
        } else {
            ""
        };
        let suffix = if at < raw.len() { newline(raw) } else { "" };
        if !text.is_empty() {
            edits.push((Range(at, at), format!("{prefix}{text}{suffix}")));
        }
        Ok(splice(raw, edits))
    }
    /// Reattach already-split hidden Org property rows to this exact visible
    /// buffer. Shares property placement with `Properties`; O(block bytes), no
    /// additional ownership parse. Quarantined source refuses without writing.
    pub fn reattach_org_properties(&self, raw: &str, hidden: &str) -> Result<String, String> {
        if self.quarantined {
            return Err("Property reattachment refused: block parsing is quarantined".into());
        }
        let payload = hidden
            .split('\n')
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim_end_matches('\r'))
            .collect::<Vec<_>>()
            .join(newline(raw));
        if payload.is_empty() {
            return Ok(raw.to_string());
        }
        self.add_properties(raw, true, &payload, false, Vec::new())
    }
    /// Apply an operation using these regions from EXACTLY this raw source.
    /// O(block bytes); zero parses in release, one preservation reparse in debug.
    pub fn apply(&self, raw: &str, org: bool, edit: Edit) -> Result<String, String> {
        if self.quarantined {
            return Err("Structural edit refused: block parsing is quarantined".into());
        }
        #[cfg(debug_assertions)]
        let (property_keys, all_properties, planning_kind, drawer_name) = match &edit {
            Edit::Property { key, .. } => (vec![key.to_ascii_lowercase()], false, None, None),
            Edit::Properties { values } => (
                values.iter().map(|(k, _)| k.to_ascii_lowercase()).collect(),
                false,
                None,
                None,
            ),
            Edit::StripCopy { template } => (
                if *template {
                    vec![
                        "id".into(),
                        "template".into(),
                        "template-including-parent".into(),
                    ]
                } else {
                    vec!["id".into()]
                },
                false,
                None,
                None,
            ),
            Edit::Visible => (vec![], true, None, None),
            Edit::Planning { which, .. } => (vec![], false, Some(which.clone()), None),
            Edit::DrawerRow { name, .. } => (vec![], false, None, Some(name.clone())),
            Edit::ClockOut { .. } => (vec![], false, None, Some("LOGBOOK".into())),
            Edit::NormalizePlanning => (vec![], false, None, None),
        };
        let out = match edit {
            Edit::Property { key, value } => {
                if key.is_empty()
                    || key.contains(['\n', '\r', ':'])
                    || value.as_ref().is_some_and(|v| v.contains(['\n', '\r']))
                {
                    return Err("Invalid property edit".into());
                }
                let own = org.then(|| self.own_org_region(raw)).flatten();
                let matching: Vec<&Property> = self
                    .properties
                    .iter()
                    .filter(|p| p.key.eq_ignore_ascii_case(&key) && (!org || Some(p.region) == own))
                    .collect();
                if value.is_none() {
                    self.remove_properties(raw, org, |p| matching.contains(&p))
                } else {
                    self.set_values(raw, org, vec![(key, value.unwrap())])?
                }
            }
            Edit::Properties { values } => self.set_values(raw, org, values)?,

            Edit::Planning { which, value } => {
                if !matches!(which.as_str(), "Scheduled" | "Deadline" | "Closed") {
                    return Err("Invalid planning kind".into());
                }
                let matches: Vec<&Planning> =
                    self.planning.iter().filter(|p| p.kind == which).collect();
                if let Some(value) = value {
                    if value.contains(['\n', '\r']) {
                        return Err("Invalid planning value".into());
                    }
                    let text = format!("{}: {value}", which.to_ascii_uppercase());
                    if let Some(first) = matches.first() {
                        let suffix = &raw[first.timestamp.1..first.line.1];
                        let text = if suffix.trim().is_empty() {
                            text
                        } else {
                            format!("{text}{}", newline(raw))
                        };
                        let mut edits = vec![(first.timestamp, text)];
                        edits.extend(
                            matches
                                .iter()
                                .skip(1)
                                .map(|p| (Self::planning_removal(raw, p), String::new())),
                        );
                        splice(raw, edits)
                    } else {
                        self.inserted(raw, self.title_end(raw), &text)
                    }
                } else {
                    splice(
                        raw,
                        matches
                            .iter()
                            .map(|p| (Self::planning_removal(raw, p), String::new()))
                            .collect(),
                    )
                }
            }
            Edit::StripCopy { template } => self.remove_properties(raw, org, |p| {
                p.key.eq_ignore_ascii_case("id")
                    || (template
                        && matches!(
                            p.key.to_ascii_lowercase().as_str(),
                            "template" | "template-including-parent"
                        ))
            }),
            Edit::Visible => {
                let own = org.then(|| self.own_org_region(raw)).flatten();
                let edits: Vec<_> = self
                    .property_regions
                    .iter()
                    .enumerate()
                    .filter(|(index, r)| !self.literal_at(r.0) && (!org || Some(*index) == own))
                    .map(|(_, r)| (*r, String::new()))
                    .collect();
                if edits.is_empty() {
                    raw.to_string()
                } else {
                    splice(raw, edits).trim_end_matches('\n').to_string()
                }
            }
            Edit::NormalizePlanning => {
                if self.planning.is_empty() || self.literal_at(0) {
                    raw.to_string()
                } else {
                    self.hoist_planning(raw, "")
                        .trim_end_matches('\n')
                        .to_string()
                }
            }
            Edit::DrawerRow { name, value } => {
                if value.contains(['\n', '\r']) {
                    return Err("Invalid drawer row".into());
                }
                if let Some(d) = self
                    .drawers
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(&name))
                {
                    self.inserted(raw, d.close, &value)
                } else {
                    let mut at = self.head_end(raw);
                    if let Some(r) = self.property_regions.iter().find(|r| r.0 == at) {
                        at = r.1;
                    }
                    self.inserted(
                        raw,
                        at,
                        &format!(":{name}:{}{value}{}:END:", newline(raw), newline(raw)),
                    )
                }
            }
            Edit::ClockOut { value } => {
                if value.contains(['\n', '\r']) {
                    return Err("Invalid clock row".into());
                }
                let row = self
                    .drawers
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case("LOGBOOK"))
                    .and_then(|d| d.clocks.last());
                row.map_or_else(
                    || raw.to_string(),
                    |r| splice(raw, vec![(trimmed_range(raw, *r), value)]),
                )
            }
        };
        #[cfg(debug_assertions)]
        if out != raw {
            let after = parse(&out, org);
            debug_assert!(
                !after.quarantined,
                "I-2: edited region shape must remain parseable"
            );
            debug_assert_eq!(
                self.header, after.header,
                "I-4: structural edits preserve the parsed header"
            );
            let properties = |r: &BlockRegions| {
                r.properties
                    .iter()
                    .filter(|p| {
                        !all_properties && !property_keys.contains(&p.key.to_ascii_lowercase())
                    })
                    .map(|p| (p.key.clone(), p.value.clone()))
                    .collect::<Vec<_>>()
            };
            debug_assert_eq!(
                properties(self),
                properties(&after),
                "I-4: unrelated property shape changed"
            );
            let planning = |r: &BlockRegions, source: &str| {
                let mut entries = r
                    .planning
                    .iter()
                    .filter(|p| Some(&p.kind) != planning_kind.as_ref())
                    .map(|p| (p.kind.clone(), p.timestamp.slice(source).to_string()))
                    .collect::<Vec<_>>();
                entries.sort();
                entries
            };
            debug_assert_eq!(
                planning(self, raw),
                planning(&after, &out),
                "I-4: unrelated planning shape changed"
            );
            let drawers = |r: &BlockRegions, source: &str| {
                r.drawers
                    .iter()
                    .filter(|d| {
                        !drawer_name
                            .as_ref()
                            .is_some_and(|n| d.name.eq_ignore_ascii_case(n))
                    })
                    .map(|d| {
                        (
                            d.name.clone(),
                            d.range
                                .slice(source)
                                .trim_end_matches(['\r', '\n'])
                                .to_string(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            debug_assert_eq!(
                drawers(self, raw),
                drawers(&after, &out),
                "I-4: unrelated drawer shape changed"
            );
            // Literal payload bytes remain identical. Edits may move their offsets,
            // so compare source slices rather than stale absolute coordinates.
            let before_literals: Vec<_> = self
                .literals
                .iter()
                .map(|r| r.slice(raw).trim_end_matches(['\r', '\n']))
                .collect();
            let after_literals: Vec<_> = after
                .literals
                .iter()
                .map(|r| r.slice(&out).trim_end_matches(['\r', '\n']))
                .collect();
            debug_assert_eq!(
                before_literals, after_literals,
                "I-4: structural edits preserve parser-owned literals"
            );
        }
        Ok(out)
    }
}
/// Parse and edit one block. Use `BlockRegions::apply` when its AST is cached.
pub fn edit(raw: &str, org: bool, request: Edit) -> Result<String, String> {
    parse(raw, org).apply(raw, org, request)
}
