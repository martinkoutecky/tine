//! TQL → IR (SPEC §4.2).
//!
//! TQL is **SQLite expression syntax over a fixed vocabulary**. There is no
//! hand-rolled grammar here (D-14): a deterministic pre-pass rewrites the three
//! things that are not SQL (`@block`/`@page`, `[[x]]`/`#x`, `-- ` disabled
//! runs), and everything after that is [`sqlparser`] with `SQLiteDialect`.
//!
//! **The vocabulary is a whitelist by construction.** Lowering is an exhaustive
//! match over the `Expr` shapes §4.2.3 names; every other shape — an unknown
//! identifier, an unknown function, an operator outside [`CmpOp`] — produces a
//! diagnostic and never a silently reinterpreted filter (I-22: query text is
//! hostile outside content). A pre-visit additionally rejects the four AST
//! shapes that must never reach lowering at all (subqueries, `EXISTS`,
//! `IN (SELECT …)`, placeholders), so their rejection message names what they
//! are rather than what position they appeared in.

use std::ops::ControlFlow;

use sqlparser::ast::{
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, UnaryOperator,
    Value as SqlValue, Visit, Visitor,
};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::Token;

use crate::query::ir::{
    decode_raw_hex, encode_raw_hex, Anchor, Attr, CapsuleError, CmpOp, Diagnostic, DiagnosticKind,
    Filter, Quant, Query, Rel, Source, Span, Value, ValueType, ViewSettings,
};

/// SPEC §4.2.2 guard 3. Deep enough for any authored query, shallow enough that
/// a pathological nesting cannot exhaust the stack inside the parser.
const RECURSION_LIMIT: usize = 64;

/// The ONE TQL entry point: text → `Query`, against a registry snapshot.
///
/// The registry changes exactly one thing: an unknown identifier or function
/// names the nearest property keys the graph actually has (§4.2.2). Nothing is
/// ever rewritten — a suggestion is text. Pass
/// [`crate::query::registry::Registry::none`] where there is no graph.
pub(crate) fn parse_tql(
    text: &str,
    registry: &crate::query::registry::Registry,
) -> (Query, ViewSettings) {
    parse_tql_with_options(text, String::new(), registry)
}

/// [`parse_tql`] carrying an opaque trailing options map the macro dispatch
/// already split off (§4.3.1, X4).
///
/// The pane never supplies one — pane text is filter/anchor only and an
/// appended map there is a diagnostic, not a silent option edit — so this exists
/// for the `macro_tql` input, where the map is part of the persisted bytes and
/// must survive verbatim into `Source::Tql.og_options`.
pub(crate) fn parse_tql_with_options(
    text: &str,
    og_options: String,
    registry: &crate::query::registry::Registry,
) -> (Query, ViewSettings) {
    let mut diagnostics = Vec::new();
    let pre = pre_pass(text, &mut diagnostics);
    let filter = match parse_expr_guarded(&pre.sql) {
        Ok(expr) => match reject_forbidden_shapes(&expr) {
            Some(message) => {
                diagnostics.push(Diagnostic::new(DiagnosticKind::Syntax, message));
                Filter::False
            }
            None => {
                let mut lower = Lower {
                    anchor: pre.anchor,
                    diagnostics: &mut diagnostics,
                    registry,
                    disabled_depth: 0,
                };
                let scope = match pre.anchor {
                    Anchor::Block => Scope::Block,
                    Anchor::Page => Scope::Page,
                };
                lower.filter(&expr, scope)
            }
        },
        Err(message) => {
            diagnostics.push(Diagnostic::new(DiagnosticKind::Syntax, message));
            Filter::False
        }
    };
    let query = Query {
        anchor: pre.anchor,
        filter: if pre.empty { Filter::True } else { filter },
        diagnostics,
        source: Source::Tql {
            original: text.to_string(),
            og_options,
        },
    };
    (query, ViewSettings::default())
}

// ---------------------------------------------------------------------------
// 4.2.1 Pre-pass
// ---------------------------------------------------------------------------

struct PrePass {
    sql: String,
    anchor: Anchor,
    /// The anchor token was the whole query: every row of the anchor.
    empty: bool,
}

/// Byte ranges of every `'…'` string literal, quotes included, with SQL's `''`
/// doubling honoured.
///
/// **M18's invariant, not its letter.** The spec describes one lexical scan
/// whose literal map every later step consults; the steps below re-derive the
/// map after each rewrite instead, because a rewrite moves the offsets. What
/// M18 buys — nothing inside a literal is ever recognised or rewritten,
/// including a line beginning `-- ` inside a multi-line literal — is exactly
/// what re-deriving preserves.
fn literal_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\'' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                i += 1;
                break;
            }
            i += 1;
        }
        spans.push((start, i));
    }
    spans
}

fn inside_literal(spans: &[(usize, usize)], at: usize) -> bool {
    spans.iter().any(|(start, end)| at >= *start && at < *end)
}

fn pre_pass(text: &str, diagnostics: &mut Vec<Diagnostic>) -> PrePass {
    let (anchor, rest, offset) = take_anchor(text);
    report_stray_anchor(text, &rest, offset, diagnostics);
    report_unquoted_relative_dates(text, &rest, offset, diagnostics);
    if rest.trim().is_empty() {
        return PrePass {
            sql: "true".to_string(),
            anchor,
            empty: true,
        };
    }
    // **Disabled runs are isolated FIRST (§4.2.1, §4.3.2 R4).** A run's payload
    // may be malformed — an unmatched quote or bracket — and scanning it as
    // part of the surrounding text lets it swallow the next ACTIVE row, which
    // is exactly the defect `-- task = '` followed by a valid row exposes.
    // Each run's payload is desugared in isolation with this same scanner.
    let lifted = lift_disabled_runs(&rest, diagnostics);
    let sql = desugar(&lifted, diagnostics);
    PrePass {
        sql,
        anchor,
        empty: false,
    }
}

/// Step 1: a leading `@block` / `@page`, plus one following `and`.
fn take_anchor(text: &str) -> (Anchor, String, usize) {
    let lead = text.len() - text.trim_start().len();
    let body = &text[lead..];
    let lower = body.to_ascii_lowercase();
    for (token, anchor) in [("@block", Anchor::Block), ("@page", Anchor::Page)] {
        if !lower.starts_with(token) {
            continue;
        }
        let after = &body[token.len()..];
        if after
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let trimmed = after.trim_start();
        let skipped = after.len() - trimmed.len();
        let mut consumed = lead + token.len() + skipped;
        let rest = if trimmed.len() >= 3
            && trimmed[..3].eq_ignore_ascii_case("and")
            && !trimmed[3..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            consumed += 3;
            &trimmed[3..]
        } else {
            trimmed
        };
        return (anchor, rest.to_string(), consumed);
    }
    (Anchor::Block, text.to_string(), 0)
}

/// `@` anywhere but the front is the author reaching for an anchor in the wrong
/// place — never silently ignored.
fn report_stray_anchor(
    original: &str,
    rest: &str,
    offset: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let spans = literal_spans(rest);
    if let Some(at) = rest
        .char_indices()
        .find(|(index, ch)| *ch == '@' && !inside_literal(&spans, *index))
        .map(|(index, _)| index)
    {
        diagnostics.push(
            Diagnostic::new(DiagnosticKind::Syntax, "the anchor goes first").with_span(Some(
                Span::from_byte_range(original, offset + at, offset + at + 1),
            )),
        );
    }
}

/// Where the next `[[x]]` / `#x` sits: a value position spells the page NAME as
/// a string literal (OG `parse-property-value`, M19), anywhere else it is a
/// `refs` leaf.
#[derive(Clone, Copy, PartialEq)]
enum Prev {
    Start,
    Cmp,
    Open,
    Comma,
    Other,
}

/// Step 2: sugar. `[[x]]` → `ref('x')`, `#x` → `ref('x')`, except in value
/// position where both become the string literal `'x'`.
fn desugar(text: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut prev = Prev::Start;
    // One entry per open paren: whether it is the list of an `in`.
    let mut parens: Vec<bool> = Vec::new();
    let mut pending_in = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let value_position = prev == Prev::Cmp
            || (matches!(prev, Prev::Open | Prev::Comma) && parens.last() == Some(&true));
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                out.push(bytes[i] as char);
                i += 1;
            }
            b'\'' => {
                let end = literal_end(text, i);
                out.push_str(&text[i..end]);
                prev = Prev::Other;
                i = end;
            }
            b'[' if text[i..].starts_with("[[") => {
                match text[i + 2..].find("]]") {
                    Some(offset) => {
                        let inner = &text[i + 2..i + 2 + offset];
                        emit_page_name(&mut out, inner, value_position);
                        i += 2 + offset + 2;
                    }
                    None => {
                        diagnostics.push(Diagnostic::new(
                            DiagnosticKind::Syntax,
                            "a page reference opened with `[[` is never closed",
                        ));
                        out.push_str(&text[i..]);
                        i = bytes.len();
                    }
                }
                prev = Prev::Other;
            }
            b'#' => {
                let mut end = i + 1;
                while end < bytes.len() {
                    let ch = text[end..].chars().next().expect("boundary");
                    if ch.is_whitespace() || matches!(ch, '(' | ')' | ',') {
                        break;
                    }
                    end += ch.len_utf8();
                }
                if end == i + 1 {
                    out.push('#');
                } else {
                    emit_page_name(&mut out, &text[i + 1..end], value_position);
                }
                prev = Prev::Other;
                i = end;
            }
            b'(' => {
                parens.push(pending_in);
                pending_in = false;
                out.push('(');
                prev = Prev::Open;
                i += 1;
            }
            b')' => {
                parens.pop();
                out.push(')');
                prev = Prev::Other;
                i += 1;
            }
            b',' => {
                out.push(',');
                prev = Prev::Comma;
                i += 1;
            }
            b'=' | b'<' | b'>' | b'!' => {
                let start = i;
                while i < bytes.len() && matches!(bytes[i], b'=' | b'<' | b'>' | b'!') {
                    i += 1;
                }
                out.push_str(&text[start..i]);
                prev = Prev::Cmp;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                let word = &text[start..i];
                out.push_str(word);
                pending_in = word.eq_ignore_ascii_case("in");
                // The word-spelled comparison operators put the next token in
                // value position exactly as `=` does.
                prev = if ["like", "match", "between"]
                    .iter()
                    .any(|op| word.eq_ignore_ascii_case(op))
                {
                    Prev::Cmp
                } else {
                    Prev::Other
                };
            }
            _ => {
                let ch = text[i..].chars().next().expect("boundary");
                out.push(ch);
                prev = Prev::Other;
                i += ch.len_utf8();
            }
        }
    }
    out
}

fn emit_page_name(out: &mut String, inner: &str, value_position: bool) {
    let quoted = format!("'{}'", inner.replace('\'', "''"));
    if value_position {
        out.push_str(&quoted);
    } else {
        out.push_str("ref(");
        out.push_str(&quoted);
        out.push(')');
    }
}

/// The end offset (exclusive) of the `'…'` literal starting at `start`.
fn literal_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// `today` is a vocabulary identifier; every other relative date is quoted. An
/// unquoted `-7d` would parse as arithmetic on an unknown identifier, so it is
/// caught here where the suggestion can name the fix.
fn report_unquoted_relative_dates(
    original: &str,
    text: &str,
    offset: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let spans = literal_spans(text);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if !matches!(bytes[i], b'-' | b'+') || inside_literal(&spans, i) {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == i + 1 || !bytes.get(end).is_some_and(|b| b"dwmy".contains(b)) {
            i += 1;
            continue;
        }
        let unit = end;
        end += 1;
        if bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            i += 1;
            continue;
        }
        let literal = &text[i..=unit];
        diagnostics.push(
            Diagnostic {
                suggestions: vec![format!("quote relative dates: '{literal}'")],
                ..Diagnostic::new(
                    DiagnosticKind::Syntax,
                    format!("`{literal}` is a relative date and must be quoted"),
                )
            }
            .with_span(Some(Span::from_byte_range(
                original,
                offset + i,
                offset + end,
            ))),
        );
        i = end;
    }
}

/// One `-- ` run and where it sits in the text.
struct DisabledRun {
    first_line: usize,
    last_line: usize,
    indent: String,
    payload: String,
}

/// Step 3 (Q12): a maximal run of `-- ` lines becomes a positional
/// `<connector> off(<rest>)`. Positional replacement is what makes nesting free:
/// a run inside a parenthesized group becomes an `off()` operand of that group.
fn lift_disabled_runs(text: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    let spans = literal_spans(text);
    let mut lines: Vec<String> = Vec::new();
    let mut kinds: Vec<Option<(String, String)>> = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\n') {
        let indent_len = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        let starts_in_literal = inside_literal(&spans, offset + indent_len);
        let payload = if starts_in_literal || trimmed == "--" || !trimmed.starts_with("-- ") {
            None
        } else {
            let rest = trimmed[3..].trim();
            (!rest.is_empty()).then(|| (line[..indent_len].to_string(), rest.to_string()))
        };
        kinds.push(payload);
        lines.push(line.to_string());
        offset += line.len() + 1;
    }

    let mut runs: Vec<DisabledRun> = Vec::new();
    let mut index = 0usize;
    while index < kinds.len() {
        let Some((indent, payload)) = kinds[index].clone() else {
            index += 1;
            continue;
        };
        let first_line = index;
        let mut joined = payload;
        index += 1;
        while let Some(Some((_, next))) = kinds.get(index) {
            joined.push(' ');
            joined.push_str(next);
            index += 1;
        }
        runs.push(DisabledRun {
            first_line,
            last_line: index - 1,
            indent,
            payload: joined,
        });
    }

    for run in &runs {
        let (connector, rest) = split_connector(&run.payload);
        // The isolated operand gets the same sugar treatment as active text,
        // from the same scanner (§4.2.1) — a disabled `[[a]]` is still a ref.
        // Its diagnostics are disabled: a broken row a user turned off must not
        // invalidate the query (§3.5).
        let mut inner_diagnostics = Vec::new();
        let sugared = desugar(rest, &mut inner_diagnostics);
        let parses = parse_expr_guarded(&sugared).is_ok();
        if parses {
            for diagnostic in inner_diagnostics {
                diagnostics.push(Diagnostic {
                    disabled: true,
                    ..diagnostic
                });
            }
        }
        let replacement = if parses {
            format!("{}{connector}off({sugared})", run.indent)
        } else {
            // **Captured before the surrounding parse, never substituted with
            // `off(false)` (§4.2.1, R4).** `off(false)` is a lie the author
            // never wrote and it destroys the payload; the capsule keeps the
            // exact bytes, so a save and reopen returns the same broken row and
            // the renderer can show what the author typed. The disabled
            // diagnostic that goes with it is derived at lowering, from the
            // `off(…)` this replacement puts around it.
            format!(
                "{}{connector}off(raw_hex('{}', '{}'))",
                run.indent,
                DiagnosticKind::Syntax.capsule_name(),
                encode_raw_hex(rest)
            )
        };
        lines[run.first_line] = replacement;
        for line in run.first_line + 1..=run.last_line {
            lines[line] = String::new();
        }
    }
    lines.join("\n")
}

fn split_connector(payload: &str) -> (&'static str, &str) {
    for (word, connector) in [("and", "and "), ("or", "or ")] {
        if payload.len() > word.len()
            && payload[..word.len()].eq_ignore_ascii_case(word)
            && payload.as_bytes()[word.len()].is_ascii_whitespace()
        {
            return (connector, payload[word.len()..].trim_start());
        }
    }
    ("", payload)
}

// ---------------------------------------------------------------------------
// 4.2.2 Guards
// ---------------------------------------------------------------------------

fn parse_expr_guarded(sql: &str) -> Result<Expr, String> {
    let dialect = SQLiteDialect {};
    let mut parser = Parser::new(&dialect)
        .with_recursion_limit(RECURSION_LIMIT)
        .try_with_sql(sql)
        .map_err(|error| error.to_string())?;
    let expr = parser.parse_expr().map_err(|error| error.to_string())?;
    if parser.peek_token().token != Token::EOF {
        return Err(format!(
            "the query ends after a complete condition; `{}` is left over",
            parser.peek_token()
        ));
    }
    Ok(expr)
}

struct ForbiddenShapes;

impl Visitor for ForbiddenShapes {
    type Break = &'static str;

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        let rejected = match expr {
            Expr::Subquery(_) => Some("a subquery is not part of the query language"),
            Expr::Exists { .. } => Some("`exists` is not part of the query language"),
            Expr::InSubquery { .. } => Some("`in (select …)` is not part of the query language"),
            Expr::Value(value) => matches!(
                value.value,
                SqlValue::Placeholder(_) | SqlValue::DollarQuotedString(_)
            )
            .then_some("a placeholder is not part of the query language"),
            _ => None,
        };
        match rejected {
            Some(message) => ControlFlow::Break(message),
            None => ControlFlow::Continue(()),
        }
    }
}

fn reject_forbidden_shapes(expr: &Expr) -> Option<&'static str> {
    match expr.visit(&mut ForbiddenShapes) {
        ControlFlow::Break(message) => Some(message),
        ControlFlow::Continue(()) => None,
    }
}

// ---------------------------------------------------------------------------
// 4.2.3 Vocabulary → IR
// ---------------------------------------------------------------------------

/// What a bare identifier binds to at this point in the expression. Inside a
/// relation predicate, identifiers bind to the ELEMENT, never to the outer row
/// (§3.2).
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Block,
    Page,
    /// The atom of one property key: the single identifier `value`.
    Atom,
}

/// The left-hand side of a comparison, resolved.
enum Target {
    Attr {
        through_page: bool,
        attr: Attr,
        ty: ValueType,
    },
    /// A property element of the block (or, with `through_page`, of its page).
    Prop { through_page: bool, key: String },
    /// The contextual `value` identifier inside a property-atom expression.
    Atom,
}

struct Lower<'a> {
    anchor: Anchor,
    diagnostics: &'a mut Vec<Diagnostic>,
    /// The snapshot `UnknownIdent` suggestions are drawn from (§4.2.2, §6.2).
    /// Never consulted for anything else: the vocabulary is the whitelist in
    /// this file, not whatever keys a graph happens to hold.
    registry: &'a crate::query::registry::Registry,
    /// How many `off(…)` calls enclose the node being lowered. A diagnostic
    /// raised inside one is `disabled` — the row renders greyed with its
    /// message and does NOT invalidate the query (§3.5). Disabled state is
    /// DERIVED from the current tree, never stored on the node (§4.3.2).
    disabled_depth: usize,
}

impl Lower<'_> {
    fn reject(&mut self, kind: DiagnosticKind, message: impl Into<String>) -> Filter {
        self.diagnose(Diagnostic::new(kind, message));
        Filter::False
    }

    /// Record a diagnostic, marking it `disabled` when it was raised inside an
    /// `off(…)` (§3.5). Every diagnostic this lowering produces goes through
    /// here, so the derivation has exactly one implementation.
    fn diagnose(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(Diagnostic {
            disabled: self.disabled_depth > 0,
            ..diagnostic
        });
    }

    fn filter(&mut self, expr: &Expr, scope: Scope) -> Filter {
        match expr {
            Expr::Nested(inner) => self.filter(inner, scope),
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => Filter::not(self.filter(expr, scope)),
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => {
                    Filter::and(vec![self.filter(left, scope), self.filter(right, scope)])
                }
                BinaryOperator::Or => {
                    Filter::or(vec![self.filter(left, scope), self.filter(right, scope)])
                }
                BinaryOperator::Regexp => self.regexp(left, right, scope),
                _ => match binary_cmp(op) {
                    Some(op) => self.compare(left, op, right, scope),
                    None => self.reject(
                        DiagnosticKind::Syntax,
                        format!("`{op}` is not a comparison the query language has"),
                    ),
                },
            },
            Expr::IsNull(inner) => self.presence(inner, CmpOp::IsNotSet, scope),
            Expr::IsNotNull(inner) => self.presence(inner, CmpOp::IsSet, scope),
            Expr::InList {
                expr,
                list,
                negated,
            } => self.in_list(expr, list, *negated, scope),
            Expr::Between {
                expr,
                negated,
                low,
                high,
            } => {
                let filter = self.between(expr, low, high, scope);
                if *negated {
                    Filter::not(filter)
                } else {
                    filter
                }
            }
            Expr::Like {
                negated,
                any: false,
                expr,
                pattern,
                escape_char: _,
            } => {
                let filter = self.like(expr, pattern, scope);
                if *negated {
                    Filter::not(filter)
                } else {
                    filter
                }
            }
            // Measured on sqlparser 0.62.0: plain `x regexp 'p'` arrives as
            // `BinaryOp { op: Regexp }` and only the NEGATED form arrives as
            // `RLike { regexp: true }`. The `RLIKE` alias is `regexp: false` and
            // is deliberately NOT admitted (§4.3.2): one spelling, one operator.
            Expr::RLike {
                negated,
                expr,
                pattern,
                regexp: true,
            } => {
                let filter = self.regexp(expr, pattern, scope);
                if *negated {
                    Filter::not(filter)
                } else {
                    filter
                }
            }
            Expr::Function(function) => self.function(function, scope),
            Expr::Value(value) => match &value.value {
                SqlValue::Boolean(true) => Filter::True,
                SqlValue::Boolean(false) => Filter::False,
                other => self.reject(
                    DiagnosticKind::Syntax,
                    format!("`{other}` is a value, not a condition"),
                ),
            },
            Expr::Identifier(ident) => self.reject_ident(
                &ident.value,
                format!("`{ident}` is not a condition on its own"),
            ),
            other => self.reject(
                DiagnosticKind::Syntax,
                format!("`{other}` is not part of the query language"),
            ),
        }
    }

    // -- comparisons --------------------------------------------------------

    fn compare(&mut self, left: &Expr, op: CmpOp, right: &Expr, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let ty = self.value_type(&target, right);
        let Some(value) = self.value(right, ty) else {
            return Filter::False;
        };
        // `prop('k') = ''` is the IsBlank spelling (§4.2.3): present, and no
        // atoms. It is a property form only — `content = ''` is an ordinary
        // equality against the empty string.
        if matches!(target, Target::Prop { .. }) && op == CmpOp::Eq && value == Value::text("") {
            return self.build(target, CmpOp::IsBlank, Value::None, ty);
        }
        self.build(target, op, value, ty)
    }

    fn presence(&mut self, inner: &Expr, op: CmpOp, scope: Scope) -> Filter {
        let Some(target) = self.target(inner, scope) else {
            return Filter::False;
        };
        let ty = match &target {
            Target::Attr { ty, .. } => *ty,
            _ => ValueType::Text,
        };
        self.build(target, op, Value::None, ty)
    }

    fn in_list(&mut self, left: &Expr, list: &[Expr], negated: bool, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let ty = list
            .first()
            .map(|first| self.value_type(&target, first))
            .unwrap_or(ValueType::Text);
        let mut items = Vec::with_capacity(list.len());
        for item in list {
            match self.value(item, ty) {
                Some(value) => items.push(value),
                None => return Filter::False,
            }
        }
        let op = if negated { CmpOp::NotIn } else { CmpOp::In };
        self.build(target, op, Value::List { items }, ty)
    }

    fn between(&mut self, left: &Expr, low: &Expr, high: &Expr, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let ty = self.value_type(&target, low);
        let (Some(low), Some(high)) = (self.value(low, ty), self.value(high, ty)) else {
            return Filter::False;
        };
        self.build(
            target,
            CmpOp::Between,
            Value::List {
                items: vec![low, high],
            },
            ty,
        )
    }

    /// `content regexp '<pattern>'` (§4.2.3, §4.3.2). Content-only, text pattern.
    ///
    /// This SERIALIZES an operation Tine already performs — the OG
    /// `(content-regex "…")` head — so the semantics are exactly today's
    /// `regex::Regex` over the block's ORIGINAL visible text, case and inline
    /// flags included. It is not a second regex engine and not an opening for
    /// arbitrary functions. `op_applies` refuses it on every other row and type
    /// in the §4.2.3 matrix.
    fn regexp(&mut self, left: &Expr, pattern: &Expr, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let Some(Value::Text { text }) = self.value(pattern, ValueType::Text) else {
            return self.reject(
                DiagnosticKind::Syntax,
                "a `regexp` pattern is a quoted string",
            );
        };
        self.build(target, CmpOp::Regex, Value::text(text), ValueType::Text)
    }

    fn like(&mut self, left: &Expr, pattern: &Expr, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let Some(Value::Text { text }) = self.value(pattern, ValueType::Text) else {
            return self.reject(
                DiagnosticKind::Syntax,
                "a `like` pattern is a quoted string",
            );
        };
        match starts_with_prefix(&text) {
            Some(prefix) => self.build(
                target,
                CmpOp::StartsWith,
                Value::text(prefix),
                ValueType::Text,
            ),
            None => self.build(target, CmpOp::Like, Value::text(text), ValueType::Text),
        }
    }

    // -- leaf construction --------------------------------------------------

    fn build(&mut self, target: Target, op: CmpOp, value: Value, ty: ValueType) -> Filter {
        if !op_applies(&target, op, ty) {
            let what = match &target {
                Target::Attr { attr, .. } => format!("`{}`", attr_label(*attr)),
                Target::Prop { .. } | Target::Atom => format!("a {} property", ty.label()),
            };
            return self.reject(
                DiagnosticKind::Syntax,
                format!("`{}` does not apply to {what}", op_label(op)),
            );
        }
        match target {
            Target::Attr {
                through_page, attr, ..
            } => self.hop(through_page, Filter::attr(attr, op, value)),
            Target::Atom => Filter::attr(Attr::Value, op, value),
            Target::Prop { through_page, key } => {
                let key_test = Filter::attr(Attr::Key, CmpOp::Eq, Value::text(key));
                let leaf = match op {
                    CmpOp::IsSet => Filter::rel(Rel::Props, Quant::Any, key_test),
                    CmpOp::IsNotSet => Filter::rel(Rel::Props, Quant::None, key_test),
                    CmpOp::IsBlank => Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![
                            key_test,
                            Filter::attr(Attr::AtomCount, CmpOp::Eq, Value::Number { number: 0.0 }),
                        ]),
                    ),
                    op => Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![key_test, Filter::attr(Attr::Value, op, value)]),
                    ),
                };
                self.hop(through_page, leaf)
            }
        }
    }

    /// A page-row leaf reached from a block row needs the `page` hop; at the
    /// `@page` anchor the page row IS the current row.
    fn hop(&mut self, through_page: bool, filter: Filter) -> Filter {
        if through_page && self.anchor == Anchor::Block {
            Filter::rel(Rel::Page, Quant::Any, filter)
        } else {
            filter
        }
    }

    // -- identifiers --------------------------------------------------------

    fn target(&mut self, expr: &Expr, scope: Scope) -> Option<Target> {
        match expr {
            Expr::Nested(inner) => self.target(inner, scope),
            Expr::Identifier(ident) => {
                let name = ident.value.to_ascii_lowercase();
                let resolved = match scope {
                    Scope::Atom => (name == "value").then_some(Target::Atom),
                    Scope::Block => block_attr(&name).map(|(attr, ty)| Target::Attr {
                        through_page: false,
                        attr,
                        ty,
                    }),
                    Scope::Page => page_attr(&name).map(|(attr, ty)| Target::Attr {
                        through_page: true,
                        attr,
                        ty,
                    }),
                };
                if resolved.is_none() {
                    self.unknown_ident(&ident.value);
                }
                resolved
            }
            Expr::CompoundIdentifier(parts) => {
                let spelled = parts
                    .iter()
                    .map(|part| part.value.to_ascii_lowercase())
                    .collect::<Vec<_>>();
                if scope == Scope::Block && spelled.len() == 2 && spelled[0] == "page" {
                    if let Some((attr, ty)) = page_attr(&spelled[1]) {
                        return Some(Target::Attr {
                            through_page: true,
                            attr,
                            ty,
                        });
                    }
                }
                self.unknown_ident(&spelled.join("."));
                None
            }
            Expr::Function(function) => {
                let name = function_name(function);
                let args = function_args(function);
                match (name.as_str(), args.len()) {
                    ("prop", 1) => self.string_arg(args[0]).map(|key| Target::Prop {
                        through_page: scope == Scope::Page,
                        key: crate::doc::property_key_norm(&key),
                    }),
                    ("page_prop", 1) => self.string_arg(args[0]).map(|key| Target::Prop {
                        through_page: true,
                        key: crate::doc::property_key_norm(&key),
                    }),
                    _ => {
                        self.reject_ident(
                            &name,
                            format!("`{name}` is not something the query language compares"),
                        );
                        None
                    }
                }
            }
            other => {
                self.reject(
                    DiagnosticKind::Syntax,
                    format!("`{other}` is not something the query language compares"),
                );
                None
            }
        }
    }

    /// SPEC §4.2.2 guard 2. Suggestions are the registry's nearest keys —
    /// property keys the graph actually has, written as the `prop('…')` call
    /// that would have worked. Nothing is rewritten silently.
    fn unknown_ident(&mut self, name: &str) {
        let diagnostic = Diagnostic::new(
            DiagnosticKind::UnknownIdent,
            format!("`{name}` is not a field of this query"),
        );
        self.diagnostics.push(self.suggested(diagnostic, name));
    }

    /// Attach the registry's nearest keys to a diagnostic that named an
    /// identifier the vocabulary does not have.
    fn suggested(&self, mut diagnostic: Diagnostic, name: &str) -> Diagnostic {
        diagnostic.suggestions = self.registry.suggestions(name);
        diagnostic
    }

    /// `reject` for the identifier-shaped rejections, which carry suggestions.
    fn reject_ident(&mut self, name: &str, message: impl Into<String>) -> Filter {
        let diagnostic = Diagnostic::new(DiagnosticKind::UnknownIdent, message);
        self.diagnostics.push(self.suggested(diagnostic, name));
        Filter::False
    }

    // -- values -------------------------------------------------------------

    /// The compared type: an attribute's fixed type, or — for a property atom —
    /// the type the literal spells. This types the IR's *value*, not the atom:
    /// the atom is coerced at evaluation by its key's effective type (§6.3), so
    /// the same printed query answers correctly under either.
    fn value_type(&mut self, target: &Target, operand: &Expr) -> ValueType {
        match target {
            Target::Attr { ty, .. } => *ty,
            Target::Prop { .. } | Target::Atom => literal_type(operand),
        }
    }

    fn value(&mut self, expr: &Expr, ty: ValueType) -> Option<Value> {
        match expr {
            Expr::Nested(inner) => self.value(inner, ty),
            Expr::Identifier(ident) if ident.value.eq_ignore_ascii_case("today") => {
                Some(Value::date("today"))
            }
            Expr::UnaryOp {
                op: UnaryOperator::Minus,
                expr,
            } => match self.value(expr, ty)? {
                Value::Number { number } => Some(Value::Number { number: -number }),
                other => {
                    self.reject(
                        DiagnosticKind::Syntax,
                        format!("`-` does not apply to {other:?}"),
                    );
                    None
                }
            },
            Expr::Value(value) => match &value.value {
                SqlValue::SingleQuotedString(text) | SqlValue::DoubleQuotedString(text) => {
                    Some(if ty == ValueType::Date {
                        Value::date(text)
                    } else {
                        Value::text(text)
                    })
                }
                SqlValue::Number(text, _) => {
                    if ty == ValueType::Date {
                        return Some(Value::date(text));
                    }
                    match text.parse::<f64>() {
                        Ok(number) => Some(Value::Number { number }),
                        Err(_) => {
                            self.reject(
                                DiagnosticKind::Syntax,
                                format!("`{text}` is not a number"),
                            );
                            None
                        }
                    }
                }
                SqlValue::Boolean(value) => Some(Value::Bool { value: *value }),
                other => {
                    self.reject(
                        DiagnosticKind::Syntax,
                        format!("`{other}` is not a value the query language compares against"),
                    );
                    None
                }
            },
            other => {
                self.reject(
                    DiagnosticKind::Syntax,
                    format!("`{other}` is not a value the query language compares against"),
                );
                None
            }
        }
    }

    fn string_arg(&mut self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Value(value) => match &value.value {
                SqlValue::SingleQuotedString(text) | SqlValue::DoubleQuotedString(text) => {
                    Some(text.clone())
                }
                other => {
                    self.reject(
                        DiagnosticKind::Syntax,
                        format!("`{other}` is not a quoted name"),
                    );
                    None
                }
            },
            other => {
                self.reject(
                    DiagnosticKind::Syntax,
                    format!("`{other}` is not a quoted name"),
                );
                None
            }
        }
    }

    // -- functions used as conditions ---------------------------------------

    fn function(&mut self, function: &sqlparser::ast::Function, scope: Scope) -> Filter {
        let name = function_name(function);
        let args = function_args(function);
        match (name.as_str(), args.len()) {
            ("ref", 1) => match self.string_arg(args[0]) {
                Some(page) => Filter::page_ref(page),
                None => Filter::False,
            },
            ("tag", 1) if scope == Scope::Block => match self.string_arg(args[0]) {
                Some(tag) => Filter::rel(
                    Rel::Tags,
                    Quant::Any,
                    Filter::attr(Attr::Name, CmpOp::Eq, Value::text(tag)),
                ),
                None => Filter::False,
            },
            ("page_tag", 1) | ("tag", 1) => match self.string_arg(args[0]) {
                Some(tag) => {
                    let leaf = Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![
                            Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                            Filter::attr(Attr::Value, CmpOp::Eq, Value::text(tag)),
                        ]),
                    );
                    self.hop(true, leaf)
                }
                None => Filter::False,
            },
            ("off", 1) => {
                self.disabled_depth += 1;
                let inner = self.filter(args[0], scope);
                self.disabled_depth -= 1;
                Filter::off(inner)
            }
            ("raw_hex", 2) => self.raw_hex(args[0], args[1]),
            ("any", 2) => self.quantified(Quant::Any, args[0], args[1], scope),
            ("every", 2) => self.quantified(Quant::Every, args[0], args[1], scope),
            ("none", 2) => self.quantified(Quant::None, args[0], args[1], scope),
            _ => self.reject_ident(
                &name,
                format!("`{name}` is not a function of the query language"),
            ),
        }
    }

    /// `raw_hex('<kind>', '<hex>')` — the preservation capsule (§4.3.2, R4).
    ///
    /// Literal arguments only, decoded strictly, and **never evaluated or
    /// reparsed**: the result is a `Raw` node carrying the exact original
    /// payload and the kind that rejected it, plus that kind's diagnostic. A
    /// capsule that will not decode degrades to `Syntax` with the undecodable
    /// text retained — a corrupt capsule must not become an executable
    /// predicate, and it must not silently lose the author's bytes either.
    fn raw_hex(&mut self, kind_arg: &Expr, hex_arg: &Expr) -> Filter {
        let (Some(kind_name), Some(hex)) = (self.string_arg(kind_arg), self.string_arg(hex_arg))
        else {
            return Filter::False;
        };
        match decode_raw_hex(&kind_name, &hex, crate::query::QUERY_SOURCE_MAX_BYTES) {
            Ok((kind, text)) => {
                self.diagnose(Diagnostic::new(kind, retained_message(kind, &text)));
                Filter::raw(text, kind)
            }
            Err(error) => {
                let why = match error {
                    CapsuleError::UnknownKind => "an unknown kind",
                    CapsuleError::NotHex => "invalid hexadecimal",
                    CapsuleError::NotUtf8 => "bytes that are not text",
                    CapsuleError::TooLarge => "more text than a query may hold",
                };
                self.diagnose(Diagnostic::new(
                    DiagnosticKind::Syntax,
                    format!("this preserved condition carries {why} and cannot be read back"),
                ));
                Filter::raw(hex, DiagnosticKind::Syntax)
            }
        }
    }

    fn quantified(&mut self, quant: Quant, over: &Expr, pred: &Expr, scope: Scope) -> Filter {
        // `every(prop('k'), value op v)`: the collection is the atoms of one key
        // and the predicate is the atom test, conjoined with the key equality
        // that scopes it (§3.3).
        if let Expr::Function(function) = over {
            let name = function_name(function);
            let args = function_args(function);
            if matches!(name.as_str(), "prop" | "page_prop") && args.len() == 1 {
                let Some(key) = self.string_arg(args[0]) else {
                    return Filter::False;
                };
                let atom = self.filter(pred, Scope::Atom);
                let leaf = Filter::rel(
                    Rel::Props,
                    quant,
                    Filter::and(vec![
                        Filter::attr(
                            Attr::Key,
                            CmpOp::Eq,
                            Value::text(crate::doc::property_key_norm(&key)),
                        ),
                        atom,
                    ]),
                );
                return self.hop(name == "page_prop" || scope == Scope::Page, leaf);
            }
        }
        let Expr::Identifier(ident) = over else {
            return self.reject(
                DiagnosticKind::Syntax,
                "a quantifier ranges over a relation or a property",
            );
        };
        match (ident.value.to_ascii_lowercase().as_str(), scope) {
            ("children", Scope::Block) => {
                let pred = self.filter(pred, Scope::Block);
                Filter::rel(Rel::Children, quant, pred)
            }
            ("blocks", Scope::Page) => {
                let pred = self.filter(pred, Scope::Block);
                Filter::rel(Rel::Blocks, quant, pred)
            }
            (name, _) => self.reject_ident(name, format!("`{name}` is not a relation of this row")),
        }
    }
}

// ---------------------------------------------------------------------------
// Vocabulary tables
// ---------------------------------------------------------------------------

/// The prose a retained capsule shows. Diagnostic PROSE may be regenerated;
/// the payload and its kind may not be lost (§4.3.2). The renderer shows the
/// decoded original text, never the hexadecimal.
fn retained_message(kind: DiagnosticKind, text: &str) -> String {
    let what = match kind {
        DiagnosticKind::UnknownHead => "is not a query filter",
        DiagnosticKind::UnknownIdent => "is not part of the query language",
        DiagnosticKind::NotApplicable => "does not apply to this row",
        DiagnosticKind::Depth => "is nested too deeply",
        DiagnosticKind::Size => "is too large",
        DiagnosticKind::Syntax => "does not parse",
    };
    format!("`{text}` {what}")
}

fn block_attr(name: &str) -> Option<(Attr, ValueType)> {
    Some(match name {
        "content" => (Attr::Content, ValueType::Text),
        "task" => (Attr::Task, ValueType::Text),
        "priority" => (Attr::Priority, ValueType::Text),
        "scheduled" => (Attr::Scheduled, ValueType::Date),
        "deadline" => (Attr::Deadline, ValueType::Date),
        _ => return None,
    })
}

fn page_attr(name: &str) -> Option<(Attr, ValueType)> {
    Some(match name {
        "name" => (Attr::Name, ValueType::Text),
        "journal" => (Attr::Journal, ValueType::Checkbox),
        "day" => (Attr::Day, ValueType::Date),
        "namespace" => (Attr::Namespace, ValueType::Text),
        _ => return None,
    })
}

fn attr_label(attr: Attr) -> &'static str {
    match attr {
        Attr::Content => "content",
        Attr::Task => "task",
        Attr::Priority => "priority",
        Attr::Scheduled => "scheduled",
        Attr::Deadline => "deadline",
        Attr::Name => "name",
        Attr::Journal => "journal",
        Attr::Day => "day",
        Attr::Namespace => "namespace",
        Attr::Key => "key",
        Attr::Value => "value",
        Attr::AtomCount => "atom count",
    }
}

fn op_label(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::NotEq => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Between => "between",
        CmpOp::In => "in",
        CmpOp::NotIn => "not in",
        CmpOp::Like => "like",
        CmpOp::StartsWith => "like",
        CmpOp::Match => "match",
        CmpOp::Regex => "regexp",
        CmpOp::IsSet => "is not null",
        CmpOp::IsNotSet => "is null",
        CmpOp::IsBlank => "= ''",
    }
}

fn binary_cmp(op: &BinaryOperator) -> Option<CmpOp> {
    Some(match op {
        BinaryOperator::Eq => CmpOp::Eq,
        BinaryOperator::NotEq => CmpOp::NotEq,
        BinaryOperator::Lt => CmpOp::Lt,
        BinaryOperator::LtEq => CmpOp::Le,
        BinaryOperator::Gt => CmpOp::Gt,
        BinaryOperator::GtEq => CmpOp::Ge,
        BinaryOperator::Match => CmpOp::Match,
        _ => return None,
    })
}

/// SPEC §4.2.3 operator × type matrix (K9). Anything absent here is a `Syntax`
/// diagnostic naming the operator and the type.
fn op_applies(target: &Target, op: CmpOp, ty: ValueType) -> bool {
    let on_property = matches!(target, Target::Prop { .. } | Target::Atom);
    match op {
        CmpOp::Eq | CmpOp::NotEq => true,
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge | CmpOp::Between => {
            matches!(ty, ValueType::Number | ValueType::Date)
        }
        CmpOp::In | CmpOp::NotIn => !matches!(ty, ValueType::Date | ValueType::Checkbox),
        CmpOp::Like | CmpOp::StartsWith => matches!(ty, ValueType::Text | ValueType::Ref),
        CmpOp::Match => matches!(
            target,
            Target::Attr {
                attr: Attr::Content,
                ..
            }
        ),
        // `regexp` is permitted only on `content` with a text pattern, and is
        // forbidden on every other row and type in the §4.2.3 matrix.
        CmpOp::Regex => {
            ty == ValueType::Text
                && matches!(
                    target,
                    Target::Attr {
                        attr: Attr::Content,
                        ..
                    }
                )
        }
        CmpOp::IsSet | CmpOp::IsNotSet => {
            on_property
                || matches!(
                    target,
                    Target::Attr {
                        attr: Attr::Task | Attr::Priority | Attr::Scheduled | Attr::Deadline,
                        ..
                    }
                )
        }
        CmpOp::IsBlank => on_property,
    }
}

/// The type a literal spells. It decides how the IR stores the VALUE; the atom
/// it is compared against is typed by the registry at evaluation (§6.3).
fn literal_type(expr: &Expr) -> ValueType {
    match expr {
        Expr::Nested(inner) => literal_type(inner),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => literal_type(expr),
        Expr::Identifier(ident) if ident.value.eq_ignore_ascii_case("today") => ValueType::Date,
        Expr::Value(value) => match &value.value {
            SqlValue::Number(text, _) => {
                if is_day_ordinal(text) {
                    ValueType::Date
                } else {
                    ValueType::Number
                }
            }
            SqlValue::Boolean(_) => ValueType::Checkbox,
            SqlValue::SingleQuotedString(text) | SqlValue::DoubleQuotedString(text) => {
                if is_date_literal(text) {
                    ValueType::Date
                } else {
                    ValueType::Text
                }
            }
            _ => ValueType::Text,
        },
        _ => ValueType::Text,
    }
}

fn is_day_ordinal(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// `'2026-09-04'` or a relative `'-7d'` / `'+2w'` / `'-1m'` / `'-1y'`.
fn is_date_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    if text.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return text
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    }
    if bytes.len() >= 3 && matches!(bytes[0], b'-' | b'+') {
        let unit = bytes[bytes.len() - 1];
        return b"dwmy".contains(&unit) && bytes[1..bytes.len() - 1].iter().all(u8::is_ascii_digit);
    }
    false
}

/// `like 'p%'` with no other wildcard is `StartsWith` (range-lowerable, §4.2.3);
/// `\%` / `\_` are literal characters and do not disqualify the pattern.
fn starts_with_prefix(pattern: &str) -> Option<String> {
    let mut prefix = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some(escaped @ ('%' | '_' | '\\')) => prefix.push(escaped),
                Some(other) => {
                    prefix.push('\\');
                    prefix.push(other);
                }
                None => prefix.push('\\'),
            },
            '_' => return None,
            '%' => {
                return chars.peek().is_none().then(|| prefix.clone());
            }
            other => prefix.push(other),
        }
    }
    None
}

fn function_name(function: &sqlparser::ast::Function) -> String {
    function
        .name
        .0
        .iter()
        .filter_map(|part| part.as_ident())
        .map(|ident| ident.value.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(".")
}

fn function_args(function: &sqlparser::ast::Function) -> Vec<&Expr> {
    let FunctionArguments::List(list) = &function.args else {
        return Vec::new();
    };
    list.args
        .iter()
        .filter_map(|arg| match arg {
            FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ir::Leaf;

    fn parse(text: &str) -> Query {
        parse_tql(text, crate::query::registry::Registry::none()).0
    }

    fn ok(text: &str) -> Filter {
        let query = parse(text);
        assert!(
            !query.is_invalid(),
            "{text} did not parse: {:?}",
            query.diagnostics
        );
        query.filter
    }

    fn rejected(text: &str) -> Vec<Diagnostic> {
        let query = parse(text);
        assert!(
            query.is_invalid(),
            "{text} was accepted as {:?}",
            query.filter
        );
        query.diagnostics
    }

    // -- §4.2.2 probe set ---------------------------------------------------

    #[test]
    fn probe_anchor_alone_is_every_row_of_the_anchor() {
        let query = parse("@block");
        assert_eq!(query.anchor, Anchor::Block);
        assert_eq!(query.filter, Filter::True);
        let query = parse("@page");
        assert_eq!(query.anchor, Anchor::Page);
        assert_eq!(query.filter, Filter::True);
    }

    #[test]
    fn probe_page_anchor_with_one_condition() {
        let query = parse("@page and #x");
        assert_eq!(query.anchor, Anchor::Page);
        assert_eq!(query.filter, Filter::page_ref("x"));
    }

    #[test]
    fn probe_two_refs_conjoined() {
        assert_eq!(
            ok("#x and #y"),
            Filter::and(vec![Filter::page_ref("x"), Filter::page_ref("y")])
        );
    }

    #[test]
    fn probe_bracket_ref_keeps_its_spaces() {
        assert_eq!(ok("[[a b]]"), Filter::page_ref("a b"));
    }

    #[test]
    fn probe_bracket_inside_a_literal_is_not_sugar() {
        assert_eq!(
            ok("content like '[[not sugar]]'"),
            Filter::attr(Attr::Content, CmpOp::Like, Value::text("[[not sugar]]"))
        );
    }

    #[test]
    fn probe_ref_in_value_position_compares_by_page_name() {
        assert_eq!(
            ok("prop('k') = [[x]]"),
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                    Filter::attr(Attr::Value, CmpOp::Eq, Value::text("x")),
                ])
            )
        );
    }

    #[test]
    fn probe_ref_list_in_value_position_compares_by_page_name() {
        assert_eq!(
            ok("prop('k') in (#a, #b)"),
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                    Filter::attr(
                        Attr::Value,
                        CmpOp::In,
                        Value::List {
                            items: vec![Value::text("a"), Value::text("b")]
                        }
                    ),
                ])
            )
        );
    }

    #[test]
    fn probe_between_on_a_date_attribute() {
        assert_eq!(
            ok("scheduled between today and '+7d'"),
            Filter::attr(
                Attr::Scheduled,
                CmpOp::Between,
                Value::List {
                    items: vec![Value::date("today"), Value::date("+7d")]
                }
            )
        );
    }

    #[test]
    fn probe_unquoted_relative_date_is_rejected_with_the_quoting_suggestion() {
        let diagnostics = rejected("scheduled between today and -7d");
        assert!(diagnostics.iter().any(|d| d
            .suggestions
            .contains(&"quote relative dates: '-7d'".to_string())));
    }

    #[test]
    fn probe_a_disabled_line_becomes_an_off_operand() {
        assert_eq!(ok("-- #x"), Filter::off(Filter::page_ref("x")));
    }

    #[test]
    fn probe_two_runs_separated_by_a_bare_dash_line_stay_two_nodes() {
        let filter = ok("-- #x\n--\n-- and #y");
        assert_eq!(
            filter,
            Filter::and(vec![
                Filter::off(Filter::page_ref("x")),
                Filter::off(Filter::page_ref("y")),
            ])
        );
    }

    #[test]
    fn probe_a_run_inside_a_group_is_an_operand_of_that_group() {
        let filter = ok("#a and (\n#b\n-- or #c\n)");
        assert_eq!(
            filter,
            Filter::and(vec![
                Filter::page_ref("a"),
                Filter::or(vec![
                    Filter::page_ref("b"),
                    Filter::off(Filter::page_ref("c")),
                ]),
            ])
        );
    }

    #[test]
    fn probe_a_dash_line_inside_a_multiline_literal_is_not_a_run() {
        let filter = ok("content like 'a\n-- b\nc'");
        assert_eq!(
            filter,
            Filter::attr(Attr::Content, CmpOp::Like, Value::text("a\n-- b\nc"))
        );
    }

    #[test]
    fn probe_off_written_by_hand() {
        assert_eq!(ok("off(#x)"), Filter::off(Filter::page_ref("x")));
    }

    #[test]
    fn probe_property_presence_and_blankness() {
        assert_eq!(
            ok("prop('k') is null"),
            Filter::rel(
                Rel::Props,
                Quant::None,
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k"))
            )
        );
        assert_eq!(
            ok("prop('k') is not null"),
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k"))
            )
        );
        assert_eq!(
            ok("prop('k') = ''"),
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                    Filter::attr(Attr::AtomCount, CmpOp::Eq, Value::Number { number: 0.0 }),
                ])
            )
        );
    }

    #[test]
    fn probe_every_over_property_atoms() {
        assert_eq!(
            ok("every(prop('k'), value > 3)"),
            Filter::rel(
                Rel::Props,
                Quant::Every,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("k")),
                    Filter::attr(Attr::Value, CmpOp::Gt, Value::Number { number: 3.0 }),
                ])
            )
        );
    }

    #[test]
    fn probe_content_like_is_a_like_leaf() {
        assert_eq!(
            ok("content like '%foo%'"),
            Filter::attr(Attr::Content, CmpOp::Like, Value::text("%foo%"))
        );
    }

    #[test]
    fn probe_trailing_percent_is_starts_with() {
        assert_eq!(
            ok("page.name like 'proj/%'"),
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text("proj/"))
            )
        );
    }

    #[test]
    fn probe_trailing_statement_is_rejected() {
        rejected("content = 'x' DROP TABLE blocks");
    }

    #[test]
    fn probe_subquery_is_rejected() {
        rejected("prop('k') in (select 1)");
    }

    // -- vocabulary ---------------------------------------------------------

    /// A registry holding exactly these property keys and nothing else, so a
    /// suggestion test asserts on the keys it named rather than on a fixture
    /// graph's incidental vocabulary.
    fn registry_with(keys: &[&str]) -> crate::query::registry::Registry {
        use crate::query::atom::AtomFormat;
        use crate::query::registry::{build_registry, OwnerRow, OwnerType, PageMeta};
        let config = crate::config::ParseConfig::default();
        let rows = keys.iter().enumerate().map(|(index, key)| OwnerRow {
            owner_type: OwnerType::Block,
            owner_id: format!("block-{index}"),
            page_id: "page".to_string(),
            source_name: (*key).to_string(),
            normalized_name: crate::doc::property_key_norm(key),
            ordinal: 0,
            value: "v".to_string(),
        });
        build_registry(
            rows,
            &|_| {
                Some(PageMeta {
                    format: AtomFormat::Markdown,
                    name: "page".to_string(),
                })
            },
            &config,
        )
        .expect("every row names the one page")
    }

    #[test]
    fn an_unknown_identifier_is_named_and_never_rewritten() {
        let diagnostics = rejected("frobnicate = 'x'");
        assert!(diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnknownIdent));
        // Against no registry there is nothing to suggest, and a guess is not
        // a suggestion: the list is empty rather than invented.
        assert!(diagnostics.iter().all(|d| d.suggestions.is_empty()));
        // The filter is refused either way -- a suggestion never rewrites.
        assert_eq!(parse("frobnicate = 'x'").filter, Filter::False);
    }

    #[test]
    fn an_unknown_identifier_suggests_the_registrys_nearest_keys() {
        let registry = registry_with(&["status", "statuses", "author", "unrelated"]);
        let (query, _) = parse_tql("statuss = 'done'", &registry);
        let diagnostic = query
            .diagnostics
            .iter()
            .find(|d| d.kind == DiagnosticKind::UnknownIdent)
            .expect("an unknown identifier is named");
        // Best first: `statuses` shares one more leading character with
        // `statuss` than `status` does, so Jaro-Winkler ranks it above.
        assert_eq!(
            diagnostic.suggestions,
            vec!["prop('statuses')".to_string(), "prop('status')".to_string()],
        );
        // Suggesting is not rewriting (I-22): the query is still refused.
        assert_eq!(query.filter, Filter::False);
    }

    #[test]
    fn a_registry_key_that_is_nothing_like_the_identifier_is_not_suggested() {
        let registry = registry_with(&["author", "unrelated"]);
        let (query, _) = parse_tql("statuss = 'done'", &registry);
        assert!(query.diagnostics.iter().all(|d| d.suggestions.is_empty()));
    }

    #[test]
    fn an_unknown_function_and_an_unknown_relation_suggest_keys_too() {
        let registry = registry_with(&["status"]);
        let (function, _) = parse_tql("statuss('done')", &registry);
        assert_eq!(
            function
                .diagnostics
                .iter()
                .find(|d| d.kind == DiagnosticKind::UnknownIdent)
                .map(|d| d.suggestions.clone()),
            Some(vec!["prop('status')".to_string()]),
        );
        let (relation, _) = parse_tql("any(statuss, content = 'x')", &registry);
        assert_eq!(
            relation
                .diagnostics
                .iter()
                .find(|d| d.kind == DiagnosticKind::UnknownIdent)
                .map(|d| d.suggestions.clone()),
            Some(vec!["prop('status')".to_string()]),
        );
    }

    #[test]
    fn an_unknown_function_is_not_a_condition() {
        assert!(rejected("frobnicate('x')")
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnknownIdent));
    }

    #[test]
    fn an_anchor_in_the_middle_says_where_it_belongs() {
        let diagnostics = rejected("#x and @page");
        assert!(diagnostics
            .iter()
            .any(|d| d.message == "the anchor goes first"));
    }

    #[test]
    fn ordering_operators_do_not_apply_to_text() {
        assert!(rejected("priority < 'B'")
            .iter()
            .any(|d| d.message.contains("does not apply")));
    }

    #[test]
    fn match_applies_only_to_content() {
        assert_eq!(
            ok("content match 'foo'"),
            Filter::attr(Attr::Content, CmpOp::Match, Value::text("foo"))
        );
        assert!(rejected("task match 'foo'")
            .iter()
            .any(|d| d.message.contains("does not apply")));
    }

    #[test]
    fn planning_presence_is_a_presence_leaf() {
        assert_eq!(
            ok("scheduled is not null"),
            Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None)
        );
    }

    #[test]
    fn children_quantifiers_carry_the_element_scope() {
        assert_eq!(
            ok("any(children, task = 'TODO')"),
            Filter::rel(
                Rel::Children,
                Quant::Any,
                Filter::attr(Attr::Task, CmpOp::Eq, Value::text("TODO"))
            )
        );
        assert_eq!(
            ok("@page and any(blocks, task = 'TODO')"),
            Filter::rel(
                Rel::Blocks,
                Quant::Any,
                Filter::attr(Attr::Task, CmpOp::Eq, Value::text("TODO"))
            )
        );
    }

    #[test]
    fn page_attributes_need_no_hop_at_the_page_anchor() {
        assert_eq!(
            ok("@page and journal = true"),
            Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true })
        );
    }

    #[test]
    fn a_tag_leaf_is_the_blocks_own_inline_tags() {
        assert_eq!(
            ok("tag('x')"),
            Filter::rel(
                Rel::Tags,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::Eq, Value::text("x"))
            )
        );
    }

    #[test]
    fn a_page_tag_leaf_reads_the_pages_tags_property() {
        let filter = ok("page_tag('work')");
        let Filter::Leaf {
            leaf: Leaf::Rel { rel: Rel::Page, .. },
        } = &filter
        else {
            panic!("expected a page hop, got {filter:?}");
        };
    }

    #[test]
    fn the_recursion_limit_refuses_a_pathological_nesting() {
        let deep = format!("{}#x{}", "(".repeat(200), ")".repeat(200));
        rejected(&deep);
    }

    // -- pre-pass units -----------------------------------------------------

    #[test]
    fn literal_spans_honour_doubled_quotes() {
        let text = "a 'b''c' d";
        assert_eq!(literal_spans(text), vec![(2, 8)]);
    }

    #[test]
    fn a_malformed_disabled_span_is_a_disabled_diagnostic_and_does_not_invalidate() {
        let query = parse("#a\n-- and )(");
        assert!(
            !query.is_invalid(),
            "a disabled diagnostic must not invalidate: {:?}",
            query.diagnostics
        );
        assert!(query.diagnostics.iter().any(|d| d.disabled));
    }

    #[test]
    fn starts_with_recognises_only_a_single_trailing_wildcard() {
        assert_eq!(starts_with_prefix("proj/%"), Some("proj/".to_string()));
        assert_eq!(starts_with_prefix("%proj%"), None);
        assert_eq!(starts_with_prefix("pro_j%"), None);
        assert_eq!(starts_with_prefix("proj"), None);
        assert_eq!(starts_with_prefix("50\\%%"), Some("50%".to_string()));
    }
}
