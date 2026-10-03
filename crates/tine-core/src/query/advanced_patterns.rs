//! The advanced (datalog) query's `:where` clauses as text: the clause
//! scanner, and (GH #542) the DataScript attribute patterns lowered from it —
//! `[?b :block/marker "TODO"]`, value variables narrowed by a predicate, the
//! block's journal page, and `(not [..])` of one of those — and the clause
//! heads (`task`, `between`, ...) with their typed `:inputs`, lowered by
//! [`advanced_pred`], the ONE advanced → IR lowerer (master's QA1 join-free
//! subset; transcribed from master `query.rs` 5dfc84503).

use super::ir::{Anchor, Attr, CmpOp, Filter, Quant, Query, Rel, Source, Value};
use super::{og, query_nesting_within_limit, query_source_within_limit, QUERY_NESTING_MAX};
use crate::date::JournalDate;

/// Collect balanced `(...)`/`[...]` groups at the top level of `s` (string-aware),
/// stopping at the first top-level *closing* bracket (so scanning after `:where`
/// halts at the find-vector's `]` rather than swallowing `:inputs`).
pub(super) fn scan_groups(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i] as char;
        if c == ')' || c == ']' || c == '}' {
            break;
        }
        // EDN/DataScript line comment (`; …` to end of line) — skip it so example
        // clauses written inside a `;;` hint are NOT parsed as real groups.
        if c == ';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // A top-level string is one literal, as `query_nesting_within_limit`
        // reads it: collecting the brackets inside it as groups handed an
        // unguarded nest to the recursive consumers (og C3 L01, I-22).
        if c == '"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
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
                } else if ch == ';' {
                    // Comment inside a group body (between clauses) — skip to EOL.
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    continue;
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
pub(super) fn where_groups(src: &str) -> Vec<String> {
    match src.find(":where") {
        Some(idx) => scan_groups(&src[idx + ":where".len()..]),
        None => Vec::new(),
    }
}

/// Top-level items of an EDN body, in order: strings, balanced collections
/// (`[..]`, `(..)`, `{..}`, `#{..}`) and bare tokens. Comments are skipped.
fn edn_items(body: &str) -> Vec<&str> {
    let b = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c == b';' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if matches!(c, b'[' | b'(' | b'{') || (c == b'#' && b.get(i + 1) == Some(&b'{')) {
            if c == b'#' {
                i += 1;
            }
            let mut depth = 0usize;
            let mut in_str = false;
            while i < b.len() {
                let ch = b[i];
                if in_str {
                    if ch == b'\\' {
                        i += 2;
                        continue;
                    }
                    if ch == b'"' {
                        in_str = false;
                    }
                } else if ch == b'"' {
                    in_str = true;
                } else if matches!(ch, b'[' | b'(' | b'{') {
                    depth += 1;
                } else if matches!(ch, b']' | b')' | b'}') {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
        } else {
            while i < b.len()
                && !b[i].is_ascii_whitespace()
                && !matches!(b[i], b'[' | b']' | b'(' | b')' | b'{' | b'}' | b'"' | b',')
            {
                i += 1;
            }
            if i == start {
                i += 1; // a stray closer is its own item
            }
        }
        out.push(&body[start..i.min(body.len())]);
    }
    out
}

/// The body of a `[..]` or `(..)` item, or `None` for anything else.
fn edn_body(item: &str, open: char, close: char) -> Option<&str> {
    item.trim().strip_prefix(open)?.strip_suffix(close)
}

/// The variable the query returns: `:find (pull ?b [*])` or `:find ?b`.
pub(super) fn advanced_find_var(src: &str) -> Option<String> {
    let at = src.find(":find")?;
    let first = *edn_items(&src[at + ":find".len()..]).first()?;
    let var = match edn_body(first, '(', ')') {
        Some(call) => {
            let items = edn_items(call);
            if items.first() != Some(&"pull") {
                return None;
            }
            *items.get(1)?
        }
        None => first,
    };
    var.starts_with('?').then(|| var.to_string())
}

/// Top-level `where` clauses with single-branch wrappers opened: an `(and ..)`
/// contributes its clauses, and an `(or ..)`/`(or-join [..] ..)` with exactly
/// one branch IS that branch. Both are the same query; only the shape differs.
pub(super) fn flatten_single_branch_groups(groups: Vec<String>) -> Vec<String> {
    // An explicit stack, not recursion: the nesting is authored text, so the
    // walk must not spend a native frame per level (I-22).
    let mut out = Vec::new();
    let mut pending: Vec<String> = groups.into_iter().rev().collect();
    while let Some(group) = pending.pop() {
        let Some(body) = edn_body(&group, '(', ')') else {
            out.push(group);
            continue;
        };
        let items = edn_items(body);
        let branches: Option<Vec<String>> = match items.first().copied() {
            Some("and") => Some(items[1..].iter().map(|item| item.to_string()).collect()),
            Some("or") if items.len() == 2 => Some(vec![items[1].to_string()]),
            Some("or-join") if items.len() == 3 && edn_body(items[1], '[', ']').is_some() => {
                Some(vec![items[2].to_string()])
            }
            _ => None,
        };
        match branches {
            Some(branches) => pending.extend(branches.into_iter().rev()),
            None => out.push(group),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvTarget {
    Block,
    Page,
}

/// A DataScript attribute Tine indexes, on the result block or on its page.
fn advanced_attribute(target: AdvTarget, attribute: &str) -> Option<(Attr, &'static str)> {
    match (target, attribute) {
        (AdvTarget::Block, ":block/marker") => Some((Attr::Task, "marker")),
        (AdvTarget::Block, ":block/priority") => Some((Attr::Priority, "priority")),
        (AdvTarget::Block, ":block/scheduled") => Some((Attr::Scheduled, "scheduled")),
        (AdvTarget::Block, ":block/deadline") => Some((Attr::Deadline, "deadline")),
        (AdvTarget::Page, ":block/journal?") => Some((Attr::Journal, "journal-page")),
        (AdvTarget::Page, ":block/journal-day") => Some((Attr::Day, "journal-day")),
        _ => None,
    }
}

/// A literal operand in the type DataScript stores for `attr`: marker and
/// priority are strings, scheduled/deadline/journal-day are yyyymmdd
/// integers, `journal?` a boolean. A mismatched literal never matches in
/// Logseq either, so it is not lowered to something that might.
fn advanced_literal(
    attr: Attr,
    token: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> Option<Value> {
    match attr {
        Attr::Task | Attr::Priority => {
            let text = token.strip_prefix('"')?.strip_suffix('"')?;
            (!text.contains('\\')).then(|| Value::text(text))
        }
        Attr::Scheduled | Attr::Deadline | Attr::Day => {
            let number = match inputs.get(token) {
                Some(AdvancedInput::Date(value)) => *value,
                Some(AdvancedInput::Page(_)) => return None,
                None => token.parse::<i64>().ok()?,
            };
            Some(Value::Number {
                number: number as f64,
            })
        }
        Attr::Journal => match token {
            "true" => Some(Value::Bool { value: true }),
            "false" => Some(Value::Bool { value: false }),
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

/// How many times `var` appears as a whole token in `text`.
fn advanced_var_uses(text: &str, var: &str) -> usize {
    let boundary = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || "()[]{},\"".contains(c));
    text.match_indices(var)
        .filter(|(at, _)| {
            boundary(text[..*at].chars().next_back())
                && boundary(text[at + var.len()..].chars().next())
        })
        .count()
}

/// GH #542: lower the DataScript attribute patterns Logseq's own examples and
/// users' `:default-queries` are written in — `[?b :block/marker "TODO"]`, a
/// value variable narrowed by `[(contains? #{..} ?m)]` or `[(<= ?d ?today)]`,
/// the block's page with `:block/journal?` / `:block/journal-day`, and
/// `(not [?b :block/scheduled ?d])`.
///
/// Like the two lowerers above this is not a join engine. It lowers a clause
/// only when its meaning is a filter on the returned block alone: the subject
/// is the `:find` variable (or its page), and a value variable is used by
/// nothing but the one pattern that binds it and the predicates lowered with
/// it. Every other clause is left for `parse_adv_group` to report as
/// unsupported.
pub(super) fn lower_attribute_patterns(
    groups: &[String],
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
    let triple = |group: &str| -> Option<[String; 3]> {
        let items = edn_items(edn_body(group, '[', ']')?);
        (items.len() == 3 && items[0].starts_with('?')).then(|| {
            [
                items[0].to_string(),
                items[1].to_string(),
                items[2].to_string(),
            ]
        })
    };
    let active: Vec<usize> = (0..groups.len()).filter(|i| !taken.contains(i)).collect();

    // The returned block's page variable(s).
    let mut pages = std::collections::HashSet::new();
    for &index in &active {
        if let Some([subject, attribute, object]) = triple(&groups[index]) {
            if subject == block && attribute == ":block/page" && object.starts_with('?') {
                pages.insert(object);
                consumed.insert(index);
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

    // Patterns: a literal is a filter; a variable is a binding the predicates
    // below may narrow.
    let mut bindings: std::collections::HashMap<String, (usize, AdvTarget, Attr, &'static str)> =
        std::collections::HashMap::new();
    for &index in &active {
        let Some([subject, attribute, object]) = triple(&groups[index]) else {
            continue;
        };
        let Some(target) = target_of(&subject) else {
            continue;
        };
        let Some((attr, label)) = advanced_attribute(target, &attribute) else {
            continue;
        };
        if object.starts_with('?') {
            if attr != Attr::Journal && !bindings.contains_key(&object) {
                bindings.insert(object, (index, target, attr, label));
            }
        } else if let Some(value) = advanced_literal(attr, &object, inputs) {
            lowered.insert(
                index,
                (
                    advanced_on_target(target, Filter::attr(attr, CmpOp::Eq, value)),
                    label,
                ),
            );
        }
    }

    // Predicates over exactly one bound variable.
    let mut predicates: std::collections::HashMap<String, Vec<(usize, Filter)>> =
        std::collections::HashMap::new();
    for &index in &active {
        let Some(call) = edn_body(&groups[index], '[', ']')
            .map(edn_items)
            .filter(|items| items.len() == 1)
            .and_then(|items| edn_body(items[0], '(', ')'))
        else {
            continue;
        };
        let items = edn_items(call);
        let lowered_predicate = (|| -> Option<(String, Filter)> {
            let (symbol, left, right) = match items.as_slice() {
                [symbol, left, right] => (*symbol, *left, *right),
                _ => return None,
            };
            if symbol == "contains?" {
                let (_, _, attr, _) = bindings.get(right)?;
                let set = left.strip_prefix('#').and_then(|s| edn_body(s, '{', '}'))?;
                let values = edn_items(set)
                    .into_iter()
                    .map(|item| advanced_literal(*attr, item, inputs))
                    .collect::<Option<Vec<_>>>()?;
                return Some((
                    right.to_string(),
                    Filter::attr(*attr, CmpOp::In, Value::List { items: values }),
                ));
            }
            let cmp = |flipped: bool| -> Option<CmpOp> {
                Some(match (symbol, flipped) {
                    ("=", _) => CmpOp::Eq,
                    ("not=", _) => CmpOp::NotEq,
                    ("<", false) | (">", true) => CmpOp::Lt,
                    ("<=", false) | (">=", true) => CmpOp::Le,
                    (">", false) | ("<", true) => CmpOp::Gt,
                    (">=", false) | ("<=", true) => CmpOp::Ge,
                    _ => return None,
                })
            };
            let (var, operand, flipped) = match (bindings.get(left), bindings.get(right)) {
                (Some(_), None) => (left, right, false),
                (None, Some(_)) => (right, left, true),
                _ => return None,
            };
            let (_, _, attr, _) = bindings.get(var)?;
            let op = cmp(flipped)?;
            let ordered = matches!(attr, Attr::Scheduled | Attr::Deadline | Attr::Day);
            if !(ordered || op == CmpOp::Eq || op == CmpOp::NotEq) {
                return None;
            }
            let value = advanced_literal(*attr, operand, inputs)?;
            Some((var.to_string(), Filter::attr(*attr, op, value)))
        })();
        if let Some((var, filter)) = lowered_predicate {
            predicates.entry(var).or_default().push((index, filter));
        }
    }

    // A binding lowers only when nothing else in the query uses its variable.
    let everything = groups.join("\n");
    for (var, (index, target, attr, label)) in bindings {
        let narrowing = predicates.remove(&var).unwrap_or_default();
        if advanced_var_uses(&everything, &var) != 1 + narrowing.len() {
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

    // `(not [?b :block/scheduled ?d])`: no such value, where `?d` is bound by
    // no clause outside a `not` (a variable the outer query never binds is
    // local to each `not`, as the reporter's two `(not [.. ?d])` rely on).
    let is_not = |group: &str| {
        edn_body(group, '(', ')')
            .map(edn_items)
            .is_some_and(|items| items.first() == Some(&"not"))
    };
    for &index in &active {
        let Some(items) = edn_body(&groups[index], '(', ')').map(edn_items) else {
            continue;
        };
        let [head, clause] = items.as_slice() else {
            continue;
        };
        if *head != "not" {
            continue;
        }
        let Some([subject, attribute, object]) = triple(clause) else {
            continue;
        };
        let Some(target) = target_of(&subject) else {
            continue;
        };
        let Some((attr, _)) = advanced_attribute(target, &attribute) else {
            continue;
        };
        let inner = if object.starts_with('?') {
            let outside = groups
                .iter()
                .enumerate()
                .filter(|(other, group)| *other != index && !is_not(group))
                .map(|(_, group)| advanced_var_uses(group, &object))
                .sum::<usize>();
            if attr == Attr::Journal || outside != 0 || advanced_var_uses(clause, &object) != 1 {
                continue;
            }
            Filter::attr(attr, CmpOp::IsSet, Value::None)
        } else {
            let Some(value) = advanced_literal(attr, &object, inputs) else {
                continue;
            };
            Filter::attr(attr, CmpOp::Eq, value)
        };
        lowered.insert(
            index,
            (Filter::not(advanced_on_target(target, inner)), "not"),
        );
    }
    (lowered, consumed)
}

/// Lower one datalog source to the simple IR, with its clause report
/// (`ran`, `ignored`). `None` means nothing in the supported subset matched,
/// or a size/depth limit refused the source. Pure; O(source length).
pub(crate) fn advanced_pred(
    query_src: &str,
    current_page: Option<&str>,
    today: JournalDate,
) -> (Option<Query>, Vec<String>, Vec<String>) {
    // Both limits live here, not only at the two `run_advanced_*` entry points,
    // because `page_affects_advanced_query` reaches this function directly. It
    // used to skip the byte ceiling entirely, which made scoped invalidation the
    // one caller that could hand an unbounded graph-authored string to the
    // parser — the exact thing QUERY_SOURCE_MAX_BYTES exists to prevent.
    if !query_source_within_limit(query_src) {
        return (None, Vec::new(), vec!["query-too-large".to_string()]);
    }
    if !query_nesting_within_limit(query_src) {
        return (None, Vec::new(), vec!["query-nesting-too-deep".to_string()]);
    }
    let inputs = resolve_inputs(query_src, current_page, today);
    let mut ran = Vec::new();
    let mut ignored = Vec::new();
    let groups = flatten_single_branch_groups(where_groups(query_src));
    let (lowered_page_properties, consumed_patterns) = lower_page_property_patterns(&groups);
    let (lowered_current_pages, current_page_patterns) =
        lower_current_page_patterns(&groups, &inputs);
    let consumed_patterns = consumed_patterns
        .into_iter()
        .chain(current_page_patterns)
        .collect::<std::collections::HashSet<_>>();
    let taken = consumed_patterns
        .iter()
        .copied()
        .chain(lowered_current_pages.keys().copied())
        .chain(lowered_page_properties.keys().copied())
        .collect::<std::collections::HashSet<_>>();
    let (lowered_attributes, attribute_patterns) = lower_attribute_patterns(
        &groups,
        &taken,
        advanced_find_var(query_src).as_deref(),
        &inputs,
    );
    let preds: Vec<Filter> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            if let Some((pred, label)) = lowered_current_pages.get(&index) {
                ran.push((*label).into());
                return Some(pred.clone());
            }
            if let Some(pred) = lowered_page_properties.get(&index) {
                ran.push("page-property".into());
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
            let lowered = parse_adv_group(group, &inputs, today, &mut ran, &mut ignored, 0);
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
    let where_text = groups.join("\n");
    for variable in declared_input_vars(query_src) {
        if !inputs.contains_key(&variable) && advanced_var_uses(&where_text, &variable) > 0 {
            ignored.push(format!("input {variable}"));
        }
    }
    // GH #542: a `:result-transform` is a Clojure function (ADR 0042 keeps
    // scripting out). It reorders or reshapes the answer, so say it did not run.
    if query_src.contains(":result-transform") {
        ignored.push("result-transform".into());
    }
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
        Anchor::Block,
        filter,
        Source::Advanced {
            original: query_src.to_string(),
            og_options: String::new(),
        },
    );
    (Some(query), ran, ignored)
}

/// Lower the exact DataScript relationship Logseq uses to connect the typed
/// `:current-page` input to blocks. This is deliberately not a general join
/// engine: one page-name identity pattern must feed one `:block/refs` or
/// `:block/page` pattern, and every other shape remains visibly unsupported.
fn lower_current_page_patterns(
    groups: &[String],
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let triples = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let inner = group.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
            let tokens = inner.split_whitespace().collect::<Vec<_>>();
            (tokens.len() == 3).then_some((index, tokens))
        })
        .collect::<Vec<_>>();

    let mut candidates = Vec::new();
    for (identity_index, identity) in &triples {
        if identity[1] != ":block/name" || !identity[0].starts_with('?') {
            continue;
        }
        let Some(AdvancedInput::Page(page)) = inputs.get(identity[2]) else {
            continue;
        };
        for (relation_index, relation) in &triples {
            if relation[0] == identity[0]
                || !relation[0].starts_with('?')
                || relation[2] != identity[0]
            {
                continue;
            }
            let lowered = match relation[1] {
                ":block/refs" => Some((Filter::page_ref(page.clone()), "current-page-ref")),
                ":block/page" => Some((
                    Filter::rel(
                        Rel::Page,
                        Quant::Any,
                        Filter::attr(Attr::Name, CmpOp::Eq, Value::text(page.clone())),
                    ),
                    "current-page",
                )),
                _ => None,
            };
            if let Some(lowered) = lowered {
                candidates.push((*identity_index, *relation_index, lowered));
            }
        }
    }
    if candidates.len() != 1 {
        return Default::default();
    }
    let (identity_index, relation_index, lowered) = candidates.pop().unwrap();
    (
        std::collections::HashMap::from([(relation_index, lowered)]),
        std::collections::HashSet::from([identity_index]),
    )
}

/// Conservatively lower only the exact DataScript relationship used by the
/// released BEGIN_QUERY page-property form. The entity/property-map pattern and
/// `(get ...)` predicate must share the literal `?props` binding; every other
/// bracket form remains visible as an unsupported `pattern` in `parse_adv_group`.
fn lower_page_property_patterns(
    groups: &[String],
) -> (
    std::collections::HashMap<usize, Filter>,
    std::collections::HashSet<usize>,
) {
    let relations = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let inner = group.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
            (inner.split_whitespace().collect::<Vec<_>>() == ["?p", ":block/properties", "?props"])
                .then_some(index)
        })
        .collect::<Vec<_>>();
    if relations.len() != 1 {
        return Default::default();
    }

    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    for (index, group) in groups.iter().enumerate() {
        let Some(inner) = group
            .trim()
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
        else {
            continue;
        };
        let Some(call) = inner
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
        else {
            continue;
        };
        let tokens = call.split_whitespace().collect::<Vec<_>>();
        if tokens.len() != 3 || tokens[0] != "get" || tokens[1] != "?props" {
            continue;
        }
        let Some(key) = tokens[2].strip_prefix(':').filter(|key| !key.is_empty()) else {
            continue;
        };
        if key
            .chars()
            .any(|c| c.is_whitespace() || "()[]{}".contains(c))
        {
            continue;
        }
        lowered.insert(
            index,
            Filter::rel(
                Rel::Page,
                Quant::Any,
                og::property_leaf(og::normalize_prop_key(key), None),
            ),
        );
        consumed.insert(relations[0]);
    }
    (lowered, consumed)
}

/// Map one `:where` group to a `Pred` (or None → ignored). Recurses for and/or/not.
fn parse_adv_group(
    group: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: JournalDate,
    ran: &mut Vec<String>,
    ignored: &mut Vec<String>,
    depth: usize,
) -> Option<Filter> {
    if depth > QUERY_NESTING_MAX {
        ignored.push("query-nesting-too-deep".into());
        return None;
    }
    let c = group.trim();
    if !c.starts_with('(') {
        ignored.push("pattern".into()); // `[?e :a ?v]` joins, etc. — not in the subset
        return None;
    }
    // A lone `(` has no body; slicing `1..0` would panic on hostile input.
    let inner = c.get(1..c.len().saturating_sub(1)).unwrap_or("");
    let head = inner
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match head.as_str() {
        "and" | "or" | "not" => {
            let ignored_before = ignored.len();
            let kids: Vec<Filter> = scan_groups(inner)
                .iter()
                .filter_map(|g| parse_adv_group(g, inputs, today, ran, ignored, depth + 1))
                .collect();
            // GH #542: dropping a clause Tine does not understand only ever
            // WIDENS a conjunction. Under `not` it narrows the answer, and an
            // `or` missing a branch drops blocks the query returns; neither is
            // a superset with a notice. Such a group is dropped whole instead,
            // which widens the enclosing conjunction like any other ignored
            // clause.
            if head != "and" && ignored.len() > ignored_before {
                ignored.push(head.clone());
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
                adv_text_list(adv_strings(inner)),
            ))
        }
        "priority" => {
            ran.push("priority".into());
            Some(Filter::attr(
                Attr::Priority,
                CmpOp::In,
                adv_text_list(adv_strings(inner)),
            ))
        }
        "page-ref" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page-ref".into());
            Filter::page_ref(n)
        }),
        "property" | "page-property" => inner
            .split_whitespace()
            .skip(1)
            .find(|t| t.starts_with(':'))
            .map(|t| t.trim_start_matches(':').to_string())
            .map(|k| {
                let val = adv_strings(inner).into_iter().next();
                ran.push(head.clone());
                let leaf = og::property_leaf(og::normalize_prop_key(&k), val);
                if head == "property" {
                    leaf
                } else {
                    Filter::rel(Rel::Page, Quant::Any, leaf)
                }
            }),
        "page" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("page".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::Eq, Value::text(n)),
            )
        }),
        "namespace" => adv_strings(inner).into_iter().next().map(|n| {
            ran.push("namespace".into());
            Filter::rel(
                Rel::Page,
                Quant::Any,
                Filter::attr(Attr::Name, CmpOp::StartsWith, Value::text(format!("{n}/"))),
            )
        }),
        "page-tags" | "tags" => {
            let ts = adv_strings(inner);
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
            let args: Vec<&str> = inner.split_whitespace().skip(1).collect();
            if args.len() < 2 {
                ignored.push("between".into());
                return None;
            }
            let attr = args
                .iter()
                .take(args.len() - 2)
                .find_map(
                    |a| match a.trim_start_matches(':').to_ascii_lowercase().as_str() {
                        "scheduled" => Some(Attr::Scheduled),
                        "deadline" => Some(Attr::Deadline),
                        "journal" => Some(Attr::Day),
                        _ => None,
                    },
                )
                .unwrap_or(Attr::Day);
            let lo = adv_bound(args[args.len() - 2], inputs, today);
            let hi = adv_bound(args[args.len() - 1], inputs, today);
            if lo.is_none() && hi.is_none() {
                ignored.push("between".into());
                return None;
            }
            ran.push("between".into());
            // The advanced dialect resolves its bounds eagerly: `:inputs` may
            // bind a bound to an already-resolved ordinal, and an advanced query
            // is never re-printed as OG DSL, so the IR carries the ordinals.
            let range = adv_range(attr, lo, hi);
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
fn adv_bound(
    tok: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    today: JournalDate,
) -> Option<i64> {
    let t = tok.trim();
    if t.starts_with('?') {
        return match inputs.get(t) {
            Some(AdvancedInput::Date(value)) => Some(*value),
            _ => None,
        };
    }
    // A literal bound may be written as a bare token (`2026-06-24`) or a quoted
    // string (`"2026-06-24"`); `split_whitespace` keeps the quotes, so strip them.
    resolve_date_token(t.trim_matches('"').trim_start_matches(':'), today)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AdvancedInput {
    Date(i64),
    Page(String),
}

/// The `?var`s an `:in` clause declares, in order.
fn declared_input_vars(src: &str) -> Vec<String> {
    match src.find(":in") {
        Some(i) => {
            let rest = &src[i + 3..];
            let end = rest
                .find(":where")
                .or_else(|| rest.find(']'))
                .unwrap_or(rest.len());
            rest[..end]
                .split_whitespace()
                .filter(|t| t.starts_with('?'))
                .map(String::from)
                .collect()
        }
        None => Vec::new(),
    }
}

/// Build a typed positional input map by zipping `:in $ ?a ?b …` with
/// `:inputs [ … ]`. Dates stay numeric; Logseq's typed `:current-page` keyword
/// receives the caller's focused page. Unknown keywords remain unbound.
fn resolve_inputs(
    src: &str,
    current_page: Option<&str>,
    today: JournalDate,
) -> std::collections::HashMap<String, AdvancedInput> {
    let mut map = std::collections::HashMap::new();
    let vars = declared_input_vars(src);
    let vals: Vec<String> = match src.find(":inputs") {
        Some(i) => {
            let rest = &src[i + ":inputs".len()..];
            match (rest.find('['), rest.find(']')) {
                (Some(a), Some(b)) if b > a => rest[a + 1..b]
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
                _ => Vec::new(),
            }
        }
        None => Vec::new(),
    };
    for (v, val) in vars.iter().zip(vals.iter()) {
        if val.eq_ignore_ascii_case(":current-page") {
            if let Some(page) = current_page.map(str::trim).filter(|page| !page.is_empty()) {
                map.insert(v.clone(), AdvancedInput::Page(page.to_lowercase()));
            }
        } else if let Some(ord) = resolve_date_token(val.trim_start_matches(':'), today) {
            map.insert(v.clone(), AdvancedInput::Date(ord));
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

/// The largest relative offset a date token may carry, in years (and the
/// same span in days, weeks and months). Within it every resolved date keeps a
/// monotone `yyyymmdd` ordinal; beyond it the token is [`DateToken::OutOfRange`].
pub const MAX_DATE_OFFSET_YEARS: i64 = 10_000;

/// The unit of a relative date token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateUnit {
    /// `d`
    Day,
    /// `w`
    Week,
    /// `m`
    Month,
    /// `y`
    Year,
}

/// A date-literal token (a `between` bound, a TQL date, an advanced input),
/// read WITHOUT `today`. The ONE date-token grammar: the resolver
/// ([`resolve_date_token`]) and the OG printer's bare-or-`[[ ]]` choice both
/// read it (Rule 3), and the parsers diagnose [`DateToken::OutOfRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateToken {
    /// `today`/`now` (0), `yesterday` (−1), `tomorrow` (+1), any case: a day
    /// offset.
    Keyword(i64),
    /// `[+|-]N[dwmy]`: ASCII digits, a lowercase unit (`-7D` is not one).
    Relative {
        /// The signed count of units.
        n: i64,
        /// The unit.
        unit: DateUnit,
    },
    /// A relative token past ±[`MAX_DATE_OFFSET_YEARS`] years (or past `i64`):
    /// never resolves; parsers report it instead of answering a garbage day.
    OutOfRange,
    /// A `yyyy_MM_dd` or `yyyy-MM-dd` stem (zero padding optional), calendar
    /// valid.
    Stem(JournalDate),
    /// A journal title in the default `MMM do, yyyy` format. The OG printer
    /// writes it inside `[[ ]]`; every other shape prints bare.
    Title(JournalDate),
}

impl DateToken {
    /// Read `tok` (trimmed). `None`: not a date token.
    pub fn parse(tok: &str) -> Option<DateToken> {
        let t = tok.trim();
        for (keyword, offset) in [("today", 0), ("now", 0), ("yesterday", -1), ("tomorrow", 1)] {
            if t.eq_ignore_ascii_case(keyword) {
                return Some(DateToken::Keyword(offset));
            }
        }
        if let Some(token) = Self::relative(t) {
            return Some(token);
        }
        if let Some(date) = JournalDate::from_file_stem(t) {
            return Some(DateToken::Stem(date));
        }
        // master `journal_ordinal`: a default-format journal title literal.
        JournalDate::from_title(t).map(DateToken::Title)
    }

    fn relative(t: &str) -> Option<DateToken> {
        let (sign, rest) = match t.as_bytes().first()? {
            b'+' => (1i64, &t[1..]),
            b'-' => (-1i64, &t[1..]),
            _ => (1i64, t),
        };
        let unit = match rest.as_bytes().last()? {
            b'd' => DateUnit::Day,
            b'w' => DateUnit::Week,
            b'm' => DateUnit::Month,
            b'y' => DateUnit::Year,
            _ => return None,
        };
        let digits = &rest[..rest.len() - 1];
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let per_year = match unit {
            DateUnit::Day => 366,
            DateUnit::Week => 53,
            DateUnit::Month => 12,
            DateUnit::Year => 1,
        };
        match digits.parse::<i64>() {
            Ok(n) if n <= MAX_DATE_OFFSET_YEARS * per_year => {
                Some(DateToken::Relative { n: sign * n, unit })
            }
            _ => Some(DateToken::OutOfRange),
        }
    }

    /// The `yyyymmdd` ordinal this token names on `today`. Offsets are bounded
    /// at parse, so the arithmetic cannot overflow.
    pub fn resolve(self, today: JournalDate) -> Option<i64> {
        let date = match self {
            DateToken::Keyword(offset) => today.add_days(offset),
            DateToken::Relative { n, unit } => match unit {
                DateUnit::Day => today.add_days(n),
                DateUnit::Week => today.add_days(n * 7),
                DateUnit::Month => today.add_months(n),
                DateUnit::Year => today.add_months(n * 12),
            },
            DateToken::OutOfRange => return None,
            DateToken::Stem(date) | DateToken::Title(date) => date,
        };
        Some(date.ordinal_key())
    }

    /// Whether the OG printer may write this token bare (a title needs `[[ ]]`).
    pub fn prints_bare(self) -> bool {
        !matches!(self, DateToken::Title(_))
    }
}

/// Resolve a date-literal token to a `yyyymmdd` ordinal against `today`: the
/// [`DateToken`] grammar. `None` for a non-date or an out-of-range offset.
/// The ONE date-literal resolver: the advanced lowering and the executor's
/// date comparisons both call it (I-12).
pub fn resolve_date_token(tok: &str, today: JournalDate) -> Option<i64> {
    DateToken::parse(tok)?.resolve(today)
}

#[cfg(test)]
mod date_token_tests {
    use super::*;

    /// Reader B (og 14 Q2): an offset past the admitted range neither panics
    /// (debug overflow in `to_days() + n`, `n * 7`, `n * 12`) nor resolves to a
    /// garbage day in release; it does not resolve at all.
    #[test]
    fn an_out_of_range_offset_resolves_to_nothing_and_never_panics() {
        let today = JournalDate::from_ordinal(20260904);
        for token in [
            "+9223372036854775807d",
            "-9223372036854775807w",
            "1537228672809129302m",
            "-768614336404564651y",
            "+99999999999999999999999d",
            "-99999y",
        ] {
            assert_eq!(
                DateToken::parse(token),
                Some(DateToken::OutOfRange),
                "{token}"
            );
            assert_eq!(resolve_date_token(token, today), None, "{token}");
        }
        assert!(
            resolve_date_token("-10000y", today).is_some(),
            "the admitted limit resolves"
        );
        assert_eq!(resolve_date_token("+2w", today), Some(20260918));
        assert_eq!(
            resolve_date_token("-7D", today),
            None,
            "units are lowercase"
        );
        assert_eq!(resolve_date_token("2026_01_05", today), Some(20260105));
    }
}
