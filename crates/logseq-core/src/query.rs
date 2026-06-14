//! Backlinks and the `{{query}}` subset engine. Evaluated by scanning parsed
//! pages (no datalog). Pragmatic subset: page/tag refs, boolean and/or/not,
//! task markers, and property filters. Advanced datalog (`[:find ...]`) is
//! detected and reported as unsupported rather than crashed.

use crate::doc::{DocBlock, Document};
use crate::model::{block_to_dto, Graph, PageEntry, RefGroup};
use crate::refs;

/// Walk all blocks of a document depth-first, calling `f(block)`.
fn walk<'a>(blocks: &'a [DocBlock], f: &mut impl FnMut(&'a DocBlock)) {
    for b in blocks {
        f(b);
        walk(&b.children, f);
    }
}

/// Load+parse every page, skipping unreadable files.
fn all_pages(graph: &Graph) -> Vec<(PageEntry, Document)> {
    graph
        .list_pages()
        .into_iter()
        .filter_map(|e| graph.read_document(&e).ok().map(|d| (e, d)))
        .collect()
}

/// Collect matching blocks across the graph, grouped by source page.
fn collect(graph: &Graph, mut keep: impl FnMut(&DocBlock) -> bool, exclude: Option<&str>) -> Vec<RefGroup> {
    let ex = exclude.map(refs::normalize);
    let mut groups: Vec<RefGroup> = Vec::new();
    for (entry, doc) in all_pages(graph) {
        if ex.as_deref() == Some(&refs::normalize(&entry.name)) {
            continue;
        }
        let mut matched: Vec<&DocBlock> = Vec::new();
        walk(&doc.roots, &mut |b| {
            if keep(b) {
                matched.push(b);
            }
        });
        if !matched.is_empty() {
            groups.push(RefGroup {
                page: entry.name.clone(),
                kind: entry.kind,
                blocks: matched.into_iter().map(block_to_dto).collect(),
            });
        }
    }
    groups
}

pub fn backlinks(graph: &Graph, target: &str) -> Vec<RefGroup> {
    collect(graph, |b| refs::references_page(&b.raw, target), Some(target))
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

pub fn run_query(graph: &Graph, query_src: &str) -> Vec<RefGroup> {
    match Pred::parse(query_src) {
        Some(pred) => collect(graph, |b| pred.eval(b), None),
        None => Vec::new(),
    }
}

/// Full-text search: blocks whose text contains `query` (case-insensitive),
/// grouped by page, capped at `limit` total blocks.
pub fn search(graph: &Graph, query: &str, limit: usize) -> Vec<RefGroup> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut groups = collect(graph, |b| b.raw.to_lowercase().contains(&q), None);
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
    for (entry, doc) in all_pages(graph) {
        let mut found: Option<&DocBlock> = None;
        walk(&doc.roots, &mut |b| {
            if found.is_none() && b.property("id").as_deref() == Some(uuid) {
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
    Property(String, Option<String>),
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
}

impl Pred {
    fn parse(src: &str) -> Option<Pred> {
        if is_advanced(src) {
            return None;
        }
        let tokens = tokenize(src);
        let mut pos = 0;
        let p = parse_expr(&tokens, &mut pos)?;
        Some(p)
    }

    fn eval(&self, block: &DocBlock) -> bool {
        match self {
            Pred::PageRef(name) => refs::references_page(&block.raw, name),
            Pred::Task(markers) => block
                .marker()
                .map(|m| markers.iter().any(|x| x.eq_ignore_ascii_case(m)))
                .unwrap_or(false),
            Pred::Property(key, val) => block.properties().iter().any(|(k, v)| {
                k.eq_ignore_ascii_case(key) && val.as_ref().map(|vv| vv == v).unwrap_or(true)
            }),
            Pred::And(ps) => ps.iter().all(|p| p.eval(block)),
            Pred::Or(ps) => ps.iter().any(|p| p.eval(block)),
            Pred::Not(p) => !p.eval(block),
        }
    }
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

fn parse_expr(toks: &[Tok], pos: &mut usize) -> Option<Pred> {
    let t = toks.get(*pos)?.clone();
    match t {
        Tok::PageRef(name) | Tok::Tag(name) => {
            *pos += 1;
            Some(Pred::PageRef(name))
        }
        Tok::LParen => {
            *pos += 1; // consume (
            let head = match toks.get(*pos)? {
                Tok::Word(w) => w.to_lowercase(),
                _ => return None,
            };
            *pos += 1;
            let pred = match head.as_str() {
                "and" => Pred::And(parse_list(toks, pos)),
                "or" => Pred::Or(parse_list(toks, pos)),
                "not" => Pred::Not(Box::new(parse_expr(toks, pos)?)),
                "task" | "todo" => {
                    let markers = parse_words(toks, pos);
                    // `(todo)` with no args means any open task.
                    if markers.is_empty() {
                        Pred::Task(vec!["TODO".into(), "DOING".into(), "NOW".into(), "LATER".into()])
                    } else {
                        Pred::Task(markers)
                    }
                }
                "page-ref" => {
                    let name = parse_name(toks, pos)?;
                    Pred::PageRef(name)
                }
                "property" => {
                    let key = parse_name(toks, pos)?;
                    let val = parse_opt_name(toks, pos);
                    Pred::Property(key, val)
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

fn parse_list(toks: &[Tok], pos: &mut usize) -> Vec<Pred> {
    let mut out = Vec::new();
    while let Some(t) = toks.get(*pos) {
        if *t == Tok::RParen {
            break;
        }
        match parse_expr(toks, pos) {
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

    fn pred(src: &str) -> Pred {
        Pred::parse(src).expect("parse")
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
        assert!(Pred::parse("[:find ?b :where ...]").is_none());
    }

    #[test]
    fn eval_against_blocks() {
        let task = DocBlock::new("TODO buy milk for [[Home]]");
        assert!(pred("(task TODO)").eval(&task));
        assert!(pred("[[Home]]").eval(&task));
        assert!(pred("(and (task TODO) [[Home]])").eval(&task));
        assert!(!pred("(and (task DONE) [[Home]])").eval(&task));
        assert!(pred("(not [[Work]])").eval(&task));

        let mut withprop = DocBlock::new("a book");
        withprop.raw.push_str("\ntype:: book");
        assert!(pred("(property type book)").eval(&withprop));
        assert!(pred("(property type)").eval(&withprop));
        assert!(!pred("(property type article)").eval(&withprop));
    }
}
