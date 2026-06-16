//! Backlinks and the `{{query}}` subset engine. Evaluated by scanning parsed
//! pages (no datalog). Pragmatic subset: page/tag refs, boolean and/or/not,
//! task markers, and property filters. Advanced datalog (`[:find ...]`) is
//! detected and reported as unsupported rather than crashed.

use crate::date::JournalDate;
use crate::doc::DocBlock;
use crate::model::{block_to_dto, BlockDto, Graph, PageEntry, RefGroup, TemplateDto};
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
    // aliases, and looking up an alias resolves to its canonical page.
    let aliases = page_aliases(graph);
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
    collect(graph, |b| names.iter().any(|n| refs::references_page(&b.raw, n)), Some(&canonical))
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
        |b| contains_word(&b.raw.to_lowercase(), &lower) && !refs::references_page(&b.raw, target),
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

    let mut groups = graph.with_pages(|pages| {
        let mut groups: Vec<RefGroup> = Vec::new();
        for (entry, doc) in pages {
            let (page_props, page_tags) = page_facets(doc.pre_block.as_deref());
            let ctx = EvalCtx {
                journal: entry.date_key,
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
                let mut blocks: Vec<BlockDto> = matched.into_iter().map(block_to_dto).collect();
                // sort-by a property value (or the block text) within the group.
                if let Some((field, asc)) = &opts.sort {
                    blocks.sort_by(|a, b| {
                        let ka = sort_key(a, field);
                        let kb = sort_key(b, field);
                        if *asc { ka.cmp(&kb) } else { kb.cmp(&ka) }
                    });
                }
                groups.push(RefGroup { page: entry.name.clone(), kind: entry.kind, blocks });
            }
        }
        groups
    });

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

/// Sort key for a result block: the named property's value if present, else the
/// block's visible first line (lowercased for stable case-insensitive order).
fn sort_key(b: &BlockDto, field: &str) -> String {
    if let Some((_, v)) = blockview_property(&b.raw, field) {
        return v.to_lowercase();
    }
    visible_text(&b.raw).lines().next().unwrap_or("").to_lowercase()
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
    if q.is_empty() {
        return Vec::new();
    }
    let mut groups = collect(graph, |b| visible_text(&b.raw).to_lowercase().contains(&q), None);
    let mut remaining = limit;
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
    groups
}

/// Find every `template:: <name>` block and the blocks an insertion produces.
pub fn templates(graph: &Graph) -> Vec<TemplateDto> {
    graph.with_pages(|pages| {
        let mut out: Vec<TemplateDto> = Vec::new();
        for (_entry, doc) in pages {
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
                out.push(TemplateDto { name, blocks });
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
    let mut scored: Vec<(i32, PageEntry)> = graph
        .list_pages()
        .into_iter()
        .filter_map(|e| score_name(&e.name.to_lowercase(), &q).map(|s| (s - e.name.len() as i32, e)))
        .collect();
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
    graph.with_pages(|pages| {
        for (entry, doc) in pages {
            let mut found: Option<&DocBlock> = None;
            walk(&doc.roots, &mut |b| {
                if found.is_none()
                    && (b.uuid == uuid || b.property("id").as_deref() == Some(uuid))
                {
                    found = Some(b);
                }
            });
            if let Some(b) = found {
                return Some(RefGroup {
                    page: entry.name.clone(),
                    kind: entry.kind,
                    blocks: vec![block_to_dto(b)],
                });
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
    /// Date range (inclusive) over the block's journal day or its
    /// scheduled/deadline date. Bounds are `yyyymmdd` ordinals; `None` = open.
    Between(Option<i64>, Option<i64>),
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

/// Ordinals from a block's SCHEDULED:/DEADLINE: lines.
fn block_date_ordinals(raw: &str) -> Vec<i64> {
    raw.lines()
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix("SCHEDULED:")
                .or_else(|| t.strip_prefix("DEADLINE:"))
                .and_then(parse_angle_date)
        })
        .collect()
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
            Pred::PageRef(name) => refs::references_page(&block.raw, name),
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
            Pred::Between(lo, hi) => {
                let in_range = |c: i64| lo.map_or(true, |l| c >= l) && hi.map_or(true, |h| c <= h);
                ctx.journal.is_some_and(in_range)
                    || block_date_ordinals(&block.raw).into_iter().any(in_range)
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
            Pred::Content(s) => visible_text(&block.raw).to_lowercase().contains(&s.to_lowercase()),
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
            while j < chars.len() && chars[j] != '"' {
                s.push(chars[j]);
                j += 1;
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
                "between" => {
                    // (between START END): journal titles, `today`/`yesterday`/
                    // `tomorrow`, signed durations `±N[dwmy]`, or `yyyy-MM-dd`.
                    let lo = parse_name(toks, pos).and_then(|s| resolve_date_token(&s, today));
                    let hi = parse_name(toks, pos).and_then(|s| resolve_date_token(&s, today));
                    Pred::Between(lo, hi)
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
        EvalCtx { journal: None, page_name: "Test", page_props: &[], page_tags: &[] }
    }
    fn ctx_journal<'a>(key: i64) -> EvalCtx<'a> {
        EvalCtx { journal: Some(key), page_name: "Journal", page_props: &[], page_tags: &[] }
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
        assert_eq!(q, Pred::Between(Some(20260609), Some(20260623)));
        let b = DocBlock::new("x");
        assert!(q.eval(&b, &ctx_journal(20260616)));
        assert!(q.eval(&b, &ctx_journal(20260609)));
        assert!(!q.eval(&b, &ctx_journal(20260601)));
        // keyword bounds + month/year units
        assert_eq!(pred("(between today tomorrow)"), Pred::Between(Some(20260616), Some(20260617)));
        assert_eq!(pred("(between -1m +1y)"), Pred::Between(Some(20260516), Some(20270616)));
    }

    #[test]
    fn eval_page_and_namespace() {
        let b = DocBlock::new("hi");
        let ctx = EvalCtx { journal: None, page_name: "Project/Alpha", page_props: &[], page_tags: &[] };
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
        let ctx = EvalCtx { journal: None, page_name: "P", page_props: &props, page_tags: &tags };
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
}
