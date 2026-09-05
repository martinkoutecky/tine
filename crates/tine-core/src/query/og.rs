//! OG DSL ⇄ IR (SPEC §4.1, §4.3).
//!
//! Transcribed (D-9) from OG `src/main/frontend/db/query_dsl.cljs` at the
//! read-only checkout `/aux/koutecky/logseq/og` commit `6e7afa8eb`
//! (`git describe`: `1.0.0-12-g6e7afa8eb`): `pre-transform` (`:452-472`),
//! `simplify-query` (`:505-516`), `build-query` (`:377-445`) and its
//! `build-*` helpers, `parse-property-value` (`:242-252`),
//! `build-all-page-tags` (`:322-325`), and the `blocks?` anchor rule
//! (`:388-393`). The serializer transcribes the frontend's own `toDsl`
//! (`src/editor/queryBuilder.ts:482`) with its `quoteStr`/`needsQuote`
//! escaping (`:409-424`) and single-child `and` simplification.
//!
//! **The unknown head no longer truncates.** OG's `build-query` returns `nil`
//! for an unrecognised head and the old Tine parser propagated that as "the
//! whole parse failed from here", silently dropping the rest of the query
//! (`(and (task TODO) (frobnicate x))` ran as `(task TODO)`). An unknown head
//! is now a [`Filter::Raw`] leaf plus an `UnknownHead` diagnostic, which makes
//! the query invalid and returns zero results — the rest of the tree is still
//! parsed and shown.

use crate::date::JournalDate;
use crate::doc::property_key_norm;
use crate::query::ir::{
    Anchor, Attr, CmpOp, Diagnostic, DiagnosticKind, Field, Filter, Leaf, Quant, Query, Rel,
    SortDir, Source, Span, Value, ViewSettings,
};

use super::QUERY_NESTING_MAX;

// ---------------------------------------------------------------------------
// pre-transform (OG query_dsl.cljs:452-472), transcribed
// ---------------------------------------------------------------------------
//
// **This block is the ORACLE, not the pipeline.** OG rewrites the query text
// before reading it; Tine's tokenizer implements the same net effect directly
// so that `Raw` spans stay exact offsets into the text the author wrote. The
// transcription is kept verbatim beside the tokenizer and the two are pinned
// against each other by `pre_transform_equivalent_shapes` — so it is compiled
// under `cfg(test)`, where a reader can see it is evidence rather than a second
// code path (I-11).

#[cfg(test)]
const TAG_PLACEHOLDER: &str = "~~~tag-placeholder~~~";

/// OG `gp-util/wrapped-by-quotes?` (`graph_parser/util.cljs:78`).
#[cfg(test)]
fn wrapped_by_quotes(v: &str) -> bool {
    v.len() >= 2 && v.starts_with('"') && v.ends_with('"')
}

/// OG `page-ref/get-page-name` (`util/page_ref.cljs:47`): the inner text of a
/// whole-string `[[…]]`, else `None`.
fn get_page_name(s: &str) -> Option<&str> {
    s.strip_prefix("[[")?.strip_suffix("]]")
}

/// OG `page-ref/get-page-name!`: the inner text, falling back to the argument.
fn get_page_name_bang(s: &str) -> &str {
    get_page_name(s).unwrap_or(s)
}

/// Replace every non-greedy `[[(.*?)]]` match, OG `page-ref/page-ref-re`.
#[cfg(test)]
fn replace_page_refs(s: &str, mut replace: impl FnMut(&str) -> String) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"[[") {
            if let Some(offset) = s[index + 2..].find("]]") {
                let inner = &s[index + 2..index + 2 + offset];
                out.push_str(&replace(inner));
                index += 2 + offset + 2;
                continue;
            }
        }
        let ch = s[index..].chars().next().expect("char boundary");
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// OG's `text-util/between-re` = `#"\(between ([^\)]+)\)"`: rewrite each
/// `(between …)` argument list, keywordizing signed or unit-suffixed tokens.
#[cfg(test)]
fn replace_between(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("(between ") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "(between ".len()..];
        let Some(close) = after.find(')') else {
            out.push_str(rest);
            return out;
        };
        let inner = &after[..close];
        let shaped = inner
            .split(' ')
            .filter(|token| !token.trim().is_empty())
            .map(|token| {
                // In ClojureScript `(first "…")` is a one-character STRING, so
                // OG's `(contains? #{"+" "-"} (first x))` really does fire.
                let first = token.chars().next().unwrap_or(' ');
                let signed = first == '+' || first == '-';
                let unit_suffixed = first.is_ascii_digit()
                    && ["y", "m", "d", "h", "min"]
                        .iter()
                        .any(|unit| token.ends_with(unit));
                if signed || unit_suffixed {
                    format!(":{token}")
                } else {
                    token.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!("(between {shaped})"));
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Replace `#` with the placeholder inside every `"[^"]+"` run, OG's fourth step.
#[cfg(test)]
fn protect_hashes_in_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('"') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('"') {
            // `[^"]+` requires at least one character between the quotes.
            Some(close) if close > 0 => {
                out.push('"');
                out.push_str(&after[..close].replace('#', TAG_PLACEHOLDER));
                out.push('"');
                rest = &after[close + 1..];
            }
            _ => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// OG `pre-transform` (`query_dsl.cljs:452-472`), transcribed verbatim in order.
///
/// Its net effect on the token stream — `[[x]]` and `#x` both become page refs,
/// `#` inside a string stays literal — is what [`tokenize`] implements directly,
/// which is why the parser runs on the ORIGINAL text (spans stay exact) while
/// `pre_transform_equivalent_shapes` pins the two against each other.
#[cfg(test)]
pub(crate) fn pre_transform(s: &str) -> String {
    if wrapped_by_quotes(s) {
        return s.to_string();
    }
    let quoted = replace_page_refs(s, |inner| {
        format!("\"[[{}]]\"", inner.replace('#', TAG_PLACEHOLDER))
    });
    let betweened = replace_between(&quoted);
    let protected = protect_hashes_in_strings(&betweened);
    let tagged = protected.replace(" #", " #tag ");
    let tagged = match tagged.strip_prefix('#') {
        Some(rest) => format!("#tag {rest}"),
        None => tagged,
    };
    tagged.replace(TAG_PLACEHOLDER, "#")
}

// ---------------------------------------------------------------------------
// Tokenizer (spans are byte offsets into the ORIGINAL form text)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    LParen,
    RParen,
    /// `[[name]]`, `"[[name]]"` (what `pre-transform` produces) or `#name` /
    /// `#tag name` — all four are OG page refs (Q2).
    PageRef(String),
    Word(String),
    Str(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Spanned {
    pub(crate) tok: Tok,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Tokenize an OG DSL form. Recognises exactly the token classes OG's reader
/// sees after `pre-transform`: bare and quoted `[[x]]`, `#x`, `#tag x`, quoted
/// strings with `\"`/`\\` escapes, parentheses, and bare words.
pub(crate) fn tokenize(src: &str) -> Vec<Spanned> {
    let bytes = src.as_bytes();
    let mut out: Vec<Spanned> = Vec::new();
    let mut i = 0usize;
    let push = |out: &mut Vec<Spanned>, tok, start, end| out.push(Spanned { tok, start, end });
    while i < bytes.len() {
        let ch = src[i..].chars().next().expect("char boundary");
        if ch.is_whitespace() {
            i += ch.len_utf8();
        } else if ch == '(' {
            push(&mut out, Tok::LParen, i, i + 1);
            i += 1;
        } else if ch == ')' {
            push(&mut out, Tok::RParen, i, i + 1);
            i += 1;
        } else if src[i..].starts_with("[[") {
            let (name, end) = read_page_ref(src, i);
            push(&mut out, Tok::PageRef(name), i, end);
            i = end;
        } else if ch == '#' {
            let start = i;
            if src[i..].starts_with("#[[") {
                let (name, end) = read_page_ref(src, i + 1);
                push(&mut out, Tok::PageRef(name), start, end);
                i = end;
            } else {
                let mut j = i + 1;
                while j < bytes.len() {
                    let c = src[j..].chars().next().expect("char boundary");
                    if c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.') {
                        j += c.len_utf8();
                    } else {
                        break;
                    }
                }
                let name = src[i + 1..j].to_string();
                i = j;
                // `#tag foo` is OG's own reader macro (`custom-readers`,
                // `query_dsl.cljs:519`): pre-transform emits it for every ` #x`,
                // so accept the spelled-out form as the same page ref.
                if name == "tag" {
                    let after = skip_ws(src, j);
                    if let Some((inner, end)) = read_bare_name(src, after) {
                        push(&mut out, Tok::PageRef(inner), start, end);
                        i = end;
                        continue;
                    }
                }
                push(&mut out, Tok::PageRef(name), start, i);
            }
        } else if ch == '"' {
            let (text, end) = read_string(src, i);
            // OG's `page-ref?` in `build-query` treats a STRING that is a page
            // ref as a page ref — this is exactly how pre-transform's quoting
            // survives the reader.
            match get_page_name(&text) {
                Some(name) => push(&mut out, Tok::PageRef(name.to_string()), i, end),
                None => push(&mut out, Tok::Str(text), i, end),
            }
            i = end;
        } else {
            let mut j = i;
            while j < bytes.len() {
                let c = src[j..].chars().next().expect("char boundary");
                if c.is_whitespace() || matches!(c, '(' | ')') {
                    break;
                }
                j += c.len_utf8();
            }
            push(&mut out, Tok::Word(src[i..j].to_string()), i, j);
            i = j;
        }
    }
    out
}

fn skip_ws(src: &str, mut at: usize) -> usize {
    while at < src.len() {
        let c = src[at..].chars().next().expect("char boundary");
        if c.is_whitespace() {
            at += c.len_utf8();
        } else {
            break;
        }
    }
    at
}

/// A bare word after `#tag`, stopping at whitespace or a paren.
fn read_bare_name(src: &str, at: usize) -> Option<(String, usize)> {
    let mut j = at;
    while j < src.len() {
        let c = src[j..].chars().next().expect("char boundary");
        if c.is_whitespace() || matches!(c, '(' | ')') {
            break;
        }
        j += c.len_utf8();
    }
    (j > at).then(|| (src[at..j].to_string(), j))
}

/// Read `[[…]]` starting at `at`, returning the inner name and the end offset.
fn read_page_ref(src: &str, at: usize) -> (String, usize) {
    let inner_start = at + 2;
    match src[inner_start..].find("]]") {
        Some(offset) => (
            src[inner_start..inner_start + offset].to_string(),
            inner_start + offset + 2,
        ),
        None => (src[inner_start..].to_string(), src.len()),
    }
}

/// Escape-aware string read: ONLY `\"` and `\\` are escapes, so a hand-authored
/// `"C:\tmp"` round-trips unchanged (this matches the frontend's `quoteStr`).
fn read_string(src: &str, at: usize) -> (String, usize) {
    let bytes = src.as_bytes();
    let mut text = String::new();
    let mut j = at + 1;
    while j < bytes.len() && bytes[j] != b'"' {
        if bytes[j] == b'\\' && matches!(bytes.get(j + 1), Some(b'"') | Some(b'\\')) {
            text.push(bytes[j + 1] as char);
            j += 2;
        } else {
            let c = src[j..].chars().next().expect("char boundary");
            text.push(c);
            j += c.len_utf8();
        }
    }
    (text, (j + 1).min(src.len()))
}

// ---------------------------------------------------------------------------
// parse-property-value (OG query_dsl.cljs:242-252), transcribed
// ---------------------------------------------------------------------------

/// OG `parse-property-value`: a `#tag` loses its `#`, a `[[page]]` loses its
/// brackets, everything else is the trimmed text.
///
/// OG additionally *types* `"true"`/`"false"`/`^\d+$` here
/// (`text/parse-non-string-property-value`, `text.cljs:87-96`). Tine keeps the
/// text form in the IR because a property atom is typed by the registry's
/// **effective type** (§6.3) at evaluation, not by the query's spelling: the
/// same `(property size 5)` compares as a number under a number key and as
/// text under a text key.
pub(crate) fn parse_property_value(value: &str) -> String {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('#') {
        return rest.trim().to_string();
    }
    get_page_name_bang(value).trim().to_string()
}

/// A property KEY as OG's query DSL normalizes it: drop a leading `:`,
/// lowercase, map spaces/underscores to dashes.
pub(crate) fn normalize_prop_key(key: &str) -> String {
    property_key_norm(key.trim_start_matches(':'))
}

// ---------------------------------------------------------------------------
// build-query (OG query_dsl.cljs:377-445), transcribed onto the IR
// ---------------------------------------------------------------------------

struct OgParse<'a> {
    toks: Vec<Spanned>,
    pos: usize,
    src: &'a str,
    view: ViewSettings,
    diagnostics: Vec<Diagnostic>,
    /// OG's `blocks?` atom (`build-query:388-393`): the anchor rule.
    blocks: bool,
}

/// Wrap a page-row filter so it reads through a block's owning page. At the
/// `@page` anchor these wrappers are unwrapped again by [`rebase_to_page`],
/// which is how one parse serves both anchors without a second traversal.
fn through_page(filter: Filter) -> Filter {
    Filter::rel(Rel::Page, Quant::Any, filter)
}

/// `@page` anchor: the page row IS the current row, so every `page` hop the
/// parser inserted collapses away. OG's DSL has no `children`/`blocks`
/// relation, so this only ever meets boolean nodes and page hops.
fn rebase_to_page(filter: Filter) -> Filter {
    match filter {
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Page,
                    quant: Quant::Any,
                    pred,
                },
        } => *pred,
        Filter::And { items } => Filter::and(items.into_iter().map(rebase_to_page).collect()),
        Filter::Or { items } => Filter::or(items.into_iter().map(rebase_to_page).collect()),
        Filter::Not { inner } => Filter::not(rebase_to_page(*inner)),
        Filter::Off { inner } => Filter::off(rebase_to_page(*inner)),
        other => other,
    }
}

impl<'a> OgParse<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn span_at(&self, index: usize) -> Option<Span> {
        self.toks
            .get(index)
            .map(|t| Span::from_byte_range(self.src, t.start, t.end))
    }

    fn diagnose(&mut self, kind: DiagnosticKind, message: impl Into<String>, span: Option<Span>) {
        self.diagnostics
            .push(Diagnostic::new(kind, message).with_span(span));
    }

    /// A bare name token: word, string, or page ref (OG's `get-page-name!`).
    fn name(&mut self) -> Option<String> {
        let value = match self.peek()? {
            Tok::Word(w) => w.clone(),
            Tok::Str(s) => s.clone(),
            Tok::PageRef(p) => p.clone(),
            _ => return None,
        };
        self.pos += 1;
        Some(value)
    }

    fn opt_word_or_string(&mut self) -> Option<String> {
        match self.peek() {
            Some(Tok::Word(_)) | Some(Tok::Str(_)) => self.name(),
            _ => None,
        }
    }

    /// Every remaining name token before the closing paren.
    fn names(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while matches!(
            self.peek(),
            Some(Tok::Word(_)) | Some(Tok::Str(_)) | Some(Tok::PageRef(_))
        ) {
            if let Some(name) = self.name() {
                out.push(name);
            }
        }
        out
    }

    /// Skip to just past the `)` closing the form that opened at depth 0 here.
    fn skip_to_close(&mut self) {
        let mut depth = 1usize;
        while let Some(tok) = self.peek() {
            match tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn list(&mut self, depth: usize) -> Vec<Filter> {
        let mut out = Vec::new();
        while let Some(tok) = self.peek() {
            if *tok == Tok::RParen {
                break;
            }
            match self.expr(depth) {
                Some(filter) => out.push(filter),
                None => break,
            }
        }
        out
    }

    fn expr(&mut self, depth: usize) -> Option<Filter> {
        if depth > QUERY_NESTING_MAX {
            let span = self.span_at(self.pos);
            self.diagnose(DiagnosticKind::Depth, "the query nests too deeply", span);
            return None;
        }
        let tok = self.toks.get(self.pos)?.tok.clone();
        match tok {
            Tok::PageRef(name) => {
                self.pos += 1;
                self.blocks = true;
                Some(Filter::page_ref(name))
            }
            // A bare string (or a macro-decoded bare word) is OG's block-content
            // full-text search. Fold ONCE here so the evaluator compares against
            // an already-folded term instead of re-folding per candidate block.
            Tok::Str(text) | Tok::Word(text) => {
                self.pos += 1;
                self.blocks = true;
                Some(content_like(&text))
            }
            Tok::LParen => {
                let open = self.pos;
                self.pos += 1;
                let head = match self.peek() {
                    Some(Tok::Word(w)) => w.to_lowercase(),
                    _ => {
                        let span = self.span_at(open);
                        self.diagnose(
                            DiagnosticKind::Syntax,
                            "a query form must start with a head word",
                            span,
                        );
                        self.skip_to_close();
                        return Some(Filter::Raw {
                            text: String::new(),
                            kind: DiagnosticKind::Syntax,
                            span,
                        });
                    }
                };
                self.pos += 1;
                let filter = self.head(&head, open, depth);
                if let Some(Tok::RParen) = self.peek() {
                    self.pos += 1;
                }
                filter
            }
            Tok::RParen => None,
        }
    }

    fn head(&mut self, head: &str, open: usize, depth: usize) -> Option<Filter> {
        let filter = match head {
            "and" => Filter::and(lift_directives(self.list(depth + 1))),
            "or" => Filter::or(lift_directives(self.list(depth + 1))),
            "not" => Filter::not(self.expr(depth + 1)?),
            "task" | "todo" => {
                self.blocks = true;
                let markers = self.names();
                // OG drops `(task)` with no markers; Tine's shipped behaviour
                // reads it as "any open task" and the corpus depends on it.
                let markers = if markers.is_empty() {
                    vec![
                        "TODO".to_string(),
                        "DOING".to_string(),
                        "NOW".to_string(),
                        "LATER".to_string(),
                    ]
                } else {
                    markers
                };
                Filter::attr(Attr::Task, CmpOp::In, text_list(markers))
            }
            "priority" => {
                self.blocks = true;
                let levels = self.names();
                let levels = if levels.is_empty() {
                    vec!["A".to_string(), "B".to_string(), "C".to_string()]
                } else {
                    levels
                };
                Filter::attr(Attr::Priority, CmpOp::In, text_list(levels))
            }
            // Tine-only spellings of a bare `[[x]]` / `#x`. OG's `blocks?` rule
            // only knows the bare token, so Tine's extension heads set the flag
            // themselves — every one of them tests a BLOCK row.
            "page-ref" | "tag" => {
                self.blocks = true;
                Filter::page_ref(self.name()?)
            }
            "page" => {
                self.blocks = true;
                through_page(Filter::attr(
                    Attr::Name,
                    CmpOp::Eq,
                    Value::text(self.name()?),
                ))
            }
            // OG `(namespace x)` is recursive membership: the normalized page
            // name starts with `x/` (§3.2 M20).
            "namespace" => through_page(Filter::attr(
                Attr::Name,
                CmpOp::StartsWith,
                Value::text(format!("{}/", self.name()?)),
            )),
            "property" => {
                self.blocks = true;
                let key = normalize_prop_key(&self.name()?);
                let value = self.opt_property_value();
                property_leaf(key, value)
            }
            "page-property" => through_page({
                let key = normalize_prop_key(&self.name()?);
                let value = self.opt_property_value();
                property_leaf(key, value)
            }),
            "page-tags" | "tags" => {
                let tags = self.names();
                through_page(Filter::rel(
                    Rel::Props,
                    Quant::Any,
                    Filter::and(vec![
                        Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                        Filter::attr(Attr::Value, CmpOp::In, text_list(tags)),
                    ]),
                ))
            }
            // OG `build-all-page-tags` (`:322-325`): pages carrying at least one
            // tag. It takes no arguments.
            "all-page-tags" => {
                let extra = self.names();
                if !extra.is_empty() {
                    let span = self.span_at(open);
                    self.diagnose(
                        DiagnosticKind::Syntax,
                        "(all-page-tags) takes no arguments",
                        span,
                    );
                }
                through_page(Filter::rel(
                    Rel::Props,
                    Quant::Any,
                    Filter::and(vec![
                        Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                        Filter::attr(Attr::AtomCount, CmpOp::Gt, Value::Number { number: 0.0 }),
                    ]),
                ))
            }
            // Tine extensions, kept parsing for existing files, never OG-expressible.
            "search" => {
                self.blocks = true;
                Filter::attr(Attr::Content, CmpOp::Match, Value::text(self.name()?))
            }
            "content-regex" => {
                self.blocks = true;
                Filter::attr(Attr::Content, CmpOp::Regex, Value::text(self.name()?))
            }
            "scheduled" => {
                self.blocks = true;
                Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None)
            }
            "deadline" => {
                self.blocks = true;
                Filter::attr(Attr::Deadline, CmpOp::IsSet, Value::None)
            }
            "journal" => {
                self.blocks = true;
                through_page(Filter::attr(
                    Attr::Journal,
                    CmpOp::Eq,
                    Value::Bool { value: true },
                ))
            }
            "between" => {
                self.blocks = true;
                self.between()
            }
            "sample" => {
                if let Some(n) = self.name().and_then(|s| s.trim().parse::<u32>().ok()) {
                    self.view.sample = Some(n);
                }
                Filter::True
            }
            "sort-by" => {
                let field = self.name().unwrap_or_default();
                // OG's own default here is `:desc`; Tine has always defaulted to
                // ascending and the shipped behaviour is what P0 must preserve.
                let dir = match self.opt_word_or_string() {
                    Some(d) if d.eq_ignore_ascii_case("desc") => SortDir::Desc,
                    _ => SortDir::Asc,
                };
                self.view.sort = vec![(Field::new(field), dir)];
                Filter::True
            }
            "aggregate" => {
                use crate::query::ir::AggFn;
                let pair = match self.name() {
                    Some(k) => match k.to_ascii_lowercase().as_str() {
                        "sum" => (Field::new(self.name().unwrap_or_default()), AggFn::Sum),
                        "avg" | "average" => {
                            (Field::new(self.name().unwrap_or_default()), AggFn::Avg)
                        }
                        _ => (Field::new(""), AggFn::Count),
                    },
                    None => (Field::new(""), AggFn::Count),
                };
                self.view.aggregates = vec![pair];
                Filter::True
            }
            "group-by" => {
                let field = self.name().unwrap_or_else(|| "page".to_string());
                self.view.group_by = Some(Field::new(field));
                Filter::True
            }
            unknown => {
                // THE CATALOGUED BUG: OG returns nil here and Tine used to drop
                // the rest of the query with it.
                let start = self.toks[open].start;
                self.skip_to_close();
                let end = self
                    .toks
                    .get(self.pos.saturating_sub(1))
                    .map(|t| t.end)
                    .unwrap_or(self.src.len());
                // `head()`'s caller consumes one more `)`; we already did.
                self.pos = self.pos.saturating_sub(1);
                let span = Some(Span::from_byte_range(self.src, start, end));
                self.diagnose(
                    DiagnosticKind::UnknownHead,
                    format!("`{unknown}` is not a query filter"),
                    span,
                );
                Filter::Raw {
                    text: self.src[start..end.min(self.src.len())].to_string(),
                    kind: DiagnosticKind::UnknownHead,
                    span,
                }
            }
        };
        Some(filter)
    }

    /// A property VALUE: like a name, but a `[[page]]` / `#tag` token is unwrapped
    /// exactly as OG's `parse-property-value` unwraps it.
    fn opt_property_value(&mut self) -> Option<String> {
        match self.peek() {
            Some(Tok::Word(_)) | Some(Tok::Str(_)) | Some(Tok::PageRef(_)) => {
                self.name().map(|raw| parse_property_value(&raw))
            }
            _ => None,
        }
    }

    /// `(between [FIELD] START END)`. OG's fieldless form is journal-only; the
    /// optional leading field keyword is Tine's extension and `any` keeps its
    /// journal-or-planning reading.
    fn between(&mut self) -> Filter {
        #[derive(Clone, Copy, PartialEq)]
        enum BetweenField {
            Journal,
            Scheduled,
            Deadline,
            Any,
        }
        let field = match self.peek() {
            Some(Tok::Word(w)) => match w.to_ascii_lowercase().as_str() {
                "journal" => {
                    self.pos += 1;
                    BetweenField::Journal
                }
                "scheduled" => {
                    self.pos += 1;
                    BetweenField::Scheduled
                }
                "deadline" => {
                    self.pos += 1;
                    BetweenField::Deadline
                }
                "any" => {
                    self.pos += 1;
                    BetweenField::Any
                }
                _ => BetweenField::Journal,
            },
            _ => BetweenField::Journal,
        };
        // OG's `pre-transform` keywordizes a signed or unit-suffixed bound
        // (`-7d` → `:-7d`) before its own reader sees it, and a query saved by
        // OG carries the keyword form on disk. Tine parses the original text,
        // so it accepts both spellings and stores one.
        let bound = |token: Option<String>| -> Option<String> {
            token.map(|token| token.strip_prefix(':').unwrap_or(&token).to_string())
        };
        let low = bound(self.name());
        let high = bound(self.name());
        let range = |attr: Attr| bounded(attr, low.as_deref(), high.as_deref());
        match field {
            BetweenField::Journal => through_page(range(Attr::Day)),
            BetweenField::Scheduled => range(Attr::Scheduled),
            BetweenField::Deadline => range(Attr::Deadline),
            BetweenField::Any => Filter::or(vec![
                through_page(range(Attr::Day)),
                range(Attr::Scheduled),
                range(Attr::Deadline),
            ]),
        }
    }
}

/// A date range leaf. Bounds stay UNRESOLVED in the IR (`Value::Date` carries
/// the literal); an open side becomes the one-sided comparison and two open
/// sides become plain presence, which is what today's `(between)` with two
/// unresolvable bounds already means.
fn bounded(attr: Attr, low: Option<&str>, high: Option<&str>) -> Filter {
    match (low, high) {
        (Some(low), Some(high)) => Filter::attr(
            attr,
            CmpOp::Between,
            Value::List {
                items: vec![Value::date(low), Value::date(high)],
            },
        ),
        (Some(low), None) => Filter::attr(attr, CmpOp::Ge, Value::date(low)),
        (None, Some(high)) => Filter::attr(attr, CmpOp::Le, Value::date(high)),
        (None, None) => Filter::attr(attr, CmpOp::IsSet, Value::None),
    }
}

fn text_list(values: Vec<String>) -> Value {
    Value::List {
        items: values.into_iter().map(Value::text).collect(),
    }
}

/// `(property k)` / `(property k v)` in the one shape §3.3 defines: a `props`
/// relation whose predicate is the key equality, optionally conjoined with one
/// atom test.
pub(crate) fn property_leaf(key: String, value: Option<String>) -> Filter {
    let key_test = Filter::attr(Attr::Key, CmpOp::Eq, Value::text(key));
    let pred = match value {
        Some(value) => Filter::and(vec![
            key_test,
            Filter::attr(Attr::Value, CmpOp::Eq, Value::text(value)),
        ]),
        None => key_test,
    };
    Filter::rel(Rel::Props, Quant::Any, pred)
}

/// A bare string is a case-insensitive substring test on the block's visible
/// content — SQL's `content like '%x%'`, with `%`/`_` in the user's text escaped.
pub(crate) fn content_like(text: &str) -> Filter {
    Filter::attr(
        Attr::Content,
        CmpOp::Like,
        Value::text(format!("%{}%", escape_like(text))),
    )
}

/// Escape the LIKE metacharacters so a literal `%`/`_`/`\` in user text is data.
pub(crate) fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// The inverse of [`escape_like`] for a pattern that is exactly `%<literal>%`.
pub(crate) fn plain_like_substring(pattern: &str) -> Option<String> {
    let inner = pattern.strip_prefix('%')?.strip_suffix('%')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push(chars.next()?),
            '%' | '_' => return None,
            other => out.push(other),
        }
    }
    Some(out)
}

/// Parse one OG `{{query …}}` macro argument into the IR and its view settings.
///
/// `text` is the macro argument (or, from the `macro_query` dispatch, the form
/// slice it already split). The trailing OG options map is split off by the one
/// shared `split_trailing_map` (§4.3.1 W3) and preserved verbatim in
/// `Source::Og.og_options`; `Source::Og.original` is the exact FORM slice
/// without it (§3.1), so a caller re-emits the pair rather than re-deriving the
/// boundary. Splitting a form that has no trailing map is a no-op, so the
/// dispatch may split first and still call in here.
pub(crate) fn parse_og(text: &str, _today: JournalDate) -> (Query, ViewSettings) {
    let (form, og_options) = split_trailing_map(text);
    let mut parse = OgParse {
        toks: tokenize(&form),
        pos: 0,
        src: &form,
        view: ViewSettings::default(),
        diagnostics: Vec::new(),
        blocks: false,
    };
    let mut items = Vec::new();
    while parse.pos < parse.toks.len() {
        match parse.expr(0) {
            Some(filter) => items.push(filter),
            None => {
                // A form that does not parse at all — a stray `)`, or a head
                // whose required argument is missing. Unlike an unknown head
                // (which becomes `Raw` and keeps the rest of the query alive),
                // there is no shape here to preserve, so the query is reported
                // as invalid rather than silently answered with a filter the
                // author never wrote.
                if !parse
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == DiagnosticKind::Syntax)
                {
                    parse.diagnostics.push(Diagnostic::new(
                        DiagnosticKind::Syntax,
                        "the query does not parse",
                    ));
                }
                parse.pos += 1;
            }
        }
    }
    let items = lift_directives(items);
    let filter = match items.len() {
        0 => Filter::True,
        1 => items.into_iter().next().expect("one"),
        _ => Filter::and(items),
    };
    let anchor = if parse.blocks {
        Anchor::Block
    } else {
        Anchor::Page
    };
    let filter = match anchor {
        Anchor::Block => filter,
        Anchor::Page => rebase_to_page(filter),
    };
    let query = Query {
        anchor,
        filter,
        diagnostics: parse.diagnostics,
        source: Source::Og {
            original: form.clone(),
            og_options,
        },
    };
    let view = parse.view;
    (query, view)
}

/// **A view directive contributes no filter operand** (§3.5). `sort-by`,
/// `sample`, `aggregate` and `group-by` are lifted into [`ViewSettings`]; the
/// head handler returns `Filter::True` as its "nothing here" value, and this
/// removes it from the group it appeared in. The OG DSL has no boolean literal,
/// so every `True` an OG parse produces IS a lifted directive — which is why
/// this belongs here and not in `normalized()`, where dropping a constant would
/// change what a disabled sibling means (R2).
fn lift_directives(items: Vec<Filter>) -> Vec<Filter> {
    items
        .into_iter()
        .filter(|item| *item != Filter::True)
        .collect()
}

/// The OG half of the ONE `split_trailing_map` (§4.3.1 W3).
///
/// The scan itself moved to [`crate::query::macro_text`] when TQL and advanced
/// forms gained a trailing options map too: one splitter, parameterized by the
/// language family, rather than a second copy per grammar (I-12, D-14).
pub(crate) fn split_trailing_map(arg: &str) -> (String, String) {
    crate::query::macro_text::split_trailing_map(arg, crate::query::macro_text::FormFamily::Edn)
}

/// The inverse of [`rebase_to_page`]: put every page-row leaf of a
/// `@page`-anchored filter back behind a `page` hop, so the legacy block-group
/// walk evaluates it exactly as it always has (a `(page-property …)` query
/// returns the BLOCKS of the matching pages). At the `@page` anchor every leaf
/// is a page-row leaf, so the wrap is total and exact.
pub(crate) fn rebase_to_block(filter: &Filter) -> Filter {
    match filter {
        Filter::And { items } => Filter::and(items.iter().map(rebase_to_block).collect()),
        Filter::Or { items } => Filter::or(items.iter().map(rebase_to_block).collect()),
        Filter::Not { inner } => Filter::not(rebase_to_block(inner)),
        Filter::Off { inner } => Filter::off(rebase_to_block(inner)),
        Filter::Leaf { .. } => through_page(filter.clone()),
        other => other.clone(),
    }
}

/// The token stream without spans, for the shape guards that only classify
/// tokens (the managed sparse path's strictness checks).
pub(crate) fn tokens_only(src: &str) -> Vec<Tok> {
    tokenize(src)
        .into_iter()
        .map(|spanned| spanned.tok)
        .collect()
}

/// Whether `tokens` is exactly ONE balanced expression that consumes the whole
/// source — the shape the managed sparse path requires before it will enumerate
/// a narrowed candidate stream.
pub(crate) fn is_single_expression(tokens: &[Tok]) -> bool {
    fn walk(tokens: &[Tok], position: &mut usize) -> bool {
        match tokens.get(*position) {
            Some(Tok::LParen) => {
                *position += 1;
                while !matches!(tokens.get(*position), Some(Tok::RParen)) {
                    if !walk(tokens, position) {
                        return false;
                    }
                }
                *position += 1;
                true
            }
            Some(Tok::RParen) | None => false,
            Some(_) => {
                *position += 1;
                true
            }
        }
    }
    let mut position = 0;
    walk(tokens, &mut position) && position == tokens.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Query {
        parse_og(source, JournalDate::from_ordinal(20260904)).0
    }

    /// The parser runs on the ORIGINAL text so `Raw` spans stay exact offsets
    /// into what the author wrote, while OG rewrites the text first. The two
    /// must agree on every shape `pre-transform` touches — that agreement is the
    /// only reason the tokenizer is allowed to skip the rewrite (D-9).
    #[test]
    fn pre_transform_equivalent_shapes() {
        for form in [
            "[[Alpha]]",
            "#Alpha",
            "(and #Alpha #Beta)",
            "(and [[Alpha]] #Beta)",
            "(property note \"a #b c\")",
            "(between scheduled -7d today)",
            "(between journal 2026-01-01 2026-12-31)",
            "(task TODO)",
            "(and (task TODO) [[Alpha]])",
            "\"a plain string\"",
            "(page-tags a b)",
        ] {
            let direct = parse(form);
            let rewritten = parse(&pre_transform(form));
            assert_eq!(
                rewritten.normalized().filter,
                direct.normalized().filter,
                "{form:?} rewrote to {:?}",
                pre_transform(form)
            );
        }
    }

    /// REG-P0-QUERY-UNKNOWN-HEAD-001. OG's `build-query` returns `nil` for an
    /// unrecognised head; the old Tine parser propagated that as "the parse
    /// failed from here" and silently ran a DIFFERENT, shorter query.
    #[test]
    fn an_unknown_head_keeps_the_rest_of_the_query_and_reports_itself() {
        let query = parse("(and (task TODO) (frobnicate x))");
        assert!(
            query.is_invalid(),
            "an unknown head must invalidate, not narrow: {:?}",
            query.diagnostics
        );
        assert!(
            query
                .diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::UnknownHead),
            "{:?}",
            query.diagnostics
        );
        let Filter::And { items } = &query.filter else {
            panic!("the conjunction survives: {:?}", query.filter);
        };
        assert_eq!(items.len(), 2, "{:?}", query.filter);
        assert!(
            matches!(&items[1], Filter::Raw { .. }),
            "the unknown head is preserved as `Raw`: {:?}",
            items[1]
        );
    }

    #[test]
    fn an_unknown_head_alone_is_raw_and_spanned() {
        let query = parse("(frobnicate x)");
        let Filter::Raw { text, span, kind } = &query.filter else {
            panic!("{:?}", query.filter);
        };
        assert_eq!(text, "(frobnicate x)");
        assert!(span.is_some(), "the raw node carries its source span");
        assert_eq!(
            *kind,
            DiagnosticKind::UnknownHead,
            "the capsule retains the kind that rejected it (§4.3.2)"
        );
    }

    /// REG-P0-QUERY-ALL-PAGE-TAGS-001. OG has `(all-page-tags)`; Tine did not,
    /// so a graph carrying one silently returned nothing.
    #[test]
    fn all_page_tags_is_the_pages_tags_property_with_at_least_one_atom() {
        let query = parse("(all-page-tags)");
        assert!(!query.is_invalid(), "{:?}", query.diagnostics);
        assert_eq!(query.anchor, Anchor::Page);
        assert_eq!(
            query.filter,
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                    Filter::attr(Attr::AtomCount, CmpOp::Gt, Value::Number { number: 0.0 }),
                ])
            )
        );
    }

    #[test]
    fn all_page_tags_takes_no_arguments() {
        let query = parse("(all-page-tags extra)");
        assert!(query
            .diagnostics
            .iter()
            .any(|d| d.message.contains("takes no arguments")));
    }
}
