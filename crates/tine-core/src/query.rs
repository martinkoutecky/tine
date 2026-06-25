//! Backlinks and the `{{query}}` subset engine. Evaluated by scanning parsed
//! pages (no datalog). Pragmatic subset: page/tag refs, boolean and/or/not,
//! task markers, and property filters. Advanced datalog (`[:find ...]`) is
//! detected and reported as unsupported rather than crashed.

use crate::date::JournalDate;
use crate::doc::{DocBlock, Document};
use crate::model::{block_to_dto, BlockDto, Graph, PageEntry, PageKind, RefGroup, TemplateDto};
use crate::refs;

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

/// A short, single-line label for a block in a breadcrumb trail.
fn crumb_line(b: &DocBlock) -> String {
    let line = visible_text(&b.raw).lines().next().unwrap_or("").trim().to_string();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line
    }
}

/// Collect matching blocks across the graph, grouped by source page. Scans the
/// graph's in-memory page cache (built once, kept in sync by edits) so no disk
/// I/O or re-parsing happens per call.
fn collect(graph: &Graph, mut keep: impl FnMut(&DocBlock) -> bool, exclude: Option<&str>) -> Vec<RefGroup> {
    let ex = exclude.map(refs::normalize);
    graph.with_pages(|pages| {
        let mut groups: Vec<RefGroup> = Vec::new();
        for (entry, doc) in pages {
            if ex.as_deref() == Some(&refs::normalize(&entry.name)) {
                continue;
            }
            let mut matched: Vec<BlockDto> = Vec::new();
            let mut path: Vec<&DocBlock> = Vec::new();
            walk_path(&doc.roots, &mut path, &mut |b, anc| {
                if keep(b) {
                    let mut dto = block_to_dto(b);
                    dto.breadcrumb = anc.iter().map(|a| crumb_line(a)).collect();
                    matched.push(dto);
                }
            });
            if !matched.is_empty() {
                groups.push(RefGroup { page: entry.name.clone(), kind: entry.kind, blocks: matched });
            }
        }
        groups
    })
}

/// Map of `alias::` → canonical page name (original case), scanned from every
/// page's pre-block. The alias key is normalized for lookup.
pub fn page_aliases(graph: &Graph) -> Vec<(String, String)> {
    graph.with_pages(|pages| {
        let mut out: Vec<(String, String)> = Vec::new();
        for (entry, doc) in pages {
            let Some(pre) = &doc.pre_block else { continue };
            for line in pre.lines() {
                if let Some((k, v)) = crate::doc::parse_property_line(line) {
                    if k.eq_ignore_ascii_case("alias") {
                        for a in v.split(',') {
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

pub fn backlinks(graph: &Graph, target: &str) -> Vec<RefGroup> {
    // Alias-aware: a page's backlinks include references made through any of its
    // aliases, and looking up an alias resolves to its canonical page. Use the
    // cached alias map rather than rescanning every page on each backlink call.
    let aliases = graph.page_aliases();
    let tnorm = refs::normalize(target);
    let canonical = aliases
        .iter()
        .find(|(a, _)| *a == tnorm)
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| target.to_string());
    let cnorm = refs::normalize(&canonical);
    let mut names: Vec<String> = vec![canonical.clone()];
    for (a, c) in &aliases {
        if refs::normalize(c) == cnorm {
            names.push(a.clone());
        }
    }
    collect(graph, |b| names.iter().any(|n| b.projection().refs_contains(n)), Some(&canonical))
}

fn contains_word(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = hay.as_bytes();
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        let i = start + pos;
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after = i + needle.len();
        let after_ok = after >= hay.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = i + needle.len();
    }
    false
}

/// Unlinked references: blocks that mention `target` as plain text (whole word)
/// but do NOT link it via `[[..]]`/`#tag`.
pub fn unlinked_refs(graph: &Graph, target: &str) -> Vec<RefGroup> {
    let lower = target.to_lowercase();
    collect(
        graph,
        |b| contains_word(&b.raw.to_lowercase(), &lower) && !b.projection().refs_contains(target),
        Some(target),
    )
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
    let Some(pred) = Pred::parse(query_src, today) else { return Vec::new() };
    let mut opts = QueryOpts::default();
    pred.collect_opts(&mut opts);
    run_pred(graph, &pred, &opts)
}

/// Evaluate a parsed predicate against the whole graph (shared by the simple-DSL
/// `run_query` and the advanced-datalog `run_advanced_query`).
fn run_pred(graph: &Graph, pred: &Pred, opts: &QueryOpts) -> Vec<RefGroup> {
    let mut groups = graph.with_pages(|pages| {
        let mut groups: Vec<RefGroup> = Vec::new();
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
                let blocks: Vec<BlockDto> = matched.into_iter().map(block_to_dto).collect();
                groups.push(RefGroup { page: entry.name.clone(), kind: entry.kind, blocks });
            }
        }
        groups
    });

    // sort-by is GLOBAL (like Logseq): flatten every matched block across all
    // pages, sort the whole set, and emit one block per group in that order — so
    // e.g. priority-A tasks float to the very top regardless of which page they
    // live on. (Sorting within each page group, as before, can't express a global
    // order once results are grouped by page.) Non-sorted queries keep their
    // natural page grouping untouched.
    if let Some((field, asc)) = &opts.sort {
        let mut flat: Vec<RefGroup> = Vec::new();
        for g in groups {
            let RefGroup { page, kind, blocks } = g;
            for b in blocks {
                flat.push(RefGroup { page: page.clone(), kind, blocks: vec![b] });
            }
        }
        flat.sort_by(|a, b| {
            let ka = sort_key(&a.blocks[0], &a.page, field);
            let kb = sort_key(&b.blocks[0], &b.page, field);
            if *asc { ka.cmp(&kb) } else { kb.cmp(&ka) }
        });
        groups = flat;
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
    let Some(pred) = Pred::parse(src, today) else { return false };
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
pub(crate) fn page_affects_backlinks(aliases: &[(String, String)], target: &str, doc: &Document) -> bool {
    let tnorm = refs::normalize(target);
    let canonical = aliases
        .iter()
        .find(|(a, _)| *a == tnorm)
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| target.to_string());
    let cnorm = refs::normalize(&canonical);
    let mut names: Vec<String> = vec![canonical];
    for (a, c) in aliases {
        if refs::normalize(c) == cnorm {
            names.push(a.clone());
        }
    }
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit && names.iter().any(|n| b.projection().refs_contains(n)) {
            hit = true;
        }
    });
    hit
}

/// Whether page `doc` plain-text-mentions `target` unlinked — i.e. could be in
/// `unlinked_refs(target)`. Mirrors `unlinked_refs`'s matcher.
pub(crate) fn page_affects_unlinked(target: &str, doc: &Document) -> bool {
    let lower = target.to_lowercase();
    let mut hit = false;
    walk(&doc.roots, &mut |b| {
        if !hit
            && contains_word(&b.raw.to_lowercase(), &lower)
            && !b.projection().refs_contains(target)
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
pub fn run_advanced_query(graph: &Graph, query_src: &str, current_page: Option<&str>) -> AdvancedResult {
    let today = JournalDate::today();
    let inputs = resolve_inputs(query_src, current_page, today);
    let mut ran = Vec::new();
    let mut ignored = Vec::new();
    let preds: Vec<Pred> = where_groups(query_src)
        .iter()
        .filter_map(|g| parse_adv_group(g, &inputs, today, &mut ran, &mut ignored))
        .collect();
    if preds.is_empty() {
        return AdvancedResult { groups: Vec::new(), ran, ignored, supported: false };
    }
    let pred = if preds.len() == 1 { preds.into_iter().next().unwrap() } else { Pred::And(preds) };
    let mut opts = QueryOpts::default();
    pred.collect_opts(&mut opts);
    let groups = run_pred(graph, &pred, &opts);
    AdvancedResult { groups, ran, ignored, supported: true }
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
    let head = inner.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
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
        "between" => {
            // (between ?b ?start ?end): the last two args are the bounds.
            let args: Vec<&str> = inner.split_whitespace().skip(1).collect();
            if args.len() < 2 {
                ignored.push("between".into());
                return None;
            }
            let lo = adv_bound(args[args.len() - 2], inputs, today);
            let hi = adv_bound(args[args.len() - 1], inputs, today);
            if lo.is_none() && hi.is_none() {
                ignored.push("between".into());
                return None;
            }
            ran.push("between".into());
            Some(Pred::Between(BetweenField::Journal, lo, hi)) // OG :between = journal-day
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
fn adv_bound(tok: &str, inputs: &std::collections::HashMap<String, i64>, today: JournalDate) -> Option<i64> {
    let t = tok.trim();
    if t.starts_with('?') {
        return inputs.get(t).copied();
    }
    resolve_date_token(t.trim_start_matches(':'), today)
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
            let end = rest.find(":where").or_else(|| rest.find(']')).unwrap_or(rest.len());
            rest[..end].split_whitespace().filter(|t| t.starts_with('?')).map(String::from).collect()
        }
        None => Vec::new(),
    };
    let vals: Vec<String> = match src.find(":inputs") {
        Some(i) => {
            let rest = &src[i + ":inputs".len()..];
            match (rest.find('['), rest.find(']')) {
                (Some(a), Some(b)) if b > a => {
                    rest[a + 1..b].split_whitespace().map(String::from).collect()
                }
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

/// Sort key for a result block: the named property's value if present, else the
/// block's visible first line (lowercased for stable case-insensitive order).
fn sort_key(b: &BlockDto, page: &str, field: &str) -> String {
    match field.to_ascii_lowercase().as_str() {
        // Task priority is the `[#A]` marker, NOT a `priority::` property — map it
        // to A<B<C and sort unprioritized blocks last (so ascending floats A to the
        // top). Descending naturally reverses (A sinks to the bottom).
        "priority" => match block_priority(&b.raw) {
            Some(c) => c.to_ascii_uppercase().to_string(),
            None => "Z".to_string(),
        },
        // Sort by the source page name.
        "page" => page.to_lowercase(),
        // Otherwise: a block property value if present, else the block's text.
        _ => {
            if let Some((_, v)) = blockview_property(&b.raw, field) {
                return v.to_lowercase();
            }
            visible_text(&b.raw).lines().next().unwrap_or("").to_lowercase()
        }
    }
}
fn blockview_property(raw: &str, key: &str) -> Option<(String, String)> {
    raw.lines()
        .filter_map(crate::doc::parse_property_line)
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
}

/// A block's *visible* text for search: the body the user actually reads, with
/// `key:: value` property lines (block ids, `ls-type`, `hl-color`, user props,
/// `collapsed`, …) removed. Matching the rendered text instead of the raw
/// markdown avoids false positives where the query only appears in hidden
/// metadata (e.g. a uuid fragment or a property value).
pub fn visible_text(raw: &str) -> String {
    raw.lines()
        .filter(|l| crate::doc::parse_property_line(l).is_none())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Full-text search: blocks whose visible text contains `query`
/// (case-insensitive), grouped by page, capped at `limit` total blocks.
pub fn search(graph: &Graph, query: &str, limit: usize) -> Vec<RefGroup> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    // Stop scanning once we've collected `limit` matches, rather than walking the
    // whole graph and truncating afterwards — Ctrl-K only shows the first page of
    // results, so on a large graph this avoids most of the work.
    graph.with_pages(|pages| {
        let mut groups: Vec<RefGroup> = Vec::new();
        let mut remaining = limit;
        for (entry, doc) in pages {
            if remaining == 0 {
                break;
            }
            let mut matched: Vec<BlockDto> = Vec::new();
            let mut path: Vec<&DocBlock> = Vec::new();
            walk_path(&doc.roots, &mut path, &mut |b, anc| {
                if remaining == 0 {
                    return;
                }
                if b.projection().visible_lower.contains(&q) {
                    let mut dto = block_to_dto(b);
                    dto.breadcrumb = anc.iter().map(|a| crumb_line(a)).collect();
                    matched.push(dto);
                    remaining -= 1;
                }
            });
            if !matched.is_empty() {
                groups.push(RefGroup { page: entry.name.clone(), kind: entry.kind, blocks: matched });
            }
        }
        groups
    })
}

/// Find every `template:: <name>` block and the blocks an insertion produces.
pub fn templates(graph: &Graph) -> Vec<TemplateDto> {
    graph.with_pages(|pages| {
        let mut out: Vec<TemplateDto> = Vec::new();
        for (entry, doc) in pages {
            walk(&doc.roots, &mut |b| {
                let Some(name) = b.property("template") else { return };
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
                out.push(TemplateDto { name, blocks, page: entry.name.clone(), kind: entry.kind });
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
                    && (t.starts_with("template::") || t.starts_with("template-including-parent::")));
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

/// Fuzzy page-name matcher for the quick switcher. Ranks prefix > substring >
/// subsequence, then by name length.
pub fn quick_switch(graph: &Graph, query: &str, limit: usize) -> Vec<PageEntry> {
    let q = query.trim().to_lowercase();
    let file_pages = graph.list_pages();
    let mut scored: Vec<(i32, PageEntry)> = file_pages
        .iter()
        .filter_map(|e| score_name(&e.name.to_lowercase(), &q).map(|s| (s - e.name.len() as i32, e.clone())))
        .collect();
    // Pages referenced by `#tag` / `[[link]]` but with no file of their own still
    // "exist" (OG semantics): include them (deduped against file pages, which are
    // authoritative) so a tag already used elsewhere shows as the page rather than
    // a misleading "Create …" in autocomplete.
    let have: std::collections::HashSet<String> =
        file_pages.iter().map(|e| e.name.to_lowercase()).collect();
    for name in graph.referenced_page_names() {
        let lower = name.to_lowercase();
        if have.contains(&lower) {
            continue;
        }
        if let Some(s) = score_name(&lower, &q) {
            let len = name.len() as i32;
            scored.push((
                s - len,
                PageEntry { name, kind: PageKind::Page, date_key: None, path: std::path::PathBuf::new() },
            ));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, e)| e).collect()
}

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
                if found.is_none()
                    && (b.uuid == uuid || b.property("id").as_deref() == Some(uuid))
                {
                    found = Some(b);
                }
            });
            found.map(|b| RefGroup {
                page: entry.name.clone(),
                kind: entry.kind,
                blocks: vec![block_to_dto(b)],
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
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
    /// Result-level options (always pass as filters; collected as `QueryOpts`).
    Sample(usize),
    SortBy(String, bool),
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

fn block_priority(raw: &str) -> Option<char> {
    let first = raw.lines().next().unwrap_or("");
    let i = first.find("[#")?;
    let rest = &first[i + 2..];
    let c = rest.chars().next()?;
    if matches!(c, 'A' | 'B' | 'C') && rest[c.len_utf8()..].starts_with(']') {
        Some(c)
    } else {
        None
    }
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
            Pred::Priority(ps) => block_priority(&block.raw)
                .map(|c| ps.iter().any(|x| x.eq_ignore_ascii_case(&c.to_string())))
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
                            || block_date_ordinals(&block.raw, None).into_iter().any(in_range)
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
            Pred::PageTags(tags) => tags.iter().any(|t| {
                ctx.page_tags.iter().any(|pt| pt.eq_ignore_ascii_case(t))
            }),
            Pred::Content(s) => block.projection().visible_lower.contains(&s.to_lowercase()),
            Pred::And(ps) => ps.iter().all(|p| p.eval(block, ctx)),
            Pred::Or(ps) => ps.iter().any(|p| p.eval(block, ctx)),
            Pred::Not(p) => !p.eval(block, ctx),
            // Options are not filters.
            Pred::Sample(_) | Pred::SortBy(..) => true,
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
    let t = t.strip_prefix("[[").and_then(|x| x.strip_suffix("]]")).unwrap_or(t);
    t.strip_prefix('#').unwrap_or(t).trim().to_string()
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
        // A bare quoted string is a full-text content filter.
        Tok::Str(s) => {
            *pos += 1;
            Some(Pred::Content(s))
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
                        Pred::Task(vec!["TODO".into(), "DOING".into(), "NOW".into(), "LATER".into()])
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
                "page-ref" => {
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
                    let key = parse_name(toks, pos)?;
                    let val = parse_opt_name(toks, pos);
                    Pred::Property(key, val)
                }
                "page-property" => {
                    let key = parse_name(toks, pos)?;
                    let val = parse_opt_name(toks, pos);
                    Pred::PageProperty(key, val)
                }
                "page-tags" | "tags" => Pred::PageTags(parse_words(toks, pos)),
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
                    let n = parse_name(toks, pos).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                    Pred::Sample(n)
                }
                "sort-by" => {
                    let field = parse_name(toks, pos).unwrap_or_default();
                    let dir = parse_opt_name(toks, pos).unwrap_or_else(|| "asc".into());
                    Pred::SortBy(field, !dir.eq_ignore_ascii_case("desc"))
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed "today" so relative-date tests are deterministic: 2026-06-16.
    const TODAY: JournalDate = JournalDate { year: 2026, month: 6, day: 16 };

    fn pred(src: &str) -> Pred {
        Pred::parse(src, TODAY).expect("parse")
    }

    /// A minimal eval context for a block on a named (non-journal) page.
    fn ctx_named<'a>() -> EvalCtx<'a> {
        EvalCtx { journal: None, is_journal: false, page_name: "Test", page_props: &[], page_tags: &[] }
    }
    fn ctx_journal<'a>(key: i64) -> EvalCtx<'a> {
        EvalCtx { journal: Some(key), is_journal: true, page_name: "Journal", page_props: &[], page_tags: &[] }
    }

    #[test]
    fn parse_pageref_and_tag() {
        assert_eq!(pred("[[Foo]]"), Pred::PageRef("Foo".into()));
        assert_eq!(pred("#bar"), Pred::PageRef("bar".into()));
    }

    #[test]
    fn parse_boolean() {
        assert_eq!(
            pred("(and [[A]] [[B]])"),
            Pred::And(vec![Pred::PageRef("A".into()), Pred::PageRef("B".into())])
        );
        assert_eq!(pred("(not [[A]])"), Pred::Not(Box::new(Pred::PageRef("A".into()))));
    }

    #[test]
    fn parse_task_and_property() {
        assert_eq!(pred("(task TODO DOING)"), Pred::Task(vec!["TODO".into(), "DOING".into()]));
        assert_eq!(pred("(property type book)"), Pred::Property("type".into(), Some("book".into())));
        assert_eq!(pred("(property public)"), Pred::Property("public".into(), None));
    }

    #[test]
    fn parse_escaped_string_content() {
        // `\"`/`\\` inside a quoted full-text term are unescaped (mirrors the
        // query-builder serializer's quoteStr), so a quote in the term doesn't
        // end the string early and silently truncate the query.
        assert_eq!(pred("\"foo \\\"bar\\\"\""), Pred::Content("foo \"bar\"".into()));
        assert_eq!(pred("\"a\\\\b\""), Pred::Content("a\\b".into()));
        // Only `\"`/`\\` are escapes: a hand-authored backslash before another
        // char is literal, so `"C:\tmp"` stays `C:\tmp` (not `C:tmp`).
        assert_eq!(pred("\"a\\q\""), Pred::Content("a\\q".into()));
        assert_eq!(pred("\"C:\\tmp\""), Pred::Content("C:\\tmp".into()));
        // End-to-end: the term still matches a block whose text contains the quote.
        let none = ctx_named();
        let b = DocBlock::new("note: foo \"bar\" baz");
        assert!(pred("\"foo \\\"bar\\\"\"").eval(&b, &none));
    }

    #[test]
    fn advanced_datalog_is_unsupported() {
        assert!(is_advanced("[:find (pull ?b [*]) :where [?b :block/marker]]"));
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
        assert_eq!(q, Pred::Between(BetweenField::Any, Some(20260609), Some(20260623)));
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
        let ctx = EvalCtx { journal: None, is_journal: false, page_name: "Project/Alpha", page_props: &[], page_tags: &[] };
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
        let ctx = EvalCtx { journal: None, is_journal: false, page_name: "P", page_props: &props, page_tags: &tags };
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
        assert_eq!(block_date_ordinals("TODO x\nSCHEDULED: <2026-07-06 Mon>", Some("SCHEDULED:")), vec![ord]);
        // inline on the marker line (the regressed case)
        assert_eq!(block_date_ordinals("TODO SCHEDULED: <2026-07-06 Mon> do it", Some("SCHEDULED:")), vec![ord]);
        // with trailing text after the timestamp
        assert_eq!(
            block_date_ordinals(" SCHEDULED: <2026-07-06 Mon> #email students", Some("SCHEDULED:")),
            vec![ord]
        );
        // DEADLINE restricted; SCHEDULED-only query ignores it
        assert!(block_date_ordinals("DEADLINE: <2026-07-06 Mon>", Some("SCHEDULED:")).is_empty());
        assert_eq!(block_date_ordinals("DEADLINE: <2026-07-06 Mon>", Some("DEADLINE:")), vec![ord]);
    }
}
