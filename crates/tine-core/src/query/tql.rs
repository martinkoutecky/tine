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
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Spanned as _,
    UnaryOperator, Value as SqlValue, Visit, Visitor,
};
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

mod boolean;
mod prepass;
use prepass::*;

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
                    diagnostics: &mut diagnostics,
                    registry,
                    disabled_depth: 0,
                    original: text,
                    source: &pre.sql,
                    source_offset: pre.offset,
                    origins: &pre.origins,
                    not_applicable: None,
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
// 4.2.2 Guards
// ---------------------------------------------------------------------------

fn parse_expr_guarded(sql: &str) -> Result<Expr, String> {
    let dialect = boolean::BooleanDialect;
    let tokens = Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .map_err(|error| error.to_string())?;
    boolean::admit_tokens(&tokens)?;
    let mut parser = Parser::new(&dialect)
        .with_recursion_limit(RECURSION_LIMIT)
        .with_tokens_with_locations(tokens);
    let expr = parser.parse_expr().map_err(|error| error.to_string())?;
    boolean::admit_ast(&expr)?;
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
    /// The text the author typed, for `Span::from_byte_range`.
    original: &'a str,
    /// The text the PARSER saw — `pre.sql`, after `lift_disabled_runs` and
    /// `desugar`. sqlparser's spans index into this, so a retained payload is
    /// sliced from here (§7.4).
    source: &'a str,
    /// `original` byte offset of `source[0]`, or `None` when the pre-pass
    /// rewrote something and the two no longer line up.
    source_offset: Option<usize>,
    origins: &'a [usize],
    /// **The one deferred rejection (§7.4).** A name that resolves on the OTHER
    /// row is not unknown — it does not APPLY here — and §7.4 requires the
    /// author's leaf to stay in the tree instead of collapsing to `False`. The
    /// resolver cannot build that leaf: it only sees the identifier, not the
    /// comparison or quantifier that encloses it. So it parks the message here
    /// and [`Lower::leaf`], which does hold the whole expression, turns it into
    /// the retained `Raw` capsule. Always consumed by the enclosing `leaf`
    /// call, so it never leaks across siblings.
    not_applicable: Option<(String, Vec<String>)>,
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
        if let Some((and, items)) = boolean::items(expr) {
            let items = items.iter().map(|item| self.filter(item, scope)).collect();
            return if and {
                Filter::and(items)
            } else {
                Filter::or(items)
            };
        }
        match expr {
            Expr::Nested(inner) => self.filter(inner, scope),
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr,
            } => Filter::not(self.filter(expr, scope)),
            // Everything else is ONE condition of the author's, which is the
            // unit §7.4 retains when it does not apply to the anchor.
            leaf => self.leaf(leaf, scope),
        }
    }

    /// One condition, plus the §7.4 wrong-anchor retention.
    ///
    /// The IDENTIFIER is what resolves wrongly, but the LEAF is what the author
    /// wrote and what has to survive the anchor switch, so the check lands
    /// here: lower the condition, and if the resolver parked a "does not apply"
    /// on the way through, discard the `False` it produced and keep the exact
    /// source text of this condition as a `Raw` capsule instead.
    fn leaf(&mut self, expr: &Expr, scope: Scope) -> Filter {
        let diagnostic_start = self.diagnostics.len();
        let filter = self.condition(expr, scope);
        let Some((message, suggestions)) = self.not_applicable.take() else {
            if self.disabled_depth > 0 && !matches!(filter, Filter::Raw { .. }) {
                if let Some(diagnostic) = self.diagnostics.get(diagnostic_start) {
                    let kind = diagnostic.kind;
                    let (text, _) = self.retained_slice(expr);
                    return Filter::raw(text, kind);
                }
            }
            return filter;
        };
        let (text, span) = self.retained_slice(expr);
        let mut diagnostic =
            Diagnostic::new(DiagnosticKind::NotApplicable, message).with_span(span);
        diagnostic.suggestions = suggestions;
        self.diagnose(diagnostic);
        Filter::Raw {
            text,
            kind: DiagnosticKind::NotApplicable,
            span,
        }
    }

    /// The exact source text of `expr`, and its span in the ORIGINAL when the
    /// two texts line up.
    ///
    /// The bytes the author typed are preferred; `Display` of the rebuilt AST
    /// is the fallback for an expression whose span sqlparser leaves empty. It
    /// is lossless in meaning (that is what the canonical printer is), it is
    /// just not the author's spelling — so it is unspanned, because there is
    /// nothing honest to point at.
    fn retained_slice(&self, expr: &Expr) -> (String, Option<Span>) {
        // sqlparser 0.62's `Spanned for Function` unions the name and the
        // ARGUMENT spans and never the parentheses, so `any(children, true)`
        // reports through `true` and stops. Measured, not assumed — the
        // `any`/`blocks` cases below pin it. Closing what the span left open is
        // a lexical repair of the span, not a second parser.

        let span = expr.span();
        let range = byte_offset(self.source, span.start)
            .zip(byte_offset(self.source, span.end))
            .filter(|(start, end)| start < end);
        let Some((start, end)) = range else {
            return (expr.to_string(), None);
        };
        let end = balanced_end(self.source, start, end);
        let authored = self.origins.get(start).zip(self.origins.get(end));
        let text = authored
            .and_then(|(from, to)| self.original.get(*from..*to))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| self.source.get(start..end).expect("SQL span boundaries"))
            .to_string();
        let span = self
            .source_offset
            .map(|offset| Span::from_byte_range(self.original, offset + start, offset + end));
        (text, span)
    }

    fn condition(&mut self, expr: &Expr, scope: Scope) -> Filter {
        match expr {
            Expr::BinaryOp {
                left,
                op: binary,
                right,
            } => match binary {
                BinaryOperator::Regexp => self.regexp(left, right, scope),
                _ => match binary_cmp(binary) {
                    Some(op) => self.compare(left, op, right, scope),
                    None => self.reject(
                        DiagnosticKind::Syntax,
                        format!("`{binary}` is not a comparison the query language has"),
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

    /// The pattern is always text, but whether `like` applies is decided by the
    /// TARGET's type, as `presence` does: §4.2.3 refuses it on a date or
    /// checkbox attribute.
    fn like(&mut self, left: &Expr, pattern: &Expr, scope: Scope) -> Filter {
        let Some(target) = self.target(left, scope) else {
            return Filter::False;
        };
        let ty = match &target {
            Target::Attr { ty, .. } => *ty,
            _ => ValueType::Text,
        };
        let Some(Value::Text { text }) = self.value(pattern, ValueType::Text) else {
            return self.reject(
                DiagnosticKind::Syntax,
                "a `like` pattern is a quoted string",
            );
        };
        match crate::query::text::LikePattern::compile(&text).starts_with_prefix() {
            Some(prefix) => self.build(target, CmpOp::StartsWith, Value::text(prefix), ty),
            None => self.build(target, CmpOp::Like, Value::text(text), ty),
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
                    CmpOp::Eq
                    | CmpOp::NotEq
                    | CmpOp::Lt
                    | CmpOp::Le
                    | CmpOp::Gt
                    | CmpOp::Ge
                    | CmpOp::Between
                    | CmpOp::In
                    | CmpOp::NotIn
                    | CmpOp::Like
                    | CmpOp::StartsWith
                    | CmpOp::Match
                    | CmpOp::Regex => Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![key_test, Filter::attr(Attr::Value, op, value)]),
                    ),
                };
                self.hop(through_page, leaf)
            }
        }
    }

    /// `through_page` means an actual hop from the CURRENT block row. It is
    /// resolved while the target is read, rather than from the query's outer
    /// anchor: inside `@page and any(blocks, ...)` the current row is a block.
    fn hop(&mut self, through_page: bool, filter: Filter) -> Filter {
        if through_page {
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
                        through_page: false,
                        attr,
                        ty,
                    }),
                };
                if resolved.is_none() && !self.wrong_row_ident(&name, scope) {
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
                        through_page: false,
                        key: crate::doc::property_key_norm(&key),
                    }),
                    ("page_prop", 1) => self.string_arg(args[0]).map(|key| Target::Prop {
                        through_page: scope == Scope::Block,
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

    /// **The anchor mismatch, §3.5's own diagnostic source (§7.4).**
    ///
    /// `task` at `@page` is not an unknown name: it is a perfectly good block
    /// field asked of a page row. Saying "is not a field of this query" sends
    /// the author looking for a typo, and collapsing the leaf to `False`
    /// throws away the condition they wrote — which is exactly what an anchor
    /// switch must not do, because switching back has to bring it home.
    ///
    /// Returns whether the name belongs to the OTHER row; when it does, the
    /// message is parked for [`Lower::leaf`] to attach to the retained capsule.
    fn wrong_row_ident(&mut self, name: &str, scope: Scope) -> bool {
        let suggestions = match scope {
            // A page field asked of a block row has an honest spelling that
            // works: the explicit hop.
            Scope::Block if page_attr(name).is_some() => vec![format!("page.{name}")],
            Scope::Page if block_attr(name).is_some() => Vec::new(),
            _ => return false,
        };
        self.park_not_applicable(name, scope, suggestions);
        true
    }

    fn park_not_applicable(&mut self, name: &str, scope: Scope, suggestions: Vec<String>) {
        let row = match scope {
            Scope::Page => "pages",
            _ => "blocks",
        };
        self.not_applicable = Some((format!("`{name}` does not apply to {row}"), suggestions));
    }

    /// SPEC §4.2.2 guard 2. Suggestions are the registry's nearest keys —
    /// property keys the graph actually has, written as the `prop('…')` call
    /// that would have worked. Nothing is rewritten silently.
    fn unknown_ident(&mut self, name: &str) {
        let diagnostic = Diagnostic::new(
            DiagnosticKind::UnknownIdent,
            format!("`{name}` is not a field of this query"),
        );
        self.diagnose(self.suggested(diagnostic, name));
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
        self.diagnose(self.suggested(diagnostic, name));
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

    /// A date literal; an out-of-range relative offset is a diagnostic, never
    /// a garbage day (the [`DateToken`](crate::query::DateToken) grammar).
    fn date(&mut self, text: &str) -> Option<Value> {
        if crate::query::DateToken::parse(text) == Some(crate::query::DateToken::OutOfRange) {
            self.reject(
                DiagnosticKind::Syntax,
                format!(
                    "date offset `{text}` is out of range (at most {} years)",
                    crate::query::MAX_DATE_OFFSET_YEARS
                ),
            );
            return None;
        }
        Some(Value::date(text))
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
                        self.date(text)?
                    } else {
                        Value::text(text)
                    })
                }
                SqlValue::Number(text, _) => {
                    if ty == ValueType::Date {
                        return self.date(text);
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
                    self.hop(scope == Scope::Block, leaf)
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
                return self.hop(name == "page_prop" && scope == Scope::Block, leaf);
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
            // The relation exists — on the other row (§7.4).
            (name @ "children", Scope::Page) | (name @ "blocks", Scope::Block) => {
                self.park_not_applicable(name, scope, Vec::new());
                Filter::False
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

/// A sqlparser `Location` (1-based line, 1-based CHARACTER column) as a byte
/// offset into the text it was measured on. `None` for the empty location
/// sqlparser uses when a node has no span.
/// Extend `end` forward over whatever closes the parentheses `text[start..end]`
/// left open, so a partial function span still yields the author's whole call.
/// String literals (with SQL's `''` doubling) are skipped, never counted.
fn balanced_end(text: &str, start: usize, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut literal = false;
    let mut i = start;
    let mut close_at = None;
    while i < bytes.len() {
        if i >= end && depth <= 0 {
            break;
        }
        match bytes[i] {
            b'\'' if literal => {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                literal = false;
            }
            b'\'' => literal = true,
            b'(' if !literal => depth += 1,
            b')' if !literal => {
                depth -= 1;
                if depth == 0 && i >= end {
                    close_at = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    close_at.unwrap_or(end)
}

fn byte_offset(text: &str, location: sqlparser::tokenizer::Location) -> Option<usize> {
    if location.line == 0 || location.column == 0 {
        return None;
    }
    let mut line = 1u64;
    let mut column = 1u64;
    for (index, ch) in text.char_indices() {
        if line == location.line && column == location.column {
            return Some(index);
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line == location.line && column == location.column).then_some(text.len())
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

/// The comparison a SQL binary operator names. `sqlparser` has dozens of
/// operators the query language does not, so this match alone keeps a rest arm.
fn binary_cmp(binary: &BinaryOperator) -> Option<CmpOp> {
    Some(match binary {
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

/// `'2026-09-04'` or a SIGNED relative `'-7d'` / `'+2w'` / `'-1m'` / `'-1y'`:
/// the subset of the [`DateToken`](crate::query::DateToken) grammar that types
/// a quoted TQL literal as a date (Rule 3: one grammar, no local recognizer).
/// An out-of-range offset still types as a date, so it is diagnosed.
fn is_date_literal(text: &str) -> bool {
    use crate::query::DateToken;
    if text != text.trim() {
        return false;
    }
    match DateToken::parse(text) {
        Some(DateToken::Relative { .. } | DateToken::OutOfRange) => {
            matches!(text.as_bytes().first(), Some(b'+' | b'-'))
        }
        Some(DateToken::Stem(_)) => text.len() == 10 && text.as_bytes()[4] == b'-',
        _ => false,
    }
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
#[path = "tql_tests.rs"]
mod tests;
