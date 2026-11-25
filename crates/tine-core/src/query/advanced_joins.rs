//! Bounded advanced-query identity joins and date alternatives.
//! No arbitrary Datalog: every participating clause must be accounted for.

use super::*;

/// A built-in rule filters the returned entity or its explicitly joined page.
/// Other rule subjects are arbitrary joins, even with a familiar rule name.
pub(super) fn rules_target_result(groups: &[&Edn], find: &str, anchor: Anchor) -> bool {
    let pages = groups
        .iter()
        .filter_map(|group| {
            let (subject, key, object) = triple(group)?;
            let page = variable(object?)?;
            (subject == find && key == "block/page" && page != find).then_some(page)
        })
        .collect::<std::collections::HashSet<_>>();
    let mut pending = groups.to_vec();
    while let Some(group) = pending.pop() {
        let Some(args) = list(group) else {
            continue;
        };
        let subject = args.get(1).and_then(variable);
        match args.first().and_then(Edn::symbol) {
            Some("and" | "or" | "not") => pending.extend(&args[1..]),
            Some("between") => {
                if anchor == Anchor::Page
                    || args.len() < 4
                    || !args[1..args.len() - 2]
                        .iter()
                        .any(|arg| variable(arg) == Some(find))
                {
                    return false;
                }
            }
            Some("page-property") => {
                if !(anchor == Anchor::Page && subject == Some(find))
                    && !subject.is_some_and(|var| pages.contains(var))
                {
                    return false;
                }
            }
            Some(
                "task" | "todo" | "priority" | "page-ref" | "property" | "page" | "namespace"
                | "page-tags" | "tags" | "scheduled" | "deadline" | "journal",
            ) => {
                if anchor == Anchor::Page || subject != Some(find) {
                    return false;
                }
            }
            _ => (),
        }
    }
    true
}

/// OG's page-property rule binds :block/name, so its subject is a page.
/// A negated rule alone does not bind the result entity. Keep unsupported
/// joins outside this bounded translator rather than guessing from variable names.
pub(super) fn page_property_result(groups: &[&Edn], find: &str) -> bool {
    let mut pending = groups.to_vec();
    while let Some(group) = pending.pop() {
        let Some(args) = list(group) else {
            continue;
        };
        match args.first().and_then(Edn::symbol) {
            Some("and" | "or") => pending.extend(&args[1..]),
            Some("page-property") if args.get(1).and_then(variable) == Some(find) => return true,
            _ => (),
        }
    }
    false
}

/// Tagged page-name results (docs example 6, GH #628). Only identity and
/// page-tag triples are accepted here; all clauses must be consumed together.
/// Scalar page names become page rows in Tine's own display, never block rows.
pub(super) fn lower_named_page_query(
    src: &str,
    find: &str,
    groups: &[&Edn],
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> Option<Query> {
    let triples = groups
        .iter()
        .map(|g| {
            let (subject, key, object) = triple(g)?;
            Some((subject, key, object?))
        })
        .collect::<Option<Vec<_>>>()?;
    let projection = triples.iter().position(|(_, key, object)| {
        *key == "block/name" && variable(object) == Some(find) && !inputs.contains_key(find)
    });
    // The name projection proves the result entity is a page. A tag triple
    // alone can also bind a tagged Org block in OG. An unfiltered name query
    // includes reference-only page entities which @page does not enumerate.
    let projection = projection?;
    let page = triples[projection].0;
    let mut used = std::collections::HashSet::new();
    used.insert(projection);
    let mut filters = Vec::new();
    for (i, &(subject, key, object)) in triples.iter().enumerate() {
        if subject != page || key != "block/tags" {
            continue;
        }
        let tagged = variable(object)?;
        let (identity, name) =
            triples
                .iter()
                .enumerate()
                .find_map(|(j, &(subject, key, name))| {
                    if subject != tagged || key != "block/name" {
                        return None;
                    }
                    let name = page_operand(name, inputs)?;
                    Some((j, name))
                })?;
        // A name variable shared beyond this one tag join is arbitrary Datalog.
        if groups_use(groups, tagged) != 2 {
            return None;
        }
        used.extend([i, identity]);
        filters.push(if name != name.to_lowercase() || name != name.trim() {
            Filter::False
        } else {
            Filter::rel(
                Rel::Props,
                Quant::Any,
                Filter::and(vec![
                    Filter::attr(Attr::Key, CmpOp::Eq, Value::text("tags")),
                    Filter::attr(Attr::Value, CmpOp::Eq, Value::text(name)),
                ]),
            )
        });
    }
    if used.len() != groups.len() || filters.is_empty() {
        return None;
    }
    Some(Query::new(
        Anchor::Page,
        Filter::and(filters),
        Source::Advanced {
            original: src.into(),
            og_options: String::new(),
        },
    ))
}

pub(super) fn lower_date_disjunctions(
    groups: &[&Edn],
    taken: &std::collections::HashSet<usize>,
    find: Option<&str>,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let Some(find) = find else {
        return Default::default();
    };
    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    for (index, group) in groups
        .iter()
        .enumerate()
        .filter(|(i, _)| !taken.contains(i))
    {
        let Some([head, rest @ ..]) = list(group) else {
            continue;
        };
        let (join, branches) = match (head.symbol(), rest) {
            (Some("or"), branches) => (None, branches),
            (Some("or-join"), [join, branches @ ..]) => match vector(join) {
                Some(join) => (Some(join), branches),
                None => continue,
            },
            _ => continue,
        };
        // DataScript refuses an `or` whose branches bind different variables.
        if branches.len() < 2 || (join.is_none() && !same_variables(branches)) {
            continue;
        }
        let Some(alternatives) = branches
            .iter()
            .map(branch_alternatives)
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let alternatives = alternatives.into_iter().flatten().collect::<Vec<_>>();
        let dates = alternatives
            .iter()
            .flatten()
            .filter_map(|clause| match triple(clause)? {
                (_, "block/scheduled" | "block/deadline" | "block/journal-day", Some(object)) => {
                    variable(object)
                }
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let Some(&var) = dates.first().filter(|_| dates.len() == 1) else {
            continue;
        };
        let mut joined = std::collections::BTreeSet::new();
        join.unwrap_or_default()
            .iter()
            .for_each(|v| variables(v, &mut joined));
        let join_ok = join.is_none_or(|join| {
            join.len() == 2 && joined == std::collections::BTreeSet::from([find, var])
        });
        if inputs.contains_key(var) || !join_ok {
            continue;
        }
        // Branch-local variables (the journal page) join nothing outside.
        let mut local = std::collections::BTreeSet::new();
        variables(group, &mut local);
        if local.iter().any(|&other| {
            other != find
                && other != var
                && groups_use(groups, other) != advanced_var_uses(group, other)
        }) {
            continue;
        }
        let predicates = groups
            .iter()
            .enumerate()
            .filter(|(i, g)| *i != index && !taken.contains(i) && advanced_var_uses(g, var) > 0)
            .collect::<Vec<_>>();
        if groups_use(groups, var) != advanced_var_uses(group, var) + predicates.len() {
            continue;
        }
        // Each alternative goes through the one lowerer of its shape, with the
        // predicates that follow the group, so a block is listed when SOME
        // branch's date passes them (DataScript's existential `or-join`).
        let mut filters = Vec::new();
        for clauses in &alternatives {
            let local = clauses
                .iter()
                .copied()
                .chain(predicates.iter().map(|(_, g)| **g))
                .collect::<Vec<_>>();
            let (parts, used) =
                lower_attribute_patterns(&local, &Default::default(), Some(find), inputs);
            if parts.len() + used.len() != local.len() {
                break;
            }
            filters.push(Filter::and(parts.into_values().map(|(f, _)| f).collect()));
        }
        if filters.len() == alternatives.len() {
            lowered.insert(index, (Filter::or(filters), "scheduled-or-deadline"));
            consumed.extend(predicates.into_iter().map(|(i, _)| i));
        }
    }
    (lowered, consumed)
}

/// A date branch as conjunctions of clauses: its `and`s opened and at most one
/// inner `or` (GH #1017's `:block/page` or `:block/ref-pages`) expanded.
fn branch_alternatives(branch: &Edn) -> Option<Vec<Vec<&Edn>>> {
    let mut alternatives = vec![Vec::new()];
    for clause in flatten_single_branch_groups(std::slice::from_ref(branch)) {
        match list(clause) {
            Some([head, options @ ..]) if head.symbol() == Some("or") => {
                if alternatives.len() > 1 || options.len() < 2 || !same_variables(options) {
                    return None;
                }
                let before = alternatives.pop()?;
                for option in options {
                    let mut clauses = before.clone();
                    clauses.extend(flatten_single_branch_groups(std::slice::from_ref(option)));
                    alternatives.push(clauses);
                }
            }
            _ => alternatives
                .iter_mut()
                .for_each(|clauses| clauses.push(clause)),
        }
    }
    Some(alternatives)
}

fn same_variables(branches: &[Edn]) -> bool {
    let mut sets = branches.iter().map(|branch| {
        let mut set = std::collections::BTreeSet::new();
        variables(branch, &mut set);
        set
    });
    let first = sets.next();
    sets.all(|set| Some(set) == first)
}

fn variables<'a>(value: &'a Edn, out: &mut std::collections::BTreeSet<&'a str>) {
    match value {
        Edn::Vec(items) | Edn::List(items) | Edn::Set(items) => {
            items.iter().for_each(|item| variables(item, out))
        }
        _ => out.extend(variable(value)),
    }
}

/// Lower the exact DataScript relationship Logseq uses to connect the typed
/// `:current-page` input to blocks. This is deliberately not a general join
/// engine: one page-name identity pattern must feed one `:block/refs` or
/// `:block/page` pattern, and every other shape remains visibly unsupported.
///
/// One reference rule (D-20261019-Q8, ruled for advanced queries as QL-1 on
/// 2026-11-03): the `:block/refs` and `:block/path-refs` shapes lower to the
/// same reference leaves as `[[A]]` / `(page-ref A)`, so they match A's whole
/// alias group, although OG's DataScript joins one `:block/name` entity. The
/// IR carries no "raw Datalog, stay literal" mark on purpose: that would be a
/// second rule for one question (I-12). `:block/page` lowers to `Rel::Page`
/// and stays literal, like `(page A)`.
pub(super) fn lower_current_page_patterns(
    groups: &[&Edn],
    inputs: &std::collections::HashMap<String, AdvancedInput>,
    find: Option<&str>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let triples = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let (subject, key, object) = triple(group)?;
            Some((index, subject, key, object?))
        })
        .collect::<Vec<_>>();

    let mut candidates = Vec::new();
    for &(identity_index, subject, key, name) in &triples {
        if key != "block/name" {
            continue;
        }
        let Some(page) = page_operand(name, inputs) else {
            continue;
        };
        for &(relation_index, related, relation, object) in &triples {
            if related == subject || Some(related) != find || variable(object) != Some(subject) {
                continue;
            }
            let lowered = match relation {
                "block/refs" => Some((
                    Filter::rel(
                        Rel::DirectRefs,
                        Quant::Any,
                        Filter::attr(Attr::Name, CmpOp::Eq, Value::text(page.clone())),
                    ),
                    "current-page-ref",
                )),
                "block/path-refs" => Some((Filter::page_ref(page.clone()), "page-path-ref")),
                "block/page" => Some((
                    Filter::rel(
                        Rel::Page,
                        Quant::Any,
                        Filter::attr(Attr::Name, CmpOp::Eq, Value::text(page.clone())),
                    ),
                    "current-page",
                )),
                _ => None,
            };
            if let Some((filter, label)) = lowered {
                if groups_use(groups, subject) != 2 {
                    continue;
                }
                let lowered = (
                    if page != page.to_lowercase() || page != page.trim() {
                        Filter::False
                    } else {
                        filter
                    },
                    label,
                );
                candidates.push((identity_index, relation_index, lowered));
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
