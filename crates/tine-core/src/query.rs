//! Backlinks and the `{{query}}` subset engine. Evaluated by scanning parsed
//! pages (no datalog). Pragmatic subset: page/tag refs, boolean and/or/not,
//! task markers, and property filters. Advanced datalog (`[:find ...]`) is
//! detected and reported as unsupported rather than crashed.

pub mod atom;
#[cfg(test)]
mod conformance;
pub(crate) mod eval;
pub mod ir;
pub mod macro_text;
pub(crate) mod og;
pub mod print;
pub mod registry;
pub(crate) mod tql;
pub mod view;

use eval::EvalCtx;
use ir::{
    Anchor, Attr, CmpOp, Filter, Leaf, Quant, Query, Rel, SortDir, Source, Value, ViewSettings,
};

use crate::date::JournalDate;
use crate::doc::{property_key_norm, DocBlock, Document};
use crate::model::{
    block_to_shallow_dto, dto_block_to_doc_block, BacklinkFilterContext, BacklinkFilterEntry,
    BacklinkFilterTarget, BlockDto, BlockPreview, Format, Graph, PageDto, PageEntry, PageKind,
    RefGroup, ReferenceBlockEvidence, ReferenceDiagnosticTrace, ReferenceDiagnostics,
    ReferenceKind, TemplateDto,
};
use crate::refs;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Query source crosses several boundaries (live macros, native IPC, static
/// publication, and export). Keep one shared ceiling so no caller can make the
/// parser or its cache key proportional to an unbounded graph-authored string.
pub const QUERY_SOURCE_MAX_BYTES: usize = 64 * 1024;
pub(crate) const QUERY_NESTING_MAX: usize = 64;

pub fn query_source_within_limit(source: &str) -> bool {
    source.len() <= QUERY_SOURCE_MAX_BYTES
}

/// Iterative, string/comment-aware guard before either recursive DSL parser.
/// Count parentheses because those are the only delimiters that construct
/// recursive predicates; brackets/braces are scanned iteratively as data.
pub fn query_nesting_within_limit(source: &str) -> bool {
    let semicolon_comments = is_advanced(source);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    for byte in source.bytes() {
        if in_comment {
            if byte == b'\n' {
                in_comment = false;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b';' if semicolon_comments => in_comment = true,
            b'"' => in_string = true,
            b'(' => {
                depth = depth.saturating_add(1);
                if depth > QUERY_NESTING_MAX {
                    return false;
                }
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

#[derive(Debug, Clone)]
pub struct BoundedGroups {
    pub groups: Vec<RefGroup>,
    pub total: usize,
    pub exceeded: bool,
}

/// The ONE result-construction accounting rule, shared by Direct Files and
/// managed storage.
///
/// It exists as a type rather than as an open-coded pair of counters because
/// the two storage modes had drifted apart on exactly this: Direct charged
/// `payload + page name + 256` per admitted row and latched `exceeded`, while
/// the managed block-referrer loop probed a group overhead per row but
/// accumulated it once per emitted group -- so the same `max_bytes` admitted a
/// different number of rows on the two paths for identical content.
pub(crate) struct ConstructionBudget {
    max_rows: usize,
    max_bytes: usize,
    rows: usize,
    bytes: usize,
    pub(crate) total: usize,
    pub(crate) exceeded: bool,
}

impl ConstructionBudget {
    pub(crate) fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            max_rows,
            max_bytes,
            rows: 0,
            bytes: 0,
            total: 0,
            exceeded: false,
        }
    }

    pub(crate) fn admit_estimated(&mut self, page: &str, payload_bytes: usize) -> bool {
        self.total = self.total.saturating_add(1);
        let bytes = payload_bytes.saturating_add(page.len()).saturating_add(256);
        if self.exceeded
            || self.rows >= self.max_rows
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            self.exceeded = true;
            return false;
        }
        self.rows += 1;
        self.bytes += bytes;
        true
    }

    pub(crate) fn deny_match(&mut self) {
        self.total = self.total.saturating_add(1);
        self.exceeded = true;
    }

    pub(crate) fn closed(&self) -> bool {
        self.exceeded || self.rows >= self.max_rows
    }
}

/// A page already declared to [`BoundedReferenceGroups`]. Opaque so a caller
/// cannot fabricate a slot for a page it never declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReferencePageSlot(usize);

struct BoundedReferenceGroup {
    date_key: Option<i64>,
    order_key: String,
    group: RefGroup,
}

/// The ONE bounded reference-result accumulator (I-12, I-13).
///
/// Direct Files' `collect_reference_occurrences_bounded`, managed backlinks and
/// unlinked references (`bound_application_reference_sources`) and the managed
/// block-referrer path each used to own a private copy of the same four rules —
/// [`ConstructionBudget`] admission, grouping duplicate logical page names under
/// the canonical [`crate::refs::page_key`], carrying `total`/`exceeded`, and the
/// OG display order — and the three copies had already drifted: the managed
/// block-referrer path grouped by storage path, so two files whose titles fold
/// to one logical page produced two reference groups where Direct produced one.
/// They are call sites now; this type is the algorithm.
///
/// Rows are admitted in the caller's declaration order, which every caller makes
/// source-path order, because the budget truncates and Direct Files charges
/// page-by-page in path order. Display order is applied only at [`Self::finish`].
///
/// `evidence` is parallel to `blocks` within a group. A producer is either
/// evidence-bearing for every row (backlinks, unlinked references) or for none
/// (block referrers); mixing the two within one group is not a supported shape.
pub(crate) struct BoundedReferenceGroups {
    budget: ConstructionBudget,
    declarations: Vec<ReferencePageDeclaration>,
    groups: Vec<BoundedReferenceGroup>,
    by_key: HashMap<String, usize>,
}

struct ReferencePageDeclaration {
    order_key: String,
    name: String,
    kind: PageKind,
    date_key: Option<i64>,
    group: Option<usize>,
}

impl BoundedReferenceGroups {
    pub(crate) fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            budget: ConstructionBudget::new(max_rows, max_bytes),
            declarations: Vec::new(),
            groups: Vec::new(),
            by_key: HashMap::new(),
        }
    }

    /// Declare the page whose rows follow. Declaring is free: a page that
    /// retains no row never reaches the result and never contributes its
    /// journal day or path to a group it shares a canonical key with.
    pub(crate) fn page(
        &mut self,
        order_key: &str,
        name: &str,
        kind: PageKind,
        date_key: Option<i64>,
    ) -> ReferencePageSlot {
        self.declarations.push(ReferencePageDeclaration {
            order_key: order_key.to_owned(),
            name: name.to_owned(),
            kind,
            date_key,
            group: None,
        });
        ReferencePageSlot(self.declarations.len() - 1)
    }

    pub(crate) fn closed(&self) -> bool {
        self.budget.closed()
    }

    /// Count a match the budget will not construct.
    pub(crate) fn deny(&mut self) {
        self.budget.deny_match();
    }

    /// Create or join the group this declaration belongs to, merging duplicate
    /// logical page names under the canonical [`crate::refs::page_key`].
    /// `order_key` (the source path in every caller) only breaks a display tie;
    /// the smallest wins, so order never depends on which duplicate came first.
    fn group_for(&mut self, slot: ReferencePageSlot) -> usize {
        if let Some(index) = self.declarations[slot.0].group {
            return index;
        }
        let key = refs::page_key(&self.declarations[slot.0].name);
        let index = match self.by_key.get(&key).copied() {
            Some(index) => {
                let declaration = &self.declarations[slot.0];
                let existing = &mut self.groups[index];
                existing.date_key = match (existing.date_key, declaration.date_key) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (current @ Some(_), None) => current,
                    (None, other) => other,
                };
                if declaration.order_key < existing.order_key {
                    existing.order_key.clone_from(&declaration.order_key);
                }
                index
            }
            None => {
                let declaration = &self.declarations[slot.0];
                let index = self.groups.len();
                self.groups.push(BoundedReferenceGroup {
                    date_key: declaration.date_key,
                    order_key: declaration.order_key.clone(),
                    group: RefGroup {
                        page: declaration.name.clone(),
                        kind: declaration.kind,
                        blocks: Vec::new(),
                        evidence: Vec::new(),
                    },
                });
                self.by_key.insert(key, index);
                index
            }
        };
        self.declarations[slot.0].group = Some(index);
        index
    }

    /// Charge one row against the shared budget and, only if it is admitted,
    /// build and retain it. The row is constructed inside `build` so an
    /// over-budget match costs an estimate, never a DTO.
    pub(crate) fn admit_with(
        &mut self,
        slot: ReferencePageSlot,
        payload_bytes: usize,
        build: impl FnOnce() -> (BlockDto, Option<ReferenceBlockEvidence>),
    ) -> bool {
        let index = self.group_for(slot);
        let Self { budget, groups, .. } = self;
        let entry = &mut groups[index];
        if !budget.admit_estimated(&entry.group.page, payload_bytes) {
            return false;
        }
        let (block, evidence) = build();
        entry.group.blocks.push(block);
        if let Some(evidence) = evidence {
            entry.group.evidence.push(evidence);
        }
        true
    }

    /// [`Self::admit_with`] for a caller that already owns the constructed row.
    pub(crate) fn admit(
        &mut self,
        slot: ReferencePageSlot,
        block: BlockDto,
        evidence: Option<ReferenceBlockEvidence>,
        payload_bytes: usize,
    ) -> bool {
        self.admit_with(slot, payload_bytes, move || (block, evidence))
    }

    /// Drop the pages that retained nothing and apply the OG display order:
    /// newest journal day first, non-journal pages last, page name as the
    /// deterministic tie-break, source path breaking a name tie.
    pub(crate) fn finish(self) -> BoundedGroups {
        let mut groups = self.groups;
        groups.retain(|entry| !entry.group.blocks.is_empty());
        groups.sort_by(|a, b| reference_group_display_order(a, b));
        BoundedGroups {
            groups: groups.into_iter().map(|entry| entry.group).collect(),
            total: self.budget.total,
            exceeded: self.budget.exceeded,
        }
    }
}

/// The ONE display order for reference groups: journal day descending, then
/// page name, then the group's admission key.
///
/// It has exactly one production call site --
/// [`BoundedReferenceGroups::finish`] -- because every reference surface
/// (Direct occurrences, managed backlinks, managed unlinked references, managed
/// block referrers) reaches display order through that one accumulator. A
/// second caller would mean a second producer of this answer (I-12).
fn reference_group_display_order(
    a: &BoundedReferenceGroup,
    b: &BoundedReferenceGroup,
) -> std::cmp::Ordering {
    b.date_key
        .unwrap_or(i64::MIN)
        .cmp(&a.date_key.unwrap_or(i64::MIN))
        .then_with(|| a.group.page.cmp(&b.group.page))
        .then_with(|| a.order_key.cmp(&b.order_key))
}

/// The `yyyymmdd` ordinal of a journal title read with the DEFAULT title format
/// (e.g. "Jan 1st, 2022").
///
/// This is NOT the producer of "what day is this journal page". That question
/// has one config-aware owner, [`crate::date::JournalFormat::parse`], which is
/// what fills `PageEntry::date_key` for Direct Files and
/// `ApplicationQueryPage::journal` for the managed adapters. Deriving the day
/// from the title with the default format instead is what made managed
/// journal-range queries answer empty on every graph configuring a custom
/// `:journal/page-title-format` (REG-W4-C7B-MANAGED-JOURNAL-ORDINAL-001).
///
/// Two callers survive, and neither can make the two backends disagree:
///
/// - the sparse task-candidate evaluator, whose `ApplicationSparseQueryPage`
///   carries no day and is constructed in `direct_projection.rs` as well, so
///   BOTH backends read the same (equally default-bound) value there;
/// - [`resolve_date_token`], which reads a `(between …)` BOUND LITERAL the user
///   typed rather than a page's day, in code shared by both backends.
///
/// Giving either the configured format is a follow-up, not a parity defect.
fn journal_ordinal(title: &str) -> Option<i64> {
    JournalDate::from_title(title).map(|d| d.ordinal_key())
}

/// Walk all blocks of a document depth-first, calling `f(block)`.
fn walk<'a>(blocks: &'a [DocBlock], f: &mut impl FnMut(&'a DocBlock)) {
    for b in blocks {
        f(b);
        walk(&b.children, f);
    }
}

type PathRefCounts = std::collections::HashMap<String, usize>;

fn push_path_refs(block: &DocBlock, refs: &mut PathRefCounts) {
    for reference in &block.projection().refs_norm {
        *refs.entry(reference.clone()).or_default() += 1;
    }
}

fn pop_path_refs(block: &DocBlock, refs: &mut PathRefCounts) {
    for reference in &block.projection().refs_norm {
        let remove = if let Some(count) = refs.get_mut(reference) {
            *count -= 1;
            *count == 0
        } else {
            false
        };
        if remove {
            refs.remove(reference);
        }
    }
}

/// Walk all blocks while maintaining the normalized union of ancestor refs.
/// This mirrors OG's materialized `:block/path-refs` without adding a second
/// persistent index or turning deep outlines into an O(nodes * depth) scan.
fn walk_path_refs<'a>(
    blocks: &'a [DocBlock],
    refs: &mut PathRefCounts,
    track_refs: bool,
    f: &mut impl FnMut(&'a DocBlock, &PathRefCounts),
) {
    for block in blocks {
        f(block, refs);
        if track_refs {
            push_path_refs(block, refs);
        }
        walk_path_refs(&block.children, refs, track_refs, f);
        if track_refs {
            pop_path_refs(block, refs);
        }
    }
}

/// Collect matches in document order while evaluating every candidate exactly
/// once. OG query presentation removes a result only when its *immediate parent*
/// is also in the unfiltered result set (`tree/filter-top-level-blocks`); it does
/// not prune the rest of a matching block's subtree. Reference occurrence
/// surfaces use `suppress_direct_child = false` because every referring block is
/// independently countable/navigable.
fn collect_matching_path<'a, M, T>(
    blocks: &'a [DocBlock],
    path: &mut Vec<&'a DocBlock>,
    parent_matched: bool,
    suppress_direct_child: bool,
    classify: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock]) -> Option<M>,
    materialize: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock], M) -> Option<T>,
    out: &mut Vec<T>,
) {
    for block in blocks {
        let classification = classify(block, path);
        let matched = classification.is_some();
        if !suppress_direct_child || !parent_matched {
            if let Some(classification) = classification {
                if let Some(item) = materialize(block, path, classification) {
                    out.push(item);
                }
            }
        }
        path.push(block);
        collect_matching_path(
            &block.children,
            path,
            matched,
            suppress_direct_child,
            classify,
            materialize,
            out,
        );
        path.pop();
    }
}

fn collect_og_query_roots<'a, M, T>(
    blocks: &'a [DocBlock],
    path: &mut Vec<&'a DocBlock>,
    path_refs: &mut PathRefCounts,
    track_path_refs: bool,
    parent_matched: bool,
    classify: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock], &PathRefCounts) -> Option<M>,
    materialize: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock], M) -> Option<T>,
    out: &mut Vec<T>,
) {
    for block in blocks {
        let classification = classify(block, path, path_refs);
        let matched = classification.is_some();
        if !parent_matched {
            if let Some(classification) = classification {
                if let Some(item) = materialize(block, path, classification) {
                    out.push(item);
                }
            }
        }
        path.push(block);
        if track_path_refs {
            push_path_refs(block, path_refs);
        }
        collect_og_query_roots(
            &block.children,
            path,
            path_refs,
            track_path_refs,
            matched,
            classify,
            materialize,
            out,
        );
        if track_path_refs {
            pop_path_refs(block, path_refs);
        }
        path.pop();
    }
}

fn collect_reference_matches<'a, M, T>(
    blocks: &'a [DocBlock],
    path: &mut Vec<&'a DocBlock>,
    classify: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock]) -> Option<M>,
    materialize: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock], M) -> Option<T>,
    out: &mut Vec<T>,
) {
    collect_matching_path(blocks, path, false, false, classify, materialize, out);
}

fn crumb_line_estimated_bytes(block: &DocBlock) -> usize {
    let line = block.visible_text().lines().next().unwrap_or("").trim();
    let mut chars = line.chars();
    let bytes = chars
        .by_ref()
        .take(crate::doc::CRUMB_MAX_CHARS)
        .map(char::len_utf8)
        .sum::<usize>();
    bytes + usize::from(chars.next().is_some()) * '…'.len_utf8()
}

fn shallow_dto_estimated_bytes(block: &DocBlock, ancestors: &[&DocBlock]) -> usize {
    let projection = block.projection();
    let id_bytes = if block.uuid.is_empty() {
        36
    } else {
        block.uuid.len()
    };
    id_bytes
        .saturating_add(block.raw.len())
        .saturating_add(
            ancestors
                .iter()
                .map(|ancestor| crumb_line_estimated_bytes(ancestor))
                .sum::<usize>(),
        )
        .saturating_add(projection.tags.iter().map(String::len).sum::<usize>())
        .saturating_add(
            projection
                .properties
                .iter()
                .map(|(key, value)| key.len().saturating_add(value.len()))
                .sum::<usize>(),
        )
        .saturating_add(128)
}

pub(crate) fn reference_evidence_estimated_bytes(evidence: &ReferenceBlockEvidence) -> usize {
    evidence.block_id.len()
        + evidence
            .occurrences
            .iter()
            .map(|occurrence| {
                occurrence
                    .matched_name
                    .len()
                    .saturating_add(occurrence.canonical.len())
                    .saturating_add(occurrence.rule.len())
                    .saturating_add(std::mem::size_of_val(occurrence))
            })
            .sum::<usize>()
}

fn result_dto(block: &DocBlock) -> BlockDto {
    #[cfg(test)]
    RESULT_DTO_CONSTRUCTIONS.with(|count| count.set(count.get().saturating_add(1)));
    block_to_shallow_dto(block)
}

#[cfg(test)]
thread_local! {
    static RESULT_DTO_CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Cancellable variant used by interactive search. Returning false from `f`
/// stops the entire depth-first walk, including the current deep page.
/// Collect matching blocks from an exact candidate set, or from the complete
/// already-parsed graph when no safe candidate set is available. The parser
/// remains the semantic authority; this helper performs no disk I/O or parsing.
fn collect_bounded_candidates(
    graph: &Graph,
    candidate_pages: Option<Vec<(PageEntry, std::sync::Arc<Document>)>>,
    mut keep: impl FnMut(&DocBlock) -> bool,
    mut keep_page_properties: impl FnMut(&PageEntry, &str) -> Option<BlockDto>,
    exclude: Option<&str>,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let ex = exclude.map(refs::normalize);
    let mut budget = ConstructionBudget::new(max_rows, max_bytes);
    let groups = graph.with_pages(|all_pages| {
        let pages = candidate_pages.as_deref().unwrap_or(all_pages);
        // Pair each group with the referring page's journal `date_key` so the result
        // can be ordered like OG (the page cache itself is in arbitrary read_dir order).
        let mut groups: Vec<(Option<i64>, RefGroup)> = Vec::new();
        for (entry, doc) in pages {
            if ex.as_deref() == Some(&refs::normalize(&entry.name)) {
                continue;
            }
            let mut matched: Vec<BlockDto> = Vec::new();
            if let Some(pre) = doc.pre_block.as_deref() {
                if let Some(property_ref) = keep_page_properties(entry, pre) {
                    if budget.admit_estimated(
                        &entry.name,
                        crate::model::block_dto_estimated_bytes(&property_ref),
                    ) {
                        matched.push(property_ref);
                    }
                }
            }
            let mut path: Vec<&DocBlock> = Vec::new();
            collect_reference_matches(
                &doc.roots,
                &mut path,
                &mut |block, _| keep(block).then_some(()),
                &mut |block, ancestors, ()| {
                    if budget.closed() {
                        budget.deny_match();
                        return None;
                    }
                    if !budget
                        .admit_estimated(&entry.name, shallow_dto_estimated_bytes(block, ancestors))
                    {
                        return None;
                    }
                    let mut dto = result_dto(block);
                    dto.breadcrumb = ancestors
                        .iter()
                        .map(|ancestor| crate::doc::crumb_line(ancestor))
                        .collect();
                    Some(dto)
                },
                &mut matched,
            );
            if !matched.is_empty() {
                groups.push((
                    entry.date_key,
                    RefGroup {
                        page: entry.name.clone(),
                        kind: entry.kind,
                        blocks: matched,
                        evidence: Vec::new(),
                    },
                ));
            }
        }
        // OG parity (components/block.cljs:3521 `sort-by :block/journal-day >`): order the
        // reference groups by the referring page's journal day DESCENDING — newest journal
        // day first, non-journal pages (date_key None → i64::MIN) last. The graph cache
        // inherits filesystem enumeration order, so use the page name as a deterministic
        // tie-breaker. Without it, static Guide exports differed across machines.
        groups.sort_by(|a, b| {
            b.0.unwrap_or(i64::MIN)
                .cmp(&a.0.unwrap_or(i64::MIN))
                .then_with(|| a.1.page.cmp(&b.1.page))
        });
        groups.into_iter().map(|(_, g)| g).collect()
    });
    BoundedGroups {
        groups,
        total: budget.total,
        exceeded: budget.exceeded,
    }
}

/// True when every non-empty line of a block's raw text is a `key:: value`
/// property line — i.e. the block carries only properties. OG treats such a
/// FIRST block as the page-properties (pre-)block. Empty (no property) → false.
fn is_properties_only(raw: &str) -> bool {
    let mut saw_prop = false;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if crate::doc::parse_property_line(line).is_none() {
            return false;
        }
        saw_prop = true;
    }
    saw_prop
}

/// Map of `alias::` → canonical page name (original case). The alias key is
/// normalized for lookup. Page-level `alias::` comes from the page pre-block
/// (Logseq's on-disk file convention) OR — when the user typed it as the first
/// bullet in the outliner — from a properties-only first block, which OG also
/// treats as page properties (GH #62). Without the latter, `- alias:: book`
/// typed in the editor never registers as an alias, so link navigation and
/// backlinks don't merge the two pages.
/// The normalized aliases contributed by one document, using the exact same
/// page-property rules as [`page_aliases`]. Keeping this extraction shared also
/// lets cache invalidation compare the old and new semantic alias sets instead
/// of treating the mere presence of an unchanged `alias::` line as a change.
pub(crate) fn document_aliases(doc: &Document) -> Vec<String> {
    let alias_text: Option<&str> = match &doc.pre_block {
        Some(pre) => Some(pre.as_str()),
        // No pre-block: a properties-only FIRST block is the page-properties
        // block in OG (it gets written back as a pre-block on save there).
        None => doc
            .roots
            .first()
            .filter(|b| is_properties_only(&b.raw))
            .map(|b| b.raw.as_str()),
    };
    let Some(text) = alias_text else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    for line in text.lines() {
        if let Some((k, v)) = crate::doc::parse_property_line(line) {
            let key = property_key_norm(&k);
            if key == "alias" || key == "aliases" {
                let trimmed = v.trim();
                if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
                    continue;
                }
                for alias in v.split([',', '，']) {
                    let alias = strip_ref(alias.trim());
                    if !alias.is_empty() {
                        aliases.push(refs::page_key(&alias));
                    }
                }
            }
        }
    }
    // Ordering and duplicate spelling do not alter alias resolution. Comparing
    // the semantic set avoids graph-wide invalidation for harmless formatting.
    aliases.sort_unstable();
    aliases.dedup();
    aliases
}

fn sorted_alias_owners(
    mut owned: Vec<(std::path::PathBuf, String, String)>,
) -> Vec<(String, String)> {
    // Keep every owner for duplicate aliases. Sorting makes the public alias
    // relation stable without collapsing edges needed by component resolution.
    owned.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    owned
        .into_iter()
        .map(|(_, alias, owner)| (alias, owner))
        .collect()
}

pub fn page_aliases(graph: &Graph) -> Vec<(String, String)> {
    graph.with_pages(|pages| {
        let mut owned = Vec::new();
        for (entry, doc) in pages {
            for alias in document_aliases(doc) {
                owned.push((entry.path.clone(), alias, entry.name.clone()));
            }
        }
        sorted_alias_owners(owned)
    })
}

pub(crate) fn page_aliases_with_owners(graph: &Graph) -> Vec<(String, String, String)> {
    graph.with_pages(|pages| {
        let mut owned = Vec::new();
        for (entry, doc) in pages {
            for alias in document_aliases(doc) {
                owned.push((
                    entry.path.clone(),
                    alias,
                    entry.name.clone(),
                    entry.rel_path.clone(),
                ));
            }
        }
        owned.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        owned
            .into_iter()
            .map(|(_, alias, owner, owner_rel_path)| (alias, owner, owner_rel_path))
            .collect()
    })
}

pub(crate) type RealPageNames = std::collections::HashMap<String, (std::path::PathBuf, String)>;

pub(crate) fn real_page_names(graph: &Graph) -> RealPageNames {
    if let Some(indexed) = graph.reference_real_page_names() {
        return indexed;
    }
    graph.with_pages(|pages| {
        let mut real = RealPageNames::new();
        for (entry, _) in pages {
            let key = refs::page_key(&entry.name);
            match real.get_mut(&key) {
                Some((winner_path, winner_name)) if entry.path < *winner_path => {
                    *winner_path = entry.path.clone();
                    *winner_name = entry.name.clone();
                }
                Some(_) => {}
                None => {
                    real.insert(key, (entry.path.clone(), entry.name.clone()));
                }
            }
        }
        real
    })
}

/// Resolve a requested page/alias to its canonical display name, the complete
/// alias-connected component, and the real page to exclude as self. The
/// normalized component is shared by backlinks, unlinked references, and their
/// scoped-invalidation predicates so those paths cannot drift.
pub(crate) fn equivalent_page_names(
    real_pages: &RealPageNames,
    aliases: &[(String, String)],
    target: &str,
) -> (String, Vec<String>, String) {
    let target_norm = refs::page_key(target);
    let mut neighbors = std::collections::HashMap::<String, Vec<String>>::new();
    let mut original_names = vec![(target_norm.clone(), target.to_string())];
    for (alias, owner) in aliases {
        let alias_norm = refs::page_key(alias);
        let owner_norm = refs::page_key(owner);
        neighbors
            .entry(alias_norm.clone())
            .or_default()
            .push(owner_norm.clone());
        neighbors
            .entry(owner_norm.clone())
            .or_default()
            .push(alias_norm.clone());
        original_names.push((alias_norm, alias.clone()));
        original_names.push((owner_norm, owner.clone()));
    }

    let mut component = std::collections::BTreeSet::new();
    let mut pending = vec![target_norm.clone()];
    while let Some(name) = pending.pop() {
        if !component.insert(name.clone()) {
            continue;
        }
        if let Some(adjacent) = neighbors.get(&name) {
            pending.extend(adjacent.iter().cloned());
        }
    }

    let canonical = component
        .iter()
        .filter_map(|name| real_pages.get(name).map(|(_, stored)| stored))
        .min()
        .cloned()
        .or_else(|| {
            original_names
                .iter()
                .filter(|(key, _)| component.contains(key))
                .map(|(_, original)| original)
                .min()
                .cloned()
        })
        .unwrap_or_else(|| target.to_string());
    let self_page = real_pages
        .get(&target_norm)
        .map(|(_, stored)| stored.clone())
        .unwrap_or_else(|| canonical.clone());
    (canonical, component.into_iter().collect(), self_page)
}

fn graph_equivalent_page_names(
    graph: &Graph,
    aliases: &[(String, String)],
    target: &str,
) -> (String, Vec<String>, String) {
    equivalent_page_names(&real_page_names(graph), aliases, target)
}

fn org_property_line(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("#+") {
        return rest
            .split_once(':')
            .is_some_and(|(key, _)| !key.trim().is_empty());
    }
    trimmed
        .strip_prefix(':')
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(key, _)| !key.trim().is_empty())
}

/// Keep only page-property source lines from a document pre-block. Free-form
/// preamble text is not a Logseq page property and must not become a backlink.
fn page_property_raw(pre: &str, is_org: bool) -> String {
    pre.lines()
        .filter(|line| {
            crate::doc::parse_property_line(line).is_some() || (is_org && org_property_line(line))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn property_projection(raw: &str, is_org: bool) -> DocBlock {
    DocBlock {
        raw: raw.to_string(),
        children: Vec::new(),
        uuid: String::new(),
        is_org,
        proj: std::sync::OnceLock::new(),
    }
}

fn page_property_block(entry: &PageEntry, pre: &str) -> Option<DocBlock> {
    let is_org = Format::from_path(&entry.path) == Format::Org;
    page_property_block_parts(&entry.name, entry.kind, is_org, pre)
}

fn page_property_block_parts(
    name: &str,
    kind: PageKind,
    is_org: bool,
    pre: &str,
) -> Option<DocBlock> {
    let raw = page_property_raw(pre, is_org);
    if raw.is_empty() {
        return None;
    }
    let mut block = property_projection(&raw, is_org);
    block.uuid = format!("page-property:{:?}:{}", kind, refs::page_key(name));
    Some(block)
}

fn block_reference_evidence(
    block: &DocBlock,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> Option<ReferenceBlockEvidence> {
    let result = crate::reference_evidence::occurrences_of_kind_bounded(
        &block.raw,
        &block.projection().reference_source,
        canonical,
        names_norm,
        kind,
        config,
    );
    (!result.occurrences.is_empty()).then(|| ReferenceBlockEvidence {
        block_id: block.uuid.clone(),
        occurrences: result.occurrences,
        total: result.total,
        truncated: result.truncated,
    })
}

/// Exact parser-owned reference matches from one application-gateway page.
/// SQLite callers may restrict ordinary blocks by flattened parser index, but
/// the evidence and frontend identities always come from `PageDto`.
/// Reference matches within ONE page, over that page's projected forest.
///
/// `roots` is the page's `DocBlock` forest: Direct hands its cached
/// `Arc<Document>` roots, managed reads hand the forest retained by
/// `application_projection_roots`, so neither side converts a `BlockDto` tree
/// here. The walk itself is [`collect_reference_matches`] -- the same visitor
/// the Direct occurrence collector uses -- so pre-order block indexing,
/// ancestor breadcrumbs and result-row shape have ONE owner (I-12).
pub(crate) fn application_page_reference_matches(
    roots: &[DocBlock],
    page: &PageDto,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    allowed_indices: Option<&std::collections::HashSet<usize>>,
    allow_preamble: bool,
    config: &crate::config::Config,
) -> Vec<(BlockDto, ReferenceBlockEvidence)> {
    let is_org = page.format == Format::Org;
    let mut matches = Vec::new();
    if allow_preamble {
        if let Some(block) = page
            .pre_block
            .as_deref()
            .and_then(|pre| page_property_block_parts(&page.name, page.kind, is_org, pre))
        {
            if let Some(hit) = block_reference_evidence(&block, canonical, names_norm, kind, config)
            {
                let mut dto = block_to_shallow_dto(&block);
                dto.page_property = true;
                matches.push((dto, hit));
            }
        }
    }

    // Pre-order position of the block being classified, which is the index
    // `allowed_indices` is expressed in.
    let index = std::cell::Cell::new(0_usize);
    let mut path = Vec::new();
    let mut found: Vec<()> = Vec::new();
    collect_reference_matches(
        roots,
        &mut path,
        &mut |block, _| {
            let current = index.get();
            index.set(current.saturating_add(1));
            if allowed_indices.is_some_and(|allowed| !allowed.contains(&current)) {
                return None;
            }
            block_reference_evidence(block, canonical, names_norm, kind, config)
        },
        &mut |block, ancestors, hit| {
            let mut dto = result_dto(block);
            dto.breadcrumb = ancestors
                .iter()
                .map(|ancestor| crate::doc::crumb_line(ancestor))
                .collect();
            matches.push((dto, hit));
            None
        },
        &mut found,
    );
    matches
}

/// Block-level referrers within ONE managed page, over that page's projected
/// forest and the same [`collect_reference_matches`] visitor.
///
/// Replaces the managed twin that flattened the `BlockDto` tree, re-parsed
/// every block through `render::block_refs`, and built a throwaway `DocBlock`
/// per block just to produce a breadcrumb. Here the match test reads the
/// forest's memoized `block_refs` projection and breadcrumbs are computed only
/// for admitted rows.
pub(crate) fn application_page_block_referrers(
    roots: &[DocBlock],
    target: &str,
    allowed_indices: Option<&std::collections::HashSet<usize>>,
) -> Vec<BlockDto> {
    let index = std::cell::Cell::new(0_usize);
    let mut output = Vec::new();
    let mut path = Vec::new();
    let mut found: Vec<()> = Vec::new();
    collect_reference_matches(
        roots,
        &mut path,
        &mut |block, _| {
            let current = index.get();
            index.set(current.saturating_add(1));
            if allowed_indices.is_some_and(|allowed| !allowed.contains(&current)) {
                return None;
            }
            block
                .projection()
                .block_refs
                .iter()
                .any(|uuid| uuid == target)
                .then_some(())
        },
        &mut |block, ancestors, ()| {
            let mut dto = result_dto(block);
            dto.breadcrumb = ancestors
                .iter()
                .map(|ancestor| crate::doc::crumb_line(ancestor))
                .collect();
            output.push(dto);
            None
        },
        &mut found,
    );
    output
}

pub(crate) fn application_page_property_dto(page: &PageDto) -> Option<BlockDto> {
    let mut dto = block_to_shallow_dto(&page_property_block_parts(
        &page.name,
        page.kind,
        page.format == Format::Org,
        page.pre_block.as_deref()?,
    )?);
    dto.page_property = true;
    Some(dto)
}

fn block_has_reference(
    block: &DocBlock,
    names_norm: &[String],
    kind: ReferenceKind,
    config: &crate::config::Config,
) -> bool {
    crate::reference_evidence::has_occurrence_kind(
        &block.raw,
        &block.projection().reference_source,
        names_norm,
        kind,
        config,
    )
}

fn collect_reference_occurrences(
    graph: &Graph,
    canonical: &str,
    self_page: &str,
    names_norm: &[String],
    kind: ReferenceKind,
) -> Vec<RefGroup> {
    collect_reference_occurrences_bounded(
        graph,
        canonical,
        self_page,
        names_norm,
        kind,
        usize::MAX,
        usize::MAX,
    )
    .groups
}

fn collect_reference_occurrences_bounded(
    graph: &Graph,
    canonical: &str,
    self_page: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let exclude =
        refs::ReferenceSourceExclusions::new(self_page, graph.config.favorites_page.as_deref());
    let mut accumulator = BoundedReferenceGroups::new(max_rows, max_bytes);
    let candidate_pages = graph.reference_candidate_pages(names_norm, kind);
    let pages = candidate_pages.pages.as_slice();
    let mut sources = pages.iter().collect::<Vec<_>>();
    sources.sort_by(|(a, _), (b, _)| a.path.cmp(&b.path));
    for (entry, doc) in sources {
        if exclude.excludes_name(&entry.name) {
            continue;
        }
        let slot = accumulator.page(&entry.rel_path, &entry.name, entry.kind, entry.date_key);
        if let Some(mut block) = doc
            .pre_block
            .as_deref()
            .and_then(|pre| page_property_block(entry, pre))
        {
            if accumulator.closed() {
                if block_has_reference(&block, names_norm, kind, &graph.config) {
                    accumulator.deny();
                }
            } else if let Some(hit) =
                block_reference_evidence(&block, canonical, names_norm, kind, &graph.config)
            {
                // The page-property DTO is the estimate's own input here, so it
                // is built before admission on this one row (unchanged).
                let mut dto = block_to_shallow_dto(&block);
                dto.page_property = true;
                let estimated = crate::model::block_dto_estimated_bytes(&dto)
                    .saturating_add(reference_evidence_estimated_bytes(&hit));
                accumulator.admit(slot, dto, Some(hit), estimated);
            }
            block.children.clear();
        }
        let mut path = Vec::new();
        let mut found: Vec<()> = Vec::new();
        let construction_closed = std::cell::Cell::new(accumulator.closed());
        collect_reference_matches(
            &doc.roots,
            &mut path,
            &mut |block, _| {
                if construction_closed.get() {
                    block_has_reference(block, names_norm, kind, &graph.config).then_some(None)
                } else {
                    block_reference_evidence(block, canonical, names_norm, kind, &graph.config)
                        .map(Some)
                }
            },
            &mut |block, ancestors, hit| {
                let Some(hit) = hit else {
                    accumulator.deny();
                    construction_closed.set(true);
                    return None;
                };
                let estimated = shallow_dto_estimated_bytes(block, ancestors)
                    .saturating_add(reference_evidence_estimated_bytes(&hit));
                let admitted = accumulator.admit_with(slot, estimated, || {
                    let mut dto = result_dto(block);
                    dto.breadcrumb = ancestors
                        .iter()
                        .map(|ancestor| crate::doc::crumb_line(ancestor))
                        .collect();
                    (dto, Some(hit))
                });
                if !admitted {
                    construction_closed.set(true);
                    return None;
                }
                construction_closed.set(accumulator.closed());
                None
            },
            &mut found,
        );
    }
    accumulator.finish()
}

pub fn backlinks(graph: &Graph, target: &str) -> Vec<RefGroup> {
    let aliases = graph.page_aliases();
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, &aliases, target);
    collect_reference_occurrences(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Explicit,
    )
}

pub fn backlinks_bounded(
    graph: &Graph,
    target: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let aliases = graph.page_aliases();
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, &aliases, target);
    collect_reference_occurrences_bounded(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Explicit,
        max_rows,
        max_bytes,
    )
}

pub(crate) const BACKLINK_FILTER_MAX_BYTES: usize = 16 * 1024 * 1024;
const BACKLINK_FILTER_MAX_TEXT_BYTES: usize = 64 * 1024;
const BACKLINK_FILTER_MAX_FACETS: usize = 256;

fn append_bounded_text(out: &mut String, value: &str, max_bytes: usize) -> bool {
    if value.is_empty() || out.len() >= max_bytes {
        return !value.is_empty() && out.len() >= max_bytes;
    }
    if !out.is_empty() {
        if out.len() + 1 > max_bytes {
            return true;
        }
        out.push('\n');
    }
    let remaining = max_bytes.saturating_sub(out.len());
    if value.len() <= remaining {
        out.push_str(value);
        return false;
    }
    let mut end = remaining;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&value[..end]);
    true
}

pub(crate) fn backlink_filter_entry(
    page: &str,
    kind: PageKind,
    block: &DocBlock,
    excluded_refs: &std::collections::HashSet<String>,
    remaining_bytes: usize,
) -> BacklinkFilterEntry {
    let max_text = BACKLINK_FILTER_MAX_TEXT_BYTES.min(remaining_bytes);
    let mut text = String::new();
    let mut facets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut facets_truncated = false;

    let mut add_facet = |name: &str| {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let key = refs::normalize(name);
        if excluded_refs.contains(&key) || !seen.insert(key) {
            return;
        }
        if facets.len() >= BACKLINK_FILTER_MAX_FACETS {
            facets_truncated = true;
        } else {
            facets.push(name.to_string());
        }
    };

    fn visit(
        block: &DocBlock,
        depth: usize,
        text: &mut String,
        max_text: usize,
        add_facet: &mut impl FnMut(&str),
        truncated: &mut bool,
    ) {
        if depth > crate::model::MAX_MANAGED_BLOCK_DEPTH {
            *truncated = true;
            return;
        }
        *truncated |= append_bounded_text(text, block.visible_text(), max_text);
        let projection = block.projection();
        for name in &projection.refs_page {
            add_facet(name);
        }
        if let Some(marker) = projection.marker.as_deref() {
            add_facet(marker);
        }
        // OG treats tags::/alias:: property values as page references too. The
        // property boundary itself is parser-owned; only its comma-separated
        // semantic values are unwrapped here.
        for (key, value) in &projection.properties {
            if !(key.eq_ignore_ascii_case("tags")
                || key.eq_ignore_ascii_case("alias")
                || key.eq_ignore_ascii_case("aliases"))
            {
                continue;
            }
            let quoted = value.trim();
            if quoted.len() >= 2 && quoted.starts_with('"') && quoted.ends_with('"') {
                continue;
            }
            for value in value.split([',', '，']) {
                let name = strip_ref(value.trim());
                add_facet(&name);
            }
        }
        for child in &block.children {
            visit(child, depth + 1, text, max_text, add_facet, truncated);
        }
    }

    let mut text_truncated = false;
    visit(
        block,
        1,
        &mut text,
        max_text,
        &mut add_facet,
        &mut text_truncated,
    );
    BacklinkFilterEntry {
        page: page.to_string(),
        kind,
        block_id: block.uuid.clone(),
        text,
        facets,
        truncated: text_truncated || facets_truncated,
    }
}

pub(crate) fn backlink_filter_entry_estimated_bytes(entry: &BacklinkFilterEntry) -> usize {
    entry.text.len()
        + entry.facets.iter().map(String::len).sum::<usize>()
        + entry.page.len()
        + entry.block_id.len()
        + 128
}

/// Build search/facet metadata only for the shallow backlink roots already in
/// one rendered panel. This deliberately does not rerun backlink selection and
/// cannot turn into a graph-sized arbitrary export: the request is ID-scoped,
/// de-duplicated, and the response has both per-root and total byte ceilings.
pub fn backlink_filter_context(
    graph: &Graph,
    target: &str,
    targets: &[BacklinkFilterTarget],
) -> BacklinkFilterContext {
    let aliases = graph.page_aliases();
    let (_, names_norm, _) = graph_equivalent_page_names(graph, &aliases, target);
    let excluded_refs = names_norm
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut requested =
        std::collections::HashMap::<(PageKind, String), std::collections::HashSet<String>>::new();
    for item in targets {
        requested
            .entry((item.kind, refs::normalize(&item.page)))
            .or_default()
            .insert(item.block_id.clone());
    }
    let requested_unique = requested
        .values()
        .map(std::collections::HashSet::len)
        .sum::<usize>();

    let mut context = BacklinkFilterContext::default();
    let mut bytes = 0usize;
    graph.with_pages(|pages| {
        for (page, document) in pages {
            let Some(ids) = requested.get(&(page.kind, refs::normalize(&page.name))) else {
                continue;
            };
            if let Some(pre) = document.pre_block.as_deref() {
                if let Some(block) = page_property_block(page, pre) {
                    if ids.contains(&block.uuid) {
                        let entry = backlink_filter_entry(
                            &page.name,
                            page.kind,
                            &block,
                            &excluded_refs,
                            BACKLINK_FILTER_MAX_BYTES.saturating_sub(bytes),
                        );
                        let estimated = backlink_filter_entry_estimated_bytes(&entry);
                        if bytes.saturating_add(estimated) > BACKLINK_FILTER_MAX_BYTES {
                            context.truncated = true;
                        } else {
                            bytes += estimated;
                            // Same flag propagation as the ordinary-root loop
                            // below and the managed twin: an entry truncated at
                            // its own text/facet budget must mark the context,
                            // or Direct reports truncated=false where managed
                            // reports true for identical content (DUP-6).
                            context.truncated |= entry.truncated;
                            context.entries.push(entry);
                        }
                    }
                }
            }
            fn collect<'a>(
                blocks: &'a [DocBlock],
                ids: &std::collections::HashSet<String>,
                out: &mut Vec<&'a DocBlock>,
            ) {
                for block in blocks {
                    if ids.contains(&block.uuid) {
                        out.push(block);
                    }
                    collect(&block.children, ids, out);
                }
            }
            let mut blocks = Vec::new();
            collect(&document.roots, ids, &mut blocks);
            for block in blocks {
                if bytes >= BACKLINK_FILTER_MAX_BYTES {
                    context.truncated = true;
                    break;
                }
                let entry = backlink_filter_entry(
                    &page.name,
                    page.kind,
                    block,
                    &excluded_refs,
                    BACKLINK_FILTER_MAX_BYTES.saturating_sub(bytes),
                );
                let estimated = backlink_filter_entry_estimated_bytes(&entry);
                if bytes.saturating_add(estimated) > BACKLINK_FILTER_MAX_BYTES {
                    context.truncated = true;
                    break;
                }
                bytes += estimated;
                context.truncated |= entry.truncated;
                context.entries.push(entry);
            }
        }
    });
    if context.entries.len() < requested_unique {
        // Missing IDs can be stale results after an external edit; the frontend
        // can still search each root's shallow raw text but must not claim the
        // descendant index is complete.
        context.truncated = true;
    }
    context
}

/// Block-level referrers: every block across the graph that references the block
/// with `id:: uuid` (via `((uuid))`, `[..](((uuid)))`, or `{{embed ((uuid))}}`),
/// grouped by source page. Unlike page `backlinks`, this passes `exclude: None`,
/// so a referrer on the *same page* as the target is included — matching OG's
/// `get-block-referenced-blocks` (no self-page exclusion at the block level).
pub fn block_referrers(graph: &Graph, uuid: &str) -> Vec<RefGroup> {
    let u = uuid.trim();
    if u.is_empty() {
        return Vec::new();
    }
    collect_bounded_candidates(
        graph,
        graph.direct_projection_block_referrer_candidate_pages(u),
        |b| b.projection().block_refs.iter().any(|r| r == u),
        |_, _| None,
        None,
        usize::MAX,
        usize::MAX,
    )
    .groups
}

pub fn block_referrers_bounded(
    graph: &Graph,
    uuid: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let u = uuid.trim();
    if u.is_empty() {
        return BoundedGroups {
            groups: Vec::new(),
            total: 0,
            exceeded: false,
        };
    }
    collect_bounded_candidates(
        graph,
        graph.direct_projection_block_referrer_candidate_pages(u),
        |b| b.projection().block_refs.iter().any(|r| r == u),
        |_, _| None,
        None,
        max_rows,
        max_bytes,
    )
}

/// Unlinked references: parser-visible plain occurrences outside explicit
/// reference syntax. A block containing both kinds appears once in each surface,
/// with the corresponding occurrence evidence.
pub fn unlinked_refs(graph: &Graph, target: &str) -> Vec<RefGroup> {
    let aliases = graph.page_aliases();
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, &aliases, target);
    collect_reference_occurrences(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Plain,
    )
}

pub fn unlinked_refs_bounded(
    graph: &Graph,
    target: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let aliases = graph.page_aliases();
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, &aliases, target);
    collect_reference_occurrences_bounded(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Plain,
        max_rows,
        max_bytes,
    )
}

/// Target-scoped trace for bug reports. Membership comes from the exact same
/// occurrence engine as the panels; the deliberately uncached parser path makes
/// projection-cache drift visible. No launcher history is read or returned.
pub fn reference_diagnostics(graph: &Graph, target: &str) -> ReferenceDiagnostics {
    let aliases = graph.page_aliases();
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, &aliases, target);
    let excluded_page = refs::page_key(&self_page);
    let mut traces = graph.with_pages(|pages| {
        let mut traces = Vec::new();
        for (entry, document) in pages {
            let self_page = refs::normalize(&entry.name) == excluded_page;
            let mut inspect = |block: &DocBlock| {
                let occurrences = crate::reference_evidence::slow_occurrences(
                    &block.raw,
                    block.is_org,
                    &canonical,
                    &names_norm,
                    &graph.config,
                );
                let raw_lower = block.raw.to_lowercase();
                let textual_candidate = names_norm.iter().any(|name| raw_lower.contains(name));
                if occurrences.is_empty() && !textual_candidate {
                    return;
                }
                let explicit = occurrences
                    .iter()
                    .any(|occurrence| occurrence.kind == ReferenceKind::Explicit);
                let plain = occurrences
                    .iter()
                    .any(|occurrence| occurrence.kind == ReferenceKind::Plain);
                traces.push(ReferenceDiagnosticTrace {
                    page: entry.name.clone(),
                    kind: entry.kind,
                    block_id: block.uuid.clone(),
                    occurrences,
                    included_linked: !self_page && explicit,
                    included_unlinked: !self_page && plain,
                    exclusion_reason: if self_page {
                        Some("self_page_excluded".to_string())
                    } else if !explicit && !plain {
                        Some("parser_excluded_context_or_boundary".to_string())
                    } else {
                        None
                    },
                });
            };
            if let Some(block) = document
                .pre_block
                .as_deref()
                .and_then(|pre| page_property_block(entry, pre))
            {
                inspect(&block);
            }
            walk(&document.roots, &mut inspect);
        }
        traces
    });
    traces.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| a.block_id.cmp(&b.block_id))
    });
    ReferenceDiagnostics {
        engine_version: crate::reference_evidence::ENGINE_VERSION.to_string(),
        target: canonical,
        traces,
    }
}

/// Page-level properties and `tags::` values parsed from a page's pre-block.
fn page_facets(pre_block: Option<&str>) -> (Vec<(String, String)>, Vec<String>) {
    let mut props = Vec::new();
    let mut tags = Vec::new();
    if let Some(pre) = pre_block {
        for line in pre.lines() {
            if let Some((k, v)) = crate::doc::parse_property_line(line) {
                if property_key_norm(k) == "tags" {
                    tags = v
                        .split(',')
                        .map(|t| strip_ref(t.trim()))
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                props.push((k.to_string(), v.to_string()));
            }
        }
    }
    (props, tags)
}

pub fn run_query(graph: &Graph, query_src: &str) -> Vec<RefGroup> {
    run_query_bounded(graph, query_src, usize::MAX, usize::MAX).groups
}

pub fn run_query_bounded(
    graph: &Graph,
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    run_query_bounded_over(&GraphQueryPages(graph), query_src, max_rows, max_bytes)
}

/// Which surface syntax a query's text is written in (SPEC §4).
///
/// The macro name chooses it when the block is saved (Q3): `{{query …}}` is the
/// OG DSL, `{{tine-query …}}` is TQL. Both are the same IR afterwards — the
/// dialect is a property of the TEXT, never of the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDialect {
    Og,
    Tql,
}

/// The ONE text → IR entry, for either dialect: the I-22 source limits, then
/// the dialect's parser (§4.1, §4.2). A source that is refused, or is datalog
/// rather than a simple query, comes back as a query carrying its diagnostic —
/// never as a silently empty one.
pub fn parse_query_text(
    query_src: &str,
    dialect: QueryDialect,
    today: JournalDate,
) -> (Query, ViewSettings) {
    parse_query_text_with_registry(query_src, dialect, today, registry::Registry::none())
}

/// [`parse_query_text`] against a registry snapshot. The parse is identical;
/// only an `UnknownIdent` diagnostic differs, gaining the nearest property keys
/// the graph actually has as `prop('…')` suggestions (§4.2.2). The OG dialect
/// has no identifier vocabulary to be wrong about, so it ignores the registry.
pub fn parse_query_text_with_registry(
    query_src: &str,
    dialect: QueryDialect,
    today: JournalDate,
    registry: &registry::Registry,
) -> (Query, ViewSettings) {
    match dialect {
        QueryDialect::Og => parse_query_source(query_src, today),
        QueryDialect::Tql => {
            use ir::{Diagnostic, DiagnosticKind};
            if !query_source_within_limit(query_src) {
                let mut query = Query::new(
                    Anchor::Block,
                    Filter::False,
                    Source::Tql {
                        original: query_src.to_string(),
                        og_options: String::new(),
                    },
                );
                query.diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Size,
                    "the query source is too large",
                ));
                return (query, ViewSettings::default());
            }
            tql::parse_tql(query_src, registry)
        }
    }
}

/// **The macro-input dispatch (§7.1, C3).** Which INPUT a caller has, which is
/// not the same question as which grammar the text is written in.
///
/// `Og`, `Tql` and `Advanced` are explicit FORM inputs: the caller already knows
/// the grammar (the TQL pane, the `#+BEGIN_QUERY` container extractor). The two
/// `Macro*` inputs take the COMPLETE raw macro argument, without the outer
/// `{{`/`}}`, and are the only place a query argument is ever split — after this
/// wave nothing outside `query_parse` splits one (§4.3, Y2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryInput {
    /// An OG DSL form.
    Og,
    /// A TQL form: filter and anchor only, never options (§4.3.1).
    Tql,
    /// A datalog form, including a whole `{:query … :inputs …}` map (§4.4).
    Advanced,
    /// The complete argument of a `{{query …}}` macro: OG or advanced.
    MacroQuery,
    /// The complete argument of a `{{tine-query …}}` macro: TQL.
    MacroTql,
}

/// Whether a `{{query …}}` form is datalog rather than the OG DSL.
///
/// **The ONE discriminator** (§7.1): the existing `Macro.tsx` / `ExportModal.tsx`
/// regexes are deleted in P0-ts and every caller asks this instead, so the two
/// cannot disagree about which source variant a block holds. A `:find` or
/// `:where` token inside an OG string or a page ref is text, not datalog — which
/// is exactly the case the TypeScript regexes got wrong — so the scan protects
/// both. There is **no speculative parse-and-fallback**: the token decides.
fn advanced_form(form: &str) -> bool {
    let bytes = form.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                // An OG double-quoted string, backslash-escaped.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'[' if form[i..].starts_with("[[") => {
                i = match form[i + 2..].find("]]") {
                    Some(offset) => i + 2 + offset + 2,
                    None => form.len(),
                };
            }
            b':' => {
                let rest = &form[i..];
                if rest.starts_with(":find") || rest.starts_with(":where") {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// The ONE text → IR entry for every input shape (§7.1, C3).
///
/// The macro inputs split their argument once, here, with the one
/// [`macro_text::split_trailing_map`]; the source variant records the grammar,
/// `source.original` holds the exact form slice and `source.og_options` the
/// opaque map or the empty string. The §4.1 precedence merge of the host
/// block's `tine.*` properties happens above this, in the command.
pub fn parse_query_input(
    text: &str,
    input: QueryInput,
    today: JournalDate,
    registry: &registry::Registry,
) -> (Query, ViewSettings) {
    match input {
        QueryInput::Og => parse_query_source(text, today),
        QueryInput::Advanced => advanced_source_query(text, String::new()),
        QueryInput::Tql => parse_query_text_with_registry(text, QueryDialect::Tql, today, registry),
        QueryInput::MacroTql => {
            let (form, og_options) =
                macro_text::split_trailing_map(text, macro_text::FormFamily::Tql);
            if !query_source_within_limit(&form) {
                return refuse_tql_source(&form, og_options);
            }
            tql::parse_tql_with_options(&form, og_options, registry)
        }
        QueryInput::MacroQuery => {
            let (form, og_options) =
                macro_text::split_trailing_map(text, macro_text::FormFamily::Edn);
            // A whole advanced map is the FORM, never options: the splitter
            // already refused to split a map with nothing before it (§4.4).
            if advanced_form(&form) {
                return advanced_source_query(&form, og_options);
            }
            let (mut query, view) = parse_query_source(&form, today);
            if let Source::Og {
                og_options: slot, ..
            } = &mut query.source
            {
                *slot = og_options;
            }
            (query, view)
        }
    }
}

/// A datalog form, retained as [`Source::Advanced`] with its complete authored
/// text (§4.4, C2).
///
/// The SIMPLE engine still refuses to run it — that is unchanged, and §4.4's
/// shared `resolve_for_execution` boundary is Wave D. What changes here is that
/// the source survives as the advanced variant, so a title-only edit can print
/// it back through the source-preserving path instead of being told the OG
/// printer cannot express it. `original` is the whole form, `:query`/`:inputs`
/// map and all; only a map that FOLLOWS it is options.
fn advanced_source_query(form: &str, og_options: String) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let mut query = Query::new(
        Anchor::Block,
        Filter::False,
        Source::Advanced {
            original: form.to_string(),
            og_options,
        },
    );
    query.diagnostics.push(Diagnostic::new(
        DiagnosticKind::Syntax,
        "this is an advanced (datalog) query, not the simple DSL",
    ));
    (query, ViewSettings::default())
}

fn refuse_tql_source(form: &str, og_options: String) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let mut query = Query::new(
        Anchor::Block,
        Filter::False,
        Source::Tql {
            original: form.to_string(),
            og_options,
        },
    );
    query.diagnostics.push(Diagnostic::new(
        DiagnosticKind::Size,
        "the query source is too large",
    ));
    (query, ViewSettings::default())
}

/// The OG `{{query}}` half of [`parse_query_text`].
pub(crate) fn parse_query_source(query_src: &str, today: JournalDate) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let refuse = |kind, message: &str| {
        let mut query = Query::new(
            Anchor::Block,
            Filter::False,
            Source::Og {
                original: query_src.to_string(),
                og_options: String::new(),
            },
        );
        query.diagnostics.push(Diagnostic::new(kind, message));
        (query, ViewSettings::default())
    };
    if !query_source_within_limit(query_src) {
        return refuse(DiagnosticKind::Size, "the query source is too large");
    }
    if !query_nesting_within_limit(query_src) {
        return refuse(DiagnosticKind::Depth, "the query nests too deeply");
    }
    if is_advanced(query_src) {
        return refuse(
            DiagnosticKind::Syntax,
            "this is an advanced (datalog) query, not the simple DSL",
        );
    }
    og::parse_og(query_src, today)
    // NOTE: the advanced refusal above is the SIMPLE-query engine's answer and
    // is unchanged. `query_parse`'s advanced inspection (§4.4) is Wave D's
    // `resolve_for_execution` boundary; `advanced_form` above is only the
    // §7.1 discriminator, and this wave routes both to the OG parser exactly as
    // Wave B did, so no behaviour depends on it yet.
}

// ---------------------------------------------------------------------------
// SPEC §4.4 (R5): execution-time binding
// ---------------------------------------------------------------------------

/// The provisional diagnostic `query_parse(advanced)` attaches to an advanced
/// form it has only INSPECTED (§4.4).
///
/// It is not a syntax verdict — the form may be perfectly well formed — it says
/// "the simple engine cannot answer this as it stands". §4.4 calls this a
/// *provisional inspection diagnostic* and requires the bound lowering's own
/// diagnostics to REPLACE it at execution time, which
/// [`resolve_for_execution`] does by matching this exact message. Every other
/// parse diagnostic (an I-22 size or depth refusal) is STATIC and survives.
pub(crate) const ADVANCED_UNRESOLVED_MESSAGE: &str =
    "this is an advanced (datalog) query, not the simple DSL";

/// The message a resolution that could not bind the query reports (§4.4). The
/// strict no-results behaviour is unchanged: a partially recognized tree is
/// never run.
pub(crate) const ADVANCED_UNSUPPORTED_MESSAGE: &str =
    "this advanced query's clauses are not supported, so it returns no results";

/// A query BOUND to one execution (SPEC §4.4, R5).
///
/// **The type is the guarantee.** Every evaluator, every explain-empty
/// decomposition and every result cache below takes a `ResolvedQuery`, and the
/// only way to obtain one is [`resolve_for_execution`], which consumes an
/// unresolved [`Query`]. A resolved query therefore cannot be resolved again —
/// not by convention, but because there is no function that accepts one and
/// returns another.
///
/// It carries its own `today`, the ONE execution-day snapshot: taken once here
/// rather than by each evaluator, so a rollover cannot land between the
/// page-anchored and block-anchored halves of a single answer, nor between a
/// result and the explanation of why it was empty.
#[derive(Debug, Clone)]
pub struct ResolvedQuery {
    query: Query,
    report: ir::QueryReport,
    today: JournalDate,
}

impl ResolvedQuery {
    /// The bound IR — an advanced form's lowered filter, or the OG/TQL IR
    /// unchanged.
    pub fn query(&self) -> &Query {
        &self.query
    }

    /// The support report this binding produced (M5). OG and TQL report an
    /// empty `ignored` and `supported = true`.
    pub fn report(&self) -> &ir::QueryReport {
        &self.report
    }

    /// The one execution-day snapshot every leaf in this execution reads.
    pub fn today(&self) -> JournalDate {
        self.today
    }

    /// Whether this binding produced executable IR at all. A refused advanced
    /// resolution is `false`: it has diagnostics and a report, and no counts.
    pub fn is_executable(&self) -> bool {
        self.report.supported && !self.query.is_invalid()
    }
}

/// **The ONE execution-time binding boundary** (SPEC §4.4, R5).
///
/// It runs BEFORE the invalidity check, before normalization and cache lookup,
/// before SQL/walk dispatch, and before explain-empty decomposition — so that
/// every one of those sees the same bound tree, and none of them can be handed
/// an advanced placeholder to interpret on its own.
///
/// For [`Source::Advanced`] it calls the ONE existing lowerer,
/// [`advanced_pred`], with the AUTHORED source (`Source::Advanced.original`,
/// `:query`/`:inputs` and all), the caller's current page, and this execution's
/// day. The lowering's `ran`/`ignored`/`supported` report is carried through
/// verbatim, its diagnostics replace the provisional inspection one, and static
/// (size/depth) diagnostics survive. Missing required inputs or unsupported
/// clauses keep today's strict no-results behaviour: the filter is
/// [`Filter::False`] and nothing partial runs.
///
/// For every other source the IR is already the query; only the execution-day
/// snapshot is added, which is what makes an OG `(between -7d today)` and a TQL
/// `day > -7d` read the same clock as an advanced `?today`.
pub fn resolve_for_execution(
    query: &Query,
    context: &ir::ExecutionContext,
    today: JournalDate,
) -> ResolvedQuery {
    use ir::{Diagnostic, DiagnosticKind};

    let Source::Advanced { original, .. } = &query.source else {
        return ResolvedQuery {
            query: query.clone(),
            report: ir::QueryReport {
                ran: Vec::new(),
                ignored: Vec::new(),
                supported: true,
            },
            today,
        };
    };

    // Static diagnostics survive the binding; the provisional inspection one
    // does not (§4.4).
    let static_diagnostics: Vec<Diagnostic> = query
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            !(diagnostic.kind == DiagnosticKind::Syntax
                && diagnostic.message == ADVANCED_UNRESOLVED_MESSAGE)
        })
        .cloned()
        .collect();

    let (lowered, ran, ignored) = advanced_pred(original, context.current_page.as_deref(), today);
    let mut bound = Query {
        anchor: query.anchor,
        filter: lowered
            .as_ref()
            .map_or(Filter::False, |query| query.filter.clone()),
        diagnostics: static_diagnostics,
        // The immutable source stays available for printing (§4.4). It is never
        // re-read as a filter after this point.
        source: query.source.clone(),
    };
    let supported = lowered.is_some();
    if !supported {
        bound.diagnostics.push(Diagnostic::new(
            DiagnosticKind::Syntax,
            ADVANCED_UNSUPPORTED_MESSAGE,
        ));
    }
    ResolvedQuery {
        query: bound,
        report: ir::QueryReport {
            ran,
            ignored,
            supported,
        },
        today,
    }
}

/// The ONE simple-query entry: source limits, parse, options, evaluation.
fn run_query_bounded_over(
    source: &dyn QueryPageSource,
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let today = JournalDate::today();
    let (query, view) = parse_query_source(query_src, today);
    run_pred_bounded_over(source, &query, &view, today, max_rows, max_bytes)
}

/// One page as the shared query drivers see it, borrowed from whichever backend
/// produced it.
///
/// Deliberately borrowed rather than owned. Direct Files holds an
/// `Arc<Document>` per page and managed storage holds an `Arc<Vec<DocBlock>>`;
/// an owned page shape (an `ApplicationQueryPage`, say) would force Direct to
/// construct and clone a whole `DocBlock` forest per query per page just to
/// satisfy the shared signature. `roots` is a borrowed slice for exactly that
/// reason, and `recency` is a callback because Direct Files answers it with a
/// filesystem `stat` that must not run for a page the query never matched.
pub(crate) struct QueryPageView<'a> {
    pub(crate) name: &'a str,
    pub(crate) kind: PageKind,
    pub(crate) pre_block: Option<&'a str>,
    pub(crate) roots: &'a [DocBlock],
    /// The page's journal ordinal as its own backend already knows it: Direct
    /// Files from the filename it parsed at inventory time, managed storage
    /// from the page title.
    pub(crate) journal: Option<i64>,
    /// The page's on-disk format. The property atomizer parses a value with the
    /// page's own inline grammar (§6.2 E4), so an `Outline.ORG` page's values
    /// are read as Org by the walk exactly as by the projection.
    pub(crate) format: crate::query::atom::AtomFormat,
    pub(crate) recency: &'a dyn Fn() -> i64,
}

/// The ONE page-source abstraction the shared simple/advanced/export query
/// drivers evaluate over (I-12, D-4). A backend implements this; it does not
/// re-implement the driver.
///
/// `visit` returning [`std::ops::ControlFlow::Break`] stops the walk, which is
/// what lets export hydration stop as soon as every wanted root is found.
pub(crate) trait QueryPageSource {
    fn for_each_page(&self, visit: &mut dyn FnMut(QueryPageView<'_>) -> std::ops::ControlFlow<()>);

    /// Lend the backend's complete page set as ONE borrowed slice, for the one
    /// consumer that must retain block references ACROSS pages: export
    /// hydration resolves selected roots in query order, not page order, so it
    /// cannot emit while streaming. Streaming callers use
    /// [`QueryPageSource::for_each_page`] and allocate nothing extra.
    fn with_hydration_pages(&self, run: &mut dyn FnMut(&[ExportHydrationPage<'_>]));

    /// The graph config the property atomizer reads (§5.8 M21). Supplied by the
    /// backend so the walk never re-reads `config.edn` per query.
    fn parse_config(&self) -> crate::config::ParseConfig;

    /// ONE coherent registry snapshot for the whole query (§6.2): the walk's
    /// coercion, the TQL diagnostics and `query_registry` all read the same
    /// `Arc`, so a query sees one generation end to end.
    fn registry(&self) -> std::sync::Arc<crate::query::registry::Registry>;

    /// Which of the §8.1 counterfactual modes this source evaluates under.
    /// Every product source is Tine; only gate 1's wrapper says otherwise.
    fn compare_mode(&self) -> atom::CompareMode {
        atom::CompareMode::Both
    }

    /// Test-only instrumentation hook: one predicate evaluation over this
    /// source is about to begin. Only Direct Files' whole-graph source counts,
    /// because the counter exists to prove the candidate planner avoided a full
    /// graph walk.
    #[cfg(test)]
    fn note_predicate_evaluation(&self) {}
}

/// One Direct Files graph walked under one of the §8.1 counterfactual modes.
///
/// Gate 1 needs the SAME walk over the SAME pages with one decision switched
/// off, so this delegates everything except the mode. Nothing in the product
/// constructs it; [`run_query_bounded_in_mode`] is its only caller.
pub(crate) struct GraphQueryPagesInMode<'a>(
    pub(crate) GraphQueryPages<'a>,
    pub(crate) atom::CompareMode,
);

impl QueryPageSource for GraphQueryPagesInMode<'_> {
    fn for_each_page(&self, visit: &mut dyn FnMut(QueryPageView<'_>) -> std::ops::ControlFlow<()>) {
        self.0.for_each_page(visit);
    }
    fn with_hydration_pages(&self, run: &mut dyn FnMut(&[ExportHydrationPage<'_>])) {
        self.0.with_hydration_pages(run);
    }
    fn parse_config(&self) -> crate::config::ParseConfig {
        self.0.parse_config()
    }
    fn registry(&self) -> std::sync::Arc<crate::query::registry::Registry> {
        self.0.registry()
    }
    fn compare_mode(&self) -> atom::CompareMode {
        self.1
    }
}

/// [`run_query_bounded`] under one §8.1 mode. Gate 1's entry point: the walk is
/// identical, only the atomizer's split, the atom identity and the coercion
/// change, so a difference between two modes attributes itself.
pub fn run_query_bounded_in_mode(
    graph: &Graph,
    query_src: &str,
    mode: atom::CompareMode,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    run_query_bounded_over(
        &GraphQueryPagesInMode(GraphQueryPages(graph), mode),
        query_src,
        max_rows,
        max_bytes,
    )
}

/// Direct Files' page source: the cached `Arc<Document>` snapshot.
pub(crate) struct GraphQueryPages<'a>(pub(crate) &'a Graph);

impl QueryPageSource for GraphQueryPages<'_> {
    fn for_each_page(&self, visit: &mut dyn FnMut(QueryPageView<'_>) -> std::ops::ControlFlow<()>) {
        self.0.with_pages(|pages| {
            for (entry, doc) in pages {
                let recency = || page_recency_secs(entry);
                let flow = visit(QueryPageView {
                    name: &entry.name,
                    kind: entry.kind,
                    pre_block: doc.pre_block.as_deref(),
                    roots: &doc.roots,
                    journal: entry.date_key,
                    format: Format::from_path(std::path::Path::new(&entry.rel_path)).into(),
                    recency: &recency,
                });
                if flow.is_break() {
                    break;
                }
            }
        });
    }

    fn with_hydration_pages(&self, run: &mut dyn FnMut(&[ExportHydrationPage<'_>])) {
        self.0.with_pages(|pages| {
            let pages = pages
                .iter()
                .map(|(entry, doc)| ExportHydrationPage {
                    kind: entry.kind,
                    name: &entry.name,
                    roots: &doc.roots,
                })
                .collect::<Vec<_>>();
            run(&pages);
        });
    }

    fn parse_config(&self) -> crate::config::ParseConfig {
        self.0.config.parse_config()
    }

    fn registry(&self) -> std::sync::Arc<crate::query::registry::Registry> {
        self.0.property_registry()
    }

    #[cfg(test)]
    fn note_predicate_evaluation(&self) {
        FULL_GRAPH_QUERY_EVALUATIONS.with(|count| count.set(count.get().saturating_add(1)));
    }
}

/// Managed storage's page source: the already-narrowed candidate set, each page
/// carrying the `DocBlock` forest the projection cache retained for it.
pub(crate) struct ApplicationQueryPages<'a> {
    pub(crate) pages: &'a [ApplicationQueryPage],
    pub(crate) config: crate::config::ParseConfig,
    pub(crate) registry: std::sync::Arc<crate::query::registry::Registry>,
}

impl QueryPageSource for ApplicationQueryPages<'_> {
    fn for_each_page(&self, visit: &mut dyn FnMut(QueryPageView<'_>) -> std::ops::ControlFlow<()>) {
        for source in self.pages {
            let page = &source.page;
            let recency = || source.recency;
            let flow = visit(QueryPageView {
                name: &page.name,
                kind: page.kind,
                pre_block: page.pre_block.as_deref(),
                roots: source.roots.as_slice(),
                journal: source.journal,
                format: page.format.into(),
                recency: &recency,
            });
            if flow.is_break() {
                break;
            }
        }
    }

    fn with_hydration_pages(&self, run: &mut dyn FnMut(&[ExportHydrationPage<'_>])) {
        let pages = self
            .pages
            .iter()
            .map(|source| ExportHydrationPage {
                kind: source.page.kind,
                name: source.page.name.as_str(),
                roots: source.roots.as_slice(),
            })
            .collect::<Vec<_>>();
        run(&pages);
    }

    fn parse_config(&self) -> crate::config::ParseConfig {
        self.config.clone()
    }

    fn registry(&self) -> std::sync::Arc<crate::query::registry::Registry> {
        std::sync::Arc::clone(&self.registry)
    }
}

fn run_pred_bounded(
    graph: &Graph,
    query: &Query,
    view: &ViewSettings,
    today: JournalDate,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    run_pred_bounded_over(
        &GraphQueryPages(graph),
        query,
        view,
        today,
        max_rows,
        max_bytes,
    )
}

// RETIREMENT-CANDIDATE: the in-memory query walk.
//
// WHAT MAY BE DELETED: `run_pred_bounded_over` together with the whole
// page-scanning evaluator it drives (`crate::query::eval`, `collect_og_query_roots`'s
// query use, the per-mode `QueryPageSource` implementations that exist only to
// feed it) and the `ApplicationProjectionCache` that keeps its managed input
// warm. The IR, the parsers, the printers and `finish_query_groups` stay: they
// are shared with the SQL route.
//
// CONDITION FOR DELETION: the private queue card `PVTI_lAHOAAbLVc4BhPsyzg5gS_0`
// ("Retire the query walk: bounded projection-readiness wait + pending render"),
// which SPEC §5.9 makes concrete — one shipped release of the SQL lowering with
// oracle gates §8.1–8.3 green and the field fallback counter observed only in
// the two windows §5.9 names, plus the bounded wait on the projection's
// `changed` condvar for `ready_at(gen)` and the "indexing…" pending render that
// replace today's silent walk fallback.
//
// WHAT CURRENTLY BLOCKS DELETION: the walk is still the ONLY answer in three
// live situations — a Direct Files projection that is not ready (open
// reconciliation, full rebuild, and the milliseconds after every save while the
// delta applies), a Direct Files projection whose read failed (D-3 recovery),
// and Managed Storage's unaccepted local overlay, whose pages have no
// materialized rows at all. It is also the differential oracle the SQL lowering
// is proven against in gate 1 (SPEC §8.1), so it cannot go before that lowering
// has shipped and been observed. Deleting it earlier would turn each of those
// windows into "no results" rather than "slower results".
//
/// The page-anchored half of the walk (§7.1, K16): `@page` rows as PAGE rows.
///
/// A `@page` query selects pages, so this reads the page index — name, kind,
/// journal day and the page's own `key:: value` preamble — and never descends
/// into a document. `@block` delegates to [`run_pred_bounded_over`], whose
/// block groups are the shipped shape.
///
/// **Post-resolution only** (§4.4): `query` is the BOUND tree
/// [`ResolvedQuery::query`] carries and `today` its one execution-day snapshot.
/// `run_resolved_query_result_over` is the entry that establishes both; this
/// function is also the probe evaluator explain-empty reuses per conjunct,
/// which is why it takes the pieces rather than the `ResolvedQuery` itself.
pub(crate) fn run_query_result_over(
    source: &dyn QueryPageSource,
    query: &Query,
    view: &ViewSettings,
    today: JournalDate,
    bounds: ir::Bounds,
) -> ir::QueryResult {
    let report = ir::QueryReport {
        ran: Vec::new(),
        ignored: Vec::new(),
        supported: true,
    };
    let mut result = ir::QueryResult {
        rows: ir::QueryRows::Page { pages: Vec::new() },
        diagnostics: query.diagnostics.clone(),
        report,
        total: 0,
        exceeded: false,
    };
    if query.anchor == Anchor::Block {
        let bounded = run_pred_bounded_over(
            source,
            query,
            view,
            today,
            bounds.max_rows,
            bounds.max_bytes,
        );
        result.rows = ir::QueryRows::Block {
            groups: bounded.groups,
        };
        result.total = bounded.total;
        result.exceeded = bounded.exceeded;
        return result;
    }
    if query.is_invalid() {
        return result;
    }
    let filter = query.evaluable_filter();
    let compiled = eval::CompiledLeaves::for_query(&filter);
    let parse_config = source.parse_config();
    let registry = source.registry();
    let mut pages: Vec<ir::PageRow> = Vec::new();
    let mut exceeded = false;
    source.for_each_page(&mut |page| {
        let (page_props, _tags) = page_facets(page.pre_block);
        if !eval::page_row_matches(
            &filter,
            page.name,
            page.kind,
            page.journal,
            &page_props,
            page.format,
            today,
            &compiled,
            &parse_config,
            &registry,
        ) {
            return std::ops::ControlFlow::Continue(());
        }
        if pages.len() >= bounds.max_rows {
            exceeded = true;
            return std::ops::ControlFlow::Break(());
        }
        pages.push(ir::PageRow {
            name: page.name.to_string(),
            kind: page.kind,
            journal_day: page.journal,
        });
        std::ops::ControlFlow::Continue(())
    });
    result.total = pages.len();
    result.exceeded = exceeded;
    result.rows = ir::QueryRows::Page { pages };
    result
}

/// The public page-or-block entry over a Direct Files graph. The dialect is the
/// caller's: it comes from the macro name the text was read out of (Q3).
pub fn run_query_result(
    graph: &Graph,
    query_src: &str,
    dialect: QueryDialect,
    bounds: ir::Bounds,
) -> ir::QueryResult {
    let today = JournalDate::today();
    let (query, view) = parse_query_text(query_src, dialect, today);
    run_query_result_over(&GraphQueryPages(graph), &query, &view, today, bounds)
}

/// The ONE post-resolution result driver (§4.4): evaluate the bound tree, then
/// attach the binding's support report.
///
/// The report is attached HERE, after the evaluation (and, for a caching
/// caller, after the result-cache retrieval), because it is a property of how
/// this source was bound and not of the rows — which is exactly why the rows may
/// be shared and the report may not.
pub(crate) fn run_resolved_query_result_over(
    source: &dyn QueryPageSource,
    resolved: &ResolvedQuery,
    view: &ViewSettings,
    bounds: ir::Bounds,
) -> ir::QueryResult {
    let mut result =
        run_query_result_over(source, resolved.query(), view, resolved.today(), bounds);
    result.report = resolved.report().clone();
    result
}

/// The ONE simple-query evaluator. Both storage modes reach it through
/// [`QueryPageSource`]; neither owns a second copy of the budget, the page loop,
/// the OG top-level-root filter, the sample cap or the recency axis.
fn run_pred_bounded_over(
    source: &dyn QueryPageSource,
    query: &Query,
    view: &ViewSettings,
    // §4.4: the ONE execution-day snapshot, taken by `resolve_for_execution`
    // (or by the one text entry above) and never re-read from the clock here —
    // a rollover between two halves of one answer is not a thing that can
    // happen.
    today: JournalDate,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    #[cfg(test)]
    source.note_predicate_evaluation();
    // An invalid query (an unknown head, a syntax refusal, a depth/size refusal)
    // returns zero results plus its diagnostics — never a truncated answer
    // (§3.5).
    if query.is_invalid() {
        return BoundedGroups {
            groups: Vec::new(),
            total: 0,
            exceeded: false,
        };
    }
    let opts = QueryOpts::from_view(view);
    let opts = &opts;
    // The legacy block-group adapter evaluates a `@page`-anchored filter
    // BLOCK-anchored — page attributes and relations read through `block.page` —
    // because that is today's semantics verbatim (`(page-property …)`,
    // `(page-tags …)` and `(namespace …)` have always returned blocks). The
    // page-anchored result rows live behind `run_query_result_over`.
    let filter = match query.anchor {
        Anchor::Block => query.evaluable_filter(),
        Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
    };
    let compiled = eval::CompiledLeaves::for_query(&filter);
    let parse_config = source.parse_config();
    let registry = source.registry();
    let mode = source.compare_mode();
    let mut budget = ConstructionBudget::new(max_rows, max_bytes);
    // An unsorted `(sample N)` semantically needs only the first N matches in
    // deterministic traversal order. Do not construct or classify the rest as
    // an over-budget failure. Sorted samples still require global ranking and
    // therefore retain the ordinary construction ceiling.
    let sample_admission_cap = opts.sample.filter(|_| opts.sort.is_none());
    // A recency sort (`(sort-by modified …)`) needs each result page's position on
    // a single time axis: journal pages by the day they represent, other pages by
    // file mtime. Only computed when such a sort is active (else we skip the stat).
    let want_recency = matches!(&opts.sort, Some((f, _)) if is_recency_field(f));
    let mut groups: Vec<RefGroup> = Vec::new();
    let mut recency_by_page: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    source.for_each_page(&mut |page| {
        let (page_props, _page_tags) = page_facets(page.pre_block);
        let ctx = EvalCtx {
            journal: page.journal,
            is_journal: page.kind == PageKind::Journal,
            page_name: page.name,
            page_props: &page_props,
            today,
            compiled: &compiled,
            format: page.format,
            config: &parse_config,
            registry: &registry,
            mode,
        };
        let mut matched: Vec<BlockDto> = Vec::new();
        let mut path = Vec::new();
        let mut path_refs = PathRefCounts::new();
        let track_path_refs = eval::uses_path_refs(&filter);
        collect_og_query_roots(
            page.roots,
            &mut path,
            &mut path_refs,
            track_path_refs,
            false,
            &mut |block, _, ancestor_refs| {
                eval::eval_block(&filter, block, ancestor_refs, &ctx).then_some(())
            },
            &mut |block, _, ()| {
                if sample_admission_cap.is_some_and(|cap| budget.rows >= cap) {
                    return None;
                }
                if budget.closed() {
                    budget.deny_match();
                    return None;
                }
                if !budget.admit_estimated(page.name, shallow_dto_estimated_bytes(block, &[])) {
                    return None;
                }
                Some(result_dto(block))
            },
            &mut matched,
        );
        if !matched.is_empty() {
            if want_recency {
                recency_by_page.insert(page.name.to_owned(), (page.recency)());
            }
            groups.push(RefGroup {
                page: page.name.to_owned(),
                kind: page.kind,
                blocks: matched,
                evidence: Vec::new(),
            });
        }
        std::ops::ControlFlow::Continue(())
    });

    finish_query_groups(groups, recency_by_page, opts, budget)
}

#[cfg(test)]
thread_local! {
    static FULL_GRAPH_QUERY_EVALUATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_full_graph_query_evaluations() {
    FULL_GRAPH_QUERY_EVALUATIONS.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn full_graph_query_evaluations() -> u64 {
    FULL_GRAPH_QUERY_EVALUATIONS.with(std::cell::Cell::get)
}

fn finish_query_groups(
    mut groups: Vec<RefGroup>,
    recency_by_page: std::collections::HashMap<String, i64>,
    opts: &QueryOpts,
    budget: ConstructionBudget,
) -> BoundedGroups {
    // The source traversal is path-stable in both Direct Files and the managed
    // application gateway. Make the displayed base order stable before sampling
    // and before it becomes the tie-breaker for an explicit sort.
    groups.sort_by(|a, b| {
        a.page.cmp(&b.page).then_with(|| {
            let rank = |kind| match kind {
                PageKind::Journal => 0,
                PageKind::Page => 1,
            };
            rank(a.kind).cmp(&rank(b.kind))
        })
    });

    // sort-by is GLOBAL (like Logseq): order every matched block across all pages on
    // one axis, so e.g. priority-A tasks float to the very top regardless of which
    // page they live on. We flatten to one block per group, sort, then RE-COALESCE
    // runs of adjacent same-page blocks back under a single page heading — N
    // consecutive results from one page show ONCE, not N times (a page whose blocks
    // land at different sort positions, e.g. an A and a C task under a priority sort,
    // still appears at each of those positions). Non-sorted queries keep their
    // natural page grouping untouched.
    if let Some((field, asc)) = &opts.sort {
        // Decorate each block with its sort key (computed ONCE — an lsdoc parse per
        // result block, not per comparison) and its original index. The index is a
        // stable tiebreaker so equal-key blocks keep DOCUMENT order in both
        // directions: a plain `reverse()` for `desc` would flip a page's blocks
        // upside-down under its heading.
        let mut flat: Vec<(SortDecor, usize, RefGroup)> = Vec::new();
        for g in groups {
            let RefGroup {
                page,
                kind,
                blocks,
                evidence: _,
            } = g;
            for b in blocks {
                let key = if is_recency_field(field) {
                    // Recency is numeric (Unix seconds on one axis): journal pages by
                    // the day they represent, others by file mtime.
                    SortDecor::Num(recency_by_page.get(&page).copied().unwrap_or(i64::MIN))
                } else {
                    SortDecor::Text(sort_key(&b, &page, field))
                };
                let idx = flat.len();
                flat.push((
                    key,
                    idx,
                    RefGroup {
                        page: page.clone(),
                        kind,
                        blocks: vec![b],
                        evidence: Vec::new(),
                    },
                ));
            }
        }
        flat.sort_by(|a, b| {
            let ord = a.0.cmp(&b.0);
            (if *asc { ord } else { ord.reverse() }).then(a.1.cmp(&b.1))
        });
        // Merge adjacent one-block groups that share a page (and kind) into a single
        // group, so consecutive same-page results render under one heading.
        let mut merged: Vec<RefGroup> = Vec::with_capacity(flat.len());
        for (_, _, g) in flat {
            match merged.last_mut() {
                Some(last) if last.page == g.page && last.kind == g.kind => {
                    last.blocks.extend(g.blocks)
                }
                _ => merged.push(g),
            }
        }
        groups = merged;
    }

    // sample N: cap total results (deterministic: first N across pages).
    if let Some(n) = opts.sample {
        let mut remaining = n;
        groups.retain_mut(|g| {
            if remaining == 0 {
                return false;
            }
            if g.blocks.len() > remaining {
                g.blocks.truncate(remaining);
            }
            remaining -= g.blocks.len();
            true
        });
    }
    BoundedGroups {
        groups,
        total: budget.total,
        exceeded: budget.exceeded,
    }
}

/// One exact parser-owned page selected by the managed query candidate plan.
/// `recency` shares Direct Files' axis: journal midnight or projected-file mtime.
pub(crate) struct ApplicationQueryPage {
    pub(crate) page: PageDto,
    /// The journal day this page evaluates as, or `None` for an ordinary page.
    ///
    /// Supplied by the caller from the graph's configured `JournalFormat` -- the
    /// same producer that fills `PageEntry::date_key` on the Direct side -- so
    /// the two backends answer journal-day predicates identically.
    pub(crate) journal: Option<i64>,
    /// The page's block tree already converted for evaluation. Supplied by the
    /// caller from [`ApplicationProjectionCache`] so an unchanged page keeps its
    /// memoized lsdoc projections across queries, the way Direct Files keeps
    /// them in its cached `Arc<Document>`.
    pub(crate) roots: std::sync::Arc<Vec<DocBlock>>,
    pub(crate) recency: i64,
}

// RETIREMENT-CANDIDATE: the pre-SQL candidate planner for the walk.
//
// WHAT MAY BE DELETED: `SimpleQueryCandidateSource`, `SimpleQueryCandidatePlan`,
// `simple_query_candidate_plan`, `SparseTaskQueryEligibility` and
// `sparse_task_query_eligibility`, together with the two consumers that exist
// only to feed them — `direct_projection`'s `simple_query_candidate_paths` and
// managed storage's candidate-page selection. Nothing else reads them: they are
// a page PRE-FILTER for the walk, never an answer.
//
// CONDITION FOR DELETION: the same card as the walk itself,
// `PVTI_lAHOAAbLVc4BhPsyzg5gS_0`, one step earlier. A planner that narrows which
// pages the walk visits has no purpose once the SQL lowering (SPEC §5) answers
// the query directly from indices — the lowering's own plan supersedes it, and
// keeping both means two independent opinions about which rows can match, which
// is exactly the second interpretation I-12 forbids. Concretely: when
// `run_query_result` reaches SQL for a query shape, that shape's candidate plan
// is dead code.
//
// WHAT CURRENTLY BLOCKS DELETION: the walk is still the answer (see the marker
// on `run_pred_bounded_over`), and on a large graph the walk without this
// planner reads every page of the graph for every keystroke in a query block.
// The planner is what keeps `(task TODO)` and `[[Page]]` — the two shapes almost
// every real query uses — off the full-graph path. Deleting it before the
// lowering ships would not remove a code path; it would make the shipped
// product visibly slower on exactly the queries people write.
//
/// One reconstructible, page-complete candidate source for a managed simple
/// query. These facts only choose pages; the exact current parser DTO remains
/// authoritative for block membership and result shape.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum SimpleQueryCandidateSource {
    Task(String),
    PageRef(String),
    BlockProperty(String),
    PageProperty(String),
    Page(String),
    Namespace(String),
    Journal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SimpleQueryCandidatePlan {
    Empty,
    Indexed(Vec<SimpleQueryCandidateSource>),
    All,
}

impl SimpleQueryCandidatePlan {
    /// B4 admits only the first ranked grammar class on Direct Files. Boolean
    /// composition remains eligible when the planner proved PageRef is the
    /// complete narrowing source; mixed-source unions stay on the full walk.
    pub(crate) fn is_page_ref_only(&self) -> bool {
        matches!(
            self,
            Self::Indexed(sources)
                if !sources.is_empty()
                    && sources
                        .iter()
                        .all(|source| matches!(source, SimpleQueryCandidateSource::PageRef(_)))
        )
    }
}

/// Exact marker streams a managed sparse task-query reader may enumerate.
///
/// This is deliberately narrower than [`SimpleQueryCandidatePlan`]: the latter
/// only needs a complete page source, while this plan promises that every
/// selected row is a parser-owned task candidate.  The sparse runner still
/// parses and evaluates the complete query against each returned raw block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SparseTaskQueryEligibility {
    pub(crate) markers: Vec<String>,
    pub(crate) uses_recency: bool,
}

/// The existing simple parser intentionally accepts a recoverable prefix for
/// Direct Files.  A sparse reader cannot safely decide that an incomplete
/// source has a complete marker stream, so it additionally requires one full,
/// balanced expression.  This is a syntax guard over the shared tokenizer and
/// parser, not a second query dialect.
fn sparse_query_source_is_complete(query_src: &str) -> bool {
    if !query_source_within_limit(query_src) || !query_nesting_within_limit(query_src) {
        return false;
    }
    let mut in_string = false;
    let mut escaped = false;
    for ch in query_src.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        }
    }
    if in_string {
        return false;
    }

    let tokens = og::tokens_only(query_src);
    let mut depth = 0usize;
    for token in &tokens {
        match token {
            og::Tok::LParen => depth = depth.saturating_add(1),
            og::Tok::RParen => match depth.checked_sub(1) {
                Some(next) => depth = next,
                None => return false,
            },
            _ => {}
        }
    }
    if depth != 0 {
        return false;
    }
    // One balanced expression that consumes the whole source, and a parse that
    // raised no diagnostic: the sparse path may not narrow a query whose filter
    // the shared parser only half understood.
    og::is_single_expression(&tokens)
        && !parse_query_source(query_src, JournalDate::today())
            .0
            .is_invalid()
}

/// The shared parser deliberately supplies recoverable defaults for malformed
/// presentation directives so Direct Files can keep evaluating the rest of a
/// query.  Sparse selection cannot turn those defaults into an authority to
/// enumerate a narrowed candidate stream.  Validate only the source shapes
/// whose parsed forms the sparse path accepts; the parser itself remains the
/// sole evaluator and Direct Files keeps its existing recovery behavior.
fn sparse_task_directive_shapes_are_strict(query_src: &str) -> bool {
    use og::Tok;
    fn name(token: &Tok) -> Option<&str> {
        match token {
            Tok::Word(value) | Tok::Str(value) | Tok::PageRef(value) => Some(value),
            Tok::LParen | Tok::RParen => None,
        }
    }

    fn nonempty_name(token: &Tok) -> bool {
        name(token).is_some_and(|value| !value.trim().is_empty())
    }

    fn flat_args<'a>(tokens: &'a [Tok], position: &mut usize) -> Option<&'a [Tok]> {
        let start = *position;
        while !matches!(tokens.get(*position), Some(Tok::RParen)) {
            if matches!(tokens.get(*position), Some(Tok::LParen) | None) {
                return None;
            }
            *position += 1;
        }
        let args = &tokens[start..*position];
        *position += 1; // closing parenthesis
        Some(args)
    }

    fn between(args: &[Tok], today: JournalDate) -> bool {
        let [Tok::Word(field), lo, hi] = args else {
            return false;
        };
        matches!(
            field.to_ascii_lowercase().as_str(),
            "scheduled" | "deadline"
        ) && name(lo)
            .and_then(|value| resolve_date_token(value, today))
            .is_some()
            && name(hi)
                .and_then(|value| resolve_date_token(value, today))
                .is_some()
    }

    fn sample(args: &[Tok]) -> bool {
        matches!(args, [argument] if name(argument).is_some_and(|value| value.parse::<usize>().is_ok()))
    }

    fn sort_by(args: &[Tok]) -> bool {
        match args {
            [field] => nonempty_name(field),
            [field, Tok::Word(direction) | Tok::Str(direction)] => {
                nonempty_name(field)
                    && matches!(direction.to_ascii_lowercase().as_str(), "asc" | "desc")
            }
            _ => false,
        }
    }

    fn aggregate(args: &[Tok]) -> bool {
        match args {
            [kind] if name(kind).is_some_and(|value| value.eq_ignore_ascii_case("count")) => true,
            [kind, field]
                if name(kind).is_some_and(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "sum" | "avg" | "average"
                    )
                }) =>
            {
                nonempty_name(field)
            }
            _ => false,
        }
    }

    fn group_by(args: &[Tok]) -> bool {
        // `page` is the built-in grouping key; any nonempty name is an exact
        // property key, matching the existing `(group-by page|<prop>)` grammar.
        matches!(args, [field] if nonempty_name(field))
    }

    fn expression(tokens: &[Tok], position: &mut usize, today: JournalDate) -> bool {
        match tokens.get(*position) {
            Some(Tok::LParen) => {
                *position += 1;
                let Some(Tok::Word(head)) = tokens.get(*position) else {
                    return false;
                };
                *position += 1;
                match head.to_ascii_lowercase().as_str() {
                    "between" => {
                        flat_args(tokens, position).is_some_and(|args| between(args, today))
                    }
                    "sample" => flat_args(tokens, position).is_some_and(sample),
                    "sort-by" => flat_args(tokens, position).is_some_and(sort_by),
                    "aggregate" => flat_args(tokens, position).is_some_and(aggregate),
                    "group-by" => flat_args(tokens, position).is_some_and(group_by),
                    _ => {
                        while !matches!(tokens.get(*position), Some(Tok::RParen)) {
                            if !expression(tokens, position, today) {
                                return false;
                            }
                        }
                        *position += 1;
                        true
                    }
                }
            }
            Some(Tok::RParen) | None => false,
            Some(_) => {
                *position += 1;
                true
            }
        }
    }

    let tokens = og::tokens_only(query_src);
    let mut position = 0;
    expression(&tokens, &mut position, JournalDate::today()) && position == tokens.len()
}

/// Conservative eligibility/extraction for the block-level managed task path.
///
/// Keep this beside the broader page candidate planner so marker
/// canonicalization and malformed-query handling stay shared.  The accepted
/// filter grammar is intentionally small: positive task leaves combined by
/// `and`, optionally one priority leaf and scheduled/deadline presence or
/// range leaves.  Presentation directives remain neutral filters and are
/// handed to the existing finalizer below.
pub(crate) fn sparse_task_query_eligibility(query_src: &str) -> Option<SparseTaskQueryEligibility> {
    if !sparse_query_source_is_complete(query_src)
        || !sparse_task_directive_shapes_are_strict(query_src)
    {
        return None;
    }
    // Reuse the established candidate-plan parser and its marker
    // canonicalization.  In particular, do not grow another token parser here.
    let SimpleQueryCandidatePlan::Indexed(sources) = simple_query_candidate_plan(query_src) else {
        return None;
    };
    let planned_markers = sources
        .iter()
        .filter_map(|source| match source {
            SimpleQueryCandidateSource::Task(marker) => Some(marker.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if planned_markers.is_empty()
        || sources
            .iter()
            .any(|source| !matches!(source, SimpleQueryCandidateSource::Task(_)))
    {
        return None;
    }

    let (query, view) = parse_query_source(query_src, JournalDate::today());
    if query.is_invalid() {
        return None;
    }
    let mut task_marker_sets = Vec::<BTreeSet<String>>::new();
    let mut saw_priority = false;

    fn accepted_shape(
        filter: &Filter,
        task_marker_sets: &mut Vec<BTreeSet<String>>,
        saw_priority: &mut bool,
    ) -> bool {
        match filter {
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Task,
                        op: CmpOp::In,
                        value: Value::List { items },
                    },
            } => {
                let markers = items
                    .iter()
                    .filter_map(|item| match item {
                        Value::Text { text } => Some(text.to_ascii_uppercase()),
                        _ => None,
                    })
                    .collect::<BTreeSet<_>>();
                if markers.is_empty() || markers.len() != items.len() {
                    return false;
                }
                task_marker_sets.push(markers);
                true
            }
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Priority,
                        ..
                    },
            } => {
                if *saw_priority {
                    return false;
                }
                *saw_priority = true;
                true
            }
            // Planning presence and planning ranges stay enumerable by the
            // marker index; a journal range does not (it selects pages).
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Scheduled | Attr::Deadline,
                        ..
                    },
            } => true,
            // The four lifted view directives are no longer filter nodes at all;
            // they contribute `True`, which is neutral for the marker index.
            Filter::True => true,
            Filter::And { items } if !items.is_empty() => items
                .iter()
                .all(|child| accepted_shape(child, task_marker_sets, saw_priority)),
            // OR, NOT, page/ref/property/tag/journal/content/search/regex and
            // any future predicate are not safely enumerable by a marker index.
            _ => false,
        }
    }

    if !accepted_shape(
        &query.evaluable_filter(),
        &mut task_marker_sets,
        &mut saw_priority,
    ) {
        return None;
    }
    let markers = task_marker_sets
        .into_iter()
        .reduce(|intersection, markers| {
            intersection
                .intersection(&markers)
                .cloned()
                .collect::<BTreeSet<_>>()
        })?;
    // Multiple positive task leaves with no common marker are a contradictory
    // shape.  Refuse the sparse path instead of making marker enumeration a
    // second interpretation of the query.
    if markers.is_empty() || !markers.is_subset(&planned_markers) {
        return None;
    }
    let opts = QueryOpts::from_view(&view);
    Some(SparseTaskQueryEligibility {
        markers: markers.into_iter().collect(),
        uses_recency: matches!(&opts.sort, Some((field, _)) if is_recency_field(field)),
    })
}

/// Conservative page-level candidate plan for indexed managed simple-query
/// families. A returned union is complete: every matching block must live on a
/// page selected by at least one source. AND may choose one complete child; OR
/// may union only when every branch is complete. Valid shapes that cannot be
/// narrowed use the explicit all-page plan; invalid shapes need no page reads.
pub(crate) fn simple_query_candidate_plan(query_src: &str) -> SimpleQueryCandidatePlan {
    type Sources = std::collections::BTreeSet<SimpleQueryCandidateSource>;
    fn one(source: SimpleQueryCandidateSource) -> Option<Sources> {
        Some(std::iter::once(source).collect())
    }
    /// The page-row leaves, reached through a `page` hop on a block row or
    /// directly at the `@page` anchor.
    fn page_sources(filter: &Filter) -> Option<Sources> {
        match filter {
            Filter::Leaf {
                leaf: Leaf::Attr { attr, op, value },
            } => match (attr, op, value) {
                (Attr::Name, CmpOp::Eq, Value::Text { text }) => {
                    one(SimpleQueryCandidateSource::Page(refs::page_key(text)))
                }
                (Attr::Name, CmpOp::StartsWith, Value::Text { text }) => {
                    one(SimpleQueryCandidateSource::Namespace(refs::page_key(
                        text.trim_end_matches('/'),
                    )))
                }
                (Attr::Journal, CmpOp::Eq, Value::Bool { value: true }) => {
                    one(SimpleQueryCandidateSource::Journal)
                }
                (Attr::Day, _, _) => one(SimpleQueryCandidateSource::Journal),
                _ => None,
            },
            Filter::Leaf {
                leaf:
                    Leaf::Rel {
                        rel: Rel::Props,
                        quant: Quant::Any,
                        pred,
                    },
            } => pred.props_key().and_then(|key| {
                one(SimpleQueryCandidateSource::PageProperty(property_key_norm(
                    &key,
                )))
            }),
            Filter::And { items } => items.iter().find_map(page_sources),
            Filter::Or { items } => {
                let mut union = Sources::new();
                for item in items {
                    union.extend(page_sources(item)?);
                }
                Some(union)
            }
            _ => None,
        }
    }
    fn sources(filter: &Filter) -> Option<Sources> {
        match filter {
            Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Task,
                        op: CmpOp::In,
                        value: Value::List { items },
                    },
            } => items
                .iter()
                .map(|item| match item {
                    Value::Text { text } => {
                        Some(SimpleQueryCandidateSource::Task(text.to_ascii_uppercase()))
                    }
                    _ => None,
                })
                .collect::<Option<Sources>>(),
            Filter::Leaf {
                leaf:
                    Leaf::Rel {
                        rel: Rel::Refs,
                        quant: Quant::Any,
                        pred,
                    },
            } => pred
                .ref_name()
                .and_then(|name| one(SimpleQueryCandidateSource::PageRef(refs::page_key(&name)))),
            Filter::Leaf {
                leaf:
                    Leaf::Rel {
                        rel: Rel::Props,
                        quant: Quant::Any,
                        pred,
                    },
            } => pred.props_key().and_then(|key| {
                one(SimpleQueryCandidateSource::BlockProperty(
                    property_key_norm(&key),
                ))
            }),
            Filter::Leaf {
                leaf:
                    Leaf::Rel {
                        rel: Rel::Page,
                        quant: Quant::Any,
                        pred,
                    },
            } => page_sources(pred),
            Filter::And { items } => items.iter().find_map(sources),
            Filter::Or { items } => {
                let mut union = Sources::new();
                for item in items {
                    union.extend(sources(item)?);
                }
                Some(union)
            }
            _ => None,
        }
    }

    let (query, _view) = parse_query_source(query_src, JournalDate::today());
    if query.is_invalid() {
        return SimpleQueryCandidatePlan::Empty;
    }
    let filter = match query.anchor {
        Anchor::Block => query.evaluable_filter(),
        Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
    };
    match sources(&filter) {
        Some(sources) => SimpleQueryCandidatePlan::Indexed(sources.into_iter().collect()),
        None => SimpleQueryCandidatePlan::All,
    }
}

pub(crate) fn application_query_doc_block(block: &BlockDto, is_org: bool) -> DocBlock {
    let mut doc = dto_block_to_doc_block(block, is_org);
    doc.children = block
        .children
        .iter()
        .map(|child| application_query_doc_block(child, is_org))
        .collect();
    doc
}

/// Default bounds for [`ApplicationProjectionCache`].
///
/// The byte bound counts SOURCE raw text, not retained memory: a retained tree
/// is roughly three to four times its raw text once every block's projection is
/// filled (`visible` + `visible_lower` + reference vectors + per-block
/// overhead), so 16 MiB of source is the order of 60 MiB retained at the very
/// worst -- and only when a query actually projected every block of every
/// cached page. Martin's real graph is 4.5 MiB across 1,045 files, so both
/// bounds hold it whole; a graph larger than that degrades to LRU misses
/// rather than to unbounded growth.
pub(crate) const APPLICATION_PROJECTION_CACHE_MAX_PAGES: usize = 4_096;
pub(crate) const APPLICATION_PROJECTION_CACHE_MAX_RAW_BYTES: usize = 16 * 1024 * 1024;

struct ApplicationProjectionCacheEntry {
    is_org: bool,
    raw_bytes: usize,
    used: u64,
    roots: std::sync::Arc<Vec<DocBlock>>,
}

/// Converted managed page block trees, retained across managed query
/// evaluations so an unchanged page is parsed once instead of once per query.
///
/// Why this exists at all: Direct Files gets projection memoization for free.
/// `Graph::with_pages` hands out a cached `Arc<Document>` whose `DocBlock`s each
/// memoize ONE lsdoc parse in a `OnceLock` ([`DocBlock::projection`]), so after
/// the first query every Direct block projection is warm. The managed evaluator
/// rebuilds a `PageDto` per request and used to call
/// [`application_query_doc_block`] on it, allocating a fresh `OnceLock::new()`
/// per block -- so managed re-parsed every block of every candidate page on
/// EVERY query, on `{{query}}` re-render and on every search keystroke.
///
/// **Staleness is impossible by construction, and that is deliberate.** The
/// cache is content-addressed by exact comparison rather than by a digest or a
/// generation counter: a retained tree is reused only after
/// [`doc_roots_match_dtos`] proves it structurally equal to the incoming DTO
/// tree (raw text, block identity, child shape) at the same `is_org`, and a
/// `BlockProjection` is a pure function of `(raw, is_org)`. There is therefore
/// no generation window to get wrong, no digest collision to defend against,
/// and no invalidation hook that a future write path can forget to call: a
/// changed page simply fails the comparison and is rebuilt. The comparison is a
/// length-guarded `memcmp` over the same bytes a parse would have read, i.e.
/// cheaper than the parse it replaces by orders of magnitude.
///
/// Bounded by page count AND source bytes, evicting least-recently-used, so a
/// graph larger than the bound degrades to the previous per-query rebuild for
/// the evicted pages instead of growing without limit.
pub(crate) struct ApplicationProjectionCache {
    entries: HashMap<String, ApplicationProjectionCacheEntry>,
    max_pages: usize,
    max_raw_bytes: usize,
    raw_bytes: usize,
    clock: u64,
    #[cfg(test)]
    hits: usize,
    #[cfg(test)]
    misses: usize,
}

impl Default for ApplicationProjectionCache {
    fn default() -> Self {
        Self::new(
            APPLICATION_PROJECTION_CACHE_MAX_PAGES,
            APPLICATION_PROJECTION_CACHE_MAX_RAW_BYTES,
        )
    }
}

impl std::fmt::Debug for ApplicationProjectionCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationProjectionCache")
            .field("pages", &self.entries.len())
            .field("raw_bytes", &self.raw_bytes)
            .finish()
    }
}

impl ApplicationProjectionCache {
    pub(crate) fn new(max_pages: usize, max_raw_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_pages,
            max_raw_bytes,
            raw_bytes: 0,
            clock: 0,
            #[cfg(test)]
            hits: 0,
            #[cfg(test)]
            misses: 0,
        }
    }

    /// The converted block tree for one exact managed page.
    ///
    /// `path` only selects which retained tree to COMPARE against; it never
    /// substitutes for the comparison, so a path reused for different content
    /// (rename, replacement, external edit) misses rather than lies.
    pub(crate) fn roots(&mut self, path: &str, page: &PageDto) -> std::sync::Arc<Vec<DocBlock>> {
        let is_org = page.format == Format::Org;
        self.clock = self.clock.saturating_add(1);
        let clock = self.clock;
        if let Some(entry) = self.entries.get_mut(path) {
            if entry.is_org == is_org && doc_roots_match_dtos(&entry.roots, &page.blocks) {
                entry.used = clock;
                #[cfg(test)]
                {
                    self.hits = self.hits.saturating_add(1);
                }
                return std::sync::Arc::clone(&entry.roots);
            }
        }
        #[cfg(test)]
        {
            self.misses = self.misses.saturating_add(1);
        }
        let roots = std::sync::Arc::new(
            page.blocks
                .iter()
                .map(|block| application_query_doc_block(block, is_org))
                .collect::<Vec<_>>(),
        );
        let raw_bytes = dto_raw_bytes(&page.blocks);
        if raw_bytes > self.max_raw_bytes || self.max_pages == 0 {
            // One page bigger than the whole budget must not evict the rest of
            // the graph to store an entry that the next insert would drop.
            self.forget(path);
            return roots;
        }
        if let Some(previous) = self.entries.insert(
            path.to_owned(),
            ApplicationProjectionCacheEntry {
                is_org,
                raw_bytes,
                used: clock,
                roots: std::sync::Arc::clone(&roots),
            },
        ) {
            self.raw_bytes = self.raw_bytes.saturating_sub(previous.raw_bytes);
        }
        self.raw_bytes = self.raw_bytes.saturating_add(raw_bytes);
        self.evict();
        roots
    }

    fn forget(&mut self, path: &str) {
        if let Some(previous) = self.entries.remove(path) {
            self.raw_bytes = self.raw_bytes.saturating_sub(previous.raw_bytes);
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > self.max_pages || self.raw_bytes > self.max_raw_bytes {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(path, entry)| (entry.used, (*path).clone()))
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            self.forget(&victim);
        }
    }

    #[cfg(test)]
    pub(crate) fn counters(&self) -> (usize, usize, usize) {
        (self.hits, self.misses, self.entries.len())
    }

    #[cfg(test)]
    pub(crate) fn reset_counters(&mut self) {
        self.hits = 0;
        self.misses = 0;
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.raw_bytes = 0;
    }
}

fn dto_raw_bytes(blocks: &[BlockDto]) -> usize {
    blocks
        .iter()
        .map(|block| {
            block
                .raw
                .len()
                .saturating_add(block.id.len())
                .saturating_add(dto_raw_bytes(&block.children))
        })
        .sum()
}

/// Exact structural equality between a retained `DocBlock` tree and the DTO
/// tree it was converted from. Only the fields [`application_query_doc_block`]
/// reads participate, because only those can make the retained tree wrong:
/// `raw` (which the memoized projection is a pure function of) and `id` (which
/// becomes `DocBlock::uuid` and reaches the result DTO as its identity).
fn doc_roots_match_dtos(cached: &[DocBlock], blocks: &[BlockDto]) -> bool {
    cached.len() == blocks.len()
        && cached.iter().zip(blocks).all(|(cached, block)| {
            cached.raw == block.raw
                && cached.uuid == block.id
                && doc_roots_match_dtos(&cached.children, &block.children)
        })
}

/// Evaluate one already-narrowed exact managed page set with the same predicate,
/// OG top-level-root filter, result budgets, sorting and sampling as Direct Files.
pub(crate) fn run_application_query_pages_bounded(
    pages: &[ApplicationQueryPage],
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
    config: crate::config::ParseConfig,
    registry: std::sync::Arc<registry::Registry>,
) -> BoundedGroups {
    run_query_bounded_over(
        &ApplicationQueryPages {
            pages,
            config,
            registry,
        },
        query_src,
        max_rows,
        max_bytes,
    )
}

/// The §7.1 `query_run` evaluator over managed pages: the IR arrives already
/// parsed, so this is `run_query_result_over` with the managed page source
/// bound. One evaluator, two backends (I-19) — the Direct Files twin is
/// [`run_query_result`].
pub(crate) fn run_application_query_result(
    pages: &[ApplicationQueryPage],
    query: &Query,
    view: &ViewSettings,
    bounds: ir::Bounds,
    config: crate::config::ParseConfig,
    registry: std::sync::Arc<registry::Registry>,
    context: &ir::ExecutionContext,
) -> ir::QueryResult {
    let resolved = resolve_for_execution(query, context, JournalDate::today());
    run_resolved_query_result_over(
        &ApplicationQueryPages {
            pages,
            config,
            registry,
        },
        &resolved,
        view,
        bounds,
    )
}

/// The §7.1 `query_explain_empty` computation over managed pages.
pub(crate) fn explain_application_empty_query(
    pages: &[ApplicationQueryPage],
    query: &Query,
    view: &ViewSettings,
    bounds: ir::Bounds,
    config: crate::config::ParseConfig,
    registry: std::sync::Arc<registry::Registry>,
    context: &ir::ExecutionContext,
) -> ir::ExplainEmptyResult {
    let resolved = resolve_for_execution(query, context, JournalDate::today());
    view::explain_empty(
        &ApplicationQueryPages {
            pages,
            config,
            registry,
        },
        &resolved,
        view,
        bounds,
    )
}

/// The Direct Files twin of [`run_application_query_result`]: `query_run` when
/// the IR is already parsed (the §7.1 command hands the IR, not text).
///
/// §4.4: the IR arriving already parsed is exactly why this resolves. A parse
/// is context-free, so the `{query, view}` a caller holds may have been parsed
/// on another page, on another day, or by another window; the binding happens
/// here, per execution.
pub fn run_query_result_ir(
    graph: &Graph,
    query: &Query,
    view: &ViewSettings,
    bounds: ir::Bounds,
    context: &ir::ExecutionContext,
) -> ir::QueryResult {
    let resolved = resolve_for_execution(query, context, JournalDate::today());
    run_resolved_query_result_over(&GraphQueryPages(graph), &resolved, view, bounds)
}

/// The Direct Files twin of [`explain_application_empty_query`].
pub fn explain_empty_query(
    graph: &Graph,
    query: &Query,
    view: &ViewSettings,
    bounds: ir::Bounds,
    context: &ir::ExecutionContext,
) -> ir::ExplainEmptyResult {
    let resolved = resolve_for_execution(query, context, JournalDate::today());
    view::explain_empty(&GraphQueryPages(graph), &resolved, view, bounds)
}

/// Storage-independent shallow block supplied by the managed sparse reader.
///
/// `identity` is the caller's already-authoritative result identity (external
/// UUID or its managed internal fallback); it is never inferred from raw text.
/// `dfs_order` is the complete root-to-leaf structural key supplied by the
/// caller.  Equal keys retain the caller's input order.
#[derive(Clone, Debug)]
pub(crate) struct ApplicationSparseQueryCandidate {
    pub(crate) raw: String,
    pub(crate) identity: String,
    pub(crate) page: ApplicationSparseQueryPage,
    pub(crate) parent_identity: Option<String>,
    pub(crate) dfs_order: Vec<String>,
}

/// Parser mode and page facts needed to evaluate one sparse candidate without
/// constructing a page DTO or hydrating its outline.
#[derive(Clone, Debug)]
pub(crate) struct ApplicationSparseQueryPage {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) kind: PageKind,
    pub(crate) is_org: bool,
    pub(crate) recency: i64,
}

/// A sparse runner failure means the caller must run the established complete
/// evaluator; it must never combine a partial sparse result with fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationSparseQueryError {
    Ineligible,
    MissingIdentity,
    DuplicateIdentity,
}

/// One exact parser-owned block selected by a disposable physical index.
///
/// Direct Files uses this view to avoid reparsing SQL candidates: SQLite only
/// chooses structural coordinates, while the current `Arc<Document>` supplies
/// the same memoized `DocBlock` used by the established whole-page evaluator.
pub(crate) struct ParserSparseQueryCandidate<'a> {
    pub(crate) block: &'a DocBlock,
    pub(crate) identity: &'a str,
    pub(crate) page: &'a ApplicationSparseQueryPage,
    pub(crate) parent_identity: Option<&'a str>,
    pub(crate) dfs_order: &'a [String],
}

fn application_sparse_query_doc_block(candidate: &ApplicationSparseQueryCandidate) -> DocBlock {
    let mut block = DocBlock::new(candidate.raw.clone());
    block.uuid.clone_from(&candidate.identity);
    block.is_org = candidate.page.is_org;
    block
}

/// Run a marker-narrowed sparse task candidate set with the ordinary simple
/// query parser, evaluator, DTO constructor, result budget and finalizer.
///
/// The managed reader must provide every marker candidate and a complete DFS
/// key/parent relation.  We evaluate all candidates before admitting any DTO so
/// immediate-parent suppression is based on the full matched-ID set, even when
/// a result budget or sample later truncates the display.
pub(crate) fn run_application_sparse_task_query_bounded(
    candidates: &[ApplicationSparseQueryCandidate],
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
    config: &crate::config::ParseConfig,
    registry: &registry::Registry,
) -> Result<BoundedGroups, ApplicationSparseQueryError> {
    let blocks = candidates
        .iter()
        .map(application_sparse_query_doc_block)
        .collect::<Vec<_>>();
    let views = candidates
        .iter()
        .zip(&blocks)
        .map(|(candidate, block)| ParserSparseQueryCandidate {
            block,
            identity: &candidate.identity,
            page: &candidate.page,
            parent_identity: candidate.parent_identity.as_deref(),
            dfs_order: &candidate.dfs_order,
        })
        .collect::<Vec<_>>();
    run_parser_sparse_task_query_bounded(&views, query_src, max_rows, max_bytes, config, registry)
}

pub(crate) fn run_parser_sparse_task_query_bounded(
    candidates: &[ParserSparseQueryCandidate<'_>],
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
    config: &crate::config::ParseConfig,
    registry: &registry::Registry,
) -> Result<BoundedGroups, ApplicationSparseQueryError> {
    if sparse_task_query_eligibility(query_src).is_none() {
        return Err(ApplicationSparseQueryError::Ineligible);
    }
    let today = JournalDate::today();
    let (query, view) = parse_query_source(query_src, today);
    if query.is_invalid() {
        return Err(ApplicationSparseQueryError::Ineligible);
    }
    let filter = match query.anchor {
        Anchor::Block => query.evaluable_filter(),
        Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
    };
    let compiled = eval::CompiledLeaves::for_query(&filter);
    let opts = QueryOpts::from_view(&view);

    let mut ordered = candidates.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.dfs_order.cmp(right.dfs_order));

    let mut identities = HashSet::with_capacity(ordered.len());
    for candidate in &ordered {
        if candidate.identity.is_empty() {
            return Err(ApplicationSparseQueryError::MissingIdentity);
        }
        if !identities.insert(candidate.identity) {
            return Err(ApplicationSparseQueryError::DuplicateIdentity);
        }
    }

    struct EvaluatedCandidate<'a> {
        candidate: &'a ParserSparseQueryCandidate<'a>,
        matched: bool,
    }

    let mut evaluated = Vec::with_capacity(ordered.len());
    for candidate in ordered {
        let empty_props = Vec::new();
        let ctx = EvalCtx {
            journal: (candidate.page.kind == PageKind::Journal)
                .then(|| journal_ordinal(&candidate.page.name))
                .flatten(),
            is_journal: candidate.page.kind == PageKind::Journal,
            page_name: &candidate.page.name,
            page_props: &empty_props,
            today,
            compiled: &compiled,
            mode: atom::CompareMode::Both,
            // A sparse task candidate is a marker-narrowed block; the eligibility
            // gate (`sparse_task_query_eligibility`) admits no property leaf, so
            // no atom is ever parsed on this path. Markdown is the honest default.
            format: crate::query::atom::AtomFormat::Markdown,
            config,
            registry,
        };
        let matched = eval::eval_block(&filter, candidate.block, &PathRefCounts::new(), &ctx);
        evaluated.push(EvaluatedCandidate { candidate, matched });
    }
    let matched_ids = evaluated
        .iter()
        .filter(|candidate| candidate.matched)
        .map(|candidate| candidate.candidate.identity)
        .collect::<HashSet<_>>();

    struct SparseGroup {
        page: String,
        kind: PageKind,
        recency: i64,
        blocks: Vec<BlockDto>,
    }

    let mut budget = ConstructionBudget::new(max_rows, max_bytes);
    let sample_admission_cap = opts.sample.filter(|_| opts.sort.is_none());
    let want_recency = matches!(&opts.sort, Some((field, _)) if is_recency_field(field));
    let mut groups = Vec::<SparseGroup>::new();
    let mut group_indexes = HashMap::<(String, String, PageKind), usize>::new();
    for candidate in &evaluated {
        if !candidate.matched
            || candidate
                .candidate
                .parent_identity
                .is_some_and(|parent| matched_ids.contains(parent))
        {
            continue;
        }
        if sample_admission_cap.is_some_and(|cap| budget.rows >= cap) {
            continue;
        }
        if budget.closed() {
            budget.deny_match();
            continue;
        }
        if !budget.admit_estimated(
            &candidate.candidate.page.name,
            shallow_dto_estimated_bytes(candidate.candidate.block, &[]),
        ) {
            continue;
        }

        let page = &candidate.candidate.page;
        let key = (page.name.clone(), page.path.clone(), page.kind);
        let index = match group_indexes.get(&key) {
            Some(index) => *index,
            None => {
                let index = groups.len();
                group_indexes.insert(key, index);
                groups.push(SparseGroup {
                    page: page.name.clone(),
                    kind: page.kind,
                    recency: page.recency,
                    blocks: Vec::new(),
                });
                index
            }
        };
        groups[index]
            .blocks
            .push(result_dto(candidate.candidate.block));
    }

    let mut recency_by_page = HashMap::new();
    let groups = groups
        .into_iter()
        .map(|group| {
            if want_recency {
                recency_by_page.insert(group.page.clone(), group.recency);
            }
            RefGroup {
                page: group.page,
                kind: group.kind,
                blocks: group.blocks,
                evidence: Vec::new(),
            }
        })
        .collect();
    Ok(finish_query_groups(groups, recency_by_page, &opts, budget))
}

// --- Scoped-invalidation support (#52) --------------------------------------
// "Could an edit to page (entry, doc) change this derived result?" Each reuses
// the SAME parse + EvalCtx + eval (or alias resolution) as the real matcher, so
// the keep/evict decision can never drift from what a full recompute would give.

/// Whether page (entry, doc) contributes any block to query `src`.
pub(crate) fn page_affects_query(
    src: &str,
    entry: &PageEntry,
    doc: &Document,
    config: &crate::config::ParseConfig,
    registry: &registry::Registry,
) -> bool {
    let today = JournalDate::today();
    let (query, _view) = parse_query_source(src, today);
    if query.is_invalid() {
        return false;
    }
    let filter = match query.anchor {
        Anchor::Block => query.evaluable_filter(),
        Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
    };
    page_contributes_to_filter(&filter, entry, doc, today, config, registry)
}

/// Whether a query source carries a `props` leaf, and is therefore sensitive to
/// the registry's effective types (C6): its cached result must be evicted when
/// the registry generation advances, because per-page retention evaluates the
/// query against ONE saved page and cannot see a graph-wide type change.
pub fn query_source_has_props_leaf(src: &str) -> bool {
    let (query, _view) = parse_query_source(src, JournalDate::today());
    query.filter.has_props_leaf()
}

/// "Could an edit to this page change the derived result of `filter`?" — the
/// SAME parse and the SAME evaluator the real matcher uses, so a keep/evict
/// decision can never drift from what a full recompute would give.
fn page_contributes_to_filter(
    filter: &Filter,
    entry: &PageEntry,
    doc: &Document,
    today: JournalDate,
    config: &crate::config::ParseConfig,
    registry: &registry::Registry,
) -> bool {
    let (page_props, _page_tags) = page_facets(doc.pre_block.as_deref());
    let compiled = eval::CompiledLeaves::for_query(filter);
    let ctx = EvalCtx {
        journal: entry.date_key,
        is_journal: entry.kind == PageKind::Journal,
        page_name: &entry.name,
        page_props: &page_props,
        today,
        compiled: &compiled,
        mode: atom::CompareMode::Both,
        format: Format::from_path(std::path::Path::new(&entry.rel_path)).into(),
        config,
        registry,
    };
    let mut hit = false;
    let mut path_refs = PathRefCounts::new();
    walk_path_refs(
        &doc.roots,
        &mut path_refs,
        eval::uses_path_refs(filter),
        &mut |block, ancestor_refs| {
            if !hit && eval::eval_block(filter, block, ancestor_refs, &ctx) {
                hit = true;
            }
        },
    );
    hit
}

/// Whether page `doc` references `target` or any of its aliases — i.e. could be
/// in `backlinks(target)`. Mirrors `backlinks`'s alias resolution; takes the
/// resolved alias map so the caller needn't hold the graph lock.
pub(crate) fn page_affects_backlinks(
    real_pages: &RealPageNames,
    aliases: &[(String, String)],
    target: &str,
    entry: &PageEntry,
    doc: &Document,
) -> bool {
    let (canonical, names_norm, _) = equivalent_page_names(real_pages, aliases, target);
    // Scoped invalidation has no Graph/config parameter. Default-enabled matching
    // is conservative for disabled/excluded property pages (it may evict an
    // unaffected cache entry, but cannot retain a stale one).
    let config = crate::config::Config::default();
    if doc.pre_block.as_deref().is_some_and(|pre| {
        page_property_block(entry, pre).is_some_and(|block| {
            block_reference_evidence(
                &block,
                &canonical,
                &names_norm,
                ReferenceKind::Explicit,
                &config,
            )
            .is_some()
        })
    }) {
        return true;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && block_reference_evidence(
                b,
                &canonical,
                &names_norm,
                ReferenceKind::Explicit,
                &config,
            )
            .is_some()
        {
            hit = true;
        }
    });
    hit
}

/// Whether page `doc` plain-text-mentions `target` unlinked — i.e. could be in
/// `unlinked_refs(target)`. Mirrors `unlinked_refs`'s matcher.
pub(crate) fn page_affects_unlinked(
    real_pages: &RealPageNames,
    aliases: &[(String, String)],
    target: &str,
    entry: &PageEntry,
    doc: &Document,
) -> bool {
    let (canonical, names_norm, _) = equivalent_page_names(real_pages, aliases, target);
    let config = crate::config::Config::default();
    if doc.pre_block.as_deref().is_some_and(|pre| {
        page_property_block(entry, pre).is_some_and(|block| {
            block_reference_evidence(
                &block,
                &canonical,
                &names_norm,
                ReferenceKind::Plain,
                &config,
            )
            .is_some()
        })
    }) {
        return true;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && block_reference_evidence(b, &canonical, &names_norm, ReferenceKind::Plain, &config)
                .is_some()
        {
            hit = true;
        }
    });
    hit
}

/// Whether this page contains a referrer to one block UUID. This is the exact
/// predicate used by `block_referrers_bounded`, without DTO construction.
pub(crate) fn page_affects_block_referrers(uuid: &str, doc: &Document) -> bool {
    let uuid = uuid.trim();
    if uuid.is_empty() {
        return false;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |block| {
        if !hit
            && block
                .projection()
                .block_refs
                .iter()
                .any(|reference| reference == uuid)
        {
            hit = true;
        }
    });
    hit
}

/// Whether an edited page can contribute to the supported advanced-query
/// subset. Parsing and evaluation are shared with the real advanced query, so
/// scoped cache invalidation cannot drift into a second query dialect.
pub(crate) fn page_affects_advanced_query(
    query_src: &str,
    current_page: Option<&str>,
    entry: &PageEntry,
    doc: &Document,
    config: &crate::config::ParseConfig,
    registry: &registry::Registry,
) -> bool {
    let today = JournalDate::today();
    let (Some(query), _, _) = advanced_pred(query_src, current_page, today) else {
        return false;
    };
    page_contributes_to_filter(
        &query.evaluable_filter(),
        entry,
        doc,
        today,
        config,
        registry,
    )
}

/// Result of an advanced (datalog) query: matched groups + which clause heads
/// ran vs were ignored, so the UI shows "ran X; ignored Y" rather than a blunt
/// "unsupported". `supported` is false only when nothing in the subset matched.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdvancedResult {
    pub groups: Vec<RefGroup>,
    pub ran: Vec<String>,
    pub ignored: Vec<String>,
    pub supported: bool,
}

pub(crate) fn rejected_advanced_query(reason: &str) -> AdvancedResult {
    AdvancedResult {
        groups: Vec::new(),
        ran: Vec::new(),
        ignored: vec![reason.to_string()],
        supported: false,
    }
}

/// Run an advanced `[:find … :where …]` / `{:query … :inputs …}` query by mapping
/// the common clause subset (task / between / page-ref / property / page-property
/// / priority + and/or/not) onto the ONE IR `Filter` the OG DSL and TQL also
/// lower to — the matching leaves already exist. Unrecognized clauses (custom
/// rules, `[?e ?a ?v]` joins, `:view`/`:result-transform`) are listed in
/// `ignored` and skipped, never guessed (a wrong result is worse than
/// "unsupported").
pub fn run_advanced_query(
    graph: &Graph,
    query_src: &str,
    current_page: Option<&str>,
) -> AdvancedResult {
    run_advanced_query_bounded(graph, query_src, current_page, usize::MAX, usize::MAX).0
}

pub fn run_advanced_query_bounded(
    graph: &Graph,
    query_src: &str,
    current_page: Option<&str>,
    max_rows: usize,
    max_bytes: usize,
) -> (AdvancedResult, bool, usize) {
    run_advanced_query_bounded_over(
        &GraphQueryPages(graph),
        query_src,
        current_page,
        max_rows,
        max_bytes,
    )
}

pub(crate) fn run_application_advanced_query_pages_bounded(
    pages: &[ApplicationQueryPage],
    query_src: &str,
    current_page: Option<&str>,
    max_rows: usize,
    max_bytes: usize,
    config: crate::config::ParseConfig,
    registry: std::sync::Arc<registry::Registry>,
) -> (AdvancedResult, bool, usize) {
    run_advanced_query_bounded_over(
        &ApplicationQueryPages {
            pages,
            config,
            registry,
        },
        query_src,
        current_page,
        max_rows,
        max_bytes,
    )
}

/// The ONE advanced-query evaluator: source limits, clause lowering, the
/// `ran`/`ignored` report and delegation to the shared simple-query driver all
/// live here, so the two storage modes cannot answer an advanced query
/// differently (I-12, I-19).
fn run_advanced_query_bounded_over(
    source: &dyn QueryPageSource,
    query_src: &str,
    current_page: Option<&str>,
    max_rows: usize,
    max_bytes: usize,
) -> (AdvancedResult, bool, usize) {
    if !query_source_within_limit(query_src) {
        return (rejected_advanced_query("query-too-large"), false, 0);
    }
    if !query_nesting_within_limit(query_src) {
        return (rejected_advanced_query("query-nesting-too-deep"), false, 0);
    }
    // §4.4: the SAME `resolve_for_execution` boundary the §7.1 commands use.
    // This path used to call `advanced_pred` itself, which is how one lowerer
    // ended up with two callers that could disagree about what a missing input
    // means.
    let today = JournalDate::today();
    let (parsed, _) = advanced_source_query(query_src, String::new());
    let resolved = resolve_for_execution(
        &parsed,
        &ir::ExecutionContext {
            current_page: current_page.map(str::to_string),
        },
        today,
    );
    let ran = resolved.report().ran.clone();
    let ignored = resolved.report().ignored.clone();
    if !resolved.report().supported {
        return (
            AdvancedResult {
                groups: Vec::new(),
                ran,
                ignored,
                supported: false,
            },
            false,
            0,
        );
    }
    let bounded = run_pred_bounded_over(
        source,
        resolved.query(),
        &ViewSettings::default(),
        resolved.today(),
        max_rows,
        max_bytes,
    );
    (
        AdvancedResult {
            groups: bounded.groups,
            ran,
            ignored,
            supported: true,
        },
        bounded.exceeded,
        bounded.total,
    )
}

fn advanced_pred(
    query_src: &str,
    current_page: Option<&str>,
    today: JournalDate,
) -> (Option<Query>, Vec<String>, Vec<String>) {
    // Both limits live here, not only at the two `run_advanced_*` entry points,
    // because `page_affects_advanced_query` reaches this function directly. It
    // used to skip the byte ceiling entirely, which made scoped invalidation the
    // one caller that could hand an unbounded graph-authored string to the
    // parser — the exact thing QUERY_SOURCE_MAX_BYTES exists to prevent.
    if !query_source_within_limit(query_src) {
        return (None, Vec::new(), vec!["query-too-large".to_string()]);
    }
    if !query_nesting_within_limit(query_src) {
        return (None, Vec::new(), vec!["query-nesting-too-deep".to_string()]);
    }
    let inputs = resolve_inputs(query_src, current_page, today);
    let mut ran = Vec::new();
    let mut ignored = Vec::new();
    let groups = where_groups(query_src);
    let (lowered_page_properties, consumed_patterns) = lower_page_property_patterns(&groups);
    let (lowered_current_pages, current_page_patterns) =
        lower_current_page_patterns(&groups, &inputs);
    let consumed_patterns = consumed_patterns
        .into_iter()
        .chain(current_page_patterns)
        .collect::<std::collections::HashSet<_>>();
    let preds: Vec<Filter> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            if let Some((pred, label)) = lowered_current_pages.get(&index) {
                ran.push((*label).into());
                return Some(pred.clone());
            }
            if let Some(pred) = lowered_page_properties.get(&index) {
                ran.push("page-property".into());
                return Some(pred.clone());
            }
            if consumed_patterns.contains(&index) {
                return None;
            }
            parse_adv_group(group, &inputs, today, &mut ran, &mut ignored, 0)
        })
        .collect();
    if ignored.iter().any(|item| item == "query-nesting-too-deep") {
        return (None, Vec::new(), ignored);
    }
    if preds.is_empty() {
        return (None, ran, ignored);
    }
    let filter = if preds.len() == 1 {
        preds.into_iter().next().expect("one")
    } else {
        Filter::and(preds)
    };
    // The advanced context and report survive verbatim (M5): `current_page` is
    // already folded into the lowered clauses above, and the caller keeps
    // `ran`/`ignored`/`supported`.
    let query = Query::new(
        Anchor::Block,
        filter,
        Source::Advanced {
            original: query_src.to_string(),
            og_options: String::new(),
        },
    );
    (Some(query), ran, ignored)
}

/// Lower the exact DataScript relationship Logseq uses to connect the typed
/// `:current-page` input to blocks. This is deliberately not a general join
/// engine: one page-name identity pattern must feed one `:block/refs` or
/// `:block/page` pattern, and every other shape remains visibly unsupported.
fn lower_current_page_patterns(
    groups: &[String],
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let triples = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let inner = group.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
            let tokens = inner.split_whitespace().collect::<Vec<_>>();
            (tokens.len() == 3).then_some((index, tokens))
        })
        .collect::<Vec<_>>();

    let mut candidates = Vec::new();
    for (identity_index, identity) in &triples {
        if identity[1] != ":block/name" || !identity[0].starts_with('?') {
            continue;
        }
        let Some(AdvancedInput::Page(page)) = inputs.get(identity[2]) else {
            continue;
        };
        for (relation_index, relation) in &triples {
            if relation[0] == identity[0]
                || !relation[0].starts_with('?')
                || relation[2] != identity[0]
            {
                continue;
            }
            let lowered = match relation[1] {
                ":block/refs" => Some((Filter::page_ref(page.clone()), "current-page-ref")),
                ":block/page" => Some((
                    Filter::rel(
                        Rel::Page,
                        Quant::Any,
                        Filter::attr(Attr::Name, CmpOp::Eq, Value::text(page.clone())),
                    ),
                    "current-page",
                )),
                _ => None,
            };
            if let Some(lowered) = lowered {
                candidates.push((*identity_index, *relation_index, lowered));
            }
        }
    }
    if candidates.len() != 1 {
        return Default::default();
    }
    let (identity_index, relation_index, lowered) = candidates.pop().unwrap();
    (
        std::collections::HashMap::from([(relation_index, lowered)]),
        std::collections::HashSet::from([identity_index]),
    )
}

/// Conservatively lower only the exact DataScript relationship used by the
/// released BEGIN_QUERY page-property form. The entity/property-map pattern and
/// `(get ...)` predicate must share the literal `?props` binding; every other
/// bracket form remains visible as an unsupported `pattern` in `parse_adv_group`.
fn lower_page_property_patterns(
    groups: &[String],
) -> (
    std::collections::HashMap<usize, Filter>,
    std::collections::HashSet<usize>,
) {
    let relations = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let inner = group.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
            (inner.split_whitespace().collect::<Vec<_>>() == ["?p", ":block/properties", "?props"])
                .then_some(index)
        })
        .collect::<Vec<_>>();
    if relations.len() != 1 {
        return Default::default();
    }

    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    for (index, group) in groups.iter().enumerate() {
        let Some(inner) = group
            .trim()
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
        else {
            continue;
        };
        let Some(call) = inner
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
        else {
            continue;
        };
        let tokens = call.split_whitespace().collect::<Vec<_>>();
        if tokens.len() != 3 || tokens[0] != "get" || tokens[1] != "?props" {
            continue;
        }
        let Some(key) = tokens[2].strip_prefix(':').filter(|key| !key.is_empty()) else {
            continue;
        };
        if key
            .chars()
            .any(|c| c.is_whitespace() || "()[]{}".contains(c))
        {
            continue;
        }
        lowered.insert(
            index,
            Filter::rel(
                Rel::Page,
                Quant::Any,
                og::property_leaf(og::normalize_prop_key(key), None),
            ),
        );
        consumed.insert(relations[0]);
    }
    (lowered, consumed)
}

/// Collect balanced `(...)`/`[...]` groups at the top level of `s` (string-aware),
/// stopping at the first top-level *closing* bracket (so scanning after `:where`
/// halts at the find-vector's `]` rather than swallowing `:inputs`).
fn scan_groups(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c == ')' || c == ']' || c == '}' {
            break;
        }
        // EDN/DataScript line comment (`; …` to end of line) — skip it so example
        // clauses written inside a `;;` hint are NOT parsed as real groups.
        if c == ';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == '(' || c == '[' {
            let start = i;
            let mut depth = 0;
            let mut in_str = false;
            while i < b.len() {
                let ch = b[i] as char;
                if in_str {
                    if ch == '\\' {
                        i += 2;
                        continue;
                    }
                    if ch == '"' {
                        in_str = false;
                    }
                } else if ch == ';' {
                    // Comment inside a group body (between clauses) — skip to EOL.
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                } else if ch == '"' {
                    in_str = true;
                } else if ch == '(' || ch == '[' || ch == '{' {
                    depth += 1;
                } else if ch == ')' || ch == ']' || ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            out.push(s[start..i.min(s.len())].to_string());
            continue;
        }
        i += 1;
    }
    out
}

/// The clause groups in the `:where` section.
fn where_groups(src: &str) -> Vec<String> {
    match src.find(":where") {
        Some(idx) => scan_groups(&src[idx + ":where".len()..]),
        None => Vec::new(),
    }
}

/// Map one `:where` group to a `Pred` (or None → ignored). Recurses for and/or/not.
fn parse_adv_group(
    group: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: JournalDate,
    ran: &mut Vec<String>,
    ignored: &mut Vec<String>,
    depth: usize,
) -> Option<Filter> {
    if depth > QUERY_NESTING_MAX {
        ignored.push("query-nesting-too-deep".into());
        return None;
    }
    let c = group.trim();
    if !c.starts_with('(') {
        ignored.push("pattern".into()); // `[?e :a ?v]` joins, etc. — not in the subset
        return None;
    }
    let inner = &c[1..c.len().saturating_sub(1)];
    let head = inner
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match head.as_str() {
        "and" | "or" | "not" => {
            let kids: Vec<Filter> = scan_groups(inner)
                .iter()
                .filter_map(|g| parse_adv_group(g, inputs, today, ran, ignored, depth + 1))
                .collect();
            if kids.is_empty() {
                None
            } else if head == "not" {
                Some(Filter::not(kids.into_iter().next().expect("one")))
            } else if head == "or" {
                Some(Filter::or(kids))
            } else {
                Some(Filter::and(kids))
            }
        }
        "task" | "todo" => {
            ran.push("task".into());
            Some(Filter::attr(
                Attr::Task,
                CmpOp::In,
                adv_text_list(adv_strings(inner)),
            ))
        }
        "priority" => {
            ran.push("priority".into());
            Some(Filter::attr(
                Attr::Priority,
                CmpOp::In,
                adv_text_list(adv_strings(inner)),
            ))
        }
        "page-ref" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page-ref".into());
            Filter::page_ref(n)
        }),
        "property" | "page-property" => inner
            .split_whitespace()
            .skip(1)
            .find(|t| t.starts_with(':'))
            .map(|t| t.trim_start_matches(':').to_string())
            .map(|k| {
                let val = adv_strings(inner).into_iter().next();
                ran.push(head.clone());
                let leaf = og::property_leaf(og::normalize_prop_key(&k), val);
                if head == "property" {
                    leaf
                } else {
                    Filter::rel(Rel::Page, Quant::Any, leaf)
                }
            }),
        "page" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::Eq, Value::text(n)),
            )
        }),
        "namespace" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("namespace".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text(format!("{n}/"))),
            )
        }),
        "page-tags" | "tags" => {
            let ts = adv_strings(inner);
            if ts.is_empty() {
                ignored.push(head.clone());
                None
            } else {
                ran.push("page-tags".into());
                Some(Filter::rel(
                    Rel::Page,
                    Quant::Any,
                    Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![
                            Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                            Filter::attr(Attr::Value, CmpOp::In, adv_text_list(ts)),
                        ]),
                    ),
                ))
            }
        }
        "scheduled" => {
            ran.push("scheduled".into());
            Some(Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None))
        }
        "deadline" => {
            ran.push("deadline".into());
            Some(Filter::attr(Attr::Deadline, CmpOp::IsSet, Value::None))
        }
        "journal" => {
            ran.push("journal".into());
            Some(Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true }),
            ))
        }
        "between" => {
            // (between [FIELD] ?b ?start ?end): the last two args are always the
            // bounds. An optional field keyword (journal|scheduled|deadline) may
            // appear among the earlier args — matching the simple parser. The bare
            // `(between ?b lo hi)` keeps OG's journal-day semantics.
            let args: Vec<&str> = inner.split_whitespace().skip(1).collect();
            if args.len() < 2 {
                ignored.push("between".into());
                return None;
            }
            let attr = args
                .iter()
                .take(args.len() - 2)
                .find_map(
                    |a| match a.trim_start_matches(':').to_ascii_lowercase().as_str() {
                        "scheduled" => Some(Attr::Scheduled),
                        "deadline" => Some(Attr::Deadline),
                        "journal" => Some(Attr::Day),
                        _ => None,
                    },
                )
                .unwrap_or(Attr::Day);
            let lo = adv_bound(args[args.len() - 2], inputs, today);
            let hi = adv_bound(args[args.len() - 1], inputs, today);
            if lo.is_none() && hi.is_none() {
                ignored.push("between".into());
                return None;
            }
            ran.push("between".into());
            // The advanced dialect resolves its bounds eagerly: `:inputs` may
            // bind a bound to an already-resolved ordinal, and an advanced query
            // is never re-printed as OG DSL, so the IR carries the ordinals.
            let range = adv_range(attr, lo, hi);
            Some(if attr == Attr::Day {
                Filter::rel(Rel::Page, Quant::Any, range)
            } else {
                range
            })
        }
        other => {
            if !other.is_empty() {
                ignored.push(other.to_string());
            }
            None
        }
    }
}

/// All double-quoted string literals in a clause (markers, page names, values).
fn adv_strings(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            out.push(s[start..i.min(s.len())].to_string());
        }
        i += 1;
    }
    out
}

/// Resolve a `between` bound: an input `?var` (looked up) or a literal token.
fn adv_bound(
    tok: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: JournalDate,
) -> Option<i64> {
    let t = tok.trim();
    if t.starts_with('?') {
        return match inputs.get(t) {
            Some(AdvancedInput::Date(value)) => Some(*value),
            _ => None,
        };
    }
    // A literal bound may be written as a bare token (`2026-06-24`) or a quoted
    // string (`"2026-06-24"`); `split_whitespace` keeps the quotes, so strip them.
    resolve_date_token(t.trim_matches('"').trim_start_matches(':'), today)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AdvancedInput {
    Date(i64),
    Page(String),
}

/// Build a typed positional input map by zipping `:in $ ?a ?b …` with
/// `:inputs [ … ]`. Dates stay numeric; Logseq's typed `:current-page` keyword
/// receives the caller's focused page. Unknown keywords remain unbound.
fn resolve_inputs(
    src: &str,
    current_page: Option<&str>,
    today: JournalDate,
) -> std::collections::HashMap<String, AdvancedInput> {
    let mut map = std::collections::HashMap::new();
    let vars: Vec<String> = match src.find(":in") {
        Some(i) => {
            let rest = &src[i + 3..];
            let end = rest
                .find(":where")
                .or_else(|| rest.find(']'))
                .unwrap_or(rest.len());
            rest[..end]
                .split_whitespace()
                .filter(|t| t.starts_with('?'))
                .map(String::from)
                .collect()
        }
        None => Vec::new(),
    };
    let vals: Vec<String> = match src.find(":inputs") {
        Some(i) => {
            let rest = &src[i + ":inputs".len()..];
            match (rest.find('['), rest.find(']')) {
                (Some(a), Some(b)) if b > a => rest[a + 1..b]
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
                _ => Vec::new(),
            }
        }
        None => Vec::new(),
    };
    for (v, val) in vars.iter().zip(vals.iter()) {
        if val.eq_ignore_ascii_case(":current-page") {
            if let Some(page) = current_page.map(str::trim).filter(|page| !page.is_empty()) {
                map.insert(v.clone(), AdvancedInput::Page(page.to_lowercase()));
            }
        } else if let Some(ord) = resolve_date_token(val.trim_start_matches(':'), today) {
            map.insert(v.clone(), AdvancedInput::Date(ord));
        }
    }
    map
}

/// A result block's sort key: a numeric axis (recency, in Unix seconds) or a text
/// value (priority/page/property/planning date). Within one sort every block uses
/// the same variant; the derived `Ord` only ever compares like with like.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SortDecor {
    Num(i64),
    Text(String),
}

/// Fields naming a block's position on the recency time-axis (journal day for
/// journal pages, file mtime otherwise) — sorted numerically, not lexically.
/// `modified` is the canonical token; `updated`/`updated-at`/`date` are aliases.
fn is_recency_field(field: &str) -> bool {
    matches!(
        field.to_ascii_lowercase().as_str(),
        "modified" | "updated" | "updated-at" | "date"
    )
}

/// A page's position on the recency axis, in Unix seconds: a journal page by the
/// midnight of the day it represents (stable — independent of when it was last
/// edited); any other page by its file's last-modified time. `i64::MIN` when a
/// non-journal page can't be stat'd (so it sorts oldest).
fn page_recency_secs(entry: &PageEntry) -> i64 {
    if let Some(dk) = entry.date_key {
        return JournalDate::from_ordinal(dk).to_days() * 86_400;
    }
    std::fs::metadata(&entry.path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(i64::MIN)
}

/// Sort key for a result block: the named property's value if present, else the
/// block's visible first line (lowercased for stable case-insensitive order).
fn sort_key(b: &BlockDto, page: &str, field: &str) -> String {
    match field.to_ascii_lowercase().as_str() {
        // Task priority is the `[#A]` marker, NOT a `priority::` property — map it
        // to A<B<C and sort unprioritized blocks last (so ascending floats A to the
        // top). Descending naturally reverses (A sinks to the bottom).
        // Priority off the DTO's lsdoc-derived facet (header-position `[#A]`, matching
        // the chip) — no reparse, no `[#A]`-anywhere false positive (audit C3/P4).
        "priority" => b
            .priority
            .as_deref()
            .map_or_else(|| "Z".to_string(), |c| c.to_ascii_uppercase()),
        // Sort by the source page name.
        "page" => page.to_lowercase(),
        // SCHEDULED / DEADLINE planning dates off the DTO facet (lead with
        // `YYYY-MM-DD`, so lexical order == chronological). Blocks without one sort
        // last in ascending ("soonest first") order via the high sentinel `~`.
        "deadline" => b.deadline.clone().unwrap_or_else(|| "~".to_string()),
        "scheduled" => b.scheduled.clone().unwrap_or_else(|| "~".to_string()),
        // Otherwise: a block property value (off the DTO's lsdoc properties — no
        // reparse, format-correct, audit P4), else the block's visible first line.
        _ => {
            let field = property_key_norm(field);
            if let Some((_, v)) = b
                .properties
                .iter()
                .find(|(k, _)| property_key_norm(k) == field)
            {
                return v.to_lowercase();
            }
            // Fallback: visible text (the DTO carries no visible text; reparse, bounded
            // to sorted-result blocks via `sort_by_cached_key`).
            let (_, visible) = crate::doc::block_sort_facets(&b.raw);
            visible.lines().next().unwrap_or("").to_lowercase()
        }
    }
}

/// Literal fuzzy full-text autocomplete for the `((` block picker, grouped by
/// page and capped at `limit` total blocks. Ctrl-K uses `run_graph_search*` and
/// retains the shared search dialect through `QueryPlan::friendly*`.
pub fn search(graph: &Graph, query: &str, limit: usize) -> Vec<RefGroup> {
    search_cancellable(graph, query, limit, || false)
}

/// Search with cooperative cancellation for interactive callers. The cheap
/// callback is checked before each block projection, so a superseded rare-prefix
/// scan does not finish walking a huge page in the background.
pub fn search_cancellable(
    graph: &Graph,
    query: &str,
    limit: usize,
    cancelled: impl Fn() -> bool,
) -> Vec<RefGroup> {
    let plan = crate::query_plan::QueryPlan::block_search_literal(query, limit);
    let execution = plan.execute(graph, cancelled);
    if execution.cancelled {
        Vec::new()
    } else {
        crate::query_plan::block_hits_to_groups(execution.hits)
    }
}

/// Find every `template:: <name>` block and the blocks an insertion produces.
pub fn templates(graph: &Graph) -> Vec<TemplateDto> {
    graph.with_pages(|pages| {
        let mut out: Vec<TemplateDto> = Vec::new();
        for (entry, doc) in pages {
            walk(&doc.roots, &mut |b| {
                let Some(name) = b.property("template") else {
                    return;
                };
                if name.is_empty() {
                    return;
                }
                let include_parent =
                    b.property("template-including-parent").as_deref() != Some("false");
                let blocks = if include_parent {
                    vec![template_dto(b, true)]
                } else {
                    b.children.iter().map(|c| template_dto(c, false)).collect()
                };
                out.push(TemplateDto {
                    name,
                    blocks,
                    page: entry.name.clone(),
                    kind: entry.kind,
                });
            });
        }
        out
    })
}

/// Extract templates from one exact application page. `allowed_indices` uses
/// parser pre-order indices and is a candidate filter only; properties are
/// always verified from the current parser DTO before a template is exposed.
pub(crate) fn application_page_templates(
    page: &PageDto,
    allowed_indices: Option<&std::collections::HashSet<usize>>,
) -> Vec<TemplateDto> {
    fn visit_template_blocks(
        blocks: &[BlockDto],
        doc_blocks: &[DocBlock],
        page: &PageDto,
        allowed_indices: Option<&std::collections::HashSet<usize>>,
        next_index: &mut usize,
        out: &mut Vec<TemplateDto>,
    ) {
        for (block, doc_block) in blocks.iter().zip(doc_blocks) {
            let index = *next_index;
            *next_index = next_index.saturating_add(1);
            if allowed_indices.is_none_or(|allowed| allowed.contains(&index)) {
                if let Some(name) = doc_block
                    .property("template")
                    .filter(|name| !name.is_empty())
                {
                    let include_parent =
                        doc_block.property("template-including-parent").as_deref() != Some("false");
                    let blocks = if include_parent {
                        vec![template_dto(doc_block, true)]
                    } else {
                        doc_block
                            .children
                            .iter()
                            .map(|child| template_dto(child, false))
                            .collect()
                    };
                    out.push(TemplateDto {
                        name,
                        blocks,
                        page: page.name.clone(),
                        kind: page.kind,
                    });
                }
            }
            visit_template_blocks(
                &block.children,
                &doc_block.children,
                page,
                allowed_indices,
                next_index,
                out,
            );
        }
    }

    let mut out = Vec::new();
    let mut next_index = 0;
    let is_org = page.format == Format::Org;
    let roots = page
        .blocks
        .iter()
        .map(|block| application_query_doc_block(block, is_org))
        .collect::<Vec<_>>();
    visit_template_blocks(
        &page.blocks,
        &roots,
        page,
        allowed_indices,
        &mut next_index,
        &mut out,
    );
    out
}

/// Convert a template block subtree to a DTO, dropping `id::` (so inserted
/// copies get fresh ids) and, at the root, the `template*` properties.
fn template_dto(b: &DocBlock, strip_template: bool) -> BlockDto {
    let raw = b
        .raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            let drop = t.starts_with("id::")
                || (strip_template
                    && (t.starts_with("template::")
                        || t.starts_with("template-including-parent::")));
            !drop
        })
        .collect::<Vec<_>>()
        .join("\n");
    BlockDto {
        id: String::new(),
        raw,
        collapsed: false,
        children: b.children.iter().map(|c| template_dto(c, false)).collect(),
        breadcrumb: Vec::new(),
        // DUP-8: every non-content field is deliberately reset at insertion;
        // both Direct and Managed template walks delegate to this one leaf.
        page_property: false,
        marker: None,
        priority: None,
        heading_level: None,
        scheduled: None,
        deadline: None,
        tags: Vec::new(),
        properties: Vec::new(),
    }
}

/// Properties that are internal/metadata and shouldn't be offered as query
/// filters (mirrors the frontend's hidden-property set).
const INTERNAL_PROPS: &[&str] = &[
    "id",
    "collapsed",
    "hl-page",
    "hl-color",
    "hl-type",
    "ls-type",
    "background-color",
    "logseq.order-list-type",
    "template",
    "template-including-parent",
];

/// The built-in half of the registry's internal-key exclusion (§6.2 K15). ONE
/// definition: the registry excludes this set ∪ the user's configured
/// `hidden_properties` ∪ every `tine.*` key, and the query-builder facets hide
/// exactly this set.
pub fn internal_property_keys() -> &'static [&'static str] {
    INTERNAL_PROPS
}

#[derive(Clone, Copy)]
pub(crate) enum PropertyFacetMode {
    QueryBuilder,
    Autocomplete,
}

pub(crate) struct PropertyFacetAccumulator {
    mode: PropertyFacetMode,
    hidden: std::collections::HashSet<String>,
    max_items: usize,
    max_bytes: usize,
    items: usize,
    bytes: usize,
    exceeded: bool,
    map: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}

impl PropertyFacetAccumulator {
    pub(crate) fn query_builder(max_values: usize, max_bytes: usize) -> Self {
        Self::new(PropertyFacetMode::QueryBuilder, &[], max_values, max_bytes)
    }

    pub(crate) fn autocomplete(
        extra_hidden: &[String],
        max_items: usize,
        max_bytes: usize,
    ) -> Self {
        Self::new(
            PropertyFacetMode::Autocomplete,
            extra_hidden,
            max_items,
            max_bytes,
        )
    }

    fn new(
        mode: PropertyFacetMode,
        extra_hidden: &[String],
        max_items: usize,
        max_bytes: usize,
    ) -> Self {
        let hidden = match mode {
            PropertyFacetMode::QueryBuilder => INTERNAL_PROPS
                .iter()
                .map(|key| property_key_norm(key))
                .collect(),
            PropertyFacetMode::Autocomplete => OG_AUTOCOMPLETE_HIDDEN_PROPS
                .iter()
                .map(|key| property_key_norm(key))
                .chain(
                    extra_hidden
                        .iter()
                        .map(|key| property_key_norm(key.trim_start_matches(':'))),
                )
                .collect(),
        };
        Self {
            mode,
            hidden,
            max_items,
            max_bytes,
            items: 0,
            bytes: 0,
            exceeded: false,
            map: std::collections::BTreeMap::new(),
        }
    }

    pub(crate) fn offer(&mut self, source_key: &str, source_value: &str) {
        let key = property_key_norm(source_key);
        if key.is_empty() || self.hidden.contains(&key) {
            return;
        }
        let key_missing = !self.map.contains_key(&key);
        match self.mode {
            PropertyFacetMode::QueryBuilder => {
                let value = source_value;
                if value.trim().is_empty()
                    || self
                        .map
                        .get(&key)
                        .is_some_and(|values| values.contains(value))
                {
                    return;
                }
                let key_bytes = if key_missing { key.len() + 64 } else { 0 };
                let next_bytes = self
                    .bytes
                    .saturating_add(key_bytes)
                    .saturating_add(value.len())
                    .saturating_add(64);
                if self.items >= self.max_items || next_bytes > self.max_bytes {
                    self.exceeded = true;
                    return;
                }
                self.items += 1;
                self.bytes = next_bytes;
                self.map.entry(key).or_default().insert(value.to_string());
            }
            PropertyFacetMode::Autocomplete => {
                if key_missing {
                    let key_bytes = key.len().saturating_add(64);
                    if self.items >= self.max_items
                        || self.bytes.saturating_add(key_bytes) > self.max_bytes
                    {
                        self.exceeded = true;
                        return;
                    }
                    self.items += 1;
                    self.bytes = self.bytes.saturating_add(key_bytes);
                    self.map
                        .insert(key.clone(), std::collections::BTreeSet::new());
                }
                let value = source_value.trim();
                if value.is_empty()
                    || self
                        .map
                        .get(&key)
                        .is_some_and(|values| values.contains(value))
                {
                    return;
                }
                let value_bytes = value.len().saturating_add(64);
                if self.items >= self.max_items
                    || self.bytes.saturating_add(value_bytes) > self.max_bytes
                {
                    self.exceeded = true;
                    return;
                }
                self.items += 1;
                self.bytes = self.bytes.saturating_add(value_bytes);
                self.map.entry(key).or_default().insert(value.to_string());
            }
        }
    }

    pub(crate) fn finish(self) -> (Vec<(String, Vec<String>)>, bool) {
        (
            self.map
                .into_iter()
                .map(|(key, values)| (key, values.into_iter().collect()))
                .collect(),
            self.exceeded,
        )
    }
}

/// The owner-preserving property rows of ONE Managed Storage overlay page
/// (§6.2): the registry's Managed source is the masked materialized stream
/// followed by this iterator over every unaccepted local overlay page, so a
/// pending edit's properties are in the snapshot the walk coerces against.
///
/// `application_page_property_pairs` is a thin wrapper over this function —
/// ONE producer, never a twin (D-4).
pub(crate) fn application_page_property_owner_rows(
    page: &PageDto,
    page_id: &str,
    include_page_properties: bool,
) -> Vec<registry::OwnerRow> {
    fn visit(blocks: &[BlockDto], page_id: &str, output: &mut Vec<registry::OwnerRow>) {
        for block in blocks {
            for (ordinal, (key, value)) in block.properties.iter().enumerate() {
                output.push(registry::OwnerRow {
                    owner_type: registry::OwnerType::Block,
                    owner_id: format!("b:{}", block.id),
                    page_id: page_id.to_owned(),
                    source_name: key.clone(),
                    normalized_name: property_key_norm(key),
                    ordinal: ordinal as u32,
                    value: value.clone(),
                });
            }
            visit(&block.children, page_id, output);
        }
    }

    let mut output = Vec::new();
    if include_page_properties {
        for (ordinal, (key, value)) in page_facets(page.pre_block.as_deref())
            .0
            .into_iter()
            .enumerate()
        {
            output.push(registry::OwnerRow {
                owner_type: registry::OwnerType::Page,
                owner_id: format!("p:{page_id}"),
                page_id: page_id.to_owned(),
                source_name: key.clone(),
                normalized_name: property_key_norm(&key),
                ordinal: ordinal as u32,
                value,
            });
        }
    }
    visit(&page.blocks, page_id, &mut output);
    output
}

pub(crate) fn application_page_property_pairs(
    page: &PageDto,
    include_page_properties: bool,
) -> Vec<(String, String)> {
    application_page_property_owner_rows(page, page.name.as_str(), include_page_properties)
        .into_iter()
        .map(|row| (row.source_name, row.value))
        .collect()
}

/// The Direct Files **document** row source (§6.2): the registry's iterator when
/// the projection is not ready. Walks pages (preamble properties, owner = page)
/// and blocks (owner = block) out of the cached `Arc<Document>` snapshot, next
/// to [`property_facets_bounded`] — which is NOT a source, because it aggregates
/// owner identity away and owner identity is what gives cardinality and the
/// distinct-owner counts.
pub fn property_owner_rows(
    graph: &Graph,
) -> (
    Vec<registry::OwnerRow>,
    std::collections::HashMap<String, registry::PageMeta>,
) {
    let mut rows = Vec::new();
    let mut pages = std::collections::HashMap::new();
    graph.with_pages(|entries| {
        for (entry, doc) in entries {
            let page_id = entry.rel_path.clone();
            pages.insert(
                page_id.clone(),
                registry::PageMeta {
                    format: Format::from_path(std::path::Path::new(&entry.rel_path)).into(),
                    name: entry.name.clone(),
                },
            );
            for (ordinal, (key, value)) in page_facets(doc.pre_block.as_deref())
                .0
                .into_iter()
                .enumerate()
            {
                rows.push(registry::OwnerRow {
                    owner_type: registry::OwnerType::Page,
                    owner_id: format!("p:{page_id}"),
                    page_id: page_id.clone(),
                    source_name: key.clone(),
                    normalized_name: property_key_norm(&key),
                    ordinal: ordinal as u32,
                    value,
                });
            }
            walk(&doc.roots, &mut |block| {
                for (ordinal, (key, value)) in block.properties().into_iter().enumerate() {
                    rows.push(registry::OwnerRow {
                        owner_type: registry::OwnerType::Block,
                        owner_id: format!("b:{page_id}#{}", block.uuid),
                        page_id: page_id.clone(),
                        source_name: key.clone(),
                        normalized_name: property_key_norm(&key),
                        ordinal: ordinal as u32,
                        value,
                    });
                }
            });
        }
    });
    (rows, pages)
}

/// Distinct property keys (each with its sorted distinct values) used across the
/// graph. Drives the query builder's property-filter pickers.
pub fn property_facets(graph: &Graph) -> Vec<(String, Vec<String>)> {
    property_facets_bounded(graph, usize::MAX, usize::MAX).0
}

pub fn property_facets_bounded(
    graph: &Graph,
    max_values: usize,
    max_bytes: usize,
) -> (Vec<(String, Vec<String>)>, bool) {
    let mut accumulator = PropertyFacetAccumulator::query_builder(max_values, max_bytes);
    graph.with_pages(|pages| {
        for (_entry, doc) in pages {
            walk(&doc.roots, &mut |b| {
                for (k, v) in b.properties() {
                    accumulator.offer(&k, &v);
                }
            });
        }
    });
    accumulator.finish()
}

/// OG-visible property names and their distinct values for editor completion.
/// Unlike query-builder facets, this includes page preambles and editable
/// built-ins such as `template`/`title`, while applying graph-configured hidden
/// keys. OG sources: db/model.cljs:1394-1405,1422-1443; search.cljs:184-215;
/// util/property.cljs:18-24 at checkout 6e7afa8eb.
const OG_AUTOCOMPLETE_HIDDEN_PROPS: &[&str] = &[
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
];

pub fn autocomplete_property_facets_bounded(
    graph: &Graph,
    max_items: usize,
    max_bytes: usize,
) -> (Vec<(String, Vec<String>)>, bool) {
    let mut accumulator = PropertyFacetAccumulator::autocomplete(
        &graph.config.block_hidden_properties,
        max_items,
        max_bytes,
    );
    graph.with_pages(|pages| {
        for (_entry, doc) in pages {
            for (key, value) in page_facets(doc.pre_block.as_deref()).0 {
                accumulator.offer(&key, &value);
            }
            walk(&doc.roots, &mut |block| {
                for (key, value) in block.properties() {
                    accumulator.offer(&key, &value);
                }
            });
        }
    });
    accumulator.finish()
}

#[cfg(test)]
struct ScoredQuickSwitchCand {
    score: i32,
    index: usize,
}

#[cfg(test)]
impl ScoredQuickSwitchCand {
    fn is_better_than(&self, other: &Self) -> bool {
        self.score > other.score || (self.score == other.score && self.index < other.index)
    }
}

#[cfg(test)]
impl PartialEq for ScoredQuickSwitchCand {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.index == other.index
    }
}

#[cfg(test)]
impl Eq for ScoredQuickSwitchCand {}

#[cfg(test)]
impl PartialOrd for ScoredQuickSwitchCand {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
impl Ord for ScoredQuickSwitchCand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is max-first; define "greater" as worse so the root is the
        // candidate to evict. The final rank remains score desc, index asc.
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.index.cmp(&other.index))
    }
}

#[cfg(test)]
fn push_quick_switch_top(
    heap: &mut std::collections::BinaryHeap<ScoredQuickSwitchCand>,
    limit: usize,
    candidate: ScoredQuickSwitchCand,
) {
    if heap.len() < limit {
        heap.push(candidate);
        return;
    }
    if heap
        .peek()
        .is_some_and(|worst| candidate.is_better_than(worst))
    {
        let mut worst = heap.peek_mut().unwrap();
        *worst = candidate;
    }
}

#[cfg(test)]
fn finish_quick_switch_top(
    heap: std::collections::BinaryHeap<ScoredQuickSwitchCand>,
) -> Vec<ScoredQuickSwitchCand> {
    let mut top = heap.into_vec();
    top.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.index.cmp(&b.index)));
    top
}

/// Fuzzy page-name matcher for the quick switcher. Ranks prefix > substring >
/// subsequence, then by name length.
pub fn quick_switch(graph: &Graph, query: &str, limit: usize) -> Vec<PageEntry> {
    let plan = crate::query_plan::QueryPlan::legacy_page_search(query, limit);
    let execution = plan.execute(graph, || false);
    crate::query_plan::page_hits_to_entries(execution.hits)
}

/// Resolve a `((uuid))` block reference to a shallow identity/result row.
/// Descendants are owned by the source page; explicit bounded consumers use
/// `preview_block`.
pub fn resolve_block(graph: &Graph, uuid: &str) -> Option<RefGroup> {
    // Jump to the owning page via the uuid index, falling back to a full scan if
    // the hint is missing or stale (so a lagging index can never give a wrong
    // answer — just a slower one).
    let hint = graph.block_page_hint(uuid);
    graph.with_pages(|pages| {
        let find_in = |entry: &PageEntry, doc: &Document| -> Option<RefGroup> {
            let mut found: Option<&DocBlock> = None;
            walk(&doc.roots, &mut |b| {
                if found.is_none() && (b.uuid == uuid || b.property("id").as_deref() == Some(uuid))
                {
                    found = Some(b);
                }
            });
            found.map(|b| RefGroup {
                page: entry.name.clone(),
                kind: entry.kind,
                blocks: vec![block_to_shallow_dto(b)],
                evidence: Vec::new(),
            })
        };
        if let Some(h) = &hint {
            if let Some((entry, doc)) = pages.iter().find(|(e, _)| &e.name == h) {
                if let Some(rg) = find_in(entry, doc) {
                    return Some(rg);
                }
            }
        }
        for (entry, doc) in pages {
            if let Some(rg) = find_in(entry, doc) {
                return Some(rg);
            }
        }
        None
    })
}

/// Resolve many `((uuid))` block references in a single graph pass — the real
/// batch behind `Graph::resolve_blocks` (a page full of refs/embeds is one IPC,
/// and now one scan rather than U independent `resolve_block` calls, each of which
/// could whole-graph-scan on a hint miss). Hinted ids are grouped by page and each
/// hinted page is walked ONCE for all of its ids; whatever a hint missed (stale or
/// absent) falls back to a SINGLE whole-graph scan. Match semantics + first-block-
/// wins ordering are identical to `resolve_block`. Output is positional and
/// per-input (duplicate input uuids each get their own `Some(..)`/`None`).
pub fn resolve_blocks(graph: &Graph, uuids: &[String]) -> Vec<Option<RefGroup>> {
    resolve_blocks_bounded(graph, uuids, usize::MAX, usize::MAX).0
}

/// Logseq file graphs keep the first block UUID claimant encountered by the
/// parser and rewrite every later duplicate to a fresh UUID. Provenance:
/// logseq/logseq `c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968`,
/// `deps/graph-parser/src/logseq/graph_parser/block.cljs`,
/// `fix-block-id-if-duplicated!`.
///
/// A physical index may call this only when it has parser order. With an
/// ambiguous unordered hint, `None` deliberately requests parser fallback.
pub(crate) fn logseq_uuid_owner<T>(
    claimants: impl IntoIterator<Item = T>,
    parser_ordered: bool,
) -> Option<T> {
    let mut claimants = claimants.into_iter();
    let first = claimants.next()?;
    if parser_ordered || claimants.next().is_none() {
        Some(first)
    } else {
        None
    }
}

pub fn resolve_blocks_bounded(
    graph: &Graph,
    uuids: &[String],
    max_rows: usize,
    max_bytes: usize,
) -> (Vec<Option<RefGroup>>, bool, usize) {
    use std::collections::{HashMap, HashSet};
    // Distinct requested ids (a page often refs the same uuid repeatedly).
    let distinct: HashSet<&str> = uuids.iter().map(String::as_str).collect();
    if distinct.is_empty() {
        return (uuids.iter().map(|_| None).collect(), false, 0);
    }
    // Bucket each distinct id under its page hint (O(1) per id off the cached
    // uuid index); unhinted ids go straight to the whole-graph fallback.
    let mut by_page: HashMap<String, Vec<&str>> = HashMap::new();
    let mut unhinted: Vec<&str> = Vec::new();
    for &id in &distinct {
        match graph.block_page_hint(id) {
            Some(page) => by_page.entry(page).or_default().push(id),
            None => unhinted.push(id),
        }
    }

    let mut resolved: HashMap<&str, RefGroup> = HashMap::new();
    let mut resolved_budget = ConstructionBudget::new(max_rows, max_bytes);
    graph.with_pages(|pages| {
        let mut page_by_name: HashMap<&str, (&PageEntry, &std::sync::Arc<Document>)> =
            HashMap::with_capacity(pages.len());
        for (entry, doc) in pages {
            page_by_name
                .entry(entry.name.as_str())
                .or_insert((entry, doc));
        }
        // 1) Each hinted page: ONE walk resolving all of its hinted ids.
        for (page, ids) in &by_page {
            if let Some(&(entry, doc)) = page_by_name.get(page.as_str()) {
                let want: HashSet<&str> = ids.iter().copied().collect();
                resolve_ids_in_page(entry, doc, &want, &mut resolved, &mut resolved_budget);
            }
        }
        // 2) Remaining ids (no hint, or the hinted page didn't actually hold the
        //    block) get ONE whole-graph scan — never one-scan-per-id.
        let mut remaining: HashSet<&str> = unhinted.into_iter().collect();
        for &id in &distinct {
            if !resolved.contains_key(id) {
                remaining.insert(id);
            }
        }
        if !remaining.is_empty() {
            for (entry, doc) in pages {
                if resolved.len() == distinct.len() {
                    break; // everything found
                }
                resolve_ids_in_page(entry, doc, &remaining, &mut resolved, &mut resolved_budget);
            }
        }
    });

    let mut output_budget = ConstructionBudget::new(max_rows, max_bytes);
    let output = uuids
        .iter()
        .map(|u| {
            let group = resolved.get(u.as_str())?;
            let block = group.blocks.first()?;
            output_budget
                .admit_estimated(&group.page, crate::model::block_dto_estimated_bytes(block))
                .then(|| group.clone())
        })
        .collect();
    (
        output,
        resolved_budget.exceeded || output_budget.exceeded,
        output_budget.total,
    )
}

fn subtree_node_count(root: &DocBlock) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        count = count.saturating_add(1);
        stack.extend(block.children.iter());
    }
    count
}

fn block_to_bounded_dto(
    block: &DocBlock,
    remaining_nodes: &mut usize,
    remaining_bytes: &mut usize,
) -> Option<BlockDto> {
    if *remaining_nodes == 0 {
        return None;
    }
    let minimum_bytes = block
        .raw
        .len()
        .saturating_add(if block.uuid.is_empty() {
            36
        } else {
            block.uuid.len()
        })
        .saturating_add(128);
    if minimum_bytes > *remaining_bytes {
        return None;
    }
    let mut dto = block_to_shallow_dto(block);
    let dto_bytes = crate::model::block_dto_estimated_bytes(&dto);
    if dto_bytes > *remaining_bytes {
        return None;
    }
    *remaining_nodes -= 1;
    *remaining_bytes -= dto_bytes;
    for child in &block.children {
        let Some(child_dto) = block_to_bounded_dto(child, remaining_nodes, remaining_bytes) else {
            break;
        };
        dto.children.push(child_dto);
    }
    Some(dto)
}

/// One query macro requested by Copy / Export. Query evaluation and subtree
/// hydration stay in the same native operation so a shallow result never causes
/// the WebView to fetch and retain its complete source page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryExportSpec {
    pub key: String,
    pub query: String,
    pub advanced: bool,
    /// The page this macro is written on — the §4.4 execution context, so an
    /// exported advanced query binds `?current-page` to the SAME page the
    /// rendered one did. Defaulted rather than required: the frontend caller
    /// that supplies it is P0-ts, and an absent page is the honest "no binding"
    /// value, never a guess.
    #[serde(default)]
    pub current_page: Option<String>,
}

/// A single query macro's bounded, hierarchy-preserving export projection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportResult {
    pub key: String,
    pub groups: Vec<RefGroup>,
    pub shown: usize,
    pub total: usize,
    pub omitted_nodes: usize,
}

/// All query macros in one export session share the same construction budget.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryExportBatch {
    pub results: Vec<QueryExportResult>,
    /// Query macros beyond the native request cap are not evaluated. The caller
    /// renders an explicit truncation note rather than silently expanding them
    /// through an unbounded sequence of independent requests.
    pub omitted_queries: usize,
}

#[derive(Debug)]
struct SelectedExportRoot {
    page: String,
    kind: PageKind,
    id: String,
}

#[derive(Debug)]
struct SelectedExportQuery {
    key: String,
    total: usize,
    roots: Vec<SelectedExportRoot>,
}

/// One page as export hydration sees it. Built by each backend's
/// [`QueryPageSource::with_hydration_pages`]; neither export entry point
/// constructs it.
pub(crate) struct ExportHydrationPage<'a> {
    kind: PageKind,
    name: &'a str,
    roots: &'a [DocBlock],
}

fn hydrate_selected_export_queries(
    selected: Vec<SelectedExportQuery>,
    source: &dyn QueryPageSource,
    max_nodes: usize,
    max_bytes: usize,
) -> Vec<QueryExportResult> {
    let mut wanted_by_page: HashMap<(PageKind, String), HashSet<String>> = HashMap::new();
    for query in &selected {
        for root in &query.roots {
            wanted_by_page
                .entry((root.kind, root.page.clone()))
                .or_default()
                .insert(root.id.clone());
        }
    }

    let total_wanted = wanted_by_page.values().map(HashSet::len).sum::<usize>();
    let mut results = Vec::new();
    source.with_hydration_pages(&mut |pages| {
        let mut found: HashMap<(PageKind, String, String), &DocBlock> = HashMap::new();
        for page in pages {
            if found.len() == total_wanted {
                break;
            }
            let page_key = (page.kind, page.name.to_owned());
            let Some(wanted) = wanted_by_page.get(&page_key) else {
                continue;
            };
            let mut stack: Vec<&DocBlock> = page.roots.iter().rev().collect();
            while let Some(block) = stack.pop() {
                let property_id = block.property("id");
                let matched = if wanted.contains(block.uuid.as_str()) {
                    Some(block.uuid.as_str())
                } else {
                    property_id.as_deref().filter(|id| wanted.contains(*id))
                };
                if let Some(id) = matched {
                    found.insert((page.kind, page.name.to_owned(), id.to_string()), block);
                    if found.len() == total_wanted {
                        break;
                    }
                }
                for child in block.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        results = emit_selected_export_queries(&selected, &found, max_nodes, max_bytes);
    });
    results
}

/// Emit the bounded DTOs for the already-located roots, in SELECTED-QUERY order
/// (not page order): the node and byte budget is cumulative across macros, so
/// the emission order is part of the contract.
fn emit_selected_export_queries(
    selected: &[SelectedExportQuery],
    found: &HashMap<(PageKind, String, String), &DocBlock>,
    max_nodes: usize,
    max_bytes: usize,
) -> Vec<QueryExportResult> {
    let mut remaining_nodes = max_nodes.max(1);
    let mut remaining_bytes = max_bytes.max(1);
    selected
        .iter()
        .map(|query| {
            let mut groups: Vec<RefGroup> = Vec::new();
            let mut shown = 0usize;
            let mut omitted_nodes = 0usize;
            for root in &query.roots {
                let Some(block) = found.get(&(root.kind, root.page.clone(), root.id.clone()))
                else {
                    omitted_nodes = omitted_nodes.saturating_add(1);
                    continue;
                };
                let total_nodes = subtree_node_count(block);
                let before_nodes = remaining_nodes;
                let dto = block_to_bounded_dto(block, &mut remaining_nodes, &mut remaining_bytes);
                let emitted = before_nodes.saturating_sub(remaining_nodes);
                omitted_nodes = omitted_nodes.saturating_add(total_nodes.saturating_sub(emitted));
                let Some(dto) = dto else {
                    continue;
                };
                shown += 1;
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group.kind == root.kind && group.page == root.page)
                {
                    group.blocks.push(dto);
                } else {
                    groups.push(RefGroup {
                        page: root.page.clone(),
                        kind: root.kind,
                        blocks: vec![dto],
                        evidence: Vec::new(),
                    });
                }
            }
            QueryExportResult {
                key: query.key.clone(),
                groups,
                shown,
                total: query.total,
                omitted_nodes,
            }
        })
        .collect()
}

fn select_export_queries(
    specs: &[QueryExportSpec],
    max_queries: usize,
    max_roots: usize,
    mut evaluate: impl FnMut(&QueryExportSpec) -> BoundedGroups,
) -> (usize, Vec<SelectedExportQuery>) {
    let query_limit = max_queries.max(1);
    let mut remaining_roots = max_roots.max(1);
    let mut selected = Vec::new();
    for spec in specs.iter().take(query_limit) {
        let bounded = evaluate(spec);
        let total = bounded.total;
        let mut roots = Vec::new();
        for group in if bounded.exceeded {
            &[]
        } else {
            bounded.groups.as_slice()
        } {
            for block in &group.blocks {
                if remaining_roots == 0 {
                    break;
                }
                roots.push(SelectedExportRoot {
                    page: group.page.clone(),
                    kind: group.kind,
                    id: block.id.clone(),
                });
                remaining_roots -= 1;
            }
            if remaining_roots == 0 {
                break;
            }
        }
        selected.push(SelectedExportQuery {
            key: spec.key.clone(),
            total,
            roots,
        });
    }
    (query_limit, selected)
}

/// Evaluate and hydrate several Copy / Export query macros under one cumulative
/// root, node, and byte budget. Only the selected block subtrees are cloned into
/// DTOs; complete PageDto values never cross IPC or accumulate in the WebView.
///
/// `max_roots` is deliberately global, not per macro. This keeps a selection
/// containing many distinct query blocks from multiplying the same advertised
/// export limit. Each relevant source document is scanned at most once and only
/// references to the requested roots are retained while the graph snapshot is
/// borrowed.
pub fn export_query_subtrees(
    graph: &Graph,
    specs: &[QueryExportSpec],
    max_queries: usize,
    max_roots: usize,
    max_nodes: usize,
    max_bytes: usize,
) -> QueryExportBatch {
    export_query_subtrees_over(
        &GraphQueryPages(graph),
        specs,
        max_queries,
        max_roots,
        max_nodes,
        max_bytes,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn export_application_query_subtrees(
    pages: &[ApplicationQueryPage],
    specs: &[QueryExportSpec],
    max_queries: usize,
    max_roots: usize,
    max_nodes: usize,
    max_bytes: usize,
    config: crate::config::ParseConfig,
    registry: std::sync::Arc<registry::Registry>,
) -> QueryExportBatch {
    export_query_subtrees_over(
        &ApplicationQueryPages {
            pages,
            config,
            registry,
        },
        specs,
        max_queries,
        max_roots,
        max_nodes,
        max_bytes,
    )
}

/// The construction ceiling one exported query macro may reach while SELECTING
/// its roots, before the caller's own node/byte budget bounds hydration. One
/// definition: the two storage modes previously carried a private copy each.
const QUERY_EXPORT_CONSTRUCTION_ROWS: usize = 20_000;
const QUERY_EXPORT_CONSTRUCTION_BYTES: usize = 32 * 1024 * 1024;

/// The ONE query-export driver: selection ceiling, simple/advanced dispatch,
/// root selection under the global root budget, and hydration.
fn export_query_subtrees_over(
    source: &dyn QueryPageSource,
    specs: &[QueryExportSpec],
    max_queries: usize,
    max_roots: usize,
    max_nodes: usize,
    max_bytes: usize,
) -> QueryExportBatch {
    let (query_limit, selected) = select_export_queries(specs, max_queries, max_roots, |spec| {
        if spec.advanced {
            let (result, exceeded, total) = run_advanced_query_bounded_over(
                source,
                &spec.query,
                spec.current_page.as_deref(),
                QUERY_EXPORT_CONSTRUCTION_ROWS,
                QUERY_EXPORT_CONSTRUCTION_BYTES,
            );
            BoundedGroups {
                groups: result.groups,
                total,
                exceeded,
            }
        } else {
            run_query_bounded_over(
                source,
                &spec.query,
                QUERY_EXPORT_CONSTRUCTION_ROWS,
                QUERY_EXPORT_CONSTRUCTION_BYTES,
            )
        }
    });
    QueryExportBatch {
        results: hydrate_selected_export_queries(selected, source, max_nodes, max_bytes),
        omitted_queries: specs.len().saturating_sub(query_limit),
    }
}

/// Resolve one block for a hover/export consumer that explicitly needs a
/// subtree. This compatibility wrapper applies the caller's node bound; native
/// and export consumers use `preview_block_with_budget` to add a byte bound.
pub fn preview_block(graph: &Graph, uuid: &str, max_nodes: usize) -> Option<BlockPreview> {
    preview_block_with_budget(graph, uuid, max_nodes, usize::MAX)
}

/// Node-and-byte-bounded preview used by IPC and static/export consumers. The
/// byte cap is applied while constructing the DTO, so a legal node count cannot
/// still create an unbounded structured-clone payload. If even the root cannot
/// fit, the preview is returned with an empty block list and the exact omitted
/// count; callers can disclose truncation without confusing "too large" with
/// "block not found".
pub fn preview_block_with_budget(
    graph: &Graph,
    uuid: &str,
    max_nodes: usize,
    max_bytes: usize,
) -> Option<BlockPreview> {
    let max_nodes = max_nodes.max(1);
    let max_bytes = max_bytes.max(1);
    let hint = graph.block_page_hint(uuid);
    graph.with_pages(|pages| {
        let find_in = |entry: &PageEntry, doc: &Document| -> Option<BlockPreview> {
            let mut found: Option<&DocBlock> = None;
            walk(&doc.roots, &mut |block| {
                if found.is_none()
                    && (block.uuid == uuid || block.property("id").as_deref() == Some(uuid))
                {
                    found = Some(block);
                }
            });
            found.map(|block| {
                let total = subtree_node_count(block);
                let mut remaining_nodes = max_nodes;
                let mut remaining_bytes = max_bytes;
                let blocks =
                    block_to_bounded_dto(block, &mut remaining_nodes, &mut remaining_bytes)
                        .into_iter()
                        .collect::<Vec<_>>();
                let emitted = max_nodes - remaining_nodes;
                BlockPreview {
                    group: RefGroup {
                        page: entry.name.clone(),
                        kind: entry.kind,
                        blocks,
                        evidence: Vec::new(),
                    },
                    truncated: total.saturating_sub(emitted),
                }
            })
        };
        if let Some(hint) = &hint {
            if let Some((entry, doc)) = pages.iter().find(|(entry, _)| &entry.name == hint) {
                if let Some(preview) = find_in(entry, doc) {
                    return Some(preview);
                }
            }
        }
        for (entry, doc) in pages {
            if let Some(preview) = find_in(entry, doc) {
                return Some(preview);
            }
        }
        None
    })
}

/// Walk `doc` once, resolving any block whose uuid (or persisted `id::`) is a
/// still-unresolved id in `want`. First block in walk order wins per id (matches
/// `resolve_block`).
fn resolve_ids_in_page<'a>(
    entry: &PageEntry,
    doc: &Document,
    want: &std::collections::HashSet<&'a str>,
    resolved: &mut std::collections::HashMap<&'a str, RefGroup>,
    budget: &mut ConstructionBudget,
) {
    let mut claimants = std::collections::HashMap::<&'a str, Vec<&DocBlock>>::new();
    let mut order = Vec::new();
    walk(&doc.roots, &mut |b| {
        // A block's identity is its uuid OR its persisted `id::`; check both
        // against the wanted set with O(1) lookups (no per-id rescan).
        let hit: Option<&'a str> = want
            .get(b.uuid.as_str())
            .copied()
            .filter(|id| !resolved.contains_key(id))
            .or_else(|| {
                b.property("id")
                    .and_then(|id| want.get(id.as_str()).copied())
                    .filter(|id| !resolved.contains_key(id))
            });
        if let Some(id) = hit {
            if !claimants.contains_key(id) {
                order.push(id);
            }
            claimants.entry(id).or_default().push(b);
        }
    });
    for id in order {
        if let Some(block) = logseq_uuid_owner(claimants.remove(id).unwrap_or_default(), true) {
            if budget.closed() {
                budget.deny_match();
                continue;
            }
            if budget.admit_estimated(&entry.name, shallow_dto_estimated_bytes(block, &[])) {
                let dto = result_dto(block);
                resolved.insert(
                    id,
                    RefGroup {
                        page: entry.name.clone(),
                        kind: entry.kind,
                        blocks: vec![dto],
                        evidence: Vec::new(),
                    },
                );
            }
        }
    }
}

/// Is this query body an advanced datalog query we don't support?
pub fn is_advanced(query_src: &str) -> bool {
    let s = query_src.trim_start();
    s.starts_with("[:find") || s.contains(":where") || s.contains(":find")
}

/// Result-level options extracted from the query's VIEW settings. The walk still
/// consumes this shape in `finish_query_groups`; `ViewSettings` is the truth
/// (Q15) and this is the one adapter between them (it replaced `collect_opts`,
/// which read sort/sample back out of the filter tree).
#[derive(Debug, Default, Clone)]
struct QueryOpts {
    sample: Option<usize>,
    sort: Option<(String, bool)>, // (field, ascending)
}

impl QueryOpts {
    fn from_view(view: &ViewSettings) -> QueryOpts {
        QueryOpts {
            sample: view.sample.map(|n| n as usize),
            sort: view
                .sort
                .first()
                .map(|(field, dir)| (field.0.clone(), *dir == SortDir::Asc)),
        }
    }
}

/// A `(between …)` range whose bounds the advanced dialect already resolved to
/// `yyyymmdd` ordinals.
fn adv_range(attr: Attr, low: Option<i64>, high: Option<i64>) -> Filter {
    let number = |value: i64| Value::Number {
        number: value as f64,
    };
    match (low, high) {
        (Some(low), Some(high)) => {
            // OG's `build-between-two-arg` sorts its two bounds, so
            // `(between END START)` is the same inclusive interval.
            let (low, high) = if low > high { (high, low) } else { (low, high) };
            Filter::attr(
                attr,
                CmpOp::Between,
                Value::List {
                    items: vec![number(low), number(high)],
                },
            )
        }
        (Some(low), None) => Filter::attr(attr, CmpOp::Ge, number(low)),
        (None, Some(high)) => Filter::attr(attr, CmpOp::Le, number(high)),
        (None, None) => Filter::attr(attr, CmpOp::IsSet, Value::None),
    }
}

fn adv_text_list(values: Vec<String>) -> Value {
    Value::List {
        items: values.into_iter().map(Value::text).collect(),
    }
}

fn strip_ref(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t).trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|x| x.strip_suffix("]]"))
        .unwrap_or(t);
    t.trim().to_string()
}

/// Resolve a `between` bound token to a `yyyymmdd` ordinal: `today`/`yesterday`/
/// `tomorrow`, signed durations `±N[dwmy]`, `yyyy-MM-dd`, or a journal title.
fn resolve_date_token(tok: &str, today: JournalDate) -> Option<i64> {
    let t = tok.trim();
    match t.to_ascii_lowercase().as_str() {
        "today" | "now" => return Some(today.ordinal_key()),
        "yesterday" => return Some(today.add_days(-1).ordinal_key()),
        "tomorrow" => return Some(today.add_days(1).ordinal_key()),
        _ => {}
    }
    if let Some(d) = parse_relative(t, today) {
        return Some(d.ordinal_key());
    }
    if let Some(jd) = JournalDate::from_file_stem(t) {
        return Some(jd.ordinal_key());
    }
    journal_ordinal(t)
}

/// Parse a signed relative duration like `-7d`, `+2w`, `3m`, `-1y` off `today`.
fn parse_relative(t: &str, today: JournalDate) -> Option<JournalDate> {
    let bytes = t.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (sign, rest) = match bytes[0] {
        b'+' => (1i64, &t[1..]),
        b'-' => (-1i64, &t[1..]),
        _ => (1i64, t),
    };
    let unit = rest.chars().last()?;
    if !matches!(unit, 'd' | 'w' | 'm' | 'y') {
        return None;
    }
    let n: i64 = rest[..rest.len() - 1].parse().ok()?;
    let n = sign * n;
    Some(match unit {
        'd' => today.add_days(n),
        'w' => today.add_days(n * 7),
        'm' => today.add_months(n),
        'y' => today.add_months(n * 12),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed "today" so relative-date tests are deterministic: 2026-06-16.
    const TODAY: JournalDate = JournalDate {
        year: 2026,
        month: 6,
        day: 16,
    };

    /// The block-anchored evaluable filter of an OG `{{query}}` source — what
    /// the legacy `run_query` path evaluates.
    fn pred(src: &str) -> Filter {
        let (query, _view) = parse_query_source(src, TODAY);
        assert!(
            !query.is_invalid(),
            "{src} parsed with diagnostics: {:?}",
            query.diagnostics
        );
        match query.anchor {
            Anchor::Block => query.evaluable_filter(),
            Anchor::Page => og::rebase_to_block(&query.evaluable_filter()),
        }
    }

    fn view_of(src: &str) -> ViewSettings {
        parse_query_source(src, TODAY).1
    }

    /// Where a block sits, for the leaf tests. `EvalCtx` borrows its
    /// compiled-pattern table, which is per FILTER rather than per page, so the
    /// two are assembled together in [`TestEval::eval`].
    struct Place {
        journal: Option<i64>,
        is_journal: bool,
        page_name: String,
        page_props: Vec<(String, String)>,
    }

    fn ctx_named<'a>() -> Place {
        Place {
            journal: None,
            is_journal: false,
            page_name: "Test".into(),
            page_props: Vec::new(),
        }
    }
    fn ctx_journal<'a>(key: i64) -> Place {
        Place {
            journal: Some(key),
            is_journal: true,
            page_name: "Journal".into(),
            page_props: Vec::new(),
        }
    }
    fn ctx_page(name: &str, props: &[(&str, &str)]) -> Place {
        Place {
            journal: None,
            is_journal: false,
            page_name: name.into(),
            page_props: props
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    trait TestEval {
        fn eval(&self, block: &DocBlock, place: &Place) -> bool;
    }

    impl TestEval for Filter {
        fn eval(&self, block: &DocBlock, place: &Place) -> bool {
            let compiled = eval::CompiledLeaves::for_query(self);
            let config = crate::config::ParseConfig::default();
            let registry = registry::Registry::empty(&config);
            let ctx = EvalCtx {
                journal: place.journal,
                is_journal: place.is_journal,
                page_name: &place.page_name,
                page_props: &place.page_props,
                today: TODAY,
                compiled: &compiled,
                format: crate::query::atom::AtomFormat::Markdown,
                config: &config,
                registry: &registry,
                mode: atom::CompareMode::Both,
            };
            eval::eval_block(self, block, &PathRefCounts::new(), &ctx)
        }
    }

    // Expected-value constructors, so the assertions below read like the DSL
    // they parse rather than like the IR's constructors.
    fn e_page_ref(name: &str) -> Filter {
        Filter::page_ref(name)
    }
    fn e_task(markers: &[&str]) -> Filter {
        Filter::attr(
            Attr::Task,
            CmpOp::In,
            Value::List {
                items: markers.iter().map(|m| Value::text(*m)).collect(),
            },
        )
    }
    fn e_property(key: &str, value: Option<&str>) -> Filter {
        og::property_leaf(key.to_string(), value.map(str::to_string))
    }
    fn e_page_property(key: &str, value: Option<&str>) -> Filter {
        Filter::rel(Rel::Page, Quant::Any, e_property(key, value))
    }
    fn e_content(text: &str) -> Filter {
        og::content_like(text)
    }
    fn e_journal_between(low: &str, high: &str) -> Filter {
        Filter::rel(
            Rel::Page,
            Quant::Any,
            Filter::attr(
                Attr::Day,
                CmpOp::Between,
                Value::List {
                    items: vec![Value::date(low), Value::date(high)],
                },
            ),
        )
    }

    fn projection_cache_page(path: &str, format: Format, raws: &[&str]) -> PageDto {
        PageDto {
            name: path.into(),
            kind: PageKind::Page,
            title: path.into(),
            pre_block: None,
            blocks: raws
                .iter()
                .enumerate()
                .map(|(index, raw)| BlockDto {
                    id: format!("block-{index}"),
                    raw: (*raw).into(),
                    ..BlockDto::default()
                })
                .collect(),
            rev: None,
            format,
            read_only: false,
            path: path.into(),
            activation: None,
            guide: false,
        }
    }

    /// The managed projection cache reuses a retained tree only for content it
    /// has PROVED identical, and it stays inside both of its bounds.
    ///
    /// The path is a lookup key, never a substitute for the comparison. A path
    /// that comes back with different content -- an external edit, a rename
    /// that reuses a filename, a replacement page -- must miss, because the
    /// retained tree carries memoized lsdoc projections of the OLD raw text and
    /// the OLD block identities.
    #[test]
    fn application_projection_cache_reuses_only_proven_identical_content() {
        let mut cache = ApplicationProjectionCache::default();
        let page = projection_cache_page("a.md", Format::Md, &["- one", "- two"]);

        let first = cache.roots("a.md", &page);
        let second = cache.roots("a.md", &page);
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(cache.counters(), (1, 1, 1));

        // Same path, changed raw text.
        let edited = projection_cache_page("a.md", Format::Md, &["- one", "- CHANGED"]);
        let third = cache.roots("a.md", &edited);
        assert!(!std::sync::Arc::ptr_eq(&second, &third));
        assert_eq!(third[1].raw, "- CHANGED");
        assert_eq!(cache.counters(), (1, 2, 1));

        // Same path and text, changed block identity: the id becomes
        // `DocBlock::uuid` and reaches the result DTO, so it cannot be reused.
        let mut reidentified = edited.clone();
        reidentified.blocks[0].id = "block-renamed".into();
        let fourth = cache.roots("a.md", &reidentified);
        assert_eq!(fourth[0].uuid, "block-renamed");
        assert_eq!(cache.counters(), (1, 3, 1));

        // Same path, same text, different parser mode.
        let org = projection_cache_page("a.md", Format::Org, &["- one", "- CHANGED"]);
        let mut org = org;
        org.blocks[0].id = "block-renamed".into();
        let _ = cache.roots("a.md", &org);
        assert_eq!(cache.counters(), (1, 4, 1));

        // Changed child shape at identical parent raw text.
        let mut nested = projection_cache_page("b.md", Format::Md, &["- parent"]);
        nested.blocks[0].children = vec![BlockDto {
            id: "child".into(),
            raw: "- child".into(),
            ..BlockDto::default()
        }];
        let _ = cache.roots("b.md", &nested);
        let flat = projection_cache_page("b.md", Format::Md, &["- parent"]);
        let _ = cache.roots("b.md", &flat);
        assert_eq!(cache.counters(), (1, 6, 2));
    }

    /// Both bounds hold, and a page bigger than the whole byte budget is served
    /// without evicting the graph to store something the next insert would drop.
    #[test]
    fn application_projection_cache_stays_inside_both_bounds() {
        let mut cache = ApplicationProjectionCache::new(2, 1024);
        for page in 0..4 {
            let path = format!("p{page}.md");
            let dto = projection_cache_page(&path, Format::Md, &["- small"]);
            let _ = cache.roots(&path, &dto);
        }
        let (_, _, retained) = cache.counters();
        assert_eq!(retained, 2, "the page bound must evict least-recently-used");

        // The most recent two survive; the oldest was evicted and misses.
        let oldest = projection_cache_page("p0.md", Format::Md, &["- small"]);
        cache.reset_counters();
        let _ = cache.roots("p0.md", &oldest);
        assert_eq!(cache.counters().0, 0, "an evicted page must miss");

        let mut cache = ApplicationProjectionCache::new(64, 64);
        let huge = "- ".to_string() + &"x".repeat(4096);
        let big = projection_cache_page("big.md", Format::Md, &[huge.as_str()]);
        let roots = cache.roots("big.md", &big);
        assert_eq!(roots.len(), 1, "an over-budget page is still served");
        assert_eq!(
            cache.counters().2,
            0,
            "an over-budget page must not be retained"
        );
        let small = projection_cache_page("small.md", Format::Md, &["- s"]);
        let _ = cache.roots("small.md", &small);
        assert_eq!(
            cache.counters().2,
            1,
            "storing the over-budget page must not have evicted the budget"
        );
    }

    fn nested_boolean(head: &str, depth: usize, leaf: &str) -> String {
        format!(
            "{}{}{}",
            format!("({head} ").repeat(depth),
            leaf,
            ")".repeat(depth)
        )
    }

    #[test]
    fn query_parsers_fail_closed_past_the_shared_depth_and_size_limits() {
        let simple_at_limit = nested_boolean("and", QUERY_NESTING_MAX - 1, "(task TODO)");
        assert!(!parse_query_source(&simple_at_limit, TODAY).0.is_invalid());
        let simple_too_deep = nested_boolean("and", QUERY_NESTING_MAX, "(task TODO)");
        assert!(parse_query_source(&simple_too_deep, TODAY).0.is_invalid());

        let advanced_at_limit = format!(
            "[:find (pull ?b [*]) :where {}]",
            nested_boolean("and", QUERY_NESTING_MAX - 1, "(task ?b #{\"TODO\"})")
        );
        let (accepted, _, rejected) = advanced_pred(&advanced_at_limit, None, TODAY);
        assert!(
            accepted.is_some(),
            "unexpected ignored clauses: {rejected:?}"
        );
        let advanced_too_deep = format!(
            "[:find (pull ?b [*]) :where {}]",
            nested_boolean("and", QUERY_NESTING_MAX, "(task ?b #{\"TODO\"})")
        );
        let (rejected, ran, ignored) = advanced_pred(&advanced_too_deep, None, TODAY);
        assert!(rejected.is_none());
        assert!(ran.is_empty());
        assert!(ignored.iter().any(|item| item == "query-nesting-too-deep"));

        let oversized = "x".repeat(QUERY_SOURCE_MAX_BYTES + 1);
        assert!(!query_source_within_limit(&oversized));
        assert!(parse_query_source(&oversized, TODAY).0.is_invalid());

        // The advanced parser must fail closed on size too, at `advanced_pred`
        // itself. `page_affects_advanced_query` calls it directly, so a ceiling
        // enforced only at the `run_advanced_*` entry points is not the shared
        // ceiling this module's doc comment promises.
        let oversized_advanced = format!(
            "[:find (pull ?b [*]) :where (property ?b :note \"{}\")]",
            "y".repeat(QUERY_SOURCE_MAX_BYTES)
        );
        assert!(!query_source_within_limit(&oversized_advanced));
        let (rejected, ran, ignored) = advanced_pred(&oversized_advanced, None, TODAY);
        assert!(rejected.is_none());
        assert!(ran.is_empty());
        assert!(ignored.iter().any(|item| item == "query-too-large"));

        let harmless = format!("(and (content \"{}\"))", "(".repeat(QUERY_NESTING_MAX + 10));
        assert!(query_nesting_within_limit(&harmless));

        let simple_semicolon = format!(";{}", "(".repeat(QUERY_NESTING_MAX + 1));
        assert!(
            !query_nesting_within_limit(&simple_semicolon),
            "semicolon is ordinary text, not a comment, in the simple DSL"
        );
        let advanced_comment = format!(
            "[:find ?b :where ;; {}\n(task ?b #{{\"TODO\"}})]",
            "(".repeat(QUERY_NESTING_MAX + 1)
        );
        assert!(
            query_nesting_within_limit(&advanced_comment),
            "advanced EDN comments must not count delimiter text"
        );
    }

    #[test]
    fn application_backlink_filter_truncates_past_the_managed_block_depth() {
        let mut block = BlockDto {
            id: "leaf".into(),
            raw: "leaf-sentinel".into(),
            ..BlockDto::default()
        };
        for depth in 0..=crate::model::MAX_MANAGED_BLOCK_DEPTH {
            block = BlockDto {
                id: format!("depth-{depth}"),
                raw: format!("depth-{depth}"),
                children: vec![block],
                ..BlockDto::default()
            };
        }
        let projected = application_query_doc_block(&block, false);
        let entry = backlink_filter_entry(
            "Hostile",
            PageKind::Page,
            &projected,
            &std::collections::HashSet::new(),
            usize::MAX,
        );
        assert!(entry.truncated);
        assert!(!entry.text.contains("leaf-sentinel"));

        let mut doc_block = DocBlock {
            raw: "leaf-sentinel".into(),
            children: Vec::new(),
            uuid: "leaf".into(),
            is_org: false,
            proj: std::sync::OnceLock::new(),
        };
        for depth in 0..=crate::model::MAX_MANAGED_BLOCK_DEPTH {
            doc_block = DocBlock {
                raw: format!("depth-{depth}"),
                children: vec![doc_block],
                uuid: format!("doc-depth-{depth}"),
                is_org: false,
                proj: std::sync::OnceLock::new(),
            };
        }
        let entry = backlink_filter_entry(
            "Hostile",
            PageKind::Page,
            &doc_block,
            &std::collections::HashSet::new(),
            usize::MAX,
        );
        assert!(entry.truncated);
        assert!(!entry.text.contains("leaf-sentinel"));
    }

    #[test]
    fn backlink_filter_context_indexes_visible_descendants_and_parser_owned_facets() {
        use std::fs;

        const ROOT: &str = "12345678-1234-4234-8234-123456789abc";
        let dir = std::env::temp_dir().join(format!(
            "tine-backlink-filter-context-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            format!(
                "- Parent [[Target]]\n  id:: {ROOT}\n  - A descendant carries the exact needle [[Other]] #tag\n    tags:: Team\n  - TODO parser-owned task state\n  - ```\n    [[CodeOnly]]\n    ```\n"
            ),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let runtime_id = graph.backlinks("Target")[0].blocks[0].id.clone();
        let context = backlink_filter_context(
            &graph,
            "Target",
            &[
                BacklinkFilterTarget {
                    page: "Source".into(),
                    kind: PageKind::Page,
                    block_id: runtime_id.clone(),
                },
                // Defensive duplicate input must not make a complete response
                // look truncated or duplicate its payload.
                BacklinkFilterTarget {
                    page: "Source".into(),
                    kind: PageKind::Page,
                    block_id: runtime_id,
                },
            ],
        );

        assert!(!context.truncated);
        assert_eq!(context.entries.len(), 1);
        let entry = &context.entries[0];
        assert!(entry.text.contains("exact needle"), "{:?}", entry.text);
        assert!(
            !entry.text.contains("id::"),
            "properties are not visible text"
        );
        let facets = entry
            .facets
            .iter()
            .map(|facet| refs::normalize(facet))
            .collect::<std::collections::HashSet<_>>();
        for expected in ["other", "tag", "team", "todo"] {
            assert!(
                facets.contains(expected),
                "missing {expected}: {:?}",
                entry.facets
            );
        }
        assert!(!facets.contains("target"));
        assert!(
            !facets.contains("codeonly"),
            "code-fence text is not a reference facet"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_pageref_and_tag() {
        assert_eq!(pred("[[Foo]]"), e_page_ref("Foo"));
        assert_eq!(pred("#bar"), e_page_ref("bar"));
        assert_eq!(pred("(tag Foo)"), e_page_ref("Foo"));
    }

    #[test]
    fn parse_boolean() {
        assert_eq!(
            pred("(and [[A]] [[B]])"),
            Filter::and(vec![e_page_ref("A"), e_page_ref("B")])
        );
        assert_eq!(pred("(not [[A]])"), Filter::not(e_page_ref("A")));
    }

    #[test]
    fn parse_task_and_property() {
        assert_eq!(pred("(task TODO DOING)"), e_task(&["TODO", "DOING"]));
        assert_eq!(
            pred("(property type book)"),
            e_property("type", Some("book"))
        );
        assert_eq!(pred("(property public)"), e_property("public", None));
    }

    #[test]
    fn property_key_and_ref_value_match_logseq() {
        // Leading `:` on the key is stripped (keyword form == symbol form).
        assert_eq!(
            pred("(property :type book)"),
            e_property("type", Some("book"))
        );
        // `_` → `-` (Logseq stores `my_key` as `my-key`).
        assert_eq!(pred("(property my_key v)"), e_property("my-key", Some("v")));
        // A `[[page]]` value is captured (was dropped, leaking a stray page-ref).
        assert_eq!(
            pred("(property :fach [[Foo Bar]])"),
            e_property("fach", Some("Foo Bar"))
        );
        // A `#tag` value is captured.
        assert_eq!(
            pred("(property :type #assignment)"),
            e_property("type", Some("assignment"))
        );
        // page-property mirrors the same normalization + value capture.
        assert_eq!(
            pred("(page-property :fach [[Foo]])"),
            e_page_property("fach", Some("Foo"))
        );
    }

    #[test]
    fn reported_and_of_colon_properties_parses_both_clauses() {
        // GH: `(and (property :fach [[X]]) (property :type "#assignment"))` used to
        // parse to And[Property(":fach", None), PageRef(X)] — the colon key never
        // matched, the ref leaked, and the second clause was dropped → "No results".
        let p = pred(
            r##"(and (property :fach [[Management der digitalen Transformation]]) (property :type "#assignment"))"##,
        );
        assert_eq!(
            p,
            Filter::and(vec![
                e_property("fach", Some("Management der digitalen Transformation")),
                // OG's `parse-property-value` strips the leading `#` of a tag
                // spelling, quoted or not.
                e_property("type", Some("assignment")),
            ])
        );
    }

    #[test]
    fn eval_colon_property_and_query_matches_block() {
        let none = ctx_named();
        let mut b = DocBlock::new("assignment one");
        b.raw
            .push_str("\nfach:: [[Management der digitalen Transformation]]\ntype:: #assignment");
        // The reported query now matches a block carrying both properties.
        assert!(pred(
            r##"(and (property :fach [[Management der digitalen Transformation]]) (property :type "#assignment"))"##
        )
        .eval(&b, &none));
        // A different course value does not match.
        assert!(!pred("(property :fach [[Other Course]])").eval(&b, &none));
        // Colon-less form still works (unchanged behavior).
        assert!(pred("(property type assignment)").eval(&b, &none));
    }

    #[test]
    fn parse_escaped_string_content() {
        // `\"`/`\\` inside a quoted full-text term are unescaped (mirrors the
        // query-builder serializer's quoteStr), so a quote in the term doesn't
        // end the string early and silently truncate the query.
        assert_eq!(pred("\"foo \\\"bar\\\"\""), e_content("foo \"bar\""));
        assert_eq!(pred("\"a\\\\b\""), e_content("a\\b"));
        // Only `\"`/`\\` are escapes: a hand-authored backslash before another
        // char is literal, so `"C:\tmp"` stays `C:\tmp` (not `C:tmp`). The term
        // is case-folded at parse time (the content match is case-insensitive).
        assert_eq!(pred("\"a\\q\""), e_content("a\\q"));
        // The IR keeps the user's spelling; the case fold happens at evaluation
        // so the printer can re-emit the term byte-for-byte.
        assert_eq!(pred("\"C:\\tmp\""), e_content("C:\\tmp"));
        // End-to-end: the term still matches a block whose text contains the quote.
        let none = ctx_named();
        let b = DocBlock::new("note: foo \"bar\" baz");
        assert!(pred("\"foo \\\"bar\\\"\"").eval(&b, &none));
    }

    #[test]
    fn search_predicate_preserves_escaped_friendly_source_and_evaluates_it() {
        let parsed = pred(r#"(search "foo \"exact phrase\" -draft OR C:\\tmp")"#);
        assert_eq!(
            parsed,
            Filter::attr(
                Attr::Content,
                CmpOp::Match,
                Value::text(r#"foo "exact phrase" -draft OR C:\tmp"#)
            )
        );

        let none = ctx_named();
        assert!(parsed.eval(&DocBlock::new("foo and an exact phrase, ready"), &none));
        assert!(!parsed.eval(&DocBlock::new("foo and an exact phrase, but draft"), &none));
        // The decoded backslash is passed losslessly to the friendly parser;
        // the second OR branch can therefore match a Windows-style path.
        assert!(parsed.eval(&DocBlock::new(r"open C:\tmp\notes"), &none));

        // The predicate remains an ordinary composable query-DSL clause.
        let task_search = pred(r#"(and (task TODO) (search "foo -draft"))"#);
        assert!(task_search.eval(&DocBlock::new("TODO foo ready"), &none));
        assert!(!task_search.eval(&DocBlock::new("DONE foo ready"), &none));
    }

    #[test]
    fn content_regex_preserves_escapes_and_invalid_patterns_match_nothing() {
        let parsed = pred(r#"(content-regex "ID:\\s+[A-Z]{3}\\d+\\s+\"quoted\"")"#);
        assert_eq!(
            parsed,
            Filter::attr(
                Attr::Content,
                CmpOp::Regex,
                Value::text(r#"ID:\s+[A-Z]{3}\d+\s+"quoted""#)
            )
        );

        let none = ctx_named();
        assert!(parsed.eval(&DocBlock::new(r#"prefix ID: ABC42 "quoted" suffix"#), &none));
        // Rust regex matching is intentionally case-sensitive.
        assert!(!parsed.eval(&DocBlock::new(r#"prefix ID: abc42 "quoted" suffix"#), &none));

        let invalid = pred(r#"(content-regex "[unclosed")"#);
        assert!(matches!(
            invalid,
            Filter::Leaf {
                leaf: Leaf::Attr {
                    op: CmpOp::Regex,
                    ..
                }
            }
        ));
        assert!(!invalid.eval(&DocBlock::new("[unclosed"), &none));
    }

    #[test]
    fn aggregate_and_group_by_parse_as_noop_filters() {
        // 1a: the aggregation/grouping directives ride in the DSL (D2) so the
        // builder round-trips and run_query succeeds; they never filter (eval→true).
        use ir::{AggFn, Field};
        // D2: the four directives are LIFTED out of the filter into the view;
        // what stays in the tree is the neutral `True`.
        assert_eq!(pred("(aggregate count)"), Filter::True);
        assert_eq!(
            view_of("(aggregate count)").aggregates,
            vec![(Field::new(""), AggFn::Count)]
        );
        assert_eq!(
            view_of("(aggregate sum hours)").aggregates,
            vec![(Field::new("hours"), AggFn::Sum)]
        );
        assert_eq!(
            view_of("(aggregate avg score)").aggregates,
            vec![(Field::new("score"), AggFn::Avg)]
        );
        assert_eq!(
            view_of("(group-by page)").group_by,
            Some(Field::new("page"))
        );
        assert_eq!(
            view_of("(group-by status)").group_by,
            Some(Field::new("status"))
        );

        // No-op filter: a block passes regardless.
        let none = ctx_named();
        let b = DocBlock::new("just a note");
        assert!(pred("(aggregate count)").eval(&b, &none));
        assert!(pred("(group-by page)").eval(&b, &none));
        // Combined with a real filter, the aggregate doesn't restrict the matches.
        let task = DocBlock::new("TODO ship it");
        assert!(pred("(and (task TODO) (aggregate count))").eval(&task, &none));
        assert!(!pred("(and (task DONE) (aggregate count))").eval(&task, &none));
    }

    #[test]
    fn advanced_datalog_is_unsupported() {
        assert!(is_advanced(
            "[:find (pull ?b [*]) :where [?b :block/marker]]"
        ));
        assert!(parse_query_source("[:find ?b :where ...]", TODAY)
            .0
            .is_invalid());
    }

    #[test]
    fn advanced_exact_page_property_pair_matches_page_property_predicate() {
        let source = r#"[:find (pull ?p [*])
                         :where
                         [?p :block/properties ?props]
                         [(get ?props :class)]]"#;
        let (lowered, ran, ignored) = advanced_pred(source, None, TODAY);

        assert_eq!(
            lowered.map(|query| query.filter),
            Some(pred("(page-property :class)"))
        );
        assert_eq!(ran, vec!["page-property"]);
        assert!(ignored.is_empty());
    }

    #[test]
    fn advanced_current_page_input_lowers_the_standard_page_relationship() {
        // Logseq graph-parser revision 6e7afa8eb040686ff057156ee877193b581dd369
        // resolves the typed :current-page keyword positionally through
        // current-page-fn and lowercases it before DataScript execution.
        let refs = r#"{:query [:find (pull ?b [*])
                              :in $ ?current-page
                              :where
                              [?p :block/name ?current-page]
                              [?b :block/refs ?p]]
                      :inputs [:current-page]}"#;
        let (lowered, ran, ignored) = advanced_pred(refs, Some("Focus A"), TODAY);

        assert_eq!(
            lowered.map(|query| query.filter),
            Some(e_page_ref("focus a"))
        );
        assert_eq!(ran, vec!["current-page-ref"]);
        assert!(ignored.is_empty());

        let physical = refs.replace(":block/refs", ":block/page");
        let (lowered, ran, ignored) = advanced_pred(&physical, Some("Focus A"), TODAY);
        assert_eq!(
            lowered.map(|query| query.filter),
            Some(Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::Eq, Value::text("focus a"))
            ))
        );
        assert_eq!(ran, vec!["current-page"]);
        assert!(ignored.is_empty());
    }

    #[test]
    fn advanced_typed_inputs_keep_date_bounds_numeric() {
        let source = r#"[:find (pull ?b [*])
                         :in $ ?start ?end
                         :where (between ?b ?start ?end)]
                        :inputs [2026-06-01 2026-06-30]"#;
        let (lowered, ran, ignored) = advanced_pred(source, Some("Not a date"), TODAY);

        assert_eq!(
            lowered.map(|query| query.filter),
            Some(Filter::rel(
                Rel::Page,
                Quant::Any,
                adv_range(Attr::Day, Some(20260601), Some(20260630))
            ))
        );
        assert_eq!(ran, vec!["between"]);
        assert!(ignored.is_empty());
    }

    #[test]
    fn advanced_unrelated_bracket_pattern_stays_unsupported() {
        let source = r#"[:find (pull ?p [*])
                         :where
                         [?p :block/name ?name]
                         [(get ?name :class)]]"#;
        let (lowered, ran, ignored) = advanced_pred(source, None, TODAY);

        assert!(lowered.is_none());
        assert!(ran.is_empty());
        assert_eq!(ignored, vec!["pattern", "pattern"]);
    }

    #[test]
    fn eval_against_blocks() {
        let none = ctx_named();
        let task = DocBlock::new("TODO buy milk for [[Home]]");
        assert!(pred("(task TODO)").eval(&task, &none));
        assert!(pred("[[Home]]").eval(&task, &none));
        assert!(pred("(and (task TODO) [[Home]])").eval(&task, &none));
        assert!(!pred("(and (task DONE) [[Home]])").eval(&task, &none));
        assert!(pred("(not [[Work]])").eval(&task, &none));

        let mut withprop = DocBlock::new("a book");
        withprop.raw.push_str("\ntype:: book");
        assert!(pred("(property type book)").eval(&withprop, &none));
        assert!(pred("(property type)").eval(&withprop, &none));
        assert!(!pred("(property type article)").eval(&withprop, &none));
    }

    #[test]
    fn eval_between_journal_titles() {
        let on_2022 = ctx_journal(20220615);
        let on_2019 = ctx_journal(20190101);
        let b = DocBlock::new("TODO something");
        let q = pred("(between [[Jan 1st, 2021]] [[Jan 1st, 2100]])");
        assert!(q.eval(&b, &on_2022));
        assert!(!q.eval(&b, &on_2019));
        let sched = DocBlock::new("TODO x\nSCHEDULED: <2022-03-03 Thu>");
        assert!(!q.eval(&sched, &ctx_named()));
        assert!(
            pred("(between any [[Jan 1st, 2021]] [[Jan 1st, 2100]])").eval(&sched, &ctx_named())
        );
    }

    /// The unqualified two-bound form is OG's journal-page range. Scheduled and
    /// deadline ranges remain available through their explicit field selectors;
    /// Tine's former permissive union is retained only as explicit `any`.
    #[test]
    fn og_unqualified_between_is_bounded_to_journal_pages() {
        use std::fs;

        const DEC_5_A: &str = "44444444-4444-4444-8444-444444444441";
        const DEC_5_B: &str = "44444444-4444-4444-8444-444444444442";
        const DEC_7_A: &str = "44444444-4444-4444-8444-444444444443";
        const DEC_7_B: &str = "44444444-4444-4444-8444-444444444444";
        const OUTSIDE_JOURNAL: &str = "55555555-5555-4555-8555-555555555555";
        const NAMED_SCHEDULED: &str = "66666666-6666-4666-8666-666666666666";
        let dir = std::env::temp_dir().join(format!(
            "tine-og-between-journal-bounds-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("journals/2020_12_05.md"),
            format!("- first in range\n  id:: {DEC_5_A}\n- second in range\n  id:: {DEC_5_B}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("journals/2020_12_07.md"),
            format!("- third in range\n  id:: {DEC_7_A}\n- fourth in range\n  id:: {DEC_7_B}\n"),
        )
        .unwrap();
        // Both rows have an in-range planning timestamp but live outside the
        // requested journal-page interval. The old Any default leaked them.
        fs::write(
            dir.join("journals/2021_07_01.md"),
            format!("- outside journal\n  SCHEDULED: <2020-12-06 Sun>\n  id:: {OUTSIDE_JOURNAL}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Named.md"),
            format!("- named scheduled\n  DEADLINE: <2020-12-06 Sun>\n  id:: {NAMED_SCHEDULED}\n"),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let ids = run_query(&graph, "(between [[Dec 5th, 2020]] [[Dec 7th, 2020]])")
            .into_iter()
            .flat_map(|group| group.blocks.into_iter().map(persisted_dto_id))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                DEC_5_A.to_string(),
                DEC_5_B.to_string(),
                DEC_7_A.to_string(),
                DEC_7_B.to_string(),
            ]
        );

        // Reversed bounds are normalized by OG's build-between-two-arg.
        let reversed = run_query(&graph, "(between [[Dec 7th, 2020]] [[Dec 5th, 2020]])");
        assert_eq!(
            reversed
                .iter()
                .map(|group| group.blocks.len())
                .sum::<usize>(),
            4
        );
        // Tine's union remains explicitly requestable.
        let any = run_query(&graph, "(between any [[Dec 5th, 2020]] [[Dec 7th, 2020]])");
        assert_eq!(any.iter().map(|group| group.blocks.len()).sum::<usize>(), 6);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn eval_between_relative_dates() {
        // TODAY = 2026-06-16. (between -7d +7d) => [2026-06-09, 2026-06-23].
        let q = pred("(between -7d +7d)");
        // The IR keeps the bounds UNRESOLVED; `today` is applied at evaluation.
        assert_eq!(q, e_journal_between("-7d", "+7d"));
        let b = DocBlock::new("x");
        assert!(q.eval(&b, &ctx_journal(20260616)));
        assert!(q.eval(&b, &ctx_journal(20260609)));
        assert!(!q.eval(&b, &ctx_journal(20260601)));
        // keyword bounds + month/year units
        assert_eq!(
            pred("(between today tomorrow)"),
            e_journal_between("today", "tomorrow")
        );
        let b = DocBlock::new("x");
        assert!(pred("(between today tomorrow)").eval(&b, &ctx_journal(20260617)));
        assert!(!pred("(between today tomorrow)").eval(&b, &ctx_journal(20260618)));
        assert_eq!(pred("(between -1m +1y)"), e_journal_between("-1m", "+1y"));
        assert!(pred("(between -1m +1y)").eval(&b, &ctx_journal(20260516)));
        assert!(!pred("(between -1m +1y)").eval(&b, &ctx_journal(20260515)));
    }

    #[test]
    fn between_field_selector_and_journal_only() {
        // Field keyword parses into the right variant.
        assert_eq!(
            pred("(between journal -30d today)"),
            e_journal_between("-30d", "today")
        );
        assert_eq!(
            pred("(between scheduled -7d +7d)"),
            Filter::attr(
                Attr::Scheduled,
                CmpOp::Between,
                Value::List {
                    items: vec![Value::date("-7d"), Value::date("+7d")]
                }
            )
        );

        // `between journal` restricts to journal pages: a block with an in-range
        // SCHEDULED date on a *named* page must NOT match.
        let q = pred("(between journal -30d today)");
        let sched = DocBlock::new("TODO x\nSCHEDULED: <2026-06-10 Wed>");
        assert!(!q.eval(&sched, &ctx_named())); // named page, journal=None
        assert!(q.eval(&DocBlock::new("TODO y"), &ctx_journal(20260610))); // journal page in range
        assert!(!q.eval(&DocBlock::new("TODO z"), &ctx_journal(20260101))); // journal page out of range

        // `between scheduled` ignores the page's journal date entirely.
        let qs = pred("(between scheduled -30d today)");
        assert!(qs.eval(&sched, &ctx_named()));
        assert!(!qs.eval(&DocBlock::new("TODO y"), &ctx_journal(20260610)));

        // `between deadline` only looks at DEADLINE lines.
        let qd = pred("(between deadline -30d today)");
        let dead = DocBlock::new("TODO x\nDEADLINE: <2026-06-10 Wed>");
        assert!(qd.eval(&dead, &ctx_named()));
        assert!(!qd.eval(&sched, &ctx_named()));
    }

    #[test]
    fn agenda_query_keys_off_scheduled_deadline_not_journal_date() {
        // The journal-agenda DSL the app inserts (window = ±7d around TODAY).
        // It must match on the SCHEDULED/DEADLINE date itself, NOT the journal
        // day the block happens to live on — otherwise a stale-deadline item
        // carried onto a recent day shows up forever (the reported bug).
        let q = pred("(or (between scheduled -7d +7d) (between deadline -7d +7d))");

        // Ancient deadline, sitting on TODAY's journal page: must NOT match.
        let stale = DocBlock::new("TODO old thing\nDEADLINE: <2025-01-01 Wed>");
        assert!(!q.eval(&stale, &ctx_journal(20260616)));

        // Deadline today (on any page): matches.
        let due = DocBlock::new("TODO pay\nDEADLINE: <2026-06-16 Tue>");
        assert!(q.eval(&due, &ctx_named()));

        // Scheduled in range but on an OLD journal page: still matches (the scan
        // is whole-graph; the journal day is irrelevant to the window).
        let sched = DocBlock::new("TODO meet\nSCHEDULED: <2026-06-18 Thu>");
        assert!(q.eval(&sched, &ctx_journal(20200101)));

        // No scheduled/deadline at all: never in the agenda, even on today.
        assert!(!q.eval(&DocBlock::new("just a note"), &ctx_journal(20260616)));
    }

    #[test]
    fn journal_predicate_and_target_query() {
        let b = DocBlock::new("TODO buy milk");
        assert_eq!(
            pred("(journal)"),
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true })
            )
        );
        assert!(pred("(journal)").eval(&b, &ctx_journal(20260616)));
        assert!(!pred("(journal)").eval(&b, &ctx_named()));

        // The motivating query: TODOs on journal pages dated in the last 30 days.
        let q = pred("(and (task TODO) (between journal -30d today))");
        assert!(q.eval(&b, &ctx_journal(20260601)));
        assert!(!q.eval(&b, &ctx_journal(20260101))); // too old
        assert!(!q.eval(&DocBlock::new("DONE buy milk"), &ctx_journal(20260601))); // not TODO
        assert!(!q.eval(&b, &ctx_named())); // not a journal page
    }

    #[test]
    fn eval_page_and_namespace() {
        let b = DocBlock::new("hi");
        let ctx = ctx_page("Project/Alpha", &[]);
        assert!(pred("(page Project/Alpha)").eval(&b, &ctx));
        assert!(!pred("(page Project/Beta)").eval(&b, &ctx));
        assert!(pred("(namespace Project)").eval(&b, &ctx));
        assert!(!pred("(namespace Other)").eval(&b, &ctx));
    }

    #[test]
    fn eval_page_property_and_tags() {
        let b = DocBlock::new("hi");
        let ctx = ctx_page("P", &[("type", "project"), ("tags", "research, active")]);
        assert!(pred("(page-property type project)").eval(&b, &ctx));
        assert!(pred("(page-property type)").eval(&b, &ctx));
        assert!(!pred("(page-property type book)").eval(&b, &ctx));
        assert!(pred("(page-tags research)").eval(&b, &ctx));
        assert!(!pred("(page-tags archived)").eval(&b, &ctx));
    }

    #[test]
    fn eval_content_and_multivalue_property() {
        let none = ctx_named();
        let b = DocBlock::new("the quick brown fox");
        assert!(pred("\"quick brown\"").eval(&b, &none));
        assert!(!pred("\"slow\"").eval(&b, &none));
        // multi-value + page-ref property value matching
        let mut mv = DocBlock::new("x");
        mv.raw.push_str("\ntags:: [[research]], optimization");
        assert!(pred("(property tags research)").eval(&mv, &none));
        assert!(pred("(property tags optimization)").eval(&mv, &none));
        assert!(!pred("(property tags cooking)").eval(&mv, &none));
    }

    #[test]
    fn property_query_matches_folded_source_key() {
        let none = ctx_named();
        let mut block = DocBlock::new("shipped task");
        block.raw.push_str("\ndone_at:: 2026-07-19");

        assert!(pred("(property done-at 2026-07-19)").eval(&block, &none));
        assert!(pred("(property DONE_AT)").eval(&block, &none));
    }

    #[test]
    fn property_facets_group_folded_keys() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-property-key-norm-facets-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages/Properties.md"),
            "- first\n  done_at:: one\n- second\n  done-at:: two\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        assert_eq!(
            property_facets(&graph),
            vec![(
                "done-at".to_string(),
                vec!["one".to_string(), "two".to_string()]
            )]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn managed_simple_query_candidate_plan_is_conservative_across_boolean_shapes() {
        use SimpleQueryCandidatePlan as Plan;
        use SimpleQueryCandidateSource as Source;

        assert_eq!(
            simple_query_candidate_plan("(and (task todo) (priority A) (not (page Templates)))"),
            Plan::Indexed(vec![Source::Task("TODO".into())])
        );
        assert_eq!(
            simple_query_candidate_plan("(or (and (task TODO) (property x y)) (task doing now))"),
            Plan::Indexed(vec![
                Source::Task("DOING".into()),
                Source::Task("NOW".into()),
                Source::Task("TODO".into())
            ])
        );
        assert_eq!(
            simple_query_candidate_plan("(and (not (task TODO)) (property type book))"),
            Plan::Indexed(vec![Source::BlockProperty("type".into())])
        );
        assert_eq!(
            simple_query_candidate_plan(
                "(or (page-ref Alpha) (page-property status public) (page Home))"
            ),
            Plan::Indexed(vec![
                Source::PageRef("alpha".into()),
                Source::PageProperty("status".into()),
                Source::Page("home".into())
            ])
        );
        assert_eq!(
            simple_query_candidate_plan("(and (not (task TODO)) \"x\")"),
            Plan::All
        );
        assert_eq!(
            simple_query_candidate_plan("(or (task TODO) \"x\")"),
            Plan::All
        );
        assert_eq!(simple_query_candidate_plan("\"x\""), Plan::All);
        assert_eq!(
            simple_query_candidate_plan("(page-tags public private)"),
            Plan::Indexed(vec![Source::PageProperty("tags".into())])
        );
        assert_eq!(
            simple_query_candidate_plan("(between journal -7d today)"),
            Plan::Indexed(vec![Source::Journal])
        );
        assert_eq!(simple_query_candidate_plan(")"), Plan::Empty);
    }

    #[test]
    fn sparse_task_query_eligibility_is_conservative_and_canonical() {
        struct Case {
            query: &'static str,
            markers: Option<&'static [&'static str]>,
        }

        let cases = [
            Case {
                query: "(task ToDo)",
                markers: Some(&["TODO"]),
            },
            Case {
                query: "(todo doing Now)",
                markers: Some(&["DOING", "NOW"]),
            },
            Case {
                query: "(and (task TODO DOING) (task todo) (priority A B) (scheduled) (between deadline 2026-06-01 2026-06-30))",
                markers: Some(&["TODO"]),
            },
            Case {
                query: "(and (group-by page) (aggregate count) (sample 2) (sort-by priority desc) (task todo) (deadline))",
                markers: Some(&["TODO"]),
            },
            // Boolean negation/disjunction and task-free positive filters cannot
            // be reduced to a complete positive marker stream.
            Case {
                query: "(or (task TODO) (task DOING))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (not (deadline)))",
                markers: None,
            },
            Case {
                query: "(priority A)",
                markers: None,
            },
            Case {
                query: "(scheduled)",
                markers: None,
            },
            Case {
                query: "(between deadline 2026-06-01 2026-06-30)",
                markers: None,
            },
            // Every other predicate family remains on the existing full path.
            Case {
                query: "(and (task TODO) (page-ref Projects))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (tag Projects))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (page Home))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (property status active))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (page-property status active))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (page-tags research))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (namespace Work))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (journal))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) \"ship\")",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (search ship))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (content-regex \"ship.*\"))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (between any 2026-06-01 2026-06-30))",
                markers: None,
            },
            // Contradictory marker leaves and repeated priority leaves are refused
            // rather than reinterpreted as a storage filter.
            Case {
                query: "(and (task TODO) (task DONE))",
                markers: None,
            },
            Case {
                query: "(and (task TODO) (priority A) (priority B))",
                markers: None,
            },
            Case {
                query: ")",
                markers: None,
            },
            Case {
                query: "(task TODO",
                markers: None,
            },
            Case {
                query: "(task TODO) trailing",
                markers: None,
            },
            Case {
                query: "(and (task TODO) \"unterminated)",
                markers: None,
            },
            Case {
                query: "(unknown TODO)",
                markers: None,
            },
            Case {
                query: "[:find (pull ?b [*]) :where [(= ?b ?b)]]",
                markers: None,
            },
        ];

        for case in cases {
            let actual = sparse_task_query_eligibility(case.query).map(|plan| plan.markers);
            let expected = case.markers.map(|markers| {
                markers
                    .iter()
                    .map(|marker| (*marker).to_owned())
                    .collect::<Vec<_>>()
            });
            assert_eq!(actual, expected, "eligibility mismatch for {}", case.query);
        }
    }

    #[test]
    fn sparse_task_query_eligibility_refuses_permissive_directive_defaults() {
        // `Pred::parse` intentionally keeps these recoverable for Direct Files.
        // The sparse reader must not treat its fallback values as explicit
        // selector instructions.
        for query in [
            "(and (task TODO) (between scheduled))",
            "(and (task TODO) (between scheduled today))",
            "(and (task TODO) (between scheduled not-a-date tomorrow))",
            "(and (task TODO) (sample))",
            "(and (task TODO) (sample not-a-number))",
            "(and (task TODO) (sort-by))",
            "(and (task TODO) (sort-by priority sideways))",
            "(and (task TODO) (aggregate))",
            "(and (task TODO) (aggregate median))",
            "(and (task TODO) (aggregate sum))",
            "(and (task TODO) (group-by))",
            "(and (task TODO) (group-by #))",
        ] {
            assert!(
                sparse_task_query_eligibility(query).is_none(),
                "permissive directive fallback was eligible: {query}"
            );
        }

        for query in [
            "(and (task TODO) (between scheduled today tomorrow))",
            "(and (task TODO) (sample 0) (sort-by custom-prop) (aggregate avg score) (group-by status))",
        ] {
            assert!(
                sparse_task_query_eligibility(query).is_some(),
                "valid directive shape became ineligible: {query}"
            );
        }
    }

    #[test]
    fn autocomplete_property_facets_follow_og_visibility_sources_and_budget() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-property-autocomplete-facets-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:block-hidden-properties #{:hidden_config}}",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Properties.md"),
            "Page_Only:: preamble\nTitle:: Page title\nhidden_config:: secret\n\n- first\n  alpha:: one\n  Alpha_Value:: two\n  template:: My template\n  id:: hidden\n  background_color:: hidden too\n  hidden_config:: block secret\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        assert_eq!(
            autocomplete_property_facets_bounded(&graph, usize::MAX, usize::MAX),
            (
                vec![
                    ("alpha".to_string(), vec!["one".to_string()]),
                    ("alpha-value".to_string(), vec!["two".to_string()]),
                    ("page-only".to_string(), vec!["preamble".to_string()]),
                    ("template".to_string(), vec!["My template".to_string()]),
                    ("title".to_string(), vec!["Page title".to_string()]),
                ],
                false,
            )
        );

        let (bounded, exceeded) = autocomplete_property_facets_bounded(&graph, 3, usize::MAX);
        assert!(exceeded);
        assert!(
            bounded.len()
                + bounded
                    .iter()
                    .map(|(_, values)| values.len())
                    .sum::<usize>()
                <= 3
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_predicate_uses_canonical_unicode_without_accent_folding() {
        let none = ctx_named();
        let block = DocBlock::new("Re\u{301}sume\u{301}");
        assert!(pred("\"Résumé\"").eval(&block, &none));
        assert!(!pred("\"Resume\"").eval(&block, &none));
    }

    #[test]
    fn parse_extracts_options() {
        // The directives no longer ride in the filter: they are lifted into the
        // view on parse and the walk reads them through the one adapter.
        let view = view_of("(and (task TODO) (sample 5) (sort-by priority desc))");
        let opts = QueryOpts::from_view(&view);
        assert_eq!(opts.sample, Some(5));
        assert_eq!(opts.sort, Some(("priority".to_string(), false)));
    }

    #[test]
    /// The walk's planning leaves now read the PROJECTED timestamp rather than
    /// rescanning raw text (SPEC §3.2 G2). The `SCHEDULED:`-only query still
    /// ignores a `DEADLINE:` line and vice versa.
    fn planning_leaves_read_the_projected_timestamp_not_raw_text() {
        let none = ctx_named();
        let scheduled = pred("(between scheduled [[Jul 6th, 2026]] [[Jul 6th, 2026]])");
        let deadline = pred("(between deadline [[Jul 6th, 2026]] [[Jul 6th, 2026]])");

        let own_line = DocBlock::new("TODO x\nSCHEDULED: <2026-07-06 Mon>");
        assert!(scheduled.eval(&own_line, &none));
        assert!(!deadline.eval(&own_line, &none));

        let trailing = DocBlock::new(" SCHEDULED: <2026-07-06 Mon> #email students");
        assert!(scheduled.eval(&trailing, &none));

        let dead = DocBlock::new("DEADLINE: <2026-07-06 Mon>");
        assert!(!scheduled.eval(&dead, &none));
        assert!(deadline.eval(&dead, &none));
    }

    fn quick_switch_fingerprint(entries: Vec<PageEntry>) -> Vec<(String, PageKind, String)> {
        entries
            .into_iter()
            .map(|e| (e.name, e.kind, e.rel_path))
            .collect()
    }

    fn graph_from_page_snapshot(pages: &[(&str, &str, &str)]) -> Graph {
        let pages = pages
            .iter()
            .map(|(name, rel_path, source)| {
                (
                    PageEntry {
                        name: (*name).into(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: (*rel_path).into(),
                        path: (*rel_path).into(),
                    },
                    std::sync::Arc::new(crate::doc::parse(source)),
                )
            })
            .collect();
        Graph::from_page_snapshot("", pages)
    }

    fn persisted_dto_id(block: BlockDto) -> String {
        block
            .properties
            .into_iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("id"))
            .map(|(_, value)| value)
            .expect("fixture block has persisted id::")
    }

    fn search_block_texts(graph: &Graph, query: &str, limit: usize) -> Vec<String> {
        search(graph, query, limit)
            .into_iter()
            .flat_map(|group| group.blocks.into_iter().map(|block| block.raw))
            .collect()
    }

    fn graph_search_block_texts(execution: crate::query_plan::QueryExecution) -> Vec<String> {
        execution
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                crate::query_plan::QueryHit::Block { display_text, .. } => Some(display_text),
                crate::query_plan::QueryHit::Page { .. } => None,
            })
            .collect()
    }

    #[test]
    fn autocomplete_page_or_token_is_literal() {
        let graph = graph_from_page_snapshot(&[
            ("A", "pages/a.md", "- filler\n"),
            ("B", "pages/b.md", "- filler\n"),
            ("ORbit", "pages/orbit.md", "- target\n"),
            (
                "a OR b notes",
                "pages/a-or-b.md",
                "- literal multi-word target\n",
            ),
        ]);

        assert_eq!(
            quick_switch(&graph, "OR", 1)
                .into_iter()
                .map(|page| page.name)
                .collect::<Vec<_>>(),
            ["ORbit"]
        );
        assert_eq!(quick_switch(&graph, "a OR b", 1)[0].name, "a OR b notes");
    }

    #[test]
    fn autocomplete_block_or_token_is_literal() {
        let graph = graph_from_page_snapshot(&[(
            "Logic",
            "pages/logic.md",
            "- logic OR gate\n- unrelated\n",
        )]);

        assert_eq!(search_block_texts(&graph, "OR", 8), ["logic OR gate"]);
    }

    #[test]
    fn autocomplete_negation_token_is_literal() {
        let graph = graph_from_page_snapshot(&[
            ("A", "pages/a.md", "- filler\n"),
            (
                "-foo page",
                "pages/minus-foo.md",
                "- block contains -foo literally\n",
            ),
        ]);

        assert_eq!(
            quick_switch(&graph, "-foo", 1)
                .into_iter()
                .map(|page| page.name)
                .collect::<Vec<_>>(),
            ["-foo page"]
        );
        assert_eq!(
            search_block_texts(&graph, "-foo", 8),
            ["block contains -foo literally"]
        );
    }

    #[test]
    fn autocomplete_no_present_absent_present_ladder() {
        let graph = graph_from_page_snapshot(&[
            ("A", "pages/a.md", "- filler\n"),
            ("B", "pages/b.md", "- filler\n"),
            ("ORbit", "pages/orbit.md", "- target\n"),
        ]);

        for query in ["O", "OR", "ORb"] {
            assert!(
                quick_switch(&graph, query, 1)
                    .iter()
                    .any(|page| page.name == "ORbit"),
                "ORbit disappeared for autocomplete query {query:?}"
            );
        }
    }

    #[test]
    fn ctrlk_dsl_still_active() {
        let graph = graph_from_page_snapshot(&[(
            "Search",
            "pages/search.md",
            "- foo safe\n- foo x excluded\n- bar safe\n- unrelated\n",
        )]);

        let or_hits = graph_search_block_texts(graph.run_graph_search("foo OR bar", 8, 8, false));
        assert!(or_hits.iter().any(|text| text == "foo safe"));
        assert!(or_hits.iter().any(|text| text == "bar safe"));
        assert!(!or_hits.iter().any(|text| text == "unrelated"));

        let excluded = graph_search_block_texts(graph.run_graph_search("foo -x", 8, 8, false));
        assert_eq!(excluded, ["foo safe"]);
        assert!(graph.run_graph_search("-x", 8, 8, false).hits.is_empty());

        let scoped = graph_search_block_texts(graph.run_graph_search_latest_scoped(
            "ctrlk-dsl-current-page",
            "foo -x",
            8,
            8,
            Some(crate::query_plan::QueryPageScope {
                name: "Search".into(),
                page_kind: PageKind::Page,
                path: Some("pages/search.md".into()),
            }),
            false,
        ));
        assert_eq!(scoped, ["foo safe"]);
    }

    #[test]
    fn page_topk_ties_are_input_order_independent() {
        let file_forward = graph_from_page_snapshot(&[
            ("alx", "pages/alx.md", "- file page\n"),
            ("aly", "pages/aly.md", "- file page\n"),
        ]);
        let file_reversed = graph_from_page_snapshot(&[
            ("aly", "pages/aly.md", "- file page\n"),
            ("alx", "pages/alx.md", "- file page\n"),
        ]);
        for graph in [&file_forward, &file_reversed] {
            assert_eq!(quick_switch(graph, "al", 1)[0].name, "alx");
        }

        let refs_forward =
            graph_from_page_snapshot(&[("Source", "pages/source.md", "- [[alx]] [[aly]]\n")]);
        let refs_reversed =
            graph_from_page_snapshot(&[("Source", "pages/source.md", "- [[aly]] [[alx]]\n")]);
        for graph in [&refs_forward, &refs_reversed] {
            let result = quick_switch(graph, "al", 1);
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].name, "alx");
            assert!(
                result[0].rel_path.is_empty(),
                "winner must be reference-only"
            );
        }
    }

    fn quick_switch_reference_full_sort(
        graph: &Graph,
        query: &str,
        limit: usize,
    ) -> Vec<PageEntry> {
        let plan = crate::query_plan::QueryPlan::legacy_page_search(query, usize::MAX);
        crate::query_plan::page_hits_to_entries(plan.execute(graph, || false).hits)
            .into_iter()
            .take(limit)
            .collect()
    }

    #[test]
    fn quick_switch_topk_matches_stable_full_sort_with_ties() {
        use std::fs;
        let dir =
            std::env::temp_dir().join(format!("tine-quick-switch-topk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();

        for i in 0..220 {
            fs::write(
                dir.join("pages").join(format!("aa{i:03}.md")),
                "- tied page\n",
            )
            .unwrap();
        }
        let refs = (0..40)
            .map(|i| format!("[[aa-ref-{i:03}]]"))
            .collect::<Vec<_>>()
            .join(" ");
        fs::write(dir.join("pages").join("zzsource.md"), format!("- {refs}\n")).unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();

        for query in [
            "",
            "aa",
            "000",
            "\"aa\"",
            "/^aa/",
            "aa -zzz",
            "aa OR zzsource",
            "-draft",
            "/(unclosed/",
        ] {
            for limit in [1, 7, 12, 64, 199, 240, 300] {
                let got = quick_switch_fingerprint(quick_switch(&graph, query, limit));
                let expected = quick_switch_fingerprint(quick_switch_reference_full_sort(
                    &graph, query, limit,
                ));
                assert_eq!(got, expected, "query={query:?} limit={limit}");
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_search_topk_keeps_late_best_match_and_ranks_it_first() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-block-search-topk-best-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/aa-weak.md"),
            "- a long weak interior needle match\n- another long weak interior needle match\n",
        )
        .unwrap();
        fs::write(dir.join("pages/zz-best.md"), "- needle\n").unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let ranked = search(&graph, "needle", 2)
            .into_iter()
            .flat_map(|group| {
                group
                    .blocks
                    .into_iter()
                    .map(move |block| (group.page.clone(), block.raw))
            })
            .collect::<Vec<_>>();

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0], ("zz-best".into(), "needle".into()));
        assert!(ranked.iter().any(|(page, _)| page == "zz-best"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_search_topk_uses_stable_traversal_ties() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-block-search-topk-ties-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        for name in ["aa", "bb", "cc", "dd"] {
            fs::write(
                dir.join("pages").join(format!("{name}.md")),
                "- tied needle\n",
            )
            .unwrap();
        }

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let pages = search(&graph, "needle", 3)
            .into_iter()
            .flat_map(|group| std::iter::repeat_n(group.page, group.blocks.len()))
            .collect::<Vec<_>>();
        assert_eq!(pages, ["aa", "bb", "cc"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_search_topk_ties_sort_rel_path_with_reversed_page_snapshot() {
        let pages = ["dd", "cc", "bb", "aa"]
            .into_iter()
            .map(|name| {
                let rel_path = format!("pages/{name}.md");
                (
                    PageEntry {
                        name: name.into(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: rel_path.clone(),
                        path: rel_path.into(),
                    },
                    std::sync::Arc::new(crate::doc::parse("- tied needle\n")),
                )
            })
            .collect();
        let graph = Graph::from_page_snapshot("", pages);

        let pages = search(&graph, "needle", 3)
            .into_iter()
            .flat_map(|group| std::iter::repeat_n(group.page, group.blocks.len()))
            .collect::<Vec<_>>();

        assert_eq!(pages, ["aa", "bb", "cc"]);
    }

    #[test]
    fn block_search_groups_preserve_interleaved_global_rank() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-block-search-ranked-groups-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/aa.md"), "- needle\n- xneedle\n").unwrap();
        fs::write(dir.join("pages/bb.md"), "- needle plus\n").unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let groups = search(&graph, "needle", 3);
        assert_eq!(
            groups
                .iter()
                .map(|group| group.page.as_str())
                .collect::<Vec<_>>(),
            ["aa", "bb", "aa"]
        );
        assert_eq!(
            groups
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(|block| block.raw))
                .collect::<Vec<_>>(),
            ["needle", "needle plus", "xneedle"]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// OG 1.0.0 (`query_dsl.cljs` + the `:page-ref` rule) evaluates a bare
    /// `[[Page]]` simple-query clause against `:block/path-refs`. That relation
    /// includes both explicit references and the page the block physically
    /// belongs to. Keep the explicit `(page …)` operator narrower: it means
    /// physical membership only.
    #[test]
    fn og_bare_page_token_unions_physical_membership_and_explicit_refs() {
        use std::fs;

        const ON_PAGE: &str = "11111111-1111-4111-8111-111111111111";
        const EXPLICIT_REF: &str = "22222222-2222-4222-8222-222222222222";
        const UNRELATED: &str = "33333333-3333-4333-8333-333333333333";
        let dir =
            std::env::temp_dir().join(format!("tine-og-bare-page-union-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages/Parity Target.md"),
            format!("- TODO physically on target\n  id:: {ON_PAGE}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Parity Workflows.md"),
            format!("- TODO explicit [[Parity Target]] witness\n  id:: {EXPLICIT_REF}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Other.md"),
            format!("- TODO unrelated witness\n  id:: {UNRELATED}\n"),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let ids = |query: &str| {
            run_query(&graph, query)
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(persisted_dto_id))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids("(and (task TODO) [[Parity Target]])"),
            vec![ON_PAGE.to_string(), EXPLICIT_REF.to_string()]
        );
        assert_eq!(
            ids("(and (task TODO) (page \"Parity Target\"))"),
            vec![ON_PAGE.to_string()]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// OG's graph parser materializes `:block/path-refs` from every ancestor's
    /// explicit refs (`with-path-refs`), so a bare page-ref query also matches a
    /// descendant whose own text does not repeat the reference. The explicit
    /// `(page ...)` operator remains physical page membership only.
    #[test]
    fn og_bare_page_token_inherits_ancestor_path_refs() {
        use std::fs;

        const ON_PAGE: &str = "44444444-4444-4444-8444-444444444444";
        const INHERITED_CHILD: &str = "55555555-5555-4555-8555-555555555555";
        const INHERITED_GRANDCHILD: &str = "66666666-6666-4666-8666-666666666666";
        const DIRECT_REF: &str = "77777777-7777-4777-8777-777777777777";
        const UNRELATED_CHILD: &str = "88888888-8888-4888-8888-888888888888";
        const INVALIDATION_WITNESS: &str = "99999999-9999-4999-8999-999999999999";
        let dir = std::env::temp_dir().join(format!(
            "tine-og-bare-page-path-refs-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages/Target.md"),
            format!("- TODO physically on target\n  id:: {ON_PAGE}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Workflows.md"),
            format!(
                "- Parent [[Target]]\n  - TODO inherited child\n    id:: {INHERITED_CHILD}\n    - TODO inherited grandchild\n      id:: {INHERITED_GRANDCHILD}\n- Other parent\n  - TODO unrelated child\n    id:: {UNRELATED_CHILD}\n- TODO direct [[Target]]\n  id:: {DIRECT_REF}\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Inherited Only.md"),
            format!(
                "- Cache context [[Target]]\n  - TODO inherited invalidation witness\n    id:: {INVALIDATION_WITNESS}\n"
            ),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let ids = |query: &str| {
            run_query(&graph, query)
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(persisted_dto_id))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids("(and (task TODO) [[Target]])")
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            [ON_PAGE, INHERITED_CHILD, INVALIDATION_WITNESS, DIRECT_REF]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        // OG query presentation suppresses a matching block only when its
        // immediate parent also matched, so the matching grandchild is not a
        // second top-level result.
        assert!(!ids("(and (task TODO) [[Target]])").contains(&INHERITED_GRANDCHILD.to_string()));
        assert_eq!(
            ids("(and (task TODO) (not [[Target]]))"),
            vec![UNRELATED_CHILD.to_string()]
        );
        assert_eq!(
            ids("(and (task TODO) (page \"Target\"))"),
            vec![ON_PAGE.to_string()]
        );

        graph.with_pages(|pages| {
            let (entry, doc) = pages
                .iter()
                .find(|(entry, _)| entry.name == "Inherited Only")
                .expect("inherited-only fixture page");
            let config = crate::config::ParseConfig::default();
            let registry = registry::Registry::empty(&config);
            assert!(page_affects_query(
                "(and (task TODO) [[Target]])",
                entry,
                doc,
                &config,
                &registry
            ));
            assert!(!page_affects_query(
                "(and (task TODO) (page \"Target\"))",
                entry,
                doc,
                &config,
                &registry
            ));
            assert!(page_affects_advanced_query(
                r#"[:find (pull ?b [*]) :where (and (task ?b #{"TODO"}) (page-ref ?b "Target"))]"#,
                None,
                entry,
                doc,
                &config,
                &registry,
            ));
        });

        let _ = fs::remove_dir_all(&dir);
    }

    /// Macro arguments arrive without their source quotes after the parser has
    /// expanded `$1`. OG's simple query reader treats that bare value as a
    /// block-content term; Tine must not silently drop it from an `and` form.
    #[test]
    fn og_bare_word_is_a_content_term() {
        let parsed = pred("(and (task DONE) changelog)");
        assert_eq!(
            parsed,
            Filter::and(vec![e_task(&["DONE"]), e_content("changelog")])
        );
        assert!(parsed.eval(
            &DocBlock::new("DONE Write changelog for v0.0.9"),
            &ctx_named()
        ));
        assert!(!parsed.eval(&DocBlock::new("DONE Publish release notes"), &ctx_named()));
    }

    #[test]
    fn quick_switch_topk_sorts_only_survivors() {
        let limit = 12;
        let total = 240;
        let mut heap = std::collections::BinaryHeap::with_capacity(limit);
        let mut reference = Vec::with_capacity(total);
        for index in 0..total {
            let score = (index % 6) as i32;
            reference.push((score, index));
            push_quick_switch_top(&mut heap, limit, ScoredQuickSwitchCand { score, index });
        }

        let top = finish_quick_switch_top(heap);
        assert_eq!(
            top.len(),
            limit,
            "survivor sort must be bounded by limit, not total candidates"
        );

        reference.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        reference.truncate(limit);
        let got: Vec<(i32, usize)> = top.into_iter().map(|c| (c.score, c.index)).collect();
        assert_eq!(got, reference);
    }

    /// Issue #9: linked references are grouped by referring page, ordered by the
    /// referrer's journal day DESCENDING (newest journal first), with non-journal
    /// referrers last — matching OG (`components/block.cljs` `sort-by :block/journal-day >`).
    // Tine's Favorites layout page holds `[[links]]` so that renames follow it
    // for free — but those links are a sidebar arrangement, not a mention, so
    // the page must never appear in anyone's Linked References. Identity comes
    // from `:tine/favorites-page` in config.edn, NOT from a reserved page name:
    // a user's own page called "Favorites" must keep behaving like any page.
    #[test]
    fn favorites_layout_page_is_never_a_reference_source() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-fav-exclude-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("pages").join("Notes.md"),
            "- a real mention [[Target]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Favorites.md"),
            "tine/favorites:: true\n\n- [[Target]]\n- Work\n\t- [[Target]]\n",
        )
        .unwrap();

        // Without the config key the page is an ORDINARY page and still counts.
        fs::write(dir.join("logseq").join("config.edn"), "{}\n").unwrap();
        let plain = crate::model::Graph::open(&dir);
        let names = |groups: &[crate::model::RefGroup]| {
            let mut names = groups.iter().map(|g| g.page.clone()).collect::<Vec<_>>();
            names.sort();
            names
        };
        assert_eq!(
            names(&plain.backlinks("Target")),
            vec!["Favorites".to_string(), "Notes".to_string()],
            "an unmarked page named Favorites is just a page"
        );

        // With it, the layout page drops out and nothing else does.
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:tine/favorites-page \"Favorites\"}\n",
        )
        .unwrap();
        let marked = crate::model::Graph::open(&dir);
        assert_eq!(
            names(&marked.backlinks("Target")),
            vec!["Notes".to_string()]
        );
        // The target page itself is still excluded from its own references.
        assert!(!names(&marked.backlinks("Notes")).contains(&"Notes".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backlinks_ordered_by_referrer_journal_date_desc() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-backlinks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        // Three journals referencing [[Common]], written OUT of date order; two plain pages.
        fs::write(
            dir.join("journals").join("1897_07_24.md"),
            "- oldestref [[Common]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("2026_06_29.md"),
            "- newestref [[Common]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("journals").join("1927_07_02.md"),
            "- middleref [[Common]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Notes.md"),
            "- plainref [[Common]]\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Alpha.md"),
            "- alpharef [[Common]]\n",
        )
        .unwrap();

        let g = crate::model::Graph::open(&dir);
        let groups = g.backlinks("Common");
        // Identify each group by its block text (robust to the journal title format).
        let tags: Vec<&str> = groups
            .iter()
            .map(|gr| {
                let raw = gr.blocks[0].raw.as_str();
                [
                    "newestref",
                    "middleref",
                    "oldestref",
                    "alpharef",
                    "plainref",
                ]
                .into_iter()
                .find(|t| raw.contains(t))
                .unwrap_or("?")
            })
            .collect();
        assert_eq!(
            tags,
            vec![
                "newestref",
                "middleref",
                "oldestref",
                "alpharef",
                "plainref"
            ],
            "{tags:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn canonical_reference_evidence_keeps_mixed_alias_occurrences_and_properties() {
        use std::fs;
        let dir =
            std::env::temp_dir().join(format!("tine-reference-evidence-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages").join("Target.md"),
            "alias:: Alias\n\n- canonical page\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Source.md"),
            "- [[Alias]] then Alias and Target and `Target`\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Props.md"),
            "related:: [[Alias]]\n\n- ordinary\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let linked = backlinks(&graph, "Target");
        let source = linked.iter().find(|group| group.page == "Source").unwrap();
        assert_eq!(source.blocks.len(), 1);
        assert_eq!(source.evidence.len(), 1);
        assert_eq!(source.evidence[0].occurrences.len(), 1);
        assert_eq!(
            source.evidence[0].occurrences[0].kind,
            ReferenceKind::Explicit
        );
        let props = linked.iter().find(|group| group.page == "Props").unwrap();
        assert!(props.blocks[0].page_property);
        assert_eq!(
            props.evidence[0].occurrences[0].kind,
            ReferenceKind::Explicit
        );

        let unlinked = unlinked_refs(&graph, "Target");
        let source = unlinked
            .iter()
            .find(|group| group.page == "Source")
            .unwrap();
        assert_eq!(
            source.blocks.len(),
            1,
            "one block row, not one row per mention"
        );
        assert_eq!(
            source.evidence[0].occurrences.len(),
            3,
            "alias + title + the mention inside inline code, which Logseq also \
             reports as unlinked (GH #270)"
        );
        assert!(source.evidence[0]
            .occurrences
            .iter()
            .all(|occurrence| occurrence.kind == ReferenceKind::Plain));
        let diagnostics = reference_diagnostics(&graph, "Target");
        assert_eq!(diagnostics.engine_version, "reference-evidence/v1");
        let source_trace = diagnostics
            .traces
            .iter()
            .find(|trace| trace.page == "Source")
            .unwrap();
        assert!(source_trace.included_linked && source_trace.included_unlinked);
        // One explicit `[[Alias]]` plus the three plain mentions above.
        assert_eq!(source_trace.occurrences.len(), 4);
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("launcher-ranking"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn property_keys_create_backlink_membership_with_key_evidence() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-property-key-backlinks-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages/url.md"),
            "url:: https://self.example\n\n- target\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Key Referrer.md"),
            "url:: https://referrer.example\n\n- body\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Block Referrer.md"),
            "- body\n  url:: https://block.example\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Value Referrer.md"),
            "author:: [[url]]\n\n- body\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let refs = backlinks_bounded(&graph, "url", 100, usize::MAX);
        let pages = refs
            .groups
            .iter()
            .map(|group| group.page.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(pages.contains("Key Referrer"), "{pages:?}");
        assert!(pages.contains("Block Referrer"), "{pages:?}");
        assert!(pages.contains("Value Referrer"), "{pages:?}");
        assert!(!pages.contains("url"), "self page must remain excluded");

        let key_group = refs
            .groups
            .iter()
            .find(|group| group.page == "Key Referrer")
            .unwrap();
        let occurrence = &key_group.evidence[0].occurrences[0];
        assert_eq!(occurrence.rule, "explicit_property_key");
        assert_eq!(
            occurrence.span,
            crate::model::ReferenceSpan { start: 0, end: 3 }
        );
        assert_eq!(
            &key_group.blocks[0].raw[occurrence.span.start..occurrence.span.end],
            "url"
        );
        let block_group = refs
            .groups
            .iter()
            .find(|group| group.page == "Block Referrer")
            .unwrap();
        let block_occurrence = block_group.evidence[0]
            .occurrences
            .iter()
            .find(|occurrence| occurrence.rule == "explicit_property_key")
            .unwrap();
        assert_eq!(
            &block_group.blocks[0].raw[block_occurrence.span.start..block_occurrence.span.end],
            "url"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn property_key_membership_uses_canonical_key_fold_and_og_eligibility() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-property-key-eligibility-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:property-pages/excludelist #{:private_key}}",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            "- keys\n  Done_At:: today\n  id:: not-a-reference\n  background-color:: red\n  private-key:: hidden\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        assert_eq!(backlinks(&graph, "done-at")[0].page, "Source");
        assert!(backlinks(&graph, "id").is_empty());
        assert!(backlinks(&graph, "background-color").is_empty());
        assert!(backlinks(&graph, "private-key").is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_property_pages_suppress_only_key_membership() {
        use std::fs;

        let dir =
            std::env::temp_dir().join(format!("tine-property-key-disabled-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:property-pages/enabled? false}",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Key Referrer.md"),
            "url:: https://referrer.example\n\n- body\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Value Referrer.md"),
            "author:: [[url]]\n\n- body\n",
        )
        .unwrap();

        let graph = Graph::open(&dir);
        let refs = backlinks(&graph, "url");
        assert_eq!(
            refs.iter()
                .map(|group| group.page.as_str())
                .collect::<Vec<_>>(),
            vec!["Value Referrer"]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn og_page_identity_and_reference_grouping_use_nfc_without_accent_folding() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-ref-nfc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/Café.md"), "- target\n").unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            "- [[Cafe\u{301}]] and plain Cafe\u{301}\n",
        )
        .unwrap();
        fs::write(dir.join("pages/Ascii.md"), "- [[cafe]]\n").unwrap();
        let graph = Graph::open(&dir);
        let linked = backlinks(&graph, "Café");
        assert_eq!(
            linked.iter().filter(|group| group.page == "Source").count(),
            1
        );
        assert!(!linked.iter().any(|group| group.page == "Ascii"));
        let unlinked = unlinked_refs(&graph, "Café");
        assert_eq!(
            unlinked
                .iter()
                .filter(|group| group.page == "Source")
                .count(),
            1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_page_property_is_unlinked_and_diagnostics_agree() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-page-prop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/Target.md"), "- target\n").unwrap();
        fs::write(dir.join("pages/PageProps.md"), "note:: Target\n\n- body\n").unwrap();
        fs::write(dir.join("pages/BlockProps.md"), "- note:: Target\n").unwrap();
        let graph = Graph::open(&dir);
        let groups = unlinked_refs(&graph, "Target");
        for page in ["PageProps", "BlockProps"] {
            assert_eq!(
                groups
                    .iter()
                    .find(|group| group.page == page)
                    .unwrap()
                    .blocks
                    .len(),
                1
            );
        }
        assert!(
            groups
                .iter()
                .find(|group| group.page == "PageProps")
                .unwrap()
                .blocks[0]
                .page_property
        );
        let diagnostics = reference_diagnostics(&graph, "Target");
        assert!(
            diagnostics
                .traces
                .iter()
                .find(|trace| trace.page == "PageProps")
                .unwrap()
                .included_unlinked
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_source_page_names_merge_into_one_reference_group() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-ref-groups-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages/a")).unwrap();
        fs::create_dir_all(dir.join("pages/b")).unwrap();
        fs::write(dir.join("pages/a/Note.md"), "- first [[Target]]\n").unwrap();
        fs::write(dir.join("pages/b/Note.md"), "- second [[Target]]\n").unwrap();
        let graph = Graph::open(&dir);
        let groups = backlinks(&graph, "Target");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].page, "Note");
        assert_eq!(groups[0].blocks.len(), 2);
        assert_eq!(groups[0].evidence.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn structural_id_value_never_creates_an_unlinked_group() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-ref-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/6a55b643.md"), "- target\n").unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            "- id:: 6a55b643-1234-5678-9abc-def012345678\n",
        )
        .unwrap();
        let graph = Graph::open(&dir);
        assert!(unlinked_refs(&graph, "6a55b643").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_occurrence_evidence_reaches_reference_results() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-ref-total-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/Target.md"), "- target\n").unwrap();
        fs::write(
            dir.join("pages/Source.md"),
            format!("- {}\n", "Target ".repeat(70)),
        )
        .unwrap();
        let graph = Graph::open(&dir);
        let groups = unlinked_refs(&graph, "Target");
        let evidence = &groups[0].evidence[0];
        assert_eq!(evidence.occurrences.len(), 64);
        assert_eq!(evidence.total, 70);
        assert!(evidence.truncated);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_title_beats_colliding_alias() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!(
            "tine-real-page-before-alias-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages/X.md"), "- real title\n").unwrap();
        fs::write(dir.join("pages/Y.md"), "alias:: X\n\n- [[X]]\n").unwrap();
        let graph = Graph::open(&dir);
        assert!(backlinks(&graph, "X").iter().any(|group| group.page == "Y"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_alias_component_keeps_all_edges_and_uses_lexical_canonical() {
        let owned = vec![
            (
                std::path::PathBuf::from("pages/a/B.md"),
                "z".to_string(),
                "B".to_string(),
            ),
            (
                std::path::PathBuf::from("pages/z/A.md"),
                "z".to_string(),
                "A".to_string(),
            ),
        ];
        let aliases = sorted_alias_owners(owned);
        assert_eq!(
            aliases,
            vec![
                ("z".to_string(), "B".to_string()),
                ("z".to_string(), "A".to_string()),
            ],
            "every path-sorted alias edge must reach component resolution"
        );
        assert_eq!(
            equivalent_page_names(&RealPageNames::new(), &aliases, "Z").0,
            "A"
        );
    }

    /// Regression for the pre-0.6 performance audit: recursive `block_to_dto`
    /// used to clone a nested suffix for every matching/query/reference id,
    /// producing N(N+1)/2 wire nodes (and ~1.8 GiB RSS at N=2,000). OG query
    /// presentation suppresses a result whose direct parent is also a result;
    /// references retain every occurrence. All wire rows stay shallow, and an
    /// explicit preview is bounded before allocation.
    #[test]
    fn nested_result_contract_is_non_overlapping_and_preview_is_bounded() {
        use std::fs;

        fn collect_ids(blocks: &[BlockDto], out: &mut Vec<String>) {
            for block in blocks {
                out.push(block.id.clone());
                collect_ids(&block.children, out);
            }
        }
        fn dto_nodes(blocks: &[BlockDto]) -> usize {
            blocks
                .iter()
                .map(|block| 1 + dto_nodes(&block.children))
                .sum()
        }

        const DEPTH: usize = 128;
        let dir =
            std::env::temp_dir().join(format!("tine-non-overlap-results-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        let nested = (0..DEPTH)
            .map(|depth| format!("{}- TODO [[Target]] node {depth}\n", "  ".repeat(depth)))
            .collect::<String>();
        fs::write(dir.join("pages").join("Nested.md"), nested).unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let entry = graph
            .list_pages()
            .into_iter()
            .find(|entry| entry.name == "Nested")
            .unwrap();
        let page = graph.load_page(&entry).unwrap();
        let mut ids = Vec::new();
        collect_ids(&page.blocks, &mut ids);
        assert_eq!(ids.len(), DEPTH);

        let query = run_query(&graph, "(task TODO)");
        assert_eq!(query.iter().map(|g| g.blocks.len()).sum::<usize>(), 1);
        assert_eq!(
            dto_nodes(&query[0].blocks),
            1,
            "query membership DTOs stay shallow"
        );

        let linked = backlinks(&graph, "Target");
        assert_eq!(linked.iter().map(|g| g.blocks.len()).sum::<usize>(), DEPTH);
        assert_eq!(
            linked
                .iter()
                .flat_map(|group| &group.blocks)
                .map(|block| dto_nodes(std::slice::from_ref(block)))
                .sum::<usize>(),
            DEPTH,
            "every reference occurrence remains independently countable but shallow"
        );

        let resolved = resolve_blocks(&graph, &ids);
        assert_eq!(resolved.len(), DEPTH);
        assert_eq!(
            resolved
                .iter()
                .flatten()
                .map(|group| dto_nodes(&group.blocks))
                .sum::<usize>(),
            DEPTH,
            "N requested nested ids must produce N DTO nodes, not N(N+1)/2"
        );

        let preview = preview_block(&graph, &ids[0], 50).unwrap();
        assert_eq!(dto_nodes(&preview.group.blocks), 50);
        assert_eq!(preview.truncated, DEPTH - 50);

        let byte_bounded = preview_block_with_budget(&graph, &ids[0], DEPTH, 512).unwrap();
        assert!(
            byte_bounded
                .group
                .blocks
                .iter()
                .map(crate::model::block_dto_estimated_bytes)
                .sum::<usize>()
                <= 512
        );
        assert!(byte_bounded.truncated > 0);

        let root_too_large = preview_block_with_budget(&graph, &ids[0], DEPTH, 64).unwrap();
        assert!(root_too_large.group.blocks.is_empty());
        assert_eq!(root_too_large.truncated, DEPTH);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn og_query_roots_and_reference_occurrences_cover_matching_descendants_below_a_gap() {
        use std::fs;

        const TARGET_ID: &str = "11111111-1111-4111-8111-111111111111";
        let dir =
            std::env::temp_dir().join(format!("tine-og-query-root-gap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages").join("Nested.md"),
            format!(
                "- TODO [[Target]] (({TARGET_ID})) PlainName ancestor\n  - DONE non-matching gap\n    - TODO [[Target]] (({TARGET_ID})) PlainName grandchild\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("Target.md"),
            format!("- target\n  id:: {TARGET_ID}\n"),
        )
        .unwrap();
        fs::write(dir.join("pages").join("PlainName.md"), "- target\n").unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let raws = |groups: &[RefGroup]| {
            groups
                .iter()
                .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
                .collect::<Vec<_>>()
        };

        let simple = raws(&run_query(&graph, "(task TODO)"));
        assert_eq!(simple.len(), 2);
        assert!(simple.iter().any(|raw| raw.contains("ancestor")));
        assert!(simple.iter().any(|raw| raw.contains("grandchild")));

        let advanced = run_advanced_query(
            &graph,
            "[:find (pull ?b [*]) :where (task ?b \"TODO\")]",
            None,
        );
        assert!(advanced.supported);
        assert_eq!(raws(&advanced.groups).len(), 2);

        let linked = backlinks(&graph, "Target");
        assert_eq!(raws(&linked).len(), 2);
        assert_eq!(
            linked
                .iter()
                .map(|group| group.evidence.len())
                .sum::<usize>(),
            2
        );

        let unlinked = unlinked_refs(&graph, "PlainName");
        assert_eq!(raws(&unlinked).len(), 2);
        assert_eq!(
            unlinked
                .iter()
                .map(|group| group.evidence.len())
                .sum::<usize>(),
            2
        );

        assert_eq!(raws(&block_referrers(&graph, TARGET_ID)).len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_export_hydrates_only_selected_subtrees_under_one_session_budget() {
        use std::fs;

        let dir =
            std::env::temp_dir().join(format!("tine-query-export-budget-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();

        let wide_children = |prefix: &str| {
            (0..5_000)
                .map(|index| format!("  - {prefix} child {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Each matching root has 5,000 descendants. Page A also has a 5,000-node
        // unrelated branch: whole-page hydration would clone/index all 10,002
        // nodes before noticing the export cap.
        fs::write(
            dir.join("pages").join("A.md"),
            format!(
                "- TODO selected A\n{}\n- unrelated branch\n{}\n",
                wide_children("selected-a"),
                wide_children("unrelated-a"),
            ),
        )
        .unwrap();
        fs::write(
            dir.join("pages").join("B.md"),
            format!("- DONE selected B\n{}\n", wide_children("selected-b")),
        )
        .unwrap();

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let batch = export_query_subtrees(
            &graph,
            &[
                QueryExportSpec {
                    key: "todo".into(),
                    query: "(task TODO)".into(),
                    advanced: false,
                    current_page: None,
                },
                QueryExportSpec {
                    key: "done".into(),
                    query: "(task DONE)".into(),
                    advanced: false,
                    current_page: None,
                },
            ],
            64,
            50,
            3,
            1024 * 1024,
        );

        assert_eq!(batch.results.len(), 2);
        assert_eq!(batch.results[0].total, 1);
        assert_eq!(batch.results[0].shown, 1);
        assert_eq!(batch.results[0].groups[0].blocks[0].children.len(), 2);
        assert_eq!(batch.results[0].omitted_nodes, 4_998);
        assert_eq!(batch.results[1].total, 1);
        assert_eq!(batch.results[1].shown, 0);
        assert_eq!(batch.results[1].omitted_nodes, 5_001);
        let emitted = batch
            .results
            .iter()
            .flat_map(|result| result.groups.iter())
            .flat_map(|group| group.blocks.iter())
            .map(crate::model::block_dto_estimated_bytes)
            .sum::<usize>();
        assert!(emitted <= 1024 * 1024);
        assert!(batch.results.iter().all(|result| {
            result
                .groups
                .iter()
                .flat_map(|group| group.blocks.iter())
                .all(|block| !block.raw.contains("unrelated branch"))
        }));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interactive_search_stops_inside_a_page_when_superseded() {
        use std::cell::Cell;
        use std::fs;
        let dir = std::env::temp_dir().join(format!("tine-search-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let content = (0..1000)
            .map(|i| format!("- ordinary block {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("pages").join("Large.md"), content).unwrap();
        let graph = Graph::open(&dir);
        let checks = Cell::new(0usize);
        let result = search_cancellable(&graph, "never-matches", 10, || {
            checks.set(checks.get() + 1);
            checks.get() > 12
        });
        assert!(result.is_empty());
        assert!(checks.get() < 40, "cancellation checks: {}", checks.get());
        let _ = fs::remove_dir_all(&dir);
    }

    fn sparse_test_block(id: &str, raw: &str, children: Vec<BlockDto>) -> BlockDto {
        BlockDto {
            id: id.into(),
            raw: raw.into(),
            children,
            ..BlockDto::default()
        }
    }

    fn sparse_test_page(
        name: &str,
        path: &str,
        kind: PageKind,
        format: Format,
        recency: i64,
        blocks: Vec<BlockDto>,
    ) -> ApplicationQueryPage {
        let page = PageDto {
            name: name.into(),
            kind,
            title: name.into(),
            pre_block: None,
            blocks,
            rev: None,
            format,
            read_only: false,
            path: path.into(),
            activation: None,
            guide: false,
        };
        let roots = ApplicationProjectionCache::default().roots(&page.path, &page);
        let journal = (page.kind == PageKind::Journal)
            .then(|| crate::date::JournalFormat::default().parse(&page.name))
            .flatten()
            .map(|date| date.ordinal_key());
        ApplicationQueryPage {
            page,
            roots,
            recency,
            journal,
        }
    }

    fn sparse_test_candidate(
        id: &str,
        raw: &str,
        page: &ApplicationQueryPage,
        parent_identity: Option<&str>,
        dfs_order: &[&str],
    ) -> ApplicationSparseQueryCandidate {
        ApplicationSparseQueryCandidate {
            raw: raw.into(),
            identity: id.into(),
            page: ApplicationSparseQueryPage {
                name: page.page.name.clone(),
                path: page.page.path.clone(),
                kind: page.page.kind,
                is_org: page.page.format == Format::Org,
                recency: page.recency,
            },
            parent_identity: parent_identity.map(str::to_owned),
            dfs_order: dfs_order.iter().map(|part| (*part).to_owned()).collect(),
        }
    }

    fn sparse_result_value(result: BoundedGroups) -> serde_json::Value {
        serde_json::json!({
            "groups": result.groups,
            "total": result.total,
            "exceeded": result.exceeded,
        })
    }

    #[test]
    fn sparse_task_query_runner_matches_existing_page_evaluator() {
        let markdown = sparse_test_page(
            "Work",
            "pages/Work.md",
            PageKind::Page,
            Format::Md,
            10,
            vec![
                sparse_test_block(
                    "md-parent",
                    "TODO [#A] parent\nSCHEDULED: <2026-06-16 Tue>",
                    vec![sparse_test_block(
                        "md-child",
                        "TODO [#A] child\nSCHEDULED: <2026-06-16 Tue>",
                        Vec::new(),
                    )],
                ),
                sparse_test_block(
                    "md-deadline",
                    "TODO [#B] deadline\nDEADLINE: <2026-06-19 Fri>",
                    Vec::new(),
                ),
            ],
        );
        let org = sparse_test_page(
            "Org Tasks",
            "pages/Org Tasks.org",
            PageKind::Page,
            Format::Org,
            20,
            vec![sparse_test_block(
                "org-deadline",
                "TODO [#A] org deadline\nDEADLINE: <2026-06-18 Thu>",
                Vec::new(),
            )],
        );
        let pages = vec![markdown, org];
        // Deliberately not input-DFS order: the sparse core must use the
        // structural keys supplied by its caller before it admits result DTOs.
        let candidates = vec![
            sparse_test_candidate(
                "org-deadline",
                "TODO [#A] org deadline\nDEADLINE: <2026-06-18 Thu>",
                &pages[1],
                None,
                &["c"],
            ),
            sparse_test_candidate(
                "md-child",
                "TODO [#A] child\nSCHEDULED: <2026-06-16 Tue>",
                &pages[0],
                Some("md-parent"),
                &["a", "a"],
            ),
            sparse_test_candidate(
                "md-deadline",
                "TODO [#B] deadline\nDEADLINE: <2026-06-19 Fri>",
                &pages[0],
                None,
                &["b"],
            ),
            sparse_test_candidate(
                "md-parent",
                "TODO [#A] parent\nSCHEDULED: <2026-06-16 Tue>",
                &pages[0],
                None,
                &["a"],
            ),
        ];

        for (query, max_rows, max_bytes) in [
            (
                "(and (task todo) (priority A) (scheduled))",
                usize::MAX,
                usize::MAX,
            ),
            (
                "(and (task TODO) (between deadline 2026-06-15 2026-06-20) (sort-by priority asc))",
                usize::MAX,
                usize::MAX,
            ),
            (
                "(and (task TODO) (sort-by priority desc) (sample 2) (aggregate count) (group-by page))",
                usize::MAX,
                usize::MAX,
            ),
            ("(and (task TODO) (sample 1))", usize::MAX, usize::MAX),
            ("(task TODO)", 1, usize::MAX),
            ("(task TODO)", usize::MAX, 1),
        ] {
            let config = crate::config::ParseConfig::default();
            let registry = std::sync::Arc::new(registry::Registry::empty(&config));
            let page_result = run_application_query_pages_bounded(
                &pages,
                query,
                max_rows,
                max_bytes,
                config.clone(),
                std::sync::Arc::clone(&registry),
            );
            let sparse_result = run_application_sparse_task_query_bounded(
                &candidates,
                query,
                max_rows,
                max_bytes,
                &config,
                &registry,
            )
            .expect("eligible fixture query");
            assert_eq!(
                sparse_result_value(sparse_result),
                sparse_result_value(page_result),
                "sparse result drifted for {query}"
            );
        }
    }

    #[test]
    fn sparse_task_query_runner_refuses_noncanonical_identities() {
        let page = sparse_test_page(
            "Work",
            "pages/Work.md",
            PageKind::Page,
            Format::Md,
            0,
            Vec::new(),
        );
        let duplicate = vec![
            sparse_test_candidate("same", "TODO one", &page, None, &["a"]),
            sparse_test_candidate("same", "TODO two", &page, None, &["b"]),
        ];
        let config = crate::config::ParseConfig::default();
        let registry = registry::Registry::empty(&config);
        assert!(matches!(
            run_application_sparse_task_query_bounded(
                &duplicate,
                "(task TODO)",
                usize::MAX,
                usize::MAX,
                &config,
                &registry,
            ),
            Err(ApplicationSparseQueryError::DuplicateIdentity)
        ));
        let missing = vec![sparse_test_candidate("", "TODO one", &page, None, &["a"])];
        assert!(matches!(
            run_application_sparse_task_query_bounded(
                &missing,
                "(task TODO)",
                usize::MAX,
                usize::MAX,
                &config,
                &registry,
            ),
            Err(ApplicationSparseQueryError::MissingIdentity)
        ));
    }

    #[test]
    fn result_families_stop_constructing_at_row_and_byte_budgets() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!(
            "tine-result-construction-budget-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        let content = (0..12)
            .map(|i| {
                format!(
                    "- TODO [[Target]] item {i}\n  field-{i}:: {}",
                    "x".repeat(100)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("pages/Source.md"), content).unwrap();
        fs::write(dir.join("pages/Target.md"), "- target\n").unwrap();
        let graph = Graph::open(&dir);

        RESULT_DTO_CONSTRUCTIONS.with(|count| count.set(0));
        let query = run_query_bounded(&graph, "(task TODO)", 3, usize::MAX);
        assert!(query.exceeded);
        assert_eq!(query.total, 12);
        assert_eq!(
            query
                .groups
                .iter()
                .map(|group| group.blocks.len())
                .sum::<usize>(),
            3
        );
        assert_eq!(RESULT_DTO_CONSTRUCTIONS.with(std::cell::Cell::get), 3);

        RESULT_DTO_CONSTRUCTIONS.with(|count| count.set(0));
        crate::reference_evidence::reset_occurrence_constructions();
        let refs = backlinks_bounded(&graph, "Target", 2, usize::MAX);
        assert!(refs.exceeded);
        assert_eq!(refs.total, 12);
        assert_eq!(
            refs.groups
                .iter()
                .map(|group| group.blocks.len())
                .sum::<usize>(),
            2
        );
        assert_eq!(RESULT_DTO_CONSTRUCTIONS.with(std::cell::Cell::get), 2);
        assert_eq!(crate::reference_evidence::occurrence_constructions(), 2);

        RESULT_DTO_CONSTRUCTIONS.with(|count| count.set(0));
        let sample = run_query_bounded(&graph, "(and (task TODO) (sample 1))", 20, usize::MAX);
        assert!(!sample.exceeded);
        assert_eq!(sample.total, 1);
        assert_eq!(RESULT_DTO_CONSTRUCTIONS.with(std::cell::Cell::get), 1);

        let (facets, facets_exceeded) = property_facets_bounded(&graph, 2, usize::MAX);
        assert!(facets_exceeded);
        assert!(facets.iter().map(|(_, values)| values.len()).sum::<usize>() <= 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
