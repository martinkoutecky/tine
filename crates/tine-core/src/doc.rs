//! Single-file document model: parse a Logseq `.md` file into a tree of blocks
//! and serialize it back in Logseq-compatible form.
//!
//! Round-trip contract: for well-formed Logseq input (TAB per nesting level,
//! continuation lines = `<tabs>` + two spaces), `serialize(parse(x)) == x`.
//! For differently-indented input we canonicalize to TABs (Logseq itself
//! reformats on save, so this is acceptable — see plan "File fidelity").
//!
//! `raw` holds the full block body (first line + continuation/property lines,
//! dedented). Keeping it authoritative is what makes round-tripping safe; the
//! structured views (`properties`, `marker`, `collapsed`) are computed on top.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// Recognized task markers (leading keyword of a block).
pub const MARKERS: &[&str] = &[
    "TODO",
    "DOING",
    "DONE",
    "NOW",
    "LATER",
    "WAITING",
    "WAIT",
    "CANCELED",
    "CANCELLED",
    "STARTED",
    "IN-PROGRESS",
];

/// A parsed `.md` document: an optional page-property pre-block plus a forest
/// of blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Document {
    /// Raw text of the region before the first bullet (page properties / free
    /// text), with the trailing blank separator removed. `None` if the file
    /// starts with a bullet.
    pub pre_block: Option<String>,
    pub roots: Vec<DocBlock>,
}

/// One canonical document parse plus the exact source-byte interval owned by
/// each structural block in depth-first/source order. A parent's own interval
/// ends at its first child's header; a leaf owns through the next structural
/// header or file end.
pub(crate) struct ParsedDocument {
    pub(crate) document: Document,
    pub(crate) block_spans: Vec<Range<usize>>,
    /// Empty structural lines immediately before each block header, in
    /// depth-first/source order. These are document formatting, not block raw.
    pub(crate) blank_lines_before_blocks: Vec<usize>,
    /// Empty separator lines between the preamble and first block.
    pub(crate) blank_lines_after_preamble: usize,
    /// Empty lines before the first block when no semantic preamble exists.
    pub(crate) leading_blank_lines: usize,
    /// Exact source layout when the first root is an lsdoc-authorized ATX
    /// preamble heading that remains unbulleted while owning following outline
    /// blocks as children. Org documents never have this Markdown-only layout.
    pub(crate) promoted_heading_layout: Option<PromotedHeadingLayout>,
    /// Parser-owned outline event count and maximum representable tree depth.
    /// Sync admission consumes these instead of running a second handwritten
    /// structural grammar over the source.
    pub(crate) outline_nodes: usize,
    pub(crate) outline_depth: usize,
}

/// One receipt-proved association between a source structural locator and the
/// stable identity of the semantic block that occupied it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StructuralLayoutIdentity {
    pub(crate) locator: Vec<u32>,
    pub(crate) block_identity: String,
}

#[derive(Clone, Debug)]
struct IdentityBoundBlankLines {
    block_identity: String,
    blank_lines: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocBlock {
    /// Dedented block body: first line + continuation lines joined with `\n`.
    pub raw: String,
    pub children: Vec<DocBlock>,
    /// Runtime/store identity assigned from the document's physical owner and
    /// structural sibling-index path. Persisted `id::` is a separate external
    /// reference identity. This key round-trips through an in-memory save but is
    /// never serialized. It is NOT part of block *content*, so it is excluded
    /// from equality — otherwise the conflict guard (`parse(disk) == cached`)
    /// would always see a "change".
    #[serde(default)]
    pub uuid: String,
    /// Whether this block's page is Org (vs Markdown) — the format lsdoc needs to
    /// parse inline refs correctly (e.g. org `[[target][alias]]`). Page-level
    /// metadata, not content, so excluded from equality (like `uuid`); set at
    /// parse time. `#[serde(default)]` → false on any legacy deserialize.
    #[serde(default)]
    pub is_org: bool,
    /// Lazily-computed, memoized projection of `raw` for the hot read paths
    /// (see [`DocBlock::projection`]). Derived metadata, not content: excluded
    /// from equality + serialization, and reset on clone. `pub(crate)` only so
    /// the constructors in sibling modules can initialize it empty.
    #[serde(skip)]
    pub(crate) proj: std::sync::OnceLock<BlockProjection>,
}

/// Memoized projection of a block's `raw`, so whole-graph scans (full-text
/// search per keystroke, backlink/page-ref matching, `(content …)`) don't
/// re-parse every block's `raw` on each run.
#[derive(Debug, Clone, Default)]
pub struct BlockProjection {
    /// Visible (non-property) text, original case — the body the reader sees,
    /// for breadcrumb labels / display. `raw` minus the byte ranges lsdoc
    /// recognized as `Properties` blocks (see `visible_minus_properties`).
    pub visible: String,
    /// `visible`, lowercased then NFC-normalized — for `search` / `(content …)`
    /// (hot path, pre-folded without compatibility/accent folding).
    pub visible_lower: String,
    /// Normalized page references (`[[..]]` / `#tag`) — for backlinks / `(page-ref)`.
    pub refs_norm: Vec<String>,
    /// The SAME page references in lsdoc's original case — for `referenced_page_names`
    /// (the virtual-page list behind `[[`/`#`/Ctrl-K autocomplete), which needs display
    /// case. Kept on the projection so that hot path reads the memoized parse instead of
    /// re-parsing every block on each cache generation (audit F1).
    pub refs_page: Vec<String>,
    /// Block references (`((uuid))` / `[l](((uuid)))` / `{{embed ((uuid))}}`),
    /// UUID-gated — for the block-referrers / ref-count scans. From the same
    /// lsdoc parse as `refs_norm`.
    pub block_refs: Vec<String>,
    /// Block-header task marker (`TODO`, `DOING`, …) off lsdoc's first node — the
    /// ONE marker recognizer (no more `doc.rs`/`blockView`/lsdoc disagreement).
    pub marker: Option<String>,
    /// Block-header `[#A]` priority off lsdoc's first node — header-position only, so a
    /// mid-text/inline-code `[#A]` is NOT a priority (the old `[#A]`-anywhere scanner
    /// disagreed with the chip — audit C3).
    pub priority: Option<String>,
    /// ATX heading level (1..=6) when the block body is a heading, else `None`.
    pub heading_level: Option<u8>,
    /// `key:: value` block properties (md trailer / org `:PROPERTIES:` drawer) as
    /// lsdoc projects them — the ONE property recognizer for the read path.
    pub properties: Vec<(String, String)>,
    /// SCHEDULED / DEADLINE planning date text (the `<…>` content) when lsdoc emits
    /// a real `Timestamp` for it — code/fence-robust by construction (a `SCHEDULED:`
    /// inside inline code is NOT a Timestamp, so never badged). `None` otherwise.
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
    /// Inline `#tag` / org headline tags, first-seen and de-duplicated. Page refs
    /// stay separate in `refs_page`; this is only the tag field.
    pub tags: Vec<String>,
    /// Parser-owned source spans used by both linked and unlinked reference
    /// surfaces. Kept on the memoized projection so reference queries do not
    /// parse every block again.
    pub(crate) reference_source: crate::reference_evidence::ReferenceSourceProjection,
}

impl BlockProjection {
    /// Whether this block references page `name` (case-insensitive) — checks the
    /// lsdoc-extracted normalized refs (`refs_norm`), the live ref index.
    pub fn refs_contains(&self, name: &str) -> bool {
        self.refs_contains_norm(&crate::refs::normalize(name))
    }

    /// Like [`refs_contains`] but takes an already-[`crate::refs::normalize`]d
    /// target — for hot loops testing ONE target against every block, so the
    /// normalize is hoisted out of the per-block loop instead of repeated.
    pub fn refs_contains_norm(&self, normalized: &str) -> bool {
        self.refs_norm.iter().any(|r| r == normalized)
    }
}

// Identity is metadata, not content: two blocks are equal iff their body and
// subtree match, regardless of uuid. Keeps the external-change conflict guard
// and the round-trip tests comparing on content alone.
impl PartialEq for DocBlock {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.children == other.children
    }
}
impl Eq for DocBlock {}

// Clone resets the projection memo: the clone recomputes it from its own `raw`
// on next access, so it can never inherit a projection that a later in-place
// `raw` edit on either copy would stale.
impl Clone for DocBlock {
    fn clone(&self) -> Self {
        DocBlock {
            raw: self.raw.clone(),
            children: self.children.clone(),
            uuid: self.uuid.clone(),
            is_org: self.is_org,
            proj: std::sync::OnceLock::new(),
        }
    }
}

impl DocBlock {
    pub fn new(raw: impl Into<String>) -> Self {
        DocBlock {
            raw: raw.into(),
            children: Vec::new(),
            uuid: String::new(),
            is_org: false,
            proj: std::sync::OnceLock::new(),
        }
    }

    /// Lazily-computed, memoized projection of `raw` (visible lowercased text +
    /// normalized refs). Safe to memoize because it's a pure function of `raw`
    /// and a cached DocBlock is REPLACED wholesale (a fresh, empty cell) whenever
    /// its content changes — cached blocks are never mutated in place — so the
    /// memo can't outlive the `raw` it was derived from.
    pub fn projection(&self) -> &BlockProjection {
        self.proj.get_or_init(|| {
            // ONE lsdoc parse of the block body yields every header facet (marker,
            // heading level, properties, scheduled/deadline) AND the visible text —
            // so `doc.rs`, the TS `blockView`, and lsdoc can no longer disagree
            // about a block's grammar.
            // ONE lsdoc parse yields BOTH the block AST (facets/visible) AND the refs
            // (`block_refs`/`block_priority` used to parse the same block a 2nd time —
            // audit P1). `proj.refs` is a cheap walk over the already-built blocks.
            let proj = crate::render::parse_projection(&self.raw, self.is_org);
            let (marker, priority, heading_level, properties) = header_facets(&proj.blocks);
            let (scheduled, deadline) = planning_dates(&proj.blocks, &self.raw);
            let tags = tags_from_blocks(&proj.blocks);
            let visible = visible_minus_properties(&self.raw, &proj.blocks);
            let visible_lower = crate::search_query::canonical_fold(&visible);
            let refs_page = proj.refs.page;
            let refs_norm = refs_page
                .iter()
                .map(|r| crate::refs::normalize(r))
                .collect();
            let reference_source =
                crate::reference_evidence::project(&self.raw, self.is_org, &proj.blocks);
            BlockProjection {
                visible,
                visible_lower,
                refs_norm,
                refs_page,
                block_refs: proj.refs.block,
                marker,
                priority,
                heading_level,
                properties,
                scheduled,
                deadline,
                tags,
                reference_source,
            }
        })
    }

    /// `key:: value` block properties as lsdoc projects them (md trailer / org
    /// `:PROPERTIES:` drawer; fence-aware — a `key::` inside a code fence is content).
    pub fn properties(&self) -> Vec<(String, String)> {
        self.projection().properties.clone()
    }

    pub fn property(&self, key: &str) -> Option<String> {
        let key = property_key_norm(key);
        self.projection()
            .properties
            .iter()
            .find(|(k, _)| property_key_norm(k) == key)
            .map(|(_, v)| v.clone())
    }

    pub fn collapsed(&self) -> bool {
        self.property("collapsed").as_deref() == Some("true")
    }

    /// The leading task marker, if any (`TODO`, `DOING`, ...), off lsdoc's first node.
    pub fn marker(&self) -> Option<&str> {
        self.projection().marker.as_deref()
    }

    /// The block-header `[#A]` priority (`"A"`/`"B"`/`"C"`), off lsdoc's first node —
    /// header position only (a mid-text `[#A]` is not a priority).
    pub fn priority(&self) -> Option<&str> {
        self.projection().priority.as_deref()
    }

    /// Heading level (1..=6) if the block body is an ATX heading, else `None`.
    pub fn heading_level(&self) -> Option<u8> {
        self.projection().heading_level
    }

    /// The block's *visible* text (original case): `raw` minus property/drawer
    /// ranges. The body a reader sees — for breadcrumb labels and sort keys.
    pub fn visible_text(&self) -> &str {
        &self.projection().visible
    }

    /// SCHEDULED / DEADLINE planning date text, when lsdoc emits a real `Timestamp`
    /// (code/fence-robust). For the render badge + agenda.
    pub fn scheduled(&self) -> Option<&str> {
        self.projection().scheduled.as_deref()
    }
    pub fn deadline(&self) -> Option<&str> {
        self.projection().deadline.as_deref()
    }

    /// Inline `#tag` / org headline tags off the same lsdoc projection as the
    /// other facets.
    pub fn tags(&self) -> Vec<String> {
        self.projection().tags.clone()
    }
}

fn push_tag(out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, tag: String) {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        return;
    }
    let key = tag.to_lowercase();
    if seen.insert(key) {
        out.push(tag);
    }
}

fn tag_text(inlines: &[lsdoc::ast::Inline], out: &mut String) {
    use lsdoc::ast::{Inline, Url};
    for i in inlines {
        match i {
            Inline::Plain { text, .. }
            | Inline::Code { text, .. }
            | Inline::Verbatim { text, .. } => out.push_str(text),
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. }
            | Inline::Tag { children, .. } => tag_text(children, out),
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
                    tag_text(label, out);
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

fn collect_tags_from_inline(
    inlines: &[lsdoc::ast::Inline],
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    use lsdoc::ast::Inline;
    for i in inlines {
        match i {
            Inline::Tag { children, .. } => {
                let mut text = String::new();
                tag_text(children, &mut text);
                push_tag(out, seen, text);
                collect_tags_from_inline(children, out, seen);
            }
            Inline::Emphasis { children, .. }
            | Inline::Subscript { children, .. }
            | Inline::Superscript { children, .. } => collect_tags_from_inline(children, out, seen),
            Inline::Link { label, .. } => collect_tags_from_inline(label, out, seen),
            _ => {}
        }
    }
}

fn tags_from_blocks(blocks: &[lsdoc::ast::Block]) -> Vec<String> {
    use lsdoc::ast::Block;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for b in blocks {
        match b {
            Block::Bullet { htags, inline, .. } | Block::Heading { htags, inline, .. } => {
                for tag in htags {
                    push_tag(&mut out, &mut seen, tag.clone());
                }
                collect_tags_from_inline(inline, &mut out, &mut seen);
            }
            Block::Paragraph { inline, .. } => {
                collect_tags_from_inline(inline, &mut out, &mut seen)
            }
            _ => {}
        }
    }
    out
}

/// Block-header facets read off lsdoc's parsed blocks — the single source of truth
/// for a block's grammar (replaces the hand-rolled marker / heading / property
/// scanners that could disagree with lsdoc and the TS renderer). Returns
/// `(marker, heading_level, properties)`.
fn header_facets(
    blocks: &[lsdoc::ast::Block],
) -> (
    Option<String>,
    Option<String>,
    Option<u8>,
    Vec<(String, String)>,
) {
    use lsdoc::ast::Block;
    let (marker, priority, heading_level) = match blocks.first() {
        Some(Block::Bullet {
            marker,
            priority,
            size,
            ..
        }) => (
            marker.clone(),
            priority.clone(),
            size.and_then(|s| (1..=6).contains(&s).then_some(s as u8)),
        ),
        Some(Block::Heading {
            marker,
            priority,
            size,
            ..
        }) => (
            marker.clone(),
            priority.clone(),
            // ATX heading level lives in `.size`; `.level` is the nesting depth.
            size.and_then(|s| (1..=6).contains(&s).then_some(s as u8)),
        ),
        _ => (None, None, None),
    };
    let mut properties = Vec::new();
    for b in blocks {
        if let Block::Properties { props, .. } = b {
            properties.extend(props.iter().map(|p| (p.0.clone(), p.1.clone())));
        }
    }
    (marker, priority, heading_level, properties)
}

struct PlanningSourceLine<'a> {
    text: &'a str,
    has_trailing_body: bool,
}

/// Map a parser span from the re-bulleted input (`"- " + raw.trim_start()`) to
/// its original raw slice, but only when the span starts its source line (with
/// optional horizontal whitespace before it). Text after the span is ordinary
/// body content. Deliberate OG divergence: a parser-recognized mid-text
/// `Discuss SCHEDULED: <…>` remains content in Tine rather than header chrome.
fn standalone_source_line<'a>(
    raw: &'a str,
    span: &lsdoc::ast::Span,
) -> Option<PlanningSourceLine<'a>> {
    let lead = raw.len() - raw.trim_start().len();
    let start = span.0.checked_sub(2)?.checked_add(lead)?;
    let end = span.1.checked_sub(2)?.checked_add(lead)?;
    let source = raw.get(start..end)?;
    let line_start = raw[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = raw[end..].find('\n').map_or(raw.len(), |i| end + i);
    if !raw[line_start..start].trim().is_empty() {
        return None;
    }
    Some(PlanningSourceLine {
        text: source,
        has_trailing_body: !raw[end..line_end].trim().is_empty(),
    })
}

/// SCHEDULED / DEADLINE display text (`<…>` content) for parser-recognized
/// Timestamp nodes that start a source line. lsdoc can put same-line or next-line
/// trailing body text in the SAME Paragraph as the planning timestamp (#75), so
/// the older whole-AST-block `is_standalone_planning` check rejected genuine
/// planning lines.
fn planning_dates(blocks: &[lsdoc::ast::Block], raw: &str) -> (Option<String>, Option<String>) {
    use lsdoc::ast::{Block, Inline};
    let mut scheduled = None;
    let mut deadline = None;
    for b in blocks {
        let inlines = match b {
            Block::Bullet { inline, .. }
            | Block::Heading { inline, .. }
            | Block::Paragraph { inline, .. } => inline,
            _ => continue,
        };
        for i in inlines {
            let Inline::Timestamp {
                ts,
                span: Some(span),
                ..
            } = i
            else {
                continue;
            };
            let slot = match ts.as_str() {
                "Scheduled" => &mut scheduled,
                "Deadline" => &mut deadline,
                _ => continue,
            };
            if slot.is_some() {
                continue;
            }
            let Some(line) = standalone_source_line(raw, span) else {
                continue;
            };
            *slot = angle_after(line.text, ts);
        }
    }
    (scheduled, deadline)
}

fn inline_is_break(i: &lsdoc::ast::Inline) -> bool {
    matches!(
        i,
        lsdoc::ast::Inline::Break { .. } | lsdoc::ast::Inline::HardBreak { .. }
    )
}

fn inline_is_empty(i: &lsdoc::ast::Inline) -> bool {
    use lsdoc::ast::Inline;
    inline_is_break(i) || matches!(i, Inline::Plain { text, .. } if text.trim().is_empty())
}

/// Remove parser-confirmed line-leading planning timestamps from the body AST
/// without deleting body content that shares their Paragraph (#75). A neighboring
/// line break is removed only when the timestamp has no same-line body suffix;
/// mid-text timestamps are untouched.
pub(crate) fn strip_planning_lines(
    mut blocks: Vec<lsdoc::ast::Block>,
    raw: &str,
) -> Vec<lsdoc::ast::Block> {
    use lsdoc::ast::{Block, Inline};
    if !raw.contains("SCHEDULED:") && !raw.contains("DEADLINE:") {
        return blocks;
    }
    blocks.retain_mut(|b| {
        let inlines = match b {
            Block::Paragraph { inline, .. }
            | Block::Bullet { inline, .. }
            | Block::Heading { inline, .. } => inline,
            _ => return true,
        };
        let planning: Vec<(usize, bool)> = inlines
            .iter()
            .enumerate()
            .filter_map(|(index, i)| match i {
                Inline::Timestamp {
                    ts,
                    span: Some(span),
                    ..
                } if ts == "Scheduled" || ts == "Deadline" => {
                    standalone_source_line(raw, span).map(|line| (index, line.has_trailing_body))
                }
                _ => None,
            })
            .collect();
        if planning.is_empty() {
            return true;
        }

        let mut remove = vec![false; inlines.len()];
        for (index, has_trailing_body) in planning {
            remove[index] = true;
            if has_trailing_body {
                continue;
            }
            if inlines.get(index + 1).is_some_and(inline_is_break) {
                remove[index + 1] = true;
            } else if index > 0 && inlines.get(index - 1).is_some_and(inline_is_break) {
                remove[index - 1] = true;
            }
        }
        let mut index = 0;
        inlines.retain(|_| {
            let keep = !remove[index];
            index += 1;
            keep
        });
        !inlines.iter().all(inline_is_empty)
    });
    blocks
}

/// The `<…>` content following a `SCHEDULED:` / `DEADLINE:` keyword in `slice`.
fn angle_after(slice: &str, ts: &str) -> Option<String> {
    let kw = if ts == "Scheduled" {
        "SCHEDULED:"
    } else {
        "DEADLINE:"
    };
    let after = &slice[slice.find(kw)? + kw.len()..];
    let lt = after.find('<')?;
    let gt = after[lt + 1..].find('>')?;
    Some(after[lt + 1..lt + 1 + gt].to_string())
}

/// Properties + visible text for a block we only have `raw` for (a query-result
/// DTO has no projection), off the one lsdoc recognizer. md mode: query-result
/// sort keys are cosmetic, and an org `key::` here is format-agnostic exactly as
/// the old line-scan was. Call once per block (decorate-sort), never per compare.
pub(crate) fn block_sort_facets(raw: &str) -> (Vec<(String, String)>, String) {
    let blocks = crate::render::parse_block(raw, false);
    let (_, _, _, properties) = header_facets(&blocks);
    let visible = visible_minus_properties(raw, &blocks);
    (properties, visible)
}

/// `raw` with the byte ranges lsdoc recognized as `Properties` blocks removed,
/// whole-line (so no blank line remains). The lsdoc input is `"{prefix} {raw_trimmed}"`
/// where prefix is the 2-byte `"- "`/`"* "`, so `input[2..] == raw[lead..]` byte-for-byte
/// (`lead` = leading whitespace trimmed) and a span `[s,e)` maps to raw `[s-2+lead, e-2+lead)`.
/// Drawers (`:LOGBOOK:`) are intentionally KEPT (searchable, as before); only
/// `Properties` (md `key::` / org `:PROPERTIES:`) are dropped — exactly the lines
/// the old `visible_lines` dropped, now decided by the one property recognizer.
fn visible_minus_properties(raw: &str, blocks: &[lsdoc::ast::Block]) -> String {
    use lsdoc::ast::Block;
    let lead = raw.len() - raw.trim_start().len();
    let bytes = raw.as_bytes();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        if let Block::Properties { span: Some(sp), .. } = b {
            let mut rs = (sp.0.saturating_sub(2) + lead).min(raw.len());
            let mut re = (sp.1.saturating_sub(2) + lead).min(raw.len());
            if rs >= re {
                continue;
            }
            // Extend to whole lines (newlines are char boundaries → slices stay UTF-8 valid).
            while rs > 0 && bytes[rs - 1] != b'\n' {
                rs -= 1;
            }
            while re < raw.len() && bytes[re - 1] != b'\n' {
                re += 1;
            }
            cuts.push((rs, re));
        }
    }
    if cuts.is_empty() {
        return raw.to_string();
    }
    cuts.sort_by_key(|c| c.0);
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0usize;
    for (s, e) in cuts {
        if s < pos {
            pos = pos.max(e); // overlapping/adjacent property ranges
            continue;
        }
        out.push_str(&raw[pos..s]);
        pos = e;
    }
    out.push_str(&raw[pos..]);
    out.trim_end_matches('\n').to_string()
}

pub(crate) fn property_key_norm(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

pub(crate) fn parse_property_line(line: &str) -> Option<(String, String)> {
    // `key:: value` — key is letters/digits/_/-/. and at least one char.
    let idx = line.find("::")?;
    let key = line[..idx].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return None;
    }
    let value = line[idx + 2..].trim().to_string();
    Some((key.to_string(), value))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromotedHeadingLayout {
    /// The first parser-owned block is an unbulleted ATX heading, but its next
    /// outline event is a sibling rather than a child.
    UnbulletedRoot,
    /// Native Markdown outline: lsdoc reports the owned bullets deeper than the
    /// unbulleted heading, so children retain one indentation level on write.
    NestedChildren,
}

/// Ask lsdoc whether writing `raw` as an unbulleted heading followed by one
/// serializer-level child still forms a bounded nested-heading promotion. The
/// synthetic root boundary prevents the probe child from claiming an
/// unbounded suffix; no handwritten heading or property grammar participates.
fn lsdoc_authorizes_nested_heading_raw(raw: &str, indent: &str) -> bool {
    const CHILD_PROBE: &str = "tine nested-heading child probe";
    const BOUNDARY_PROBE: &str = "tine nested-heading boundary probe";

    let body = format!("{raw}\n{indent}- {CHILD_PROBE}\n- {BOUNDARY_PROBE}");
    matches!(
        crate::outline::parse_document(&body, crate::outline::OutlineFormat::Markdown),
        Ok(ParsedDocument {
            document: Document { roots, .. },
            promoted_heading_layout: Some(PromotedHeadingLayout::NestedChildren),
            ..
        }) if roots.len() == 2
            && roots[0].raw == raw
            && roots[0].children.len() == 1
            && roots[0].children[0].raw == CHILD_PROBE
            && roots[1].raw == BOUNDARY_PROBE
    )
}

pub fn parse(content: &str) -> Document {
    parse_with_source_spans(content).document
}

pub(crate) fn try_parse_with_source_spans(
    content: &str,
) -> Result<ParsedDocument, crate::outline::OutlineAdapterError> {
    crate::outline::parse_document(content, crate::outline::OutlineFormat::Markdown)
}

pub(crate) fn parse_with_source_spans(content: &str) -> ParsedDocument {
    try_parse_with_source_spans(content)
        .unwrap_or_else(|error| panic!("unrepresentable lsdoc Markdown outline: {error}"))
}

/// Formatting knobs detected from a file so re-saving preserves its existing
/// style (avoids gratuitous diffs / Syncthing churn). Logseq, for instance,
/// writes files with NO trailing newline; imposing one would rewrite every file.
#[derive(Debug, Clone)]
pub struct SerializeOpts {
    /// Number of trailing `\n` characters to end the file with.
    pub trailing_newlines: usize,
    /// Empty separator lines between the page preamble and first block.
    pub blank_lines_after_preamble: usize,
    /// Empty lines before the first block when no semantic preamble exists.
    pub leading_blank_lines: usize,
    /// Source-order layout is safe only when the complete semantic document is
    /// unchanged. Edited documents use the identity-bound subset below.
    source_document: Option<Document>,
    source_blank_lines_before_blocks: Vec<usize>,
    identity_bound_blank_lines: Vec<IdentityBoundBlankLines>,
    /// Unbulleted ATX headings whose parser-owned source layout is retained by
    /// a stable block identity after another block changes.
    identity_bound_unbulleted_headings: HashSet<String>,
    source_promoted_heading_layout: Option<PromotedHeadingLayout>,
    promoted_heading_identity: Option<String>,
    /// Whitespace for one level of indentation (e.g. `"\t"` or `"  "`).
    pub indent: String,
}

impl Default for SerializeOpts {
    fn default() -> Self {
        SerializeOpts {
            trailing_newlines: 1,
            blank_lines_after_preamble: 1,
            leading_blank_lines: 0,
            source_document: None,
            source_blank_lines_before_blocks: Vec::new(),
            identity_bound_blank_lines: Vec::new(),
            identity_bound_unbulleted_headings: HashSet::new(),
            source_promoted_heading_layout: None,
            promoted_heading_identity: None,
            indent: "\t".into(),
        }
    }
}

impl SerializeOpts {
    /// Infer the formatting of an existing on-disk file so a save reproduces it.
    /// `None` (new file) falls back to the default.
    pub fn detect(existing: Option<&str>) -> SerializeOpts {
        Self::detect_with_layout_identities(existing, &[])
    }

    pub(crate) fn detect_with_layout_identities(
        existing: Option<&str>,
        identities: &[StructuralLayoutIdentity],
    ) -> SerializeOpts {
        match existing {
            None => SerializeOpts::default(),
            Some(s) => {
                let parsed = parse_with_source_spans(s);
                Self::from_parsed_source(s, parsed, detect_indent(s), identities)
            }
        }
    }

    pub(crate) fn from_parsed_source(
        source: &str,
        parsed: ParsedDocument,
        indent: String,
        identities: &[StructuralLayoutIdentity],
    ) -> SerializeOpts {
        let mut locator_indexes = HashMap::with_capacity(parsed.block_spans.len());
        collect_locator_indexes(
            &parsed.document.roots,
            &mut Vec::new(),
            &mut locator_indexes,
        );
        let mut identity_bound_blank_lines = Vec::with_capacity(identities.len());
        let mut identity_bound_unbulleted_headings = HashSet::new();
        let source_promoted_heading_layout = parsed.promoted_heading_layout;
        let mut promoted_heading_identity = None;
        for identity in identities {
            let Some(index) = locator_indexes.get(identity.locator.as_slice()).copied() else {
                continue;
            };
            let Some(blank_lines) = parsed.blank_lines_before_blocks.get(index).copied() else {
                continue;
            };
            identity_bound_blank_lines.push(IdentityBoundBlankLines {
                block_identity: identity.block_identity.clone(),
                blank_lines,
            });
            if parsed
                .block_spans
                .get(index)
                .and_then(|span| source.get(span.clone()))
                .is_some_and(|block_source| {
                    crate::outline::markdown_unbulleted_heading_line_end(block_source).is_some()
                })
            {
                identity_bound_unbulleted_headings.insert(identity.block_identity.clone());
            }
            if source_promoted_heading_layout.is_some() && identity.locator.as_slice() == [0] {
                promoted_heading_identity = Some(identity.block_identity.clone());
            }
        }
        SerializeOpts {
            // Count trailing `\n` within the trailing run of newline bytes, so
            // a CRLF file's `\r` doesn't truncate the count.
            trailing_newlines: source
                .bytes()
                .rev()
                .take_while(|b| *b == b'\n' || *b == b'\r')
                .filter(|b| *b == b'\n')
                .count(),
            blank_lines_after_preamble: parsed.blank_lines_after_preamble,
            leading_blank_lines: parsed.leading_blank_lines,
            source_document: Some(parsed.document),
            source_blank_lines_before_blocks: parsed.blank_lines_before_blocks,
            identity_bound_blank_lines,
            identity_bound_unbulleted_headings,
            source_promoted_heading_layout,
            promoted_heading_identity,
            indent,
        }
    }

    pub(crate) fn resolved_blank_lines(&self, doc: &Document) -> Vec<usize> {
        if self.source_document.as_ref() == Some(doc) {
            return self.source_blank_lines_before_blocks.clone();
        }
        let by_identity = self
            .identity_bound_blank_lines
            .iter()
            .map(|layout| (layout.block_identity.as_str(), layout.blank_lines))
            .collect::<HashMap<_, _>>();
        let mut resolved = Vec::new();
        collect_identity_bound_blank_lines(&doc.roots, &by_identity, &mut resolved);
        if let Some(first) = resolved.first_mut() {
            // Moving an inter-block separator ahead of the first target block
            // would turn it into page-leading trivia (and, for Org, a semantic
            // preamble). That local context is no longer the source context.
            *first = 0;
        }
        resolved
    }

    fn promoted_heading_layout(&self, doc: &Document) -> Option<PromotedHeadingLayout> {
        let first = doc.roots.first()?;
        let source_unchanged = self.source_document.as_ref() == Some(doc);
        let promoted_identity_remains_first = !first.uuid.is_empty()
            && self.promoted_heading_identity.as_deref() == Some(first.uuid.as_str());
        let promoted_root_authorized = source_unchanged || promoted_identity_remains_first;
        authorized_promoted_heading_layout(
            doc,
            self.source_promoted_heading_layout,
            self.indent.as_str(),
            promoted_root_authorized,
        )
    }
}

fn authorized_promoted_heading_layout(
    doc: &Document,
    source_layout: Option<PromotedHeadingLayout>,
    indent: &str,
    promoted_root_authorized: bool,
) -> Option<PromotedHeadingLayout> {
    let first = doc.roots.first()?;
    match source_layout {
        Some(PromotedHeadingLayout::UnbulletedRoot)
            if promoted_root_authorized
                && lsdoc_authorizes_nested_heading_raw(&first.raw, indent) =>
        {
            Some(PromotedHeadingLayout::UnbulletedRoot)
        }
        Some(PromotedHeadingLayout::NestedChildren)
            if promoted_root_authorized
                && first.children.first().is_some()
                && lsdoc_authorizes_nested_heading_raw(&first.raw, indent) =>
        {
            Some(PromotedHeadingLayout::NestedChildren)
        }
        _ => None,
    }
}

fn collect_locator_indexes(
    blocks: &[DocBlock],
    locator: &mut Vec<u32>,
    indexes: &mut HashMap<Vec<u32>, usize>,
) {
    for (position, block) in blocks.iter().enumerate() {
        let Ok(position) = u32::try_from(position) else {
            return;
        };
        locator.push(position);
        indexes.insert(locator.clone(), indexes.len());
        collect_locator_indexes(&block.children, locator, indexes);
        locator.pop();
    }
}

fn collect_identity_bound_blank_lines(
    blocks: &[DocBlock],
    by_identity: &HashMap<&str, usize>,
    resolved: &mut Vec<usize>,
) {
    for block in blocks {
        resolved.push(by_identity.get(block.uuid.as_str()).copied().unwrap_or(0));
        collect_identity_bound_blank_lines(&block.children, by_identity, resolved);
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Infer the per-level indentation unit from a file's indented bullet lines:
/// a tab if any are tab-indented, else N spaces (the GCD of space widths).
fn detect_indent(s: &str) -> String {
    let mut space_widths: Vec<usize> = Vec::new();
    for line in s.split('\n') {
        let lead_len = line.len() - line.trim_start_matches([' ', '\t']).len();
        if lead_len == 0 {
            continue;
        }
        let rest = &line[lead_len..];
        if !(rest == "-" || rest.starts_with("- ")) {
            continue; // only indented bullet lines reveal the level unit
        }
        if line[..lead_len].contains('\t') {
            return "\t".into();
        }
        space_widths.push(lead_len);
    }
    let w = space_widths.into_iter().fold(0usize, gcd);
    if w >= 2 {
        " ".repeat(w)
    } else {
        "\t".into()
    }
}

/// Serialize a [`Document`] back to Logseq-compatible markdown (default style).
pub fn serialize(doc: &Document) -> String {
    serialize_with(doc, &SerializeOpts::default())
}

/// Serialize, reproducing a file's detected formatting (see [`SerializeOpts`]).
pub fn serialize_with(doc: &Document, opts: &SerializeOpts) -> String {
    let blank_lines_before_blocks = opts.resolved_blank_lines(doc);
    let promoted_heading_layout = opts.promoted_heading_layout(doc);
    serialize_with_layout(
        doc,
        opts.trailing_newlines,
        opts.blank_lines_after_preamble,
        opts.leading_blank_lines,
        &opts.indent,
        &blank_lines_before_blocks,
        promoted_heading_layout,
        &opts.identity_bound_unbulleted_headings,
    )
}

fn serialize_with_layout(
    doc: &Document,
    trailing_newlines: usize,
    blank_lines_after_preamble: usize,
    leading_blank_lines: usize,
    indent: &str,
    blank_lines_before_blocks: &[usize],
    promoted_heading_layout: Option<PromotedHeadingLayout>,
    identity_bound_unbulleted_headings: &HashSet<String>,
) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Some(pre) = &doc.pre_block {
        for line in pre.split('\n') {
            out.push(line.to_string());
        }
        if !doc.roots.is_empty() {
            out.extend(std::iter::repeat_with(String::new).take(blank_lines_after_preamble));
        }
    } else if !doc.roots.is_empty() {
        out.extend(std::iter::repeat_with(String::new).take(leading_blank_lines));
    }
    let mut block_index = 0_usize;
    for block in &doc.roots {
        emit_block(
            block,
            0,
            indent,
            blank_lines_before_blocks,
            &mut block_index,
            promoted_heading_layout,
            identity_bound_unbulleted_headings,
            &mut out,
        );
    }
    let mut s = out.join("\n");
    s.push_str(&"\n".repeat(trailing_newlines));
    s
}

fn emit_block(
    block: &DocBlock,
    level: usize,
    unit: &str,
    blank_lines_before_blocks: &[usize],
    block_index: &mut usize,
    promoted_heading_layout: Option<PromotedHeadingLayout>,
    identity_bound_unbulleted_headings: &HashSet<String>,
    out: &mut Vec<String>,
) {
    let unbulleted_promoted_heading = promoted_heading_layout.is_some() && *block_index == 0;
    let unbulleted_identity_heading = identity_bound_unbulleted_headings.contains(&block.uuid)
        && crate::outline::markdown_unbulleted_heading_line_end(&block.raw).is_some();
    let blank_lines = blank_lines_before_blocks
        .get(*block_index)
        .copied()
        .unwrap_or(0);
    out.extend(std::iter::repeat_with(String::new).take(blank_lines));
    *block_index = block_index.saturating_add(1);
    if unbulleted_promoted_heading || unbulleted_identity_heading {
        out.extend(block.raw.split('\n').map(ToOwned::to_owned));
        let child_level = level.saturating_add(1);
        for child in &block.children {
            emit_block(
                child,
                child_level,
                unit,
                blank_lines_before_blocks,
                block_index,
                promoted_heading_layout,
                identity_bound_unbulleted_headings,
                out,
            );
        }
        return;
    }
    let ind = unit.repeat(level);
    let mut lines = block.raw.split('\n');
    let first = lines.next().unwrap_or("");
    if first.is_empty() {
        out.push(format!("{ind}-"));
    } else {
        out.push(format!("{ind}- {first}"));
    }
    for line in lines {
        out.push(format!("{ind}  {line}"));
    }
    for child in &block.children {
        emit_block(
            child,
            level + 1,
            unit,
            blank_lines_before_blocks,
            block_index,
            promoted_heading_layout,
            identity_bound_unbulleted_headings,
            out,
        );
    }
}

/// Whether detected Markdown formatting plus the parsed document reproduces
/// the exact source bytes. Mixed line endings and non-uniform indentation fail
/// closed because detected formatting deliberately has one representation.
pub fn markdown_round_trips(content: &str) -> bool {
    let Ok(parsed) = try_parse_with_source_spans(content) else {
        return false;
    };
    let document = parsed.document.clone();
    let opts = SerializeOpts::from_parsed_source(content, parsed, detect_indent(content), &[]);
    let mut rendered = serialize_with(&document, &opts);
    if content.contains("\r\n") {
        rendered = rendered.replace('\n', "\r\n");
    }
    rendered == content
}

/// Whether parsing and format-preserving serialization retain the complete
/// document model, even when insignificant source trivia is canonicalized.
///
/// Sparse-v2 admission uses this structural criterion: activation preserves the
/// original source bytes and its backup, so harmless whitespace normalization
/// must not prevent import. A later edit may canonicalize that trivia, but it
/// may not change block content or ancestry.
pub fn markdown_structurally_round_trips(content: &str) -> bool {
    let Ok(parsed) = try_parse_with_source_spans(content) else {
        return false;
    };
    markdown_structurally_round_trips_parsed(content, &parsed)
}

pub(crate) fn markdown_structurally_round_trips_parsed(
    content: &str,
    parsed: &ParsedDocument,
) -> bool {
    let indent = detect_indent(content);
    let mut rendered = serialize_with_layout(
        &parsed.document,
        content
            .bytes()
            .rev()
            .take_while(|byte| matches!(byte, b'\n' | b'\r'))
            .filter(|byte| *byte == b'\n')
            .count(),
        parsed.blank_lines_after_preamble,
        parsed.leading_blank_lines,
        &indent,
        &parsed.blank_lines_before_blocks,
        parsed.promoted_heading_layout,
        &HashSet::new(),
    );
    if content.contains("\r\n") {
        rendered = rendered.replace('\n', "\r\n");
    }
    try_parse_with_source_spans(&rendered)
        .is_ok_and(|canonical| canonical.document == parsed.document)
}

#[cfg(test)]
mod property_fence_tests {
    use super::*;

    #[test]
    fn property_lines_skip_fenced_key_colons() {
        // A `key:: value` line inside a code fence is literal content, not a block
        // property — it must not become a chip / match a (property …) query, and it
        // must stay in the visible (searchable) text.
        let b = DocBlock::new("title:: Real\n```\nlang:: rust\nlet x = 1;\n```\nfoo:: bar");
        let props = b.properties();
        assert!(props.iter().any(|(k, _)| k == "title"));
        assert!(props.iter().any(|(k, _)| k == "foo"));
        assert!(
            !props.iter().any(|(k, _)| k == "lang"),
            "fenced lang:: is not a property: {props:?}"
        );
        assert_eq!(b.property("lang"), None);
        // The fenced property line stays visible (it's code); real props are dropped.
        let vis = b.projection().visible_lower.clone();
        assert!(
            vis.contains("lang:: rust"),
            "fenced line searchable: {vis:?}"
        );
        assert!(
            !vis.contains("title:: real"),
            "real property dropped from visible text"
        );
    }

    #[test]
    fn parse_normalizes_crlf_to_lf() {
        let crlf = parse("title:: x\r\n\r\n- a\r\n- b\r\n");
        // No stray CR leaks into the model (would otherwise pollute property/id values).
        assert_eq!(crlf.pre_block.as_deref(), Some("title:: x"));
        for b in &crlf.roots {
            assert!(!b.raw.contains('\r'), "stray CR in block raw: {:?}", b.raw);
        }
        // CRLF parses to the same model as LF; serialize is LF-canonical.
        let lf = parse("title:: x\n\n- a\n- b\n");
        assert_eq!(serialize(&crlf), serialize(&lf));
        assert!(!serialize(&crlf).contains('\r'));
    }

    #[test]
    fn detect_trailing_newlines_is_crlf_robust() {
        assert_eq!(
            SerializeOpts::detect(Some("- a\r\n\r\n")).trailing_newlines,
            2
        );
        assert_eq!(SerializeOpts::detect(Some("- a\r\n")).trailing_newlines, 1);
        assert_eq!(SerializeOpts::detect(Some("- a\n\n")).trailing_newlines, 2);
    }

    #[test]
    fn structural_blank_line_trivia_round_trips_without_entering_block_raw() {
        let cases = [
            ("final-blank-lines", "- a\n\n"),
            ("between-blocks", "- a\n\n- b\n"),
            ("crlf", "- a\r\n\r\n- b\r\n\r\n"),
            ("no-final-newline", "- a"),
            ("leading-blank-lines", "\n\n- a\n"),
            ("preamble", "title:: Page\n\n\n- a\n"),
            (
                "fence-and-logbook",
                "- fenced\n  ```text\n  \n  - literal\n  ```\n\n- task\n  :LOGBOOK:\n  CLOCK: [2026-07-29 Wed]\n  :END:\n\n- final\n",
            ),
        ];

        for (name, source) in cases {
            let parsed = parse(source);
            assert!(
                markdown_round_trips(source),
                "{name} must retain exact structural trivia"
            );
            assert!(
                parsed.roots.iter().all(|block| !block.raw.ends_with('\n')),
                "{name} leaked document-owned trailing trivia into a root raw: {:?}",
                parsed.roots
            );
        }
    }

    #[test]
    fn nested_blank_continuation_lines_round_trip_byte_exactly() {
        let source = concat!(
            "- ### Synthetic parent\n",
            "\t- First line,\n",
            "\t  wrapped continuation\n",
            "\t  \n",
            "\t  middle paragraph\n",
            "\t  \n",
            "\t  final paragraph\n",
            "- Synthetic sibling\n"
        );

        let parsed = parse(source);
        assert_eq!(
            parsed.roots[0].children[0].raw,
            "First line,\nwrapped continuation\n\nmiddle paragraph\n\nfinal paragraph"
        );
        assert!(
            markdown_round_trips(source),
            "blank continuation lines must retain their Logseq continuation prefix"
        );
        assert_eq!(
            serialize(&parsed),
            source,
            "parse/serialize must reproduce nested blank continuation bytes"
        );
    }

    #[test]
    fn between_block_blank_trivia_does_not_enter_continuation_content() {
        let source = concat!(
            "- First block\n",
            "\n",
            "- Second block\n",
            "  continuation\n",
            "\n",
            "- Third block\n"
        );

        let parsed = parse_with_source_spans(source);
        assert_eq!(parsed.blank_lines_before_blocks, [0, 1, 1]);
        assert_eq!(
            parsed
                .document
                .roots
                .iter()
                .map(|block| block.raw.as_str())
                .collect::<Vec<_>>(),
            ["First block", "Second block\ncontinuation", "Third block"]
        );
        assert!(markdown_round_trips(source));
        assert_eq!(
            serialize_with(&parsed.document, &SerializeOpts::detect(Some(source))),
            source
        );
    }
}

#[cfg(test)]
mod promoted_heading_tests {
    use super::*;

    const NESTED_SOURCE: &str =
        "# Project\n\t- child one\n\t- child two\n- sibling\n\t- nested sibling child";

    fn assign_layout_identities(doc: &mut Document) -> Vec<StructuralLayoutIdentity> {
        fn visit(
            blocks: &mut [DocBlock],
            locator: &mut Vec<u32>,
            identities: &mut Vec<StructuralLayoutIdentity>,
        ) {
            for (position, block) in blocks.iter_mut().enumerate() {
                locator.push(position as u32);
                block.uuid = format!("block-{}", identities.len());
                identities.push(StructuralLayoutIdentity {
                    locator: locator.clone(),
                    block_identity: block.uuid.clone(),
                });
                visit(&mut block.children, locator, identities);
                locator.pop();
            }
        }

        let mut identities = Vec::new();
        visit(&mut doc.roots, &mut Vec::new(), &mut identities);
        identities
    }

    fn semantic_locators(doc: &Document) -> Vec<(Vec<u32>, String)> {
        fn visit(
            blocks: &[DocBlock],
            locator: &mut Vec<u32>,
            locators: &mut Vec<(Vec<u32>, String)>,
        ) {
            for (position, block) in blocks.iter().enumerate() {
                locator.push(position as u32);
                locators.push((locator.clone(), block.raw.clone()));
                visit(&block.children, locator, locators);
                locator.pop();
            }
        }

        let mut locators = Vec::new();
        visit(&doc.roots, &mut Vec::new(), &mut locators);
        locators
    }

    #[test]
    fn lsdoc_nested_outline_promotes_only_the_heading_owned_run() {
        let doc = parse(NESTED_SOURCE);
        assert_eq!(doc.pre_block, None);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "# Project");
        assert_eq!(
            doc.roots[0]
                .children
                .iter()
                .map(|block| block.raw.as_str())
                .collect::<Vec<_>>(),
            vec!["child one", "child two"]
        );
        assert_eq!(doc.roots[1].raw, "sibling");
        assert_eq!(doc.roots[1].children.len(), 1);
        assert_eq!(doc.roots[1].children[0].raw, "nested sibling child");
        assert_eq!(
            serialize_with(&doc, &SerializeOpts::detect(Some(NESTED_SOURCE))),
            NESTED_SOURCE
        );
        assert!(markdown_round_trips(NESTED_SOURCE));
    }

    #[test]
    fn heading_led_markdown_admission_uses_original_and_canonical_parses_only() {
        crate::outline::reset_parse_attempts();

        assert!(markdown_structurally_round_trips(NESTED_SOURCE));
        assert_eq!(
            crate::outline::parse_attempts(),
            2,
            "heading-led admission needs only the retained original parse and canonical reparse"
        );
    }

    #[test]
    fn edited_promoted_heading_keeps_nested_layout_while_identity_stays_first() {
        let mut doc = parse(NESTED_SOURCE);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(NESTED_SOURCE), &identities);

        doc.roots[0].children[0].raw = "child one edited".into();
        doc.roots.push(DocBlock::new("later sibling"));

        assert_eq!(
            serialize_with(&doc, &opts),
            "# Project\n\t- child one edited\n\t- child two\n- sibling\n\t- nested sibling child\n- later sibling"
        );
    }

    #[test]
    fn identity_bound_nonleading_atx_headings_survive_unrelated_edits() {
        let source = "- editable root\n## first section\n### second section\n- trailing root";
        let mut doc = parse(source);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(source), &identities);

        doc.roots[0].raw = "edited root".into();

        let rendered = serialize_with(&doc, &opts);
        assert_eq!(
            rendered,
            "- edited root\n## first section\n### second section\n- trailing root"
        );
        assert_eq!(parse(&rendered), doc);
    }

    #[test]
    fn edited_promoted_heading_without_children_canonicalizes_before_later_sibling() {
        let source = "# Project\n\t- only child\n- sibling";
        let mut doc = parse(source);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(source), &identities);
        doc.roots[0].children.clear();
        let expected_locators = semantic_locators(&doc);

        let rendered = serialize_with(&doc, &opts);
        assert_eq!(rendered, "- # Project\n- sibling");
        let reparsed = parse(&rendered);
        assert_eq!(reparsed, doc);
        assert_eq!(semantic_locators(&reparsed), expected_locators);
    }

    #[test]
    fn edited_promoted_heading_without_children_canonicalizes_as_sole_root() {
        let source = "# Project\n\t- only child";
        let mut doc = parse(source);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(source), &identities);
        doc.roots[0].children.clear();
        let expected_locators = semantic_locators(&doc);

        let rendered = serialize_with(&doc, &opts);
        assert_eq!(rendered, "- # Project");
        let reparsed = parse(&rendered);
        assert_eq!(reparsed, doc);
        assert_eq!(semantic_locators(&reparsed), expected_locators);
    }

    #[test]
    fn edited_promoted_heading_non_heading_raw_canonicalizes_complete_tree() {
        let mut doc = parse(NESTED_SOURCE);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(NESTED_SOURCE), &identities);
        doc.roots[0].raw = "Project renamed".into();
        let expected_locators = semantic_locators(&doc);

        let rendered = serialize_with(&doc, &opts);
        assert_eq!(
            rendered,
            "- Project renamed\n\t- child one\n\t- child two\n- sibling\n\t- nested sibling child"
        );
        let reparsed = parse(&rendered);
        assert_eq!(reparsed, doc);
        assert_eq!(semantic_locators(&reparsed), expected_locators);
    }

    #[test]
    fn legacy_collapsed_heading_uses_parser_owned_same_level_topology() {
        let source = "# Parent\ncollapsed:: true\n- child\n- sibling";
        let doc = parse(source);
        assert_eq!(doc.pre_block, None);
        assert_eq!(doc.roots.len(), 3);
        assert_eq!(doc.roots[0].raw, "# Parent\ncollapsed:: true");
        assert_eq!(doc.roots[1].raw, "child");
        assert_eq!(doc.roots[2].raw, "sibling");
        assert!(doc.roots.iter().all(|root| root.children.is_empty()));
        assert!(markdown_round_trips(source));
        assert!(markdown_structurally_round_trips(source));
    }

    #[test]
    fn edited_legacy_flat_heading_with_later_root_canonicalizes_without_reparenting() {
        let source = "# Parent\ncollapsed:: true\n- child\n- sibling";
        let mut doc = parse(source);
        let identities = assign_layout_identities(&mut doc);
        let opts = SerializeOpts::detect_with_layout_identities(Some(source), &identities);
        doc.roots.push(DocBlock::new("later root"));

        let rendered = serialize_with(&doc, &opts);
        assert_eq!(
            rendered,
            "# Parent\ncollapsed:: true\n- child\n- sibling\n- later root"
        );
        let reparsed = parse(&rendered);
        assert_eq!(reparsed, doc);
        assert_eq!(
            semantic_locators(&reparsed),
            vec![
                (vec![0], "# Parent\ncollapsed:: true".into()),
                (vec![1], "child".into()),
                (vec![2], "sibling".into()),
                (vec![3], "later root".into()),
            ]
        );
    }

    #[test]
    fn parser_owned_heading_boundary_after_child_run_is_representable() {
        for source in [
            "# Parent\n\t- child\n# Same-level boundary",
            "## Parent\n\t\t- child\n# Shallower boundary",
        ] {
            let parsed = parse_with_source_spans(source);
            assert_eq!(parsed.document.pre_block, None, "{source:?}");
            assert_eq!(parsed.document.roots.len(), 2, "{source:?}");
            assert_eq!(parsed.document.roots[0].children.len(), 1, "{source:?}");
            assert!(markdown_structurally_round_trips(source), "{source:?}");
        }
    }

    #[test]
    fn promoted_heading_multi_level_indent_jump_preserves_semantics() {
        let source = "# Project\n\t\t\t- deep child\n- sibling";
        let parsed = parse_with_source_spans(source);
        assert_eq!(
            parsed.promoted_heading_layout,
            Some(PromotedHeadingLayout::NestedChildren)
        );
        let expected = parsed.document.clone();
        let opts = SerializeOpts::from_parsed_source(source, parsed, detect_indent(source), &[]);
        let rendered = serialize_with(&expected, &opts);
        assert_eq!(rendered, "# Project\n\t- deep child\n- sibling");
        assert_eq!(parse(&rendered), expected);
    }

    #[test]
    fn promoted_heading_mixed_indent_canonicalizes_without_reparenting() {
        let source = "    # Parent\n      - child\n- root\n  - other child";
        let parsed = parse_with_source_spans(source);
        assert_eq!(
            parsed.promoted_heading_layout,
            Some(PromotedHeadingLayout::NestedChildren)
        );
        let expected = parsed.document.clone();
        let expected_locators = semantic_locators(&expected);
        let opts = SerializeOpts::from_parsed_source(source, parsed, detect_indent(source), &[]);

        let rendered = serialize_with(&expected, &opts);
        assert_eq!(
            rendered,
            "-     # Parent\n  - child\n- root\n  - other child"
        );
        let reparsed = parse(&rendered);
        assert_eq!(reparsed, expected);
        assert_eq!(semantic_locators(&reparsed), expected_locators);
    }

    #[test]
    fn promoted_heading_with_lone_cr_fails_closed() {
        let source = "# Project\r\t- child\r- sibling";
        assert!(!markdown_round_trips(source));
    }

    #[test]
    fn ordinary_heading_and_same_level_bullet_are_parser_owned_siblings() {
        let source = "# Notes\n- ordinary root";
        let doc = parse(source);
        assert_eq!(doc.pre_block, None);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "# Notes");
        assert_eq!(doc.roots[1].raw, "ordinary root");
        assert!(markdown_structurally_round_trips(source));
    }

    #[test]
    fn promoted_heading_keeps_blank_lines_on_both_sides() {
        let source = "title:: Page\n\n# Project\n\n\t- child\n- sibling";
        let doc = parse(source);
        assert_eq!(doc.pre_block.as_deref(), Some("title:: Page"));
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "# Project");
        assert_eq!(doc.roots[0].children[0].raw, "child");
        assert_eq!(
            serialize_with(&doc, &SerializeOpts::detect(Some(source))),
            source
        );
        assert!(markdown_round_trips(source));
    }
}

#[cfg(test)]
mod org_container_outline_tests {
    use super::*;

    fn parse_round_trip(input: &str) -> Document {
        let doc = parse(input);
        assert_eq!(
            serialize_with(&doc, &SerializeOpts::detect(Some(input))),
            input,
            "org-container fixture must round-trip byte-exactly"
        );
        doc
    }

    #[test]
    fn quote_list_body_stays_in_one_block() {
        let input = "- #+BEGIN_QUOTE\n  - Today\n  - Tomorrow\n  #+END_QUOTE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_QUOTE\n- Today\n- Tomorrow\n#+END_QUOTE"
        );
    }

    #[test]
    fn example_nested_space_indents_stay_in_one_block() {
        let input = "- #+BEGIN_EXAMPLE\n      - a\n         - b\n            - c\n            - d\n  #+END_EXAMPLE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_EXAMPLE\n    - a\n       - b\n          - c\n          - d\n#+END_EXAMPLE"
        );
    }

    #[test]
    fn end_name_prefix_closes_container() {
        let input = "- #+BEGIN_QUOTE\n  - x\n  #+END_QUOTE_EXTRA trailing";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_QUOTE\n- x\n#+END_QUOTE_EXTRA trailing"
        );
    }

    #[test]
    fn begin_and_end_recognition_matches_mldoc_spaces_case_and_name_run() {
        let input = "-   #+begin_note options\n    - x\n  #+eNd_NoTeSuffix trailing";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "  #+begin_note options\n  - x\n#+eNd_NoTeSuffix trailing"
        );
    }

    #[test]
    fn malformed_closers_and_empty_begin_name_open_no_region() {
        let malformed = "- #+BEGIN_QUOTE\n  #+END\n  - x\n  #+END_\n  - y\n  text #+END_QUOTE\n  - z\n  #+END_QUOTE";
        let doc = parse_round_trip(malformed);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_QUOTE\n#+END\n- x\n#+END_\n- y\ntext #+END_QUOTE\n- z\n#+END_QUOTE"
        );

        let empty_name = "- #+BEGIN_ \n  x\n  #+END_\n  - child";
        let doc = parse_round_trip(empty_name);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].raw, "#+BEGIN_ \nx\n#+END_");
        assert_eq!(doc.roots[0].children[0].raw, "child");
    }

    #[test]
    fn unterminated_begin_keeps_existing_outline_shape() {
        let input = "- #+BEGIN_QUOTE\n  - a\n- sibling";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE");
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "a");
        assert_eq!(doc.roots[1].raw, "sibling");
    }

    #[test]
    fn continuation_begin_cannot_swallow_same_lane_sibling() {
        let input = "- parent\n  #+BEGIN_QUOTE\n- sibling\n  #+END_QUOTE";
        let doc = parse(input);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(
            doc.roots[0].raw,
            "parent\n#+BEGIN_QUOTE\n- sibling\n#+END_QUOTE"
        );
        assert!(doc.roots[0].children.is_empty());
        assert!(markdown_structurally_round_trips(input));
    }

    #[test]
    fn nested_child_closer_lane_does_not_open_region() {
        let tabbed = "- #+BEGIN_QUOTE\n\t- child\n\t  #+END_QUOTE";
        let doc = parse(tabbed);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE\n- child\n #+END_QUOTE");
        assert!(markdown_structurally_round_trips(tabbed));

        let spaced = "- #+BEGIN_QUOTE\n  - child\n    #+END_QUOTE";
        let doc = parse(spaced);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE\n- child\n  #+END_QUOTE");
        assert!(markdown_structurally_round_trips(spaced));
    }

    #[test]
    fn tab_child_before_matching_end_opens_no_region() {
        let input = "- \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "\t#+BEGIN_QUOTE\n- x\n #+END_QUOTE");
        assert!(markdown_structurally_round_trips(input));
    }

    #[test]
    fn continuation_opener_before_tab_child_opens_no_region() {
        let input = "- p\n  \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "p\n\t#+BEGIN_QUOTE\n- x\n #+END_QUOTE");
        assert!(markdown_structurally_round_trips(input));
    }

    #[test]
    fn sub_prefixed_end_opens_no_region() {
        let input = "-    #+BEGIN_QUOTE\n  - x\n    \x1a#+END_QUOTE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].raw, "   #+BEGIN_QUOTE");
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "x\n\x1a#+END_QUOTE");
    }

    #[test]
    fn first_compatible_end_closes_without_depth_counting() {
        let input = "- #+BEGIN_QUOTE\n  #+BEGIN_QUOTE\n  #+END_QUOTE\n- outside\n  #+END_QUOTE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_QUOTE\n#+BEGIN_QUOTE\n#+END_QUOTE"
        );
        assert_eq!(doc.roots[1].raw, "outside\n#+END_QUOTE");
    }

    #[test]
    fn org_close_is_honored_while_inner_fence_is_open() {
        let input = "- #+BEGIN_QUOTE\n  ```\n  #+END_QUOTE\n  ```\n- sibling";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE\n```\n#+END_QUOTE\n```");
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[1].raw, "sibling");
    }

    #[test]
    fn begin_inside_code_fence_opens_no_org_region() {
        let input = "- ```\n  #+BEGIN_QUERY\n  - literal\n  #+END_QUERY\n  ```\n- sibling";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(
            doc.roots[0].raw,
            "```\n#+BEGIN_QUERY\n- literal\n#+END_QUERY\n```"
        );
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[1].raw, "sibling");
    }

    #[test]
    fn src_body_and_nested_query_stay_literal() {
        let input =
            "- #+BEGIN_SRC\n  - x\n  #+BEGIN_QUERY\n  - y\n  #+END_QUERY\n  #+END_SRC\n- sibling";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(
            doc.roots[0].raw,
            "#+BEGIN_SRC\n- x\n#+BEGIN_QUERY\n- y\n#+END_QUERY\n#+END_SRC"
        );
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[1].raw, "sibling");
    }

    #[test]
    fn lone_cr_org_container_decision_is_a_known_lsdoc_parity_gap() {
        let old_mac = "- #+BEGIN_QUOTE\r  - x\r  #+END_QUOTE";
        let doc = parse(old_mac);
        assert_eq!(
            serialize_with(&doc, &SerializeOpts::detect(Some(old_mac))),
            "- #+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE"
        );
        assert_ne!(
            serialize_with(&doc, &SerializeOpts::detect(Some(old_mac))),
            old_mac
        );
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "x\n#+END_QUOTE");
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn projection_matches_direct_computation() {
        let b = DocBlock::new("TODO ship [[Foo Bar]] and #tag\nid:: abc\nprop:: secret");
        let p = b.projection();
        // visible_lower == canonical_fold(visible_text(raw)): property lines dropped
        assert_eq!(p.visible_lower, "todo ship [[foo bar]] and #tag");
        assert!(
            !p.visible_lower.contains("secret"),
            "property values excluded"
        );
        // refs_contains ≡ references_page (case-insensitive, normalized)
        assert!(p.refs_contains("foo bar"));
        assert!(p.refs_contains("TAG"));
        assert!(!p.refs_contains("nope"));
        // memoized (stable across calls); a clone recomputes to an equal projection
        assert_eq!(b.projection().visible_lower, p.visible_lower);
        assert_eq!(b.clone().projection().refs_norm, p.refs_norm);
    }

    #[test]
    fn facets_read_off_one_lsdoc_parse() {
        // marker / heading / properties all come off lsdoc's single parse now.
        let b = DocBlock::new("TODO finish it\nfoo:: bar\nid:: 123");
        assert_eq!(b.marker(), Some("TODO"));
        assert_eq!(b.property("foo").as_deref(), Some("bar"));
        assert_eq!(b.property("id").as_deref(), Some("123"));
        assert_eq!(b.heading_level(), None);
        // STARTED is an mldoc/lsdoc marker (in the recognized set).
        assert_eq!(DocBlock::new("STARTED x").marker(), Some("STARTED"));
        // ATX-heading bullet → level off lsdoc `Bullet.size`.
        assert_eq!(DocBlock::new("## A heading").heading_level(), Some(2));
        assert_eq!(DocBlock::new("plain text").heading_level(), None);
    }

    #[test]
    fn priority_is_header_position_only() {
        // audit C3: lsdoc only treats a header-position `[#A]` as priority; the old
        // `[#A]`-anywhere scanner disagreed with the chip on load.
        assert_eq!(DocBlock::new("TODO [#A] task").priority(), Some("A"));
        assert_eq!(DocBlock::new("Discuss [#A] tags").priority(), None); // mid-text
        assert_eq!(DocBlock::new("TODO task [#A] later").priority(), None); // not after marker
    }

    #[test]
    fn visible_text_drops_properties_utf8_safe() {
        // Multi-byte body before a trailing property block: the span→raw byte
        // mapping (`span - 2 + lead`) must land on char boundaries, not split UTF-8.
        let b = DocBlock::new("Über café résumé\nid:: 123\nkey:: v");
        assert_eq!(b.visible_text(), "Über café résumé");
        assert_eq!(b.projection().visible_lower, "über café résumé");
        // leading whitespace in raw (lead > 0) still maps correctly.
        let b2 = DocBlock::new("  héllo\nid:: 9");
        assert_eq!(b2.visible_text().trim(), "héllo");
    }

    #[test]
    fn planning_dates_off_lsdoc_timestamp_code_robust() {
        // Real planning lines → faithful `<…>` date text off lsdoc's Timestamp.
        let b =
            DocBlock::new("TODO ship it\nSCHEDULED: <2026-06-28 Sun>\nDEADLINE: <2026-07-01 Wed>");
        assert_eq!(b.scheduled(), Some("2026-06-28 Sun"));
        assert_eq!(b.deadline(), Some("2026-07-01 Wed"));
        // The robustness fix: a `DEADLINE:` inside inline code is `Code`, not a
        // Timestamp — so it is NEVER badged (the old regex wrongly badged it).
        let code = DocBlock::new("look at `DEADLINE: <2026-06-28 Sun>` here");
        assert_eq!(
            code.deadline(),
            None,
            "code-embedded planning is not badged"
        );
        assert_eq!(DocBlock::new("plain block").scheduled(), None);
    }

    #[test]
    fn schedule_stays_a_facet_when_body_text_follows() {
        let b = DocBlock::new("Task\nSCHEDULED: <2026-07-13 Mon>\nnotes after the schedule");
        assert_eq!(b.scheduled(), Some("2026-07-13 Mon"));
        let utf8 = DocBlock::new("Überblick\nSCHEDULED: <2026-07-14 Tue>\n続き");
        assert_eq!(utf8.scheduled(), Some("2026-07-14 Tue"));
        let mid =
            DocBlock::new("Discuss SCHEDULED: <2026-07-13 Mon> inline\nnotes after the timestamp");
        assert_eq!(mid.scheduled(), None);
    }

    #[test]
    fn line_leading_planning_timestamp_keeps_trailing_body_text() {
        for (tag, date) in [
            ("DEADLINE", "2026-07-30 Thu"),
            ("SCHEDULED", "2026-07-29 Wed"),
        ] {
            let raw = format!("TODO x\n{tag}: <{date}>tail");
            let b = DocBlock::new(raw.clone());
            if tag == "DEADLINE" {
                assert_eq!(b.deadline(), Some(date));
            } else {
                assert_eq!(b.scheduled(), Some(date));
            }
            let body = format!(
                "{:?}",
                strip_planning_lines(crate::render::parse_block(&raw, false), &raw)
            );
            assert!(body.contains("x"), "title remains body content: {body}");
            assert!(body.contains("tail"), "suffix remains body content: {body}");
            assert!(
                !body.contains(date),
                "timestamp is removed from body: {body}"
            );
            assert_eq!(b.raw, raw, "facet projection never rewrites raw");
        }

        let indented = DocBlock::new("TODO x\n  DEADLINE: <2026-07-30 Thu>tail");
        assert_eq!(indented.deadline(), Some("2026-07-30 Thu"));

        // Deliberate OG divergence: only a line-leading timestamp is planning
        // chrome; a mid-text timestamp remains ordinary body content in Tine.
        let mid = DocBlock::new("Discuss DEADLINE: <2026-07-30 Thu> inline");
        assert_eq!(mid.deadline(), None);

        let inline_code = DocBlock::new("`DEADLINE: <2026-07-30 Thu>`");
        assert_eq!(inline_code.deadline(), None);
        let fenced = DocBlock::new("```\nDEADLINE: <2026-07-30 Thu>\n```");
        assert_eq!(fenced.deadline(), None);
    }

    #[test]
    fn org_properties_from_drawer_not_key_colons() {
        // lsdoc correction: in ORG, `key:: val` is plain text (NOT a property);
        // org properties live in a `:PROPERTIES:` drawer. Tine's old line-scan
        // wrongly read org `key::` as a property — routing through lsdoc fixes it.
        let mut drawer = DocBlock::new("task\n:PROPERTIES:\n:id: 6679-abc\n:END:");
        drawer.is_org = true;
        assert_eq!(drawer.property("id").as_deref(), Some("6679-abc"));
        let mut plain = DocBlock::new("note\nfoo:: bar");
        plain.is_org = true;
        assert_eq!(plain.property("foo"), None, "org key:: is not a property");
        assert!(
            plain.visible_text().contains("foo:: bar"),
            "org key:: stays visible"
        );
    }
}
