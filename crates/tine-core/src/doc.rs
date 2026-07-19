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

#[derive(Debug, Serialize, Deserialize)]
pub struct DocBlock {
    /// Dedented block body: first line + continuation lines joined with `\n`.
    pub raw: String,
    pub children: Vec<DocBlock>,
    /// Stable identity assigned once when the block enters the in-memory cache
    /// (the persisted `id::` if any, else a generated uuid). It is the handle
    /// every surface (main view, sidebar, query result, ref) uses to address the
    /// block, and round-trips through save so it stays stable across edits. It is
    /// NOT part of block *content*, so it is excluded from equality — otherwise
    /// the conflict guard (`parse(disk) == cached`) would always see a "change".
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
        self.projection()
            .properties
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
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

/// Map a parser span from the re-bulleted input (`"- " + raw.trim_start()`) to
/// its original raw slice, but only when the span occupies a whole source line.
/// Horizontal whitespace around the token is allowed. A parser-recognized
/// `Discuss SCHEDULED: <…> inline` therefore fails this boundary check without
/// introducing a second planning-line parser.
fn standalone_source_line<'a>(raw: &'a str, span: &lsdoc::ast::Span) -> Option<&'a str> {
    let lead = raw.len() - raw.trim_start().len();
    let start = span.0.checked_sub(2)?.checked_add(lead)?;
    let end = span.1.checked_sub(2)?.checked_add(lead)?;
    let source = raw.get(start..end)?;
    let line_start = raw[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = raw[end..].find('\n').map_or(raw.len(), |i| end + i);
    if !raw[line_start..start].trim().is_empty() || !raw[end..line_end].trim().is_empty() {
        return None;
    }
    Some(source)
}

/// SCHEDULED / DEADLINE display text (`<…>` content) for parser-recognized
/// Timestamp nodes that occupy a whole source line. lsdoc can put a trailing body
/// line in the SAME Paragraph as the planning timestamp (#75), so the older
/// whole-AST-block `is_standalone_planning` check rejected a genuine planning line.
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
            *slot = angle_after(line, ts);
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

/// Remove parser-confirmed whole-line planning timestamps from the body AST without
/// deleting body content that shares their Paragraph (#75). The neighboring line
/// break is removed with the planning token; mid-text timestamps are untouched.
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
        let planning: Vec<usize> = inlines
            .iter()
            .enumerate()
            .filter_map(|(index, i)| match i {
                Inline::Timestamp {
                    ts,
                    span: Some(span),
                    ..
                } if (ts == "Scheduled" || ts == "Deadline")
                    && standalone_source_line(raw, span).is_some() =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect();
        if planning.is_empty() {
            return true;
        }

        let mut remove = vec![false; inlines.len()];
        for index in planning {
            remove[index] = true;
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

/// Number of leading whitespace characters (tabs and spaces). Used as an
/// indent "column" for nesting. Tabs and spaces each count as one; within a
/// single file indentation is consistent (all tabs or all N-spaces), so column
/// comparison recovers nesting regardless of which a file uses. Output is
/// always canonicalized to TABs.
fn leading_ws(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b'\t' || *b == b' ')
        .count()
}

/// Classify a line as a bullet at a given indent column, returning its content
/// (text after the `- ` marker). Returns `None` for non-bullet lines.
fn bullet(line: &str) -> Option<(usize, &str)> {
    let col = leading_ws(line);
    let rest = &line[col..];
    if rest == "-" {
        Some((col, ""))
    } else if let Some(content) = rest.strip_prefix("- ") {
        Some((col, content))
    } else {
        None
    }
}

/// Parse a file's contents into a [`Document`].
/// The fence marker at the start of a line: `(char, run-length)` for a run of >=3
/// backticks or tildes (leading whitespace ignored); else `None`.
pub(crate) fn fence_marker(text: &str) -> Option<(char, usize)> {
    let t = text.trim_start();
    let c = t.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = t.chars().take_while(|&x| x == c).count();
    (n >= 3).then_some((c, n))
}

/// Given the current open fence (if any) and a line, return the new fence state:
/// open on the first valid marker, close only on a matching one (same char, >=
/// the opener's length). Shared by the block parser, `property_lines`, and
/// `visible_lines` so "inside a code fence?" is decided one way.
pub(crate) fn next_fence(cur: Option<(char, usize)>, line: &str) -> Option<(char, usize)> {
    match cur {
        None => fence_marker(line),
        Some((c, n)) => match fence_marker(line) {
            Some((c2, n2)) if c2 == c && n2 >= n => None, // closing fence
            _ => Some((c, n)),                            // still inside
        },
    }
}

/// mldoc `Parsers.is_space`: space, tab, SUB, or form feed.
fn mldoc_is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | 0x1a | 0x0c)
}

fn mldoc_spaces_len(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .take_while(|&&b| mldoc_is_space(b))
        .count()
}

fn mldoc_trim_spaces_start(s: &str) -> &str {
    &s[mldoc_spaces_len(s)..]
}

/// lsdoc's line-local `ocaml_start`: space, tab, or form feed, but not SUB.
fn ocaml_spaces_len(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .take_while(|&&b| matches!(b, b' ' | b'\t' | 0x0c))
        .count()
}

// Transcribed from lsdoc v2 `block_begin_name`: after mldoc-space trimming,
// BEGIN is ASCII-case-insensitive and the non-empty name ends at mldoc space.
fn block_begin_name(s: &str) -> Option<String> {
    let t = mldoc_trim_spaces_start(s);
    if !t.get(..8)?.eq_ignore_ascii_case("#+BEGIN_") {
        return None;
    }
    let rest = &t[8..];
    let mut end = 0usize;
    let bytes = rest.as_bytes();
    while end < bytes.len() && !mldoc_is_space(bytes[end]) {
        end += 1;
    }
    (end > 0).then(|| rest[..end].to_string())
}

fn starts_ci(s: &str, prefix: &str) -> bool {
    let p = prefix.as_bytes();
    let b = s.as_bytes();
    b.len() >= p.len() && b[..p.len()].eq_ignore_ascii_case(p)
}

// Transcribed from lsdoc v2 `block_end_matches_name` / EndTrie: the first
// later END suffix with the opener name as an ASCII-case-insensitive prefix
// closes the region; no boundary after the name is required.
fn block_end_matches_name(text: &str, name: &str) -> bool {
    let t = &text[ocaml_spaces_len(text)..];
    let Some(suffix) = t.get(6..) else {
        return false;
    };
    starts_ci(t, "#+END_")
        && suffix.len() >= name.len()
        && suffix.as_bytes()[..name.len()].eq_ignore_ascii_case(name.as_bytes())
}

/// Prove that an org opener's first compatible END is in the same physical
/// continuation lane before a bullet that would fold the opener frame.
fn has_bounded_org_closer(following: &[&str], content_start: usize, name: &str) -> bool {
    let mut fence = None;
    for line in following {
        // A bullet shallower than the opener block's content lane is a genuine
        // child/sibling and bounds the lookahead. A bullet inside an inner fence
        // remains literal.
        if fence.is_none() && bullet(line).is_some_and(|(bullet_col, _)| bullet_col < content_start)
        {
            return false;
        }

        // mldoc takes the first name-compatible END. It is safe to rescue this
        // outline region only when that same END is in the block-content lane.
        if block_end_matches_name(line, name) {
            return ocaml_spaces_len(line) == content_start;
        }

        fence = next_fence(fence, line);
    }
    false
}

fn markdown_property_line(line: &str) -> bool {
    let Some((key, _)) = line.trim().split_once("::") else {
        return false;
    };
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
}

/// Recover a Logseq block shape emitted by some importers/plugins:
///
/// ```markdown
/// # Parent heading
/// collapsed:: true
/// - child
/// - child
/// ```
///
/// The ATX heading is not list-bulleted, so the generic outline parser would put
/// it in the page preamble and expose its children as roots. Logseq nevertheless
/// treats this particular shape as one collapsed parent. Promote it narrowly:
/// only an ATX heading at the END of the preamble, followed solely by block
/// property lines and carrying `collapsed:: true`. Ordinary page prose/headings
/// and genuine page properties remain untouched.
fn promote_preamble_collapsed_heading(pre_block: &mut Option<String>, roots: &mut Vec<DocBlock>) {
    if roots.is_empty() {
        return;
    }
    let Some(pre) = pre_block.as_deref() else {
        return;
    };
    let lines: Vec<&str> = pre.split('\n').collect();
    let Some(start) = lines.iter().rposition(|line| {
        let trimmed = line.trim_start();
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ')
    }) else {
        return;
    };
    if !lines[start + 1..]
        .iter()
        .all(|line| !line.trim().is_empty() && markdown_property_line(line))
    {
        return;
    }

    let raw = lines[start..].join("\n");
    let mut parent = DocBlock::new(raw);
    if parent.heading_level().is_none() || !parent.collapsed() {
        return;
    }
    parent.children = std::mem::take(roots);
    roots.push(parent);

    let mut pre_end = start;
    while pre_end > 0 && lines[pre_end - 1].trim().is_empty() {
        pre_end -= 1;
    }
    *pre_block = (pre_end > 0).then(|| lines[..pre_end].join("\n"));
}

pub fn parse(content: &str) -> Document {
    // Normalize CRLF / lone CR to LF so the in-memory model never carries a stray
    // `\r` (which would otherwise pollute property / `id::` values and break
    // matching). The file's original line endings are reproduced at the write
    // boundary (model.rs `write_page`), not here — the model is LF-canonical.
    let normalized;
    let content = if content.contains('\r') {
        normalized = content.replace('\r', "");
        normalized.as_str()
    } else {
        content
    };
    let body = content.strip_suffix('\n').unwrap_or(content);
    let lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };

    // Find the first bullet to split pre-block from block region.
    let first_bullet = lines.iter().position(|l| bullet(l).is_some());

    let (pre_lines, block_lines) = match first_bullet {
        Some(i) => (&lines[..i], &lines[i..]),
        None => (&lines[..], &[][..]),
    };

    // Pre-block: drop trailing blank lines (the separator is re-added on write).
    let mut pre_end = pre_lines.len();
    while pre_end > 0 && pre_lines[pre_end - 1].trim().is_empty() {
        pre_end -= 1;
    }
    let mut pre_block = if pre_end == 0 {
        None
    } else {
        Some(pre_lines[..pre_end].join("\n"))
    };

    // Build the block forest with a stack of frames keyed by indent column.
    struct Frame {
        col: usize,
        /// Column where the block's text starts (`col` + 2 for the `- `).
        content_start: usize,
        raw: String,
        children: Vec<DocBlock>,
        /// The open fence marker `(char, length)` if this block's content is
        /// currently inside a fenced code block — `Some` means every following
        /// line is literal continuation (even one that looks like a `- ` bullet),
        /// so fenced code isn't shredded into child blocks. A fence closes only on
        /// a marker of the SAME char and at least the opener's length, so a ````
        /// fence containing ``` (or `~~~`) round-trips correctly.
        fence: Option<(char, usize)>,
        /// The lowercased name of a terminated `#+BEGIN_<name>` region. This is
        /// independent of `fence`: an org closer is honored inside an inner code
        /// fence, and fence state keeps updating while the org region is open.
        org_block: Option<String>,
    }
    // fence_marker / next_fence are module-level (shared with property_lines /
    // visible_lines so "is this line inside a code fence" has ONE implementation).
    let mut stack: Vec<Frame> = Vec::new();
    let mut roots: Vec<DocBlock> = Vec::new();

    // Collapse frames at indent column >= `keep_above` into their parents.
    fn fold_to(stack: &mut Vec<Frame>, roots: &mut Vec<DocBlock>, keep_above: usize) {
        while let Some(top) = stack.last() {
            if top.col >= keep_above {
                let f = stack.pop().unwrap();
                let block = DocBlock {
                    raw: f.raw,
                    children: f.children,
                    uuid: String::new(),
                    is_org: false,
                    proj: std::sync::OnceLock::new(),
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(block),
                    None => roots.push(block),
                }
            } else {
                break;
            }
        }
    }

    for (line_idx, line) in block_lines.iter().enumerate() {
        let in_literal = stack
            .last()
            .map(|f| f.fence.is_some() || f.org_block.is_some())
            .unwrap_or(false);
        // A `- ` line starts a new block only when we're outside both literal
        // region kinds of the current top frame.
        if !in_literal {
            if let Some((col, content)) = bullet(line) {
                // New block: fold every block at this column or deeper, so the
                // remaining stack top (shallower column) becomes the parent.
                fold_to(&mut stack, &mut roots, col);
                let org_block = block_begin_name(content).and_then(|name| {
                    has_bounded_org_closer(&block_lines[line_idx + 1..], col + 2, &name)
                        .then(|| name.to_ascii_lowercase())
                });
                stack.push(Frame {
                    col,
                    content_start: col + 2,
                    raw: content.to_string(),
                    children: Vec::new(),
                    fence: next_fence(None, content), // bullet line may open a fence
                    org_block,
                });
                continue;
            }
        }
        if let Some(top) = stack.last_mut() {
            // Continuation line: strip the block's content-start indentation.
            let stripped = strip_n_ws(line, top.content_start);
            top.raw.push('\n');
            top.raw.push_str(stripped);

            if let Some(name) = top.org_block.as_deref() {
                // END indexing is independent of fence context in mldoc.
                if block_end_matches_name(stripped, name) {
                    top.org_block = None;
                }
            } else if top.fence.is_none() {
                // An already-open code fence suppresses BEGIN recognition.
                top.org_block = block_begin_name(stripped).and_then(|name| {
                    has_bounded_org_closer(&block_lines[line_idx + 1..], top.content_start, &name)
                        .then(|| name.to_ascii_lowercase())
                });
            }
            top.fence = next_fence(top.fence, stripped);
        }
        // (A continuation before any bullet can't happen: it'd be pre-block.)
    }
    fold_to(&mut stack, &mut roots, 0);

    promote_preamble_collapsed_heading(&mut pre_block, &mut roots);

    Document { pre_block, roots }
}

/// Remove up to `n` leading whitespace characters (tabs or spaces).
fn strip_n_ws(line: &str, n: usize) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < n && i < bytes.len() && (bytes[i] == b'\t' || bytes[i] == b' ') {
        i += 1;
    }
    &line[i..]
}

/// Formatting knobs detected from a file so re-saving preserves its existing
/// style (avoids gratuitous diffs / Syncthing churn). Logseq, for instance,
/// writes files with NO trailing newline; imposing one would rewrite every file.
#[derive(Debug, Clone)]
pub struct SerializeOpts {
    /// Number of trailing `\n` characters to end the file with.
    pub trailing_newlines: usize,
    /// Emit a blank line between the page-property pre-block and the first block.
    pub blank_after_props: bool,
    /// Whitespace for one level of indentation (e.g. `"\t"` or `"  "`).
    pub indent: String,
}

impl Default for SerializeOpts {
    fn default() -> Self {
        SerializeOpts {
            trailing_newlines: 1,
            blank_after_props: true,
            indent: "\t".into(),
        }
    }
}

impl SerializeOpts {
    /// Infer the formatting of an existing on-disk file so a save reproduces it.
    /// `None` (new file) falls back to the default.
    pub fn detect(existing: Option<&str>) -> SerializeOpts {
        match existing {
            None => SerializeOpts::default(),
            Some(s) => SerializeOpts {
                // Count trailing `\n` within the trailing run of newline bytes, so
                // a CRLF file's `\r` doesn't truncate the count (`…\r\n\r\n` ⇒ 2).
                trailing_newlines: s
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\n' || *b == b'\r')
                    .filter(|b| *b == b'\n')
                    .count(),
                blank_after_props: blank_after_props(s),
                indent: detect_indent(s),
            },
        }
    }
}

/// Does the file put a blank line between its pre-block and the first bullet?
fn blank_after_props(s: &str) -> bool {
    let lines: Vec<&str> = s.split('\n').collect();
    match lines.iter().position(|l| bullet(l).is_some()) {
        Some(i) if i > 0 => lines[i - 1].trim().is_empty(),
        _ => true,
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
    let mut out: Vec<String> = Vec::new();
    if let Some(pre) = &doc.pre_block {
        for line in pre.split('\n') {
            out.push(line.to_string());
        }
        // Blank separator before blocks — only when blocks follow and the file
        // used one.
        if !doc.roots.is_empty() && opts.blank_after_props {
            out.push(String::new());
        }
    }
    for block in &doc.roots {
        emit_block(block, 0, &opts.indent, &mut out);
    }
    let mut s = out.join("\n");
    s.push_str(&"\n".repeat(opts.trailing_newlines));
    s
}

fn emit_block(block: &DocBlock, level: usize, unit: &str, out: &mut Vec<String>) {
    let ind = unit.repeat(level);
    let mut lines = block.raw.split('\n');
    let first = lines.next().unwrap_or("");
    if first.is_empty() {
        out.push(format!("{ind}-"));
    } else {
        out.push(format!("{ind}- {first}"));
    }
    for line in lines {
        if line.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{ind}  {line}"));
        }
    }
    for child in &block.children {
        emit_block(child, level + 1, unit, out);
    }
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
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 2);
        assert_eq!(doc.roots[0].raw, "parent\n#+BEGIN_QUOTE");
        assert_eq!(doc.roots[1].raw, "sibling\n#+END_QUOTE");
        assert!(doc.roots.iter().all(|block| block.children.is_empty()));
    }

    #[test]
    fn nested_child_closer_lane_does_not_open_region() {
        let tabbed = "- #+BEGIN_QUOTE\n\t- child\n\t  #+END_QUOTE";
        let doc = parse_round_trip(tabbed);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "child\n#+END_QUOTE");

        let spaced = "- #+BEGIN_QUOTE\n  - child\n    #+END_QUOTE";
        let doc = parse_round_trip(spaced);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "child\n#+END_QUOTE");
    }

    #[test]
    fn tab_child_before_matching_end_opens_no_region() {
        let input = "- \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].raw, "\t#+BEGIN_QUOTE");
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "x\n#+END_QUOTE");
    }

    #[test]
    fn continuation_opener_before_tab_child_opens_no_region() {
        let input = "- p\n  \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(doc.roots[0].raw, "p\n\t#+BEGIN_QUOTE");
        assert_eq!(doc.roots[0].children.len(), 1);
        assert_eq!(doc.roots[0].children[0].raw, "x\n#+END_QUOTE");
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
            "- #+BEGIN_QUOTE  - x  #+END_QUOTE"
        );
        assert_ne!(
            serialize_with(&doc, &SerializeOpts::detect(Some(old_mac))),
            old_mac
        );
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
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
