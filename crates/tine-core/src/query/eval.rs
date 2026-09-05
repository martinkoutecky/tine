//! The walk's evaluation of a [`Query`] (SPEC §3.2–§3.5).
//!
//! **Two-valued leaves (Q5).** Every leaf is exactly true or false: an absent
//! optional attribute makes any comparison on it false, an atom that does not
//! coerce to the compared type fails the comparison, and `not`/`and`/`or` are
//! classical — so `(not (task DONE))` includes non-task blocks exactly as OG
//! does. `Any` over an empty relation is false and `Every` over an empty
//! relation is true (OData §5.1.1.13).
//!
//! **Property leaves in Wave A keep today's matching semantics exactly**
//! (`value_matches`: comma-split, ref-stripped, lowercased), so retargeting the
//! walk onto the IR is a pure refactor; Wave B swaps the atomizer in underneath
//! the same `Rel::Props` leaf and re-runs the corpus-parity export (O8).

use std::collections::HashMap;

use crate::date::JournalDate;
use crate::doc::{property_key_norm, DocBlock};
use crate::model::PageKind;
use crate::query::ir::{Attr, CmpOp, Filter, Leaf, Quant, Rel, Value};
use crate::refs;
use crate::search_query::{canonical_fold, Matcher};

/// Per-page evaluation context: the page row a block row belongs to, plus the
/// evaluation's `today` (relative date literals stay unresolved in the IR).
pub(crate) struct EvalCtx<'a> {
    /// The page's journal-day ordinal (`yyyymmdd`), or `None` for named pages.
    pub(crate) journal: Option<i64>,
    pub(crate) is_journal: bool,
    pub(crate) page_name: &'a str,
    pub(crate) page_props: &'a [(String, String)],
    pub(crate) today: JournalDate,
    pub(crate) compiled: &'a CompiledLeaves,
}

/// Patterns that cost real work to build (`(search …)`'s friendly matcher, a
/// `(content-regex …)` regex) compiled ONCE per query rather than per block.
/// The old `Pred` carried the compiled value inside the variant; the IR carries
/// only the user's text, so the compile cache lives here.
#[derive(Default)]
pub(crate) struct CompiledLeaves {
    matchers: HashMap<String, Matcher>,
    regexes: HashMap<String, Option<regex::Regex>>,
    folded: HashMap<String, String>,
}

impl CompiledLeaves {
    pub(crate) fn for_query(filter: &Filter) -> CompiledLeaves {
        let mut compiled = CompiledLeaves::default();
        collect_compiled(filter, &mut compiled);
        compiled
    }

    fn matcher(&self, source: &str) -> Option<&Matcher> {
        self.matchers.get(source)
    }
    fn regex(&self, source: &str) -> Option<&regex::Regex> {
        self.regexes.get(source).and_then(Option::as_ref)
    }
    fn fold(&self, text: &str) -> String {
        self.folded
            .get(text)
            .cloned()
            .unwrap_or_else(|| canonical_fold(text))
    }
}

fn collect_compiled(filter: &Filter, out: &mut CompiledLeaves) {
    filter.any_leaf(&mut |leaf| {
        if let Leaf::Attr {
            attr: Attr::Content,
            op,
            value: Value::Text { text },
        } = leaf
        {
            match op {
                CmpOp::Match => {
                    out.matchers
                        .entry(text.clone())
                        .or_insert_with(|| Matcher::parse(text));
                }
                CmpOp::Regex => {
                    out.regexes
                        .entry(text.clone())
                        .or_insert_with(|| regex::Regex::new(text).ok());
                }
                CmpOp::Like => {
                    out.folded
                        .entry(text.clone())
                        .or_insert_with(|| canonical_fold(text));
                }
                _ => {}
            }
        }
        false
    });
}

type PathRefCounts = HashMap<String, usize>;

/// Whether this query reads `:block/path-refs`, i.e. whether the walk has to
/// maintain the ancestor-ref counters at all.
pub(crate) fn uses_path_refs(filter: &Filter) -> bool {
    filter.any_leaf(&mut |leaf| matches!(leaf, Leaf::Rel { rel: Rel::Refs, .. }))
}

/// Evaluate a filter against one BLOCK row.
pub(crate) fn eval_block(
    filter: &Filter,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    match filter {
        Filter::And { items } => items
            .iter()
            .all(|item| eval_block(item, block, ancestor_refs, ctx)),
        Filter::Or { items } => items
            .iter()
            .any(|item| eval_block(item, block, ancestor_refs, ctx)),
        Filter::Not { inner } => !eval_block(inner, block, ancestor_refs, ctx),
        Filter::True => true,
        // A `Raw` span is never satisfiable: the query carrying it is invalid and
        // returns nothing, and this keeps `not(<raw>)` from inventing matches.
        Filter::False | Filter::Raw { .. } => false,
        // `Off` is removed by `Query::evaluable_filter()` before evaluation
        // (§3.5); reaching one means a caller skipped that step.
        Filter::Off { .. } => {
            debug_assert!(false, "Off must be removed before evaluation (§3.5)");
            true
        }
        Filter::Leaf { leaf } => eval_block_leaf(leaf, block, ancestor_refs, ctx),
    }
}

fn eval_block_leaf(
    leaf: &Leaf,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    match leaf {
        Leaf::Attr { attr, op, value } => match attr {
            Attr::Content => eval_content(*op, value, block, ctx),
            Attr::Task => eval_optional_text(*op, value, block.marker()),
            Attr::Priority => eval_optional_text(*op, value, block.priority()),
            Attr::Scheduled => {
                eval_planning(*op, value, block.projection().scheduled.as_deref(), ctx)
            }
            Attr::Deadline => {
                eval_planning(*op, value, block.projection().deadline.as_deref(), ctx)
            }
            // Page attributes only ever appear under a `page` relation, and the
            // property-element attributes only under `props`.
            _ => false,
        },
        Leaf::Rel { rel, quant, pred } => match rel {
            Rel::Refs => eval_refs(*quant, pred, block, ancestor_refs, ctx),
            Rel::Tags => quantify(*quant, block.projection().tags.iter(), |tag| {
                eval_name_element(pred, tag)
            }),
            Rel::Props => quantify(*quant, block.properties().iter(), |(key, value)| {
                eval_property_element(pred, key, value)
            }),
            Rel::Children => quantify(*quant, block.children.iter(), |child| {
                // A child is a fresh row: it carries no ancestor-ref context of
                // its own here, matching the direct-children-only rule (A1).
                eval_block(pred, child, ancestor_refs, ctx)
            }),
            // To-one: the page row is exactly one element, so all three
            // quantifiers reduce to the predicate (or its negation).
            Rel::Page => {
                let hit = eval_page(pred, ctx);
                match quant {
                    Quant::Any | Quant::Every => hit,
                    Quant::None => !hit,
                }
            }
            // `blocks` is a page-row relation; a block-anchored walk never sees it.
            Rel::Blocks => false,
        },
    }
}

/// Evaluate a filter against one PAGE row.
pub(crate) fn eval_page(filter: &Filter, ctx: &EvalCtx) -> bool {
    match filter {
        Filter::And { items } => items.iter().all(|item| eval_page(item, ctx)),
        Filter::Or { items } => items.iter().any(|item| eval_page(item, ctx)),
        Filter::Not { inner } => !eval_page(inner, ctx),
        Filter::True => true,
        Filter::False | Filter::Raw { .. } => false,
        Filter::Off { .. } => {
            debug_assert!(false, "Off must be removed before evaluation (§3.5)");
            true
        }
        Filter::Leaf { leaf } => match leaf {
            Leaf::Attr { attr, op, value } => match attr {
                Attr::Name => eval_page_name(*op, value, ctx.page_name),
                Attr::Journal => {
                    matches!(
                        (op, value),
                        (CmpOp::Eq, Value::Bool { value: true }) if ctx.is_journal
                    ) || matches!(
                        (op, value),
                        (CmpOp::Eq, Value::Bool { value: false }) if !ctx.is_journal
                    )
                }
                Attr::Day => eval_day(*op, value, ctx.journal, ctx.today),
                Attr::Namespace => {
                    // The immediate parent segment (Tine-only, M20).
                    let key = refs::normalize(ctx.page_name);
                    let parent = key.rsplit_once('/').map(|(head, _)| head.to_string());
                    eval_optional_text(*op, value, parent.as_deref())
                }
                _ => false,
            },
            Leaf::Rel { rel, quant, pred } => match rel {
                Rel::Props => quantify(*quant, ctx.page_props.iter(), |(key, value)| {
                    eval_property_element(pred, key, value)
                }),
                // A page's own refs, its blocks and its tag table are not walked
                // by this evaluator: the OG DSL cannot express them and the
                // page-anchored walk of Wave A reads only the page index.
                _ => false,
            },
        },
    }
}

/// `Any` false / `Every` true on an empty collection (OData §5.1.1.13, Q5).
fn quantify<T>(
    quant: Quant,
    mut items: impl Iterator<Item = T>,
    mut test: impl FnMut(T) -> bool,
) -> bool {
    match quant {
        Quant::Any => items.any(&mut test),
        Quant::None => !items.any(&mut test),
        Quant::Every => items.all(&mut test),
    }
}

/// The predicate over a ref or tag element, whose only attribute is `name`.
fn eval_name_element(pred: &Filter, name: &str) -> bool {
    match pred {
        Filter::True => true,
        Filter::False => false,
        Filter::And { items } => items.iter().all(|item| eval_name_element(item, name)),
        Filter::Or { items } => items.iter().any(|item| eval_name_element(item, name)),
        Filter::Not { inner } => !eval_name_element(inner, name),
        Filter::Leaf {
            leaf:
                Leaf::Attr {
                    attr: Attr::Name,
                    op: CmpOp::Eq,
                    value: Value::Text { text },
                },
        } => refs::page_key(name) == refs::page_key(text),
        _ => false,
    }
}

/// `refs` is OG's `:block/path-refs`: this block's explicit and property refs,
/// every ancestor's, and the page it lives on. The `name = 'x'` predicate — the
/// only one v1 accepts — is answered by membership so the walk does not
/// materialize the closure per block.
fn eval_refs(
    quant: Quant,
    pred: &Filter,
    block: &DocBlock,
    ancestor_refs: &PathRefCounts,
    ctx: &EvalCtx,
) -> bool {
    if let Some(name) = single_ref_name(pred) {
        let normalized = refs::normalize(&name);
        let hit = block.projection().refs_contains_norm(&normalized)
            || ancestor_refs.contains_key(&normalized)
            || refs::normalize(ctx.page_name) == normalized;
        return match quant {
            Quant::Any | Quant::Every => hit,
            Quant::None => !hit,
        };
    }
    let names = block
        .projection()
        .refs_norm
        .iter()
        .cloned()
        .chain(ancestor_refs.keys().cloned())
        .chain(std::iter::once(refs::normalize(ctx.page_name)))
        .collect::<Vec<_>>();
    quantify(quant, names.iter(), |name| eval_name_element(pred, name))
}

/// The `name = 'x'` shape both `[[x]]` and `#x` produce.
fn single_ref_name(pred: &Filter) -> Option<String> {
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

// ---------------------------------------------------------------------------
// Property elements — Wave A keeps `value_matches`'s exact semantics
// ---------------------------------------------------------------------------

/// One property element `(key, raw value)` of the owner row. Its attributes are
/// `key`, `value` (one atom test) and `atom_count`.
fn eval_property_element(pred: &Filter, key: &str, value: &str) -> bool {
    match pred {
        Filter::True => true,
        Filter::False => false,
        Filter::And { items } => items
            .iter()
            .all(|item| eval_property_element(item, key, value)),
        Filter::Or { items } => items
            .iter()
            .any(|item| eval_property_element(item, key, value)),
        Filter::Not { inner } => !eval_property_element(inner, key, value),
        Filter::Leaf {
            leaf: Leaf::Attr { attr, op, value: v },
        } => match attr {
            Attr::Key => match (op, v) {
                (CmpOp::Eq, Value::Text { text }) => {
                    property_key_norm(key) == property_key_norm(text)
                }
                _ => false,
            },
            Attr::Value => match (op, v) {
                (CmpOp::Eq, Value::Text { text }) => value_matches(value, Some(text)),
                (CmpOp::In, Value::List { items }) => items.iter().any(|item| match item {
                    Value::Text { text } => value_matches(value, Some(text)),
                    _ => false,
                }),
                // K3: `!=` is "coercible AND unequal" — an owner with no atom at
                // all fails it, exactly as it fails `=`.
                (CmpOp::NotEq, Value::Text { text }) => {
                    atom_count(value) > 0 && !value_matches(value, Some(text))
                }
                (CmpOp::IsSet, Value::None) => true,
                _ => false,
            },
            Attr::AtomCount => match (op, v) {
                (CmpOp::Gt, Value::Number { number }) => atom_count(value) as f64 > *number,
                (CmpOp::Eq, Value::Number { number }) => atom_count(value) as f64 == *number,
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// The number of atoms of a property value under Wave A's semantics: the
/// non-empty comma-separated segments `value_matches` already compares against.
fn atom_count(stored: &str) -> usize {
    stored
        .split(',')
        .filter(|part| !strip_ref(part.trim()).is_empty())
        .count()
}

/// Match a stored property value against a query value: multi-value
/// (comma-separated) and page-ref / tag wrapping, case-insensitively. A `None`
/// query value matches any present value.
pub(crate) fn value_matches(stored: &str, query: Option<&str>) -> bool {
    let Some(q) = query else { return true };
    let q = strip_ref(q).to_lowercase();
    stored
        .split(',')
        .map(|p| strip_ref(p.trim()).to_lowercase())
        .any(|v| v == q)
}

pub(crate) fn strip_ref(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t).trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|x| x.strip_suffix("]]"))
        .unwrap_or(t);
    t.trim().to_string()
}

// ---------------------------------------------------------------------------
// Attribute comparisons
// ---------------------------------------------------------------------------

fn eval_content(op: CmpOp, value: &Value, block: &DocBlock, ctx: &EvalCtx) -> bool {
    let Value::Text { text } = value else {
        return false;
    };
    let projection = block.projection();
    match op {
        CmpOp::Like => like_matches(&projection.visible_lower, &ctx.compiled.fold(text)),
        CmpOp::StartsWith => projection
            .visible_lower
            .starts_with(&ctx.compiled.fold(text)),
        CmpOp::Eq => projection.visible_lower == ctx.compiled.fold(text),
        CmpOp::NotEq => projection.visible_lower != ctx.compiled.fold(text),
        CmpOp::Match => ctx
            .compiled
            .matcher(text)
            .is_some_and(|m| m.matches(&projection.visible_lower, &projection.visible)),
        // An invalid regex is retained but deliberately matches nothing.
        CmpOp::Regex => ctx
            .compiled
            .regex(text)
            .is_some_and(|r| r.is_match(&projection.visible)),
        _ => false,
    }
}

/// A comparison on an OPTIONAL text attribute (`task`, `priority`, page
/// `namespace`). Absent makes every comparison false (§3.4).
fn eval_optional_text(op: CmpOp, value: &Value, actual: Option<&str>) -> bool {
    match (op, actual) {
        (CmpOp::IsSet, _) => actual.is_some(),
        (CmpOp::IsNotSet, _) => actual.is_none(),
        (_, None) => false,
        (CmpOp::Eq, Some(actual)) => match value {
            Value::Text { text } => actual.eq_ignore_ascii_case(text),
            _ => false,
        },
        (CmpOp::NotEq, Some(actual)) => match value {
            Value::Text { text } => !actual.eq_ignore_ascii_case(text),
            _ => false,
        },
        (CmpOp::In, Some(actual)) => match value {
            Value::List { items } => items.iter().any(|item| match item {
                Value::Text { text } => actual.eq_ignore_ascii_case(text),
                _ => false,
            }),
            _ => false,
        },
        (CmpOp::NotIn, Some(actual)) => match value {
            Value::List { items } => !items.iter().any(|item| match item {
                Value::Text { text } => actual.eq_ignore_ascii_case(text),
                _ => false,
            }),
            _ => false,
        },
        _ => false,
    }
}

fn eval_page_name(op: CmpOp, value: &Value, page_name: &str) -> bool {
    let key = refs::page_key(page_name);
    match (op, value) {
        (CmpOp::Eq, Value::Text { text }) => key == refs::page_key(text),
        (CmpOp::NotEq, Value::Text { text }) => key != refs::page_key(text),
        (CmpOp::StartsWith, Value::Text { text }) => key.starts_with(&page_prefix_key(text)),
        (CmpOp::Like, Value::Text { text }) => like_matches(&key, &canonical_fold(text)),
        (CmpOp::In, Value::List { items }) => items.iter().any(|item| match item {
            Value::Text { text } => key == refs::page_key(text),
            _ => false,
        }),
        _ => false,
    }
}

/// The page-identity fold applied to a PREFIX rather than to a whole name.
///
/// `refs::page_key` removes one slash at each boundary, because a page's
/// identity does not depend on them. A namespace prefix is precisely "this
/// name, then a boundary", so the trailing `/` carries the whole meaning and
/// has to survive the fold: without this, `(namespace Proj)` lowers to
/// `name starts-with "Proj/"`, folds to `"proj"`, and matches the page `Proj`
/// itself along with every page merely beginning with those letters.
fn page_prefix_key(text: &str) -> String {
    match text.strip_suffix('/') {
        Some(head) => format!("{}/", refs::page_key(head)),
        None => refs::page_key(text),
    }
}

/// A date comparison on a page's journal day ordinal.
fn eval_day(op: CmpOp, value: &Value, day: Option<i64>, today: JournalDate) -> bool {
    match op {
        CmpOp::IsSet => return day.is_some(),
        CmpOp::IsNotSet => return day.is_none(),
        _ => {}
    }
    let Some(day) = day else { return false };
    compare_day(op, value, day, today)
}

/// A planning-date comparison. **Presence IS a projected timestamp (G2):** the
/// walk reads `BlockProjection.scheduled`/`deadline`, which lsdoc fills only
/// from a `Timestamp` inline that starts a source line — so a bare `SCHEDULED:`
/// with no date, or one inside inline code or a fenced block, is no match, and a
/// malformed `<2026-13-45 …>` has presence but no day (E1). This replaces the
/// old raw-text `raw.contains("SCHEDULED:")` scan, which saw both.
fn eval_planning(op: CmpOp, value: &Value, text: Option<&str>, ctx: &EvalCtx) -> bool {
    match op {
        CmpOp::IsSet => return text.is_some(),
        CmpOp::IsNotSet => return text.is_none(),
        _ => {}
    }
    let Some(day) = text.and_then(planning_day) else {
        return false;
    };
    compare_day(op, value, day, ctx.today)
}

fn compare_day(op: CmpOp, value: &Value, day: i64, today: JournalDate) -> bool {
    let resolve = |value: &Value| match value {
        Value::Date { literal } => super::resolve_date_token(literal, today),
        Value::Number { number } => Some(*number as i64),
        _ => None,
    };
    match (op, value) {
        (CmpOp::Between, Value::List { items }) if items.len() == 2 => {
            // OG's `build-between-two-arg` sorts its two resolved bounds, so
            // `(between END START)` is the same inclusive interval.
            let (low, high) = (resolve(&items[0]), resolve(&items[1]));
            let (low, high) = match (low, high) {
                (Some(low), Some(high)) if low > high => (Some(high), Some(low)),
                pair => pair,
            };
            low.is_none_or(|low| day >= low) && high.is_none_or(|high| day <= high)
        }
        (CmpOp::Ge, value) => resolve(value).is_none_or(|bound| day >= bound),
        (CmpOp::Le, value) => resolve(value).is_none_or(|bound| day <= bound),
        (CmpOp::Gt, value) => resolve(value).is_some_and(|bound| day > bound),
        (CmpOp::Lt, value) => resolve(value).is_some_and(|bound| day < bound),
        (CmpOp::Eq, value) => resolve(value).is_some_and(|bound| day == bound),
        (CmpOp::NotEq, value) => resolve(value).is_some_and(|bound| day != bound),
        _ => false,
    }
}

/// The ONE timestamp-text → `yyyymmdd` primitive (D-14, J10), grown from the
/// walk's old `parse_angle_date`: it consumes the BRACKETLESS facet text exactly
/// as `doc::planning_dates` stores it on `BlockProjection::scheduled` and
/// `BlockProjection::deadline`, and an angle-bracketed caller strips the `<`
/// first.
///
/// **Calendar-validated (C5).** The old parser accepted `2026-13-45` because it
/// only read three integers. The month/day are now checked against the existing
/// `date.rs` `is_leap`/`days_in_month` (reused, never re-derived), so a
/// malformed timestamp has presence and no day.
pub(crate) fn planning_day(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text.strip_prefix('<').unwrap_or(text);
    let end = text.find([' ', '>']).unwrap_or(text.len());
    let mut parts = text[..end].split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) {
        return None;
    }
    let year_i32 = i32::try_from(year).ok()?;
    let month_u32 = u32::try_from(month).ok()?;
    if day < 1 || day > i64::from(crate::date::days_in_month(year_i32, month_u32)) {
        return None;
    }
    Some(year * 10000 + month * 100 + day)
}

/// SQL `LIKE` over an already-folded haystack: `%` matches any run, `_` any one
/// character, and `\` escapes either (the lowering emits `LIKE ? ESCAPE '\'`).
pub(crate) fn like_matches(haystack: &str, pattern: &str) -> bool {
    #[derive(Debug)]
    enum Part {
        Literal(String),
        Any,
        One,
    }
    let mut parts: Vec<Part> = Vec::new();
    let mut literal = String::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(next) = chars.next() {
                    literal.push(next);
                }
            }
            '%' | '_' => {
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(if ch == '%' { Part::Any } else { Part::One });
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }
    let haystack: Vec<char> = haystack.chars().collect();
    fn matches(parts: &[Part], haystack: &[char], at: usize) -> bool {
        match parts.first() {
            None => at == haystack.len(),
            Some(Part::One) => at < haystack.len() && matches(&parts[1..], haystack, at + 1),
            Some(Part::Any) => {
                (at..=haystack.len()).any(|next| matches(&parts[1..], haystack, next))
            }
            Some(Part::Literal(text)) => {
                let literal: Vec<char> = text.chars().collect();
                at + literal.len() <= haystack.len()
                    && haystack[at..at + literal.len()] == literal[..]
                    && matches(&parts[1..], haystack, at + literal.len())
            }
        }
    }
    matches(&parts, &haystack, 0)
}

/// Does this page row satisfy a `@page`-anchored query? Used by the page-anchored
/// walk, which reads the page index and loads no document (K16).
pub(crate) fn page_row_matches(
    query: &Filter,
    name: &str,
    kind: PageKind,
    journal: Option<i64>,
    page_props: &[(String, String)],
    today: JournalDate,
    compiled: &CompiledLeaves,
) -> bool {
    let ctx = EvalCtx {
        journal,
        is_journal: kind == PageKind::Journal,
        page_name: name,
        page_props,
        today,
        compiled,
    };
    eval_page(query, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_day_accepts_the_bracketless_projection_text_and_the_angle_form() {
        assert_eq!(planning_day("2026-07-29 Wed"), Some(20260729));
        assert_eq!(planning_day("<2026-07-29 Wed>"), Some(20260729));
        assert_eq!(planning_day("2026-07-29"), Some(20260729));
    }

    #[test]
    fn planning_day_validates_the_calendar_so_a_malformed_date_has_no_day() {
        // C5: the old `parse_angle_date` answered 20261345 for the first of these.
        assert_eq!(planning_day("2026-13-45"), None);
        assert_eq!(planning_day("2026-02-30"), None);
        assert_eq!(planning_day("2023-02-29"), None);
        assert_eq!(planning_day("2026-04-31"), None);
        assert_eq!(planning_day("2024-02-29"), Some(20240229));
    }

    #[test]
    fn like_matches_wildcards_escapes_and_anchors() {
        assert!(like_matches("hello world", "%world%"));
        assert!(like_matches("hello world", "hello%"));
        assert!(!like_matches("hello world", "world%"));
        assert!(like_matches("a_b", "a\\_b"));
        assert!(!like_matches("axb", "a\\_b"));
        assert!(like_matches("axb", "a_b"));
        assert!(like_matches("100%", "%\\%"));
        assert!(like_matches("abc", "abc"));
        assert!(!like_matches("abcd", "abc"));
    }
}
