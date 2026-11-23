//! OG's older NEXT query: one existential over direct journal references.
use super::*;

pub(super) fn lower_ref_journal_patterns(
    groups: &[&Edn],
    taken: &std::collections::HashSet<usize>,
    block: &str,
    inputs: &std::collections::HashMap<String, AdvancedInput>,
) -> (
    std::collections::HashMap<usize, (Filter, &'static str)>,
    std::collections::HashSet<usize>,
) {
    let mut lowered = std::collections::HashMap::new();
    let mut consumed = std::collections::HashSet::new();
    let active: Vec<usize> = (0..groups.len()).filter(|i| !taken.contains(i)).collect();
    // OG query_react.cljs::resolve-query aliases ref-pages to direct refs,
    // not path-refs. Keep all page predicates inside ONE existential: different
    // references must not separately satisfy the lower and upper date bounds.
    for &index in &active {
        let Some((subject, "block/ref-pages", Some(object))) = triple(groups[index]) else {
            continue;
        };
        let Some(page) = variable(object) else {
            continue;
        };
        if subject != block || page == block || inputs.contains_key(page) {
            continue;
        }
        let mut variables = vec![page];
        for &other in &active {
            if let Some((subject, attribute, Some(object))) = triple(groups[other]) {
                if subject == page && matches!(attribute, "block/journal?" | "block/journal-day") {
                    if let Some(var) = variable(object).filter(|var| !inputs.contains_key(*var)) {
                        variables.push(var);
                    }
                }
            }
        }
        let related: Vec<usize> = active
            .iter()
            .copied()
            .filter(|other| {
                *other != index
                    && variables
                        .iter()
                        .any(|var| advanced_var_uses(&groups[*other], var) > 0)
            })
            .collect();
        if taken.iter().any(|other| {
            variables
                .iter()
                .any(|var| advanced_var_uses(&groups[*other], var) > 0)
        }) {
            continue;
        }
        // Only page attributes and their unary predicates belong here. Any
        // shared block variable or other join leaves the original clauses intact.
        if related.is_empty()
            || related
                .iter()
                .any(|other| advanced_var_uses(&groups[*other], block) > 0)
        {
            continue;
        }
        let relation = Edn::Vec(vec![
            Edn::Symbol(block.into()),
            Edn::Keyword("block/page".into()),
            Edn::Symbol(page.into()),
        ]);
        let mut local = vec![&relation];
        local.extend(related.iter().map(|other| groups[*other]));
        let (filters, used) =
            lower_attribute_patterns(&local, &Default::default(), Some(block), inputs);
        if (1..local.len()).any(|other| !filters.contains_key(&other) || used.contains(&other)) {
            continue;
        }
        let predicates = (1..local.len())
            .map(|other| {
                let (filter, _) = &filters[&other];
                match filter {
                    Filter::Leaf {
                        leaf:
                            super::super::ir::Leaf::Rel {
                                rel: Rel::Page,
                                quant: Quant::Any,
                                pred,
                            },
                    } => Some((**pred).clone()),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>();
        if let Some(predicates) = predicates {
            lowered.insert(
                index,
                (
                    Filter::rel(Rel::DirectRefs, Quant::Any, Filter::and(predicates)),
                    "ref-pages",
                ),
            );
            consumed.extend(related);
        }
    }

    (lowered, consumed)
}
