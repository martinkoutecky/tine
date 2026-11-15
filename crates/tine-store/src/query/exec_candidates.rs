//! Exact page constraints and property presence narrow execution through the
//! existing name/slot maps and the query index's key -> pages table. A
//! conjunction narrows by its tightest constraint, never by evaluation order.
use super::*;
use crate::model::persistent::Pages;

/// One condition on a row's page.
enum Need {
    /// Carry this page name (as written).
    Name(String),
    /// Hold a row of this property key ([`property_key_norm`]), on the page
    /// or on one of its blocks.
    Key(String),
}

/// What every row's page must satisfy: a conservative necessary condition,
/// never a replacement for evaluation. Negation and partially
/// constrained ORs must retain the whole candidate set. A `props` presence
/// (`(property k)`, `(page-property k)`, `any(props, key = 'k' …)`) holds only
/// where the row (a block, or the page itself) has a row of `k`.
enum Req {
    Need(Need),
    /// One of these holds (an `or` whose every branch narrows).
    Any(Vec<Req>),
    /// Each of these holds (an `and` with several narrowing conjuncts). It
    /// narrows to the tightest of them, whatever their evaluation order.
    All(Vec<Req>),
}

impl Req {
    fn has_key(&self) -> bool {
        match self {
            Req::Need(need) => matches!(need, Need::Key(_)),
            Req::Any(items) | Req::All(items) => items.iter().any(Req::has_key),
        }
    }
}

fn required(filter: &Filter, anchor: Anchor) -> Option<Req> {
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
            .map(|key| Req::Need(Need::Key(tine_core::doc::property_key_norm(&key)))),
        Filter::Leaf {
            leaf:
                Leaf::Rel {
                    rel: Rel::Page,
                    quant: Quant::Any | Quant::Every,
                    pred,
                },
        } if anchor == Anchor::Block => {
            eval::single_ref_name(pred).map(|name| Req::Need(Need::Name(name.to_owned())))
        }
        Filter::Leaf { .. } if anchor == Anchor::Page => {
            eval::single_ref_name(filter).map(|name| Req::Need(Need::Name(name.to_owned())))
        }
        Filter::And { items } => {
            let mut reqs: Vec<Req> = items
                .iter()
                .filter_map(|filter| required(filter, anchor))
                .collect();
            match reqs.len() {
                0 | 1 => reqs.pop(),
                _ => Some(Req::All(reqs)),
            }
        }
        Filter::Or { items } if !items.is_empty() => items
            .iter()
            .map(|filter| required(filter, anchor))
            .collect::<Option<Vec<_>>>()
            .map(Req::Any),
        _ => None,
    }
}

/// The page keys a name need probes. A date spelling names its journal
/// (S12, `eval_page_name`): the journal is named by the configured title,
/// so its key joins the probe. The one date answerer reads the spelling.
fn name_keys(name: &str, format: &tine_core::date::JournalFormat) -> impl Iterator<Item = String> {
    let journal = eval::reference_day(name, format)
        .and(format.parse(name))
        .map(|day| refs::page_key(&format.title(day)));
    std::iter::once(refs::page_key(name)).chain(journal)
}

/// The page keys a fileless row meeting `req` must carry: such a row has
/// no properties, so it meets only a name need, whether or not a key is
/// indexed. An `and` intersects its conjuncts' keys.
fn fileless_names(
    req: &Req,
    format: &tine_core::date::JournalFormat,
) -> std::collections::BTreeSet<String> {
    match req {
        Req::Need(Need::Key(_)) => Default::default(),
        Req::Need(Need::Name(name)) => name_keys(name, format).collect(),
        Req::Any(items) => items
            .iter()
            .flat_map(|req| fileless_names(req, format))
            .collect(),
        Req::All(items) => items
            .iter()
            .map(|req| fileless_names(req, format))
            .reduce(|kept, next| kept.intersection(&next).cloned().collect())
            .unwrap_or_default(),
    }
}

impl Plan {
    /// Whether a fileless row (a reference-only page or a namespace parent)
    /// keyed by one of `keys` can be one of this plan's rows: the plan is
    /// page-anchored and the row carries a name its required condition
    /// needs, the names execution narrows its fileless rows by. A necessary
    /// condition, never evaluation: a page that starts or stops existing
    /// without a file changes the answer only of a plan this admits. `None`
    /// means unavailable evidence and admits any possible fileless row. O(keys
    /// x log of the filter's names); no page is read.
    pub(crate) fn may_list_fileless(
        &self,
        keys: Option<&[String]>,
        format: &tine_core::date::JournalFormat,
    ) -> bool {
        if self.anchor != Anchor::Page || keys.is_some_and(|keys| keys.is_empty()) {
            return false;
        }
        let Some(req) = required(&self.filter, self.anchor) else {
            return true;
        };
        let names = fileless_names(&req, format);
        keys.map_or_else(
            || !names.is_empty(),
            |keys| keys.iter().any(|key| names.contains(key)),
        )
    }
}

#[cfg(test)]
thread_local! {
    /// Test oracle switch: `false` scans every page despite a key need, the
    /// answer the narrowing must equal.
    pub(super) static NARROW_BY_KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// Candidate pages handed to the row loop since the last reset.
    pub(super) static CANDIDATE_PAGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Fileless rows enumerated (`reference_only_pages`) since the last reset.
    pub(super) static FILELESS_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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

/// Resolves a [`Req`] against one generation through the name/slot maps and
/// the index's key -> pages table.
struct Selector<'a> {
    pages: &'a Pages,
    index: Option<&'a QueryIndex>,
    named: Arc<crate::query::RealPageNames>,
    /// The graph root: physical paths and relative slot keys belong to one
    /// graph, derived from an existing entry without crossing the store door.
    root: Option<&'a std::path::Path>,
    format: tine_core::date::JournalFormat,
}

impl<'a> Selector<'a> {
    fn new(graph: &ReadSnapshot, pages: &'a Pages, index: Option<&'a QueryIndex>) -> Self {
        let root = pages.iter().next().and_then(|(entry, _)| {
            entry.path.ancestors().nth(
                std::path::Path::new(entry.rel_path_str())
                    .components()
                    .count(),
            )
        });
        Self {
            pages,
            index,
            named: graph
                .reference_real_page_names()
                .expect("published query inputs have their real page names"),
            root,
            format: graph.config().journal_format(),
        }
    }

    /// An upper bound on the pages `req` selects, read without walking a
    /// key's holders; `None` when `req` cannot narrow (a key that is not
    /// indexed). O(nodes x depth + named claimants).
    fn bound(&self, req: &Req) -> Option<usize> {
        match req {
            Req::Need(Need::Key(key)) => self.index?.key_holders(key),
            Req::Need(Need::Name(name)) => Some(
                name_keys(name, &self.format)
                    .map(|key| self.named.paths(&key).count())
                    .sum(),
            ),
            Req::Any(items) => items.iter().map(|req| self.bound(req)).sum(),
            Req::All(items) => items.iter().filter_map(|req| self.bound(req)).min(),
        }
    }

    /// The conjunct an `and` narrows by: the smallest bound, the first on a
    /// tie; never an unindexed key.
    fn tightest<'r>(&self, items: &'r [Req]) -> Option<&'r Req> {
        items
            .iter()
            .filter_map(|req| self.bound(req).map(|bound| (bound, req)))
            .min_by_key(|(bound, _)| *bound)
            .map(|(_, req)| req)
    }

    /// Adds the slots of the pages meeting a bounded `req`. O(its bound x
    /// log P).
    fn slots(&self, req: &Req, out: &mut std::collections::BTreeSet<usize>) {
        match req {
            Req::Need(Need::Key(key)) => {
                let holders = self.index.and_then(|index| index.pages_with_key(key));
                for path in holders.into_iter().flatten() {
                    out.extend(self.pages.positions.get(path).copied());
                }
            }
            Req::Need(Need::Name(name)) => {
                let Some(root) = self.root else { return };
                for key in name_keys(name, &self.format) {
                    for path in self.named.paths(&key) {
                        if let Ok(rel) = path.strip_prefix(root) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            out.extend(self.pages.positions.get(rel.as_str()).copied());
                        }
                    }
                }
            }
            Req::Any(items) => items.iter().for_each(|req| self.slots(req, out)),
            Req::All(items) => {
                if let Some(req) = self.tightest(items) {
                    self.slots(req, out);
                }
            }
        }
    }

    /// The slots of the pages meeting `req` and the page keys a fileless row
    /// must carry; `None` when `req` cannot narrow, so every page stays a
    /// candidate.
    fn select(&self, req: &Req) -> Option<(Vec<usize>, Vec<String>)> {
        self.bound(req)?;
        let mut slots = std::collections::BTreeSet::new();
        self.slots(req, &mut slots);
        Some((
            slots.into_iter().collect(),
            fileless_names(req, &self.format).into_iter().collect(),
        ))
    }
}

impl<'a> Candidates<'a> {
    pub(super) fn new(graph: &ReadSnapshot, pages: &'a Pages, plan: &Plan) -> Self {
        let req = required(&plan.filter, plan.anchor);
        #[cfg(test)]
        let req = req.filter(|req| NARROW_BY_KEYS.with(std::cell::Cell::get) || !req.has_key());
        // Global property coercions and used-as-tag predicates still need their
        // graph-wide facts. Other named-page queries derive only selected facts.
        // Link conditions evaluate their element and seed conditions over
        // every page, so they read every page's facts too. Key needs read the
        // index's key -> pages table.
        let index = (req.as_ref().is_none_or(Req::has_key)
            || plan.registry.is_some()
            || plan.tag_targets.is_some()
            || plan.reads_links())
        .then(|| graph.query_index());
        let selected = req
            .as_ref()
            .and_then(|req| Selector::new(graph, pages, index.as_deref()).select(req));
        let index = index.or_else(|| selected.is_none().then(|| graph.query_index()));
        let (selected, names) = selected.unzip();
        // A fileless row has no properties, so it meets only a name need. A
        // narrowing no name key survives (property presence alone, or in a
        // conjunction) leaves none of them a candidate: skip the walk of
        // every referenced name.
        let fileless_possible = names.as_ref().is_none_or(|names| !names.is_empty());
        let reference_only = if plan.anchor == Anchor::Page && fileless_possible {
            let wanted: Option<std::collections::HashSet<&str>> = names
                .as_ref()
                .map(|names| names.iter().map(String::as_str).collect());
            let empty = Arc::new(Document::default());
            let rows = graph.reference_only_pages();
            #[cfg(test)]
            FILELESS_ROWS.with(|count| count.set(count.get() + rows.len()));
            rows.into_iter()
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
