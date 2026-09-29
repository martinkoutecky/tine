//! IR → text (SPEC §4.3).
//!
//! Two printers, one entry: [`query_print`]. TQL is total — every IR the
//! parsers can build has a TQL spelling. The OG DSL is **partial** and says so:
//! it is defined only where [`og_expressible`] holds, and returns a
//! `NotApplicable` diagnostic everywhere else rather than emitting a query that
//! means something different (I-12: one canonical answer).
//!
//! The OG serialization was transcribed (D-9) from the frontend's own `toDsl`
//! in `src/editor/queryBuilder.ts` — its `quoteStr`/`needsQuote` escaping, its
//! `dateBound` bare-vs-`[[…]]` rule, and its single-child `and` simplification
//! (which matches OG `simplify-query`). **That printer no longer exists**: it
//! was deleted (last present at commit `52cb16fe`) when the builder moved to the
//! IR, precisely so that this module is the only place a query is printed
//! (I-12). The rules it transcribed are now defined HERE, and pinned against the
//! reader by [`super::og`] round-tripping every og-expressible query
//! (`tests::og_expressible_queries_round_trip_through_the_og_printer`,
//! `tests::a_quoted_value_survives_the_og_escaping`) rather than by agreement
//! with a second implementation.
//!
//! **Deviation from §4.3, recorded (D-14 would otherwise apply):** the TQL
//! printer emits text directly instead of `Display`-ing a rebuilt `sqlparser`
//! AST. `sqlparser`'s `Display` cannot produce either of the two things the
//! canonical form is defined by — `[[x]]` restored for a `refs` leaf, and the
//! K10 line layout with `-- ` prefixes — so a rebuilt AST would be
//! post-processed into unrecognisability. Semantic round-trip is pinned over
//! every parser shape, and editable TQL additionally preserves n-ary sibling
//! lists and explicit same-operator parentheses exactly: those boundaries are
//! controls in the query sheet even when boolean evaluation could flatten them.

use crate::query::ir::{
    Anchor, Attr, CmpOp, Diagnostic, DiagnosticKind, Filter, Leaf, Quant, Query, Rel, SortDir,
    Source, Value, ViewSettings,
};
use crate::query::macro_text::{self, FormFamily};

/// The four printed forms of a query (§4.3, §7.1).
///
/// Three of them are MACRO dialects — they produce the bytes that go inside a
/// `{{…}}` in a document — and every one of those validates its final argument
/// before returning (§4.3.1). `Tql` is the text PANE's rendering: the editing
/// form, multi-line, never options, never checked for macro safety because it
/// is never written to a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintDialect {
    /// The OG DSL, for `{{query …}}`. Partial: defined only where
    /// [`og_expressible`] holds.
    Og,
    /// The text pane's layout (K10): one root operand per line,
    /// connector-leading, disabled operands as `-- ` lines.
    Tql,
    /// The persisted TQL macro form (X1): one line, always anchored,
    /// every `Off` inline as `off(…)`, options appended once.
    TqlMacro,
    /// A `{{query [:find …]}}` advanced macro, printed from its authored source
    /// rather than regenerated from the IR.
    AdvancedMacro,
}

impl PrintDialect {
    /// The macro name this dialect writes, or `None` for the pane.
    fn macro_name(self) -> Option<&'static str> {
        match self {
            PrintDialect::Og | PrintDialect::AdvancedMacro => Some("query"),
            PrintDialect::TqlMacro => Some("tine-query"),
            PrintDialect::Tql => None,
        }
    }
}

/// Print a query in the requested dialect (§4.3, §7.1).
///
/// `view` is explicit because two view directives, `sort-by` (single key) and
/// `sample`, live outside the filter and are re-emitted into the OG text (M15).
/// `aggregate` and `group-by` are NEVER printed: they persist only as
/// `tine.col-aggregates::` / `tine.group-by::` block properties, which the
/// caller must write; columns and the view kind likewise. A multi-key sort makes
/// the OG dialect refuse (`og_expressible` is false). The macro dialects append
/// the options map verbatim from `query.source.og_options()`, never derived from
/// `view`; `TqlMacro` ignores `view` entirely.
///
/// `preserve_form` is the **source-preserving** path: a title-only edit must not
/// convert the author's filter. It re-emits `source.original` plus the changed
/// options map once, without re-lowering the IR and without consulting
/// `og_expressible`, so an unsupported authored query can still have its title
/// edited. It requires a source-backed query whose macro dialect matches the
/// source variant; a `Builder` query is refused, because there is no authored
/// form to preserve. `advanced_macro` is source-preserving whether or not the
/// flag is set — Q13 keeps advanced filters read-only.
///
/// Every macro dialect validates its final argument before returning: the
/// lexical rule ([`macro_text::macro_safe`]) and then the real document parser
/// ([`macro_text::recognizable_macro`]). A refusal is a located diagnostic and
/// nothing is written (I-4).
pub fn query_print(
    query: &Query,
    view: &ViewSettings,
    dialect: PrintDialect,
    preserve_form: bool,
) -> Result<String, Diagnostic> {
    if dialect == PrintDialect::Tql {
        if preserve_form {
            return Err(Diagnostic::new(
                DiagnosticKind::NotApplicable,
                "the text pane has no authored form to preserve",
            ));
        }
        // The pane is not a document: no options, no view directives, no
        // macro-safety check.
        return Ok(print_tql(query));
    }
    let argument = if preserve_form || dialect == PrintDialect::AdvancedMacro {
        preserved_form(query, dialect)?
    } else {
        match dialect {
            PrintDialect::Og => {
                let options = query.source.og_options();
                with_options(print_og(query, view, !options.is_empty())?, options)
            }
            PrintDialect::TqlMacro => {
                with_options(print_tql_macro(query), query.source.og_options())
            }
            PrintDialect::Tql | PrintDialect::AdvancedMacro => unreachable!("handled above"),
        }
    };
    if let Some(name) = dialect.macro_name() {
        macro_text::macro_safe(&argument, FormFamily::for_macro_name(name))?;
        macro_text::recognizable_macro(name, &argument)?;
    }
    Ok(argument)
}

/// The source-preserving argument: `source.original` plus the changed options
/// map, once. It deliberately does not consult `og_expressible` and does not
/// re-lower the IR — title editing is not a filter conversion (§4.3.1).
fn preserved_form(query: &Query, dialect: PrintDialect) -> Result<String, Diagnostic> {
    let matches = matches!(
        (&query.source, dialect),
        (Source::Og { .. }, PrintDialect::Og)
            | (Source::Tql { .. }, PrintDialect::TqlMacro)
            | (Source::Advanced { .. }, PrintDialect::AdvancedMacro)
    );
    if !matches {
        return Err(Diagnostic::new(
            DiagnosticKind::NotApplicable,
            "preserving the authored form needs a query read from that macro dialect",
        ));
    }
    let original = query.source.original().expect("matched a source variant");
    Ok(with_options(
        original.to_string(),
        query.source.og_options(),
    ))
}

/// Append the opaque options map ONCE, verbatim (§4.3, Y2). It is the author's
/// text, not something a printer re-derives (I-4).
fn with_options(form: String, options: &str) -> String {
    if options.is_empty() {
        return form;
    }
    if form.is_empty() {
        return options.to_string();
    }
    format!("{form} {options}")
}

/// **The persisted TQL macro form (X1).** One line, because the document parser
/// does not carry a macro across a line break — measured, not assumed.
///
/// It ALWAYS starts with `@block` or `@page`. That is not decoration: a macro
/// argument beginning with a page reference takes the document parser's other
/// argument alternative, so `{{tine-query [[a]] and task = 'TODO'}}` is not a
/// macro at all while the anchored form is (§4.3.1, measured in both Markdown
/// and Org). ` and <filter>` follows unless the filter is exactly `True`, when
/// the anchor alone says the same thing. Every `Off` prints inline as `off(…)`;
/// the `-- ` layout is the pane's, not the document's.
fn print_tql_macro(query: &Query) -> String {
    let anchor = match query.anchor {
        Anchor::Block => "@block",
        Anchor::Page => "@page",
    };
    if tql_root_is_true(&query.filter) {
        return anchor.to_string();
    }
    format!("{anchor} and {}", tql_expr(&query.filter, Prec::Or))
}

// ---------------------------------------------------------------------------
// IR → TQL
// ---------------------------------------------------------------------------

/// Binding strength, so the printer parenthesizes exactly where SQL needs it.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Prec {
    Or,
    And,
    Not,
    Atom,
}

pub fn print_tql(query: &Query) -> String {
    // Print the editable tree itself. `Query::normalized()` is appropriate for
    // semantic comparison and OG's simplified form, but it flattens an explicit
    // same-kind child group that the sheet must recover after save.
    let anchor = match query.anchor {
        Anchor::Block => "",
        Anchor::Page => "@page",
    };
    if tql_root_is_true(&query.filter) {
        return if anchor.is_empty() {
            "@block".to_string()
        } else {
            anchor.to_string()
        };
    }
    let body = if root_operands(&query.filter).iter().any(contains_off) {
        print_tql_layered(&query.filter)
    } else {
        tql_expr(&query.filter, Prec::Or)
    };
    if anchor.is_empty() {
        body
    } else if body.starts_with("--") || body.contains('\n') {
        format!("{anchor}\nand {body}")
    } else {
        format!("{anchor} and {body}")
    }
}

/// The root's operands: an `And`/`Or` contributes its children, anything else
/// is itself one operand.
fn root_operands(filter: &Filter) -> Vec<&Filter> {
    match filter {
        Filter::And { items } | Filter::Or { items } => items.iter().collect(),
        other => vec![other],
    }
}

fn tql_root_is_true(filter: &Filter) -> bool {
    matches!(filter, Filter::True) || matches!(filter, Filter::And { items } if items.is_empty())
}

fn contains_off(filter: &&Filter) -> bool {
    matches!(filter, Filter::Off { .. })
}

/// K10: one root operand per line, connector-leading, a root `Off` operand's
/// lines prefixed `-- `, and a bare `--` between two consecutive root `Off`
/// siblings so the pre-pass reads them back as two nodes (N2).
fn print_tql_layered(filter: &Filter) -> String {
    let (connector, parent) = match filter {
        Filter::Or { .. } => ("or ", Prec::Or),
        _ => ("and ", Prec::And),
    };
    let operands = root_operands(filter);
    let mut lines: Vec<String> = Vec::new();
    let mut previous_was_off = false;
    for (index, operand) in operands.iter().enumerate() {
        let lead = if index == 0 { "" } else { connector };
        match operand {
            Filter::Off { inner } => {
                if previous_was_off {
                    lines.push("--".to_string());
                }
                lines.push(format!(
                    "-- {lead}{}",
                    tql_expr(off_content(inner), Prec::And)
                ));
                previous_was_off = true;
            }
            other => {
                lines.push(format!("{lead}{}", tql_group_item(other, parent)));
                previous_was_off = false;
            }
        }
    }
    lines.join("\n")
}

fn parens(text: String, needed: bool) -> String {
    if needed {
        format!("({text})")
    } else {
        text
    }
}

/// Keep a same-operator child visibly parenthesized. Precedence alone cannot
/// distinguish `a and (b and c)` from `a and b and c`, but the sheet can: the
/// former is a nested authored group and the latter is three siblings.
fn tql_group_item(filter: &Filter, parent: Prec) -> String {
    let text = tql_expr(filter, parent);
    let same_group = match parent {
        Prec::And => matches!(filter, Filter::And { .. }),
        Prec::Or => matches!(filter, Filter::Or { .. }),
        Prec::Not | Prec::Atom => false,
    };
    parens(text, same_group)
}

// Repeated disabling has one persisted representation. Keep this narrow
// canonicalization separate from boolean group structure, which is editable.
fn off_content(mut filter: &Filter) -> &Filter {
    while let Filter::Off { inner } = filter {
        filter = inner;
    }
    filter
}

fn tql_expr(filter: &Filter, context: Prec) -> String {
    match filter {
        Filter::True => "true".to_string(),
        Filter::False => "false".to_string(),
        // The lossless preservation capsule (§4.3.2). Hex is an INTERNAL form:
        // it is excluded from the vocabulary picker and the error renderer shows
        // the decoded original text, never this.
        Filter::Raw { text, kind, .. } => format!(
            "raw_hex({}, {})",
            sql_string(kind.capsule_name()),
            sql_string(&crate::query::ir::encode_raw_hex(text))
        ),
        // Below the root every `Off` prints inline as the function form, which
        // is legal TQL (§4.2.3) and is what the parser reads back.
        Filter::Off { inner } => format!("off({})", tql_expr(off_content(inner), Prec::Or)),
        Filter::Not { inner } => {
            // sqlparser 0.62 reads `not value between …` / `not value in (…)`
            // as a parse error, so a negated atom test keeps its parentheses.
            let operand = match inner.as_ref() {
                Filter::Leaf {
                    leaf:
                        Leaf::Attr {
                            attr: Attr::Value, ..
                        },
                } => format!("({})", tql_expr(inner, Prec::Or)),
                other => tql_expr(other, Prec::Not),
            };
            parens(format!("not {operand}"), context > Prec::Not)
        }
        Filter::And { items } => {
            if items.is_empty() {
                return "true".to_string();
            }
            let text = items
                .iter()
                .map(|item| tql_group_item(item, Prec::And))
                .collect::<Vec<_>>()
                .join(" and ");
            parens(text, context > Prec::And)
        }
        Filter::Or { items } => {
            if items.is_empty() {
                return "false".to_string();
            }
            let text = items
                .iter()
                .map(|item| tql_group_item(item, Prec::Or))
                .collect::<Vec<_>>()
                .join(" or ");
            parens(text, context > Prec::Or)
        }
        Filter::Leaf { leaf } => tql_leaf(leaf, false),
    }
}

/// `through_page` is set while printing the predicate of a `page` hop: the
/// spellings differ (`name` → `page.name`, `prop` → `page_prop`).
fn tql_leaf(leaf: &Leaf, through_page: bool) -> String {
    match leaf {
        Leaf::Attr { attr, op, value } => {
            let name = tql_attr_name(*attr, through_page);
            tql_comparison(&name, *op, value)
        }
        Leaf::Rel { rel, quant, pred } => tql_rel(*rel, *quant, pred, through_page),
    }
}

fn tql_rel(rel: Rel, quant: Quant, pred: &Filter, through_page: bool) -> String {
    match rel {
        Rel::Page => match pred {
            Filter::Leaf { leaf } => tql_leaf(leaf, true),
            other => tql_expr(other, Prec::Atom),
        },
        Rel::Refs => match single_name(pred) {
            Some(name) => format!("[[{name}]]"),
            None => format!("any(refs, {})", tql_expr(pred, Prec::Or)),
        },
        Rel::Tags => match single_name(pred) {
            Some(name) => format!("tag({})", sql_string(&name)),
            None => format!("any(tags, {})", tql_expr(pred, Prec::Or)),
        },
        Rel::Props => tql_props(quant, pred, through_page),
        Rel::Children | Rel::Blocks => format!(
            "{}({}, {})",
            quant_name(quant),
            rel.tql_name(),
            tql_expr(pred, Prec::Or)
        ),
    }
}

fn quant_name(quant: Quant) -> &'static str {
    match quant {
        Quant::Any => "any",
        Quant::Every => "every",
        Quant::None => "none",
    }
}

fn tql_props(quant: Quant, pred: &Filter, through_page: bool) -> String {
    let Some(key) = pred.props_key() else {
        return format!("{}(props, {})", quant_name(quant), tql_expr(pred, Prec::Or));
    };
    let call = if through_page { "page_prop" } else { "prop" };
    let spelled = format!("{call}({})", sql_string(&key));
    let atom = pred.props_atom_test();
    let shorthand = atom.as_ref().and_then(shorthand_comparison);
    match (quant, &atom, shorthand) {
        (Quant::Any, None, _) => format!("{spelled} is not null"),
        (Quant::None, None, _) => format!("{spelled} is null"),
        (quant, None, _) => format!("{}({spelled}, true)", quant_name(quant)),
        (Quant::Any, Some(atom), _) if blank_test(atom) => format!("{spelled} = ''"),
        (Quant::Any, Some(atom), Some((op, value))) => {
            // A `tags` property whose only test is an equality is the page's
            // tag: the shorter spelling reads back as the same leaf.
            if through_page && key == "tags" {
                if let Some(tag) = single_value_equality(atom) {
                    return format!("page_tag({})", sql_string(&tag));
                }
            }
            tql_comparison(&spelled, op, value)
        }
        // Every other atom test keeps its quantifier: `prop('k') > 1 and
        // prop('k') < 5` would read back as two independent `any` leaves, and
        // `not prop('k') = 'a'` as `not any(…)` (og C3 L02, I-4).
        (quant, Some(atom), _) => format!(
            "{}({spelled}, {})",
            quant_name(quant),
            tql_expr(atom, Prec::Or)
        ),
    }
}

/// The one atom-test shape the `prop('k') op v` shorthand reads back as: a
/// single `value` comparison whose `prop(...)` spelling is not claimed by a
/// property form (`= ''` is IsBlank, `is [not] null` is key presence).
fn shorthand_comparison(atom: &Filter) -> Option<(CmpOp, &Value)> {
    let Filter::Leaf {
        leaf:
            Leaf::Attr {
                attr: Attr::Value,
                op,
                value,
            },
    } = atom
    else {
        return None;
    };
    match op {
        CmpOp::IsSet | CmpOp::IsNotSet | CmpOp::IsBlank => None,
        CmpOp::Eq if *value == Value::text("") => None,
        _ => Some((*op, value)),
    }
}

fn blank_test(atom: &Filter) -> bool {
    matches!(
        atom,
        Filter::Leaf {
            leaf: Leaf::Attr {
                attr: Attr::AtomCount,
                op: CmpOp::Eq,
                value: Value::Number { number },
            },
        } if *number == 0.0
    )
}

fn single_value_equality(atom: &Filter) -> Option<String> {
    match atom {
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Value,
                    op: CmpOp::Eq,
                    value: Value::Text { text },
                },
        } => Some(text.clone()),
        _ => None,
    }
}

/// The `name = 'x'` predicate a ref or tag element leaf carries.
fn single_name(pred: &Filter) -> Option<String> {
    match pred {
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Name,
                    op: CmpOp::Eq,
                    value: Value::Text { text },
                },
        } => Some(text.clone()),
        _ => None,
    }
}

fn tql_attr_name(attr: Attr, through_page: bool) -> String {
    let bare = match attr {
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
        Attr::AtomCount => "atom_count",
    };
    let page_row = matches!(
        attr,
        Attr::Name | Attr::Journal | Attr::Day | Attr::Namespace
    );
    if through_page && page_row {
        format!("page.{bare}")
    } else {
        bare.to_string()
    }
}

fn tql_comparison(subject: &str, op: CmpOp, value: &Value) -> String {
    match op {
        CmpOp::IsSet => format!("{subject} is not null"),
        CmpOp::IsNotSet => format!("{subject} is null"),
        CmpOp::IsBlank => format!("{subject} = ''"),
        CmpOp::Between => match value {
            Value::List { items } if items.len() == 2 => format!(
                "{subject} between {} and {}",
                tql_value(&items[0]),
                tql_value(&items[1])
            ),
            other => format!("{subject} between {}", tql_value(other)),
        },
        CmpOp::In | CmpOp::NotIn => {
            let spelled = if op == CmpOp::In { "in" } else { "not in" };
            match value {
                Value::List { items } => format!(
                    "{subject} {spelled} ({})",
                    items.iter().map(tql_value).collect::<Vec<_>>().join(", ")
                ),
                other => format!("{subject} {spelled} ({})", tql_value(other)),
            }
        }
        CmpOp::StartsWith => match value {
            Value::Text { text } => format!(
                "{subject} like {}",
                sql_string(&format!("{}%", escape_like_literal(text)))
            ),
            other => format!("{subject} like {}", tql_value(other)),
        },
        CmpOp::Like => format!("{subject} like {}", tql_value(value)),
        CmpOp::Match => format!("{subject} match {}", tql_value(value)),
        // `content regexp '…'` (§4.2.3): the TQL spelling of the legacy
        // `(content-regex …)` head, which parses back to the same leaf.
        CmpOp::Regex => format!("{subject} regexp {}", tql_value(value)),
        CmpOp::Eq => format!("{subject} = {}", tql_value(value)),
        CmpOp::NotEq => format!("{subject} != {}", tql_value(value)),
        CmpOp::Lt => format!("{subject} < {}", tql_value(value)),
        CmpOp::Le => format!("{subject} <= {}", tql_value(value)),
        CmpOp::Gt => format!("{subject} > {}", tql_value(value)),
        CmpOp::Ge => format!("{subject} >= {}", tql_value(value)),
    }
}

fn tql_value(value: &Value) -> String {
    match value {
        Value::Text { text } => sql_string(text),
        Value::Number { number } => {
            if number.fract() == 0.0 && number.abs() < 1e15 {
                format!("{}", *number as i64)
            } else {
                format!("{number}")
            }
        }
        // `today` is a vocabulary identifier; every other relative or absolute
        // date is a quoted literal (§4.2.1).
        Value::Date { literal } if literal.eq_ignore_ascii_case("today") => "today".to_string(),
        Value::Date { literal } => sql_string(literal),
        Value::Bool { value } => value.to_string(),
        Value::List { items } => format!(
            "({})",
            items.iter().map(tql_value).collect::<Vec<_>>().join(", ")
        ),
        Value::None => "null".to_string(),
    }
}

fn sql_string(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// `%`/`_`/`\` inside a `StartsWith` prefix are data, so they are escaped before
/// the single trailing wildcard is appended.
fn escape_like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------
// IR → OG DSL
// ---------------------------------------------------------------------------

/// Whether the OG DSL can say this query — the precondition of the OG printer
/// and of the Q3 "save as `{{query}}`" policy.
pub fn og_expressible(query: &Query, view: &ViewSettings) -> bool {
    og_form(query).is_some() && og_view(view).is_some()
}

fn print_og(query: &Query, view: &ViewSettings, has_options: bool) -> Result<String, Diagnostic> {
    let not_applicable = |what: &str| {
        Err(Diagnostic::new(
            DiagnosticKind::NotApplicable,
            format!("the OG query syntax cannot express {what}"),
        ))
    };
    let Some(form) = og_form(query) else {
        return not_applicable("this filter");
    };
    let Some(directives) = og_view(view) else {
        return not_applicable("this view");
    };
    let mut out = form;
    // mldoc's macro grammar takes a leading-page-reference argument alternative,
    // so an argument that STARTS with `[[` and carries ANYTHING after it comes
    // back as plain text rather than a `Macro` node — a view directive is enough,
    // an options map is not required. Measured on mldoc 1.5.7 and lsdoc, in both
    // Markdown and Org. A single-child `and` is the same query (OG's own
    // `simplify-query` collapses it) and IS read back, so wrap rather than refuse:
    // `recognizable_macro` would otherwise reject a form the author may write.
    // Only when something follows — a bare `[[a]]` is already a macro, and
    // rewriting it would churn every such block on its next save. `#tag` takes a
    // different alternative and is unaffected.
    if out.starts_with("[[") && (has_options || !directives.is_empty()) {
        out = format!("(and {out})");
    }
    for directive in directives {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&directive);
    }
    // The options map is NOT appended here: `with_options` in `query_print` is
    // the one appender for every macro dialect (§4.3, Y2), so a map cannot be
    // emitted twice by two printers that each thought they owned it.
    Ok(out)
}

/// `toDsl`'s root rule: a single-child `and` simplifies to the child, matching
/// OG `simplify-query`; an empty root is the empty string.
fn og_form(query: &Query) -> Option<String> {
    // Print the editable tree itself. `Query::normalized()` is the execution
    // equivalence form and deliberately flattens authored same-kind groups.
    let filter = &query.filter;
    // `@page` is OG's `blocks?` rule reading false — the anchor is implied by
    // the heads, so a page-anchored filter is printable exactly when every one
    // of its leaves is a page-row head.
    match filter {
        Filter::True => Some(String::new()),
        Filter::And { items } if items.is_empty() => Some(String::new()),
        Filter::And { items } if items.len() == 1 => og_clause(&items[0], query.anchor),
        other => og_clause(other, query.anchor),
    }
}

fn og_clause(filter: &Filter, anchor: Anchor) -> Option<String> {
    match filter {
        // OG has no boolean literal and an empty nested group is one. The root
        // empty `And` is handled as the historical empty query in `og_form`.
        Filter::And { items } | Filter::Or { items } if items.is_empty() => None,
        Filter::And { items } | Filter::Or { items } => {
            let head = if matches!(filter, Filter::And { .. }) {
                "and"
            } else {
                "or"
            };
            let kids = items
                .iter()
                .map(|item| og_clause(item, anchor))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("({head} {})", kids.join(" ")))
        }
        Filter::Not { inner } => Some(format!("(not {})", og_clause(inner, anchor)?)),
        Filter::Leaf { leaf } => og_leaf(leaf, anchor, false),
        // `Off`, `Raw`, `True` and `False` have no OG spelling: OG has no
        // disabled node, no unknown head it would keep, and no boolean literal.
        _ => None,
    }
}

fn og_leaf(leaf: &Leaf, anchor: Anchor, through_page: bool) -> Option<String> {
    match leaf {
        Leaf::Attr { attr, op, value } => {
            og_attr(*attr, *op, value, through_page || anchor == Anchor::Page)
        }
        Leaf::Rel { rel, quant, pred } => og_rel(*rel, *quant, pred, anchor),
    }
}

/// **The Tine-only heads are deliberately absent (§3.3 B4).** `(scheduled)`,
/// `(deadline)`, `(search …)` and `(content-regex …)` are heads Tine's OG-syntax
/// PARSER accepts so existing files keep working; the printer never emits them,
/// because a `{{query}}` block containing one would not mean the same thing in
/// OG. An edited presence leaf is persisted as `{{tine-query …}}` in TQL, and an
/// untouched block stays byte-stable (I-4).
fn og_attr(attr: Attr, op: CmpOp, value: &Value, on_page: bool) -> Option<String> {
    match (attr, op) {
        (Attr::Content, CmpOp::Like) => {
            // `(content-like)` does not exist: a bare string IS the substring
            // test, and only the `%x%` shape is that test. The unescaping is
            // `og::escape_like`'s own inverse, never a second copy of it.
            let Value::Text { text } = value else {
                return None;
            };
            crate::query::og::plain_like_substring(text).map(|plain| quote_str(&plain))
        }
        (Attr::Task, CmpOp::In) => Some(og_words("task", list_of(value)?)),
        (Attr::Priority, CmpOp::In) => Some(og_words("priority", list_of(value)?)),
        (Attr::Scheduled, CmpOp::Between) => og_between("scheduled", value),
        (Attr::Deadline, CmpOp::Between) => og_between("deadline", value),
        (Attr::Day, CmpOp::Between) if on_page => og_between("journal", value),
        (Attr::Journal, CmpOp::Eq) if on_page && *value == (Value::Bool { value: true }) => {
            Some("(journal)".to_string())
        }
        (Attr::Name, CmpOp::Eq) if on_page => Some(format!("(page {})", word(text_of(value)?))),
        (Attr::Name, CmpOp::StartsWith) if on_page => {
            let namespace = text_of(value)?.strip_suffix('/')?;
            Some(format!("(namespace {})", word(namespace)))
        }
        _ => None,
    }
}

fn og_rel(rel: Rel, quant: Quant, pred: &Filter, anchor: Anchor) -> Option<String> {
    if quant != Quant::Any {
        return None;
    }
    match rel {
        Rel::Refs => Some(format!("[[{}]]", single_name(pred)?)),
        Rel::Page => match pred {
            Filter::Leaf { leaf } => og_leaf(leaf, anchor, true),
            _ => None,
        },
        Rel::Props => og_props(pred, anchor == Anchor::Page),
        // `tags` (the block's own inline tags), `children` and `blocks` are
        // Tine-only relations: OG's DSL has no head for any of them.
        Rel::Tags | Rel::Children | Rel::Blocks => None,
    }
}

fn og_props(pred: &Filter, on_page: bool) -> Option<String> {
    let key = pred.props_key()?;
    let head = if on_page { "page-property" } else { "property" };
    match pred.props_atom_test() {
        None => Some(format!("({head} {})", word(&key))),
        Some(atom) => {
            // `(page-tags …)` is the page's `tags` property with a set test.
            if on_page && key == "tags" {
                if let Filter::Leaf {
                    leaf:
                        Leaf::Attr {
                            attr: Attr::Value,
                            op: CmpOp::In,
                            value,
                        },
                } = &atom
                {
                    let tags = list_of(value)?;
                    return Some(format!("(page-tags {})", tags.join(" ")));
                }
            }
            let Filter::Leaf {
                leaf:
                    Leaf::Attr {
                        attr: Attr::Value,
                        op: CmpOp::Eq,
                        value,
                    },
            } = &atom
            else {
                return None;
            };
            Some(format!("({head} {} {})", word(&key), word(text_of(value)?)))
        }
    }
}

fn og_words(head: &str, items: Vec<String>) -> String {
    if items.is_empty() {
        format!("({head})")
    } else {
        format!("({head} {})", items.join(" "))
    }
}

fn og_between(field: &str, value: &Value) -> Option<String> {
    let Value::List { items } = value else {
        return None;
    };
    let [low, high] = items.as_slice() else {
        return None;
    };
    let field = if field == "journal" {
        String::new()
    } else {
        format!("{field} ")
    };
    Some(format!(
        "(between {field}{} {})",
        date_bound(low)?,
        date_bound(high)?
    ))
}

/// A bound that resolves on its own is bare; a journal page title is wrapped in
/// `[[ ]]`. Read back by `og`'s `between` parser, which is what defines the
/// distinction now that the frontend's `dateBound` is gone.
fn date_bound(value: &Value) -> Option<String> {
    let Value::Date { literal } = value else {
        return None;
    };
    let text = literal.trim();
    Some(if is_bare_date_token(text) {
        text.to_string()
    } else {
        format!("[[{text}]]")
    })
}

/// The [`DateToken`](crate::query::DateToken) grammar's own answer, so the
/// printer and the resolver cannot drift (Rule 3). A non-date literal is
/// written as a page title, `[[ ]]`.
fn is_bare_date_token(text: &str) -> bool {
    crate::query::DateToken::parse(text).is_some_and(|token| token.prints_bare())
}

/// A DSL double-quoted string with `\` and `"` escaped, so a value containing a
/// quote round-trips through `og::read_string`.
fn quote_str(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Quote when the value cannot be a bare word — the complement of what
/// `og::read_string` accepts unquoted.
fn needs_quote(text: &str) -> bool {
    text.is_empty()
        || text
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '"'))
}

fn word(text: &str) -> String {
    if needs_quote(text) {
        quote_str(text)
    } else {
        text.to_string()
    }
}

fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Text { text } => Some(text),
        _ => None,
    }
}

fn list_of(value: &Value) -> Option<Vec<String>> {
    let Value::List { items } = value else {
        return None;
    };
    items
        .iter()
        .map(|item| text_of(item).map(str::to_string))
        .collect()
}

/// The view directives OG can carry, in `toDsl`'s order. `None` means the view
/// has something OG cannot say, which is exactly one thing: more than one sort
/// key. A COLUMN SET was never part of that answer — the code below has only
/// ever returned `None` for `view.sort.len() > 1`, and columns have no OG text
/// spelling at all in either direction (nothing parses one, nothing prints
/// one). They live only in block properties, which is why `tine.columns::` can
/// be added without narrowing `og_expressible` (P5A, F1).
fn og_view(view: &ViewSettings) -> Option<Vec<String>> {
    if view.sort.len() > 1 {
        return None;
    }
    let mut out = Vec::new();
    if let Some((field, dir)) = view.sort.first() {
        let dir = match dir {
            SortDir::Asc => "asc",
            SortDir::Desc => "desc",
        };
        out.push(format!("(sort-by {} {dir})", word(field.as_str())));
    }
    if let Some(sample) = view.sample {
        out.push(format!("(sample {sample})"));
    }
    // **Aggregates and `group-by` are NOT re-emitted (§4.3 "Directive migration",
    // Q15).** They live in `tine.col-aggregates::` / `tine.group-by::`, which
    // `merge_block_property_view` reads with absolute precedence over the DSL
    // text. Printing them here as well would put the same view in two places and
    // let the text copy outlive a removal made in the builder. `sort-by` and
    // `sample` keep their text form, because OG itself reads those.
    //
    // This does not narrow `og_view`'s `None` (still only a multi-key sort), so
    // `og_expressible` is unchanged: a query with aggregates stays expressible.
    Some(out)
}

#[cfg(test)]
#[path = "print_tests.rs"]
mod tests;
