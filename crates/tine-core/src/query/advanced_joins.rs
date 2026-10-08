//! Bounded advanced-query identity joins and date alternatives.
//! No arbitrary Datalog: every participating clause must be accounted for.

use super::*;

/// A built-in rule filters the returned entity; a rule on some other
/// variable is an arbitrary join, even when its name is familiar.
pub(super) fn rules_target_result(groups: &[String], find: &str) -> bool {
    let mut pending = groups.iter().map(String::as_str).collect::<Vec<_>>();
    while let Some(group) = pending.pop() {
        let Some(body) = edn_body(group, '(', ')') else {
            continue;
        };
        let args = edn_items(body);
        match args.first().copied() {
            Some("and" | "or" | "not") => pending.extend(args[1..].iter().copied()),
            Some("between") => {
                if args.len() < 4 || !args[1..args.len() - 2].contains(&find) {
                    return false;
                }
            }
            Some(
                "task" | "todo" | "priority" | "page-ref" | "property" | "page-property" | "page"
                | "namespace" | "page-tags" | "tags" | "scheduled" | "deadline" | "journal",
            ) => {
                if args.get(1).copied() != Some(find) {
                    return false;
                }
            }
            _ => (),
        }
    }
    true
}

/// Tagged page-name results (docs example 6, GH #628). Only identity and
/// page-tag triples are accepted here; all clauses must be consumed together.
/// Scalar page names become page rows in Tine's own display, never block rows.
pub(super) fn lower_named_page_query(
    src: &str,
    groups: &[String],
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> Option<Query> {
    let find = advanced_find_var(src)?;
    let triples = groups
        .iter()
        .map(|g| {
            let items = edn_items(edn_body(g, '[', ']')?);
            (items.len() == 3).then_some(items)
        })
        .collect::<Option<Vec<_>>>()?;
    let projection = triples
        .iter()
        .position(|t| t[1] == ":block/name" && t[2] == find && !inputs.contains_key(&find));
    // The name projection proves the result entity is a page. A tag triple
    // alone can also bind a tagged Org block in OG. An unfiltered name query
    // includes reference-only page entities which @page does not enumerate.
    let projection = projection?;
    let find_items = edn_items(edn_body(query_vector(src)?, '[', ']')?);
    let find_at = find_items.iter().position(|s| *s == ":find")?;
    if find_items[find_at + 1].starts_with('(') {
        return None;
    }
    let page = triples[projection][0];
    let mut used = std::collections::HashSet::new();
    used.insert(projection);
    let mut filters = Vec::new();
    for (i, t) in triples.iter().enumerate() {
        if t[0] != page || t[1] != ":block/tags" {
            continue;
        }
        let (identity, name) = triples.iter().enumerate().find_map(|(j, identity)| {
            if identity[0] != t[2] || identity[1] != ":block/name" {
                return None;
            }
            let name = match inputs.get(identity[2]) {
                Some(AdvancedInput::Page(s)) => s.clone(),
                _ => edn_string(identity[2])?,
            };
            Some((j, name))
        })?;
        // A name variable shared beyond this one tag join is arbitrary Datalog.
        if advanced_var_uses(&groups.join(" "), t[2]) != 2 {
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
    groups: &[String],
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
        let Some(body) = edn_body(group, '(', ')') else {
            continue;
        };
        let items = edn_items(body);
        if items.first() != Some(&"or") || items.len() != 3 {
            continue;
        }
        let mut branches = Vec::new();
        let mut date_var = None;
        for branch in &items[1..] {
            let Some(body) = edn_body(branch, '[', ']') else {
                break;
            };
            let triple = edn_items(body);
            if triple.len() != 3
                || triple[0] != find
                || !triple[2].starts_with('?')
                || inputs.contains_key(triple[2])
            {
                break;
            }
            if !matches!(triple[1], ":block/scheduled" | ":block/deadline") {
                break;
            }
            if date_var.is_some_and(|v| v != triple[2]) {
                break;
            }
            date_var = Some(triple[2]);
            branches.push(branch.to_string());
        }
        if branches.len() != 2 {
            continue;
        }
        let var = date_var.unwrap();
        let predicates = groups
            .iter()
            .enumerate()
            .filter(|(i, g)| *i != index && !taken.contains(i) && advanced_var_uses(g, var) > 0)
            .collect::<Vec<_>>();
        if advanced_var_uses(&groups.join(" "), var) != 2 + predicates.len() {
            continue;
        }
        let mut filters = Vec::new();
        for branch in branches {
            let local = std::iter::once(branch)
                .chain(predicates.iter().map(|(_, g)| (*g).clone()))
                .collect::<Vec<_>>();
            let (parts, used) =
                lower_attribute_patterns(&local, &Default::default(), Some(find), inputs);
            if parts.len() + used.len() != local.len() {
                break;
            }
            filters.push(Filter::and(parts.into_values().map(|(f, _)| f).collect()));
        }
        if filters.len() == 2 {
            lowered.insert(index, (Filter::or(filters), "scheduled-or-deadline"));
            consumed.extend(predicates.into_iter().map(|(i, _)| i));
        }
    }
    (lowered, consumed)
}

/// Lower the exact DataScript relationship Logseq uses to connect the typed
/// `:current-page` input to blocks. This is deliberately not a general join
/// engine: one page-name identity pattern must feed one `:block/refs` or
/// `:block/page` pattern, and every other shape remains visibly unsupported.
pub(super) fn lower_current_page_patterns(
    groups: &[String],
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
            let inner = group.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
            let tokens = edn_items(inner);
            (tokens.len() == 3).then_some((index, tokens))
        })
        .collect::<Vec<_>>();

    let mut candidates = Vec::new();
    for (identity_index, identity) in &triples {
        if identity[1] != ":block/name" || !identity[0].starts_with('?') {
            continue;
        }
        let page = match inputs.get(identity[2]) {
            Some(AdvancedInput::Page(page)) => page.clone(),
            _ => match edn_string(identity[2]) {
                Some(page) => page,
                None => continue,
            },
        };
        for (relation_index, relation) in &triples {
            if relation[0] == identity[0] || Some(relation[0]) != find || relation[2] != identity[0]
            {
                continue;
            }
            let lowered = match relation[1] {
                ":block/refs" => Some((
                    Filter::rel(
                        Rel::DirectRefs,
                        Quant::Any,
                        Filter::attr(Attr::Name, CmpOp::Eq, Value::text(page.clone())),
                    ),
                    "current-page-ref",
                )),
                ":block/path-refs" => Some((Filter::page_ref(page.clone()), "page-path-ref")),
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
            if let Some((filter, label)) = lowered {
                if advanced_var_uses(&groups.join(" "), identity[0]) != 2 {
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
