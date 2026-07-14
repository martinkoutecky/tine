//! Backlinks and the `{{query}}` subset engine. Evaluated by scanning parsed
//! pages (no datalog). Pragmatic subset: page/tag refs, boolean and/or/not,
//! task markers, and property filters. Advanced datalog (`[:find ...]`) is
//! detected and reported as unsupported rather than crashed.

use crate::date::JournalDate;
use crate::doc::{DocBlock, Document};
use crate::model::{
    block_to_dto, BlockDto, Format, Graph, PageEntry, PageKind, RefGroup, ReferenceBlockEvidence,
    ReferenceDiagnosticTrace, ReferenceDiagnostics, ReferenceKind, TemplateDto,
};
use crate::refs;
use crate::search_query::Matcher;

/// Parse a journal-page title (e.g. "Jan 1st, 2022") to a `yyyymmdd` ordinal.
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

/// Walk depth-first, passing each block's ancestor chain (outermost first).
fn walk_path<'a>(
    blocks: &'a [DocBlock],
    path: &mut Vec<&'a DocBlock>,
    f: &mut impl FnMut(&'a DocBlock, &[&'a DocBlock]),
) {
    for b in blocks {
        f(b, path);
        path.push(b);
        walk_path(&b.children, path, f);
        path.pop();
    }
}

/// Cancellable variant used by interactive search. Returning false from `f`
/// stops the entire depth-first walk, including the current deep page.
/// A short, single-line label for a block in a breadcrumb trail.
fn crumb_line(b: &DocBlock) -> String {
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
fn collect(
    graph: &Graph,
    mut keep: impl FnMut(&DocBlock) -> bool,
    mut keep_page_properties: impl FnMut(&PageEntry, &str) -> Option<BlockDto>,
    exclude: Option<&str>,
) -> Vec<RefGroup> {
    let ex = exclude.map(refs::normalize);
    graph.with_pages(|pages| {
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
                    matched.push(property_ref);
                }
            }
            let mut path: Vec<&DocBlock> = Vec::new();
            walk_path(&doc.roots, &mut path, &mut |b, anc| {
                if keep(b) {
                    let mut dto = block_to_dto(b);
                    dto.breadcrumb = anc.iter().map(|a| crumb_line(a)).collect();
                    matched.push(dto);
                }
            });
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
    })
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
pub fn page_aliases(graph: &Graph) -> Vec<(String, String)> {
    graph.with_pages(|pages| {
        let mut out: Vec<(String, String)> = Vec::new();
        for (entry, doc) in pages {
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
            let Some(text) = alias_text else { continue };
            for line in text.lines() {
                if let Some((k, v)) = crate::doc::parse_property_line(line) {
                    if k.eq_ignore_ascii_case("alias") || k.eq_ignore_ascii_case("aliases") {
                        let trimmed = v.trim();
                        if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"')
                        {
                            continue;
                        }
                        for a in v.split([',', '，']) {
                            let a = strip_ref(a.trim());
                            if !a.is_empty() {
                                out.push((refs::normalize(&a), entry.name.clone()));
                            }
                        }
                    }
                }
            }
        }
        out
    })
}

/// Resolve a requested page/alias to its canonical display name and all
/// normalized names that identify that page. The normalized list is shared by
/// backlinks, unlinked references, and their scoped-invalidation predicates so
/// those paths cannot drift.
fn equivalent_page_names(aliases: &[(String, String)], target: &str) -> (String, Vec<String>) {
    let target_norm = refs::normalize(target);
    let canonical = aliases
        .iter()
        .find(|(alias, _)| *alias == target_norm)
        .map(|(_, canonical)| canonical.clone())
        .unwrap_or_else(|| target.to_string());
    let canonical_norm = refs::normalize(&canonical);
    let mut names = vec![canonical_norm.clone()];
    for (alias, alias_target) in aliases {
        if refs::normalize(alias_target) == canonical_norm && !names.contains(alias) {
            names.push(alias.clone());
        }
    }
    (canonical, names)
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

fn block_reference_evidence(
    block: &DocBlock,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
) -> Option<ReferenceBlockEvidence> {
    let occurrences = crate::reference_evidence::occurrences(
        &block.raw,
        &block.projection().reference_source,
        canonical,
        names_norm,
    )
    .into_iter()
    .filter(|occurrence| occurrence.kind == kind)
    .collect::<Vec<_>>();
    (!occurrences.is_empty()).then(|| ReferenceBlockEvidence {
        block_id: block.uuid.clone(),
        occurrences,
    })
}

fn collect_reference_occurrences(
    graph: &Graph,
    canonical: &str,
    names_norm: &[String],
    kind: ReferenceKind,
) -> Vec<RefGroup> {
    let exclude = refs::normalize(canonical);
    graph.with_pages(|pages| {
        let mut groups: Vec<(Option<i64>, RefGroup)> = Vec::new();
        for (entry, doc) in pages {
            if refs::normalize(&entry.name) == exclude {
                continue;
            }
            let mut blocks = Vec::new();
            let mut evidence = Vec::new();
            if kind == ReferenceKind::Explicit {
                if let Some(mut block) = doc
                    .pre_block
                    .as_deref()
                    .and_then(|pre| page_property_block(entry, pre))
                {
                    if let Some(hit) = block_reference_evidence(&block, canonical, names_norm, kind)
                    {
                        let mut dto = block_to_dto(&block);
                        dto.page_property = true;
                        blocks.push(dto);
                        evidence.push(hit);
                    }
                    // Make it impossible to accidentally retain this synthetic
                    // block past the result construction boundary.
                    block.children.clear();
                }
            }
            let mut path = Vec::new();
            walk_path(&doc.roots, &mut path, &mut |block, ancestors| {
                if let Some(hit) = block_reference_evidence(block, canonical, names_norm, kind) {
                    let mut dto = block_to_dto(block);
                    dto.breadcrumb = ancestors
                        .iter()
                        .map(|ancestor| crumb_line(ancestor))
                        .collect();
                    blocks.push(dto);
                    evidence.push(hit);
                }
            });
            if !blocks.is_empty() {
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
        groups.sort_by(|a, b| {
            b.0.unwrap_or(i64::MIN)
                .cmp(&a.0.unwrap_or(i64::MIN))
                .then_with(|| a.1.page.cmp(&b.1.page))
        });
        groups.into_iter().map(|(_, group)| group).collect()
    })
}

pub fn backlinks(graph: &Graph, target: &str) -> Vec<RefGroup> {
    let aliases = graph.page_aliases();
    let (canonical, names_norm) = equivalent_page_names(&aliases, target);
    collect_reference_occurrences(graph, &canonical, &names_norm, ReferenceKind::Explicit)
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
    collect(
        graph,
        |b| b.projection().block_refs.iter().any(|r| r == u),
        |_, _| None,
        None,
    )
}

/// Unlinked references: parser-visible plain occurrences outside explicit
/// reference syntax. A block containing both kinds appears once in each surface,
/// with the corresponding occurrence evidence.
pub fn unlinked_refs(graph: &Graph, target: &str) -> Vec<RefGroup> {
    let aliases = graph.page_aliases();
    let (canonical, names_norm) = equivalent_page_names(&aliases, target);
    collect_reference_occurrences(graph, &canonical, &names_norm, ReferenceKind::Plain)
}

/// Target-scoped trace for bug reports. Membership comes from the exact same
/// occurrence engine as the panels; the deliberately uncached parser path makes
/// projection-cache drift visible. No launcher history is read or returned.
pub fn reference_diagnostics(graph: &Graph, target: &str) -> ReferenceDiagnostics {
    let aliases = graph.page_aliases();
    let (canonical, names_norm) = equivalent_page_names(&aliases, target);
    let excluded_page = refs::normalize(&canonical);
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
                if k.eq_ignore_ascii_case("tags") {
                    tags = v
                        .split(',')
                        .map(|t| strip_ref(t.trim()))
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                props.push((k, v));
            }
        }
    }
    (props, tags)
}

pub fn run_query(graph: &Graph, query_src: &str) -> Vec<RefGroup> {
    let today = JournalDate::today();
    let Some(pred) = Pred::parse(query_src, today) else {
        return Vec::new();
    };
    let mut opts = QueryOpts::default();
    pred.collect_opts(&mut opts);
    run_pred(graph, &pred, &opts)
}

/// Evaluate a parsed predicate against the whole graph (shared by the simple-DSL
/// `run_query` and the advanced-datalog `run_advanced_query`).
fn run_pred(graph: &Graph, pred: &Pred, opts: &QueryOpts) -> Vec<RefGroup> {
    // A recency sort (`(sort-by modified …)`) needs each result page's position on
    // a single time axis: journal pages by the day they represent, other pages by
    // file mtime. Only computed when such a sort is active (else we skip the stat).
    let want_recency = matches!(&opts.sort, Some((f, _)) if is_recency_field(f));
    let (mut groups, recency_by_page) = graph.with_pages(|pages| {
        let mut groups: Vec<RefGroup> = Vec::new();
        let mut recency: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for (entry, doc) in pages {
            let (page_props, page_tags) = page_facets(doc.pre_block.as_deref());
            let ctx = EvalCtx {
                journal: entry.date_key,
                is_journal: entry.kind == PageKind::Journal,
                page_name: &entry.name,
                page_props: &page_props,
                page_tags: &page_tags,
            };
            let mut matched: Vec<&DocBlock> = Vec::new();
            walk(&doc.roots, &mut |b| {
                if pred.eval(b, &ctx) {
                    matched.push(b);
                }
            });
            if !matched.is_empty() {
                if want_recency {
                    recency.insert(entry.name.clone(), page_recency_secs(entry));
                }
                let blocks: Vec<BlockDto> = matched.into_iter().map(block_to_dto).collect();
                groups.push(RefGroup {
                    page: entry.name.clone(),
                    kind: entry.kind,
                    blocks,
                    evidence: Vec::new(),
                });
            }
        }
        (groups, recency)
    });

    // `with_pages` inherits filesystem/cache enumeration order. Make the base
    // order stable before sampling and before it becomes the tie-breaker for an
    // explicit sort; otherwise identical graph exports can differ by machine.
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
    groups
}

// --- Scoped-invalidation support (#52) --------------------------------------
// "Could an edit to page (entry, doc) change this derived result?" Each reuses
// the SAME parse + EvalCtx + eval (or alias resolution) as the real matcher, so
// the keep/evict decision can never drift from what a full recompute would give.

/// Whether page (entry, doc) contributes any block to query `src`.
pub(crate) fn page_affects_query(src: &str, entry: &PageEntry, doc: &Document) -> bool {
    let today = JournalDate::today();
    let Some(pred) = Pred::parse(src, today) else {
        return false;
    };
    let (page_props, page_tags) = page_facets(doc.pre_block.as_deref());
    let ctx = EvalCtx {
        journal: entry.date_key,
        is_journal: entry.kind == PageKind::Journal,
        page_name: &entry.name,
        page_props: &page_props,
        page_tags: &page_tags,
    };
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit && pred.eval(b, &ctx) {
            hit = true;
        }
    });
    hit
}

/// Whether page `doc` references `target` or any of its aliases — i.e. could be
/// in `backlinks(target)`. Mirrors `backlinks`'s alias resolution; takes the
/// resolved alias map so the caller needn't hold the graph lock.
pub(crate) fn page_affects_backlinks(
    aliases: &[(String, String)],
    target: &str,
    entry: &PageEntry,
    doc: &Document,
) -> bool {
    let (canonical, names_norm) = equivalent_page_names(aliases, target);
    if doc.pre_block.as_deref().is_some_and(|pre| {
        page_property_block(entry, pre).is_some_and(|block| {
            block_reference_evidence(&block, &canonical, &names_norm, ReferenceKind::Explicit)
                .is_some()
        })
    }) {
        return true;
    }
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && block_reference_evidence(b, &canonical, &names_norm, ReferenceKind::Explicit)
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
    aliases: &[(String, String)],
    target: &str,
    doc: &Document,
) -> bool {
    let (canonical, names_norm) = equivalent_page_names(aliases, target);
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && block_reference_evidence(b, &canonical, &names_norm, ReferenceKind::Plain).is_some()
        {
            hit = true;
        }
    });
    hit
}

/// Result of an advanced (datalog) query: matched groups + which clause heads
/// ran vs were ignored, so the UI shows "ran X; ignored Y" rather than a blunt
/// "unsupported". `supported` is false only when nothing in the subset matched.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdvancedResult {
    pub groups: Vec<RefGroup>,
    pub ran: Vec<String>,
    pub ignored: Vec<String>,
    pub supported: bool,
}

/// Run an advanced `[:find … :where …]` / `{:query … :inputs …}` query by mapping
/// the common clause subset (task / between / page-ref / property / page-property
/// / priority + and/or/not) onto the simple-DSL `Pred` engine — the matching
/// predicates already exist. Unrecognized clauses (custom rules, `[?e ?a ?v]`
/// joins, `:view`/`:result-transform`) are listed in `ignored` and skipped, never
/// guessed (a wrong result is worse than "unsupported").
pub fn run_advanced_query(
    graph: &Graph,
    query_src: &str,
    current_page: Option<&str>,
) -> AdvancedResult {
    let today = JournalDate::today();
    let inputs = resolve_inputs(query_src, current_page, today);
    let mut ran = Vec::new();
    let mut ignored = Vec::new();
    let preds: Vec<Pred> = where_groups(query_src)
        .iter()
        .filter_map(|g| parse_adv_group(g, &inputs, today, &mut ran, &mut ignored))
        .collect();
    if preds.is_empty() {
        return AdvancedResult {
            groups: Vec::new(),
            ran,
            ignored,
            supported: false,
        };
    }
    let pred = if preds.len() == 1 {
        preds.into_iter().next().unwrap()
    } else {
        Pred::And(preds)
    };
    let mut opts = QueryOpts::default();
    pred.collect_opts(&mut opts);
    let groups = run_pred(graph, &pred, &opts);
    AdvancedResult {
        groups,
        ran,
        ignored,
        supported: true,
    }
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
    inputs: &std::collections::HashMap<String, i64>,
    today: JournalDate,
    ran: &mut Vec<String>,
    ignored: &mut Vec<String>,
) -> Option<Pred> {
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
            let kids: Vec<Pred> = scan_groups(inner)
                .iter()
                .filter_map(|g| parse_adv_group(g, inputs, today, ran, ignored))
                .collect();
            if kids.is_empty() {
                None
            } else if head == "not" {
                Some(Pred::Not(Box::new(kids.into_iter().next().unwrap())))
            } else if head == "or" {
                Some(Pred::Or(kids))
            } else {
                Some(Pred::And(kids))
            }
        }
        "task" | "todo" => {
            ran.push("task".into());
            Some(Pred::Task(adv_strings(inner)))
        }
        "priority" => {
            ran.push("priority".into());
            Some(Pred::Priority(adv_strings(inner)))
        }
        "page-ref" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page-ref".into());
            Pred::PageRef(n)
        }),
        "property" | "page-property" => inner
            .split_whitespace()
            .skip(1)
            .find(|t| t.starts_with(':'))
            .map(|t| t.trim_start_matches(':').to_string())
            .map(|k| {
                let val = adv_strings(inner).into_iter().next();
                ran.push(head.clone());
                if head == "property" {
                    Pred::Property(k, val)
                } else {
                    Pred::PageProperty(k, val)
                }
            }),
        "page" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page".into());
            Pred::Page(n)
        }),
        "namespace" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("namespace".into());
            Pred::Namespace(n)
        }),
        "page-tags" | "tags" => {
            let ts = adv_strings(inner);
            if ts.is_empty() {
                ignored.push(head.clone());
                None
            } else {
                ran.push("page-tags".into());
                Some(Pred::PageTags(ts))
            }
        }
        "scheduled" => {
            ran.push("scheduled".into());
            Some(Pred::Scheduled)
        }
        "deadline" => {
            ran.push("deadline".into());
            Some(Pred::Deadline)
        }
        "journal" => {
            ran.push("journal".into());
            Some(Pred::Journal)
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
            let field = args
                .iter()
                .take(args.len() - 2)
                .find_map(
                    |a| match a.trim_start_matches(':').to_ascii_lowercase().as_str() {
                        "scheduled" => Some(BetweenField::Scheduled),
                        "deadline" => Some(BetweenField::Deadline),
                        "journal" => Some(BetweenField::Journal),
                        _ => None,
                    },
                )
                .unwrap_or(BetweenField::Journal);
            let lo = adv_bound(args[args.len() - 2], inputs, today);
            let hi = adv_bound(args[args.len() - 1], inputs, today);
            if lo.is_none() && hi.is_none() {
                ignored.push("between".into());
                return None;
            }
            ran.push("between".into());
            Some(Pred::Between(field, lo, hi))
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
    inputs: &std::collections::HashMap<String, i64>,
    today: JournalDate,
) -> Option<i64> {
    let t = tok.trim();
    if t.starts_with('?') {
        return inputs.get(t).copied();
    }
    // A literal bound may be written as a bare token (`2026-06-24`) or a quoted
    // string (`"2026-06-24"`); `split_whitespace` keeps the quotes, so strip them.
    resolve_date_token(t.trim_matches('"').trim_start_matches(':'), today)
}

/// Build a `?var → yyyymmdd` map by zipping `:in $ ?a ?b …` var names with the
/// `:inputs [ … ]` values (Logseq's positional binding). Only date inputs resolve
/// to an ordinal; others (e.g. `:current-page`) are skipped — their pattern
/// clause is ignored anyway.
fn resolve_inputs(
    src: &str,
    _current_page: Option<&str>,
    today: JournalDate,
) -> std::collections::HashMap<String, i64> {
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
        if let Some(ord) = resolve_date_token(val.trim_start_matches(':'), today) {
            map.insert(v.clone(), ord);
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
            if let Some((_, v)) = b
                .properties
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(field))
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

/// Full-text search: blocks whose visible text matches `query` under the Ctrl-K
/// search dialect (whitespace=AND, `OR`, `-exclude`, `"phrase"`, `/regex/`; see
/// [`crate::search_query`]), grouped by page, capped at `limit` total blocks.
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
    let plan = crate::query_plan::QueryPlan::block_search(query, limit);
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
        ..Default::default()
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

/// Distinct property keys (each with its sorted distinct values) used across the
/// graph. Drives the query builder's property-filter pickers.
pub fn property_facets(graph: &Graph) -> Vec<(String, Vec<String>)> {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    graph.with_pages(|pages| {
        let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (_entry, doc) in pages {
            walk(&doc.roots, &mut |b| {
                for (k, v) in b.properties() {
                    if INTERNAL_PROPS.iter().any(|p| p.eq_ignore_ascii_case(&k)) {
                        continue;
                    }
                    let set = map.entry(k).or_default();
                    if !v.trim().is_empty() {
                        set.insert(v);
                    }
                }
            });
        }
        map.into_iter()
            .map(|(k, vs)| (k, vs.into_iter().collect()))
            .collect()
    })
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

#[cfg(test)]
fn score_name(name: &str, q: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    if name.starts_with(q) {
        Some(1000)
    } else if name.contains(q) {
        Some(500)
    } else if is_subsequence(q, name) {
        Some(100)
    } else {
        None
    }
}

#[cfg(test)]
fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// Resolve a `((uuid))` block reference to its block (with subtree).
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
                blocks: vec![block_to_dto(b)],
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
    use std::collections::{HashMap, HashSet};
    // Distinct requested ids (a page often refs the same uuid repeatedly).
    let distinct: HashSet<&str> = uuids.iter().map(String::as_str).collect();
    if distinct.is_empty() {
        return uuids.iter().map(|_| None).collect();
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
                resolve_ids_in_page(entry, doc, &want, &mut resolved);
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
                resolve_ids_in_page(entry, doc, &remaining, &mut resolved);
            }
        }
    });

    uuids
        .iter()
        .map(|u| resolved.get(u.as_str()).cloned())
        .collect()
}

/// Walk `doc` once, resolving any block whose uuid (or persisted `id::`) is a
/// still-unresolved id in `want`. First block in walk order wins per id (matches
/// `resolve_block`).
fn resolve_ids_in_page<'a>(
    entry: &PageEntry,
    doc: &Document,
    want: &std::collections::HashSet<&'a str>,
    resolved: &mut std::collections::HashMap<&'a str, RefGroup>,
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
            resolved.insert(
                id,
                RefGroup {
                    page: entry.name.clone(),
                    kind: entry.kind,
                    blocks: vec![block_to_dto(b)],
                    evidence: Vec::new(),
                },
            );
        }
    });
}

/// Is this query body an advanced datalog query we don't support?
pub fn is_advanced(query_src: &str) -> bool {
    let s = query_src.trim_start();
    s.starts_with("[:find") || s.contains(":where") || s.contains(":find")
}

// ---------------------------------------------------------------------------
// Query predicate AST + parser + evaluator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Pred {
    PageRef(String),
    Task(Vec<String>),
    Priority(Vec<String>),
    Property(String, Option<String>),
    Scheduled,
    Deadline,
    /// Block lives on a journal page.
    Journal,
    /// Date range (inclusive) over a chosen date field. Bounds are `yyyymmdd`
    /// ordinals; `None` = open.
    Between(BetweenField, Option<i64>, Option<i64>),
    /// Blocks on a specific page (by name).
    Page(String),
    /// Pages whose name is under a namespace (`ns/…`).
    Namespace(String),
    /// A page-level property (on the page's pre-block).
    PageProperty(String, Option<String>),
    /// Page has any of these `tags::`.
    PageTags(Vec<String>),
    /// Full-text match on the block's visible content.
    Content(String),
    /// The friendly Ctrl-K search language, explicitly embedded in the durable
    /// query DSL. The decoded source is retained exactly and compiled once when
    /// the surrounding query is parsed.
    Search(FriendlySearch),
    /// A case-sensitive Rust regex over the block's projected visible content.
    /// Invalid patterns are retained but deliberately match nothing.
    ContentRegex(ContentRegex),
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
    /// Result-level options (always pass as filters; collected as `QueryOpts`).
    Sample(usize),
    SortBy(String, bool),
    /// Result-level aggregation, computed in the FRONTEND from the returned block
    /// list (D1). Parsed-but-ignored here (eval → true) so `run_query` succeeds and
    /// the builder DSL round-trips; the frontend re-parses the same DSL to render it.
    Aggregate(AggKind),
    /// Result-level grouping (`(group-by page|<prop>)`), also frontend-computed.
    GroupBy(String),
}

/// Compiled `(search "...")` predicate. Equality intentionally compares the
/// lossless decoded source rather than the matcher's internal representation;
/// this keeps parser tests useful without making the shared matcher API expose
/// implementation details.
#[derive(Clone)]
struct FriendlySearch {
    source: String,
    matcher: Matcher,
}

impl FriendlySearch {
    fn new(source: String) -> Self {
        let matcher = Matcher::parse(&source);
        Self { source, matcher }
    }

    fn matches(&self, block: &DocBlock) -> bool {
        let projection = block.projection();
        self.matcher
            .matches(&projection.visible_lower, &projection.visible)
    }
}

impl std::fmt::Debug for FriendlySearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FriendlySearch").field(&self.source).finish()
    }
}

impl PartialEq for FriendlySearch {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

/// Compiled `(content-regex "...")` predicate. Keeping an invalid pattern as
/// `None` makes its behavior deterministic (no panic, no accidental match-all)
/// while retaining the original source for diagnostics and round-tripping.
#[derive(Clone)]
struct ContentRegex {
    source: String,
    compiled: Option<regex::Regex>,
}

impl ContentRegex {
    fn new(source: String) -> Self {
        let compiled = regex::Regex::new(&source).ok();
        Self { source, compiled }
    }

    fn matches(&self, block: &DocBlock) -> bool {
        self.compiled
            .as_ref()
            .is_some_and(|regex| regex.is_match(&block.projection().visible))
    }
}

impl std::fmt::Debug for ContentRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentRegex")
            .field("source", &self.source)
            .field("valid", &self.compiled.is_some())
            .finish()
    }
}

impl PartialEq for ContentRegex {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

/// A result aggregation directive. `Sum`/`Avg` carry the property whose numeric
/// values are combined; `Count` needs no field.
#[derive(Debug, Clone, PartialEq)]
enum AggKind {
    Count,
    Sum(String),
    Avg(String),
}

/// Which date a `between` range is tested against.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BetweenField {
    /// Journal date OR scheduled OR deadline (Tine's permissive default;
    /// fieldless `(between …)` keeps this for back-compat).
    Any,
    /// The page's journal date only — implies journal pages, matching OG's
    /// `:between` rule (`:block/journal? true`).
    Journal,
    Scheduled,
    Deadline,
}

/// Result-level options extracted from the query (sample, sort-by).
#[derive(Debug, Default, Clone)]
struct QueryOpts {
    sample: Option<usize>,
    sort: Option<(String, bool)>, // (field, ascending)
}

/// Per-block evaluation context (the page it lives on).
struct EvalCtx<'a> {
    /// The page's journal-day ordinal (`yyyymmdd`), or `None` for named pages.
    journal: Option<i64>,
    /// Whether the block lives on a journal page (drives `(journal)`).
    is_journal: bool,
    page_name: &'a str,
    page_props: &'a [(String, String)],
    page_tags: &'a [String],
}

fn date_ordinal(y: i64, m: i64, d: i64) -> i64 {
    y * 10000 + m * 100 + d
}

/// Parse an org timestamp body like `<2026-06-15 Mon>` to a `yyyymmdd` ordinal.
fn parse_angle_date(s: &str) -> Option<i64> {
    let s = s.trim().strip_prefix('<')?;
    let end = s.find([' ', '>']).unwrap_or(s.len());
    let mut it = s[..end].split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    Some(date_ordinal(y, m, d))
}

/// Ordinals from a block's SCHEDULED:/DEADLINE: lines. `only` restricts to one
/// marker (`"SCHEDULED:"` / `"DEADLINE:"`); `None` returns both.
fn block_date_ordinals(raw: &str, only: Option<&str>) -> Vec<i64> {
    // Match the planning marker ANYWHERE on a line (not just at line start), so an
    // inline `TODO SCHEDULED: <…> do the thing` is found too — consistent with the
    // lenient render (block.ts) and `Pred::Scheduled`'s `raw.contains`. The angle
    // date is parsed from just after the marker; trailing text is ignored.
    let want_sched = !matches!(only, Some("DEADLINE:"));
    let want_dead = !matches!(only, Some("SCHEDULED:"));
    let mut out = Vec::new();
    for line in raw.lines() {
        if want_sched {
            if let Some(i) = line.find("SCHEDULED:") {
                if let Some(o) = parse_angle_date(&line[i + "SCHEDULED:".len()..]) {
                    out.push(o);
                }
            }
        }
        if want_dead {
            if let Some(i) = line.find("DEADLINE:") {
                if let Some(o) = parse_angle_date(&line[i + "DEADLINE:".len()..]) {
                    out.push(o);
                }
            }
        }
    }
    out
}

impl Pred {
    fn parse(src: &str, today: JournalDate) -> Option<Pred> {
        if is_advanced(src) {
            return None;
        }
        let tokens = tokenize(src);
        let mut pos = 0;
        let p = parse_expr(&tokens, &mut pos, today)?;
        Some(p)
    }

    /// Pull result-level options (sample / sort-by) out of the tree.
    fn collect_opts(&self, opts: &mut QueryOpts) {
        match self {
            Pred::Sample(n) => opts.sample = Some(*n),
            Pred::SortBy(f, asc) => opts.sort = Some((f.clone(), *asc)),
            Pred::And(ps) | Pred::Or(ps) => ps.iter().for_each(|p| p.collect_opts(opts)),
            Pred::Not(p) => p.collect_opts(opts),
            _ => {}
        }
    }

    fn eval(&self, block: &DocBlock, ctx: &EvalCtx) -> bool {
        match self {
            Pred::PageRef(name) => block.projection().refs_contains(name),
            Pred::Task(markers) => block
                .marker()
                .map(|m| markers.iter().any(|x| x.eq_ignore_ascii_case(m)))
                .unwrap_or(false),
            Pred::Priority(ps) => block
                .priority()
                .map(|p| ps.iter().any(|x| x.eq_ignore_ascii_case(p)))
                .unwrap_or(false),
            Pred::Property(key, val) => block
                .properties()
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case(key) && value_matches(v, val.as_deref())),
            Pred::Scheduled => block.raw.contains("SCHEDULED:"),
            Pred::Deadline => block.raw.contains("DEADLINE:"),
            Pred::Journal => ctx.is_journal,
            Pred::Between(field, lo, hi) => {
                let in_range = |c: i64| lo.map_or(true, |l| c >= l) && hi.map_or(true, |h| c <= h);
                match field {
                    BetweenField::Any => {
                        ctx.journal.is_some_and(in_range)
                            || block_date_ordinals(&block.raw, None)
                                .into_iter()
                                .any(in_range)
                    }
                    BetweenField::Journal => ctx.journal.is_some_and(in_range),
                    BetweenField::Scheduled => block_date_ordinals(&block.raw, Some("SCHEDULED:"))
                        .into_iter()
                        .any(in_range),
                    BetweenField::Deadline => block_date_ordinals(&block.raw, Some("DEADLINE:"))
                        .into_iter()
                        .any(in_range),
                }
            }
            Pred::Page(name) => refs::normalize(ctx.page_name) == refs::normalize(name),
            Pred::Namespace(ns) => {
                let p = refs::normalize(ctx.page_name);
                let n = refs::normalize(ns);
                p.starts_with(&format!("{n}/"))
            }
            Pred::PageProperty(key, val) => ctx
                .page_props
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case(key) && value_matches(v, val.as_deref())),
            Pred::PageTags(tags) => tags
                .iter()
                .any(|t| ctx.page_tags.iter().any(|pt| pt.eq_ignore_ascii_case(t))),
            // `s` is already lowercased at parse time; `visible_lower` is the
            // block's lowercased visible text — a direct substring test.
            Pred::Content(s) => block.projection().visible_lower.contains(s.as_str()),
            Pred::Search(search) => search.matches(block),
            Pred::ContentRegex(regex) => regex.matches(block),
            Pred::And(ps) => ps.iter().all(|p| p.eval(block, ctx)),
            Pred::Or(ps) => ps.iter().any(|p| p.eval(block, ctx)),
            Pred::Not(p) => !p.eval(block, ctx),
            // Options and frontend-computed directives are not filters.
            Pred::Sample(_) | Pred::SortBy(..) | Pred::Aggregate(_) | Pred::GroupBy(_) => true,
        }
    }
}

/// Match a stored property value against a query value. Handles multi-value
/// (comma-separated) and page-ref / tag wrapping, case-insensitively. A `None`
/// query value matches any present value.
fn value_matches(stored: &str, query: Option<&str>) -> bool {
    let Some(q) = query else { return true };
    let q = strip_ref(q).to_lowercase();
    stored
        .split(',')
        .map(|p| strip_ref(p.trim()).to_lowercase())
        .any(|v| v == q)
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

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LParen,
    RParen,
    PageRef(String), // [[...]]
    Tag(String),     // #...
    Word(String),
    Str(String),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut i = 0;
    let chars: Vec<char> = src.chars().collect();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '(' {
            toks.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            toks.push(Tok::RParen);
            i += 1;
        } else if c == '[' && i + 1 < chars.len() && chars[i + 1] == '[' {
            // [[ ... ]]
            let mut j = i + 2;
            let mut name = String::new();
            while j + 1 < chars.len() && !(chars[j] == ']' && chars[j + 1] == ']') {
                name.push(chars[j]);
                j += 1;
            }
            toks.push(Tok::PageRef(name));
            i = j + 2;
        } else if c == '#' {
            if i + 2 < chars.len() && chars[i + 1] == '[' && chars[i + 2] == '[' {
                let mut j = i + 3;
                let mut name = String::new();
                while j + 1 < chars.len() && !(chars[j] == ']' && chars[j + 1] == ']') {
                    name.push(chars[j]);
                    j += 1;
                }
                toks.push(Tok::Tag(name));
                i = j + 2;
            } else {
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len()
                    && (chars[j].is_alphanumeric() || matches!(chars[j], '-' | '_' | '/' | '.'))
                {
                    name.push(chars[j]);
                    j += 1;
                }
                toks.push(Tok::Tag(name));
                i = j;
            }
        } else if c == '"' {
            let mut j = i + 1;
            let mut s = String::new();
            // Escape-aware: ONLY `\"` and `\\` are escapes (→ literal quote/
            // backslash), so a quote inside the value doesn't end the string
            // early. A backslash before any other char is kept literally, so a
            // hand-authored path like `"C:\tmp"` round-trips unchanged (mirrors
            // the frontend query-builder tokenizer + serializer's quoteStr).
            while j < chars.len() && chars[j] != '"' {
                if chars[j] == '\\' && matches!(chars.get(j + 1), Some('"') | Some('\\')) {
                    s.push(chars[j + 1]);
                    j += 2;
                } else {
                    s.push(chars[j]);
                    j += 1;
                }
            }
            toks.push(Tok::Str(s));
            i = j + 1;
        } else {
            let mut j = i;
            let mut w = String::new();
            while j < chars.len() && !chars[j].is_whitespace() && !matches!(chars[j], '(' | ')') {
                w.push(chars[j]);
                j += 1;
            }
            toks.push(Tok::Word(w));
            i = j;
        }
    }
    toks
}

fn parse_expr(toks: &[Tok], pos: &mut usize, today: JournalDate) -> Option<Pred> {
    let t = toks.get(*pos)?.clone();
    match t {
        Tok::PageRef(name) | Tok::Tag(name) => {
            *pos += 1;
            Some(Pred::PageRef(name))
        }
        // A bare quoted string is a full-text content filter. Fold to lowercase
        // ONCE here (the match is case-insensitive) so the per-block evaluator
        // compares against an already-lowered term instead of re-lowering the
        // constant query string for every candidate block (perf Codex#7).
        Tok::Str(s) => {
            *pos += 1;
            Some(Pred::Content(s.to_lowercase()))
        }
        Tok::LParen => {
            *pos += 1; // consume (
            let head = match toks.get(*pos)? {
                Tok::Word(w) => w.to_lowercase(),
                _ => return None,
            };
            *pos += 1;
            let pred = match head.as_str() {
                "and" => Pred::And(parse_list(toks, pos, today)),
                "or" => Pred::Or(parse_list(toks, pos, today)),
                "not" => Pred::Not(Box::new(parse_expr(toks, pos, today)?)),
                "task" | "todo" => {
                    let markers = parse_words(toks, pos);
                    // `(todo)` with no args means any open task.
                    if markers.is_empty() {
                        Pred::Task(vec![
                            "TODO".into(),
                            "DOING".into(),
                            "NOW".into(),
                            "LATER".into(),
                        ])
                    } else {
                        Pred::Task(markers)
                    }
                }
                "priority" => {
                    let ps = parse_words(toks, pos);
                    if ps.is_empty() {
                        Pred::Priority(vec!["A".into(), "B".into(), "C".into()])
                    } else {
                        Pred::Priority(ps)
                    }
                }
                "page-ref" | "tag" => {
                    let name = parse_name(toks, pos)?;
                    Pred::PageRef(name)
                }
                "page" => {
                    let name = parse_name(toks, pos)?;
                    Pred::Page(name)
                }
                "namespace" => {
                    let name = parse_name(toks, pos)?;
                    Pred::Namespace(name)
                }
                "property" => {
                    let key = normalize_prop_key(&parse_name(toks, pos)?);
                    let val = parse_opt_value(toks, pos);
                    Pred::Property(key, val)
                }
                "page-property" => {
                    let key = normalize_prop_key(&parse_name(toks, pos)?);
                    let val = parse_opt_value(toks, pos);
                    Pred::PageProperty(key, val)
                }
                "page-tags" | "tags" => Pred::PageTags(parse_words(toks, pos)),
                "search" => Pred::Search(FriendlySearch::new(parse_name(toks, pos)?)),
                "content-regex" => Pred::ContentRegex(ContentRegex::new(parse_name(toks, pos)?)),
                "scheduled" => Pred::Scheduled,
                "deadline" => Pred::Deadline,
                "journal" => Pred::Journal,
                "between" => {
                    // (between [FIELD] START END): optional leading field keyword
                    // journal|scheduled|deadline (default Any = journal-or-
                    // scheduled-or-deadline); bounds are journal titles,
                    // `today`/`yesterday`/`tomorrow`, signed durations `±N[dwmy]`,
                    // or `yyyy-MM-dd`.
                    let field = match toks.get(*pos) {
                        Some(Tok::Word(w)) => match w.to_ascii_lowercase().as_str() {
                            "journal" => {
                                *pos += 1;
                                BetweenField::Journal
                            }
                            "scheduled" => {
                                *pos += 1;
                                BetweenField::Scheduled
                            }
                            "deadline" => {
                                *pos += 1;
                                BetweenField::Deadline
                            }
                            _ => BetweenField::Any,
                        },
                        _ => BetweenField::Any,
                    };
                    let lo = parse_name(toks, pos).and_then(|s| resolve_date_token(&s, today));
                    let hi = parse_name(toks, pos).and_then(|s| resolve_date_token(&s, today));
                    Pred::Between(field, lo, hi)
                }
                "sample" => {
                    let n = parse_name(toks, pos)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);
                    Pred::Sample(n)
                }
                "sort-by" => {
                    let field = parse_name(toks, pos).unwrap_or_default();
                    let dir = parse_opt_name(toks, pos).unwrap_or_else(|| "asc".into());
                    Pred::SortBy(field, !dir.eq_ignore_ascii_case("desc"))
                }
                // Frontend-computed result directives (D1/D2): parse-but-ignore so
                // run_query succeeds and the builder round-trips the DSL text.
                "aggregate" => {
                    let kind = match parse_name(toks, pos) {
                        Some(k) => match k.to_ascii_lowercase().as_str() {
                            "sum" => AggKind::Sum(parse_name(toks, pos).unwrap_or_default()),
                            "avg" | "average" => {
                                AggKind::Avg(parse_name(toks, pos).unwrap_or_default())
                            }
                            _ => AggKind::Count,
                        },
                        None => AggKind::Count,
                    };
                    Pred::Aggregate(kind)
                }
                "group-by" => Pred::GroupBy(parse_name(toks, pos).unwrap_or_else(|| "page".into())),
                _ => return None,
            };
            // consume closing )
            if let Some(Tok::RParen) = toks.get(*pos) {
                *pos += 1;
            }
            Some(pred)
        }
        _ => None,
    }
}

fn parse_list(toks: &[Tok], pos: &mut usize, today: JournalDate) -> Vec<Pred> {
    let mut out = Vec::new();
    while let Some(t) = toks.get(*pos) {
        if *t == Tok::RParen {
            break;
        }
        match parse_expr(toks, pos, today) {
            Some(p) => out.push(p),
            None => break,
        }
    }
    out
}

fn parse_words(toks: &[Tok], pos: &mut usize) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(t) = toks.get(*pos) {
        match t {
            Tok::Word(w) => {
                out.push(w.clone());
                *pos += 1;
            }
            Tok::Str(s) => {
                out.push(s.clone());
                *pos += 1;
            }
            Tok::Tag(s) | Tok::PageRef(s) => {
                out.push(s.clone());
                *pos += 1;
            }
            _ => break,
        }
    }
    out
}

fn parse_name(toks: &[Tok], pos: &mut usize) -> Option<String> {
    match toks.get(*pos)?.clone() {
        Tok::Word(w) => {
            *pos += 1;
            Some(w)
        }
        Tok::Str(s) => {
            *pos += 1;
            Some(s)
        }
        Tok::PageRef(s) | Tok::Tag(s) => {
            *pos += 1;
            Some(s)
        }
        _ => None,
    }
}

fn parse_opt_name(toks: &[Tok], pos: &mut usize) -> Option<String> {
    match toks.get(*pos) {
        Some(Tok::Word(_)) | Some(Tok::Str(_)) => parse_name(toks, pos),
        _ => None,
    }
}

/// A property KEY, normalized the way Logseq's query DSL does (`(name k)` then
/// `_`→`-`, see query_dsl.cljs build-property-two-arg): drop a leading `:` so the
/// keyword form `:fach` and the symbol form `fach` both mean `fach`, and map
/// underscores to dashes (Logseq stores `my_key` as `my-key`). WITHOUT this the
/// simple parser kept `:fach` verbatim and it never matched the stored key `fach`.
fn normalize_prop_key(k: &str) -> String {
    k.trim_start_matches(':').replace('_', "-")
}

/// Optional property VALUE: like `parse_opt_name`, but also accepts a `[[page]]`
/// or `#tag` token (Logseq's parse-property-value extracts the page name and
/// strips a leading `#`; `value_matches` does the ref/tag stripping on both
/// sides). WITHOUT this, `(property k [[Page]])` / `(property k #tag)` dropped the
/// value AND leaked the ref token, which was then mis-parsed as a stray page-ref
/// clause — the second reported failure mode.
fn parse_opt_value(toks: &[Tok], pos: &mut usize) -> Option<String> {
    match toks.get(*pos) {
        Some(Tok::Word(_)) | Some(Tok::Str(_)) | Some(Tok::PageRef(_)) | Some(Tok::Tag(_)) => {
            parse_name(toks, pos)
        }
        _ => None,
    }
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

    fn pred(src: &str) -> Pred {
        Pred::parse(src, TODAY).expect("parse")
    }

    /// A minimal eval context for a block on a named (non-journal) page.
    fn ctx_named<'a>() -> EvalCtx<'a> {
        EvalCtx {
            journal: None,
            is_journal: false,
            page_name: "Test",
            page_props: &[],
            page_tags: &[],
        }
    }
    fn ctx_journal<'a>(key: i64) -> EvalCtx<'a> {
        EvalCtx {
            journal: Some(key),
            is_journal: true,
            page_name: "Journal",
            page_props: &[],
            page_tags: &[],
        }
    }

    #[test]
    fn parse_pageref_and_tag() {
        assert_eq!(pred("[[Foo]]"), Pred::PageRef("Foo".into()));
        assert_eq!(pred("#bar"), Pred::PageRef("bar".into()));
        assert_eq!(pred("(tag Foo)"), Pred::PageRef("Foo".into()));
    }

    #[test]
    fn parse_boolean() {
        assert_eq!(
            pred("(and [[A]] [[B]])"),
            Pred::And(vec![Pred::PageRef("A".into()), Pred::PageRef("B".into())])
        );
        assert_eq!(
            pred("(not [[A]])"),
            Pred::Not(Box::new(Pred::PageRef("A".into())))
        );
    }

    #[test]
    fn parse_task_and_property() {
        assert_eq!(
            pred("(task TODO DOING)"),
            Pred::Task(vec!["TODO".into(), "DOING".into()])
        );
        assert_eq!(
            pred("(property type book)"),
            Pred::Property("type".into(), Some("book".into()))
        );
        assert_eq!(
            pred("(property public)"),
            Pred::Property("public".into(), None)
        );
    }

    #[test]
    fn property_key_and_ref_value_match_logseq() {
        // Leading `:` on the key is stripped (keyword form == symbol form).
        assert_eq!(
            pred("(property :type book)"),
            Pred::Property("type".into(), Some("book".into()))
        );
        // `_` → `-` (Logseq stores `my_key` as `my-key`).
        assert_eq!(
            pred("(property my_key v)"),
            Pred::Property("my-key".into(), Some("v".into()))
        );
        // A `[[page]]` value is captured (was dropped, leaking a stray page-ref).
        assert_eq!(
            pred("(property :fach [[Foo Bar]])"),
            Pred::Property("fach".into(), Some("Foo Bar".into()))
        );
        // A `#tag` value is captured.
        assert_eq!(
            pred("(property :type #assignment)"),
            Pred::Property("type".into(), Some("assignment".into()))
        );
        // page-property mirrors the same normalization + value capture.
        assert_eq!(
            pred("(page-property :fach [[Foo]])"),
            Pred::PageProperty("fach".into(), Some("Foo".into()))
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
            Pred::And(vec![
                Pred::Property(
                    "fach".into(),
                    Some("Management der digitalen Transformation".into())
                ),
                Pred::Property("type".into(), Some("#assignment".into())),
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
        assert_eq!(
            pred("\"foo \\\"bar\\\"\""),
            Pred::Content("foo \"bar\"".into())
        );
        assert_eq!(pred("\"a\\\\b\""), Pred::Content("a\\b".into()));
        // Only `\"`/`\\` are escapes: a hand-authored backslash before another
        // char is literal, so `"C:\tmp"` stays `C:\tmp` (not `C:tmp`). The term
        // is case-folded at parse time (the content match is case-insensitive).
        assert_eq!(pred("\"a\\q\""), Pred::Content("a\\q".into()));
        assert_eq!(pred("\"C:\\tmp\""), Pred::Content("c:\\tmp".into()));
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
            Pred::Search(FriendlySearch::new(
                r#"foo "exact phrase" -draft OR C:\tmp"#.into()
            ))
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
            Pred::ContentRegex(ContentRegex::new(r#"ID:\s+[A-Z]{3}\d+\s+"quoted""#.into()))
        );

        let none = ctx_named();
        assert!(parsed.eval(&DocBlock::new(r#"prefix ID: ABC42 "quoted" suffix"#), &none));
        // Rust regex matching is intentionally case-sensitive.
        assert!(!parsed.eval(&DocBlock::new(r#"prefix ID: abc42 "quoted" suffix"#), &none));

        let invalid = pred(r#"(content-regex "[unclosed")"#);
        assert!(matches!(invalid, Pred::ContentRegex(_)));
        assert!(!invalid.eval(&DocBlock::new("[unclosed"), &none));
    }

    #[test]
    fn aggregate_and_group_by_parse_as_noop_filters() {
        // 1a: the aggregation/grouping directives ride in the DSL (D2) so the
        // builder round-trips and run_query succeeds; they never filter (eval→true).
        assert_eq!(pred("(aggregate count)"), Pred::Aggregate(AggKind::Count));
        assert_eq!(
            pred("(aggregate sum hours)"),
            Pred::Aggregate(AggKind::Sum("hours".into()))
        );
        assert_eq!(
            pred("(aggregate avg score)"),
            Pred::Aggregate(AggKind::Avg("score".into()))
        );
        assert_eq!(pred("(group-by page)"), Pred::GroupBy("page".into()));
        assert_eq!(pred("(group-by status)"), Pred::GroupBy("status".into()));

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
        assert!(Pred::parse("[:find ?b :where ...]", TODAY).is_none());
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
        assert!(q.eval(&sched, &ctx_named()));
    }

    #[test]
    fn eval_between_relative_dates() {
        // TODAY = 2026-06-16. (between -7d +7d) => [2026-06-09, 2026-06-23].
        let q = pred("(between -7d +7d)");
        assert_eq!(
            q,
            Pred::Between(BetweenField::Any, Some(20260609), Some(20260623))
        );
        let b = DocBlock::new("x");
        assert!(q.eval(&b, &ctx_journal(20260616)));
        assert!(q.eval(&b, &ctx_journal(20260609)));
        assert!(!q.eval(&b, &ctx_journal(20260601)));
        // keyword bounds + month/year units
        assert_eq!(
            pred("(between today tomorrow)"),
            Pred::Between(BetweenField::Any, Some(20260616), Some(20260617))
        );
        assert_eq!(
            pred("(between -1m +1y)"),
            Pred::Between(BetweenField::Any, Some(20260516), Some(20270616))
        );
    }

    #[test]
    fn between_field_selector_and_journal_only() {
        // Field keyword parses into the right variant.
        assert_eq!(
            pred("(between journal -30d today)"),
            Pred::Between(BetweenField::Journal, Some(20260517), Some(20260616))
        );
        assert_eq!(
            pred("(between scheduled -7d +7d)"),
            Pred::Between(BetweenField::Scheduled, Some(20260609), Some(20260623))
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
        assert_eq!(pred("(journal)"), Pred::Journal);
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
        let ctx = EvalCtx {
            journal: None,
            is_journal: false,
            page_name: "Project/Alpha",
            page_props: &[],
            page_tags: &[],
        };
        assert!(pred("(page Project/Alpha)").eval(&b, &ctx));
        assert!(!pred("(page Project/Beta)").eval(&b, &ctx));
        assert!(pred("(namespace Project)").eval(&b, &ctx));
        assert!(!pred("(namespace Other)").eval(&b, &ctx));
    }

    #[test]
    fn eval_page_property_and_tags() {
        let b = DocBlock::new("hi");
        let props = vec![("type".to_string(), "project".to_string())];
        let tags = vec!["research".to_string(), "active".to_string()];
        let ctx = EvalCtx {
            journal: None,
            is_journal: false,
            page_name: "P",
            page_props: &props,
            page_tags: &tags,
        };
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
    fn parse_extracts_options() {
        let mut opts = QueryOpts::default();
        pred("(and (task TODO) (sample 5) (sort-by priority desc))").collect_opts(&mut opts);
        assert_eq!(opts.sample, Some(5));
        assert_eq!(opts.sort, Some(("priority".to_string(), false)));
    }

    #[test]
    fn block_date_ordinals_finds_planning_markers_anywhere() {
        let ord = date_ordinal(2026, 7, 6);
        // own line (after trim)
        assert_eq!(
            block_date_ordinals("TODO x\nSCHEDULED: <2026-07-06 Mon>", Some("SCHEDULED:")),
            vec![ord]
        );
        // inline on the marker line (the regressed case)
        assert_eq!(
            block_date_ordinals("TODO SCHEDULED: <2026-07-06 Mon> do it", Some("SCHEDULED:")),
            vec![ord]
        );
        // with trailing text after the timestamp
        assert_eq!(
            block_date_ordinals(
                " SCHEDULED: <2026-07-06 Mon> #email students",
                Some("SCHEDULED:")
            ),
            vec![ord]
        );
        // DEADLINE restricted; SCHEDULED-only query ignores it
        assert!(block_date_ordinals("DEADLINE: <2026-07-06 Mon>", Some("SCHEDULED:")).is_empty());
        assert_eq!(
            block_date_ordinals("DEADLINE: <2026-07-06 Mon>", Some("DEADLINE:")),
            vec![ord]
        );
    }

    fn quick_switch_fingerprint(entries: Vec<PageEntry>) -> Vec<(String, PageKind, String)> {
        entries
            .into_iter()
            .map(|e| (e.name, e.kind, e.rel_path))
            .collect()
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
            2,
            "alias + title; code excluded"
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
        assert_eq!(source_trace.occurrences.len(), 3);
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("launcher-ranking"));
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
}
