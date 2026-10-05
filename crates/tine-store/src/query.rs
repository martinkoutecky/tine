//! Graph-wide reference, simple-query, and advanced-query evaluation over
//! parsed pages. Simple queries support page and tag references, Boolean
//! clauses, task markers, and property filters. Advanced `[:find ...]`
//! queries evaluate the supported `:where` subset for tasks, dates,
//! references, properties, priority, and Boolean clauses. Unsupported
//! clauses are returned as diagnostics in `AdvancedResult`.

use crate::model::GraphRead;
use tine_core::date::{JournalDate, JournalFormat};
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::model::{
    BacklinkFilterContext, BacklinkFilterEntry, BacklinkFilterTarget, BlockDto, BlockPreview,
    Format, PageEntry, PageKind, RefGroup, ReferenceBlockEvidence, ReferenceKind, TemplateDto,
};
use tine_core::projection::block_to_shallow_dto;
use tine_core::projection::{block_to_bounded_dto, subtree_node_count};
use tine_core::query::{
    admit_source, AdvancedResult, QueryExportBatch, QueryExportResult, QueryExportSpec,
};
use tine_core::refs;
mod eval;
pub(crate) mod exec;
pub(crate) mod index;
pub(crate) mod memo;
pub(crate) mod page_properties;
#[cfg(test)]
mod sample_seed_tests;
pub(crate) use page_properties::page_facets;
use page_properties::{page_document_is_org, page_property_lines};

#[derive(Debug, Clone)]
pub(crate) struct BoundedGroups {
    pub(crate) groups: Vec<RefGroup>,
    pub(crate) total: usize,
    pub(crate) exceeded: bool,
}

struct ConstructionBudget {
    max_rows: usize,
    max_bytes: usize,
    rows: usize,
    bytes: usize,
    total: usize,
    exceeded: bool,
}

impl ConstructionBudget {
    fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            max_rows,
            max_bytes,
            rows: 0,
            bytes: 0,
            total: 0,
            exceeded: false,
        }
    }

    fn admit_estimated(&mut self, page: &str, payload_bytes: usize) -> bool {
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

    /// A page row costs its raw estimate; `total` counts admitted rows.
    fn admit_page_estimated(&mut self, estimated_bytes: usize) -> bool {
        if self.exceeded
            || self.rows >= self.max_rows
            || self.bytes.saturating_add(estimated_bytes) > self.max_bytes
        {
            self.exceeded = true;
            return false;
        }
        self.rows += 1;
        self.bytes += estimated_bytes;
        self.total = self.rows;
        true
    }

    fn deny_match(&mut self) {
        self.total = self.total.saturating_add(1);
        self.exceeded = true;
    }

    fn closed(&self) -> bool {
        self.exceeded || self.rows >= self.max_rows
    }
}

/// Walk all blocks of a document depth-first, calling `f(block)`.
fn walk<'a>(blocks: &'a [DocBlock], f: &mut impl FnMut(&'a DocBlock)) {
    for b in blocks {
        f(b);
        walk(&b.children, f);
    }
}

/// Preorder walk that stops as soon as the bounded collector rejects a value.
fn walk_until<'a>(blocks: &'a [DocBlock], f: &mut impl FnMut(&'a DocBlock) -> bool) -> bool {
    for block in blocks {
        if !f(block) || !walk_until(&block.children, f) {
            return false;
        }
    }
    true
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
    let bytes = chars.by_ref().take(60).map(char::len_utf8).sum::<usize>();
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
        .saturating_add(block.raw().len())
        .saturating_add(
            ancestors
                .iter()
                .map(|ancestor| crumb_line_estimated_bytes(ancestor))
                .sum::<usize>(),
        )
        .saturating_add(projection.tags().iter().map(String::len).sum::<usize>())
        .saturating_add(
            projection
                .properties()
                .iter()
                .map(|(key, value)| key.len().saturating_add(value.len()))
                .sum::<usize>(),
        )
        .saturating_add(128)
}

fn reference_evidence_estimated_bytes(evidence: &ReferenceBlockEvidence) -> usize {
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

#[cfg(test)]
pub(crate) fn result_dto_constructions() -> usize {
    RESULT_DTO_CONSTRUCTIONS.with(std::cell::Cell::get)
}

/// Cancellable variant used by interactive search. Returning false from `f`
/// stops the entire depth-first walk, including the current deep page.
/// A short, single-line label for a block in a breadcrumb trail.
pub(crate) fn crumb_line(b: &DocBlock) -> String {
    let line = b
        .visible_text()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line
    }
}

/// Collect matching blocks across the graph, grouped by source page. Scans the
/// graph's in-memory page cache (built once, kept in sync by edits) so no disk
/// I/O or re-parsing happens per call.
#[cfg(test)]
fn collect(
    graph: &impl GraphRead,
    keep: impl FnMut(&DocBlock) -> bool,
    keep_page_properties: impl FnMut(&PageEntry, &str) -> Option<BlockDto>,
    exclude: Option<&str>,
) -> Vec<RefGroup> {
    collect_bounded(
        graph,
        keep,
        keep_page_properties,
        exclude,
        usize::MAX,
        usize::MAX,
    )
    .groups
}

fn collect_bounded(
    graph: &impl GraphRead,
    mut keep: impl FnMut(&DocBlock) -> bool,
    mut keep_page_properties: impl FnMut(&PageEntry, &str) -> Option<BlockDto>,
    exclude: Option<&str>,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let ex = exclude.map(refs::normalize);
    let mut budget = ConstructionBudget::new(max_rows, max_bytes);
    let groups = graph.with_pages(|pages| {
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
                        tine_core::model::block_dto_estimated_bytes(&property_ref),
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
                        .map(|ancestor| crumb_line(ancestor))
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
        // tie-breaker. Without it, static Guide/demo exports differed across machines.
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

/// True when a block's raw text is only page-header properties (OG treats such
/// a FIRST block as the page-properties (pre-)block). One answerer: the parser's
/// page header (`tine_core::block_regions::page_header_only`, I-12).
fn is_properties_only(raw: &str) -> bool {
    tine_core::block_regions::page_header_only(raw.trim_end_matches(['\n', '\r'])).is_some()
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
/// lets cache invalidation compare sorted, deduplicated normalized alias sets
/// instead of treating an unchanged `alias::` line as a change.
pub(crate) fn document_aliases(doc: &Document) -> Vec<String> {
    let alias_text: Option<&str> = match &doc.pre_block {
        Some(pre) => Some(pre.as_str()),
        // No pre-block: a properties-only FIRST block is the page-properties
        // block in OG (it gets written back as a pre-block on save there).
        None => doc
            .roots
            .first()
            .filter(|b| is_properties_only(b.raw()))
            .map(|b| b.raw()),
    };
    let Some(text) = alias_text else {
        return Vec::new();
    };
    let mut aliases = Vec::new();
    for (k, v) in page_property_lines(text, page_document_is_org(doc)) {
        let key = property_key_norm(&k);
        if key == "alias" || key == "aliases" {
            let trimmed = v.trim();
            if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
                continue;
            }
            for alias in v.split(tine_core::refs::is_linkable_property_separator) {
                let alias = strip_ref(alias.trim());
                if !alias.is_empty() {
                    aliases.push(refs::page_key(&alias));
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

#[cfg(test)]
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

mod page_names;
pub(crate) use page_names::{real_page_names, RealPageNames};

/// The alias relation of one generation, indexed once: normalized name to its
/// alias-adjacent normalized names, and to the spellings it was written with.
/// A component walk then costs O(component), not O(every alias in the graph)
/// (I-25); a save that moves no alias carries the whole value by `Arc`.
#[derive(Default)]
pub(crate) struct AliasEdges {
    neighbors: std::collections::HashMap<String, Vec<String>>,
    originals: std::collections::HashMap<String, Vec<String>>,
}

impl AliasEdges {
    pub(crate) fn new(aliases: &[(String, String)]) -> Self {
        let mut edges = Self::default();
        for (alias, owner) in aliases {
            let alias_norm = refs::page_key(alias);
            let owner_norm = refs::page_key(owner);
            edges
                .neighbors
                .entry(alias_norm.clone())
                .or_default()
                .push(owner_norm.clone());
            edges
                .neighbors
                .entry(owner_norm.clone())
                .or_default()
                .push(alias_norm.clone());
            edges
                .originals
                .entry(alias_norm)
                .or_default()
                .push(alias.clone());
            edges
                .originals
                .entry(owner_norm)
                .or_default()
                .push(owner.clone());
        }
        edges
    }
}

/// Resolve a requested page/alias to its canonical display name, the complete
/// alias-connected component, and the real page to exclude as self. The
/// normalized component is shared by backlinks, unlinked references, and their
/// scoped-invalidation predicates so those paths cannot drift.
fn equivalent_page_names(
    real_pages: &RealPageNames,
    edges: &AliasEdges,
    target: &str,
) -> (String, Vec<String>, String) {
    let target_norm = refs::page_key(target);
    let mut component = std::collections::BTreeSet::new();
    let mut pending = vec![target_norm.clone()];
    while let Some(name) = pending.pop() {
        if !component.insert(name.clone()) {
            continue;
        }
        if let Some(adjacent) = edges.neighbors.get(&name) {
            pending.extend(adjacent.iter().cloned());
        }
    }

    let canonical = component
        .iter()
        .filter_map(|name| real_pages.get(name).map(|(_, stored)| stored))
        .min()
        .cloned()
        .or_else(|| {
            // No real page in the component: the least spelling any member
            // was written with, including the requested one.
            std::iter::once(target)
                .chain(
                    component
                        .iter()
                        .filter_map(|name| edges.originals.get(name))
                        .flatten()
                        .map(String::as_str),
                )
                .min()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| target.to_string());
    let self_page = real_pages
        .get(&target_norm)
        .map(|(_, stored)| stored.clone())
        .unwrap_or_else(|| canonical.clone());
    (canonical, component.into_iter().collect(), self_page)
}

fn graph_equivalent_page_names(
    graph: &impl GraphRead,
    target: &str,
) -> (String, Vec<String>, String) {
    let mut resolved = equivalent_page_names(&real_page_names(graph), &graph.alias_edges(), target);
    let format = journal_format(graph.config());
    let Some(target_day) = format.parse(target) else {
        return resolved;
    };
    // O(journals) name parses, only for a date-shaped target.
    let pages = graph.page_list_arc();
    let Some(journal) = pages.iter().find(|entry| {
        entry.kind == PageKind::Journal && format.parse(&entry.name) == Some(target_day)
    }) else {
        return resolved;
    };
    resolved
        .1
        .extend(journal_spelling_keys(&format, target_day, &journal.name));
    resolved.1.sort();
    resolved.1.dedup();
    resolved.0 = journal.name.clone();
    resolved.2 = journal.name.clone();
    resolved
}

/// The graph's journal date format (title/file patterns from its config).
pub(crate) fn journal_format(config: &tine_core::config::Config) -> JournalFormat {
    JournalFormat::new(
        config.journal_file_name_format.as_deref(),
        config.journal_page_title_format.as_deref(),
    )
}

/// Every spelling of a journal day a link may use to reach that journal (GH #481,
/// master ee7730b48): its own name, the graph's title/file forms, the default
/// title/file forms and ISO `yyyy-MM-dd`, as page keys. The one definition both
/// query-time resolution and scoped invalidation use (I-12).
fn journal_spelling_keys(
    format: &JournalFormat,
    day: JournalDate,
    journal_name: &str,
) -> Vec<String> {
    [
        journal_name.to_string(),
        format.title(day),
        format.file_stem(day),
        day.title(),
        day.file_stem(),
        format!("{:04}-{:02}-{:02}", day.year, day.month, day.day),
    ]
    .iter()
    .map(|spelling| refs::page_key(spelling))
    .collect()
}

/// Scoped invalidation cannot see which journal pages exist, so a date-shaped
/// target is widened by every accepted spelling of its day whether or not that
/// journal exists: it may evict an unaffected entry but never keep a stale one.
fn widen_for_journal_day(names_norm: &mut Vec<String>, format: &JournalFormat, target: &str) {
    if let Some(day) = format.parse(target) {
        names_norm.extend(journal_spelling_keys(format, day, target));
        names_norm.sort();
        names_norm.dedup();
    }
}

/// Project only parser-owned page properties into native block syntax. Keeping
/// the whole Org drawer preserves parser ownership for reference evidence.
pub(crate) fn page_property_raw(pre: &str, is_org: bool) -> String {
    let entries = page_property_lines(pre, is_org);
    if entries.is_empty() {
        return String::new();
    }
    if is_org {
        format!(
            ":PROPERTIES:\n{}\n:END:",
            entries
                .iter()
                .map(|(key, value)| format!(":{key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        entries
            .iter()
            .map(|(key, value)| format!("{key}:: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn property_projection(raw: &str, is_org: bool) -> DocBlock {
    let mut block = DocBlock::new(raw);
    block.set_org(is_org);
    block
}

fn page_property_block(entry: &PageEntry, pre: &str) -> Option<DocBlock> {
    let is_org = Format::from_path(&entry.path) == Format::Org;
    let raw = page_property_raw(pre, is_org);
    if raw.is_empty() {
        return None;
    }
    let mut block = property_projection(&raw, is_org);
    block.uuid = format!(
        "page-property:{:?}:{}",
        entry.kind,
        refs::page_key(&entry.name)
    );
    Some(block)
}

/// The header pre-block as a block, from the document alone (no page entry).
/// I-12: the one projection of "the pre-block is a real block with `:block/refs`"
/// for the walkers that have only a `Document` (block-ref badge counts, scoped
/// referrer invalidation); the entry-taking `page_property_block` builds the
/// same block with a page-scoped identity for the DTO-producing walkers.
pub(crate) fn document_page_property_block(doc: &Document) -> Option<DocBlock> {
    let pre = doc.pre_block.as_deref()?;
    let is_org = page_document_is_org(doc);
    let raw = page_property_raw(pre, is_org);
    if raw.is_empty() {
        return None;
    }
    Some(property_projection(&raw, is_org))
}

/// The header pre-block when it references block `uuid`, as a page-property row.
fn page_property_referrer(entry: &PageEntry, pre: &str, uuid: &str) -> Option<BlockDto> {
    let block = page_property_block(entry, pre)?;
    if !block.projection().block_refs().iter().any(|r| r == uuid) {
        return None;
    }
    let mut dto = block_to_shallow_dto(&block);
    dto.page_property = true;
    Some(dto)
}

/// Parser-owned explicit page-reference targets contributed by one physical
/// cached page. This is the projection used by the reconstructible candidate
/// index; query-time occurrence verification still uses the full evidence
/// engine below and remains authoritative.
pub(crate) fn document_explicit_reference_names(entry: &PageEntry, doc: &Document) -> Vec<String> {
    fn collect(blocks: &[DocBlock], names: &mut Vec<String>) {
        for block in blocks {
            names.extend(
                block
                    .projection()
                    .reference_source()
                    .explicit
                    .iter()
                    .map(|reference| refs::page_key(&reference.name)),
            );
            collect(&block.children, names);
        }
    }

    let mut names = Vec::new();
    if let Some(block) = doc
        .pre_block
        .as_deref()
        .and_then(|pre| page_property_block(entry, pre))
    {
        names.extend(
            block
                .projection()
                .reference_source()
                .explicit
                .iter()
                .map(|reference| refs::page_key(&reference.name)),
        );
    }
    collect(&doc.roots, &mut names);
    names.sort_unstable();
    names.dedup();
    names
}

fn block_reference_evidence(
    block: &DocBlock,
    canonical: &str,
    names: &tine_core::reference_evidence::ReferenceNeedles<'_>,
    kind: ReferenceKind,
    config: &tine_core::config::Config,
) -> Option<ReferenceBlockEvidence> {
    let result = tine_core::reference_evidence::occurrences_of_kind_prepared(
        block.raw(),
        block.projection().reference_source(),
        canonical,
        names,
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

fn block_has_reference(
    block: &DocBlock,
    names: &tine_core::reference_evidence::ReferenceNeedles<'_>,
    kind: ReferenceKind,
    config: &tine_core::config::Config,
) -> bool {
    tine_core::reference_evidence::has_occurrence_prepared(
        block.raw(),
        block.projection().reference_source(),
        names,
        kind,
        config,
    )
}

#[cfg(test)]
fn collect_reference_occurrences(
    graph: &impl GraphRead,
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
    graph: &impl GraphRead,
    canonical: &str,
    self_page: &str,
    names_norm: &[String],
    kind: ReferenceKind,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let config = graph.config();
    let exclude = refs::ReferenceSourceExclusions::new(self_page, config.favorites_page.as_deref());
    let mut budget = ConstructionBudget::new(max_rows, max_bytes);
    // Folded once per query, shared by every block (GH #623, I-25).
    let needles = tine_core::reference_evidence::ReferenceNeedles::new(names_norm);
    let candidate_pages = graph.reference_candidate_pages(names_norm, kind);
    let groups = {
        let pages = candidate_pages.pages.as_slice();
        let mut groups: Vec<(Option<i64>, RefGroup)> = Vec::new();
        let mut by_name = std::collections::HashMap::<String, usize>::new();
        let mut sources = (0..pages.len()).collect::<Vec<_>>();
        sources.sort_by(|&a, &b| pages[a].0.path.cmp(&pages[b].0.path));
        for page in sources {
            let (entry, doc) = &pages[page];
            if exclude.excludes_name(&entry.name) {
                continue;
            }
            let mut blocks = Vec::new();
            let mut evidence = Vec::new();
            if let Some(mut block) = doc
                .pre_block
                .as_deref()
                .filter(|_| candidate_pages.admits(page, 0))
                .and_then(|pre| page_property_block(entry, pre))
            {
                if budget.closed() {
                    if block_has_reference(&block, &needles, kind, config) {
                        budget.deny_match();
                    }
                } else if let Some(hit) =
                    block_reference_evidence(&block, canonical, &needles, kind, config)
                {
                    let mut dto = block_to_shallow_dto(&block);
                    dto.page_property = true;
                    let estimated = tine_core::model::block_dto_estimated_bytes(&dto)
                        .saturating_add(reference_evidence_estimated_bytes(&hit));
                    if budget.admit_estimated(&entry.name, estimated) {
                        blocks.push(dto);
                        evidence.push(hit);
                    }
                }
                block.children.clear();
            }
            let mut path = Vec::new();
            let mut found: Vec<(BlockDto, ReferenceBlockEvidence)> = Vec::new();
            let construction_closed = std::cell::Cell::new(budget.closed());
            // Pre-order ordinal of the block being classified (0 is the page
            // properties), the key into the page's block signatures.
            let ordinal = std::cell::Cell::new(0usize);
            collect_reference_matches(
                &doc.roots,
                &mut path,
                &mut |block, _| {
                    ordinal.set(ordinal.get() + 1);
                    if !candidate_pages.admits(page, ordinal.get()) {
                        None
                    } else if construction_closed.get() {
                        block_has_reference(block, &needles, kind, config).then_some(None)
                    } else {
                        block_reference_evidence(block, canonical, &needles, kind, config).map(Some)
                    }
                },
                &mut |block, ancestors, hit| {
                    let Some(hit) = hit else {
                        budget.deny_match();
                        construction_closed.set(true);
                        return None;
                    };
                    let estimated = shallow_dto_estimated_bytes(block, ancestors)
                        .saturating_add(reference_evidence_estimated_bytes(&hit));
                    if !budget.admit_estimated(&entry.name, estimated) {
                        construction_closed.set(true);
                        return None;
                    }
                    construction_closed.set(budget.closed());
                    let mut dto = result_dto(block);
                    dto.breadcrumb = ancestors
                        .iter()
                        .map(|ancestor| crumb_line(ancestor))
                        .collect();
                    Some((dto, hit))
                },
                &mut found,
            );
            for (dto, hit) in found {
                blocks.push(dto);
                evidence.push(hit);
            }
            if !blocks.is_empty() {
                let key = refs::normalize(&entry.name);
                if let Some(&index) = by_name.get(&key) {
                    let (date_key, group) = &mut groups[index];
                    *date_key = match (*date_key, entry.date_key) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (current @ Some(_), None) => current,
                        (None, other) => other,
                    };
                    group.blocks.extend(blocks);
                    group.evidence.extend(evidence);
                } else {
                    by_name.insert(key, groups.len());
                    groups.push((
                        entry.date_key,
                        RefGroup {
                            page: entry.name.clone(),
                            kind: entry.kind,
                            blocks,
                            evidence,
                        },
                    ));
                }
            }
        }
        groups.sort_by(|a, b| {
            b.0.unwrap_or(i64::MIN)
                .cmp(&a.0.unwrap_or(i64::MIN))
                .then_with(|| a.1.page.cmp(&b.1.page))
        });
        groups.into_iter().map(|(_, group)| group).collect()
    };
    BoundedGroups {
        groups,
        total: budget.total,
        exceeded: budget.exceeded,
    }
}

#[cfg(test)]
pub(crate) fn backlinks(graph: &impl GraphRead, target: &str) -> Vec<RefGroup> {
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, target);
    collect_reference_occurrences(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Explicit,
    )
}

pub(crate) fn backlinks_bounded(
    graph: &impl GraphRead,
    target: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, target);
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

const BACKLINK_FILTER_MAX_BYTES: usize = 16 * 1024 * 1024;
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

fn backlink_filter_entry(
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
        text: &mut String,
        max_text: usize,
        add_facet: &mut impl FnMut(&str),
        truncated: &mut bool,
    ) {
        *truncated |= append_bounded_text(text, block.visible_text(), max_text);
        let projection = block.projection();
        for name in projection.refs_page() {
            add_facet(name);
        }
        if let Some(marker) = projection.marker().as_deref() {
            add_facet(marker);
        }
        // OG treats tags::/alias:: property values as page references too. The
        // property boundary itself is parser-owned; only its comma-separated
        // semantic values are unwrapped here.
        for (key, value) in projection.properties() {
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
            for value in value.split(tine_core::refs::is_linkable_property_separator) {
                let name = strip_ref(value.trim());
                add_facet(&name);
            }
        }
        for child in &block.children {
            visit(child, text, max_text, add_facet, truncated);
        }
    }

    let mut text_truncated = false;
    visit(
        block,
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

/// Build search/facet metadata only for the shallow backlink roots already in
/// one rendered panel. This deliberately does not rerun backlink selection and
/// cannot turn into a graph-sized arbitrary export: the request is ID-scoped,
/// de-duplicated, and the response has both per-root and total byte ceilings.
pub(crate) fn backlink_filter_context(
    graph: &impl GraphRead,
    target: &str,
    targets: &[BacklinkFilterTarget],
) -> BacklinkFilterContext {
    let (_, names_norm, _) = graph_equivalent_page_names(graph, target);
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
                        let estimated = entry.text.len()
                            + entry.facets.iter().map(String::len).sum::<usize>()
                            + entry.page.len()
                            + entry.block_id.len()
                            + 128;
                        if bytes.saturating_add(estimated) > BACKLINK_FILTER_MAX_BYTES {
                            context.truncated = true;
                        } else {
                            bytes += estimated;
                            // Same flag propagation as the ordinary-root loop
                            // below: an entry clipped at its own text/facet
                            // budget marks the context (master 39791fba2e8f).
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
                let estimated = entry.text.len()
                    + entry.facets.iter().map(String::len).sum::<usize>()
                    + entry.page.len()
                    + entry.block_id.len()
                    + 128;
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
#[cfg(test)]
pub(crate) fn block_referrers(graph: &impl GraphRead, uuid: &str) -> Vec<RefGroup> {
    let u = uuid.trim();
    if u.is_empty() {
        return Vec::new();
    }
    collect(
        graph,
        |b| b.projection().block_refs().iter().any(|r| r == u),
        |entry, pre| page_property_referrer(entry, pre, u),
        None,
    )
}

pub(crate) fn block_referrers_bounded(
    graph: &impl GraphRead,
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
    collect_bounded(
        graph,
        |b| b.projection().block_refs().iter().any(|r| r == u),
        |entry, pre| page_property_referrer(entry, pre, u),
        None,
        max_rows,
        max_bytes,
    )
}

/// Unlinked references: parser-visible plain occurrences outside explicit
/// reference syntax. A block containing both kinds appears once in each surface,
/// with the corresponding occurrence evidence.
#[cfg(test)]
pub(crate) fn unlinked_refs(graph: &impl GraphRead, target: &str) -> Vec<RefGroup> {
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, target);
    collect_reference_occurrences(
        graph,
        &canonical,
        &self_page,
        &names_norm,
        ReferenceKind::Plain,
    )
}

pub(crate) fn unlinked_refs_bounded(
    graph: &impl GraphRead,
    target: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    let (canonical, names_norm, self_page) = graph_equivalent_page_names(graph, target);
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

#[cfg(test)]
pub(crate) fn run_query(graph: &impl GraphRead, query_src: &str) -> Vec<RefGroup> {
    run_query_bounded(graph, query_src, usize::MAX, usize::MAX).groups
}

/// Run a simple query over in-memory blocks ([`exec`]); output limits do not
/// bound graph traversal. Refused/malformed input returns empty groups, total
/// 0, exceeded false, like no matches. No disk read.
pub(crate) fn run_query_bounded(
    graph: &impl GraphRead,
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
) -> BoundedGroups {
    if admit_source(query_src).is_err() {
        return BoundedGroups {
            groups: Vec::new(),
            total: 0,
            exceeded: false,
        };
    }
    exec::run_query_bounded(graph, query_src, max_rows, max_bytes).0
}

// --- Scoped-invalidation support (#52) --------------------------------------
// "Could an edit to page (entry, doc) change this derived result?" These use
// the match predicates; result-level sampling and source admission can differ.

pub(crate) fn page_affects_backlinks(
    real_pages: &RealPageNames,
    aliases: &AliasEdges,
    journal: &JournalFormat,
    target: &str,
    entry: &PageEntry,
    doc: &Document,
) -> bool {
    let (canonical, mut names_norm, _) = equivalent_page_names(real_pages, aliases, target);
    widen_for_journal_day(&mut names_norm, journal, target);
    // Scoped invalidation has no Graph/config parameter. Default-enabled matching
    // is conservative for disabled/excluded property pages (it may evict an
    // unaffected cache entry, but cannot retain a stale one).
    let config = tine_core::config::Config::default();
    let needles = tine_core::reference_evidence::ReferenceNeedles::new(&names_norm);
    if doc.pre_block.as_deref().is_some_and(|pre| {
        page_property_block(entry, pre).is_some_and(|block| {
            block_reference_evidence(
                &block,
                &canonical,
                &needles,
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
            && block_reference_evidence(b, &canonical, &needles, ReferenceKind::Explicit, &config)
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
    aliases: &AliasEdges,
    journal: &JournalFormat,
    target: &str,
    entry: &PageEntry,
    doc: &Document,
) -> bool {
    let (canonical, mut names_norm, _) = equivalent_page_names(real_pages, aliases, target);
    widen_for_journal_day(&mut names_norm, journal, target);
    let config = tine_core::config::Config::default();
    let needles = tine_core::reference_evidence::ReferenceNeedles::new(&names_norm);
    if doc.pre_block.as_deref().is_some_and(|pre| {
        page_property_block(entry, pre).is_some_and(|block| {
            block_reference_evidence(&block, &canonical, &needles, ReferenceKind::Plain, &config)
                .is_some()
        })
    }) {
        return true;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && block_reference_evidence(b, &canonical, &needles, ReferenceKind::Plain, &config)
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
    if document_page_property_block(doc).is_some_and(|block| {
        block
            .projection()
            .block_refs()
            .iter()
            .any(|reference| reference == uuid)
    }) {
        return true;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |block| {
        if !hit
            && block
                .projection()
                .block_refs()
                .iter()
                .any(|reference| reference == uuid)
        {
            hit = true;
        }
    });
    hit
}

pub(crate) fn rejected_advanced_query(reason: &str) -> AdvancedResult {
    AdvancedResult {
        groups: Vec::new(),
        ran: Vec::new(),
        ignored: vec![reason.to_string()],
        supported: false,
    }
}

/// Run an advanced `[:find … :where …]` / `{:query … :inputs …}` query: the
/// join-free pattern subset (#542) lowered by `tine_core::query` and executed
/// by [`exec`]. Unrecognized clauses are listed in `ignored` and the query is
/// unsupported, never guessed (a wrong result is worse than "unsupported").
#[cfg(test)]
pub(crate) fn run_advanced_query(graph: &impl GraphRead, query_src: &str) -> AdvancedResult {
    run_advanced_query_bounded(graph, query_src, usize::MAX, usize::MAX).0
}

/// Run supported advanced clauses over graph blocks; output limits do not bound traversal.
/// Return result, exceeded flag, attempted rows. AdvancedResult reports refusal/unsupported forms.
pub(crate) fn run_advanced_query_bounded(
    graph: &impl GraphRead,
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
) -> (AdvancedResult, bool, usize) {
    if let Err(reason) = admit_source(query_src) {
        let message = match reason {
            tine_core::query::SourceRefusal::TooLarge => "query-too-large",
            tine_core::query::SourceRefusal::TooDeep => "query-nesting-too-deep",
        };
        return (rejected_advanced_query(message), false, 0);
    }
    exec::run_advanced_query_bounded(graph, query_src, max_rows, max_bytes).0
}

/// Literal fuzzy full-text autocomplete for the `((` block picker, grouped by
/// page and capped at `limit` total blocks. Ctrl-K uses `run_graph_search*` and
/// retains the shared search dialect through `QueryPlan::friendly*`.
#[cfg(test)]
pub(crate) fn search(graph: &impl GraphRead, query: &str, limit: usize) -> Vec<RefGroup> {
    search_cancellable(graph, query, limit, || false)
}

/// Search with cooperative cancellation for interactive callers. The cheap
/// callback is checked before each block projection, so a superseded rare-prefix
/// scan does not finish walking a huge page in the background.
#[cfg(test)]
pub(crate) fn search_cancellable(
    graph: &impl GraphRead,
    query: &str,
    limit: usize,
    cancelled: impl Fn() -> bool,
) -> Vec<RefGroup> {
    search_cancellable_result(graph, query, limit, cancelled).unwrap_or_default()
}

pub(crate) fn search_cancellable_result(
    graph: &impl GraphRead,
    query: &str,
    limit: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<RefGroup>> {
    let plan = crate::query_plan::QueryPlan::block_search_literal_with_policy(
        query,
        limit,
        graph.config().enable_search_remove_accents,
    );
    let execution = plan.execute(graph, cancelled);
    if execution.cancelled {
        None
    } else {
        Some(crate::query_plan::block_hits_to_groups(execution.hits))
    }
}

/// Scan all graph blocks for templates and return insertion DTOs; do not insert.
/// Include roots unless template-including-parent is false; strip id properties
/// and included roots' template property. O(graph blocks plus copied subtrees).
pub(crate) fn templates(graph: &impl GraphRead) -> Vec<TemplateDto> {
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

/// Convert a template block subtree to a DTO, dropping `id::` (so inserted
/// copies get fresh ids) and, at the root, the `template*` properties.
fn template_dto(b: &DocBlock, strip_template: bool) -> BlockDto {
    let raw = b
        .projection()
        .regions
        .apply(
            b.raw(),
            b.is_org(),
            tine_core::block_regions::Edit::StripCopy {
                template: strip_template,
            },
        )
        .expect("parsed template");
    BlockDto {
        id: String::new(),
        raw,
        collapsed: false,
        children: b.children.iter().map(|c| template_dto(c, false)).collect(),
        breadcrumb: Vec::new(),
        ..Default::default()
    }
}

#[cfg(test)]
#[test]
fn template_copy_drops_markdown_and_org_ids_regardless_of_case() {
    let block = DocBlock::new("body\nID:: md-id\n:PROPERTIES:\n:Id: org-id\n:END:\nkeep:: yes");
    let copied = template_dto(&block, false);
    assert_eq!(copied.raw, "body\n:PROPERTIES:\n:END:\nkeep:: yes");
}

/// Distinct property keys (each with its sorted distinct values) used across the
/// graph. Drives the query builder's property-filter pickers.
pub(crate) fn property_facets_bounded(
    graph: &impl GraphRead,
    max_values: usize,
    max_bytes: usize,
) -> (Vec<(String, Vec<String>)>, bool) {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    let mut values = 0usize;
    let mut bytes = 0usize;
    let mut exceeded = false;
    let facets = graph.with_pages(|pages| {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        // Returns false once the budget is exhausted.
        let mut offer = |k: String, v: String| -> bool {
            let k = property_key_norm(&k);
            if tine_core::query::internal_property_keys()
                .iter()
                .any(|p| property_key_norm(p) == k)
            {
                return true;
            }
            if v.trim().is_empty() {
                return true;
            }
            if map.get(&k).is_some_and(|set| set.contains(&v)) {
                return true;
            }
            let key_bytes = if map.contains_key(&k) {
                0
            } else {
                k.len() + 64
            };
            let next_bytes = bytes
                .saturating_add(key_bytes)
                .saturating_add(v.len())
                .saturating_add(64);
            if values >= max_values || next_bytes > max_bytes {
                exceeded = true;
                return false;
            }
            values += 1;
            bytes = next_bytes;
            map.entry(k).or_default().insert(v);
            true
        };
        'pages: for (_entry, doc) in pages {
            // OG parity (#7): the header pre-block is a block whose properties
            // are among the graph's property names and values.
            for (k, v) in page_facets(doc).0 {
                if !offer(k, v) {
                    break 'pages;
                }
            }
            if !walk_until(&doc.roots, &mut |b| {
                for (k, v) in b.properties() {
                    if !offer(k, v) {
                        return false;
                    }
                }
                true
            }) {
                break;
            }
        }
        map.into_iter()
            .map(|(k, vs)| (k, vs.into_iter().collect()))
            .collect()
    });
    (facets, exceeded)
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

/// Collect visible keys and distinct nonempty values from page preblocks and
/// blocks. Limits charge both keys and values plus estimated bytes. Return sorted
/// facets and a flag when the next distinct item cannot fit; stop at first
/// excess. Otherwise may scan all graph blocks. No disk read.
pub(crate) fn autocomplete_property_facets_bounded(
    graph: &impl GraphRead,
    max_items: usize,
    max_bytes: usize,
) -> (Vec<(String, Vec<String>)>, bool) {
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    let hidden: HashSet<String> = OG_AUTOCOMPLETE_HIDDEN_PROPS
        .iter()
        .map(|key| property_key_norm(key))
        .chain(
            graph
                .config()
                .block_hidden_properties
                .iter()
                .map(|key| property_key_norm(key.trim_start_matches(':'))),
        )
        .collect();
    let mut items = 0usize;
    let mut bytes = 0usize;
    let exceeded = std::cell::Cell::new(false);

    let facets = graph.with_pages(|pages| {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut offer = |source_key: String, source_value: String| {
            let key = property_key_norm(&source_key);
            if key.is_empty() || hidden.contains(&key) {
                return;
            }

            if !map.contains_key(&key) {
                let key_bytes = key.len().saturating_add(64);
                if items >= max_items || bytes.saturating_add(key_bytes) > max_bytes {
                    exceeded.set(true);
                    return;
                }
                items += 1;
                bytes = bytes.saturating_add(key_bytes);
                map.insert(key.clone(), BTreeSet::new());
            }

            let value = source_value.trim();
            if value.is_empty() || map.get(&key).is_some_and(|values| values.contains(value)) {
                return;
            }
            let value_bytes = value.len().saturating_add(64);
            if items >= max_items || bytes.saturating_add(value_bytes) > max_bytes {
                exceeded.set(true);
                return;
            }
            items += 1;
            bytes = bytes.saturating_add(value_bytes);
            map.entry(key).or_default().insert(value.to_string());
        };

        for (_entry, doc) in pages {
            for (key, value) in page_facets(doc).0 {
                offer(key, value);
                if exceeded.get() {
                    break;
                }
            }
            if exceeded.get() {
                break;
            }
            if !walk_until(&doc.roots, &mut |block| {
                for (key, value) in block.properties() {
                    offer(key, value);
                    if exceeded.get() {
                        return false;
                    }
                }
                true
            }) {
                break;
            }
        }

        map.into_iter()
            .map(|(key, values)| (key, values.into_iter().collect()))
            .collect()
    });
    (facets, exceeded.get())
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
pub(crate) fn quick_switch(graph: &impl GraphRead, query: &str, limit: usize) -> Vec<PageEntry> {
    let plan = crate::query_plan::QueryPlan::legacy_page_search_with_policy(
        query,
        limit,
        graph.config().enable_search_remove_accents,
    );
    let execution = plan.execute(graph, || false);
    crate::query_plan::page_hits_to_entries(execution.hits)
}

/// Resolve many `((uuid))` block references in a single graph pass — the real
/// batch behind `Graph::resolve_blocks` (a page full of refs/embeds is one IPC,
/// and now one scan rather than U independent `resolve_block` calls, each of which
/// could whole-graph-scan on a hint miss). Hinted ids are grouped by page and each
/// hinted page is walked ONCE for all of its ids; whatever a hint missed (stale or
/// absent) falls back to a SINGLE whole-graph scan. Match semantics + first-block-
/// wins ordering are identical to `resolve_block`. Output is positional and
/// per-input (duplicate input uuids each get their own `Some(..)`/`None`).
#[cfg(test)]
pub(crate) fn resolve_blocks(graph: &impl GraphRead, uuids: &[String]) -> Vec<Option<RefGroup>> {
    resolve_blocks_bounded(graph, uuids, usize::MAX, usize::MAX).0
}

pub(crate) fn resolve_blocks_bounded(
    graph: &impl GraphRead,
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
                .admit_estimated(
                    &group.page,
                    tine_core::model::block_dto_estimated_bytes(block),
                )
                .then(|| group.clone())
        })
        .collect();
    (
        output,
        resolved_budget.exceeded || output_budget.exceeded,
        output_budget.total,
    )
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

/// Evaluate and hydrate several Copy / Export query macros under one cumulative
/// root, node, and byte budget. Only the selected block subtrees are cloned into
/// DTOs; complete PageDto values never cross IPC or accumulate in the WebView.
///
/// `max_roots` is deliberately global, not per macro. This keeps a selection
/// containing many distinct query blocks from multiplying the same advertised
/// export limit. Each relevant source document is scanned at most once and only
/// references to the requested roots are retained while the graph snapshot is
/// borrowed.
pub(crate) fn export_query_subtrees(
    graph: &impl GraphRead,
    specs: &[QueryExportSpec],
    max_queries: usize,
    max_roots: usize,
    max_nodes: usize,
    max_bytes: usize,
) -> QueryExportBatch {
    let query_limit = max_queries.max(1);
    let mut remaining_roots = max_roots.max(1);
    let mut selected = Vec::new();

    // Evaluate one query at a time and retain only at most `max_roots` identities
    // across the whole session. Cached shallow results may be larger, but they are
    // dropped before the next query and never become complete page trees.
    for spec in specs.iter().take(query_limit) {
        const QUERY_EXPORT_CONSTRUCTION_ROWS: usize = 20_000;
        const QUERY_EXPORT_CONSTRUCTION_BYTES: usize = 32 * 1024 * 1024;
        // One answerer (I-12): the caller's `advanced` flag is not trusted.
        let bounded = if tine_core::query::is_advanced(&spec.query) {
            let (result, exceeded, total) = run_advanced_query_bounded(
                graph,
                &spec.query,
                QUERY_EXPORT_CONSTRUCTION_ROWS,
                QUERY_EXPORT_CONSTRUCTION_BYTES,
            );
            BoundedGroups {
                groups: result.groups,
                total,
                exceeded,
            }
        } else {
            run_query_bounded(
                graph,
                &spec.query,
                QUERY_EXPORT_CONSTRUCTION_ROWS,
                QUERY_EXPORT_CONSTRUCTION_BYTES,
            )
        };
        let total = bounded.total;
        let mut roots = Vec::new();
        // Do not emit a wrongly ordered prefix of a globally-sorted query. A
        // query over the construction ceiling is disclosed as entirely omitted;
        // ordinary bounded queries still retain the existing first-N export.
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

    let results = graph.with_pages(|pages| {
        use std::collections::{HashMap, HashSet};

        let mut wanted_by_page: HashMap<(PageKind, String), HashSet<String>> = HashMap::new();
        for query in &selected {
            for root in &query.roots {
                wanted_by_page
                    .entry((root.kind, root.page.clone()))
                    .or_default()
                    .insert(root.id.clone());
            }
        }

        // Borrow at most `max_roots` matching blocks. Walking with an explicit
        // stack avoids both recursive call growth and variadic child spreading on
        // a page with hundreds of thousands of direct children.
        let total_wanted = wanted_by_page.values().map(HashSet::len).sum::<usize>();
        let mut found: HashMap<(PageKind, String, String), &DocBlock> = HashMap::new();
        // The page's header property block is a row of a block query (exec.rs
        // `Hit::PageProperty`) but lives in `Document::pre_block`, not in
        // `roots`: it is rebuilt here exactly as the query rebuilds it.
        let mut headers: HashMap<(PageKind, String, String), DocBlock> = HashMap::new();
        for (entry, doc) in pages {
            if found.len() + headers.len() == total_wanted {
                break;
            }
            let page_key = (entry.kind, entry.name.clone());
            let Some(wanted) = wanted_by_page.get(&page_key) else {
                continue;
            };
            if let Some(header) = doc
                .pre_block
                .as_deref()
                .and_then(|pre| page_property_block(entry, pre))
                .filter(|header| wanted.contains(header.uuid.as_str()))
            {
                headers.insert(
                    (entry.kind, entry.name.clone(), header.uuid.clone()),
                    header,
                );
            }
            let mut stack: Vec<&DocBlock> = doc.roots.iter().rev().collect();
            while let Some(block) = stack.pop() {
                let property_id = block.property("id");
                let matched = if wanted.contains(block.uuid.as_str()) {
                    Some(block.uuid.as_str())
                } else {
                    property_id.as_deref().filter(|id| wanted.contains(*id))
                };
                if let Some(id) = matched {
                    found.insert((entry.kind, entry.name.clone(), id.to_string()), block);
                    if found.len() + headers.len() == total_wanted {
                        break;
                    }
                }
                for child in block.children.iter().rev() {
                    stack.push(child);
                }
            }
        }

        let mut remaining_nodes = max_nodes.max(1);
        let mut remaining_bytes = max_bytes.max(1);
        selected
            .into_iter()
            .map(|query| {
                let mut groups: Vec<RefGroup> = Vec::new();
                let mut shown = 0usize;
                let mut omitted_nodes = 0usize;
                for root in query.roots {
                    let wanted = (root.kind, root.page.clone(), root.id.clone());
                    let Some(block) = found.get(&wanted).copied().or_else(|| headers.get(&wanted))
                    else {
                        // The graph changed between query evaluation and the
                        // borrowed hydration snapshot. Count the missing result as
                        // omitted instead of falling back to an unbounded page load.
                        omitted_nodes = omitted_nodes.saturating_add(1);
                        continue;
                    };
                    let total_nodes = subtree_node_count(block);
                    let before_nodes = remaining_nodes;
                    let dto =
                        block_to_bounded_dto(block, &mut remaining_nodes, &mut remaining_bytes);
                    let emitted = before_nodes.saturating_sub(remaining_nodes);
                    omitted_nodes =
                        omitted_nodes.saturating_add(total_nodes.saturating_sub(emitted));
                    let Some(mut dto) = dto else {
                        continue;
                    };
                    dto.page_property = headers.contains_key(&wanted);
                    shown += 1;
                    if let Some(group) = groups
                        .iter_mut()
                        .find(|group| group.kind == root.kind && group.page == root.page)
                    {
                        group.blocks.push(dto);
                    } else {
                        groups.push(RefGroup {
                            page: root.page,
                            kind: root.kind,
                            blocks: vec![dto],
                            evidence: Vec::new(),
                        });
                    }
                }
                QueryExportResult {
                    key: query.key,
                    groups,
                    shown,
                    total: query.total,
                    omitted_nodes,
                }
            })
            .collect()
    });

    QueryExportBatch {
        results,
        omitted_queries: specs.len().saturating_sub(query_limit),
    }
}

/// Node-and-byte-bounded preview used by IPC and static/export consumers. The
/// byte cap is applied while constructing the DTO, so a legal node count cannot
/// still create an unbounded structured-clone payload. If even the root cannot
/// fit, the preview is returned with an empty block list and the exact omitted
/// count; callers can disclose truncation without confusing "too large" with
/// "block not found".
pub(crate) fn preview_block_with_budget(
    graph: &impl GraphRead,
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
            if budget.closed() {
                budget.deny_match();
                return;
            }
            if budget.admit_estimated(&entry.name, shallow_dto_estimated_bytes(b, &[])) {
                let dto = result_dto(b);
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
    });
}

/// A page-ref argument's bare name: `#tag`, `[[Page]]` and `Page` alike.
fn strip_ref(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t).trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|x| x.strip_suffix("]]"))
        .unwrap_or(t);
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_advanced_query_returns_unsupported() {
        let dir =
            std::env::temp_dir().join(format!("tine-malformed-advanced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        let graph = test_snapshot(&dir);
        let result = run_advanced_query(&graph, "[:find ?b :where (");
        assert!(!result.supported);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn org_directives_resolve_alias_and_match_page_facets() {
        let dir = std::env::temp_dir().join(format!("tine-org-query-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(
            dir.join("pages/Book.org"),
            "#+TITLE: Book\n#+ALIAS: Novel\n#+tags: research, reading\n\n* chapter\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("pages/Empty.org"),
            "#+TITLE: Empty\n#+ALIAS: Vacant\n",
        )
        .unwrap();
        let (store, _, _) =
            crate::store::Store::open(&dir, crate::store::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap();
        assert!(matches!(
            graph.resolve("Novel", false),
            crate::Resolved::Alias { .. }
        ));
        assert!(matches!(
            graph.resolve("Vacant", false),
            crate::Resolved::Alias { .. }
        ));
        let snapshot = graph.test_read_snapshot();
        assert_eq!(run_query(&snapshot, "(page-property title Book)").len(), 1);
        assert_eq!(run_query(&snapshot, "(page-tags research)").len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn uppercase_template_marker_is_removed_from_inserted_copy() {
        let dir = std::env::temp_dir().join(format!("tine-template-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(
            dir.join("pages/Templates.md"),
            "- boilerplate\n  Template:: sample\n",
        )
        .unwrap();
        let graph = test_snapshot(&dir);
        let found = templates(&graph);
        assert_eq!(found.len(), 1);
        assert!(!found[0].blocks[0]
            .raw
            .to_ascii_lowercase()
            .contains("template::"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn test_snapshot(dir: &std::path::Path) -> std::sync::Arc<crate::model::ReadSnapshot> {
        let (store, _, _) =
            crate::store::Store::open(dir, crate::store::OpenOptions::default()).unwrap();
        store.whole_graph().unwrap().test_read_snapshot()
    }

    #[test]
    fn advanced_content_and_search_follow_real_graph_accent_policy() {
        use std::fs;
        let dir =
            std::env::temp_dir().join(format!("tine-query-accent-off-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:feature/enable-search-remove-accents? false}",
        )
        .unwrap();
        fs::write(dir.join("pages/Accent.md"), "- café body\n- cafe body\n").unwrap();
        let graph = test_snapshot(&dir);
        let raw = |query: &str| {
            run_query(&graph, query)
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(|block| block.raw))
                .collect::<Vec<_>>()
        };
        for query in ["\"cafe\"", "(search \"cafe\")"] {
            assert_eq!(raw(query), vec!["cafe body"], "{query}");
        }
        assert_eq!(raw("(search \"café\")"), vec!["café body"]);
        let tql_raw = |source: &str| {
            use tine_core::query::{parse_query_text, resolve_for_execution, QueryDialect};
            let (query, view) = parse_query_text(source, QueryDialect::Tql, TODAY);
            assert!(
                query.diagnostics.is_empty(),
                "{source}: {:?}",
                query.diagnostics
            );
            let resolved = resolve_for_execution(
                &query,
                &tine_core::query::ir::ExecutionContext::none(),
                TODAY,
            );
            exec::run_block_groups(&graph, &resolved, &view, usize::MAX, usize::MAX)
                .0
                .groups
                .into_iter()
                .flat_map(|group| group.blocks.into_iter().map(|block| block.raw))
                .collect::<Vec<_>>()
        };
        for source in [
            "content like '%cafe%'",
            "content like 'cafe%'",
            "content = 'cafe body'",
            "content in ('cafe body')",
            "content match 'cafe'",
        ] {
            assert_eq!(tql_raw(source), vec!["cafe body"], "{source}");
        }
        for source in ["content != 'cafe body'", "content not in ('cafe body')"] {
            assert_eq!(tql_raw(source), vec!["café body"], "{source}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // Fixed "today" so relative-date tests are deterministic: 2026-06-16.
    const TODAY: JournalDate = JournalDate {
        year: 2026,
        month: 6,
        day: 16,
    };

    fn nested_boolean(head: &str, depth: usize, leaf: &str) -> String {
        format!(
            "{}{}{}",
            format!("({head} ").repeat(depth),
            leaf,
            ")".repeat(depth)
        )
    }

    // The Pred engine's parser/evaluator unit tests are replaced by the tests
    // below, which assert the same selections through the ONE executor
    // (`exec`, over a real snapshot) instead of a private AST. The parse
    // shapes themselves are pinned by tine-core's `query/og.rs` and
    // `query/parse_tests.rs` (og 14 Q1).

    fn one_page(name: &str, journal: Option<i64>, source: &str) -> crate::model::ReadSnapshot {
        let dir = if journal.is_some() {
            "journals"
        } else {
            "pages"
        };
        let rel = format!("{dir}/{}.md", name.replace('/', "___"));
        let entry = PageEntry {
            name: name.into(),
            kind: if journal.is_some() {
                PageKind::Journal
            } else {
                PageKind::Page
            },
            date_key: journal,
            rel_path: Some(rel.as_str().into()),
            path: rel.as_str().into(),
        };
        crate::model::ReadSnapshot::from_page_snapshot(vec![(
            entry,
            std::sync::Arc::new(tine_core::doc::parse(source)),
        )])
    }

    fn block(raw: &str) -> String {
        format!("- {}\n", raw.replace('\n', "\n  "))
    }

    /// First lines of the blocks `query` (OG DSL, executed on TODAY) returns.
    fn hits_on(query: &str, name: &str, journal: Option<i64>, source: &str) -> Vec<String> {
        let graph = one_page(name, journal, source);
        exec::run_query_at(&graph, query, usize::MAX, usize::MAX, TODAY)
            .0
            .groups
            .into_iter()
            .flat_map(|group| group.blocks)
            .map(|block| block.raw.lines().next().unwrap_or("").to_string())
            .collect()
    }

    /// Whether `query` selects the one block `raw` on an ordinary page.
    fn selects(query: &str, raw: &str) -> bool {
        !hits_on(query, "Test", None, &block(raw)).is_empty()
    }

    /// Whether `query` selects the one block `raw` on the journal of `day`.
    fn selects_on_journal(query: &str, day: i64, raw: &str) -> bool {
        !hits_on(query, "Journal", Some(day), &block(raw)).is_empty()
    }

    #[test]
    fn query_parsers_fail_closed_past_the_shared_depth_and_size_limits() {
        const DEPTH: usize = 128;
        let graph = one_page("Test", None, &block("TODO x"));
        let simple_at_limit = nested_boolean("and", DEPTH - 1, "(task TODO)");
        assert_eq!(run_query(&graph, &simple_at_limit).len(), 1);
        let simple_too_deep = nested_boolean("and", DEPTH, "(task TODO)");
        assert!(run_query(&graph, &simple_too_deep).is_empty());

        let advanced_at_limit = format!(
            "[:find (pull ?b [*]) :where {}]",
            nested_boolean("and", DEPTH - 1, "(task ?b #{\"TODO\"})")
        );
        let accepted = run_advanced_query(&graph, &advanced_at_limit);
        assert!(
            accepted.supported,
            "unexpected ignored clauses: {:?}",
            accepted.ignored
        );
        assert_eq!(accepted.groups.len(), 1);
        let advanced_too_deep = format!(
            "[:find (pull ?b [*]) :where {}]",
            nested_boolean("and", DEPTH, "(task ?b #{\"TODO\"})")
        );
        let rejected = run_advanced_query(&graph, &advanced_too_deep);
        assert!(!rejected.supported);
        assert!(rejected.ran.is_empty());
        assert!(rejected
            .ignored
            .iter()
            .any(|item| item == "query-nesting-too-deep"));

        let oversized = "x".repeat(tine_core::query::QUERY_SOURCE_MAX_BYTES + 1);
        assert!(!tine_core::query::query_source_within_limit(&oversized));
        assert!(run_query(&graph, &oversized).is_empty());

        let harmless = format!("(and (content \"{}\"))", "(".repeat(DEPTH + 10));
        assert!(tine_core::query::query_nesting_within_limit(&harmless));
        let simple_semicolon = format!(";{}", "(".repeat(DEPTH + 1));
        assert!(
            !tine_core::query::query_nesting_within_limit(&simple_semicolon),
            "semicolon is ordinary text, not a comment, in the simple DSL"
        );
        let advanced_comment = format!(
            "[:find ?b :where ;; {}\n(task ?b #{{\"TODO\"}})]",
            "(".repeat(DEPTH + 1)
        );
        assert!(
            tine_core::query::query_nesting_within_limit(&advanced_comment),
            "advanced EDN comments must not count delimiter text"
        );
    }

    #[test]
    fn page_refs_tags_and_booleans_select_blocks() {
        assert!(selects("[[Foo]]", "x [[Foo]]"));
        assert!(!selects("[[Foo]]", "x [[Other]]"));
        assert!(selects("#bar", "x #bar"));
        assert!(selects("(and [[A]] [[B]])", "x [[A]] [[B]]"));
        assert!(!selects("(and [[A]] [[B]])", "x [[A]]"));
        assert!(selects("(not [[A]])", "x [[B]]"));
        assert!(!selects("(not [[A]])", "x [[A]]"));

        let task = "TODO buy milk for [[Home]]";
        assert!(selects("(task TODO)", task));
        assert!(selects("(task TODO DOING)", task));
        assert!(!selects("(task DONE)", task));
        assert!(selects("[[Home]]", task));
        assert!(selects("(and (task TODO) [[Home]])", task));
        assert!(!selects("(and (task DONE) [[Home]])", task));
        assert!(selects("(not [[Work]])", task));
    }

    #[test]
    fn property_keys_and_ref_values_match_logseq() {
        let book = "a book\ntype:: book";
        assert!(selects("(property type book)", book));
        assert!(selects("(property type)", book));
        assert!(!selects("(property type article)", book));
        // Leading `:` on the key is stripped (keyword form == symbol form).
        assert!(selects("(property :type book)", book));
        // `_` and `-` fold to one key (Logseq stores `my_key` as `my-key`).
        assert!(selects("(property my_key v)", "x\nmy-key:: v"));
        assert!(selects(
            "(property done-at 2026-07-19)",
            "shipped task\ndone_at:: 2026-07-19"
        ));
        assert!(selects(
            "(property DONE_AT)",
            "shipped task\ndone_at:: 2026-07-19"
        ));
        // `[[page]]` and `#tag` values are values, not stray page refs.
        assert!(selects(
            "(property :fach [[Foo Bar]])",
            "x\nfach:: [[Foo Bar]]"
        ));
        assert!(!selects("(property :fach [[Foo Bar]])", "x [[Foo Bar]]"));
        assert!(selects(
            "(property :type #assignment)",
            "x\ntype:: #assignment"
        ));
        // Multi-value properties match any member.
        let tagged = "x\ntags:: [[research]], optimization";
        assert!(selects("(property tags research)", tagged));
        assert!(selects("(property tags optimization)", tagged));
        assert!(!selects("(property tags cooking)", tagged));
    }

    #[test]
    fn reported_and_of_colon_properties_matches_both_clauses() {
        // GH: this used to parse to And[Property(":fach"), PageRef(X)], dropping
        // the second clause, so the query answered "No results".
        let query = r##"(and (property :fach [[Management der digitalen Transformation]]) (property :type "#assignment"))"##;
        let block = "assignment one\nfach:: [[Management der digitalen Transformation]]\ntype:: #assignment";
        assert!(selects(query, block));
        assert!(!selects(
            query,
            "assignment one\nfach:: [[Other Course]]\ntype:: #assignment"
        ));
        assert!(!selects(
            query,
            "assignment one\nfach:: [[Management der digitalen Transformation]]\ntype:: essay"
        ));
    }

    #[test]
    fn content_terms_unescape_and_match_canonical_unicode() {
        // `\"`/`\\` inside a quoted term are unescaped; any other backslash is
        // literal, so `"C:\tmp"` stays `C:\tmp`.
        assert!(selects("\"foo \\\"bar\\\"\"", "note: foo \"bar\" baz"));
        assert!(!selects("\"foo \\\"bar\\\"\"", "note: foo bar baz"));
        assert!(selects("\"a\\\\b\"", "x a\\b y"));
        assert!(selects("\"C:\\tmp\"", "open C:\\tmp now"));
        assert!(selects("\"quick brown\"", "the quick brown fox"));
        assert!(!selects("\"slow\"", "the quick brown fox"));
        // D4 (2026-10-04): canonical composition only; no accent or case fold.
        assert!(selects("\"Résumé\"", "Re\u{301}sume\u{301}"));
        assert!(!selects("\"Resume\"", "Re\u{301}sume\u{301}"));
        assert!(!selects("\"résumé\"", "Résumé"));
    }

    /// Macro arguments arrive without their source quotes after the parser has
    /// expanded `$1`. OG's simple query reader treats that bare value as a
    /// block-content term; Tine must not silently drop it from an `and` form.
    #[test]
    fn og_bare_word_is_a_content_term() {
        let query = "(and (task DONE) changelog)";
        assert!(selects(query, "DONE Write changelog for v0.0.9"));
        assert!(!selects(query, "DONE Publish release notes"));
    }

    #[test]
    fn search_predicate_preserves_escaped_friendly_source_and_evaluates_it() {
        let query = r#"(search "foo \"exact phrase\" -draft OR C:\\tmp")"#;
        assert!(selects(query, "foo and an exact phrase, ready"));
        assert!(!selects(query, "foo and an exact phrase, but draft"));
        // The decoded backslash reaches the friendly parser losslessly.
        assert!(selects(query, r"open C:\tmp\notes"));
        // It remains an ordinary composable clause.
        let task_search = r#"(and (task TODO) (search "foo -draft"))"#;
        assert!(selects(task_search, "TODO foo ready"));
        assert!(!selects(task_search, "DONE foo ready"));
    }

    #[test]
    fn content_regex_preserves_escapes_and_invalid_patterns_match_nothing() {
        let query = r#"(content-regex "ID:\\s+[A-Z]{3}\\d+\\s+\"quoted\"")"#;
        assert!(selects(query, r#"prefix ID: ABC42 "quoted" suffix"#));
        // Regex matching is case-sensitive.
        assert!(!selects(query, r#"prefix ID: abc42 "quoted" suffix"#));
        assert!(!selects(r#"(content-regex "[unclosed")"#, "[unclosed"));
    }

    #[test]
    fn aggregate_and_group_by_do_not_filter() {
        // The directives ride in the DSL so the builder round-trips; they are
        // view settings and never restrict the matches.
        for directive in [
            "(aggregate count)",
            "(aggregate sum hours)",
            "(group-by page)",
        ] {
            assert!(selects(
                &format!("(and (task TODO) {directive})"),
                "TODO ship it"
            ));
            assert!(!selects(
                &format!("(and (task DONE) {directive})"),
                "TODO ship it"
            ));
        }
    }

    #[test]
    fn advanced_datalog_outside_the_pattern_subset_is_unsupported() {
        let graph = one_page("Test", None, &block("TODO x"));
        assert!(tine_core::query::is_advanced(
            "[:find (pull ?b [*]) :where [?b :block/marker]]"
        ));
        assert!(run_query(&graph, "[:find ?b :where ...]").is_empty());
        let unrelated = run_advanced_query(
            &graph,
            r#"[:find (pull ?p [*])
                :where
                [?p :block/name ?name]
                [(get ?name :class)]]"#,
        );
        assert!(!unrelated.supported);
        assert!(unrelated.groups.is_empty());
    }

    #[test]
    fn advanced_exact_page_property_pair_matches_page_property_predicate() {
        let source = r#"[:find (pull ?p [*])
                         :where
                         [?p :block/properties ?props]
                         [(get ?props :class)]]"#;
        let with = one_page("Classy", None, "class:: yes\n\n- body\n");
        let without = one_page("Plain", None, "other:: yes\n\n- body\n");
        let result = run_advanced_query(&with, source);
        assert!(result.supported, "{:?}", result.ignored);
        assert_eq!(
            result
                .groups
                .iter()
                .map(|g| g.page.as_str())
                .collect::<Vec<_>>(),
            run_query(&with, "(page-property :class)")
                .iter()
                .map(|g| g.page.as_str())
                .collect::<Vec<_>>()
        );
        assert!(!result.groups.is_empty());
        assert!(run_advanced_query(&without, source).groups.is_empty());
    }

    #[test]
    fn between_journal_titles_and_relative_dates() {
        let q = "(between [[Jan 1st, 2021]] [[Jan 1st, 2100]])";
        assert!(selects_on_journal(q, 20220615, "TODO something"));
        assert!(!selects_on_journal(q, 20190101, "TODO something"));
        assert!(!selects(q, "TODO x\nSCHEDULED: <2022-03-03 Thu>"));
        assert!(selects(
            "(between any [[Jan 1st, 2021]] [[Jan 1st, 2100]])",
            "TODO x\nSCHEDULED: <2022-03-03 Thu>"
        ));

        // TODAY = 2026-06-16: (between -7d +7d) is [2026-06-09, 2026-06-23].
        assert!(selects_on_journal("(between -7d +7d)", 20260616, "x"));
        assert!(selects_on_journal("(between -7d +7d)", 20260609, "x"));
        assert!(selects_on_journal("(between -7d +7d)", 20260623, "x"));
        assert!(!selects_on_journal("(between -7d +7d)", 20260601, "x"));
        assert!(!selects_on_journal("(between -7d +7d)", 20260624, "x"));
        assert!(selects_on_journal(
            "(between today tomorrow)",
            20260617,
            "x"
        ));
        assert!(!selects_on_journal(
            "(between today tomorrow)",
            20260615,
            "x"
        ));
        assert!(selects_on_journal("(between -1m +1y)", 20260516, "x"));
        assert!(selects_on_journal("(between -1m +1y)", 20270616, "x"));
        assert!(!selects_on_journal("(between -1m +1y)", 20260515, "x"));
    }

    #[test]
    fn between_field_selector_and_journal_only() {
        let sched = "TODO x\nSCHEDULED: <2026-06-10 Wed>";
        // `between journal` restricts to journal pages.
        let q = "(between journal -30d today)";
        assert!(!selects(q, sched));
        assert!(selects_on_journal(q, 20260610, "TODO y"));
        assert!(selects_on_journal(q, 20260517, "TODO y"));
        assert!(!selects_on_journal(q, 20260101, "TODO z"));
        // `between scheduled` ignores the page's journal date.
        let qs = "(between scheduled -30d today)";
        assert!(selects(qs, sched));
        assert!(!selects_on_journal(qs, 20260610, "TODO y"));
        assert!(selects("(between scheduled -7d +7d)", sched));
        // `between deadline` only looks at DEADLINE lines.
        let qd = "(between deadline -30d today)";
        assert!(selects(qd, "TODO x\nDEADLINE: <2026-06-10 Wed>"));
        assert!(!selects(qd, sched));
    }

    #[test]
    fn agenda_query_keys_off_scheduled_deadline_not_journal_date() {
        // The journal-agenda DSL the app inserts must match on the planning date
        // itself, NOT the journal day the block happens to live on.
        let q = "(or (between scheduled -7d +7d) (between deadline -7d +7d))";
        assert!(!selects_on_journal(
            q,
            20260616,
            "TODO old thing\nDEADLINE: <2025-01-01 Wed>"
        ));
        assert!(selects(q, "TODO pay\nDEADLINE: <2026-06-16 Tue>"));
        assert!(selects_on_journal(
            q,
            20200101,
            "TODO meet\nSCHEDULED: <2026-06-18 Thu>"
        ));
        assert!(!selects_on_journal(q, 20260616, "just a note"));
    }

    #[test]
    fn planning_markers_are_found_anywhere_in_the_block() {
        // TODAY + 20d = 2026-07-06.
        let q = "(between scheduled +20d +20d)";
        assert!(selects(q, "TODO x\nSCHEDULED: <2026-07-06 Mon>"));
        assert!(selects(q, "TODO SCHEDULED: <2026-07-06 Mon> do it"));
        assert!(selects(
            q,
            "TODO y\n SCHEDULED: <2026-07-06 Mon> #email students"
        ));
        assert!(!selects(q, "TODO z\nDEADLINE: <2026-07-06 Mon>"));
        assert!(selects(
            "(between deadline +20d +20d)",
            "TODO z\nDEADLINE: <2026-07-06 Mon>"
        ));
    }

    #[test]
    fn a_planning_marker_in_a_fence_or_ending_a_longer_word_is_not_planning() {
        // OG-C5-Q L02 eval.rs:154: the inline fallback scanned raw bytes, so
        // documentation of the syntax scheduled the block.
        let q = "(between scheduled +20d +20d)";
        assert!(selects(q, "TODO real\nSCHEDULED: <2026-07-06 Mon>"));
        assert!(!selects(
            q,
            "TODO doc\n```\nSCHEDULED: <2026-07-06 Mon>\n```"
        ));
        assert!(!selects(q, "TODO doc UNSCHEDULED: <2026-07-06 Mon>"));
        assert!(!selects(
            "(between deadline +20d +20d)",
            "TODO doc\n#+BEGIN_SRC org\nDEADLINE: <2026-07-06 Mon>\n#+END_SRC"
        ));
        // The recorded inline-code deviation is unchanged.
        assert!(selects(q, "TODO doc `SCHEDULED: <2026-07-06 Mon>`"));
    }

    #[test]
    fn journal_predicate_and_target_query() {
        assert!(selects_on_journal("(journal)", 20260616, "TODO buy milk"));
        assert!(!selects("(journal)", "TODO buy milk"));
        // TODOs on journal pages dated in the last 30 days.
        let q = "(and (task TODO) (between journal -30d today))";
        assert!(selects_on_journal(q, 20260601, "TODO buy milk"));
        assert!(!selects_on_journal(q, 20260101, "TODO buy milk"));
        assert!(!selects_on_journal(q, 20260601, "DONE buy milk"));
        assert!(!selects(q, "TODO buy milk"));
    }

    #[test]
    fn registry_is_built_once_and_only_for_property_queries() {
        let graph = one_page("Test", None, &block("TODO item\nstatus:: open"));
        let before = super::index::registry_builds();
        assert_eq!(
            exec::run_query_at(&graph, "(task TODO)", usize::MAX, usize::MAX, TODAY)
                .0
                .groups
                .len(),
            1
        );
        assert_eq!(
            super::index::registry_builds(),
            before,
            "a task query read the registry"
        );
        for _ in 0..2 {
            let hits = exec::run_query_at(
                &graph,
                "(property status open)",
                usize::MAX,
                usize::MAX,
                TODAY,
            )
            .0;
            assert_eq!(hits.groups.len(), 1);
        }
        assert_eq!(
            super::index::registry_builds(),
            before + 1,
            "one registry per generation"
        );
    }

    #[test]
    fn config_value_keys_reach_query_matching() {
        // `:property/separated-by-commas` keeps a value's plain segments next to
        // its refs (without it OG's ref set drops them), and
        // `:ignored-page-references-keywords` keeps one as a single string.
        let matches = |edn: &str, query: &str, raw: &str| {
            let entry = PageEntry {
                name: "Test".into(),
                kind: PageKind::Page,
                date_key: None,
                rel_path: Some("pages/Test.md".into()),
                path: "pages/Test.md".into(),
            };
            let pages = vec![(
                entry,
                std::sync::Arc::new(tine_core::doc::parse(&block(raw))),
            )];
            let graph = crate::model::Graph::from_page_snapshot("", pages);
            graph.with_pages(|_| ());
            let snapshot = crate::model::ReadSnapshot::capture(
                &graph,
                tine_core::config::Config::parse(edn),
                std::sync::Arc::new(crate::model::persistent::EntryList::from(
                    graph.list_pages_shared().as_slice(),
                )),
                None,
                &[],
            );
            let groups = exec::run_query_at(&snapshot, query, usize::MAX, usize::MAX, TODAY).0;
            !groups.groups.is_empty()
        };
        let split = "{:property/separated-by-commas #{:authors}}";
        assert!(!matches(
            "{}",
            "(property authors Bob)",
            "item\nauthors:: [[Ann]], Bob"
        ));
        assert!(matches(
            split,
            "(property authors Bob)",
            "item\nauthors:: [[Ann]], Bob"
        ));
        let ignored = "{:ignored-page-references-keywords #{:topic}}";
        assert!(matches(
            "{}",
            "(property topic Rust)",
            "item\ntopic:: [[Rust]], [[Go]]"
        ));
        assert!(!matches(
            ignored,
            "(property topic Rust)",
            "item\ntopic:: [[Rust]], [[Go]]"
        ));
    }

    #[test]
    fn page_namespace_page_property_and_page_tags() {
        let alpha = |q: &str| !hits_on(q, "Project/Alpha", None, &block("hi")).is_empty();
        assert!(alpha("(page Project/Alpha)"));
        assert!(!alpha("(page Project/Beta)"));
        assert!(alpha("(namespace Project)"));
        assert!(!alpha("(namespace Other)"));

        let source = "type:: project\ntags:: research, active\n\n- hi\n";
        let on_p = |q: &str| !hits_on(q, "P", None, source).is_empty();
        assert!(on_p("(page-property type project)"));
        assert!(on_p("(page-property type)"));
        assert!(!on_p("(page-property type book)"));
        assert!(on_p("(page-property :type project)"));
        assert!(on_p("(page-tags research)"));
        assert!(!on_p("(page-tags archived)"));
    }

    #[test]
    fn sample_and_sort_by_shape_the_result() {
        let source = (1..=8)
            .map(|n| {
                let priority = ["A", "B", "C"][n % 3];
                format!("- TODO [#{priority}] item {n}\n")
            })
            .collect::<String>();
        let rows = hits_on("(and (task TODO) (sample 5))", "Test", None, &source);
        assert_eq!(rows.len(), 5);
        let sorted = hits_on(
            "(and (task TODO) (sort-by priority desc))",
            "Test",
            None,
            &source,
        );
        let priorities = sorted
            .iter()
            .map(|line| line.split("[#").nth(1).unwrap()[..1].to_string())
            .collect::<Vec<_>>();
        let mut expected = priorities.clone();
        expected.sort_by(|a, b| b.cmp(a));
        assert_eq!(priorities, expected);
        assert_eq!(sorted.len(), 8);
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

        let graph = test_snapshot(&dir);
        let runtime_id = backlinks(&graph, "Target")[0].blocks[0].id.clone();
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

    /// master 39791fba2e8f (DUP-6): a page-property root entry that hits its own
    /// facet budget must mark the whole context truncated, as the ordinary-root
    /// loop does; otherwise the reference filter presents a clipped facet list
    /// as complete.
    #[test]
    fn backlink_filter_context_propagates_page_property_entry_truncation() {
        let dir = std::env::temp_dir().join(format!(
            "tine-backlink-filter-pp-trunc-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::create_dir_all(dir.join("journals")).unwrap();
        let tags = (0..BACKLINK_FILTER_MAX_FACETS + 20)
            .map(|i| format!("facet{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            dir.join("pages/Source.md"),
            format!("tags:: Target, {tags}\n\n- body\n"),
        )
        .unwrap();
        let graph = test_snapshot(&dir);
        let block_id = format!(
            "page-property:{:?}:{}",
            PageKind::Page,
            refs::page_key("Source")
        );
        let context = backlink_filter_context(
            &graph,
            "Target",
            &[BacklinkFilterTarget {
                page: "Source".into(),
                kind: PageKind::Page,
                block_id,
            }],
        );
        assert_eq!(
            context.entries.len(),
            1,
            "the page-property root is answered"
        );
        assert!(
            context.entries[0].truncated,
            "the entry itself hit its facet budget"
        );
        assert!(
            context.truncated,
            "a truncated page-property entry must mark the context truncated"
        );
        let _ = std::fs::remove_dir_all(&dir);
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

        let graph = test_snapshot(&dir);
        let ids = run_query(&graph, "(between [[Dec 5th, 2020]] [[Dec 7th, 2020]])")
            .into_iter()
            .flat_map(|group| group.blocks.into_iter().map(persisted_dto_id))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            // Journal groups run newest day first (OG block.cljs:3497, audit #8a).
            vec![
                DEC_7_A.to_string(),
                DEC_7_B.to_string(),
                DEC_5_A.to_string(),
                DEC_5_B.to_string(),
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

        let graph = test_snapshot(&dir);
        assert_eq!(
            property_facets_bounded(&graph, usize::MAX, usize::MAX).0,
            vec![(
                "done-at".to_string(),
                vec!["one".to_string(), "two".to_string()]
            )]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// OG parity (#7, I-12): the header pre-block is a real block with
    /// `:block/refs`, so block referrers, block-ref badges and the query
    /// builder's property pickers must see it exactly like the walkers that
    /// already project it (backlinks, query execution).
    #[test]
    fn header_pre_block_is_a_block_referrer_and_counts_as_one() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-header-preblock-block-refs-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        const ID: &str = "12345678-1234-4234-8234-123456789abc";
        fs::write(
            dir.join("pages/Source.md"),
            format!("source:: (({ID}))\n\n- body\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Plain.md"),
            format!("- body points at (({ID}))\n"),
        )
        .unwrap();
        fs::write(
            dir.join("pages/Target.md"),
            format!("- target\n  id:: {ID}\n"),
        )
        .unwrap();

        let graph = test_snapshot(&dir);
        let groups = block_referrers(&graph, ID);
        let pages: Vec<(&str, usize, bool)> = groups
            .iter()
            .map(|g| {
                (
                    g.page.as_str(),
                    g.blocks.len(),
                    g.blocks.iter().any(|b| b.page_property),
                )
            })
            .collect();
        assert!(
            pages.contains(&("Source", 1, true)),
            "the header property block referencing the id is a referrer: {pages:?}"
        );
        assert!(pages.contains(&("Plain", 1, false)), "{pages:?}");
        let bounded = block_referrers_bounded(&graph, ID, usize::MAX, usize::MAX);
        assert_eq!(bounded.total, 2);

        // Badge counts and scoped invalidation use the same projection.
        let source_doc = graph
            .with_pages(|pages| {
                pages
                    .into_iter()
                    .find(|(entry, _)| entry.name == "Source")
                    .map(|(_, doc)| Document::clone(doc))
            })
            .unwrap();
        assert_eq!(
            crate::model::document_block_ref_counts(&source_doc).get(ID),
            Some(&1),
            "the pre-block counts once like any referring block"
        );
        assert!(page_affects_block_referrers(ID, &source_doc));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn property_facets_include_header_page_properties() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "tine-header-preblock-facets-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages/Header.md"),
            "tags:: Target\nstatus:: Target\n\n- first\n  rank:: 1\n",
        )
        .unwrap();

        let graph = test_snapshot(&dir);
        let (facets, exceeded) = property_facets_bounded(&graph, usize::MAX, usize::MAX);
        assert!(!exceeded);
        assert_eq!(
            facets,
            vec![
                ("rank".to_string(), vec!["1".to_string()]),
                ("status".to_string(), vec!["Target".to_string()]),
                ("tags".to_string(), vec!["Target".to_string()]),
            ],
            "OG lists page-header properties among the property names/values"
        );
        let _ = fs::remove_dir_all(&dir);
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

        let graph = test_snapshot(&dir);
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

    fn quick_switch_fingerprint(entries: Vec<PageEntry>) -> Vec<(String, PageKind, String)> {
        entries
            .into_iter()
            .map(|e| (e.name.clone(), e.kind, e.rel_path_str().to_owned()))
            .collect()
    }

    fn graph_from_page_snapshot(pages: &[(&str, &str, &str)]) -> crate::model::ReadSnapshot {
        let pages = pages
            .iter()
            .map(|(name, rel_path, source)| {
                (
                    PageEntry {
                        name: (*name).into(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: Some((*rel_path).into()),
                        path: (*rel_path).into(),
                    },
                    std::sync::Arc::new(tine_core::doc::parse(source)),
                )
            })
            .collect();
        crate::model::ReadSnapshot::from_page_snapshot(pages)
    }

    fn persisted_dto_id(block: BlockDto) -> String {
        block
            .properties
            .into_iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("id"))
            .map(|(_, value)| value)
            .expect("fixture block has persisted id::")
    }

    fn search_block_texts(graph: &impl GraphRead, query: &str, limit: usize) -> Vec<String> {
        search(graph, query, limit)
            .into_iter()
            .flat_map(|group| group.blocks.into_iter().map(|block| block.raw))
            .collect()
    }

    fn graph_search_block_texts(execution: tine_core::query_plan::QueryExecution) -> Vec<String> {
        execution
            .hits
            .into_iter()
            .filter_map(|hit| match hit {
                tine_core::query_plan::QueryHit::Block { display_text, .. } => Some(display_text),
                tine_core::query_plan::QueryHit::Page { .. } => None,
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
        let search = |source| {
            crate::query_plan::QueryPlan::friendly(source, 8, 8).execute_with_explain(
                &graph,
                || false,
                false,
            )
        };

        let or_hits = graph_search_block_texts(search("foo OR bar"));
        assert!(or_hits.iter().any(|text| text == "foo safe"));
        assert!(or_hits.iter().any(|text| text == "bar safe"));
        assert!(!or_hits.iter().any(|text| text == "unrelated"));

        let excluded = graph_search_block_texts(search("foo -x"));
        assert_eq!(excluded, ["foo safe"]);
        assert!(search("-x").hits.is_empty());

        let scoped = graph_search_block_texts(graph.run_graph_search_latest_scoped(
            &crate::store::Cancel(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            ))),
            "foo -x",
            8,
            8,
            Some(crate::query_plan::QueryPageScope {
                path: Some("pages/search.md".into()),
                name: "Search".into(),
                page_kind: PageKind::Page,
            }),
            false,
            tine_core::query::ir::FriendlyPageMatchScope::Names,
            None,
            None,
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
                result[0].rel_path.is_none(),
                "winner must be reference-only"
            );
        }
    }

    fn quick_switch_reference_full_sort(
        graph: &impl GraphRead,
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

        let graph = test_snapshot(&dir);

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

        let graph = test_snapshot(&dir);
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

        let graph = test_snapshot(&dir);
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
                        rel_path: Some(rel_path.clone().into()),
                        path: rel_path.into(),
                    },
                    std::sync::Arc::new(tine_core::doc::parse("- tied needle\n")),
                )
            })
            .collect();
        let graph = crate::model::ReadSnapshot::from_page_snapshot(pages);

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

        let graph = test_snapshot(&dir);
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

        let graph = test_snapshot(&dir);
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

        let graph = test_snapshot(&dir);
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
            // The memo's per-page invalidation test is the plan's own
            // selection, so an inherited-only page invalidates a page-ref query
            // and not a physical-page one.
            let index = graph.query_index();
            let facts = index.facts(entry, doc);
            let config = index.parse_config();
            let touches = |plan: std::sync::Arc<exec::Plan>| plan.touches(entry, doc, &facts, config);
            assert!(touches(
                exec::run_query_bounded(&graph, "(and (task TODO) [[Target]])", 20, 1 << 20).1
            ));
            assert!(!touches(
                exec::run_query_bounded(&graph, "(and (task TODO) (page \"Target\"))", 20, 1 << 20).1
            ));
            assert!(touches(
                exec::run_advanced_query_bounded(
                    &graph,
                    r#"[:find (pull ?b [*]) :where (and (task ?b #{"TODO"}) (page-ref ?b "Target"))]"#,
                    20,
                    1 << 20,
                )
                .1
                .expect("supported advanced query")
            ));
        });

        let _ = fs::remove_dir_all(&dir);
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

        let g = test_snapshot(&dir);
        let groups = backlinks(&g, "Common");
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

        let graph = test_snapshot(&dir);
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
            "alias + title + inline code"
        );
        assert!(source.evidence[0]
            .occurrences
            .iter()
            .all(|occurrence| occurrence.kind == ReferenceKind::Plain));
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

        let graph = test_snapshot(&dir);
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
            tine_core::model::ReferenceSpan { start: 0, end: 3 }
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

        let graph = test_snapshot(&dir);
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

        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
        let graph = test_snapshot(&dir);
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
            equivalent_page_names(&RealPageNames::new(), &AliasEdges::new(&aliases), "Z").0,
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

        const DEPTH: usize = crate::model::PARSE_INPUT_MAX_DEPTH;
        let dir =
            std::env::temp_dir().join(format!("tine-non-overlap-results-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        let nested = (0..DEPTH)
            .map(|depth| format!("{}- TODO [[Target]] node {depth}\n", "  ".repeat(depth)))
            .collect::<String>();
        fs::write(dir.join("pages").join("Nested.md"), nested).unwrap();

        let graph = test_snapshot(&dir);
        let page = graph
            .pages
            .iter()
            .find(|(entry, _)| entry.name == "Nested")
            .unwrap();
        let blocks: Vec<_> = page
            .1
            .roots
            .iter()
            .map(tine_core::projection::block_to_dto)
            .collect();
        let mut ids = Vec::new();
        collect_ids(&blocks, &mut ids);
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

        let preview = preview_block_with_budget(&graph, &ids[0], 50, usize::MAX).unwrap();
        assert_eq!(dto_nodes(&preview.group.blocks), 50);
        assert_eq!(preview.truncated, DEPTH - 50);

        let byte_bounded = preview_block_with_budget(&graph, &ids[0], DEPTH, 512).unwrap();
        assert!(
            byte_bounded
                .group
                .blocks
                .iter()
                .map(tine_core::model::block_dto_estimated_bytes)
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

        let graph = test_snapshot(&dir);
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

        let advanced =
            run_advanced_query(&graph, "[:find (pull ?b [*]) :where (task ?b \"TODO\")]");
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

        let graph = test_snapshot(&dir);
        let batch = export_query_subtrees(
            &graph,
            &[
                QueryExportSpec {
                    key: "todo".into(),
                    query: "(task TODO)".into(),
                },
                QueryExportSpec {
                    key: "done".into(),
                    query: "(task DONE)".into(),
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
            .map(tine_core::model::block_dto_estimated_bytes)
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

    /// A block query answers with the page's header property block as a row
    /// (exec.rs `Hit::PageProperty`); Copy / Export must hydrate that row like
    /// the query view does instead of counting it as a vanished block
    /// (checkpoint-5 L02 B3, I-12 one answerer).
    #[test]
    fn query_export_hydrates_the_header_property_row() {
        use std::fs;

        let dir =
            std::env::temp_dir().join(format!("tine-query-export-header-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("pages").join("Headed.md"),
            "status:: exported-state\n\n- an ordinary block\n",
        )
        .unwrap();
        let graph = test_snapshot(&dir);
        let batch = export_query_subtrees(
            &graph,
            &[QueryExportSpec {
                key: "header".into(),
                query: "(property status exported-state)".into(),
            }],
            8,
            50,
            100,
            1024 * 1024,
        );
        let result = &batch.results[0];
        assert_eq!(
            (result.total, result.shown, result.omitted_nodes),
            (1, 1, 0),
            "the header property row must be exported, not counted as omitted: {result:?}"
        );
        assert!(
            result.groups[0].blocks[0].raw.contains("exported-state"),
            "{result:?}"
        );
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
        let graph = test_snapshot(&dir);
        let checks = Cell::new(0usize);
        let result = search_cancellable(&graph, "never-matches", 10, || {
            checks.set(checks.get() + 1);
            checks.get() > 12
        });
        assert!(result.is_empty());
        assert!(checks.get() < 40, "cancellation checks: {}", checks.get());
        let _ = fs::remove_dir_all(&dir);
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
        let graph = test_snapshot(&dir);

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
        tine_core::reference_evidence::reset_occurrence_constructions();
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
        assert_eq!(tine_core::reference_evidence::occurrence_constructions(), 2);

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

#[cfg(test)]
mod region_template_regression {
    #[test]
    fn store_template_query_preserves_literal_metadata() {
        let root = std::env::temp_dir().join(format!("og-d1-templates-{}", std::process::id()));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::write(root.join("pages/Template.md"),"- Task\n  template:: Example\n  id:: actual\n  ```\n  id:: literal\n  template:: literal\n  ```\n").unwrap();
        let (store, _, _) = crate::Store::open(&root, crate::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap();
        let templates = graph.templates();
        let template = templates.iter().find(|t| t.name == "Example").unwrap();
        assert!(template.blocks[0]
            .raw
            .contains("```\nid:: literal\ntemplate:: literal\n```"));
        assert!(!template.blocks[0].raw.contains("id:: actual"));
        drop(graph);
        store.close();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn template_copy_keeps_literal_metadata_and_drops_org_metadata() {
        let raw = "Task\ntemplate:: Example\nid:: real\n```\nid:: literal\ntemplate:: literal\n```";
        let b = tine_core::doc::DocBlock::new(raw);
        assert_eq!(
            super::template_dto(&b, true).raw,
            "Task\n```\nid:: literal\ntemplate:: literal\n```"
        );
        let mut b = tine_core::doc::DocBlock::new(
            "Task\n:PROPERTIES:\n:template: Example\n:id: real\n:END:",
        );
        b.set_org(true);
        assert_eq!(super::template_dto(&b, true).raw, "Task");
    }
}
