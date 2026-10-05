//! Exact page constraints narrow execution through the existing name/slot maps.
use super::*;
use crate::model::persistent::Pages;

/// A conservative necessary condition, never a replacement for evaluation.
/// Negation and partially constrained ORs must retain the whole candidate set.
fn required_names(filter: &Filter, anchor: Anchor) -> Option<Vec<String>> {
    match filter {
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Page,
                    quant: Quant::Any | Quant::Every,
                    pred,
                },
        } if anchor == Anchor::Block => {
            eval::single_ref_name(pred).map(|name| vec![refs::page_key(name)])
        }
        Filter::Leaf { .. } if anchor == Anchor::Page => {
            eval::single_ref_name(filter).map(|name| vec![refs::page_key(name)])
        }
        Filter::And { items } => items
            .iter()
            .find_map(|filter| required_names(filter, anchor)),
        Filter::Or { items } if !items.is_empty() => items
            .iter()
            .map(|filter| required_names(filter, anchor))
            .collect::<Option<Vec<_>>>()
            .map(|names| names.concat()),
        _ => None,
    }
}

pub(super) struct Candidates<'a> {
    pages: &'a Pages,
    selected: Option<Vec<usize>>,
    index: Option<Arc<QueryIndex>>,
    config: ParseConfig,
}

impl<'a> Candidates<'a> {
    pub(super) fn new(graph: &impl GraphRead, pages: &'a Pages, plan: &Plan) -> Self {
        let names = required_names(&plan.filter, plan.anchor);
        let selected = names.as_ref().map(|names| {
            let named = graph
                .reference_real_page_names()
                .expect("published query inputs have their real page names");
            // Physical paths and relative slot keys belong to one graph. Derive
            // its root from an existing entry without crossing the store door.
            let root = pages.iter().next().and_then(|(entry, _)| {
                entry.path.ancestors().nth(
                    std::path::Path::new(entry.rel_path_str())
                        .components()
                        .count(),
                )
            });
            let mut slots = std::collections::BTreeSet::new();
            if let Some(root) = root {
                for name in names {
                    for path in named.paths(name) {
                        if let Ok(rel) = path.strip_prefix(root) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            if let Some(&slot) = pages.positions.get(rel.as_str()) {
                                slots.insert(slot);
                            }
                        }
                    }
                }
            }
            slots.into_iter().collect::<Vec<_>>()
        });
        // Global property coercions and used-as-tag predicates still need their
        // graph-wide facts. Other named-page queries derive only selected facts.
        let index = (selected.is_none() || plan.registry.is_some() || plan.tag_targets.is_some())
            .then(|| graph.query_index());
        Self {
            pages,
            selected,
            config: index.as_ref().map_or_else(
                || ParseConfig::from_config(graph.config()),
                |index| index.parse_config().clone(),
            ),
            index,
        }
    }

    pub(super) fn pages(&self) -> impl Iterator<Item = &'a (PageEntry, Arc<Document>)> + '_ {
        let all = self
            .selected
            .is_none()
            .then(|| self.pages.iter())
            .into_iter()
            .flatten();
        let selected = self
            .selected
            .iter()
            .flat_map(|slots| slots.iter().filter_map(|&slot| self.pages.get(slot)));
        all.chain(selected)
    }

    pub(super) fn facts(&self, entry: &PageEntry, doc: &Document) -> Arc<PageFacts> {
        self.index.as_ref().map_or_else(
            || Arc::new(PageFacts::of(entry, doc, &self.config)),
            |index| index.facts(entry, doc),
        )
    }

    pub(super) fn config(&self) -> &ParseConfig {
        &self.config
    }
}
