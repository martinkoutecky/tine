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

use crate::outline::{self, OutlineFormat};

mod search;

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

/// Finished task markers, left out of the query hint's **Open tasks** rewrite
/// (`query::wire_parse::og_query_hint`, GH #422) and of carry-over.
/// `src/markers.test.ts` checks parity with the frontend's finished-marker set.
pub const DONE_MARKERS: &[&str] = &["DONE", "CANCELED", "CANCELLED"];

/// A parsed Markdown or Org document: an optional page-property pre-block plus a forest
/// of blocks.
#[deny(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Document {
    /// Raw text of the region before the first bullet (page properties / free
    /// text), with the trailing blank separator removed. `None` if the file
    /// starts with a bullet.
    pub pre_block: Option<String>,
    /// Top-level blocks in document order.
    pub roots: Vec<DocBlock>,
}

/// One parsed block with raw text and nested children.
#[deny(missing_docs)]
#[derive(Debug, Serialize, Deserialize)]
pub struct DocBlock {
    /// Dedented block body: first line + continuation lines joined with `\n`.
    pub(crate) raw: String,
    /// Child blocks in document order.
    pub children: Vec<DocBlock>,
    /// Runtime/store identity assigned from the document's physical owner and
    /// structural sibling-index path. Persisted `id::` is a separate external
    /// reference identity. This key round-trips through an in-memory save but is
    /// skipped by serde serialization; deserialization defaults it to an empty
    /// string until a parse assigns runtime identities. It is not part of block content, so it is excluded
    /// from equality. Store write conflicts use raw-byte `FileRev` guards.
    #[serde(default, skip_serializing)]
    pub uuid: String,
    /// Whether this block's page is Org (vs Markdown) — the format lsdoc needs to
    /// parse inline refs correctly (e.g. org `[[target][alias]]`). Page-level
    /// metadata, not content, so excluded from equality (like `uuid`); set at
    /// parse time. `#[serde(default)]` → false on any legacy deserialize.
    #[serde(default)]
    pub(crate) is_org: bool,
    /// Graph property parsing policy for synthetic page-header blocks only.
    #[serde(skip)]
    pub(crate) property_ref_config: Option<Box<crate::query::atom::ParseConfig>>,
    /// Derived projection of the current block body.
    // set_raw clears this memo; cloning starts with an empty memo.
    #[serde(skip)]
    pub(crate) proj: std::sync::OnceLock<BlockProjection>,
}

/// Parsed values derived from a block's current raw text.
// Memoization avoids reparsing each block during whole-graph scans.
//
// Memory shape (GH #623): this slot is paid inline by every block, so it holds only what
// every block has. Facets set on a minority of blocks live in one `Option<Box<..>>`, and
// text that equals another stored text is not stored again: `visible` is `None` when it
// equals the block's `raw`, `visible_lower` is `None` when it equals `visible`. Those
// fallbacks need the block's `raw`, so the ONLY readers are `DocBlock::visible_text` and
// `DocBlock::visible_folded`; the fields are private so no caller can re-implement the
// fallback. Unrelated to behaviour: every accessor returns what the eager form returned.
#[deny(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockProjection {
    /// Parser-owned raw byte regions from the same cached single-block AST.
    /// Sparse edit data stays out of the inline block; empty regions are shared.
    #[serde(deserialize_with = "deserialize_shared_regions")]
    pub regions: std::sync::Arc<crate::block_regions::BlockRegions>,
    /// Visible (non-property) text, original case — `raw` minus the byte ranges lsdoc
    /// recognized as `Properties` blocks. `None` when identical to `raw`.
    visible: Option<Box<str>>,
    /// `visible` folded with accent-removing `canonical_fold`, independent of graph
    /// policy. `None` when identical to the visible text.
    visible_lower: Option<Box<str>>,
    /// Lazy accent-sensitive fold, populated only when that graph policy is used;
    /// the inner `None` means identical to the visible text. Checkpointed in
    /// whatever state it is in (built or not), like every lazy answer.
    #[serde(with = "once_cell_as_option")]
    visible_literal: std::sync::OnceLock<Option<Box<str>>>,
    /// Byte ranges of `raw` eligible for plain-text (unlinked) reference matching.
    plain_ranges: Box<[std::ops::Range<usize>]>,
    /// Facets set on a minority of blocks; `None` when the block has none of them.
    extra: Option<Box<ProjectionExtra>>,
}

/// The facets of a [`BlockProjection`] that most blocks do not have (g13k: 80% of blocks
/// have none of them), kept behind one pointer so an ordinary block does not pay for them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProjectionExtra {
    // Friendly search is runtime-only: no checkpoint shape or meaning changes.
    #[serde(skip)]
    search: std::sync::OnceLock<Option<search::SearchProjection>>,
    refs_norm: Box<[String]>,
    refs_page: Box<[String]>,
    block_refs: Box<[String]>,
    marker: Option<String>,
    priority: Option<String>,
    heading_level: Option<u8>,
    properties: Box<[(String, String)]>,
    scheduled: Option<String>,
    deadline: Option<String>,
    tags: Box<[String]>,
    explicit: Box<[crate::reference_evidence::ProjectedPageRef]>,
    withheld_ranges: Box<[std::ops::Range<usize>]>,
}

impl ProjectionExtra {
    fn is_empty(&self) -> bool {
        self.refs_norm.is_empty()
            && self.refs_page.is_empty()
            && self.block_refs.is_empty()
            && self.marker.is_none()
            && self.priority.is_none()
            && self.heading_level.is_none()
            && self.properties.is_empty()
            && self.scheduled.is_none()
            && self.deadline.is_none()
            && self.tags.is_empty()
            && self.explicit.is_empty()
            && self.withheld_ranges.is_empty()
    }
}

impl BlockProjection {
    /// Visible text; `raw` must be the raw body this projection was built from.
    fn visible_text<'a>(&'a self, raw: &'a str) -> &'a str {
        self.visible.as_deref().unwrap_or(raw)
    }

    /// Visible text folded for the graph's search policy. The accent-sensitive
    /// form is computed once per block body; both forms reset with the projection.
    fn visible_folded<'a>(&'a self, raw: &'a str, remove_accents: bool) -> &'a str {
        let visible = self.visible_text(raw);
        if remove_accents {
            self.visible_lower.as_deref().unwrap_or(visible)
        } else {
            self.visible_literal
                .get_or_init(|| {
                    let folded = crate::search_query::literal_fold(visible);
                    (folded != visible).then(|| folded.into_boxed_str())
                })
                .as_deref()
                .unwrap_or(visible)
        }
    }

    fn extra(&self) -> Option<&ProjectionExtra> {
        self.extra.as_deref()
    }

    /// Whether this block references page `name` under page-name normalization.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn refs_contains(&self, name: &str) -> bool {
        self.refs_contains_norm(&crate::refs::normalize(name))
    }

    /// Like [`Self::refs_contains`] but takes an already-[`crate::refs::normalize`]d
    /// target — for hot loops testing ONE target against every block, so the
    /// normalize is hoisted out of the per-block loop instead of repeated.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn refs_contains_norm(&self, normalized: &str) -> bool {
        self.refs_norm().iter().any(|r| r == normalized)
    }

    /// Normalized page references (`[[..]]` / `#tag` / Org headline tags).
    pub fn refs_norm(&self) -> &[String] {
        self.extra().map_or(&[], |e| &e.refs_norm)
    }

    /// The SAME page references in lsdoc's original case — for `referenced_page_names`
    /// (the virtual-page list behind `[[`/`#`/Ctrl-K autocomplete), which needs display
    /// case. Kept on the projection to reuse the parse across cache generations.
    pub fn refs_page(&self) -> &[String] {
        self.extra().map_or(&[], |e| &e.refs_page)
    }

    /// Block references (`((uuid))` / `[l](((uuid)))` / `{{embed ((uuid))}}`),
    /// UUID-gated — for the block-referrers / ref-count scans. From the same
    /// lsdoc parse as `refs_norm`.
    pub fn block_refs(&self) -> &[String] {
        self.extra().map_or(&[], |e| &e.block_refs)
    }

    /// Block-header task marker (`TODO`, `DOING`, …) off lsdoc's first node — the
    /// ONE marker recognizer (no more `doc.rs`/`blockView`/lsdoc disagreement).
    pub fn marker(&self) -> Option<&str> {
        self.extra().and_then(|e| e.marker.as_deref())
    }

    /// Block-header `[#A]` priority off lsdoc's first node — header-position only, so a
    /// mid-text/inline-code `[#A]` is NOT a priority (the old `[#A]`-anywhere scanner
    /// disagreed with the chip — audit C3).
    pub fn priority(&self) -> Option<&str> {
        self.extra().and_then(|e| e.priority.as_deref())
    }

    /// ATX heading level (1..=6) when the block body is a heading, else `None`.
    pub fn heading_level(&self) -> Option<u8> {
        self.extra().and_then(|e| e.heading_level)
    }

    /// `key:: value` block properties (md trailer / org `:PROPERTIES:` drawer) as
    /// lsdoc projects them — the ONE property recognizer for the read path.
    pub fn properties(&self) -> &[(String, String)] {
        self.extra().map_or(&[], |e| &e.properties)
    }

    /// SCHEDULED planning date text (the `<…>` content) when lsdoc emits
    /// a real `Timestamp` for it — code/fence-robust by construction (a `SCHEDULED:`
    /// inside inline code is NOT a Timestamp, so never badged). `None` otherwise.
    pub fn scheduled(&self) -> Option<&str> {
        self.extra().and_then(|e| e.scheduled.as_deref())
    }

    /// DEADLINE planning date text, when present in parsed syntax.
    pub fn deadline(&self) -> Option<&str> {
        self.extra().and_then(|e| e.deadline.as_deref())
    }

    /// Inline `#tag` / org headline tags, first-seen and de-duplicated. Page refs
    /// stay separate in `refs_page`; this is only the tag field.
    pub fn tags(&self) -> &[String] {
        self.extra().map_or(&[], |e| &e.tags)
    }

    /// Parser-owned source byte spans used by linked and unlinked reference
    /// surfaces. Browser-facing reference spans instead use UTF-16 offsets.
    // Kept on the memoized projection so reference queries avoid reparsing.
    pub fn reference_source(&self) -> crate::reference_evidence::ReferenceSource<'_> {
        let extra = self.extra();
        crate::reference_evidence::ReferenceSource {
            explicit: extra.map_or(&[], |e| &e.explicit),
            plain_ranges: &self.plain_ranges,
            withheld_ranges: extra.map_or(&[], |e| &e.withheld_ranges),
        }
    }
}

// Identity is metadata, not content: two blocks are equal iff their body and
// subtree match, regardless of uuid. Store write guards compare raw FileRev;
// content equality is useful for in-memory comparison and round-trip tests.
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
            property_ref_config: self.property_ref_config.clone(),
            proj: std::sync::OnceLock::new(),
        }
    }
}

/// Test-only: a block whose raw text contains this panics in
/// [`DocBlock::projection`] (feature `test-faults`, never in a shipped build).
#[cfg(feature = "test-faults")]
pub const TEST_PROJECTION_PANIC_SENTINEL: &str = "__TINE_TEST_PROJECTION_PANIC__";

impl DocBlock {
    pub fn new(raw: impl Into<String>) -> Self {
        DocBlock {
            raw: raw.into(),
            children: Vec::new(),
            uuid: String::new(),
            is_org: false,
            property_ref_config: None,
            proj: std::sync::OnceLock::new(),
        }
    }

    /// Raw block body, including continuation lines and properties.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Replace the block body and invalidate its derived projection.
    pub fn set_raw(&mut self, raw: impl Into<String>) {
        self.raw = raw.into();
        self.proj = std::sync::OnceLock::new();
    }

    /// Release only the memoized projection for an isolated heap probe.
    #[cfg(feature = "test-faults")]
    pub fn clear_projection_for_tests(&mut self) {
        self.proj.take();
    }

    /// Whether this block is parsed with Org inline syntax.
    pub fn is_org(&self) -> bool {
        self.is_org
    }

    /// Change inline syntax and invalidate its derived projection.
    pub fn set_org(&mut self, is_org: bool) {
        if self.is_org != is_org {
            self.is_org = is_org;
            self.proj = std::sync::OnceLock::new();
        }
    }

    /// Set the graph policy used to project a synthetic page-property block.
    pub fn set_property_ref_config(&mut self, config: &crate::query::atom::ParseConfig) {
        self.property_ref_config = Some(Box::new(config.clone()));
        self.proj = std::sync::OnceLock::new();
    }

    /// Visible text, references, and facets derived from the current raw body.
    /// [`Self::set_raw`] clears this cache when the body changes.
    pub fn projection(&self) -> &BlockProjection {
        self.proj.get_or_init(|| {
            // Test-only fault: a block projection (the lsdoc parse every index
            // reads) that panics, for the load's one-bad-file isolation tests.
            #[cfg(feature = "test-faults")]
            if self.raw.contains(TEST_PROJECTION_PANIC_SENTINEL) {
                panic!("deterministic test sentinel for a block projection panic");
            }
            // ONE lsdoc parse of the block body yields every header facet (marker,
            // heading level, properties, scheduled/deadline) AND the visible text —
            // so `doc.rs`, the TS `blockView`, and lsdoc can no longer disagree
            // about a block's grammar.
            // ONE lsdoc parse yields AST, facets and refs; region ownership also
            // recovers an author swallowed by a final quote (GH #895).
            let proj = crate::render::parse_projection(&self.raw, self.is_org);
            let (marker, priority, heading_level, mut properties) = header_facets(&proj.blocks);
            let (scheduled, deadline) = planning_dates(&proj.blocks, &self.raw);
            let tags = tags_from_blocks(&proj.blocks);
            let regions = crate::block_regions::from_blocks(&self.raw, self.is_org, &proj.blocks);
            crate::block_regions::extend_author_facet(&mut properties, &regions);
            let visible = regions
                .apply(&self.raw, self.is_org, crate::block_regions::Edit::Visible)
                .expect("parsed regions");
            let regions = shared_regions(regions);
            let visible_lower = crate::search_query::canonical_fold(&visible);
            let reference_source = crate::reference_evidence::project_with_config(
                &self.raw,
                self.is_org,
                &proj.blocks,
                &regions,
                self.property_ref_config
                    .as_deref()
                    .unwrap_or_else(|| crate::query::atom::ParseConfig::default_ref()),
            );
            // OG 0.10.9 with-page-refs includes headline tags and linkable
            // property atoms. Queries use the same evidence as backlinks.
            let mut refs_page = proj.refs.page;
            let tag_refs = reference_source
                .explicit
                .iter()
                .filter(|r| r.rule == "explicit_tag" || r.rule == "implicit_linkable_property");
            if tag_refs.clone().next().is_some() {
                refs_page.extend(tag_refs.map(|r| r.name.clone()));
                refs_page.sort();
                refs_page.dedup();
            }
            // OG `with-page-refs` also puts the block's marker and priority in its
            // refs, so `[[A]]` / `[[TODO]]` queries find them. They stay out of
            // `refs_page`, the virtual-page list behind `[[` autocomplete: Tine
            // does not offer a page per marker or priority letter.
            let mut refs_norm: Vec<String> = refs_page
                .iter()
                .map(|r| crate::refs::normalize(r))
                .collect();
            for r in reference_source
                .explicit
                .iter()
                .filter(|r| r.rule == "explicit_task_marker" || r.rule == "explicit_priority")
            {
                let key = crate::refs::normalize(&r.name);
                if !refs_norm.contains(&key) {
                    refs_norm.push(key);
                }
            }
            // These lists never grow after publication. Boxed slices trim the
            // build's slack and avoid retaining a capacity word per list; serde
            // keeps the same sequence encoding, including in checkpoints.
            let plain_ranges = reference_source.plain_ranges.into_boxed_slice();
            let extra = ProjectionExtra {
                search: std::sync::OnceLock::new(),
                refs_norm: refs_norm.into_boxed_slice(),
                refs_page: refs_page.into_boxed_slice(),
                block_refs: proj.refs.block.into_boxed_slice(),
                marker,
                priority,
                heading_level,
                properties: properties.into_boxed_slice(),
                scheduled,
                deadline,
                tags: tags.into_boxed_slice(),
                explicit: reference_source.explicit.into_boxed_slice(),
                withheld_ranges: reference_source.withheld_ranges.into_boxed_slice(),
            };
            let extra = (!extra.is_empty()).then(|| Box::new(extra));
            let visible_lower = (visible_lower != visible).then(|| visible_lower.into_boxed_str());
            let visible = (visible != self.raw).then(|| visible.into_boxed_str());
            BlockProjection {
                regions,
                visible,
                visible_lower,
                visible_literal: std::sync::OnceLock::new(),
                plain_ranges,
                extra,
            }
        })
    }

    /// `key:: value` block properties as lsdoc projects them (md trailer / org
    /// `:PROPERTIES:` drawer; fence-aware — a `key::` inside a code fence is content).
    pub fn properties(&self) -> Vec<(String, String)> {
        self.projection().properties().to_vec()
    }

    pub fn property(&self, key: &str) -> Option<String> {
        let key = property_key_norm(key);
        self.projection()
            .properties()
            .iter()
            .find(|(k, _)| property_key_norm(k) == key)
            .map(|(_, v)| v.clone())
    }

    pub fn collapsed(&self) -> bool {
        self.property("collapsed").as_deref() == Some("true")
    }

    /// The leading task marker, if any (`TODO`, `DOING`, ...), off lsdoc's first node.
    pub fn marker(&self) -> Option<&str> {
        self.projection().marker()
    }

    /// The block-header `[#A]` priority (`"A"`/`"B"`/`"C"`), off lsdoc's first node —
    /// header position only (a mid-text `[#A]` is not a priority).
    pub fn priority(&self) -> Option<&str> {
        self.projection().priority()
    }

    /// Heading level (1..=6) if the block body is an ATX heading, else `None`.
    pub fn heading_level(&self) -> Option<u8> {
        self.projection().heading_level()
    }

    /// The block's *visible* text (original case): `raw` minus property/drawer
    /// ranges. The body a reader sees — for breadcrumb labels and sort keys.
    pub fn visible_text(&self) -> &str {
        self.projection().visible_text(&self.raw)
    }

    /// [`Self::visible_text`] folded for the graph's search policy:
    /// accent-removing `canonical_fold` when `remove_accents`, else `literal_fold`.
    /// The ONE reader of the folded text; each form is computed once per body.
    pub fn visible_folded(&self, remove_accents: bool) -> &str {
        self.projection().visible_folded(&self.raw, remove_accents)
    }

    /// SCHEDULED / DEADLINE planning date text, when lsdoc emits a real `Timestamp`
    /// (code/fence-robust). For the render badge + agenda.
    pub fn scheduled(&self) -> Option<&str> {
        self.projection().scheduled()
    }
    pub fn deadline(&self) -> Option<&str> {
        self.projection().deadline()
    }

    /// Inline `#tag` / org headline tags off the same lsdoc projection as the
    /// other facets.
    pub fn tags(&self) -> Vec<String> {
        self.projection().tags().to_vec()
    }
}

fn shared_regions(
    mut regions: crate::block_regions::BlockRegions,
) -> std::sync::Arc<crate::block_regions::BlockRegions> {
    static EMPTY: std::sync::OnceLock<std::sync::Arc<crate::block_regions::BlockRegions>> =
        std::sync::OnceLock::new();
    if regions == crate::block_regions::BlockRegions::default() {
        std::sync::Arc::clone(EMPTY.get_or_init(|| std::sync::Arc::new(Default::default())))
    } else {
        // Published projections only borrow these buffers. Release parser
        // growth capacity once, before sharing them for the page's lifetime.
        regions.literals.shrink_to_fit();
        regions.property_regions.shrink_to_fit();
        regions.properties.shrink_to_fit();
        regions.planning.shrink_to_fit();
        regions.drawers.shrink_to_fit();
        regions.literal_blocks.shrink_to_fit();
        for drawer in &mut regions.drawers {
            drawer.clocks.shrink_to_fit();
        }
        std::sync::Arc::new(regions)
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

pub(crate) fn tag_text(inlines: &[lsdoc::ast::Inline], out: &mut String) {
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
    /// Byte offset of the start of the source line, to tell which timestamps share a line.
    line_start: usize,
}

/// The line-leading planning timestamps of ONE inline flow, by index into `inlines`.
/// Map each parser span from the re-bulleted input (OG's ECMAScript left trim) to
/// its original raw slice. A timestamp is line-leading when only horizontal
/// whitespace, or an EARLIER line-leading planning timestamp of the same line,
/// precedes it: `SCHEDULED: <a> DEADLINE: <b>` on one line is two planning
/// timestamps (OG renders every Scheduled/Deadline Timestamp as nothing in the body
/// and shows both chips). `has_trailing_body` is about the text after the LAST
/// timestamp of such a run, which is ordinary body content. Deliberate OG
/// divergence: a parser-recognized mid-text `Discuss SCHEDULED: <…>` remains
/// content in Tine rather than header chrome.
fn planning_lines<'a>(
    inlines: &[lsdoc::ast::Inline],
    raw: &'a str,
) -> Vec<(usize, PlanningSourceLine<'a>)> {
    use lsdoc::ast::Inline;
    let lead = raw.len() - tine_outline::lsdoc_block_parse::trim_block_start(raw).len();
    // (index, text, line_start, end, line_end)
    let mut found: Vec<(usize, &'a str, usize, usize, usize)> = Vec::new();
    let mut last: Option<(usize, usize)> = None; // (line_start, end) of the previous accepted one
    for (index, i) in inlines.iter().enumerate() {
        let Inline::Timestamp {
            ts,
            span: Some(span),
            ..
        } = i
        else {
            continue;
        };
        if ts != "Scheduled" && ts != "Deadline" {
            continue;
        }
        let (Some(start), Some(end)) = (
            span.0.checked_sub(2).and_then(|s| s.checked_add(lead)),
            span.1.checked_sub(2).and_then(|e| e.checked_add(lead)),
        ) else {
            continue;
        };
        let Some(text) = raw.get(start..end) else {
            continue;
        };
        let line_start = raw[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = raw[end..].find('\n').map_or(raw.len(), |i| end + i);
        let after_earlier = last.is_some_and(|(ls, e)| {
            ls == line_start
                && raw
                    .get(e..start)
                    .is_some_and(tine_outline::editor_text::js_blank)
        });
        if !tine_outline::editor_text::js_blank(&raw[line_start..start]) && !after_earlier {
            continue;
        }
        found.push((index, text, line_start, end, line_end));
        last = Some((line_start, end));
    }
    // Trailing body is judged once per run, after its last timestamp.
    found
        .iter()
        .map(|&(index, text, line_start, _end, line_end)| {
            let run_end = found
                .iter()
                .filter(|f| f.2 == line_start)
                .map(|f| f.3)
                .max()
                .unwrap_or(line_start);
            (
                index,
                PlanningSourceLine {
                    text,
                    has_trailing_body: !tine_outline::editor_text::js_blank(
                        &raw[run_end..line_end],
                    ),
                    line_start,
                },
            )
        })
        .collect()
}

/// SCHEDULED / DEADLINE display text (`<…>` content) for parser-recognized
/// Timestamp nodes that start a source line (or follow another planning timestamp
/// of that line). lsdoc can put same-line or next-line
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
        for (index, line) in planning_lines(inlines, raw) {
            let Inline::Timestamp { ts, .. } = &inlines[index] else {
                continue;
            };
            let slot = match ts.as_str() {
                "Scheduled" => &mut scheduled,
                "Deadline" => &mut deadline,
                _ => continue,
            };
            if slot.is_none() {
                *slot = angle_after(line.text, ts);
            }
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

fn inline_is_blank_text(i: &lsdoc::ast::Inline) -> bool {
    matches!(i, lsdoc::ast::Inline::Plain { text, .. } if tine_outline::editor_text::js_blank(text))
}

fn inline_is_empty(i: &lsdoc::ast::Inline) -> bool {
    inline_is_break(i) || inline_is_blank_text(i)
}

/// Remove parser-confirmed line-leading planning timestamps from the body AST
/// without deleting body content that shares their Paragraph (#75). A neighboring
/// line break is removed only when the timestamp has no same-line body suffix;
/// mid-text timestamps are untouched.
pub fn strip_planning_lines(
    mut blocks: Vec<lsdoc::ast::Block>,
    raw: &str,
) -> Vec<lsdoc::ast::Block> {
    use lsdoc::ast::Block;
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
        let planning: Vec<(usize, usize, bool)> = planning_lines(inlines, raw)
            .into_iter()
            .map(|(index, line)| (index, line.line_start, line.has_trailing_body))
            .collect();
        if planning.is_empty() {
            return true;
        }

        let mut remove = vec![false; inlines.len()];
        let mut previous: Option<(usize, usize)> = None; // (index, line_start)
        for (index, line_start, has_trailing_body) in planning {
            remove[index] = true;
            // The blanks between two timestamps of one line go with them.
            if let Some((before, before_line)) = previous {
                if before_line == line_start {
                    for k in before + 1..index {
                        if inline_is_blank_text(&inlines[k]) {
                            remove[k] = true;
                        }
                    }
                }
            }
            previous = Some((index, line_start));
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

pub fn property_key_norm(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

pub use crate::property_line::parse_property_line;

/// Rewrite every line terminator to `\n`. Page text ends a line at `\r\n`,
/// `\n` or a lone `\r`, as mldoc (`eol_chars = ['\r'; '\n']`) and lsdoc's lexer
/// do. Borrows when the text has no `\r`; otherwise two O(n) copies (CRLF
/// first, then lone CR, so `\r\r\n` becomes `\n\n`).
pub fn normalize_line_endings(content: &str) -> std::borrow::Cow<'_, str> {
    if content.contains('\r') {
        std::borrow::Cow::Owned(content.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(content)
    }
}

/// The terminator a new line gets in `existing` text: CRLF when it has any,
/// else `\r` when it has a lone one, else LF. Reused lines keep their own.
/// One answer for the Markdown writers (store saves, page merge). O(n).
pub fn line_convention(existing: &str) -> &'static str {
    if existing.contains("\r\n") {
        "\r\n"
    } else if existing.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

/// Parse Markdown page text into a [`Document`]: a pre-block plus a block
/// forest. lsdoc decides which lines open a block and how blocks nest
/// (`crate::outline`, the outline authority): dash bullets and unbulleted ATX
/// headings open blocks; fences and `#+BEGIN_…` regions hide look-alike lines,
/// exactly as mldoc reads the page for OG. A block's `raw` is its header line
/// after the structural prefix (`- ` and indentation; nothing for an
/// unbulleted heading, whose whole line is `raw`) plus every following line up
/// to the next header, dedented by the bullet's content column. The
/// pre-block is everything before the first header, trailing blank lines
/// dropped (the separator is re-added on write).
///
/// Ends a line at `\r\n`, `\n` or a lone `\r`; the model is LF-canonical, so the
/// file's terminators are restored at the write boundary (tine-store
/// `model/line_endings.rs`). Assigns no block uuids (empty). Does not enforce
/// the nesting ceiling — callers validate first
/// (`parse_input_depth_within_limit`). A page whose outline lsdoc cannot
/// represent one block per line (or will not own) has no blocks: the whole
/// text is the pre-block, so a save writes it back unchanged. Pure,
/// infallible; one lsdoc parse, O(page bytes).
pub fn parse(content: &str) -> Document {
    parse_document_with(content, |_, _, _| ()).0
}

/// Parse a Markdown page and infer its serialization layout from the SAME
/// lsdoc outline. Returns the document and formatting knobs together, so a
/// writer inspecting old content need not parse it again to detect layout.
/// Pure, infallible; one lsdoc outline parse, O(page bytes). Line terminators
/// are normalized for parsing; the store restores them when writing.
pub fn parse_with_opts(content: &str) -> (Document, SerializeOpts) {
    parse_document_with(content, SerializeOpts::from_outline)
}

fn parse_document_with<T>(
    content: &str,
    layout: impl FnOnce(&str, &[&str], &[outline::Header]) -> T,
) -> (Document, T) {
    // A stray `\r` in the model would pollute property / `id::` values.
    let normalized = normalize_line_endings(content);
    let content: &str = &normalized;
    let body = content.strip_suffix('\n').unwrap_or(content);
    let lines: Vec<&str> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };
    let headers = outline::headers_or_none(content, OutlineFormat::Markdown);
    let first = headers.first().map_or(lines.len(), |header| header.line);
    let mut pre_end = first;
    while pre_end > 0 && lines[pre_end - 1].trim().is_empty() {
        pre_end -= 1;
    }
    let opts = layout(content, &lines, &headers);
    let document = Document {
        pre_block: (pre_end > 0).then(|| lines[..pre_end].join("\n")),
        roots: outline::blocks(&lines, &headers, false),
    };
    (document, opts)
}

/// Whether lsdoc reads `line` alone as an unbulleted Markdown ATX heading
/// (`# Title`), the one block form written without a `- ` bullet. For a
/// writer that keeps such a heading unbulleted; the reparse stays the judge.
pub fn is_unbulleted_heading_line(line: &str) -> bool {
    outline::is_unbulleted_heading_line(line)
}

/// The raw text the outline parser reads from a continuation `line` of a block
/// whose continuations dedent by `content_indent` bytes: up to that many
/// leading tabs, spaces or form feeds removed. For a writer reusing a line's
/// old bytes; the reparse stays the judge.
pub fn continuation_raw(line: &str, content_indent: usize) -> &str {
    outline::strip_layout_ws(line, content_indent)
}

/// Formatting knobs detected from a file so re-saving preserves its existing
/// style (avoids gratuitous diffs / Syncthing churn). Logseq, for instance,
/// writes files with NO trailing newline; imposing one would rewrite every file.
#[derive(Debug, Clone)]
pub struct SerializeOpts {
    /// Trailing `\n` count to end the file with when the last block does not
    /// already end in blank lines. When it does (parse gives EOF blank lines to
    /// the last block), the serializer appends exactly one `\n`, so a detect →
    /// parse → serialize round trip reproduces the original count.
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
    /// Infer the three formatting knobs (trailing newlines, blank after the
    /// preamble, indent unit) of an existing file, read in its LF form. Line
    /// terminators are not detected here; the store writer restores them.
    /// `None` (new file) gives the default.
    pub fn detect(existing: Option<&str>) -> SerializeOpts {
        match existing {
            None => SerializeOpts::default(),
            Some(s) => {
                let s = normalize_line_endings(s);
                let lines: Vec<&str> = s.split('\n').collect();
                Self::from_outline(
                    &s,
                    &lines,
                    &outline::headers_or_none(&s, OutlineFormat::Markdown),
                )
            }
        }
    }
    fn from_outline(s: &str, lines: &[&str], headers: &[outline::Header]) -> Self {
        Self {
            trailing_newlines: crate::org::trailing_newlines(s),
            blank_after_props: match headers.first() {
                Some(header) if header.line > 0 => lines[header.line - 1].trim().is_empty(),
                _ => true,
            },
            indent: detect_indent(lines, headers),
        }
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
fn detect_indent(lines: &[&str], headers: &[outline::Header]) -> String {
    let mut space_widths: Vec<usize> = Vec::new();
    for header in headers {
        // Inspect only the structural prefix lsdoc already identified. Literal
        // lines cannot contribute a bogus indent unit.
        let prefix = &lines[header.line][..header.prefix_len];
        let lead_len = prefix.len() - prefix.trim_start_matches([' ', '\t']).len();
        if lead_len == 0 {
            continue;
        }
        if prefix[..lead_len].contains('\t') {
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

/// Serialize a Markdown [`Document`] from scratch (no source bytes), applying the
/// three detected knobs in [`SerializeOpts`]: indent unit, blank line after the
/// page-property preamble, and trailing newlines. Everything else is canonical:
/// `- ` bullets, continuation lines at indent + two spaces, and LF line endings
/// (the caller restores CRLF). Not for Org pages. Infallible, pure, O(blocks of
/// the page).
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
    // `parse` gives blank EOF lines to the last block's raw, so the body may
    // already end in them; they then need exactly the one `\n` that `parse`
    // strips. Appending the detected count again doubled them (2n-1) per save.
    let tail = s.len() - s.trim_end_matches('\n').len();
    s.push_str(&"\n".repeat(if tail > 0 { 1 } else { opts.trailing_newlines }));
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
#[path = "doc/property_fence_tests.rs"]
mod property_fence_tests;

#[cfg(test)]
#[path = "doc/unclosed_fence_tests.rs"]
mod unclosed_fence_outline_tests;

#[cfg(test)]
#[path = "doc/outline_authority_tests.rs"]
mod outline_authority_tests;

#[cfg(test)]
mod org_container_outline_tests {
    use super::*;

    /// Canonical save keeps the meaning (not the bytes) of the fixture.
    fn parse_structural_round_trip(input: &str) -> Document {
        let doc = parse(input);
        let canonical = serialize_with(&doc, &SerializeOpts::detect(Some(input)));
        assert_eq!(parse(&canonical), doc, "{input:?} -> {canonical:?}");
        doc
    }

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
    fn continuation_begin_region_owns_the_look_alike_sibling() {
        // lsdoc (= mldoc) closes the quote at the first compatible END, so
        // the `- sibling` line inside it is quote content (master HEAD).
        let input = "- parent\n  #+BEGIN_QUOTE\n- sibling\n  #+END_QUOTE";
        let doc = parse_structural_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert_eq!(
            doc.roots[0].raw,
            "parent\n#+BEGIN_QUOTE\n- sibling\n#+END_QUOTE"
        );
        assert!(doc.roots[0].children.is_empty());
    }

    #[test]
    fn a_deeper_closer_lane_still_closes_the_region() {
        let tabbed = "- #+BEGIN_QUOTE\n\t- child\n\t  #+END_QUOTE";
        let doc = parse_structural_round_trip(tabbed);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE\n- child\n #+END_QUOTE");

        let spaced = "- #+BEGIN_QUOTE\n  - child\n    #+END_QUOTE";
        let doc = parse_structural_round_trip(spaced);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "#+BEGIN_QUOTE\n- child\n  #+END_QUOTE");
    }

    #[test]
    fn tab_child_before_matching_end_is_region_content() {
        let input = "- \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse_structural_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "\t#+BEGIN_QUOTE\n- x\n #+END_QUOTE");
    }

    #[test]
    fn continuation_opener_before_tab_child_owns_it() {
        let input = "- p\n  \t#+BEGIN_QUOTE\n\t- x\n\t  #+END_QUOTE";
        let doc = parse_structural_round_trip(input);
        assert_eq!(doc.roots.len(), 1);
        assert!(doc.roots[0].children.is_empty());
        assert_eq!(doc.roots[0].raw, "p\n\t#+BEGIN_QUOTE\n- x\n #+END_QUOTE");
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
    fn lone_cr_org_container_parses_like_its_lf_twin() {
        // K01a (og 15a) closed the former lsdoc parity gap: a lone `\r` is a
        // line break, so the container decision is the LF file's.
        let old_mac = "- #+BEGIN_QUOTE\r  - x\r  #+END_QUOTE";
        let lf = old_mac.replace('\r', "\n");
        let doc = parse(old_mac);
        assert_eq!(doc, parse(&lf));
        assert_eq!(
            serialize_with(&doc, &SerializeOpts::detect(Some(old_mac))),
            serialize_with(&parse(&lf), &SerializeOpts::detect(Some(&lf)))
        );
    }
}

#[cfg(test)]
mod projection_tests;

/// A `OnceLock` in the launch-checkpoint form: its value when built, else
/// `None`; a `None` loads back unbuilt.
mod once_cell_as_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::OnceLock;

    pub(super) fn serialize<T: Serialize, S: Serializer>(
        cell: &OnceLock<T>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        cell.get().serialize(s)
    }

    pub(super) fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<OnceLock<T>, D::Error> {
        let cell = OnceLock::new();
        if let Some(value) = Option::<T>::deserialize(d)? {
            let _ = cell.set(value);
        }
        Ok(cell)
    }
}

fn deserialize_shared_regions<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<std::sync::Arc<crate::block_regions::BlockRegions>, D::Error> {
    Ok(shared_regions(
        crate::block_regions::BlockRegions::deserialize(d)?,
    ))
}

/// A block forest in the launch-checkpoint form (storage spec §7.6): each
/// block's raw text, format flag, memoized projection (when built) and
/// children. Runtime uuids are not stored; the loader reassigns them with
/// `projection::assign_doc_runtime_ids`, as a parse does. The serde wire form of
/// [`DocBlock`] cannot be used: it skips the uuid and the projection.
pub struct CheckpointBlocks<'a>(pub &'a [DocBlock]);

struct CheckpointBlockRef<'a>(&'a DocBlock);

impl Serialize for CheckpointBlocks<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter().map(CheckpointBlockRef))
    }
}

impl Serialize for CheckpointBlockRef<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let mut out = s.serialize_tuple(4)?;
        out.serialize_element(&self.0.raw)?;
        out.serialize_element(&self.0.is_org)?;
        out.serialize_element(&self.0.proj.get())?;
        out.serialize_element(&CheckpointBlocks(&self.0.children))?;
        out.end()
    }
}

/// Owned counterpart of [`CheckpointBlocks`]' elements.
#[derive(Deserialize)]
pub struct CheckpointBlock(String, bool, Option<BlockProjection>, Vec<CheckpointBlock>);

impl CheckpointBlock {
    /// The block, with an empty runtime uuid (assign one before use).
    pub fn into_block(self) -> DocBlock {
        let CheckpointBlock(raw, is_org, projection, children) = self;
        let proj = std::sync::OnceLock::new();
        if let Some(projection) = projection {
            let _ = proj.set(projection);
        }
        DocBlock {
            raw,
            children: children.into_iter().map(Self::into_block).collect(),
            uuid: String::new(),
            is_org,
            property_ref_config: None,
            proj,
        }
    }
}
