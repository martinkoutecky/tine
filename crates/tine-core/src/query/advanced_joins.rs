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

/// The date an alternative of a date disjunction binds: GH #1017's closed list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DateKind {
    /// `[?h :block/scheduled ?d]`
    Scheduled,
    /// `[?h :block/deadline ?d]`
    Deadline,
    /// `[?h :block/page ?p] [?p :block/journal-day ?d]`
    JournalDay,
    /// `[?h :block/ref-pages ?p] [?p :block/journal-day ?d]`
    RefJournalDay,
}

/// One recognised alternative: its kind, its triples, the date variable, the
/// page variable of a journal kind, and whether it adds `[?p :block/journal? true]`.
struct DateAlternative<'a> {
    kind: DateKind,
    clauses: Vec<&'a Edn>,
    date: &'a str,
    page: Option<&'a str>,
    journal_flag: bool,
}

/// Lower `(or B1 … Bn)` or `(or-join [?h ?d] B1 … Bn)` when every alternative
/// is one of the four [`DateKind`]s on the same date variable and every other
/// use of that variable is a predicate after the group. Anything else is left
/// for the caller to report as unsupported, as DataScript refuses or answers
/// it differently. One filter is lowered per kind present (at most four),
/// each with the trailing predicates, so the work is linear in the query.
pub(super) fn lower_date_disjunctions(
    groups: &[&Edn],
    taken: &std::collections::HashSet<usize>,
    find: Option<&str>,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    let Some(find) = find.filter(|find| !inputs.contains_key(*find)) else {
        return (lowered, consumed);
    };
    let mut users = None;
    for (index, group) in groups
        .iter()
        .enumerate()
        .filter(|(i, _)| !taken.contains(i))
    {
        let Some(alternatives) = date_alternatives(group, find) else {
            continue;
        };
        let date = alternatives[0].date;
        let users = users.get_or_insert_with(|| variable_users(groups));
        // The page of a journal kind joins nothing outside its group.
        let private = |page: &str| {
            !inputs.contains_key(page) && users.get(page).is_some_and(|u| u[..] == [index])
        };
        if inputs.contains_key(date) || !alternatives.iter().filter_map(|a| a.page).all(private) {
            continue;
        }
        // DataScript binds the date inside the group: every other use of it
        // must be a predicate that comes after the group.
        let predicates = users[date]
            .iter()
            .copied()
            .filter(|&i| i != index)
            .collect::<Vec<_>>();
        if predicates
            .iter()
            .any(|&i| i < index || taken.contains(&i) || !is_predicate(groups[i]))
        {
            continue;
        }
        // The union of a journal kind with and without `journal? true` is the
        // kind without it, so each kind needs one filter.
        let mut kinds: Vec<&DateAlternative> = Vec::new();
        for alternative in &alternatives {
            match kinds.iter_mut().find(|kind| kind.kind == alternative.kind) {
                Some(kind) if kind.journal_flag && !alternative.journal_flag => *kind = alternative,
                Some(_) => (),
                None => kinds.push(alternative),
            }
        }
        let mut filters = Vec::new();
        for kind in &kinds {
            let local = kind
                .clauses
                .iter()
                .copied()
                .chain(predicates.iter().map(|&i| groups[i]))
                .collect::<Vec<_>>();
            let (parts, used) =
                lower_attribute_patterns(&local, &Default::default(), Some(find), inputs);
            if parts.len() + used.len() != local.len() {
                break;
            }
            filters.push(Filter::and(parts.into_values().map(|(f, _)| f).collect()));
        }
        if filters.len() == kinds.len() {
            lowered.insert(index, (Filter::or(filters), "scheduled-or-deadline"));
            consumed.extend(predicates);
        }
    }
    (lowered, consumed)
}

/// The recognised alternatives of a date disjunction on `find`, or `None`
/// when the group's form or any alternative is outside the closed list.
fn date_alternatives<'a>(group: &'a Edn, find: &str) -> Option<Vec<DateAlternative<'a>>> {
    let [head, rest @ ..] = list(group)? else {
        return None;
    };
    let (join, branches) = match head.symbol()? {
        // DataScript refuses an `or` whose branches bind different variables.
        "or" => (None, same_variables(rest).then_some(rest)?),
        "or-join" => {
            let [join, branches @ ..] = rest else {
                return None;
            };
            let [first, second] = vector(join)? else {
                return None;
            };
            (Some((variable(first)?, variable(second)?)), branches)
        }
        _ => return None,
    };
    if branches.len() < 2 {
        return None;
    }
    let mut alternatives = Vec::new();
    for branch in branches {
        for clauses in branch_clauses(branch)? {
            alternatives.push(date_alternative(clauses, find)?);
        }
    }
    let date = alternatives[0].date;
    let joins_block_and_date =
        |(a, b): (&str, &str)| (a, b) == (find, date) || (a, b) == (date, find);
    (alternatives.iter().all(|a| a.date == date) && join.is_none_or(joins_block_and_date))
        .then_some(alternatives)
}

/// A branch as its alternatives' triples: a triple, or one `and` of triples
/// with at most one inner plain `or` whose options are single triples on the
/// same variables. No alternative has more than three triples, so a longer
/// `and` is refused before its inner `or` is expanded.
fn branch_clauses(branch: &Edn) -> Option<Vec<Vec<&Edn>>> {
    if vector(branch).is_some() {
        return Some(vec![vec![branch]]);
    }
    let [head, clauses @ ..] = list(branch)? else {
        return None;
    };
    if head.symbol() != Some("and") || clauses.len() > 3 {
        return None;
    }
    let mut fixed = Vec::new();
    let mut options = None;
    for clause in clauses {
        if vector(clause).is_some() {
            fixed.push(clause);
            continue;
        }
        let [head, inner @ ..] = list(clause)? else {
            return None;
        };
        if head.symbol() != Some("or")
            || options.is_some()
            || inner.len() < 2
            || !inner.iter().all(|option| vector(option).is_some())
            || !same_variables(inner)
        {
            return None;
        }
        options = Some(inner);
    }
    Some(match options {
        None => vec![fixed],
        Some(options) => options
            .iter()
            .map(|option| fixed.iter().copied().chain([option]).collect())
            .collect(),
    })
}

/// Which [`DateKind`] these triples are exactly, visiting each twice at most.
fn date_alternative<'a>(clauses: Vec<&'a Edn>, find: &str) -> Option<DateAlternative<'a>> {
    let fact = |clause: &'a Edn| match triple(clause)? {
        (subject, key, Some(object)) => Some((subject, key, object)),
        _ => None,
    };
    let fresh = |object: &'a Edn| variable(object).filter(|var| *var != find);
    if let [clause] = clauses[..] {
        let (subject, key, object) = fact(clause)?;
        let kind = match key {
            "block/scheduled" => DateKind::Scheduled,
            "block/deadline" => DateKind::Deadline,
            _ => return None,
        };
        return (subject == find).then_some(DateAlternative {
            kind,
            date: fresh(object)?,
            clauses,
            page: None,
            journal_flag: false,
        });
    }
    if !(2..=3).contains(&clauses.len()) {
        return None;
    }
    let mut link = None;
    for &clause in &clauses {
        let (subject, key, object) = fact(clause)?;
        if subject == find {
            if link.is_some() {
                return None;
            }
            link = Some((key, fresh(object)?));
        }
    }
    let (key, page) = link?;
    let kind = match key {
        "block/page" => DateKind::JournalDay,
        "block/ref-pages" => DateKind::RefJournalDay,
        _ => return None,
    };
    let (mut date, mut journal_flag) = (None, false);
    for &clause in &clauses {
        let (subject, key, object) = fact(clause)?;
        match key {
            _ if subject == find => (),
            _ if subject != page => return None,
            "block/journal-day" if date.is_none() => {
                date = Some(fresh(object).filter(|var| *var != page)?);
            }
            "block/journal?" if !journal_flag && matches!(object, Edn::Bool(true)) => {
                journal_flag = true;
            }
            _ => return None,
        }
    }
    Some(DateAlternative {
        kind,
        date: date?,
        clauses,
        page: Some(page),
        journal_flag,
    })
}

/// `[(f …)]` or `(not [(f …)])`: a predicate clause, not a binding.
fn is_predicate(group: &Edn) -> bool {
    let group = match list(group) {
        Some([head, inner]) if head.symbol() == Some("not") => inner,
        _ => group,
    };
    matches!(vector(group), Some([call]) if list(call).is_some())
}

/// Each variable's groups, ascending, from one walk over the whole `:where`.
fn variable_users<'a>(groups: &[&'a Edn]) -> std::collections::HashMap<&'a str, Vec<usize>> {
    let mut users: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    for (index, group) in groups.iter().enumerate() {
        let mut pending = vec![*group];
        while let Some(value) = pending.pop() {
            match value {
                Edn::Vec(items) | Edn::List(items) | Edn::Set(items) => pending.extend(items),
                Edn::Map(entries) => entries.iter().for_each(|(k, v)| pending.extend([k, v])),
                _ => {
                    if let Some(var) = variable(value) {
                        let groups = users.entry(var).or_default();
                        if groups.last() != Some(&index) {
                            groups.push(index);
                        }
                    }
                }
            }
        }
    }
    users
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
