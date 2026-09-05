//! IR → text (SPEC §4.3).
//!
//! Two printers, one entry: [`query_print`]. TQL is total — every IR the
//! parsers can build has a TQL spelling. The OG DSL is **partial** and says so:
//! it is defined only where [`og_expressible`] holds, and returns a
//! `NotApplicable` diagnostic everywhere else rather than emitting a query that
//! means something different (I-12: one canonical answer).
//!
//! The OG serialization is transcribed (D-9) from the frontend's own `toDsl`
//! in `src/editor/queryBuilder.ts`, including its `quoteStr`/`needsQuote`
//! escaping, its `dateBound` bare-vs-`[[…]]` rule, and its single-child `and`
//! simplification (which matches OG `simplify-query`).
//!
//! **Deviation from §4.3, recorded (D-14 would otherwise apply):** the TQL
//! printer emits text directly instead of `Display`-ing a rebuilt `sqlparser`
//! AST. `sqlparser`'s `Display` cannot produce either of the two things the
//! canonical form is defined by — `[[x]]` restored for a `refs` leaf, and the
//! K10 line layout with `-- ` prefixes — so a rebuilt AST would be
//! post-processed into unrecognisability. The round-trip property
//! (`parse(print(q)).normalized() == q.normalized()`) is what actually pins
//! this printer, and it is asserted below over every shape the parsers build.

use crate::query::ir::{
    AggFn, Anchor, Attr, CmpOp, Diagnostic, DiagnosticKind, Field, Filter, Leaf, Quant, Query, Rel,
    SortDir, Source, Value, ViewSettings,
};
use crate::query::QueryDialect;

/// Print a query in the requested dialect. `view` is explicit because the view
/// directives (`sort-by`, `sample`, `aggregate`, `group-by`) live outside the
/// filter and are re-emitted into the OG text when expressible (M15).
pub fn query_print(
    query: &Query,
    view: &ViewSettings,
    dialect: QueryDialect,
) -> Result<String, Diagnostic> {
    match dialect {
        QueryDialect::Tql => Ok(print_tql(query)),
        QueryDialect::Og => print_og(query, view),
    }
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
    // The canonical form is the normalized tree: identity operands (the
    // `Filter::True` a view directive contributes, an empty `and`) are not part
    // of what the author wrote, and printing them back would make the round
    // trip depend on parser bookkeeping.
    let query = &query.normalized();
    let anchor = match query.anchor {
        Anchor::Block => "",
        Anchor::Page => "@page",
    };
    if query.filter == Filter::True {
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

fn contains_off(filter: &&Filter) -> bool {
    matches!(filter, Filter::Off { .. })
}

/// K10: one root operand per line, connector-leading, a root `Off` operand's
/// lines prefixed `-- `, and a bare `--` between two consecutive root `Off`
/// siblings so the pre-pass reads them back as two nodes (N2).
fn print_tql_layered(filter: &Filter) -> String {
    let connector = match filter {
        Filter::Or { .. } => "or ",
        _ => "and ",
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
                lines.push(format!("-- {lead}{}", tql_expr(inner, Prec::And)));
                previous_was_off = true;
            }
            other => {
                lines.push(format!("{lead}{}", tql_expr(other, Prec::And)));
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

fn tql_expr(filter: &Filter, context: Prec) -> String {
    match filter {
        Filter::True => "true".to_string(),
        Filter::False => "false".to_string(),
        Filter::Raw { text, .. } => format!("raw({})", sql_string(text)),
        // Below the root every `Off` prints inline as the function form, which
        // is legal TQL (§4.2.3) and is what the parser reads back.
        Filter::Off { inner } => format!("off({})", tql_expr(inner, Prec::Or)),
        Filter::Not { inner } => parens(
            format!("not {}", tql_expr(inner, Prec::Not)),
            context > Prec::Not,
        ),
        Filter::And { items } => {
            let text = items
                .iter()
                .map(|item| tql_expr(item, Prec::And))
                .collect::<Vec<_>>()
                .join(" and ");
            parens(text, context > Prec::And)
        }
        Filter::Or { items } => {
            let text = items
                .iter()
                .map(|item| tql_expr(item, Prec::Or))
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
    match (quant, atom) {
        (Quant::Any, None) => format!("{spelled} is not null"),
        (Quant::None, None) => format!("{spelled} is null"),
        (Quant::Any, Some(atom)) => match blank_test(&atom) {
            true => format!("{spelled} = ''"),
            false => {
                // A `tags` property whose only test is an equality is the page's
                // tag: the shorter spelling reads back as the same leaf.
                if through_page && key == "tags" {
                    if let Some(tag) = single_value_equality(&atom) {
                        return format!("page_tag({})", sql_string(&tag));
                    }
                }
                tql_atom_expr(&atom, &spelled)
            }
        },
        (quant, Some(atom)) => format!(
            "{}({spelled}, {})",
            quant_name(quant),
            tql_atom_expr(&atom, "value")
        ),
        (quant, None) => format!("{}({spelled}, true)", quant_name(quant)),
    }
}

/// An atom test printed against `subject`: at `Any` the subject is the whole
/// `prop('k')` call (`prop('k') = 'x'`); under a quantifier it is the
/// contextual identifier `value`.
fn tql_atom_expr(atom: &Filter, subject: &str) -> String {
    match atom {
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Value,
                    op,
                    value,
                },
        } => tql_comparison(subject, *op, value),
        Filter::Not { inner } => format!("not {}", tql_atom_expr(inner, subject)),
        Filter::And { items } => items
            .iter()
            .map(|item| tql_atom_expr(item, subject))
            .collect::<Vec<_>>()
            .join(" and "),
        Filter::Or { items } => format!(
            "({})",
            items
                .iter()
                .map(|item| tql_atom_expr(item, subject))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
        other => tql_expr(other, Prec::Or),
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
        // `Regex` is the OG-only `(content-regex …)` head; TQL v1 has no
        // spelling, so the printed form names it rather than pretending.
        CmpOp::Regex => format!("{subject} regex {}", tql_value(value)),
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

fn print_og(query: &Query, view: &ViewSettings) -> Result<String, Diagnostic> {
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
    for directive in directives {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&directive);
    }
    // The trailing options map is re-emitted verbatim: it is the author's text,
    // not something the printer re-derives (I-4).
    if let Source::Og { og_options, .. } = &query.source {
        if !og_options.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(og_options);
        }
    }
    Ok(out)
}

/// `toDsl`'s root rule: a single-child `and` simplifies to the child, matching
/// OG `simplify-query`; an empty root is the empty string.
fn og_form(query: &Query) -> Option<String> {
    let normalized = query.normalized();
    let filter = &normalized.filter;
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

/// `queryBuilder.ts` `dateBound`: a bound that resolves on its own is bare, a
/// journal page title is wrapped in `[[ ]]`.
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

fn is_bare_date_token(text: &str) -> bool {
    if ["today", "yesterday", "tomorrow", "now"]
        .iter()
        .any(|keyword| text.eq_ignore_ascii_case(keyword))
    {
        return true;
    }
    let bytes = text.as_bytes();
    if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return text
            .split('-')
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()));
    }
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let Some(unit) = digits.chars().last() else {
        return false;
    };
    digits.len() > 1
        && "dwmy".contains(unit.to_ascii_lowercase())
        && digits[..digits.len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
}

/// `queryBuilder.ts` `quoteStr`: a DSL double-quoted string with `\` and `"`
/// escaped, so a value containing a quote round-trips.
fn quote_str(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `queryBuilder.ts` `needsQuote`: quote when the value cannot be a bare word.
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
/// has something OG cannot say — more than one sort key, or a column set.
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
    // Until P2 the aggregate/group-by directives still live in the DSL text
    // (M15) rather than in `tine.col-aggregates::` / `tine.group-by::`.
    for (field, agg) in &view.aggregates {
        out.push(og_aggregate(field, *agg));
    }
    if let Some(field) = &view.group_by {
        out.push(format!("(group-by {})", word(field.as_str())));
    }
    Some(out)
}

fn og_aggregate(field: &Field, agg: AggFn) -> String {
    let name = match agg {
        AggFn::Count => return "(aggregate count)".to_string(),
        AggFn::Sum => "sum",
        AggFn::Avg => "avg",
    };
    format!("(aggregate {name} {})", word(field.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::JournalDate;
    use crate::query::{parse_query_text, tql::parse_tql};

    fn tql(source: &str) -> Query {
        parse_tql(source, crate::query::registry::Registry::none()).0
    }

    fn og(source: &str) -> (Query, ViewSettings) {
        parse_query_text(
            source,
            QueryDialect::Og,
            JournalDate::from_ordinal(20260904),
        )
    }

    /// The round-trip property §4.3 defines: printing then re-parsing is the
    /// identity on normalized queries.
    fn round_trips(source: &str) {
        let query = tql(source);
        assert!(
            !query.is_invalid(),
            "{source} did not parse: {:?}",
            query.diagnostics
        );
        let printed = print_tql(&query);
        let again = tql(&printed);
        assert!(
            !again.is_invalid(),
            "printing {source} produced {printed:?}, which does not parse: {:?}",
            again.diagnostics
        );
        assert_eq!(
            again.normalized(),
            query.normalized(),
            "{source} printed as {printed:?}"
        );
    }

    #[test]
    fn every_tql_shape_round_trips() {
        for source in [
            "@block",
            "@page",
            "#x",
            "[[a b]]",
            "#x and #y",
            "#x or #y",
            "not #x",
            "#x and (#y or #z)",
            "(#x or #y) and not #z",
            "task = 'TODO'",
            "task in ('TODO', 'DOING')",
            "priority = 'A'",
            "content like '%foo%'",
            "content match 'foo'",
            "scheduled is not null",
            "deadline is null",
            "scheduled between today and '+7d'",
            "page.name = 'Home'",
            "page.name like 'proj/%'",
            "page.journal = true",
            "page.day between '2026-01-01' and '2026-12-31'",
            "prop('k') = 'v'",
            "prop('k') != 'v'",
            "prop('k') is null",
            "prop('k') is not null",
            "prop('k') = ''",
            "prop('k') in ('a', 'b')",
            "prop('k') > 3",
            "every(prop('k'), value > 3)",
            "none(prop('k'), value = 'x')",
            "page_prop('status') = 'public'",
            "page_tag('work')",
            "tag('x')",
            "any(children, task = 'TODO')",
            "every(children, task = 'DONE')",
            "none(children, task = 'TODO')",
            "@page and any(blocks, task = 'TODO')",
            "@page and name = 'Home'",
            "@page and journal = true",
            "off(#x)",
            "not off([[a]])",
            "any(children, off(task = 'TODO'))",
            "([[a]] or off([[b]]))",
            "#a and (#b or off(#c))",
        ] {
            round_trips(source);
        }
    }

    /// Every `{{query …}}` shipped in the repository — the templates the app
    /// installs and the parser fixture — parsed as OG, printed as TQL, and
    /// re-parsed. This is the corpus round-trip §4.3 asks for; the private
    /// anonymized graph contains no `{{query}}` at all, so it cannot supply one.
    #[test]
    fn the_shipped_query_corpus_round_trips_through_the_tql_printer() {
        const CORPUS: &[&str] = &[
            include_str!("../templates/showcase.md"),
            include_str!("../templates/sheets.md"),
            include_str!("../templates/capture-plan-day.md"),
            include_str!("../templates/find-and-revisit.md"),
            include_str!("../templates/journals-tasks-scheduling.md"),
            include_str!("../templates/pages-links-references-search.md"),
            include_str!("../templates/structure-repeated-information.md"),
        ];
        let mut seen = 0usize;
        for text in CORPUS {
            for source in macro_arguments(text) {
                let (query, _) = og(&source);
                assert!(
                    !query.is_invalid(),
                    "shipped query {source:?} does not parse: {:?}",
                    query.diagnostics
                );
                let printed = print_tql(&query);
                let again = tql(&printed);
                assert!(
                    !again.is_invalid(),
                    "{source:?} printed as {printed:?}, which does not parse: {:?}",
                    again.diagnostics
                );
                assert_eq!(
                    again.normalized().filter,
                    query.normalized().filter,
                    "{source:?} printed as {printed:?}"
                );
                seen += 1;
            }
        }
        assert!(seen >= 10, "the corpus scan found only {seen} queries");
    }

    /// Every `{{query …}}` argument in one document, brace-balanced so a
    /// trailing options map stays inside the macro.
    fn macro_arguments(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find("{{query") {
            let after = &rest[at + "{{query".len()..];
            let Some(end) = after.find("}}") else { break };
            let argument = after[..end].trim();
            if !argument.is_empty() {
                out.push(argument.to_string());
            }
            rest = &after[end + 2..];
        }
        out
    }

    #[test]
    fn adjacent_root_off_siblings_print_as_two_dash_runs() {
        let query = tql("-- #x\n--\n-- and #y");
        assert_eq!(print_tql(&query), "-- [[x]]\n--\n-- and [[y]]");
        round_trips("-- #x\n--\n-- and #y");
    }

    #[test]
    fn an_off_below_the_root_prints_inline() {
        assert_eq!(print_tql(&tql("not off([[a]])")), "not off([[a]])");
        assert_eq!(
            print_tql(&tql("any(children, off([[a]]))")),
            "any(children, off([[a]]))"
        );
        // At the ROOT the `Off` operand takes the `-- ` line form, and the
        // connector sits inside the run exactly where the pre-pass lifts it out.
        assert_eq!(print_tql(&tql("[[a]] or off([[b]])")), "[[a]]\n-- or [[b]]");
    }

    #[test]
    fn an_off_inside_a_group_stays_inline() {
        // The `or` group is not the root, so its `Off` operand prints inline.
        let query = tql("[[z]] and ([[a]] or off([[b]]))");
        assert_eq!(print_tql(&query), "[[z]] and ([[a]] or off([[b]]))");
    }

    #[test]
    fn a_root_off_operand_prefixes_its_line() {
        let query = tql("#a\n-- and #b");
        assert_eq!(print_tql(&query), "[[a]]\n-- and [[b]]");
    }

    #[test]
    fn nested_off_collapses_under_normalization() {
        let query = tql("off(off([[a]]))");
        assert_eq!(
            query.normalized().filter,
            Filter::off(Filter::page_ref("a"))
        );
    }

    // -- OG printer ---------------------------------------------------------

    fn og_round_trips(source: &str) {
        let (query, view) = og(source);
        assert!(
            !query.is_invalid(),
            "{source} did not parse: {:?}",
            query.diagnostics
        );
        let printed = query_print(&query, &view, QueryDialect::Og)
            .unwrap_or_else(|d| panic!("{source} is not OG-printable: {d:?}"));
        let (again, again_view) = og(&printed);
        assert_eq!(
            again.normalized(),
            query.normalized(),
            "{source} printed as {printed:?}"
        );
        assert_eq!(again_view, view, "{source} printed as {printed:?}");
    }

    #[test]
    fn og_expressible_queries_round_trip_through_the_og_printer() {
        for source in [
            "[[Alpha]]",
            "(and [[Alpha]] [[Beta]])",
            "(or [[Alpha]] [[Beta]])",
            "(not [[Alpha]])",
            "(task TODO DOING)",
            "(priority A)",
            "(property status active)",
            "(page-property category work)",
            "(page Home)",
            "(namespace Project)",
            "(journal)",
            "(page-tags public private)",
            "(between scheduled today +7d)",
            "(and (task TODO) (page Home))",
        ] {
            og_round_trips(source);
        }
    }

    #[test]
    fn the_og_printer_refuses_what_og_cannot_say() {
        for source in [
            "off(#x)",
            "tag('x')",
            "any(children, task = 'TODO')",
            "every(prop('k'), value > 3)",
            "prop('k') > 3",
            // Tine-only heads the OG-syntax parser reads but the printer must
            // never write back (§3.3 B4).
            "content match 'foo'",
            "scheduled is not null",
        ] {
            let query = tql(source);
            let view = ViewSettings::default();
            assert!(
                !og_expressible(&query, &view),
                "{source} must not be OG-expressible"
            );
            let printed = query_print(&query, &view, QueryDialect::Og);
            assert!(
                matches!(&printed, Err(d) if d.kind == DiagnosticKind::NotApplicable),
                "{source} printed as {printed:?}"
            );
        }
    }

    #[test]
    fn the_og_printer_re_emits_the_view_and_the_options_map_verbatim() {
        let (query, view) = og("(task TODO) (sort-by page asc) {:title \"T\"}");
        let printed = query_print(&query, &view, QueryDialect::Og).expect("printable");
        assert_eq!(printed, "(task TODO) (sort-by page asc) {:title \"T\"}");
    }

    #[test]
    fn a_quoted_value_survives_the_og_escaping() {
        let (query, view) = og("(property note \"a \\\"b\\\" c\")");
        let printed = query_print(&query, &view, QueryDialect::Og).expect("printable");
        let (again, _) = og(&printed);
        assert_eq!(again.normalized(), query.normalized());
    }
}
