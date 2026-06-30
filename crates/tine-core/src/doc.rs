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
    "TODO", "DOING", "DONE", "NOW", "LATER", "WAITING", "WAIT", "CANCELED", "CANCELLED",
    "STARTED", "IN-PROGRESS",
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
    /// `visible`, lowercased — for `search` / `(content …)` (hot path, pre-lowered).
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
}

impl BlockProjection {
    /// Whether this block references page `name` (case-insensitive) — checks the
    /// lsdoc-extracted normalized refs (`refs_norm`), the live ref index.
    pub fn refs_contains(&self, name: &str) -> bool {
        let n = crate::refs::normalize(name);
        self.refs_norm.iter().any(|r| *r == n)
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
        DocBlock { raw: raw.into(), children: Vec::new(), uuid: String::new(), is_org: false, proj: std::sync::OnceLock::new() }
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
            let visible = visible_minus_properties(&self.raw, &proj.blocks);
            let visible_lower = visible.to_lowercase();
            let refs_page = proj.refs.page;
            let refs_norm = refs_page.iter().map(|r| crate::refs::normalize(r)).collect();
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
}

/// Block-header facets read off lsdoc's parsed blocks — the single source of truth
/// for a block's grammar (replaces the hand-rolled marker / heading / property
/// scanners that could disagree with lsdoc and the TS renderer). Returns
/// `(marker, heading_level, properties)`.
fn header_facets(
    blocks: &[lsdoc::ast::Block],
) -> (Option<String>, Option<String>, Option<u8>, Vec<(String, String)>) {
    use lsdoc::ast::Block;
    let (marker, priority, heading_level) = match blocks.first() {
        Some(Block::Bullet { marker, priority, size, .. }) => (
            marker.clone(),
            priority.clone(),
            size.and_then(|s| (1..=6).contains(&s).then_some(s as u8)),
        ),
        Some(Block::Heading { marker, priority, level, .. }) => (
            marker.clone(),
            priority.clone(),
            (1..=6u32).contains(level).then_some(*level as u8),
        ),
        _ => (None, None, None),
    };
    let mut properties = Vec::new();
    for b in blocks {
        if let Block::Properties { props, .. } = b {
            properties.extend(props.iter().cloned());
        }
    }
    (marker, priority, heading_level, properties)
}

/// SCHEDULED / DEADLINE display text (`<…>` content) for whichever top-level block
/// lsdoc tagged with a real `Scheduled`/`Deadline` `Timestamp` — so a `SCHEDULED:`
/// inside inline code (which lsdoc parses as `Code`, not a Timestamp) is never
/// badged. The display is sliced faithfully from that block's span in `raw` (exact
/// original text); the lsdoc input is `"- " + raw.trim_start()`, so a span `[s,e)`
/// maps to raw `[s-2+lead, e-2+lead)`.
fn planning_dates(blocks: &[lsdoc::ast::Block], raw: &str) -> (Option<String>, Option<String>) {
    use lsdoc::ast::{Block, Inline};
    let lead = raw.len() - raw.trim_start().len();
    let mut scheduled = None;
    let mut deadline = None;
    for b in blocks {
        let (inlines, span) = match b {
            Block::Bullet { inline, span, .. }
            | Block::Heading { inline, span, .. }
            | Block::Paragraph { inline, span, .. } => (inline, span),
            _ => continue,
        };
        // Only a STANDALONE planning line is a date badge. lsdoc v0.2.0 also makes a
        // mid-text `SCHEDULED:` a Timestamp; that's body text, not a badge (and the
        // frontend keeps it in the body), so this must agree (audit C1).
        if !is_standalone_planning(inlines) {
            continue;
        }
        for i in inlines {
            let Inline::Timestamp { ts, .. } = i else { continue };
            let slot = match ts.as_str() {
                "Scheduled" => &mut scheduled,
                "Deadline" => &mut deadline,
                _ => continue,
            };
            if slot.is_some() {
                continue;
            }
            if let Some(sp) = span {
                let rs = (sp.0.saturating_sub(2) + lead).min(raw.len());
                let re = (sp.1.saturating_sub(2) + lead).min(raw.len());
                *slot = angle_after(&raw[rs..re.max(rs)], ts);
            }
        }
    }
    (scheduled, deadline)
}

/// A block whose inline content is ONLY a SCHEDULED/DEADLINE planning timestamp (plus
/// blank text / line breaks) — the standalone planning line that becomes a date badge.
/// A mid-text timestamp leaves real content, so it is NOT standalone.
fn is_standalone_planning(inlines: &[lsdoc::ast::Inline]) -> bool {
    use lsdoc::ast::Inline;
    let mut planning = false;
    for i in inlines {
        match i {
            Inline::Timestamp { ts, .. } if ts == "Scheduled" || ts == "Deadline" => planning = true,
            Inline::Break | Inline::HardBreak => {}
            Inline::Plain { text } if text.trim().is_empty() => {}
            _ => return false,
        }
    }
    planning
}

/// The `<…>` content following a `SCHEDULED:` / `DEADLINE:` keyword in `slice`.
fn angle_after(slice: &str, ts: &str) -> Option<String> {
    let kw = if ts == "Scheduled" { "SCHEDULED:" } else { "DEADLINE:" };
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
    line.bytes().take_while(|b| *b == b'\t' || *b == b' ').count()
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
    let lines: Vec<&str> = if body.is_empty() { Vec::new() } else { body.split('\n').collect() };

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
    let pre_block = if pre_end == 0 { None } else { Some(pre_lines[..pre_end].join("\n")) };

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
                let block = DocBlock { raw: f.raw, children: f.children, uuid: String::new(), is_org: false, proj: std::sync::OnceLock::new() };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(block),
                    None => roots.push(block),
                }
            } else {
                break;
            }
        }
    }

    for line in block_lines {
        let in_code = stack.last().map(|f| f.fence.is_some()).unwrap_or(false);
        // A `- ` line starts a new block only when we're NOT inside a fenced code
        // block of the current top frame; inside a fence it's literal content.
        if !in_code {
            if let Some((col, content)) = bullet(line) {
                // New block: fold every block at this column or deeper, so the
                // remaining stack top (shallower column) becomes the parent.
                fold_to(&mut stack, &mut roots, col);
                stack.push(Frame {
                    col,
                    content_start: col + 2,
                    raw: content.to_string(),
                    children: Vec::new(),
                    fence: next_fence(None, content), // bullet line may open a fence
                });
                continue;
            }
        }
        if let Some(top) = stack.last_mut() {
            // Continuation line: strip the block's content-start indentation.
            let stripped = strip_n_ws(line, top.content_start);
            top.raw.push('\n');
            top.raw.push_str(stripped);
            top.fence = next_fence(top.fence, stripped);
        }
        // (A continuation before any bullet can't happen: it'd be pre-block.)
    }
    fold_to(&mut stack, &mut roots, 0);

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
        SerializeOpts { trailing_newlines: 1, blank_after_props: true, indent: "\t".into() }
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
    if b == 0 { a } else { gcd(b, a % b) }
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
    if w >= 2 { " ".repeat(w) } else { "\t".into() }
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
        assert!(!props.iter().any(|(k, _)| k == "lang"), "fenced lang:: is not a property: {props:?}");
        assert_eq!(b.property("lang"), None);
        // The fenced property line stays visible (it's code); real props are dropped.
        let vis = b.projection().visible_lower.clone();
        assert!(vis.contains("lang:: rust"), "fenced line searchable: {vis:?}");
        assert!(!vis.contains("title:: real"), "real property dropped from visible text");
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
        assert_eq!(SerializeOpts::detect(Some("- a\r\n\r\n")).trailing_newlines, 2);
        assert_eq!(SerializeOpts::detect(Some("- a\r\n")).trailing_newlines, 1);
        assert_eq!(SerializeOpts::detect(Some("- a\n\n")).trailing_newlines, 2);
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn projection_matches_direct_computation() {
        let b = DocBlock::new("TODO ship [[Foo Bar]] and #tag\nid:: abc\nprop:: secret");
        let p = b.projection();
        // visible_lower == visible_text(raw).to_lowercase(): property lines dropped
        assert_eq!(p.visible_lower, "todo ship [[foo bar]] and #tag");
        assert!(!p.visible_lower.contains("secret"), "property values excluded");
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
        let b = DocBlock::new("TODO ship it\nSCHEDULED: <2026-06-28 Sun>\nDEADLINE: <2026-07-01 Wed>");
        assert_eq!(b.scheduled(), Some("2026-06-28 Sun"));
        assert_eq!(b.deadline(), Some("2026-07-01 Wed"));
        // The robustness fix: a `DEADLINE:` inside inline code is `Code`, not a
        // Timestamp — so it is NEVER badged (the old regex wrongly badged it).
        let code = DocBlock::new("look at `DEADLINE: <2026-06-28 Sun>` here");
        assert_eq!(code.deadline(), None, "code-embedded planning is not badged");
        assert_eq!(DocBlock::new("plain block").scheduled(), None);
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
        assert!(plain.visible_text().contains("foo:: bar"), "org key:: stays visible");
    }
}
