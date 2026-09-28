//! In-memory execution of a resolved query (SPEC §3.5, §5.9, §7.1): the ONE
//! answerer for `query_run`, `query_explain_empty`, the legacy `run_query` /
//! `run_advanced_query` bridges and Copy/Export (I-12).
//!
//! Semantics are master's (walk + results, production comparison mode):
//! - block rows: OG top-level roots per page (a match whose immediate parent
//!   matched is dropped), base order page name then kind rank (journal first)
//!   then physical path, document order within a page;
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

use std::sync::Arc;

use tine_core::date::JournalDate;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::model::{PageEntry, PageKind, RefGroup};
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

/// One query, ready to evaluate against pages: the anchor-adjusted evaluable
/// filter, its compiled patterns and the registry snapshot it coerces by.
pub(crate) struct Plan {
    anchor: Anchor,
    filter: Filter,
    compiled: CompiledLeaves,
    track: bool,
    today: JournalDate,
    remove_accents: bool,
    registry: Option<Arc<Registry>>,
}

impl Plan {
    /// `block_rows` evaluates a `@page` query block-anchored (page attributes
    /// read through `block.page`), which is the legacy block-group bridge's
    /// semantics (master `block_anchored_filter`).
    pub(crate) fn new(
        query: &Query,
        today: JournalDate,
        block_rows: bool,
        remove_accents: bool,
        registry: impl FnOnce() -> Arc<Registry>,
    ) -> Plan {
        let evaluable = query.evaluable_filter();
        let (anchor, filter) = match query.anchor {
            Anchor::Page if block_rows => (Anchor::Block, rebase_to_block(&evaluable)),
            anchor => (anchor, evaluable),
        };
        let filter = cheapest_first(filter);
        let registry = filter.has_props_leaf().then(registry);
        Plan {
            anchor,
            compiled: CompiledLeaves::for_query(&filter, remove_accents),
            track: eval::uses_path_refs(&filter),
            filter,
            today,
            remove_accents,
            registry,
        }
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
            self.remove_accents,
            &self.compiled,
            config,
            self.registry.as_deref().unwrap_or(Registry::none()),
            atoms,
        )
    }

    /// The block rows of one page: every matching block whose immediate parent
    /// did not match (OG `tree/filter-top-level-blocks`), in document order.
    fn block_hits<'a>(&self, ctx: &EvalCtx, doc: &'a Document, out: &mut Vec<&'a DocBlock>) {
        struct Roots<'a, 'o, 'c> {
            filter: &'c Filter,
            ctx: &'c EvalCtx<'c>,
            matched: Vec<bool>,
            out: &'o mut Vec<&'a DocBlock>,
        }
        impl<'a> PathRefVisitor<'a, DocBlock> for Roots<'a, '_, '_> {
            fn enter(&mut self, block: &'a DocBlock, ancestors: &PathRefCounts) {
                let hit = eval::eval_block(self.filter, block, ancestors, self.ctx);
                if hit && !self.matched.last().copied().unwrap_or(false) {
                    self.out.push(block);
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
                self.block_hits(&ctx, doc, &mut hits);
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
                Rel::Children | Rel::Blocks => 6,
            },
        },
        _ => 0,
    }
}

/// The recency axis (Unix seconds): a journal by the day it represents, any
/// other page by the mtime captured with the page table; oldest when unknown.
fn recency(
    entry: &PageEntry,
    mtimes: &std::collections::HashMap<String, std::time::SystemTime>,
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

/// SPEC §3.5's base order for block groups (M13).
fn base_order(a: &PageEntry, b: &PageEntry) -> std::cmp::Ordering {
    a.name
        .cmp(&b.name)
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

fn block_sort_decor(
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
        "deadline" => SortDecor::Text(projection.deadline.clone().unwrap_or_else(|| "~".into())),
        "scheduled" => SortDecor::Text(projection.scheduled.clone().unwrap_or_else(|| "~".into())),
        _ => SortDecor::Text(lexical_property_sort_text(
            projection
                .properties
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            field,
            || projection.visible.lines().next().unwrap_or("").to_string(),
        )),
    }
}

fn page_sort_decor(
    field: &str,
    entry: &PageEntry,
    facts: &PageFacts,
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
            facts
                .properties()
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
    index: &QueryIndex,
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
        let config = index.parse_config();
        let atoms = EvalCache::default();
        match plan.anchor {
            Anchor::Block => {
                let mut groups: Vec<(&PageEntry, Arc<PageFacts>, Vec<&DocBlock>)> = Vec::new();
                for (entry, doc) in pages {
                    let facts = index.facts(entry, doc);
                    if plan.skips(entry, &facts) {
                        continue;
                    }
                    let mut hits = Vec::new();
                    plan.block_hits(
                        &plan.ctx(entry, doc, &facts, config, &atoms),
                        doc,
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
                    .flat_map(|(at, (_, _, hits))| hits.iter().map(move |block| (at, *block)))
                    .collect();
                result.matched_total = Some(rows.len());
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
                if let Some(sample) = sample {
                    rows.truncate(sample);
                }
                if let Some(fold) = fold.as_mut() {
                    for (at, block) in &rows {
                        let (entry, _, _) = &groups[*at];
                        let projection = block.projection();
                        let values = statistics_values(fold, &projection.properties);
                        let keys = statistics_keys(
                            fold,
                            |field| match field {
                                "tags" => Some(projection.tags.iter().cloned().map(Some).collect()),
                                "page" | "name" => Some(vec![Some(entry.name.clone())]),
                                "state" => Some(vec![block.marker().map(str::to_owned)]),
                                "priority" => Some(vec![block.priority().map(str::to_owned)]),
                                "scheduled" => Some(vec![projection.scheduled.clone()]),
                                "deadline" => Some(vec![projection.deadline.clone()]),
                                _ => None,
                            },
                            &projection.properties,
                        );
                        fold.add(&values, keys)?;
                    }
                }
                let mut budget = ConstructionBudget::new(bounds.max_rows, bounds.max_bytes);
                let mut out: Vec<RefGroup> = Vec::new();
                let mut last: Option<usize> = None;
                for (at, block) in rows {
                    let entry = groups[at].0;
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
                            .push(result_dto(block));
                    } else {
                        out.push(RefGroup {
                            page: entry.name.clone(),
                            kind: entry.kind,
                            blocks: vec![result_dto(block)],
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
                for (entry, doc) in pages {
                    let facts = index.facts(entry, doc);
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
                                        page_sort_decor(field.as_str(), entry, &facts, page_recency)
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
                    for (entry, facts) in matches.iter().take(sample.unwrap_or(usize::MAX)) {
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
                // `sample` keeps the first N ordered pages, so sampling before
                // admission returns the same rows and admits only what is
                // returned: a sampled page query over the bound is answered,
                // as a sampled block query is (Reader B, og 14 Q2).
                if let Some(sample) = sample {
                    matches.truncate(sample);
                }
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
fn count(graph: &impl GraphRead, index: &QueryIndex, plan: &Plan) -> usize {
    graph.with_pages(|pages| {
        let config = index.parse_config();
        let atoms = EvalCache::default();
        let mut count = 0usize;
        let mut hits = Vec::new();
        for (entry, doc) in pages {
            let facts = index.facts(entry, doc);
            if plan.skips(entry, &facts) {
                continue;
            }
            let ctx = plan.ctx(entry, doc, &facts, config, &atoms);
            match plan.anchor {
                Anchor::Page => count += usize::from(eval::eval_page(&plan.filter, &ctx)),
                Anchor::Block => {
                    hits.clear();
                    plan.block_hits(&ctx, doc, &mut hits);
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
    index: &QueryIndex,
    query: &Query,
    today: JournalDate,
    block_rows: bool,
) -> Plan {
    Plan::new(
        query,
        today,
        block_rows,
        graph.config().enable_search_remove_accents,
        || graph.with_pages(|pages| index.registry(pages)),
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
    let index = graph.query_index();
    let view = statistics_execution_view(resolved.query(), view);
    let plan = Arc::new(plan(
        graph,
        &index,
        resolved.query(),
        resolved.today(),
        false,
    ));
    let result =
        execute(graph, &index, &plan, resolved.query(), &view, bounds).map(|mut result| {
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
    let index = graph.query_index();
    let explain = explain_empty_plan(resolved);
    let counts: Vec<usize> = explain
        .probes
        .iter()
        .map(|probe| {
            let plan = plan(graph, &index, probe, resolved.today(), false);
            count(graph, &index, &plan)
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
    let index = graph.query_index();
    let plan = Arc::new(plan(
        graph,
        &index,
        resolved.query(),
        resolved.today(),
        true,
    ));
    let view = ViewSettings {
        aggregates: Vec::new(),
        group_by: None,
        ..view.clone()
    };
    let bounds = Bounds {
        max_rows,
        max_bytes,
    };
    let result = execute(graph, &index, &plan, resolved.query(), &view, bounds)
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
