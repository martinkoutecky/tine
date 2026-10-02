//! Launch checkpoint state (storage spec §7.6, ADR 0070): the graph's half of
//! one published generation — the parsed page cache with its content revisions
//! and observed mtimes, plus the read evaluator's eager indexes — captured for
//! the checkpoint file and installed back at launch without rebuilding any of
//! it. The file format, the writer and the launch path live in
//! `store/checkpoint.rs`. The dump is deliberately dumb: whatever the published
//! generation holds is written and loaded whole, including its lazily built
//! half ([`DerivedState`]: block index, referenced names, alias tables, the
//! query index and both memos) in whatever state it is at capture: built
//! parts are written, unbuilt parts stay unbuilt. A loaded generation is then
//! carried across the launch diff by the same rules as any published one
//! (`ReadSnapshot::capture`, `carry_memos_from`); nothing here decides
//! validity. The graph-level `find_entry_cache` is not part of the published
//! generation and is not written.
use super::*;
use persistent::EntryListParts;
use tine_core::doc::{CheckpointBlock, CheckpointBlocks};

/// The pages of a generation in checkpoint form: each slot's entry, pre-block
/// and block forest (raw text plus the memoized projection).
pub(crate) struct PagesOut(pub(crate) Arc<Pages>);

impl serde::Serialize for PagesOut {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        struct Rows<'a>(&'a Pages);
        impl serde::Serialize for Rows<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut seq = s.serialize_seq(Some(self.0.len()))?;
                for (slot, (entry, doc)) in self.0.slots() {
                    seq.serialize_element(&(
                        slot,
                        entry,
                        &doc.pre_block,
                        CheckpointBlocks(&doc.roots),
                    ))?;
                }
                seq.end()
            }
        }
        let mut out = s.serialize_tuple(2)?;
        out.serialize_element(&self.0.next_slot())?;
        out.serialize_element(&Rows(&self.0))?;
        out.end()
    }
}

/// Owned counterpart of [`PagesOut`].
#[derive(serde::Deserialize)]
pub(crate) struct PagesIn(
    usize,
    Vec<(usize, PageEntry, Option<String>, Vec<CheckpointBlock>)>,
);

impl PagesIn {
    /// The page cache these rows describe under `root`: entry paths rebuilt,
    /// runtime block ids assigned exactly as a parse assigns them. O(blocks).
    pub(crate) fn into_pages(self, root: &Path) -> Arc<Pages> {
        let PagesIn(next, rows) = self;
        let rows: Vec<(usize, (PageEntry, Arc<Document>))> = rows
            .into_iter()
            .map(|(slot, mut entry, pre_block, roots)| {
                if entry.rel_path.is_some() {
                    entry.path = root.join(entry.rel_path_str());
                }
                let mut roots: Vec<DocBlock> =
                    roots.into_iter().map(CheckpointBlock::into_block).collect();
                tine_core::projection::assign_doc_runtime_ids(&mut roots, entry.rel_path_str());
                (slot, (entry, Arc::new(Document { pre_block, roots })))
            })
            .collect();
        Arc::new(Pages::from_slots(rows, next))
    }
}

/// The graph's half of a checkpoint. `P` is [`PagesOut`] when written and
/// [`PagesIn`] when read: one field list, so the positional wire form cannot
/// drift between the two directions.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct GraphState<P> {
    pages: P,
    cache_generation: u64,
    observed_mtimes: Arc<SharedMap<String, std::time::SystemTime>>,
    failures: Vec<String>,
    disk_revs: Vec<(PathBuf, String)>,
    list: EntryListParts,
    explicit_index: SnapshotExplicitIndex,
    reference_candidate_index: SnapshotReferenceCandidateIndex,
    alias_index: Option<SnapshotPageDerivedIndex>,
    real_page_names: Arc<crate::query::RealPageNames>,
    icon_index: Option<Arc<page_icons::IconIndex>>,
    block_ref_counts: Option<Arc<SharedMap<String, usize>>>,
    derived: DerivedState,
}

impl<P> GraphState<P> {
    /// The same state with its pages converted.
    pub(crate) fn map_pages<Q>(self, f: impl FnOnce(P) -> Q) -> GraphState<Q> {
        GraphState {
            pages: f(self.pages),
            cache_generation: self.cache_generation,
            observed_mtimes: self.observed_mtimes,
            failures: self.failures,
            disk_revs: self.disk_revs,
            list: self.list,
            explicit_index: self.explicit_index,
            reference_candidate_index: self.reference_candidate_index,
            alias_index: self.alias_index,
            real_page_names: self.real_page_names,
            icon_index: self.icon_index,
            block_ref_counts: self.block_ref_counts,
            derived: self.derived,
        }
    }

    /// The alias and referenced-name indexes with every entry in shard 0 and
    /// name lists sorted. Shards are chosen by a hash of the absolute page
    /// path, so a golden image over a temporary root would otherwise move
    /// between runs (the loader's header root check keeps real lookups
    /// consistent). Unix only, like the golden test.
    #[cfg(all(test, unix))]
    pub(crate) fn with_alias_shards_merged(mut self) -> Self {
        let indexes = self
            .alias_index
            .iter_mut()
            .chain(self.derived.referenced_name_index.iter_mut());
        for index in indexes {
            let mut all = SharedMap::new();
            for shard in &index.shards {
                for (path, names) in shard.iter() {
                    // Name lists come from hash sets; their order carries no
                    // meaning, and a golden image must not see it.
                    let mut names = names.clone();
                    names.sort();
                    all.insert(path.clone(), names);
                }
            }
            let count = index.shards.len();
            index.shards = std::iter::once(Arc::new(all))
                .chain((1..count).map(|_| Arc::new(SharedMap::new())))
                .collect();
        }
        if let Some(names) = self.derived.referenced_names.as_mut() {
            names.sort();
        }
        self
    }

    /// The same state with every memo filed under `day` (golden images must
    /// not depend on the day the test runs).
    #[cfg(test)]
    pub(crate) fn at_day(mut self, day: i64) -> Self {
        if let Some(cache) = self.derived.derived_cache.as_mut() {
            cache.0.today = day;
        }
        self.derived.query_memo = self.derived.query_memo.map(|memo| memo.at_day(day));
        self
    }

    /// Forget the generation number (tests compare two captures' content).
    #[cfg(test)]
    pub(crate) fn without_generation(mut self) -> Self {
        self.cache_generation = 0;
        self
    }

    /// The disk revision the cached page at `path` was parsed from.
    pub(crate) fn disk_rev(&self, path: &Path) -> Option<&str> {
        self.disk_revs
            .binary_search_by(|(candidate, _)| candidate.as_path().cmp(path))
            .ok()
            .map(|at| self.disk_revs[at].1.as_str())
    }
}

/// Why a published generation was not captured. Each names its in-scope
/// scenario; none is an error, the next idle period tries again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotCaptured {
    /// A cache mutation is not yet published (a save or watcher reconcile in
    /// flight): the generation is not one coherent state.
    Unpublished,
    /// Unreadable files or directories (disk error, permission change, a sync
    /// client holding a file): their state must be re-observed at launch, so
    /// this generation is not checkpointed.
    Unreadable,
}

impl Graph {
    /// Capture the graph's half of `read`, the published evaluator. The caller
    /// holds the store writer, so the cache and its revisions cannot move
    /// while they are cloned (O(pages) for the revision table, O(1) Arcs
    /// otherwise); serialization happens later, off the writer.
    pub(crate) fn checkpoint_capture(
        &self,
        read: &Arc<ReadSnapshot>,
    ) -> Result<GraphState<PagesOut>, NotCaptured> {
        let cache = self.cache.read().unwrap();
        let Some(pages) = cache
            .as_ref()
            .filter(|pages| Arc::ptr_eq(pages, &read.pages))
        else {
            return Err(NotCaptured::Unpublished);
        };
        if read.cache_generation != self.cache_generation() {
            return Err(NotCaptured::Unpublished);
        }
        if !self.unreadable_pages.read().unwrap().is_empty()
            || !self.discovery_errors.read().unwrap().is_empty()
        {
            return Err(NotCaptured::Unreadable);
        }
        let mut disk_revs: Vec<(PathBuf, String)> = self
            .disk_revs
            .read()
            .unwrap()
            .iter()
            .map(|(path, rev)| (path.clone(), rev.clone()))
            .collect();
        disk_revs.sort();
        Ok(GraphState {
            pages: PagesOut(Arc::clone(pages)),
            cache_generation: read.cache_generation,
            observed_mtimes: Arc::clone(&read.observed_mtimes),
            failures: self.page_index_failures.read().unwrap().clone(),
            disk_revs,
            list: read.list.to_parts(),
            explicit_index: read.explicit_index.clone(),
            reference_candidate_index: read.reference_candidate_index.read().unwrap().clone(),
            alias_index: read.alias_index.get().cloned(),
            real_page_names: Arc::clone(&read.real_page_names),
            icon_index: read.icon_index.get().cloned(),
            block_ref_counts: read.block_ref_counts.get().cloned(),
            derived: DerivedState::of(read, &self.root),
        })
    }

    /// Install a loaded checkpoint as this graph's page cache and return the
    /// read evaluator over it, or `None` when a cache already exists (an
    /// on-demand build won the race; the launch then completes as a cold one).
    /// Entry paths are rebuilt under this root and runtime block ids are
    /// reassigned exactly as a parse assigns them. Cost O(blocks) for the ids
    /// plus O(pages) for the path index; nothing is parsed.
    pub(crate) fn checkpoint_install(
        &self,
        state: GraphState<PagesIn>,
        config: Config,
    ) -> Option<(Arc<ReadSnapshot>, Arc<EntryList>)> {
        let root = &self.root;
        let fix = |entry: &mut PageEntry| {
            if entry.rel_path.is_some() {
                entry.path = root.join(entry.rel_path_str());
            }
        };
        let state = state.map_pages(|pages| PagesOut(pages.into_pages(root)));
        let pages = state.pages.0;
        let mut list = state.list;
        list.rows = list
            .rows
            .iter()
            .map(|(slot, entry)| {
                let mut entry = entry.clone();
                fix(&mut entry);
                (*slot, entry)
            })
            .collect();
        let list = Arc::new(EntryList::from_parts(list));
        let mut reference = state.reference_candidate_index;
        reference.positions = Arc::clone(&pages.positions);
        let derived = state.derived;
        // Rows naming a missing owner cannot follow a passing checksum from
        // this writer; were they ever read, the index is left unbuilt (the
        // first block lookup builds it, as at a cold Ready), not refused.
        let block_index = derived
            .block_index
            .and_then(|index| index.into_index(root).ok());
        let query_index = crate::query::index::QueryIndexSlot::from_checkpoint(
            derived.query_index,
            &pages.positions,
        );
        let index = build_page_cache_index(&pages);
        let mut guard = self.cache.write().unwrap();
        if guard.is_some() {
            return None;
        }
        *guard = Some(Arc::clone(&pages));
        *self.observed_mtimes.write().unwrap() = Arc::clone(&state.observed_mtimes);
        *self.page_index_failures.write().unwrap() = state.failures;
        *self.unreadable_pages.write().unwrap() = Arc::new(Vec::new());
        *self.cache_index.write().unwrap() = Some(index);
        *self.disk_revs.write().unwrap() = state.disk_revs.into_iter().collect();
        // The loaded generation keeps its number, so the evaluator, the
        // reference index and the generation-keyed listing agree with it.
        self.cache_gen
            .store(state.cache_generation, std::sync::atomic::Ordering::Release);
        *self.page_list_cache.write().unwrap() = Some((state.cache_generation, list.materialize()));
        drop(guard);
        let read = ReadSnapshot {
            pages,
            config,
            list: Arc::clone(&list),
            observed_mtimes: state.observed_mtimes,
            explicit_index: state.explicit_index,
            reference_candidate_index: RwLock::new(reference),
            cache_generation: state.cache_generation,
            block_index: cell(block_index),
            alias_index: cell(state.alias_index),
            referenced_name_index: cell(derived.referenced_name_index),
            real_page_names: state.real_page_names,
            aliases: cell(derived.aliases),
            alias_owner_paths_by_key: cell(derived.alias_owner_paths_by_key.map(Sorted::into_map)),
            referenced_names: cell(derived.referenced_names),
            block_ref_counts: cell(state.block_ref_counts),
            public_block_ref_counts: cell(derived.public_block_ref_counts.map(|counts| counts.0)),
            icon_index: cell(state.icon_index),
            memos: SnapshotMemos {
                derived_cache: RwLock::new(derived.derived_cache.map(|cache| cache.0)),
                query: crate::query::memo::QueryMemo::from_checkpoint(derived.query_memo),
            },
            query_index,
            #[cfg(test)]
            block_full_builds: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            referenced_name_full_builds: std::sync::atomic::AtomicUsize::new(0),
        };
        Some((Arc::new(read), list))
    }
}

/// The lazily built half of a published generation, as it stands at capture
/// (`None`: not built in that generation). Capture clones `Arc`s and the small
/// alias tables under the store writer; the encoding work (owner interning,
/// key ordering, JSON for result DTOs) happens when it is serialized, off the
/// writer.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct DerivedState {
    block_index: Option<BlockIndexState>,
    referenced_name_index: Option<SnapshotPageDerivedIndex>,
    aliases: Option<Vec<(String, String, String)>>,
    alias_owner_paths_by_key: Option<Sorted<Vec<String>>>,
    referenced_names: Option<Vec<String>>,
    public_block_ref_counts: Option<Sorted<usize>>,
    derived_cache: Option<DerivedCacheState>,
    query_memo: Option<crate::query::memo::checkpoint::MemoState>,
    query_index: crate::query::index::checkpoint::SlotState,
}

impl DerivedState {
    fn of(read: &ReadSnapshot, root: &Path) -> DerivedState {
        DerivedState {
            block_index: read.block_index.get().map(|index| BlockIndexState::Out {
                index: index.clone(),
                root: root.to_path_buf(),
            }),
            referenced_name_index: read.referenced_name_index.get().cloned(),
            aliases: read.aliases.get().cloned(),
            alias_owner_paths_by_key: read
                .alias_owner_paths_by_key
                .get()
                .map(|map| Sorted(Arc::new(map.clone()))),
            referenced_names: read.referenced_names.get().cloned(),
            public_block_ref_counts: read.public_block_ref_counts.get().cloned().map(Sorted),
            derived_cache: read
                .memos
                .derived_cache
                .read()
                .unwrap()
                .clone()
                .map(DerivedCacheState),
            query_memo: read.memos.query.checkpoint_capture(),
            query_index: read.query_index.checkpoint_capture(),
        }
    }
}

/// A string-keyed map written in key order.
pub(crate) struct Sorted<V>(Arc<HashMap<String, V>>);

impl<V: Clone> Sorted<V> {
    fn into_map(self) -> HashMap<String, V> {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }
}

impl<V: serde::Serialize> serde::Serialize for Sorted<V> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut rows: Vec<(&String, &V)> = self.0.iter().collect();
        rows.sort_unstable_by(|a, b| a.0.cmp(b.0));
        rows.serialize(s)
    }
}

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for Sorted<V> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let rows = Vec::<(String, V)>::deserialize(d)?;
        Ok(Sorted(Arc::new(rows.into_iter().collect())))
    }
}

/// The block-id index with each owning page written once (blocks refer to it
/// by number) and its path relative to the graph root, so the image does not
/// depend on where the graph lives; the loader re-joins it under its root.
pub(crate) enum BlockIndexState {
    /// A generation's index at capture.
    Out {
        index: SnapshotBlockIndex,
        root: PathBuf,
    },
    /// The loaded rows, before their owners are joined under the root.
    In(BlockIndexParts),
}

type BlockRows = Vec<(String, Option<u32>)>;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct BlockIndexParts {
    owners: Vec<(PathBuf, String)>,
    base: BlockRows,
    overlay: BlockRows,
}

impl BlockIndexState {
    /// The index under `root`: one shared owner per page, as a build makes.
    /// O(blocks), the same order as decoding the rows.
    fn into_index(self, root: &Path) -> Result<SnapshotBlockIndex, &'static str> {
        let parts = match self {
            BlockIndexState::Out { index, .. } => return Ok(index),
            BlockIndexState::In(parts) => parts,
        };
        let owners: Vec<BlockOwner> = parts
            .owners
            .into_iter()
            .map(|(path, name)| Arc::new((root.join(path), name)))
            .collect();
        let map = |rows: BlockRows| {
            rows.into_iter()
                .map(|(id, number)| match number {
                    Some(n) => owners
                        .get(n as usize)
                        .map(|owner| (id, Some(Arc::clone(owner))))
                        .ok_or("block index names a missing owner"),
                    None => Ok((id, None)),
                })
                .collect::<Result<SharedMap<_, _>, _>>()
                .map(Arc::new)
        };
        Ok(SnapshotBlockIndex {
            base: map(parts.base)?,
            overlay: map(parts.overlay)?,
        })
    }
}

impl serde::Serialize for BlockIndexState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (index, root) = match self {
            BlockIndexState::Out { index, root } => (index, root),
            BlockIndexState::In(parts) => return parts.serialize(s),
        };
        let mut owners = Vec::new();
        let mut numbers: HashMap<*const (PathBuf, String), u32> = HashMap::new();
        let mut rows = |map: &SharedMap<String, Option<BlockOwner>>| -> BlockRows {
            map.iter()
                .map(|(id, owner)| {
                    let number = owner.as_ref().map(|owner| {
                        *numbers.entry(Arc::as_ptr(owner)).or_insert_with(|| {
                            let path = owner.0.strip_prefix(root).unwrap_or(&owner.0);
                            owners.push((path.to_path_buf(), owner.1.clone()));
                            (owners.len() - 1) as u32
                        })
                    });
                    (id.clone(), number)
                })
                .collect()
        };
        let base = rows(&index.base);
        let overlay = rows(&index.overlay);
        BlockIndexParts {
            owners,
            base,
            overlay,
        }
        .serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for BlockIndexState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        BlockIndexParts::deserialize(d).map(BlockIndexState::In)
    }
}

/// The backlink/derived-result memo, results as JSON text (their serde form
/// is JSON-shaped), written in key order.
pub(crate) struct DerivedCacheState(DerivedCache);

#[derive(serde::Serialize, serde::Deserialize)]
struct DerivedCacheParts {
    gen: u64,
    today: i64,
    /// `(key, groups JSON, total, exceeded, bytes)`.
    results: Vec<(String, String, usize, bool, usize)>,
    lru: Vec<String>,
    bytes: usize,
}

impl serde::Serialize for DerivedCacheState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use crate::query::memo::checkpoint::to_json;
        let cache = &self.0;
        let mut keys: Vec<&String> = cache.results.keys().collect();
        keys.sort_unstable();
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            let (groups, bytes) = &cache.results[key];
            results.push((
                key.clone(),
                to_json(groups.groups.as_ref())?,
                groups.total,
                groups.exceeded,
                *bytes,
            ));
        }
        DerivedCacheParts {
            gen: cache.gen,
            today: cache.today,
            results,
            lru: cache.lru.iter().cloned().collect(),
            bytes: cache.bytes,
        }
        .serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for DerivedCacheState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use crate::query::memo::checkpoint::from_json;
        let parts = DerivedCacheParts::deserialize(d)?;
        let mut results = HashMap::with_capacity(parts.results.len());
        for (key, groups, total, exceeded, bytes) in parts.results {
            let groups = BoundedRefGroups {
                groups: Arc::new(from_json(&groups)?),
                total,
                exceeded,
            };
            results.insert(key, (groups, bytes));
        }
        if parts.lru.iter().any(|key| !results.contains_key(key)) {
            return Err(serde::de::Error::custom(
                "derived memo order names a missing result",
            ));
        }
        Ok(DerivedCacheState(DerivedCache {
            gen: parts.gen,
            today: parts.today,
            results,
            lru: parts.lru.into(),
            bytes: parts.bytes,
        }))
    }
}

#[cfg(test)]
impl ReadSnapshot {
    /// Which lazily built parts this generation holds: (block index,
    /// referenced-name index, query index, derived results, query answers).
    pub(crate) fn warm_parts(&self) -> (bool, bool, bool, usize, usize) {
        (
            self.block_index.get().is_some(),
            self.referenced_name_index.get().is_some(),
            self.query_index.checkpoint_capture().is_built(),
            self.memos
                .derived_cache
                .read()
                .unwrap()
                .as_ref()
                .map_or(0, |cache| cache.results.len()),
            self.memos.query.len(),
        )
    }
}

/// What a generation has built lazily so far, for the checkpoint cadence
/// (ADR 0070, Martin 2026-10-02: a lazily built index or memo counts as a
/// change). One bit per lazily built slot the checkpoint writes, plus the two
/// memos' entry and byte counts. Reading it takes no build and clones nothing.
/// `checkpoint_config_key_tests::lazy_marks_cover_every_lazy_slot_the_checkpoint_writes`
/// keeps it in step with [`DerivedState::of`] and `checkpoint_capture`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LazyMarks {
    built: u16,
    derived: (usize, usize),
    query: (usize, usize),
}

impl LazyMarks {
    pub(crate) fn of(read: &ReadSnapshot) -> LazyMarks {
        let slots = [
            read.block_index.get().is_some(),
            read.alias_index.get().is_some(),
            read.referenced_name_index.get().is_some(),
            read.aliases.get().is_some(),
            read.alias_owner_paths_by_key.get().is_some(),
            read.referenced_names.get().is_some(),
            read.block_ref_counts.get().is_some(),
            read.public_block_ref_counts.get().is_some(),
            read.icon_index.get().is_some(),
            read.query_index.is_built(),
        ];
        let built = slots
            .iter()
            .enumerate()
            .fold(0u16, |bits, (at, &set)| bits | (u16::from(set) << at));
        let derived = read
            .memos
            .derived_cache
            .read()
            .unwrap()
            .as_ref()
            .map_or((0, 0), |cache| (cache.results.len(), cache.bytes));
        LazyMarks {
            built,
            derived,
            query: read.memos.query.checkpoint_marks(),
        }
    }

    /// Whether this holds lazily built state that `base` (what was last
    /// written or loaded) lacks: a slot built since, or a memo that changed
    /// and is not empty. Only shrinking (a later generation starting cold) is
    /// not new state. A memo that churned to the same entry and byte counts
    /// is missed; that costs a less warm launch, never a wrong answer.
    pub(crate) fn grew_since(&self, base: &LazyMarks) -> bool {
        self.built & !base.built != 0
            || (self.derived != base.derived && self.derived.0 > 0)
            || (self.query != base.query && self.query.0 > 0)
    }
}

fn cell<T>(value: Option<T>) -> std::sync::OnceLock<T> {
    let cell = std::sync::OnceLock::new();
    if let Some(value) = value {
        let _ = cell.set(value);
    }
    cell
}
