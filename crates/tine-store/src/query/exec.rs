//! In-memory execution of a resolved query (SPEC §3.5, §5.9, §7.1): the ONE
//! answerer for `query_run`, `query_explain_empty`, the legacy `run_query` /
//! `run_advanced_query` bridges and Copy/Export (I-12).
//!
//! Semantics are master's (walk + results, production comparison mode):
//! - block rows: OG top-level roots per page (a match whose immediate parent
//!   matched is dropped), base order journal day newest first (non-journal pages
//!   last), then page name, kind rank and physical path, document order within a
//!   page;
//! - `sort-by` is global over single blocks, re-coalescing adjacent same-page
//!   runs; `sample` applies after sorting; admission charges each offered row
//!   and `total` counts every offered row;
//! - page rows (`@page`): pages ordered by physical path, sorted by the page
//!   decorations, sampled, then admitted at the raw page estimate (og: master
//!   admitted before sampling, refusing a sampled query over the bound);
//! - statistics fold the ordered sample, never retained rows.
//!
//! og-only: page-ref queries narrow the scanned pages through each page's
//! reference set ([`PageFacts::may_reference`]), and `and` conjuncts are
//! evaluated cheapest first. Both are evaluation-order changes only.

use std::collections::HashSet;
use std::sync::Arc;

use tine_core::date::JournalDate;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::model::{BlockDto, PageEntry, PageKind, RefGroup};
use tine_core::query::atom::ParseConfig;
use tine_core::query::ir::{
    Anchor, Attr, Bounds, CmpOp, ExecutionContext, ExplainEmptyResult, Filter, Leaf, PageRow,
    Quant, Query, QueryResult, QueryRows, Rel, SortDir, ViewSettings,
};
use tine_core::query::og::rebase_to_block;
use tine_core::query::path_refs::{PathRefCounts, PathRefVisitor};
use tine_core::query::registry::Registry;
use tine_core::query::sort::{compare_sort_decorations, lexical_property_sort_text, SortDecor};
use tine_core::query::statistics::{StatisticsFold, StatisticsResourceLimit};
use tine_core::query::view::{explain_empty_plan, statistics_execution_view};
use tine_core::query::{
    parse_query_input, parse_query_text, resolve_for_execution, AdvancedResult, QueryDialect,
    QueryInput, ResolvedQuery,
};
use tine_core::refs;

use super::eval::{self, CompiledLeaves, EvalCache, EvalCtx};
use super::index::{atom_format, PageFacts, QueryIndex};
use super::{result_dto, shallow_dto_estimated_bytes, BoundedGroups, ConstructionBudget};
use crate::model::GraphRead;

#[path = "exec_candidates.rs"]
mod candidates;

#[cfg(test)]
#[path = "exec_scope_tests.rs"]
mod scope_tests;

/// One query, ready to evaluate against pages: the anchor-adjusted evaluable
/// filter, its compiled patterns and the registry snapshot it coerces by.
pub(crate) struct Plan {
    anchor: Anchor,
    /// Whether the header page-property block is one of the page's block rows:
    /// true for a query that is block-anchored as written (OG's block
    /// queries). A `@page` query the legacy bridge lists block-wise answers
    /// with the page's outline blocks, as before.
    page_property_rows: bool,
    filter: Filter,
    compiled: CompiledLeaves,
    track: bool,
    today: JournalDate,
    remove_accents: bool,
    registry: Option<Arc<Registry>>,
    /// The page keys some page's `tags::` names, built once per plan when the
    /// filter reads `used_as_tag` (graph-wide, so the memo cannot treat such a
    /// plan as page-local: see [`Plan::reads_tag_targets`]).
    tag_targets: Option<Arc<HashSet<String>>>,
}

/// A [`Plan`]'s persisted half (launch checkpoint, ADR 0070): every field
/// but the compiled patterns.
pub(crate) struct PlanCheckpointParts<'a> {
    pub(crate) anchor: Anchor,
    pub(crate) page_property_rows: bool,
    pub(crate) filter: &'a Filter,
    pub(crate) track: bool,
    pub(crate) today: JournalDate,
    pub(crate) remove_accents: bool,
    pub(crate) registry: Option<&'a Arc<Registry>>,
    pub(crate) tag_targets: Option<&'a Arc<HashSet<String>>>,
}

impl Plan {
    pub(super) fn estimated_bytes(&self) -> usize {
        super::memo::retained::serialized_bytes(&self.filter)
            .saturating_add(self.compiled.estimated_bytes())
            .saturating_add(self.registry.as_ref().map_or(0, |registry| {
                super::memo::retained::serialized_bytes(registry.rows())
                    .saturating_add(super::memo::retained::parse_config_bytes(registry.config()))
            }))
            // The graph-wide tag-target set is owned by this plan (built once
            // per plan, not shared across memo entries), so every retained
            // entry holds its own copy of up to one key per tagged page.
            .saturating_add(self.tag_targets.as_ref().map_or(0, |targets| {
                super::memo::retained::serialized_bytes(&**targets)
            }))
            .saturating_add(std::mem::size_of::<Self>())
    }

    /// `block_rows` evaluates a `@page` query block-anchored (page attributes
    /// read through `block.page`), which is the legacy block-group bridge's
    /// semantics (master `block_anchored_filter`).
    pub(crate) fn new(
        query: &Query,
        today: JournalDate,
        block_rows: bool,
        remove_accents: bool,
        registry: impl FnOnce() -> Arc<Registry>,
        tag_targets: impl FnOnce() -> Arc<HashSet<String>>,
    ) -> Plan {
        let evaluable = query.evaluable_filter();
        let (anchor, filter) = match query.anchor {
            Anchor::Page if block_rows => (Anchor::Block, rebase_to_block(&evaluable)),
            anchor => (anchor, evaluable),
        };
        let filter = cheapest_first(filter);
        let registry = filter.has_props_leaf().then(registry);
        let tag_targets = filter
            .any_leaf(&mut |leaf| {
                matches!(
                    leaf,
                    Leaf::Attr {
                        attr: Attr::UsedAsTag,
                        ..
                    }
                )
            })
            .then(tag_targets);
        Plan {
            anchor,
            page_property_rows: query.anchor == Anchor::Block,
            compiled: CompiledLeaves::for_query(&filter),
            track: eval::uses_path_refs(&filter),
            filter,
            today,
            remove_accents,
            registry,
            tag_targets,
        }
    }

    /// The plan's data in launch-checkpoint form (ADR 0070): everything but
    /// the compiled patterns, which are a function of the filter and policy.
    /// The tag-target set is carried as captured: the memo entry holding this
    /// plan is dropped by any later edit that moves a page's `tags::`
    /// ([`Self::reads_tag_targets`]), so while the entry lives the set equals
    /// what [`Self::new`] would build from the generation it belongs to.
    pub(crate) fn checkpoint_parts(&self) -> PlanCheckpointParts<'_> {
        let Plan {
            anchor,
            page_property_rows,
            filter,
            compiled: _,
            track,
            today,
            remove_accents,
            registry,
            tag_targets,
        } = self;
        PlanCheckpointParts {
            anchor: *anchor,
            page_property_rows: *page_property_rows,
            filter,
            track: *track,
            today: *today,
            remove_accents: *remove_accents,
            registry: registry.as_ref(),
            tag_targets: tag_targets.as_ref(),
        }
    }

    /// A plan restored from [`Self::checkpoint_parts`], its patterns compiled
    /// exactly as [`Self::new`] compiles them.
    pub(crate) fn from_checkpoint_parts(parts: PlanCheckpointParts<'_>) -> Plan {
        let PlanCheckpointParts {
            anchor,
            page_property_rows,
            filter,
            track,
            today,
            remove_accents,
            registry,
            tag_targets,
        } = parts;
        Plan {
            anchor,
            page_property_rows,
            compiled: CompiledLeaves::for_query(filter),
            track,
            filter: filter.clone(),
            today,
            remove_accents,
            registry: registry.cloned(),
            tag_targets: tag_targets.cloned(),
        }
    }

    /// Whether the answer depends on every page's `tags::` rather than only the
    /// evaluated page's own text, so an edit that moves any page's tags can
    /// change a page it never touched (the memo's page-local rule does not
    /// hold for it).
    pub(crate) fn reads_tag_targets(&self) -> bool {
        self.tag_targets.is_some()
    }

    pub(crate) fn registry(&self) -> Option<&Arc<Registry>> {
        self.registry.as_ref()
    }

    fn ctx<'a>(
        &'a self,
        entry: &'a PageEntry,
        doc: &'a Document,
        facts: &'a PageFacts,
        config: &'a ParseConfig,
        atoms: &'a EvalCache,
    ) -> EvalCtx<'a> {
        EvalCtx::new(
            &entry.name,
            entry.kind,
            entry.date_key,
            facts.properties(),
            &doc.roots,
            atom_format(entry),
            self.today,
            &self.compiled,
            config,
            self.registry.as_deref().unwrap_or(Registry::none()),
            self.tag_targets.as_deref().unwrap_or(empty_tag_targets()),
            atoms,
        )
    }

    /// The block rows of one page: every matching block whose immediate parent
    /// did not match (OG `tree/filter-top-level-blocks`), in document order.
    /// The header page-property block comes first: OG stores it as the page's
    /// first block (`:block/pre-block? true`, `:block/properties` of the page
    /// header), so `property`, `page`, `between`, page-ref and full-text
    /// predicates match it like any block (query_dsl.cljs `build-property`;
    /// rules.cljc `:property`, `:page`, `:between`), and it never has a
    /// parent or children to suppress.
    fn block_hits<'a>(
        &self,
        ctx: &EvalCtx,
        doc: &'a Document,
        facts: &PageFacts,
        out: &mut Vec<Hit<'a>>,
    ) {
        if let Some(block) = facts
            .page_property_block()
            .filter(|_| self.page_property_rows)
        {
            if eval::eval_block(&self.filter, block, &PathRefCounts::new(), ctx) {
                out.push(Hit::PageProperty);
            }
        }
        struct Roots<'a, 'o, 'c, 'data> {
            filter: &'c Filter,
            ctx: &'c EvalCtx<'data>,
            matched: Vec<bool>,
            out: &'o mut Vec<Hit<'a>>,
        }
        impl<'a> PathRefVisitor<'a, DocBlock> for Roots<'a, '_, '_, '_> {
            fn enter(&mut self, block: &'a DocBlock, ancestors: &PathRefCounts) {
                let hit = eval::eval_block(self.filter, block, ancestors, self.ctx);
                if hit && !self.matched.last().copied().unwrap_or(false) {
                    self.out.push(Hit::Block(block));
                }
                self.matched.push(hit);
            }
            fn leave(&mut self, _block: &'a DocBlock) {
                self.matched.pop();
            }
        }
        let mut visitor = Roots {
            filter: &self.filter,
            ctx,
            matched: vec![false],
            out,
        };
        eval::walk_path_refs(&doc.roots, self.track, &mut visitor);
    }

    /// Whether page `doc` contributes any row to this plan's answer — the
    /// memo's per-page invalidation test, evaluated with the result's own
    /// registry snapshot.
    pub(crate) fn touches(
        &self,
        entry: &PageEntry,
        doc: &Document,
        facts: &PageFacts,
        config: &ParseConfig,
    ) -> bool {
        if self.skips(entry, facts) {
            return false;
        }
        #[cfg(feature = "test-faults")]
        {
            fn count(blocks: &[DocBlock]) {
                for block in blocks {
                    crate::cost_counters::query_carry_block_probe();
                    count(&block.children);
                }
            }
            count(&doc.roots);
        }
        let atoms = EvalCache::default();
        let ctx = self.ctx(entry, doc, facts, config, &atoms);
        match self.anchor {
            Anchor::Page => eval::eval_page(&self.filter, &ctx),
            Anchor::Block => {
                let mut hits = Vec::new();
                self.block_hits(&ctx, doc, facts, &mut hits);
                !hits.is_empty()
            }
        }
    }

    /// Candidate narrowing: a block-anchored filter that REQUIRES one of a set
    /// of page refs can skip every page that references none of them.
    fn skips(&self, entry: &PageEntry, facts: &PageFacts) -> bool {
        self.anchor == Anchor::Block
            && required_refs(&self.filter).is_some_and(|names| !facts.may_reference(entry, &names))
    }
}

fn empty_tag_targets() -> &'static HashSet<String> {
    static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashSet::new)
}

/// One block row of a page: a block of its document, or the page's header
/// property block, which lives in the page's facts (`PageFacts::
/// page_property_block`) rather than in `Document::roots`.
#[derive(Clone, Copy)]
enum Hit<'a> {
    PageProperty,
    Block(&'a DocBlock),
}

impl<'a> Hit<'a> {
    /// `facts` must be those of the page that produced the hit.
    fn block<'f>(&self, facts: &'f PageFacts) -> &'f DocBlock
    where
        'a: 'f,
    {
        match self {
            Hit::Block(block) => block,
            Hit::PageProperty => facts
                .page_property_block()
                .expect("a PageProperty hit comes from facts that carry the block"),
        }
    }
}

/// The wire row for one hit; the header property block is a read-only
/// synthetic row (`BlockDto::page_property`), exactly as in backlinks.
fn row_dto(facts: &PageFacts, block: &DocBlock) -> BlockDto {
    let mut dto = result_dto(block);
    dto.page_property = facts
        .page_property_block()
        .is_some_and(|header| std::ptr::eq(header, block));
    dto
}

/// Page refs one of which every matching block's path-refs closure must
/// contain, normalized; `None` when the filter does not require any.
fn required_refs(filter: &Filter) -> Option<Vec<String>> {
    match filter {
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Refs,
                    quant: Quant::Any | Quant::Every,
                    pred,
                },
        } => eval::single_ref_name(pred).map(|name| vec![refs::normalize(name)]),
        Filter::And { items } => items.iter().find_map(required_refs),
        Filter::Or { items } if !items.is_empty() => items
            .iter()
            .map(required_refs)
            .collect::<Option<Vec<_>>>()
            .map(|sets| sets.concat()),
        _ => None,
    }
}

/// Reorder every `and` so its cheapest conjuncts run first. Evaluation is pure
/// and short-circuiting, so this changes cost, never truth.
fn cheapest_first(filter: Filter) -> Filter {
    match filter {
        Filter::And { items } => {
            let mut items: Vec<Filter> = items.into_iter().map(cheapest_first).collect();
            items.sort_by_key(cost);
            Filter::And { items }
        }
        Filter::Or { items } => Filter::Or {
            items: items.into_iter().map(cheapest_first).collect(),
        },
        Filter::Not { inner } => Filter::Not {
            inner: Box::new(cheapest_first(*inner)),
        },
        other => other,
    }
}

fn cost(filter: &Filter) -> u8 {
    match filter {
        Filter::And { items } | Filter::Or { items } => items.iter().map(cost).max().unwrap_or(0),
        Filter::Not { inner } => cost(inner),
        Filter::Leaf { leaf } => match leaf {
            Leaf::Attr {
                attr: Attr::Content,
                op: CmpOp::Match | CmpOp::Regex,
                ..
            } => 5,
            Leaf::Attr {
                attr: Attr::Content,
                ..
            } => 4,
            Leaf::Attr { .. } => 0,
            Leaf::Rel { rel, .. } => match rel {
                Rel::Refs | Rel::Tags => 1,
                Rel::Page => 2,
                Rel::Props => 3,
                Rel::Children | Rel::Parent | Rel::Ancestors | Rel::Descendants | Rel::Blocks => 6,
            },
        },
        _ => 0,
    }
}

thread_local! {
    /// A test's pinned `sample` seed (see [`pin_sample_seed`]); `None` draws a
    /// fresh seed per execution, as OG's `shuffle` does.
    static SAMPLE_SEED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Pin the seed every `sample` on THIS thread draws from, until the guard
/// drops. A seed is the only randomness in query execution, so a test that
/// pins it gets a reproducible subset.
#[cfg(test)]
pub(crate) fn pin_sample_seed(seed: u64) -> SampleSeedGuard {
    SAMPLE_SEED.with(|cell| cell.set(Some(seed)));
    SampleSeedGuard
}

#[cfg(test)]
pub(crate) struct SampleSeedGuard;

#[cfg(test)]
impl Drop for SampleSeedGuard {
    fn drop(&mut self) {
        SAMPLE_SEED.with(|cell| cell.set(None));
    }
}

fn fresh_sample_seed() -> u64 {
    if let Some(seed) = SAMPLE_SEED.with(|cell| cell.get()) {
        return seed;
    }
    static DRAWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    nanos
        ^ DRAWS
            .fetch_add(0x9E37_79B9_7F4A_7C15, std::sync::atomic::Ordering::Relaxed)
            .rotate_left(17)
}

/// Keep a uniformly random subset of `items` of at most `n` elements, in their
/// current relative order (a partial Fisher-Yates over the indices, splitmix64
/// as the generator). Cost O(len).
fn take_random_subset<T>(items: &mut Vec<T>, n: usize) {
    if items.len() <= n {
        return;
    }
    let mut state = fresh_sample_seed();
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    for at in 0..n {
        let pick = at + (next() % (order.len() - at) as u64) as usize;
        order.swap(at, pick);
    }
    let mut keep = vec![false; items.len()];
    for &index in &order[..n] {
        keep[index] = true;
    }
    let mut index = 0;
    items.retain(|_| {
        index += 1;
        keep[index - 1]
    });
}

/// The recency axis (Unix seconds): a journal by the day it represents, any
/// other page by the mtime captured with the page table; oldest when unknown.
fn recency(
    entry: &PageEntry,
    mtimes: &crate::model::persistent::Map<String, std::time::SystemTime>,
) -> i64 {
    if let Some(day) = entry.date_key {
        return JournalDate::from_ordinal(day).to_days() * 86_400;
    }
    mtimes
        .get(entry.rel_path_str())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(i64::MIN, |elapsed| elapsed.as_secs() as i64)
}

fn is_recency_field(field: &str) -> bool {
    matches!(
        field.to_ascii_lowercase().as_str(),
        "modified" | "updated" | "updated-at" | "date"
    )
}

fn kind_rank(kind: PageKind) -> u8 {
    match kind {
        PageKind::Journal => 0,
        PageKind::Page => 1,
    }
}

/// SPEC §3.5's base order for block groups (M13), amended for OG parity (#8a):
/// OG renders grouped results with `(sort-by (comp :block/journal-day first) >)`
/// (`components/block.cljs:3497,3523,3552`), so journal days run NEWEST first and
/// pages that are not journals follow. The reference groups (`query::collect_bounded`)
/// already order this way; one rule for every group list (I-12). Ties fall back
/// to page name, then kind rank, then physical path.
fn base_order(a: &PageEntry, b: &PageEntry) -> std::cmp::Ordering {
    b.date_key
        .unwrap_or(i64::MIN)
        .cmp(&a.date_key.unwrap_or(i64::MIN))
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        .then_with(|| a.rel_path_str().as_bytes().cmp(b.rel_path_str().as_bytes()))
}

fn first_property(properties: &[(String, String)], field: &str) -> Option<String> {
    let field = property_key_norm(field);
    properties
        .iter()
        .find(|(key, _)| property_key_norm(key) == field)
        .map(|(_, value)| value.clone())
}

/// One row's aggregate inputs: `count` reads no value; every other function
/// reads the row's first value of the named property.
fn statistics_values(
    fold: &StatisticsFold,
    properties: &[(String, String)],
) -> Vec<Option<String>> {
    fold.view()
        .aggregates
        .iter()
        .map(|(field, op)| match op {
            tine_core::query::ir::AggFn::Count => None,
            _ => first_property(properties, field.as_str()),
        })
        .collect()
}

fn statistics_keys(
    fold: &StatisticsFold,
    special: impl FnOnce(&str) -> Option<Vec<Option<String>>>,
    properties: &[(String, String)],
) -> Vec<Option<String>> {
    let mut keys = match fold.view().group_by.as_ref().map(|field| field.as_str()) {
        None => Vec::new(),
        Some(field) if field.starts_with("formula:") => Vec::new(),
        Some(field) => special(field).unwrap_or_else(|| {
            vec![first_property(
                properties,
                field.strip_prefix("prop:").unwrap_or(field),
            )]
        }),
    };
    if keys.is_empty() {
        keys.push(None);
    }
    keys
}

/// Shared block sort meaning for IR and Friendly rows. Missing properties use
/// visible first-line text; planning fields use their stable missing sentinel.
/// Cost O(properties on one block); no failure for a parsed block.
pub(crate) fn block_sort_decor(
    field: &str,
    entry: &PageEntry,
    block: &DocBlock,
    page_recency: impl FnOnce() -> i64,
) -> SortDecor {
    let projection = block.projection();
    match field.to_ascii_lowercase().as_str() {
        _ if is_recency_field(field) => SortDecor::Num(page_recency()),
        "priority" => SortDecor::Text(
            block
                .priority()
                .map_or_else(|| "Z".to_string(), str::to_ascii_uppercase),
        ),
        "page" => SortDecor::Text(entry.name.to_lowercase()),
        "deadline" => SortDecor::Text(
            projection
                .deadline()
                .map_or_else(|| "~".to_string(), str::to_owned),
        ),
        "scheduled" => SortDecor::Text(
            projection
                .scheduled()
                .map_or_else(|| "~".to_string(), str::to_owned),
        ),
        _ => SortDecor::Text(lexical_property_sort_text(
            projection
                .properties()
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            field,
            || {
                block
                    .visible_text()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            },
        )),
    }
}

/// Shared page sort meaning for IR and Friendly rows. Missing properties use
/// the page name. Cost O(properties on one page); no failure for a page entry.
pub(crate) fn page_sort_decor(
    field: &str,
    entry: &PageEntry,
    properties: &[(String, String)],
    page_recency: i64,
) -> SortDecor {
    match field.to_ascii_lowercase().as_str() {
        "name" | "page" => SortDecor::Text(entry.name.to_lowercase()),
        "kind" => SortDecor::Text(
            match entry.kind {
                PageKind::Journal => "journal",
                PageKind::Page => "page",
            }
            .into(),
        ),
        "day" | "journal-day" | "journal_day" => SortDecor::Num(entry.date_key.unwrap_or(i64::MIN)),
        _ if is_recency_field(field) => SortDecor::Num(page_recency),
        _ => SortDecor::Text(lexical_property_sort_text(
            properties
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            field,
            || entry.name.clone(),
        )),
    }
}

/// Execute one plan over the graph's current generation.
pub(crate) fn execute(
    graph: &impl GraphRead,
    plan: &Plan,
    query: &Query,
    view: &ViewSettings,
    bounds: Bounds,
) -> Result<QueryResult, StatisticsResourceLimit> {
    let mut result = QueryResult {
        rows: match plan.anchor {
            Anchor::Block => QueryRows::Block { groups: Vec::new() },
            Anchor::Page => QueryRows::Page { pages: Vec::new() },
        },
        diagnostics: query.diagnostics.clone(),
        report: tine_core::query::ir::QueryReport {
            ran: Vec::new(),
            ignored: Vec::new(),
            supported: true,
        },
        total: 0,
        matched_total: (plan.anchor == Anchor::Page).then_some(0),
        statistics: None,
        exceeded: false,
    };
    // §3.5: an invalid query returns nothing plus its diagnostics.
    if query.is_invalid() {
        return Ok(result);
    }
    let wants_recency = view
        .sort
        .iter()
        .any(|(field, _)| is_recency_field(field.as_str()));
    let mtimes = wants_recency.then(|| graph.observed_page_mtimes());
    let recency_of =
        |entry: &PageEntry| mtimes.as_deref().map_or(0, |mtimes| recency(entry, mtimes));
    let ascending: Vec<bool> = view
        .sort
        .iter()
        .map(|(_, dir)| *dir == SortDir::Asc)
        .collect();
    let sample = view.sample.map(|n| n as usize);
    let mut fold = StatisticsFold::new(view, bounds.max_bytes)?;
    graph.with_pages(|pages| -> Result<(), StatisticsResourceLimit> {
        let candidates = candidates::Candidates::new(graph, pages, plan);
        let config = candidates.config();
        let atoms = EvalCache::default();
        match plan.anchor {
            Anchor::Block => {
                let mut groups: Vec<(&PageEntry, Arc<PageFacts>, Vec<Hit>)> = Vec::new();
                for (entry, doc) in candidates.pages() {
                    let facts = candidates.facts(entry, doc);
                    if plan.skips(entry, &facts) {
                        continue;
                    }
                    let mut hits = Vec::new();
                    plan.block_hits(
                        &plan.ctx(entry, doc, &facts, config, &atoms),
                        doc,
                        &facts,
                        &mut hits,
                    );
                    if !hits.is_empty() {
                        groups.push((entry, facts, hits));
                    }
                }
                groups.sort_by(|a, b| base_order(a.0, b.0));
                let mut rows: Vec<(usize, &DocBlock)> = groups
                    .iter()
                    .enumerate()
                    .flat_map(|(at, (_, facts, hits))| {
                        hits.iter().map(move |hit| (at, hit.block(facts)))
                    })
                    .collect();
                result.matched_total = Some(rows.len());
                // OG `query` (`query_dsl.cljs:583-589`): `sample` is a random
                // subset of the FILTERED result, taken before `sort-by`.
                if let Some(sample) = sample {
                    take_random_subset(&mut rows, sample);
                }
                let sorted = !view.sort.is_empty();
                if sorted {
                    let mut page_recency: Vec<Option<i64>> = vec![None; groups.len()];
                    let mut decorated: Vec<(Vec<SortDecor>, usize, (usize, &DocBlock))> = rows
                        .into_iter()
                        .enumerate()
                        .map(|(base, (at, block))| {
                            let entry = groups[at].0;
                            let keys = view
                                .sort
                                .iter()
                                .map(|(field, _)| {
                                    block_sort_decor(field.as_str(), entry, block, || {
                                        *page_recency[at].get_or_insert_with(|| recency_of(entry))
                                    })
                                })
                                .collect();
                            (keys, base, (at, block))
                        })
                        .collect();
                    decorated.sort_by(|a, b| {
                        compare_sort_decorations(&a.0, &b.0, &ascending).then_with(|| a.1.cmp(&b.1))
                    });
                    rows = decorated.into_iter().map(|(_, _, row)| row).collect();
                }
                if let Some(fold) = fold.as_mut() {
                    for (at, block) in &rows {
                        let (entry, _, _) = &groups[*at];
                        let projection = block.projection();
                        let values = statistics_values(fold, &projection.properties());
                        let keys = statistics_keys(
                            fold,
                            |field| match field {
                                "tags" => {
                                    Some(projection.tags().iter().cloned().map(Some).collect())
                                }
                                "page" | "name" => Some(vec![Some(entry.name.clone())]),
                                "state" => Some(vec![block.marker().map(str::to_owned)]),
                                "priority" => Some(vec![block.priority().map(str::to_owned)]),
                                "scheduled" => {
                                    Some(vec![projection.scheduled().map(str::to_owned)])
                                }
                                "deadline" => Some(vec![projection.deadline().map(str::to_owned)]),
                                _ => None,
                            },
                            &projection.properties(),
                        );
                        fold.add(&values, keys)?;
                    }
                }
                let mut budget = ConstructionBudget::new(bounds.max_rows, bounds.max_bytes);
                let mut out: Vec<RefGroup> = Vec::new();
                let mut last: Option<usize> = None;
                for (at, block) in rows {
                    let entry = groups[at].0;
                    let facts = &groups[at].1;
                    if !budget.admit_estimated(&entry.name, shallow_dto_estimated_bytes(block, &[]))
                    {
                        continue;
                    }
                    let same = match (last, out.last()) {
                        (Some(previous), Some(group)) if sorted => {
                            group.page == entry.name && group.kind == entry.kind || previous == at
                        }
                        (Some(previous), Some(_)) => previous == at,
                        _ => false,
                    };
                    if same {
                        out.last_mut()
                            .expect("group")
                            .blocks
                            .push(row_dto(facts, block));
                    } else {
                        out.push(RefGroup {
                            page: entry.name.clone(),
                            kind: entry.kind,
                            blocks: vec![row_dto(facts, block)],
                            evidence: Vec::new(),
                        });
                    }
                    last = Some(at);
                }
                result.total = budget.total;
                result.exceeded = budget.exceeded;
                result.rows = QueryRows::Block { groups: out };
            }
            Anchor::Page => {
                let mut matches: Vec<(&PageEntry, Arc<PageFacts>)> = Vec::new();
                for (entry, doc) in candidates.pages() {
                    let facts = candidates.facts(entry, doc);
                    if eval::eval_page(&plan.filter, &plan.ctx(entry, doc, &facts, config, &atoms))
                    {
                        matches.push((entry, facts));
                    }
                }
                matches.sort_by(|a, b| {
                    a.0.rel_path_str()
                        .as_bytes()
                        .cmp(b.0.rel_path_str().as_bytes())
                });
                let matched = matches.len();
                if let Some(sample) = sample {
                    take_random_subset(&mut matches, sample);
                }
                if !view.sort.is_empty() {
                    let mut decorated: Vec<(Vec<SortDecor>, usize, (&PageEntry, Arc<PageFacts>))> =
                        matches
                            .into_iter()
                            .enumerate()
                            .map(|(base, (entry, facts))| {
                                let page_recency = recency_of(entry);
                                let keys = view
                                    .sort
                                    .iter()
                                    .map(|(field, _)| {
                                        page_sort_decor(
                                            field.as_str(),
                                            entry,
                                            facts.properties(),
                                            page_recency,
                                        )
                                    })
                                    .collect();
                                (keys, base, (entry, facts))
                            })
                            .collect();
                    // Ties keep physical path order, which is the base order.
                    decorated.sort_by(|a, b| {
                        compare_sort_decorations(&a.0, &b.0, &ascending).then_with(|| a.1.cmp(&b.1))
                    });
                    matches = decorated.into_iter().map(|(_, _, row)| row).collect();
                }
                if let Some(fold) = fold.as_mut() {
                    for (entry, facts) in matches.iter() {
                        let values = statistics_values(fold, facts.properties());
                        let keys = statistics_keys(
                            fold,
                            |field| match field {
                                "tags" => Some(facts.tags().iter().cloned().map(Some).collect()),
                                "page" | "name" => Some(vec![Some(entry.name.clone())]),
                                "path" => Some(vec![Some(entry.rel_path_str().to_owned())]),
                                "kind" => Some(vec![Some(
                                    match entry.kind {
                                        PageKind::Journal => "journal",
                                        PageKind::Page => "page",
                                    }
                                    .to_owned(),
                                )]),
                                "day" | "journal-day" | "journal_day" => {
                                    Some(vec![entry.date_key.map(|day| day.to_string())])
                                }
                                _ => None,
                            },
                            facts.properties(),
                        );
                        fold.add(&values, keys)?;
                    }
                }
                // The sample was taken before the sort, so admission sees only
                // the returned rows: a sampled page query over the bound is
                // answered, as a sampled block query is (Reader B, og 14 Q2).
                let offered = matches.len();
                let mut budget = ConstructionBudget::new(bounds.max_rows, bounds.max_bytes);
                let mut rows = Vec::new();
                for (entry, facts) in matches {
                    let estimated = facts.properties().iter().fold(
                        96 + entry.name.len() + entry.rel_path_str().len(),
                        |bytes, (k, v)| bytes.saturating_add(k.len()).saturating_add(v.len()),
                    );
                    if !budget.admit_page_estimated(estimated) {
                        break;
                    }
                    rows.push(PageRow {
                        path: entry.rel_path_str().to_owned(),
                        name: entry.name.clone(),
                        kind: entry.kind,
                        journal_day: entry.date_key,
                        properties: facts.properties().to_vec(),
                    });
                }
                result.total = rows.len();
                result.exceeded = budget.exceeded || offered > result.total;
                result.matched_total = Some(matched);
                result.rows = QueryRows::Page { pages: rows };
            }
        }
        Ok(())
    })?;
    result.statistics = fold.map(StatisticsFold::finish);
    Ok(result)
}

/// Count the rows a plan matches, constructing nothing (explain-empty's probe).
fn count(graph: &impl GraphRead, plan: &Plan) -> usize {
    graph.with_pages(|pages| {
        let candidates = candidates::Candidates::new(graph, pages, plan);
        let config = candidates.config();
        let atoms = EvalCache::default();
        let mut count = 0usize;
        let mut hits = Vec::new();
        for (entry, doc) in candidates.pages() {
            let facts = candidates.facts(entry, doc);
            if plan.skips(entry, &facts) {
                continue;
            }
            let ctx = plan.ctx(entry, doc, &facts, config, &atoms);
            match plan.anchor {
                Anchor::Page => count += usize::from(eval::eval_page(&plan.filter, &ctx)),
                Anchor::Block => {
                    hits.clear();
                    plan.block_hits(&ctx, doc, &facts, &mut hits);
                    count += hits.len();
                }
            }
        }
        count
    })
}

/// The plan for `query` over this generation, reading the registry only when a
/// `props` leaf needs it.
pub(crate) fn plan(
    graph: &impl GraphRead,
    query: &Query,
    today: JournalDate,
    block_rows: bool,
) -> Plan {
    Plan::new(
        query,
        today,
        block_rows,
        graph.config().enable_search_remove_accents,
        || graph.with_pages(|pages| graph.query_index().registry(pages)),
        || graph.with_pages(|pages| Arc::new(graph.query_index().tag_targets(pages))),
    )
}

/// [`crate::WholeGraph::query_ir`]'s body: the one IR front door over a snapshot.
pub(crate) fn query_ir(
    graph: &crate::model::ReadSnapshot,
    request: crate::IrRequest<'_>,
) -> Result<crate::IrAnswer, StatisticsResourceLimit> {
    use crate::query::memo::Answer;
    let today = tine_core::date::JournalDate::today();
    let (query, view, context) = match request {
        crate::IrRequest::Registry => {
            let index = graph.query_index();
            let registry = graph.with_pages(|pages| index.registry(pages));
            return Ok(crate::IrAnswer::Registry(registry));
        }
        crate::IrRequest::ExplainEmpty { query, context } => {
            let resolved = tine_core::query::resolve_for_execution(query, context, today);
            let answer = explain_empty(graph, &resolved);
            return Ok(crate::IrAnswer::ExplainEmpty(answer));
        }
        crate::IrRequest::Run {
            query,
            view,
            context,
        } => (query, view, context),
    };
    let resolved = tine_core::query::resolve_for_execution(query, context, today);
    let key = format!(
        "R\0{}\0{}",
        serde_json::to_string(&(resolved.query(), resolved.report())).unwrap_or_default(),
        serde_json::to_string(view).unwrap_or_default(),
    );
    let answer = graph.query_answer(key, today, || {
        let bounds = tine_core::query::ir::Bounds {
            max_rows: crate::store::RESULT_BRIDGE_MAX_ROWS,
            max_bytes: crate::store::RESULT_BRIDGE_MAX_BYTES,
        };
        let (result, plan) = run_resolved(graph, &resolved, view, bounds);
        (Answer::Result(result.map(Arc::new)), Some(plan))
    });
    match answer {
        Answer::Result(result) => {
            result.map(|result| crate::IrAnswer::Result(Box::new(result.as_ref().clone())))
        }
        _ => unreachable!("R keys hold IR results"),
    }
}

/// `query_run`'s evaluation (SPEC §7.1): the resolved tree under the
/// statistics-execution view, with the binding's report attached afterwards.
pub(crate) fn run_resolved(
    graph: &impl GraphRead,
    resolved: &ResolvedQuery,
    view: &ViewSettings,
    bounds: Bounds,
) -> (Result<QueryResult, StatisticsResourceLimit>, Arc<Plan>) {
    let view = statistics_execution_view(resolved.query(), view);
    let plan = Arc::new(plan(graph, resolved.query(), resolved.today(), false));
    let result = execute(graph, &plan, resolved.query(), &view, bounds).map(|mut result| {
        result.report = resolved.report().clone();
        result
    });
    (result, plan)
}

/// `query_explain_empty` (SPEC §7.1, N19): one count per probe of the plan.
pub(crate) fn explain_empty(
    graph: &impl GraphRead,
    resolved: &ResolvedQuery,
) -> ExplainEmptyResult {
    let explain = explain_empty_plan(resolved);
    let counts: Vec<usize> = explain
        .probes
        .iter()
        .map(|probe| {
            let plan = plan(graph, probe, resolved.today(), false);
            count(graph, &plan)
        })
        .collect();
    explain
        .answer(resolved, &counts)
        .expect("one count per probe, by construction")
}

/// The legacy block-group bridge for one resolved query (`run_query`, the
/// advanced bridge and Copy/Export): block rows even for a `@page` query.
pub(crate) fn run_block_groups(
    graph: &impl GraphRead,
    resolved: &ResolvedQuery,
    view: &ViewSettings,
    max_rows: usize,
    max_bytes: usize,
) -> (BoundedGroups, Arc<Plan>) {
    let plan = Arc::new(plan(graph, resolved.query(), resolved.today(), true));
    let view = ViewSettings {
        aggregates: Vec::new(),
        group_by: None,
        ..view.clone()
    };
    let bounds = Bounds {
        max_rows,
        max_bytes,
    };
    let result = execute(graph, &plan, resolved.query(), &view, bounds)
        .expect("no statistics were requested");
    let groups = BoundedGroups {
        groups: match result.rows {
            QueryRows::Block { groups } => groups,
            QueryRows::Page { .. } => Vec::new(),
        },
        total: result.total,
        exceeded: result.exceeded,
    };
    (groups, plan)
}

/// `{{query …}}` text (OG DSL) through the legacy bridge.
pub(crate) fn run_query_bounded(
    graph: &impl GraphRead,
    source: &str,
    max_rows: usize,
    max_bytes: usize,
) -> (BoundedGroups, Arc<Plan>) {
    run_query_at(graph, source, max_rows, max_bytes, JournalDate::today())
}

/// [`run_query_bounded`] on a given execution day (relative dates resolve
/// against it).
pub(crate) fn run_query_at(
    graph: &impl GraphRead,
    source: &str,
    max_rows: usize,
    max_bytes: usize,
    today: JournalDate,
) -> (BoundedGroups, Arc<Plan>) {
    let (query, view) = parse_query_text(source, QueryDialect::Og, today);
    let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
    run_block_groups(graph, &resolved, &view, max_rows, max_bytes)
}

/// An advanced (datalog) source through the legacy bridge: the join-free
/// pattern subset (#542), with no current page bound (§4.4). The plan is
/// `None` for an unsupported query, whose answer reads no page.
pub(crate) fn run_advanced_query_bounded(
    graph: &impl GraphRead,
    source: &str,
    max_rows: usize,
    max_bytes: usize,
) -> ((AdvancedResult, bool, usize), Option<Arc<Plan>>) {
    run_advanced_query_at(graph, source, max_rows, max_bytes, JournalDate::today())
}

/// [`run_advanced_query_bounded`] on a given execution day.
pub(crate) fn run_advanced_query_at(
    graph: &impl GraphRead,
    source: &str,
    max_rows: usize,
    max_bytes: usize,
    today: JournalDate,
) -> ((AdvancedResult, bool, usize), Option<Arc<Plan>>) {
    let (query, _) = parse_query_input(source, QueryInput::Advanced, today, Registry::none());
    let resolved = resolve_for_execution(&query, &ExecutionContext::none(), today);
    let report = resolved.report().clone();
    if !resolved.is_executable() {
        let result = AdvancedResult {
            groups: Vec::new(),
            ran: report.ran,
            ignored: report.ignored,
            supported: false,
        };
        return ((result, false, 0), None);
    }
    let (bounded, plan) = run_block_groups(
        graph,
        &resolved,
        &ViewSettings::default(),
        max_rows,
        max_bytes,
    );
    let result = AdvancedResult {
        groups: bounded.groups,
        ran: report.ran,
        ignored: report.ignored,
        supported: true,
    };
    ((result, bounded.exceeded, bounded.total), Some(plan))
}
