//! Exact page constraints and property presence narrow execution through the
//! existing name/slot maps and the query index's key -> pages table.
use super::*;
use crate::model::persistent::Pages;

/// What one row's page must satisfy, one of a list.
enum Need {
    /// Carry this page name (as written).
    Name(String),
    /// Hold a row of this property key ([`property_key_norm`]), on the page
    /// or on one of its blocks.
    Key(String),
}

/// The needs one of which every row's page must meet. A conservative
/// necessary condition, never a replacement for evaluation. Negation and
/// partially constrained ORs must retain the whole candidate set. A `props`
/// presence (`(property k)`, `(page-property k)`, `any(props, key = 'k' …)`)
/// holds only where the row (a block, or the page itself) has a row of `k`.
fn required(filter: &Filter, anchor: Anchor) -> Option<Vec<Need>> {
    match filter {
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Props,
                    quant: Quant::Any | Quant::Every,
                    pred,
                },
        } => pred
            .props_key()
            .map(|key| vec![Need::Key(tine_core::doc::property_key_norm(&key))]),
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Page,
                    quant: Quant::Any | Quant::Every,
                    pred,
                },
        } if anchor == Anchor::Block => {
            eval::single_ref_name(pred).map(|name| vec![Need::Name(name.to_owned())])
        }
        Filter::Leaf { .. } if anchor == Anchor::Page => {
            eval::single_ref_name(filter).map(|name| vec![Need::Name(name.to_owned())])
        }
        Filter::And { items } => items.iter().find_map(|filter| required(filter, anchor)),
        Filter::Or { items } if !items.is_empty() => items
            .iter()
            .map(|filter| required(filter, anchor))
            .collect::<Option<Vec<_>>>()
            .map(|needs| needs.into_iter().flatten().collect()),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Test oracle switch: `false` scans every page despite a key need, the
    /// answer the narrowing must equal.
    pub(super) static NARROW_BY_KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Candidate pages handed to the row loop since the last reset.
    pub(super) static CANDIDATE_PAGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

/// The slots of the pages meeting one of `needs` and the page keys the name
/// needs probe (a date spelling adds its journal's); `None` when a key need
/// is not an indexed key, so every page stays a candidate. O(needs + their
/// pages x log P).
fn select(
    graph: &ReadSnapshot,
    pages: &Pages,
    needs: &[Need],
    index: Option<&QueryIndex>,
) -> Option<(Vec<usize>, Vec<String>)> {
    let mut slots = std::collections::BTreeSet::new();
    let mut names = Vec::new();
    let format = graph.config().journal_format();
    for need in needs {
        match need {
            Need::Key(key) => {
                for path in index?.pages_with_key(key)? {
                    slots.extend(pages.positions.get(path).copied());
                }
            }
            Need::Name(name) => {
                // A date spelling names its journal (S12, `eval_page_name`):
                // the journal is named by the configured title, so its key
                // joins the probe. The one date answerer reads the spelling.
                let journal = eval::reference_day(name, &format)
                    .and(format.parse(name))
                    .map(|day| refs::page_key(&format.title(day)));
                names.extend(std::iter::once(refs::page_key(name)).chain(journal));
            }
        }
    }
    names.sort();
    names.dedup();
    if !names.is_empty() {
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
        if let Some(root) = root {
            for name in &names {
                for path in named.paths(name) {
                    if let Ok(rel) = path.strip_prefix(root) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        slots.extend(pages.positions.get(rel.as_str()).copied());
                    }
                }
            }
        }
    }
    Some((slots.into_iter().collect(), names))
}

impl<'a> Candidates<'a> {
    pub(super) fn new(graph: &ReadSnapshot, pages: &'a Pages, plan: &Plan) -> Self {
        let needs = required(&plan.filter, plan.anchor);
        #[cfg(test)]
        let needs = needs.filter(|needs| {
            NARROW_BY_KEYS.with(std::cell::Cell::get)
                || !needs.iter().any(|need| matches!(need, Need::Key(_)))
        });
        // Global property coercions and used-as-tag predicates still need their
        // graph-wide facts. Other named-page queries derive only selected facts.
        // Link conditions evaluate their element and seed conditions over
        // every page, so they read every page's facts too. Key needs read the
        // index's key -> pages table.
        let index = (needs.is_none()
            || (needs.iter().flatten()).any(|need| matches!(need, Need::Key(_)))
            || plan.registry.is_some()
            || plan.tag_targets.is_some()
            || plan.reads_links())
        .then(|| graph.query_index());
        let selected = needs
            .as_deref()
            .and_then(|needs| select(graph, pages, needs, index.as_deref()));
        let index = index.or_else(|| selected.is_none().then(|| graph.query_index()));
        let (selected, names) = selected.unzip();
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
        #[cfg(test)]
        let counted = |_: &_| CANDIDATE_PAGES.with(|count| count.set(count.get() + 1));
        #[cfg(not(test))]
        let counted = |_: &_| {};
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
        all.chain(selected).inspect(counted)
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
