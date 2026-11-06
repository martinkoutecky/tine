//! Exact page constraints narrow execution through the existing name/slot maps.
use super::*;
use crate::model::persistent::Pages;

/// The page-name operands (as written) one of which every row's page must
/// carry. A conservative necessary condition, never a replacement for evaluation.
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
            eval::single_ref_name(pred).map(|name| vec![name.to_owned()])
        }
        Filter::Leaf { .. } if anchor == Anchor::Page => {
            eval::single_ref_name(filter).map(|name| vec![name.to_owned()])
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
    /// Page-anchored plans only: the reference-only pages as fileless rows
    /// (no path, no properties, no blocks; D-20261102-CM4 CM-REFONLY and
    /// A15), narrowed by the same required names as the files.
    reference_only: Vec<(PageEntry, Arc<Document>)>,
    index: Option<Arc<QueryIndex>>,
    config: ParseConfig,
}

impl<'a> Candidates<'a> {
    pub(super) fn new(graph: &ReadSnapshot, pages: &'a Pages, plan: &Plan) -> Self {
        let names = required_names(&plan.filter, plan.anchor).map(|names| {
            // A date spelling names its journal (S12, `eval_page_name`): the
            // journal is named by the configured title, so its key joins the
            // probe. The one date answerer reads the spelling.
            let format = graph.config().journal_format();
            let mut names: Vec<String> = names
                .into_iter()
                .flat_map(|name| {
                    let journal = eval::reference_day(&name, &format)
                        .and(format.parse(&name))
                        .map(|day| refs::page_key(&format.title(day)));
                    std::iter::once(refs::page_key(&name)).chain(journal)
                })
                .collect();
            names.sort();
            names.dedup();
            names
        });
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
        // Link conditions evaluate their element and seed conditions over
        // every page, so they read every page's facts too.
        let index = (selected.is_none()
            || plan.registry.is_some()
            || plan.tag_targets.is_some()
            || plan.reads_links())
        .then(|| graph.query_index());
        let reference_only = if plan.anchor == Anchor::Page {
            let wanted: Option<std::collections::HashSet<&str>> = names
                .as_ref()
                .map(|names| names.iter().map(String::as_str).collect());
            let empty = Arc::new(Document::default());
            graph
                .reference_only_pages()
                .into_iter()
                .filter(|answer| {
                    wanted
                        .as_ref()
                        .is_none_or(|wanted| wanted.contains(refs::page_key(&answer.name).as_str()))
                })
                .map(|answer| (answer.page_entry(), Arc::clone(&empty)))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            pages,
            selected,
            reference_only,
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

    /// The page-anchored rows without a file (empty for a block anchor).
    pub(super) fn reference_only(&self) -> impl Iterator<Item = &(PageEntry, Arc<Document>)> + '_ {
        self.reference_only.iter()
    }

    /// Every page of the generation, whatever the narrowing (link conditions
    /// quantify over pages the row filter never visits).
    pub(super) fn all_pages(&self) -> impl Iterator<Item = &'a (PageEntry, Arc<Document>)> + '_ {
        self.pages.iter()
    }

    pub(super) fn facts(&self, entry: &PageEntry, doc: &Document) -> Arc<PageFacts> {
        if entry.rel_path.is_none() {
            // A fileless row has no facts the index could hold.
            return Arc::new(PageFacts::of(entry, doc, &self.config));
        }
        self.index.as_ref().map_or_else(
            || Arc::new(PageFacts::of(entry, doc, &self.config)),
            |index| index.facts(entry, doc),
        )
    }

    pub(super) fn config(&self) -> &ParseConfig {
        &self.config
    }
}
