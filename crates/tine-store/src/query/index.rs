//! Per-generation query facts: each page's own properties (parsed once, not
//! once per query) and the property registry the typed comparisons and TQL
//! diagnostics read (SPEC §6.2).
//!
//! Persisted only inside the launch checkpoint, as part of the generation it
//! belongs to (ADR 0070, `index_checkpoint.rs`); memory O(pages with a
//! preamble) plus one registry row per property key. A snapshot builds its
//! index lazily on the first query, off the UI thread (every query command runs
//! in `spawn_blocking`). A snapshot published after an edit inherits the
//! previous index as a SEED plus the changed paths; the first query of the new
//! generation re-derives only those pages. The registry is carried unchanged
//! when no changed page's property rows moved. Otherwise the next use PATCHES it
//! per key: each page records a digest of the rows it holds per key, an
//! inverted key -> pages table names the pages a moved key lives on, and only
//! those keys' rows are rebuilt by the one producer (`build_registry`) and
//! folded into the previous registry (`patch_registry`). Unit cost of a
//! property edit: O(the edited page + the pages and rows of the keys it
//! changed), never O(pages in the graph); a text-only edit reads no property
//! rows at all. A registry never built is built whole, once, on first use.

use crate::model::persistent::{Map as SharedMap, Pages};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use tine_core::config::Config;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::model::{Format, PageEntry};
use tine_core::query::atom::{AtomFormat, ParseConfig};
use tine_core::query::registry::{
    build_registry, is_internal_key, patch_registry, OwnerRow, OwnerType, PageMeta, Registry,
    DECLARED_TYPE_KEY,
};

#[path = "index_checkpoint.rs"]
pub(crate) mod checkpoint;

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
    /// For every registry-relevant property key (every non-internal key, plus
    /// the `tine.type` declaration key), a digest of the rows this page holds
    /// for it: owner, ordinal, spelling and value. Sorted by key. A key whose
    /// digest differs between two generations of the page is a key the
    /// registry must re-derive.
    keys: Box<[(String, u64)]>,
    /// The page key a `tine.type::` on this page declares a type for (the
    /// page's own name); `None` when the page carries no declaration.
    declares: Option<String>,
    /// Every normalized page reference of every block on the page, sorted and
    /// de-duplicated: the page-level half of `:block/path-refs`, which is what
    /// lets a page-ref query skip a page without walking it.
    refs: Box<[String]>,
    /// The page's header-style property block as a block-anchored query row
    /// (OG's `:block/pre-block?` block, which every block predicate sees):
    /// built once per page generation, never per query. `None` when the page
    /// has no header properties. Unit cost: one `DocBlock` of the page's
    /// header property text (O(header bytes), the same text the backlink
    /// path projects per query) per facts derivation, i.e. per page edit
    /// that reaches the query index; zero extra work per query.
    page_property_block: Option<DocBlock>,
}

impl PageFacts {
    pub(crate) fn of(entry: &PageEntry, doc: &Document, config: &ParseConfig) -> PageFacts {
        #[cfg(test)]
        DERIVED_FACT_PAGES.with(|count| count.set(count.get() + 1));
        #[cfg(feature = "test-faults")]
        crate::cost_counters::query_facts_derived();
        let (properties, tags) = super::page_facets(doc);
        let mut per_key: BTreeMap<String, DefaultHasher> = BTreeMap::new();
        let mut declares = false;
        let mut note = |owner: &str, ordinal: usize, key: &str, value: &str, page_level: bool| {
            let normalized = property_key_norm(key);
            let declaration = page_level && normalized == DECLARED_TYPE_KEY;
            if normalized.is_empty() || (!declaration && is_internal_key(&normalized, config)) {
                return;
            }
            declares |= declaration;
            (page_level, owner, ordinal, key, value).hash(per_key.entry(normalized).or_default());
        };
        for (ordinal, (key, value)) in properties.iter().enumerate() {
            note("", ordinal, key, value, true);
        }
        fn blocks(
            roots: &[DocBlock],
            note: &mut impl FnMut(&str, usize, &str, &str, bool),
            refs: &mut Vec<String>,
        ) {
            for block in roots {
                let projection = block.projection();
                for (ordinal, (key, value)) in projection.properties().iter().enumerate() {
                    note(&block.uuid, ordinal, key, value, false);
                }
                refs.extend(projection.refs_norm().iter().cloned());
                blocks(&block.children, note, refs);
            }
        }
        let mut refs = Vec::new();
        let page_property_block = doc
            .pre_block
            .as_deref()
            .and_then(|pre| super::page_property_block(entry, pre));
        if let Some(block) = &page_property_block {
            refs.extend(block.projection().refs_norm().iter().cloned());
        }
        blocks(&doc.roots, &mut note, &mut refs);
        refs.sort_unstable();
        refs.dedup();
        PageFacts {
            properties: properties.into_boxed_slice(),
            tags: tags.into_boxed_slice(),
            keys: per_key
                .into_iter()
                .map(|(key, hasher)| (key, hasher.finish()))
                .collect(),
            declares: declares.then(|| tine_core::refs::page_key(&entry.name)),
            refs: refs.into_boxed_slice(),
            page_property_block,
        }
    }

    /// Whether the registry rows this page holds differ from `other`'s: a
    /// property row of any registry-relevant key, or the declaration it makes.
    pub(crate) fn registry_rows_differ(&self, other: &PageFacts) -> bool {
        self.keys != other.keys || self.declares != other.declares
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

    /// The header page-property block, the synthetic first block of the page
    /// for block-anchored queries (see the field).
    pub(crate) fn page_property_block(&self) -> Option<&DocBlock> {
        self.page_property_block.as_ref()
    }

    /// The page's own tags (the preamble's `tags::` values).
    pub(crate) fn tags(&self) -> &[String] {
        &self.tags
    }
}

pub(crate) fn atom_format(entry: &PageEntry) -> AtomFormat {
    Format::from_path(&entry.path).into()
}

/// An inverted table (member key -> the paths holding it), shared with earlier
/// generations like [`FactsMap`]: a bounded delta over an `Arc`'d base, so a
/// patch copies the delta and the sets of the keys whose membership moved,
/// never the table.
#[derive(Clone, Default)]
struct Postings {
    base: Arc<HashMap<String, Arc<BTreeSet<String>>>>,
    delta: HashMap<String, Arc<BTreeSet<String>>>,
}

impl Postings {
    fn get(&self, key: &str) -> Option<&Arc<BTreeSet<String>>> {
        let set = self.delta.get(key).or_else(|| self.base.get(key))?;
        (!set.is_empty()).then_some(set)
    }

    /// Every key that has at least one member. O(keys), used only when a
    /// declaration moved (a key is matched to its page by `page_key`).
    fn keys(&self) -> Vec<&str> {
        fn live<'a>((key, set): (&'a String, &Arc<BTreeSet<String>>)) -> Option<&'a str> {
            (!set.is_empty()).then_some(key.as_str())
        }
        self.delta
            .iter()
            .filter_map(live)
            .chain(
                self.base
                    .iter()
                    .filter(|(key, _)| !self.delta.contains_key(*key))
                    .filter_map(live),
            )
            .collect()
    }

    /// This table with `(key, path, added)` applied.
    fn applied(&self, changes: &[(String, String, bool)]) -> Postings {
        let mut delta = self.delta.clone();
        for (key, path, added) in changes {
            let current = delta.get(key).or_else(|| self.base.get(key));
            let mut set = current.map(|set| (**set).clone()).unwrap_or_default();
            if *added {
                set.insert(path.clone());
            } else {
                set.remove(path);
            }
            delta.insert(key.clone(), Arc::new(set));
        }
        if delta.len() <= FACTS_DELTA_MAX {
            return Postings {
                base: Arc::clone(&self.base),
                delta,
            };
        }
        let mut base = (*self.base).clone();
        for (key, set) in delta {
            if set.is_empty() {
                base.remove(&key);
            } else {
                base.insert(key, set);
            }
        }
        Postings {
            base: Arc::new(base),
            delta: HashMap::new(),
        }
    }
}

/// What a generation's pages moved in the registry's inputs, relative to the
/// generation before: the keys to re-derive, the declared page keys whose
/// `tine.type::` moved, and the membership changes for the two inverted tables.
#[derive(Default)]
struct RegistryDelta {
    keys: BTreeSet<String>,
    declared: BTreeSet<String>,
    key_members: Vec<(String, String, bool)>,
    declarers: Vec<(String, String, bool)>,
}

impl RegistryDelta {
    fn note(&mut self, path: &str, before: Option<&PageFacts>, after: Option<&PageFacts>) {
        let (b, a) = (
            before.map_or(&[][..], |f| &f.keys[..]),
            after.map_or(&[][..], |f| &f.keys[..]),
        );
        let (mut i, mut j) = (0, 0);
        while i < b.len() || j < a.len() {
            let order = match (b.get(i), a.get(j)) {
                (Some(x), Some(y)) => x.0.cmp(&y.0),
                (Some(_), None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            let (key, was, is) = match order {
                std::cmp::Ordering::Less => {
                    i += 1;
                    (&b[i - 1].0, Some(b[i - 1].1), None)
                }
                std::cmp::Ordering::Greater => {
                    j += 1;
                    (&a[j - 1].0, None, Some(a[j - 1].1))
                }
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                    (&b[i - 1].0, Some(b[i - 1].1), Some(a[j - 1].1))
                }
            };
            if was == is || key == DECLARED_TYPE_KEY {
                continue;
            }
            self.keys.insert(key.clone());
            if was.is_none() || is.is_none() {
                self.key_members
                    .push((key.clone(), path.to_owned(), is.is_some()));
            }
        }
        let (was, is) = (
            before.and_then(|f| f.declares.clone()),
            after.and_then(|f| f.declares.clone()),
        );
        let declaration_moved = was != is
            || is.is_some()
                && a.iter().find(|k| k.0 == DECLARED_TYPE_KEY)
                    != b.iter().find(|k| k.0 == DECLARED_TYPE_KEY);
        if declaration_moved {
            if was != is {
                if let Some(key) = &was {
                    self.declarers.push((key.clone(), path.to_owned(), false));
                }
                if let Some(key) = &is {
                    self.declarers.push((key.clone(), path.to_owned(), true));
                }
            }
            self.declared.extend(was.into_iter().chain(is));
        }
    }

    fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.declared.is_empty()
    }
}

/// A registry inherited from an earlier generation plus the keys whose rows
/// the intervening edits moved: the registry of this generation is that one
/// with those keys re-derived, on first use (see the module doc).
struct Pending {
    base: Arc<Registry>,
    keys: BTreeSet<String>,
    declared: BTreeSet<String>,
}

/// The query facts of one snapshot generation.
pub(crate) struct QueryIndex {
    facts: FactsMap,
    /// Normalized property key -> pages holding a row of it (never the
    /// declaration key).
    key_pages: Postings,
    /// Page key -> pages whose `tine.type::` declares a type for it.
    declarers: Postings,
    parse_config: ParseConfig,
    generation: u64,
    /// This snapshot's path -> slot in its page vector.
    positions: Arc<SharedMap<String, usize>>,
    registry: OnceLock<Arc<Registry>>,
    pending: Option<Pending>,
}

impl QueryIndex {
    pub(crate) fn build(
        pages: &Pages,
        config: &Config,
        generation: u64,
        positions: Arc<SharedMap<String, usize>>,
    ) -> QueryIndex {
        #[cfg(test)]
        BUILT_FACT_PAGES.with(|count| count.set(count.get() + pages.len()));
        let parse_config = ParseConfig::from_config(config);
        let mut facts = HashMap::with_capacity(pages.len());
        let mut delta = RegistryDelta::default();
        for (entry, doc) in pages {
            let path = entry.rel_path_str();
            let page_facts = Arc::new(PageFacts::of(entry, doc, &parse_config));
            delta.note(path, None, Some(&page_facts));
            facts.insert(path.to_owned(), page_facts);
        }
        let group = |changes: &[(String, String, bool)]| {
            let mut by_key: HashMap<String, Arc<BTreeSet<String>>> = HashMap::new();
            let mut sets: HashMap<&str, BTreeSet<String>> = HashMap::new();
            for (key, path, _) in changes {
                sets.entry(key).or_default().insert(path.clone());
            }
            for (key, set) in sets {
                by_key.insert(key.to_owned(), Arc::new(set));
            }
            Postings {
                base: Arc::new(by_key),
                delta: HashMap::new(),
            }
        };
        QueryIndex {
            facts: FactsMap {
                base: Arc::new(facts),
                delta: HashMap::new(),
            },
            key_pages: group(&delta.key_members),
            declarers: group(&delta.declarers),
            parse_config,
            generation,
            positions,
            registry: OnceLock::new(),
            pending: None,
        }
    }

    /// The seed's index with `changed` re-derived; `page` finds a changed
    /// path's page in the new generation (`None`: removed). Work and bytes are
    /// O(changed pages + delta), not O(graph): the base map is shared, and a
    /// compaction copies it only once per [`FACTS_DELTA_MAX`] changed pages.
    /// The registry is carried when no changed page's registry input moved,
    /// and otherwise patched per moved key on first use.
    pub(crate) fn patched<'p>(
        &self,
        page: impl Fn(&str) -> Option<&'p (PageEntry, Arc<Document>)>,
        changed: &[String],
        generation: u64,
        positions: Arc<SharedMap<String, usize>>,
    ) -> QueryIndex {
        let mut delta = self.facts.delta.clone();
        #[cfg(feature = "test-faults")]
        crate::cost_counters::query_facts_copies(delta.len() as u64);
        let mut moved = RegistryDelta::default();
        for path in changed {
            let before = self.facts.get(path).cloned();
            let current = page(path)
                .map(|(entry, doc)| Arc::new(PageFacts::of(entry, doc, &self.parse_config)));
            moved.note(path, before.as_deref(), current.as_deref());
            delta.insert(path.clone(), current);
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
        let pending = match (self.registry.get(), &self.pending) {
            (Some(built), _) if moved.is_empty() => {
                let _ = registry.set(Arc::clone(built));
                None
            }
            (Some(built), _) => Some(Pending {
                base: Arc::clone(built),
                keys: moved.keys,
                declared: moved.declared,
            }),
            (None, Some(pending)) => Some(Pending {
                base: Arc::clone(&pending.base),
                keys: pending.keys.iter().chain(&moved.keys).cloned().collect(),
                declared: pending
                    .declared
                    .iter()
                    .chain(&moved.declared)
                    .cloned()
                    .collect(),
            }),
            (None, None) => None,
        };
        QueryIndex {
            facts,
            key_pages: self.key_pages.applied(&moved.key_members),
            declarers: self.declarers.applied(&moved.declarers),
            parse_config: self.parse_config.clone(),
            generation,
            positions,
            registry,
            pending,
        }
    }

    /// This generation's facts of one page. A page the index does not know
    /// (never expected: the index is built from the same page slice) is derived
    /// on the spot rather than read as a page with no properties or refs.
    pub(crate) fn facts(&self, entry: &PageEntry, doc: &Document) -> Arc<PageFacts> {
        self.facts
            .get(entry.rel_path_str())
            .cloned()
            .unwrap_or_else(|| Arc::new(PageFacts::of(entry, doc, &self.parse_config)))
    }

    pub(crate) fn parse_config(&self) -> &ParseConfig {
        &self.parse_config
    }

    /// The property registry of this generation, built (or patched from the
    /// inherited one) on first use.
    pub(crate) fn registry(&self, pages: &Pages) -> Arc<Registry> {
        Arc::clone(self.registry.get_or_init(|| {
            #[cfg(test)]
            REGISTRY_BUILDS.with(|count| count.set(count.get() + 1));
            Arc::new(match &self.pending {
                Some(pending) => self.patched_registry(pending, pages),
                None => self.build_registry(pages),
            })
        }))
    }

    /// Every page key some page's `tags::` names (OG `rules.cljc:96-98`
    /// `[_ :block/tags ?p]`, behind `(all-page-tags)`). One pass over the cached
    /// per-page facts: O(pages + tag values), built per plan that reads it,
    /// nothing persisted.
    pub(crate) fn tag_targets(&self, pages: &Pages) -> HashSet<String> {
        let mut targets = HashSet::new();
        for (entry, doc) in pages {
            for tag in self.facts(entry, doc).tags() {
                targets.insert(tine_core::refs::normalize(tag));
            }
        }
        targets
    }

    /// The page at `path` in this snapshot's vector.
    fn page_at<'a>(&self, pages: &'a Pages, path: &str) -> Option<&'a (PageEntry, Arc<Document>)> {
        self.positions
            .get(path)
            .and_then(|&at| pages.get(at))
            .filter(|(entry, _)| entry.rel_path_str() == path)
    }

    /// The whole registry: the one producer over every page's rows.
    fn build_registry(&self, pages: &Pages) -> Registry {
        let metas: HashMap<&str, PageMeta> = pages
            .iter()
            .map(|(entry, _)| (entry.rel_path_str(), page_meta(entry)))
            .collect();
        let mut rows = Vec::new();
        for (entry, doc) in pages {
            owner_rows(entry, doc, &self.facts(entry, doc), &|_, _| true, &mut rows);
        }
        let page_of = |page_id: &str| metas.get(page_id).cloned();
        match build_registry(rows.into_iter(), &page_of, &self.parse_config) {
            Ok(registry) => registry.with_generation(self.generation),
            // Every row's page came from the same page slice, so this cannot
            // happen; an empty registry types every key as text rather than
            // failing the query.
            Err(_) => Registry::empty(&self.parse_config).with_generation(self.generation),
        }
    }

    /// The inherited registry with each moved key re-derived by the SAME
    /// producer over that key's complete row set (plus the `tine.type::` row of
    /// the page that declares it), read from the pages the inverted tables
    /// name: O(rows of the moved keys), not O(graph).
    fn patched_registry(&self, pending: &Pending, pages: &Pages) -> Registry {
        let mut keys = pending.keys.clone();
        if !pending.declared.is_empty() {
            // A declaration binds a key to the page NAMED like it, under
            // `page_key`; find the keys such a page key covers, in the new
            // generation and in the inherited registry (a key may have lost
            // its last row).
            let covered = |key: &str| pending.declared.contains(&tine_core::refs::page_key(key));
            keys.extend(
                self.key_pages
                    .keys()
                    .into_iter()
                    .chain(
                        pending
                            .base
                            .rows()
                            .iter()
                            .map(|row| row.normalized_name.as_str()),
                    )
                    .filter(|key| covered(key))
                    .map(str::to_owned),
            );
        }
        let mut patches = Vec::with_capacity(keys.len());
        for key in keys {
            let holders = self.key_pages.get(&key);
            let declarers = self.declarers.get(&tine_core::refs::page_key(&key));
            let paths: BTreeSet<&String> = holders
                .into_iter()
                .chain(declarers)
                .flat_map(|set| set.iter())
                .collect();
            let mut rows = Vec::new();
            let mut metas: HashMap<&str, PageMeta> = HashMap::new();
            for path in paths {
                let Some((entry, doc)) = self.page_at(pages, path) else {
                    continue;
                };
                metas.insert(entry.rel_path_str(), page_meta(entry));
                owner_rows(
                    entry,
                    doc,
                    &self.facts(entry, doc),
                    &|normalized, page_level| {
                        normalized == key || page_level && normalized == DECLARED_TYPE_KEY
                    },
                    &mut rows,
                );
            }
            let page_of = |page_id: &str| metas.get(page_id).cloned();
            let row = build_registry(rows.into_iter(), &page_of, &self.parse_config)
                .ok()
                .and_then(|registry| registry.row(&key).cloned());
            patches.push((key, row));
        }
        patch_registry(&pending.base, patches).with_generation(self.generation)
    }
}

fn page_meta(entry: &PageEntry) -> PageMeta {
    PageMeta {
        format: atom_format(entry),
        name: entry.name.clone(),
    }
}

/// Every property row of `entry` whose (normalized key, page-level?) `keep`
/// accepts, in the producer's row shape: the ONE place a page becomes owner
/// rows, for the whole registry and for a patch alike.
fn owner_rows(
    entry: &PageEntry,
    doc: &Document,
    facts: &PageFacts,
    keep: &dyn Fn(&str, bool) -> bool,
    out: &mut Vec<OwnerRow>,
) {
    #[cfg(feature = "test-faults")]
    crate::cost_counters::query_registry_pages_read();
    let page_id = entry.rel_path_str();
    let mut push =
        |owner_type: OwnerType, owner_id: String, ordinal: usize, key: &str, value: &str| {
            let normalized = property_key_norm(key);
            if keep(&normalized, owner_type == OwnerType::Page) {
                out.push(OwnerRow {
                    owner_type,
                    owner_id,
                    page_id: page_id.to_owned(),
                    source_name: key.to_owned(),
                    normalized_name: normalized,
                    ordinal: ordinal as u32,
                    value: value.to_owned(),
                });
            }
        };
    for (ordinal, (key, value)) in facts.properties().iter().enumerate() {
        push(OwnerType::Page, format!("p:{page_id}"), ordinal, key, value);
    }
    fn walk(
        roots: &[DocBlock],
        page_id: &str,
        push: &mut impl FnMut(OwnerType, String, usize, &str, &str),
    ) {
        for block in roots {
            for (ordinal, (key, value)) in block.projection().properties().iter().enumerate() {
                push(
                    OwnerType::Block,
                    format!("b:{page_id}#{}", block.uuid),
                    ordinal,
                    key,
                    value,
                );
            }
            walk(&block.children, page_id, push);
        }
    }
    walk(&doc.roots, page_id, &mut push);
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
        pages: &Pages,
        positions: &Arc<SharedMap<String, usize>>,
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
                    base.patched(page, &changed, generation, Arc::clone(positions))
                }
                _ => QueryIndex::build(pages, config, generation, Arc::clone(positions)),
            })
        }))
    }
}

#[cfg(test)]
thread_local! {
    static REGISTRY_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static BUILT_FACT_PAGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static DERIVED_FACT_PAGES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn registry_builds() -> usize {
    REGISTRY_BUILDS.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator: no dependency, reproducible failures.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % bound
        }
    }

    const KEYS: [&str; 6] = [
        "status", "Rating", "due-date", "some_key", "Some-Key", "tags2",
    ];
    const VALUES: [&str; 7] = [
        "open",
        "3",
        "4.5",
        "2026-01-05",
        "[[Alpha]]",
        "a, b",
        "true",
    ];
    const NAMES: [&str; 6] = ["Alpha", "rating", "Some Key", "status", "Beta", "due date"];

    fn entry(name: &str) -> PageEntry {
        PageEntry {
            name: name.into(),
            kind: tine_core::model::PageKind::Page,
            date_key: None,
            rel_path: Some(format!("pages/{name}.md").into()),
            path: format!("pages/{name}.md").into(),
        }
    }

    /// A page's text: an optional page-property block (possibly a declaration
    /// of the type of the key named like the page), and blocks with properties.
    fn text(rng: &mut Rng) -> String {
        let mut out = String::new();
        let prop = |rng: &mut Rng| {
            format!(
                "{}:: {}",
                KEYS[rng.next(KEYS.len())],
                VALUES[rng.next(VALUES.len())]
            )
        };
        if rng.next(3) == 0 {
            out.push_str(&format!("{}\n", prop(rng)));
            if rng.next(2) == 0 {
                out.push_str(&format!(
                    "tine.type:: {}\n",
                    ["number", "text", "date", "checkbox"][rng.next(4)]
                ));
            }
            out.push('\n');
        }
        for block in 0..rng.next(4) {
            out.push_str(&format!("- block {block}\n"));
            for _ in 0..rng.next(3) {
                out.push_str(&format!("  {}\n", prop(rng)));
            }
        }
        out
    }

    fn page(name: &str, rng: &mut Rng) -> (PageEntry, Arc<Document>) {
        let entry = entry(name);
        let mut doc = tine_core::doc::parse(&text(rng));
        // The store gives every block its runtime identity; owners are keyed by it.
        tine_core::projection::assign_doc_runtime_ids(&mut doc.roots, entry.rel_path_str());
        (entry, Arc::new(doc))
    }

    fn positions(pages: &[(PageEntry, Arc<Document>)]) -> Arc<SharedMap<String, usize>> {
        Arc::new(
            pages
                .iter()
                .enumerate()
                .map(|(at, (entry, _))| (entry.rel_path_str().to_owned(), at))
                .collect(),
        )
    }

    /// A chain of patched indexes (registry built lazily, sometimes never
    /// between edits) always yields the rows a fresh build over the same pages
    /// yields, across property, declaration, add and delete edits.
    #[test]
    fn a_patched_registry_equals_a_fresh_build_after_any_edit_sequence() {
        let config = Config::default();
        let parse_config = ParseConfig::from_config(&config);
        let mut declared_rows = 0;
        for seed in 0..40u64 {
            let mut rng = Rng(seed + 1);
            let mut pages: Vec<(PageEntry, Arc<Document>)> = Vec::new();
            for name in NAMES
                .iter()
                .map(|n| n.to_string())
                .chain((0..6).map(|n| format!("P{n}")))
            {
                pages.push(page(&name, &mut rng));
            }
            let mut index = Arc::new(QueryIndex::build(
                &Pages::from(pages.clone()),
                &config,
                1,
                positions(&pages),
            ));
            index.registry(&Pages::from(pages.clone()));
            for step in 0..25u64 {
                let mut changed = Vec::new();
                for _ in 0..1 + rng.next(3) {
                    match rng.next(5) {
                        0 if pages.len() > 4 => {
                            let at = rng.next(pages.len());
                            changed.push(pages.remove(at).0.rel_path_str().to_owned());
                        }
                        1 => {
                            let name = if rng.next(2) == 0 {
                                NAMES[rng.next(NAMES.len())].to_owned()
                            } else {
                                format!("N{}", rng.next(4))
                            };
                            if !pages.iter().any(|(e, _)| e.name == name) {
                                pages.push(page(&name, &mut rng));
                                changed.push(entry(&name).rel_path_str().to_owned());
                            }
                        }
                        _ => {
                            let at = rng.next(pages.len());
                            let name = pages[at].0.name.clone();
                            pages[at] = page(&name, &mut rng);
                            changed.push(pages[at].0.rel_path_str().to_owned());
                        }
                    }
                }
                let positions = positions(&pages);
                let lookup = |path: &str| {
                    positions
                        .get(path)
                        .and_then(|&at| pages.get(at))
                        .filter(|(entry, _)| entry.rel_path_str() == path)
                };
                index = Arc::new(index.patched(lookup, &changed, 2 + step, Arc::clone(&positions)));
                // Read the registry only after some edits: pending keys must
                // accumulate across unread generations.
                if rng.next(3) != 0 {
                    let fresh = QueryIndex::build(
                        &Pages::from(pages.clone()),
                        &config,
                        2 + step,
                        Arc::clone(&positions),
                    );
                    let (got, want) = (
                        index.registry(&Pages::from(pages.clone())),
                        fresh.registry(&Pages::from(pages.clone())),
                    );
                    assert_eq!(
                        got.rows(),
                        want.rows(),
                        "seed {seed} step {step}: patched registry differs from a fresh build"
                    );
                    assert_eq!(index.parse_config(), &parse_config);
                    declared_rows += want.rows().iter().filter(|r| r.declared.is_some()).count();
                }
            }
        }
        assert!(
            declared_rows > 0,
            "the generator never produced a declared key"
        );
    }
}
