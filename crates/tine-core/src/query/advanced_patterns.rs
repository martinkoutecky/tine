//! The advanced (datalog) query's decoded `:where` clauses and the bounded
//! DataScript attribute patterns lowered from them (GH #542):
//! `[?b :block/marker "TODO"]`, value variables narrowed by a predicate, the
//! block's journal page, and `(not [..])` of one of those — and the clause
//! heads (`task`, `between`, ...) with their typed `:inputs`, lowered by
//! [`lower_advanced`], the ONE advanced → IR lowerer (master's QA1 join-free
//! subset; transcribed from master `query.rs` 5dfc84503).

use super::date_token::{resolve_date_token, DateToken};
use super::ir::{Anchor, Attr, CmpOp, Filter, Quant, Query, Rel, Source, Value};
use super::{og, query_nesting_within_limit, query_source_within_limit, QUERY_NESTING_MAX};
use crate::date::JournalDate;
use crate::edn::Edn;

#[path = "advanced_joins.rs"]
mod joins;
#[path = "advanced_ref_journals.rs"]
mod ref_journals;
use joins::{lower_current_page_patterns, lower_date_disjunctions, lower_named_page_query};

fn list(value: &Edn) -> Option<&[Edn]> {
    match value {
        Edn::List(items) => Some(items),
        _ => None,
    }
}

fn vector(value: &Edn) -> Option<&[Edn]> {
    match value {
        Edn::Vec(items) => Some(items),
        _ => None,
    }
}

fn variable(value: &Edn) -> Option<&str> {
    value.symbol().filter(|name| name.starts_with('?'))
}

fn advanced_find(items: &[Edn]) -> Option<(&str, bool)> {
    let at = items
        .iter()
        .position(|value| value.keyword() == Some("find"))?;
    let first = items.get(at + 1)?;
    if !matches!(items.get(at + 2)?.keyword(), Some("in" | "where")) {
        return None;
    }
    if let Some([head, var, projection]) = list(first) {
        let Some([all]) = vector(projection) else {
            return None;
        };
        (head.symbol() == Some("pull") && all.symbol() == Some("*")).then_some(())?;
        Some((variable(var)?, true))
    } else {
        Some((variable(first)?, false))
    }
}

/// Open conjunctions and one-branch disjunctions without spending a frame
/// per nested wrapper.
/// The clauses of `:where` with wrappers opened, and for each whether it came
/// out of a one-branch `or-join`. `and` and a one-branch `or` hide no variable;
/// an `or-join` does, and opening it erases that scope.
fn flatten_single_branch_groups(groups: &[Edn]) -> (Vec<&Edn>, Vec<bool>) {
    let (mut result, mut scoped) = (Vec::new(), Vec::new());
    let mut pending: Vec<_> = groups.iter().rev().map(|group| (group, false)).collect();
    while let Some((group, inside)) = pending.pop() {
        match list(group) {
            Some([head, rest @ ..]) if head.symbol() == Some("and") => {
                pending.extend(rest.iter().rev().map(|group| (group, inside)));
            }
            Some([head, branch]) if head.symbol() == Some("or") => pending.push((branch, inside)),
            Some([head, vars, branch])
                if head.symbol() == Some("or-join") && vector(vars).is_some() =>
            {
                pending.push((branch, true));
            }
            _ => {
                result.push(group);
                scoped.push(inside);
            }
        }
    }
    (result, scoped)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvTarget {
    Block,
    Page,
}

/// A DataScript attribute Tine indexes, on the result block or on its page.
fn advanced_attribute(target: AdvTarget, attribute: &str) -> Option<(Attr, &'static str)> {
    match (target, attribute) {
        (AdvTarget::Block, "block/marker") => Some((Attr::Task, "marker")),
        (AdvTarget::Block, "block/priority") => Some((Attr::Priority, "priority")),
        (AdvTarget::Block, "block/scheduled") => Some((Attr::Scheduled, "scheduled")),
        (AdvTarget::Block, "block/deadline") => Some((Attr::Deadline, "deadline")),
        (AdvTarget::Page, "block/journal?") => Some((Attr::Journal, "journal-page")),
        (AdvTarget::Page, "block/journal-day") => Some((Attr::Day, "journal-day")),
        _ => None,
    }
}

/// A literal operand in the type DataScript stores for `attr`: marker and
/// priority are strings, scheduled/deadline/journal-day are yyyymmdd
/// integers, `journal?` a boolean. A mismatched literal never matches in
/// Logseq either, so it is not lowered to something that might.
fn advanced_literal(
    attr: Attr,
    operand: &Edn,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> Option<Value> {
    match attr {
        Attr::Task | Attr::Priority => Some(Value::text(operand.text()?)),
        Attr::Scheduled | Attr::Deadline | Attr::Day => {
            let number = match operand.symbol().and_then(|name| inputs.get(name)) {
                Some(AdvancedInput::Date(value)) => *value as f64,
                Some(AdvancedInput::Page(_)) => return None,
                None => operand.as_f64().filter(|number| number.is_finite())?,
            };
            Some(Value::Number { number })
        }
        Attr::Journal => match operand {
            Edn::Bool(value) => Some(Value::Bool { value: *value }),
            _ => None,
        },
        _ => None,
    }
}

fn advanced_on_target(target: AdvTarget, filter: Filter) -> Filter {
    match target {
        AdvTarget::Block => filter,
        AdvTarget::Page => Filter::rel(Rel::Page, Quant::Any, filter),
    }
}

fn advanced_var_uses(value: &Edn, name: &str) -> usize {
    match value {
        Edn::Symbol(symbol) => usize::from(symbol == name),
        Edn::Vec(items) | Edn::List(items) | Edn::Set(items) => {
            items.iter().map(|item| advanced_var_uses(item, name)).sum()
        }
        Edn::Map(entries) => entries
            .iter()
            .map(|(k, v)| advanced_var_uses(k, name) + advanced_var_uses(v, name))
            .sum(),
        _ => 0,
    }
}

fn groups_use(groups: &[&Edn], name: &str) -> usize {
    groups
        .iter()
        .map(|group| advanced_var_uses(group, name))
        .sum()
}

fn triple(group: &Edn) -> Option<(&str, &str, Option<&Edn>)> {
    let items = vector(group)?;
    match items {
        [subject, attribute] => Some((variable(subject)?, attribute.keyword()?, None)),
        [subject, attribute, object] => {
            Some((variable(subject)?, attribute.keyword()?, Some(object)))
        }
        _ => None,
    }
}

/// Lower an indexed attribute pattern and the unary predicates over its bound
/// value. A variable shared with another join leaves the clauses unsupported.
pub(super) fn lower_attribute_patterns(
    groups: &[&Edn],
    taken: &std::collections::HashSet<usize>,
    find_var: Option<&str>,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    let Some(block) = find_var else {
        return (lowered, consumed);
    };
    let active: Vec<usize> = (0..groups.len()).filter(|i| !taken.contains(i)).collect();

    let (ref_filters, ref_patterns) =
        ref_journals::lower_ref_journal_patterns(groups, taken, block, inputs);
    lowered.extend(ref_filters);
    consumed.extend(ref_patterns);

    let mut pages = std::collections::HashSet::new();
    for &index in &active {
        if let Some([call]) = vector(groups[index]) {
            if let Some([head, database, entity, key]) = list(call) {
                if head.symbol() == Some("missing?")
                    && database.symbol() == Some("$")
                    && variable(entity) == Some(block)
                {
                    if let Some((attr, _)) = key
                        .keyword()
                        .and_then(|key| advanced_attribute(AdvTarget::Block, key))
                    {
                        lowered.insert(
                            index,
                            (
                                Filter::not(Filter::attr(attr, CmpOp::IsSet, Value::None)),
                                "missing?",
                            ),
                        );
                    }
                }
            }
        }
        if let Some((subject, "block/page", Some(object))) = triple(groups[index]) {
            if let Some(page) =
                variable(object).filter(|page| *page != block && !inputs.contains_key(*page))
            {
                if subject == block {
                    pages.insert(page);
                    consumed.insert(index);
                }
            }
        }
    }
    let target_of = |subject: &str| {
        if subject == block {
            Some(AdvTarget::Block)
        } else if pages.contains(subject) {
            Some(AdvTarget::Page)
        } else {
            None
        }
    };

    let mut bindings = std::collections::HashMap::new();
    for &index in &active {
        if consumed.contains(&index) {
            continue;
        }
        let Some((subject, key, object)) = triple(groups[index]) else {
            continue;
        };
        let Some(target) = target_of(subject) else {
            continue;
        };
        let Some((attr, label)) = advanced_attribute(target, key) else {
            continue;
        };
        if object.is_none_or(|object| object.symbol() == Some("_")) {
            if attr != Attr::Journal {
                lowered.insert(
                    index,
                    (
                        advanced_on_target(target, Filter::attr(attr, CmpOp::IsSet, Value::None)),
                        label,
                    ),
                );
            }
        } else if let Some(object) = object {
            if let Some(var) = variable(object).filter(|var| !inputs.contains_key(*var)) {
                if attr != Attr::Journal {
                    bindings.entry(var).or_insert((index, target, attr, label));
                }
            } else if let Some(value) = advanced_literal(attr, object, inputs) {
                lowered.insert(
                    index,
                    (
                        advanced_on_target(target, Filter::attr(attr, CmpOp::Eq, value)),
                        label,
                    ),
                );
            }
        }
    }

    let negation = |group: &Edn| {
        list(group).is_some_and(|items| items.first().and_then(Edn::symbol) == Some("not"))
    };
    let mut predicates: std::collections::HashMap<&str, Vec<(usize, Filter)>> =
        std::collections::HashMap::new();
    for &index in &active {
        let group = groups[index];
        let negated = match list(group) {
            Some([head, clause]) if head.symbol() == Some("not") => Some(clause),
            _ => None,
        };
        let Some([call]) = vector(negated.unwrap_or(group)) else {
            continue;
        };
        let Some([head, left, right]) = list(call) else {
            continue;
        };
        let binding = |value: &Edn| variable(value).is_some_and(|name| bindings.contains_key(name));
        let lowered_predicate = (|| {
            let symbol = head.symbol()?;
            if symbol == "contains?" {
                let name = variable(right)?;
                let (_, _, attr, _) = bindings.get(name)?;
                let Edn::Set(members) = left else {
                    return None;
                };
                let items = members
                    .iter()
                    .map(|item| advanced_literal(*attr, item, inputs))
                    .collect::<Option<Vec<_>>>()?;
                return Some((name, Filter::attr(*attr, CmpOp::In, Value::List { items })));
            }
            let (var, operand, flipped) = match (binding(left), binding(right)) {
                (true, false) => (variable(left)?, right, false),
                (false, true) => (variable(right)?, left, true),
                _ => return None,
            };
            let (_, _, attr, _) = bindings.get(var)?;
            let op = match (symbol, flipped) {
                ("=", _) => CmpOp::Eq,
                ("not=", _) => CmpOp::NotEq,
                ("<", false) | (">", true) => CmpOp::Lt,
                ("<=", false) | (">=", true) => CmpOp::Le,
                (">", false) | ("<", true) => CmpOp::Gt,
                (">=", false) | ("<=", true) => CmpOp::Ge,
                _ => return None,
            };
            if !matches!(attr, Attr::Scheduled | Attr::Deadline | Attr::Day)
                && !matches!(op, CmpOp::Eq | CmpOp::NotEq)
            {
                return None;
            }
            let value = advanced_literal(*attr, operand, inputs)?;
            Some((var, Filter::attr(*attr, op, value)))
        })();
        if let Some((var, filter)) = lowered_predicate {
            predicates.entry(var).or_default().push((
                index,
                if negated.is_some() {
                    Filter::not(filter)
                } else {
                    filter
                },
            ));
        }
    }

    for (var, (index, target, attr, label)) in bindings {
        let narrowing = predicates.remove(var).unwrap_or_default();
        if groups_use(groups, var) != 1 + narrowing.len() {
            continue;
        }
        lowered.insert(
            index,
            (
                advanced_on_target(target, Filter::attr(attr, CmpOp::IsSet, Value::None)),
                label,
            ),
        );
        for (predicate, filter) in narrowing {
            lowered.insert(predicate, (advanced_on_target(target, filter), "predicate"));
        }
    }

    // A variable local to this negation expresses attribute absence.
    for &index in &active {
        let Some([head, clause]) = list(groups[index]) else {
            continue;
        };
        if head.symbol() != Some("not") {
            continue;
        }
        let Some((subject, key, object)) = triple(clause) else {
            continue;
        };
        let Some(target) = target_of(subject) else {
            continue;
        };
        let Some((attr, _)) = advanced_attribute(target, key) else {
            continue;
        };
        let inner = if let Some(object) = object.filter(|object| object.symbol() != Some("_")) {
            if let Some(var) = variable(object).filter(|var| !inputs.contains_key(*var)) {
                let used_outside = groups.iter().enumerate().any(|(other, group)| {
                    other != index && !negation(group) && advanced_var_uses(group, var) != 0
                });
                if attr == Attr::Journal || used_outside || advanced_var_uses(clause, var) != 1 {
                    continue;
                }
                Filter::attr(attr, CmpOp::IsSet, Value::None)
            } else {
                let Some(value) = advanced_literal(attr, object, inputs) else {
                    continue;
                };
                Filter::attr(attr, CmpOp::Eq, value)
            }
        } else {
            Filter::attr(attr, CmpOp::IsSet, Value::None)
        };
        lowered.insert(
            index,
            (Filter::not(advanced_on_target(target, inner)), "not"),
        );
    }
    (lowered, consumed)
}

#[cfg(test)]
pub(crate) fn advanced_pred(
    query_src: &str,
    current_page: Option<&str>,
    today: JournalDate,
) -> (Option<Query>, Vec<String>, Vec<String>) {
    let lowered = lower_advanced(query_src, current_page, &LoweringDay::new(today));
    (lowered.query, lowered.ran, lowered.ignored)
}

#[derive(Default)]
pub(super) struct AdvancedLowering {
    pub(super) query: Option<Query>,
    pub(super) ran: Vec<String>,
    pub(super) ignored: Vec<String>,
    pub(super) sort: Option<super::advanced_sort::AdvancedResultSort>,
    pub(super) keeps_block_children: bool,
    pub(super) suggestion: Option<String>,
}

pub(super) fn lower_advanced(
    source: &str,
    current_page: Option<&str>,
    day: &LoweringDay,
) -> AdvancedLowering {
    let refusal = |reason: &str| AdvancedLowering {
        ignored: vec![reason.into()],
        ..Default::default()
    };
    if !query_source_within_limit(source) {
        return refusal("query-too-large");
    }
    if !query_nesting_within_limit(source) {
        return refusal("query-nesting-too-deep");
    }
    let Some(declaration) = crate::query_edn::advanced(source) else {
        let reason = if crate::query_edn::declares_option(source, ":result-transform") {
            "result-transform"
        } else {
            "query-vector"
        };
        return refusal(reason);
    };
    let sort = declaration
        .result_transform
        .as_ref()
        .and_then(super::advanced_sort::result_sort_form);
    let unsupported_sort = declaration.has_result_transform && sort.is_none();
    let (query, ran, ignored) =
        lower_declaration(source, &declaration, current_page, day, unsupported_sort);
    let suggestion = if ignored == ["result-transform"] {
        lower_declaration(source, &declaration, current_page, day, false)
            .0
            .map(|query| super::print::print_tql(&query))
    } else {
        None
    };
    AdvancedLowering {
        query,
        ran,
        ignored,
        sort,
        suggestion,
        keeps_block_children: declaration.keeps_block_children,
    }
}

/// Lower one already read declaration into the supported datalog subset.
fn lower_declaration(
    query_src: &str,
    declaration: &crate::query_edn::Advanced,
    current_page: Option<&str>,
    today: &LoweringDay,
    unsupported_sort: bool,
) -> (Option<Query>, Vec<String>, Vec<String>) {
    let mut ran = Vec::new();
    let mut ignored = Vec::new();
    let Some((find, pulls_entity)) = advanced_find(&declaration.vector) else {
        return (None, ran, vec!["find projection".into()]);
    };
    let input_bindings = declaration
        .vector
        .iter()
        .skip_while(|value| value.keyword() != Some("in"))
        .skip(1)
        .take_while(|value| value.keyword().is_none())
        .filter(|value| value.symbol() != Some("$"))
        .map(variable)
        .collect::<Option<Vec<_>>>();
    let Some(input_bindings) = input_bindings else {
        return (
            None,
            ran,
            vec!["input binding (rules or collection)".into()],
        );
    };
    if declaration.inputs.len() != input_bindings.len() {
        return (None, ran, vec!["input count".into()]);
    }
    let inputs = resolve_inputs(&input_bindings, &declaration.inputs, current_page, today);
    let where_at = declaration
        .vector
        .iter()
        .position(|value| value.keyword() == Some("where"));
    let authored = where_at.map_or(&[][..], |index| &declaration.vector[index + 1..]);
    let (groups, scoped) = flatten_single_branch_groups(authored);
    let anchor = if joins::page_property_result(&groups, find) {
        Anchor::Page
    } else {
        Anchor::Block
    };
    if !joins::rules_target_result(&groups, find, anchor) {
        return (None, ran, vec!["rule result binding".into()]);
    }
    // Authored rules may replace even familiar rule names. Never execute or
    // silently substitute our built-ins for a graph's rule definitions.
    if declaration.has_rules {
        ignored.push("rules".into());
    }
    if unsupported_sort {
        ignored.push("result-transform".into());
    }
    if !pulls_entity {
        if let Some(query) = lower_named_page_query(query_src, find, &groups, &inputs) {
            return if ignored.is_empty() {
                (Some(query), vec!["named-pages".into()], ignored)
            } else {
                (None, Vec::new(), ignored)
            };
        }
    }
    // Existing attribute/property-map lowerers describe blocks and their page.
    // Page-entity queries only lower their supported rules; unrelated patterns
    // stay visible as unsupported instead of being interpreted as block facts.
    let block_groups = if anchor == Anchor::Block {
        &groups[..]
    } else {
        &[]
    };
    let (lowered_current_pages, current_page_patterns) =
        lower_current_page_patterns(block_groups, &inputs, Some(find));
    // Property-map/get patterns stay unsupported: neither a page hop nor OG's
    // entity-level truthiness follows from a property-map binding (L7s1-02).
    let consumed_patterns = current_page_patterns;
    let taken = consumed_patterns
        .iter()
        .copied()
        .chain(lowered_current_pages.keys().copied())
        .collect::<std::collections::HashSet<_>>();
    let (mut lowered_attributes, mut attribute_patterns) =
        lower_attribute_patterns(block_groups, &taken, Some(find), &inputs);
    // Docs example 14 and GH #1017: every alternative binds the SAME date
    // variable. Move its predicates into each branch before OR, retaining
    // existential semantics when a block has two dates (one outside, one inside).
    let (date_or, date_or_patterns) =
        lower_date_disjunctions(&scoped, block_groups, &taken, Some(find), &inputs);
    lowered_attributes.extend(date_or);
    attribute_patterns.extend(date_or_patterns);
    let preds: Vec<Filter> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            if let Some((pred, label)) = lowered_current_pages.get(&index) {
                ran.push((*label).into());
                return Some(pred.clone());
            }
            if let Some((pred, label)) = lowered_attributes.get(&index) {
                ran.push((*label).into());
                return Some(pred.clone());
            }
            if consumed_patterns.contains(&index) || attribute_patterns.contains(&index) {
                return None;
            }
            let ignored_before = ignored.len();
            let lowered = parse_adv_group(group, &inputs, today, &mut ran, &mut ignored, anchor, 0);
            if lowered.is_none() && ignored.len() == ignored_before {
                // Every clause that does not lower is named: refusing is the
                // contract (below), so a silent drop would be a silent widening.
                ignored.push("clause".into());
            }
            lowered
        })
        .collect();
    // An `:in` variable the `:inputs` do not bind to a value Tine resolves
    // (`:current-block`, `:parent-block`, a `:current-page` with no current
    // page, ...) leaves every clause that reads it unevaluable.
    for variable in input_bindings {
        if !inputs.contains_key(variable) && groups_use(&groups, variable) > 0 {
            ignored.push(format!("input {variable}"));
        }
    }
    // GH #542: a `:result-transform` is a Clojure function (ADR 0042 keeps
    // scripting out). It reorders or reshapes the answer, so say it did not run.
    if ignored.iter().any(|item| item == "query-nesting-too-deep") {
        return (None, Vec::new(), ignored);
    }
    // OG executes the whole DataScript query. A clause Tine cannot lower can
    // only be dropped by answering a different (broader or narrower) question,
    // so the query is refused as a whole and the report names what it could not
    // lower (Martin, 2026-10-03). `ran` is empty: nothing ran.
    if !ignored.is_empty() {
        return (None, Vec::new(), ignored);
    }
    if preds.is_empty() {
        return (None, ran, ignored);
    }
    let filter = if preds.len() == 1 {
        preds.into_iter().next().expect("one")
    } else {
        Filter::and(preds)
    };
    // The advanced context and report survive verbatim (M5): `current_page` is
    // already folded into the lowered clauses above, and the caller keeps
    // `ran`/`ignored`/`supported`.
    let query = Query::new(
        anchor,
        filter,
        Source::Advanced {
            original: query_src.to_string(),
            og_options: String::new(),
        },
    );
    (Some(query), ran, ignored)
}

fn page_operand(
    value: &Edn,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> Option<String> {
    match variable(value).and_then(|name| inputs.get(name)) {
        Some(AdvancedInput::Page(name)) => Some(name.clone()),
        _ => value.text().map(str::to_owned),
    }
}

/// Map one `:where` group to a `Pred` (or None → ignored). Recurses for and/or/not.
fn parse_adv_group(
    group: &Edn,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: &LoweringDay,
    ran: &mut Vec<String>,
    ignored: &mut Vec<String>,
    anchor: Anchor,
    depth: usize,
) -> Option<Filter> {
    if depth > QUERY_NESTING_MAX {
        ignored.push("query-nesting-too-deep".into());
        return None;
    }
    let Some([head, args @ ..]) = list(group) else {
        ignored.push("pattern".into()); // `[?e :a ?v]` joins, etc. — not in the subset
        return None;
    };
    let head = head.symbol()?.to_ascii_lowercase();
    match head.as_str() {
        "and" | "or" | "not" => {
            let ignored_before = ignored.len();
            let mut incomplete = false;
            let kids: Vec<Filter> = args
                .iter()
                .filter_map(|group| {
                    let filter =
                        parse_adv_group(group, inputs, today, ran, ignored, anchor, depth + 1);
                    incomplete |= filter.is_none();
                    filter
                })
                .collect();
            // An unsupported child prevents the entire group from lowering.
            if incomplete {
                if head != "and" || ignored.len() == ignored_before {
                    ignored.push(head);
                }
                return None;
            }
            if kids.is_empty() {
                None
            } else if head == "not" {
                // `(not A B)` excludes blocks matching A AND B.
                Some(Filter::not(if kids.len() == 1 {
                    kids.into_iter().next().expect("one")
                } else {
                    Filter::and(kids)
                }))
            } else if head == "or" {
                Some(Filter::or(kids))
            } else {
                Some(Filter::and(kids))
            }
        }
        "task" | "todo" => {
            ran.push("task".into());
            Some(Filter::attr(
                Attr::Task,
                CmpOp::In,
                adv_text_list(string_operands(args.get(1..)?)?),
            ))
        }
        "priority" => {
            ran.push("priority".into());
            Some(Filter::attr(
                Attr::Priority,
                CmpOp::In,
                adv_text_list(string_operands(args.get(1..)?)?),
            ))
        }
        "page-ref" => {
            let [_, name] = args else {
                return None;
            };
            page_operand(name, inputs).map(|name| {
                ran.push("page-ref".into());
                // OG matches `:block/name` literally: a literal with upper case
                // matches no page (a different question from page identity,
                // declared under census Q1). A lower-case name lowers to the
                // one reference leaf and so matches the alias group
                // (D-20261019-Q8; QL-1: one rule, see `advanced_joins`).
                if name != name.to_lowercase() {
                    Filter::False
                } else {
                    Filter::page_ref(name)
                }
            })
        }
        "property" | "page-property" => {
            let (key, val) = match args {
                [_, key] => (key.keyword()?, None),
                [_, key, val] => (key.keyword()?, Some(val.text()?.to_owned())),
                _ => return None,
            };
            ran.push(head.clone());
            let leaf = og::property_leaf(og::normalize_prop_key(key), val);
            Some(if head == "property" || anchor == Anchor::Page {
                leaf
            } else {
                Filter::rel(Rel::Page, Quant::Any, leaf)
            })
        }
        "page" => args.get(1)?.text().map(|n| {
            ran.push("page".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::Eq, Value::text(n)),
            )
        }),
        "namespace" => args.get(1)?.text().map(|n| {
            ran.push("namespace".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text(format!("{n}/"))),
            )
        }),
        "page-tags" | "tags" => {
            let ts = string_operands(args.get(1..)?)?;
            if ts.is_empty() {
                ignored.push(head.clone());
                None
            } else {
                ran.push("page-tags".into());
                Some(Filter::rel(
                    Rel::Page,
                    Quant::Any,
                    Filter::rel(
                        Rel::Props,
                        Quant::Any,
                        Filter::and(vec![
                            Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                            Filter::attr(Attr::Value, CmpOp::In, adv_text_list(ts)),
                        ]),
                    ),
                ))
            }
        }
        "scheduled" => {
            ran.push("scheduled".into());
            Some(Filter::attr(Attr::Scheduled, CmpOp::IsSet, Value::None))
        }
        "deadline" => {
            ran.push("deadline".into());
            Some(Filter::attr(Attr::Deadline, CmpOp::IsSet, Value::None))
        }
        "journal" => {
            ran.push("journal".into());
            Some(Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Journal, CmpOp::Eq, Value::Bool { value: true }),
            ))
        }
        "between" => {
            // (between [FIELD] ?b ?start ?end): the last two args are always the
            // bounds. An optional field keyword (journal|scheduled|deadline) may
            // appear among the earlier args — matching the simple parser. The bare
            // `(between ?b lo hi)` keeps OG's journal-day semantics.
            let [prefix @ .., lo, hi] = args else {
                ignored.push("between".into());
                return None;
            };
            let attr = prefix
                .iter()
                .filter_map(|value| value.keyword().or_else(|| value.symbol()))
                .find_map(|a| match a.to_ascii_lowercase().as_str() {
                    "scheduled" => Some(Attr::Scheduled),
                    "deadline" => Some(Attr::Deadline),
                    "journal" => Some(Attr::Day),
                    _ => None,
                })
                .unwrap_or(Attr::Day);
            let lo = adv_bound(lo, inputs, today);
            let hi = adv_bound(hi, inputs, today);
            if lo.is_none() || hi.is_none() {
                ignored.push("between".into());
                return None;
            }
            ran.push("between".into());
            // The advanced dialect resolves its bounds eagerly: `:inputs` may
            // bind a bound to an already-resolved ordinal, and an advanced query
            // is never re-printed as OG DSL, so the IR carries the ordinals.
            // The advanced `between` RULE compares in authored order (OG
            // db/rules.cljc). The simple DSL's builder sorts its two bounds;
            // do not borrow that reversal for this Datalog rule.
            let range = if lo > hi {
                Filter::False
            } else {
                adv_range(attr, lo, hi)
            };
            Some(if attr == Attr::Day {
                Filter::rel(Rel::Page, Quant::Any, range)
            } else {
                range
            })
        }
        other => {
            if !other.is_empty() {
                ignored.push(other.to_string());
            }
            None
        }
    }
}

fn string_operands(items: &[Edn]) -> Option<Vec<String>> {
    let mut values = Vec::new();
    for item in items {
        match item {
            Edn::Vec(items) | Edn::List(items) | Edn::Set(items) => {
                for member in items {
                    values.push(member.text()?.to_owned());
                }
            }
            _ => values.push(item.text()?.to_owned()),
        }
    }
    Some(values)
}

/// Resolve an advanced date bound from decoded EDN or a bound input.
fn adv_bound(
    operand: &Edn,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: &LoweringDay,
) -> Option<i64> {
    if let Some(var) = variable(operand) {
        return match inputs.get(var) {
            Some(AdvancedInput::Date(value)) => Some(*value),
            _ => None,
        };
    }
    operand.exact_integer().or_else(|| {
        today.resolve(
            operand
                .text()
                .or_else(|| operand.keyword())
                .or_else(|| operand.symbol())?,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AdvancedInput {
    Date(i64),
    Page(String),
}

/// Build a typed positional input map by zipping `:in $ ?a ?b …` with
/// `:inputs [ … ]`. Dates stay numeric; Logseq's typed `:current-page` keyword
/// receives the caller's focused page. Unknown keywords remain unbound.
fn resolve_inputs(
    vars: &[&str],
    vals: &[crate::query_edn::InputItem],
    current_page: Option<&str>,
    today: &LoweringDay,
) -> std::collections::HashMap<String, AdvancedInput> {
    let mut map = std::collections::HashMap::new();
    for (&v, val) in vars.iter().zip(vals) {
        let crate::query_edn::InputItem::Other(val) = val else {
            if let Some(page) = current_page.map(str::trim).filter(|page| !page.is_empty()) {
                map.insert(v.to_string(), AdvancedInput::Page(page.to_lowercase()));
            }
            continue;
        };
        if let Some(text) = val.text() {
            // Declared different question (I-12, census Q1): OG lowercases a
            // `[[page]]` input into a `:block/name` literal; the literal still
            // names its page through `page_key` where it is compared, and
            // lowercase-then-page_key equals page_key (cm keys harness).
            let text = match text.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
                Some(page) => page.to_lowercase(),
                None => text.to_owned(),
            };
            map.insert(v.to_string(), AdvancedInput::Page(text));
        } else if let Some(number) = val.exact_integer() {
            // Numeric inputs are DataScript numbers already, NOT epoch days.
            map.insert(v.to_string(), AdvancedInput::Date(number));
        } else if let Some(keyword) = val.keyword() {
            let legacy = keyword
                .strip_suffix("d-after")
                .map(|n| (n, '+'))
                .or_else(|| keyword.strip_suffix("d-before").map(|n| (n, '-')))
                .or_else(|| keyword.strip_suffix('d').map(|n| (n, '-')))
                .filter(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            let token =
                legacy.map_or_else(|| keyword.to_string(), |(n, sign)| format!("{sign}{n}d"));
            // Keywords are case-sensitive in OG. Only its date-input grammar
            // resolves here: e.g. :now / :Today / :2026-10-08 stay unbound.
            let admitted = matches!(keyword, "today" | "yesterday" | "tomorrow")
                || legacy.is_some()
                || (keyword.starts_with(['+', '-'])
                    && matches!(DateToken::parse(keyword), Some(DateToken::Relative { .. })));
            if admitted {
                if let Some(ord) = today.resolve(&token) {
                    map.insert(v.to_string(), AdvancedInput::Date(ord));
                }
            }
        } else if let Some(ord) = val.symbol().and_then(|text| today.resolve(text)) {
            // Preserve Tine's previously supported bare date-stem extension.
            map.insert(v.to_string(), AdvancedInput::Date(ord));
        }
    }
    map
}

/// A `(between …)` range whose bounds the advanced dialect already resolved to
/// `yyyymmdd` ordinals.
pub(super) fn adv_range(attr: Attr, low: Option<i64>, high: Option<i64>) -> Filter {
    let number = |value: i64| Value::Number {
        number: value as f64,
    };
    match (low, high) {
        (Some(low), Some(high)) => {
            // OG's `build-between-two-arg` sorts its two bounds, so
            // `(between END START)` is the same inclusive interval.
            let (low, high) = if low > high { (high, low) } else { (low, high) };
            Filter::attr(
                attr,
                CmpOp::Between,
                Value::List {
                    items: vec![number(low), number(high)],
                },
            )
        }
        (Some(low), None) => Filter::attr(attr, CmpOp::Ge, number(low)),
        (None, Some(high)) => Filter::attr(attr, CmpOp::Le, number(high)),
        (None, None) => Filter::attr(attr, CmpOp::IsSet, Value::None),
    }
}

fn adv_text_list(values: Vec<String>) -> Value {
    Value::List {
        items: values.into_iter().map(Value::text).collect(),
    }
}

/// The execution day as the advanced lowering reads it. The lowering bakes
/// resolved ordinals into the IR, so whether the answer depends on the day is
/// recorded here, at the one place the lowering resolves a token, rather than
/// recovered from the lowered tree (card hB3Lv8).
pub(super) struct LoweringDay {
    day: JournalDate,
    read: std::cell::Cell<bool>,
}

impl LoweringDay {
    pub(super) fn new(day: JournalDate) -> LoweringDay {
        LoweringDay {
            day,
            read: std::cell::Cell::new(false),
        }
    }

    /// [`resolve_date_token`], noting a token that reads the day.
    fn resolve(&self, tok: &str) -> Option<i64> {
        let ord = resolve_date_token(tok, self.day)?;
        if DateToken::parse(tok).is_some_and(DateToken::reads_today) {
            self.read.set(true);
        }
        Some(ord)
    }

    /// Whether any resolved token read the day.
    pub(super) fn was_read(&self) -> bool {
        self.read.get()
    }
}
