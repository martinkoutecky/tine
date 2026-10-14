//! OG's older NEXT query: one existential over direct journal references.
use super::*;

pub(super) fn lower_ref_journal_patterns(
    groups: &[String],
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
    let triple = |group: &str| -> Option<[String; 3]> {
        let items = edn_items(edn_body(group, '[', ']')?);
        ((items.len() == 2 || items.len() == 3) && items[0].starts_with('?')).then(|| {
            [
                items[0].to_string(),
                items[1].to_string(),
                items.get(2).unwrap_or(&"_").to_string(),
            ]
        })
    };
    // OG query_react.cljs::resolve-query aliases ref-pages to direct refs,
    // not path-refs. Keep all page predicates inside ONE existential: different
    // references must not separately satisfy the lower and upper date bounds.
    for &index in &active {
        let Some([subject, attribute, page]) = triple(&groups[index]) else {
            continue;
        };
        if subject != block
            || attribute != ":block/ref-pages"
            || !page.starts_with('?')
            || page == block
            || inputs.contains_key(&page)
        {
            continue;
        }
        let mut variables = vec![page.clone()];
        for &other in &active {
            if let Some([subject, attribute, object]) = triple(&groups[other]) {
                if subject == page
                    && matches!(attribute.as_str(), ":block/journal?" | ":block/journal-day")
                    && object.starts_with('?')
                    && !inputs.contains_key(&object)
                {
                    variables.push(object);
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
        let mut local = vec![format!("[{block} :block/page {page}]")];
        local.extend(related.iter().map(|other| groups[*other].clone()));
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
