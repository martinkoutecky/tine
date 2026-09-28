//! Per-generation query facts: each page's own properties (parsed once, not
//! once per query) and the property registry the typed comparisons and TQL
//! diagnostics read (SPEC §6.2).
//!
//! Nothing here is persisted (Unit cost: none on disk; memory O(pages with a
//! preamble) plus one registry row per property key). A snapshot builds its
//! index lazily on the first query, off the UI thread (every query command runs
//! in `spawn_blocking`). A snapshot published after an edit inherits the
//! previous index as a SEED plus the changed paths; the first query of the new
//! generation re-derives only those pages. The registry is carried unchanged
//! when no changed page's property rows or name changed, and is otherwise
//! rebuilt from the whole graph on its next use.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use tine_core::config::Config;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::model::{Format, PageEntry};
use tine_core::query::atom::{AtomFormat, ParseConfig};
use tine_core::query::registry::{build_registry, OwnerRow, OwnerType, PageMeta, Registry};

/// A seed carries at most this many changed paths; beyond it the next index
/// is built from scratch, which costs no more than patching that many pages.
const SEED_MAX_CHANGED_PATHS: usize = 4096;

/// Re-derived pages a generation carries as a delta over the shared base map
/// before one compaction folds them in. Per edit, a patch copies the delta
/// (at most this many entries), never the graph-sized base; the compaction's
/// O(pages) copy recurs at most once per this many changed pages (I-25).
const FACTS_DELTA_MAX: usize = 256;

/// The path → facts map of one generation: a base shared (by `Arc`) with
/// earlier generations plus this generation's bounded delta (`None` = the
/// page is gone).
#[derive(Clone, Default)]
struct FactsMap {
    base: Arc<HashMap<String, Arc<PageFacts>>>,
    delta: HashMap<String, Option<Arc<PageFacts>>>,
}

impl FactsMap {
    fn get(&self, path: &str) -> Option<&Arc<PageFacts>> {
        if !self.delta.is_empty() {
            if let Some(changed) = self.delta.get(path) {
                return changed.as_ref();
            }
        }
        self.base.get(path)
    }
}

/// What one page contributes to query execution beyond its parsed document.
pub(crate) struct PageFacts {
    /// The page's own properties, in source order and spelling, and its
    /// `tags::` values: og's one page-facet reader (`page_facets`), which also
    /// reads Org `#+KEY:` directives as page properties the way OG does
    /// (master's lsdoc preamble projection drops them).
    properties: Box<[(String, String)]>,
    tags: Box<[String]>,
    /// Hash of every property row this page contributes to the registry, plus
    /// the page name (a `tine.type` declaration is keyed by it).
    rows_digest: u64,
    /// Every normalized page reference of every block on the page, sorted and
    /// de-duplicated: the page-level half of `:block/path-refs`, which is what
    /// lets a page-ref query skip a page without walking it.
    refs: Box<[String]>,
}

impl PageFacts {
    pub(crate) fn of(entry: &PageEntry, doc: &Document) -> PageFacts {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::query_facts_derived();
        let (properties, tags) = super::page_facets(doc);
        let mut hasher = DefaultHasher::new();
        entry.name.hash(&mut hasher);
        properties.hash(&mut hasher);
        fn blocks(roots: &[DocBlock], hasher: &mut DefaultHasher, refs: &mut Vec<String>) {
            for block in roots {
                let projection = block.projection();
                if !projection.properties.is_empty() {
                    block.uuid.hash(hasher);
                    projection.properties.hash(hasher);
                }
                refs.extend(projection.refs_norm.iter().cloned());
                blocks(&block.children, hasher, refs);
            }
        }
        let mut refs = Vec::new();
        blocks(&doc.roots, &mut hasher, &mut refs);
        refs.sort_unstable();
        refs.dedup();
        PageFacts {
            properties: properties.into_boxed_slice(),
            tags: tags.into_boxed_slice(),
            rows_digest: hasher.finish(),
            refs: refs.into_boxed_slice(),
        }
    }

    pub(crate) fn rows_digest(&self) -> u64 {
        self.rows_digest
    }

    /// Whether any block of this page (or the page itself, which is in every
    /// block's path-refs closure) can reference one of `names` (normalized).
    pub(crate) fn may_reference(&self, entry: &PageEntry, names: &[String]) -> bool {
        let own = tine_core::refs::normalize(&entry.name);
        names
            .iter()
            .any(|name| *name == own || self.refs.binary_search(name).is_ok())
    }

    /// The page's own `key:: value` properties, in source order and spelling.
    pub(crate) fn properties(&self) -> &[(String, String)] {
        &self.properties
    }

    /// The page's own tags (the preamble's `tags::` values).
    pub(crate) fn tags(&self) -> &[String] {
        &self.tags
    }
}

pub(crate) fn atom_format(entry: &PageEntry) -> AtomFormat {
    Format::from_path(&entry.path).into()
}

/// The query facts of one snapshot generation.
pub(crate) struct QueryIndex {
    facts: FactsMap,
    parse_config: ParseConfig,
    generation: u64,
    registry: OnceLock<Arc<Registry>>,
}

impl QueryIndex {
    pub(crate) fn build(
        pages: &[(PageEntry, Arc<Document>)],
        config: &Config,
        generation: u64,
    ) -> QueryIndex {
        QueryIndex {
            facts: FactsMap {
                base: Arc::new(
                    pages
                        .iter()
                        .map(|(entry, doc)| {
                            (
                                entry.rel_path_str().to_owned(),
                                Arc::new(PageFacts::of(entry, doc)),
                            )
                        })
                        .collect(),
                ),
                delta: HashMap::new(),
            },
            parse_config: ParseConfig::from_config(config),
            generation,
            registry: OnceLock::new(),
        }
    }

    /// The seed's index with `changed` re-derived; `page` finds a changed
    /// path's page in the new generation (`None`: removed). Work and bytes are
    /// O(changed pages + delta), not O(graph): the base map is shared, and a
    /// compaction copies it only once per [`FACTS_DELTA_MAX`] changed pages.
    /// The registry is carried when no changed page's registry input moved.
    pub(crate) fn patched<'p>(
        &self,
        page: impl Fn(&str) -> Option<&'p (PageEntry, Arc<Document>)>,
        changed: &[String],
        generation: u64,
    ) -> QueryIndex {
        let mut delta = self.facts.delta.clone();
        #[cfg(feature = "test-faults")]
        crate::cost_counters::query_facts_copies(delta.len() as u64);
        let mut rows_moved = false;
        for path in changed {
            let before = self.facts.get(path).map(|facts| facts.rows_digest);
            match page(path) {
                Some((entry, doc)) => {
                    let current = Arc::new(PageFacts::of(entry, doc));
                    rows_moved |= before != Some(current.rows_digest);
                    delta.insert(path.clone(), Some(current));
                }
                None => {
                    rows_moved |= before.is_some();
                    delta.insert(path.clone(), None);
                }
            }
        }
        let facts = if delta.len() > FACTS_DELTA_MAX {
            let mut base = (*self.facts.base).clone();
            #[cfg(feature = "test-faults")]
            crate::cost_counters::query_facts_copies(base.len() as u64);
            for (path, facts) in delta {
                match facts {
                    Some(facts) => base.insert(path, facts),
                    None => base.remove(&path),
                };
            }
            FactsMap {
                base: Arc::new(base),
                delta: HashMap::new(),
            }
        } else {
            FactsMap {
                base: Arc::clone(&self.facts.base),
                delta,
            }
        };
        let registry = OnceLock::new();
        if !rows_moved {
            if let Some(carried) = self.registry.get() {
                let _ = registry.set(Arc::clone(carried));
            }
        }
        QueryIndex {
            facts,
            parse_config: self.parse_config.clone(),
            generation,
            registry,
        }
    }

    /// This generation's facts of one page. A page the index does not know
    /// (never expected: the index is built from the same page slice) is derived
    /// on the spot rather than read as a page with no properties or refs.
    pub(crate) fn facts(&self, entry: &PageEntry, doc: &Document) -> Arc<PageFacts> {
        self.facts
            .get(entry.rel_path_str())
            .cloned()
            .unwrap_or_else(|| Arc::new(PageFacts::of(entry, doc)))
    }

    pub(crate) fn parse_config(&self) -> &ParseConfig {
        &self.parse_config
    }

    /// The property registry of this generation, built on first use.
    pub(crate) fn registry(&self, pages: &[(PageEntry, Arc<Document>)]) -> Arc<Registry> {
        Arc::clone(self.registry.get_or_init(|| {
            #[cfg(test)]
            REGISTRY_BUILDS.with(|count| count.set(count.get() + 1));
            Arc::new(self.build_registry(pages))
        }))
    }

    fn build_registry(&self, pages: &[(PageEntry, Arc<Document>)]) -> Registry {
        let metas: HashMap<&str, PageMeta> = pages
            .iter()
            .map(|(entry, _)| {
                (
                    entry.rel_path_str(),
                    PageMeta {
                        format: atom_format(entry),
                        name: entry.name.clone(),
                    },
                )
            })
            .collect();
        let rows = pages.iter().flat_map(|(entry, doc)| {
            let page_id = entry.rel_path_str().to_owned();
            let page_rows = self
                .facts(entry, doc)
                .properties()
                .iter()
                .enumerate()
                .map({
                    let page_id = page_id.clone();
                    move |(ordinal, (key, value))| {
                        owner_row(
                            OwnerType::Page,
                            format!("p:{page_id}"),
                            &page_id,
                            ordinal,
                            key,
                            value,
                        )
                    }
                })
                .collect::<Vec<_>>();
            let mut block_rows = Vec::new();
            fn walk(roots: &[DocBlock], page_id: &str, out: &mut Vec<OwnerRow>) {
                for block in roots {
                    for (ordinal, (key, value)) in block.projection().properties.iter().enumerate()
                    {
                        out.push(owner_row(
                            OwnerType::Block,
                            format!("b:{page_id}#{}", block.uuid),
                            page_id,
                            ordinal,
                            key,
                            value,
                        ));
                    }
                    walk(&block.children, page_id, out);
                }
            }
            walk(&doc.roots, &page_id, &mut block_rows);
            page_rows.into_iter().chain(block_rows)
        });
        let page_of = |page_id: &str| metas.get(page_id).cloned();
        match build_registry(rows, &page_of, &self.parse_config) {
            Ok(registry) => registry.with_generation(self.generation),
            // Every row's page came from the same page slice, so this cannot
            // happen; an empty registry types every key as text rather than
            // failing the query.
            Err(_) => Registry::empty(&self.parse_config).with_generation(self.generation),
        }
    }
}

fn owner_row(
    owner_type: OwnerType,
    owner_id: String,
    page_id: &str,
    ordinal: usize,
    key: &str,
    value: &str,
) -> OwnerRow {
    OwnerRow {
        owner_type,
        owner_id,
        page_id: page_id.to_owned(),
        source_name: key.to_owned(),
        normalized_name: property_key_norm(key),
        ordinal: ordinal as u32,
        value: value.to_owned(),
    }
}

/// One snapshot's query index: built on first use, from the predecessor's
/// index plus the paths changed since it when one was built (or inherited)
/// and the page set is otherwise the same, else from scratch.
#[derive(Default)]
pub(crate) struct QueryIndexSlot {
    built: OnceLock<Arc<QueryIndex>>,
    /// The last BUILT index of an earlier generation and every path changed
    /// since it; taken by the first build.
    seed: Mutex<Option<(Arc<QueryIndex>, Vec<String>)>>,
}

impl QueryIndexSlot {
    /// The slot of the snapshot that follows `previous` (its slot and whether
    /// both share one page vector) after `changed` paths were re-read. An empty
    /// change list over a different page vector is a reload: build afresh.
    pub(crate) fn succeeding(
        previous: Option<(&QueryIndexSlot, bool)>,
        changed: &[String],
    ) -> Self {
        let seed = previous.and_then(|(slot, same_pages)| {
            if changed.is_empty() && !same_pages {
                return None;
            }
            let (base, mut paths) = match (slot.built.get(), slot.seed.lock().unwrap().as_ref()) {
                (Some(built), _) => (Arc::clone(built), Vec::new()),
                (None, Some((base, prior))) => (Arc::clone(base), prior.clone()),
                (None, None) => return None,
            };
            paths.extend(changed.iter().cloned());
            (paths.len() <= SEED_MAX_CHANGED_PATHS).then_some((base, paths))
        });
        QueryIndexSlot {
            built: OnceLock::new(),
            seed: Mutex::new(seed),
        }
    }

    /// This generation's index. `positions` maps a relative path to its slot
    /// in `pages` (the snapshot's own map), so a patch finds each changed page
    /// without scanning or re-keying the graph.
    pub(crate) fn get(
        &self,
        pages: &[(PageEntry, Arc<Document>)],
        positions: &HashMap<String, usize>,
        config: &Config,
        generation: u64,
    ) -> Arc<QueryIndex> {
        Arc::clone(self.built.get_or_init(|| {
            let seed = self.seed.lock().unwrap().take();
            Arc::new(match seed {
                Some((base, changed)) if base.parse_config == ParseConfig::from_config(config) => {
                    let page = |path: &str| {
                        positions
                            .get(path)
                            .and_then(|&at| pages.get(at))
                            .filter(|(entry, _)| entry.rel_path_str() == path)
                    };
                    base.patched(page, &changed, generation)
                }
                _ => QueryIndex::build(pages, config, generation),
            })
        }))
    }
}

#[cfg(test)]
thread_local! {
    static REGISTRY_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn registry_builds() -> usize {
    REGISTRY_BUILDS.with(std::cell::Cell::get)
}
