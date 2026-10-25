//! Launch checkpoint state (storage spec §7.6, ADR 0070): the graph's half of
//! one published generation — the parsed page cache with its content revisions
//! and observed mtimes, plus the read evaluator's eager indexes — captured for
//! the checkpoint file and installed back at launch without rebuilding any of
//! it. The file format, the writer and the launch path live in
//! `store/checkpoint.rs`. The dump is deliberately dumb: whatever the published
//! generation holds is written and loaded whole, including its lazily built
//! half ([`DerivedState`]: referenced names, alias tables, the
//! query index and both memos) in whatever state it is at capture: built
//! parts are written, unbuilt parts stay unbuilt. A loaded generation is then
//! carried across the launch diff by the same rules as any published one
//! (`ReadSnapshot::capture`, `carry_memos_from`); nothing here decides
//! validity. The graph-level `find_entry_cache` is not part of the published
//! generation and is not written.
use super::*;
use persistent::EntryListParts;
use tine_core::doc::{CheckpointBlock, CheckpointBlocks};
mod block_index;
use block_index::BlockIndexState;

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
    /// The cached pages whose bytes carried a VCS anchor line (sorted; empty
    /// for a graph with no conflict markers): see `Graph::vcs_anchored`.
    vcs_anchored: Vec<PathBuf>,
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
            vcs_anchored: self.vcs_anchored,
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

    #[cfg(test)]
    pub(crate) fn invalidate_block_owner(&mut self) {
        self.derived.block_index = BlockIndexState::In(SnapshotBlockIndex {
            base: Arc::new(
                [("invalid-owner".to_owned(), BlockOwners::One(usize::MAX))]
                    .into_iter()
                    .collect(),
            ),
            overlay: Arc::new(SharedMap::new()),
        });
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
        let (failures, disk_revs, vcs_anchored) =
            self.checkpoint_rows(&read.pages, read.cache_generation)?;
        Ok(GraphState {
            pages: PagesOut(Arc::clone(&read.pages)),
            cache_generation: read.cache_generation,
            observed_mtimes: Arc::clone(&read.observed_mtimes),
            failures,
            disk_revs,
            vcs_anchored,
            list: read.list.to_parts(),
            explicit_index: read.explicit_index.clone(),
            reference_candidate_index: read.reference_candidate_index.read().unwrap().clone(),
            alias_index: read.alias_index.get().cloned(),
            real_page_names: Arc::clone(&read.real_page_names),
            icon_index: read.icon_index.get().cloned(),
            block_ref_counts: read.block_ref_counts.get().cloned(),
            derived: DerivedState::of(read),
        })
    }

    /// Install a loaded checkpoint as this graph's page cache and return the
    /// read evaluator over it, or `None` when a cache already exists (an
    /// on-demand build won the race) or a page is held (its rows are its
    /// owner's, R2); the launch then completes as a cold one.
    /// Entry paths are rebuilt under this root and runtime block ids are
    /// reassigned exactly as a parse assigns them. Cost O(blocks) for the ids
    /// plus O(pages) for the path index; nothing is parsed.
    pub(crate) fn checkpoint_install(
        &self,
        state: GraphState<PagesIn>,
        config: Config,
    ) -> Result<(Arc<ReadSnapshot>, Arc<EntryList>), crate::store::checkpoint::Fallback> {
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
        let block_index = derived.block_index.into_index(&pages)?;
        let query_index = crate::query::index::QueryIndexSlot::from_checkpoint(
            derived.query_index,
            &pages.positions,
        );
        self.install_checkpoint_rows(
            &pages,
            &state.observed_mtimes,
            state.failures,
            (state.disk_revs, state.vcs_anchored),
            (state.cache_generation, list.materialize()),
        )?;
        // The alias list names its owners by relative path; the owner map is
        // the same rows keyed the other way and is not read back. A row naming
        // a page the cache lacks leaves the relation unbuilt: the first alias
        // read walks the pages, as at a cold Ready.
        let alias_relation = derived
            .aliases
            .and_then(|state| match state {
                AliasListState::In(list) => crate::query::AliasRelation::from_list(&list, |rel| {
                    let slot = *pages.positions.get(rel)?;
                    pages.get(slot).map(|(entry, _)| entry.path.clone())
                }),
                AliasListState::Out(_) => None,
            })
            .map(Arc::new);
        let mut read = ReadSnapshot {
            pages,
            config,
            list: Arc::clone(&list),
            observed_mtimes: state.observed_mtimes,
            explicit_index: state.explicit_index,
            reference_candidate_index: RwLock::new(reference),
            cache_generation: state.cache_generation,
            block_index: cell(Some(block_index)),
            alias_index: cell(state.alias_index),
            referenced_name_index: cell(derived.referenced_name_index),
            real_page_names: state.real_page_names,
            alias_relation: cell(alias_relation),
            link_graph: std::sync::OnceLock::new(),
            referenced_names: cell(derived.referenced_names),
            block_ref_counts: cell(state.block_ref_counts),
            public_block_ref_counts: cell(derived.public_block_ref_counts.map(|counts| counts.0)),
            icon_index: cell(state.icon_index),
            memos: SnapshotMemos {
                derived_cache: RwLock::new(derived.derived_cache.map(|cache| cache.0)),
                query: crate::query::memo::QueryMemo::from_checkpoint(derived.query_memo),
            },
            query_index,
            transliteration_names: Arc::new(NameKeyMap::new()),
            transliteration_owners: SharedMap::new(),
            completion_name_keys: Arc::clone(&self.completion_name_keys),
            #[cfg(test)]
            name_cache_work: 0,
            #[cfg(test)]
            block_full_builds: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            referenced_name_full_builds: std::sync::atomic::AtomicUsize::new(0),
        };
        read.transliteration_names = read.build_name_keys(None, &[]);
        Ok((Arc::new(read), list))
    }
}

/// Exact block owners and the lazily built half of a published generation,
/// as it stands at capture (`None`: a lazy slot was not built). Capture shares
/// the pages and clones `Arc`s and the small alias tables under the store
/// writer; encoding work (load-time block ids, key ordering, JSON for result
/// DTOs) happens when it is serialized, off the writer.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct DerivedState {
    block_index: BlockIndexState,
    referenced_name_index: Option<SnapshotPageDerivedIndex>,
    aliases: Option<AliasListState>,
    alias_owner_paths_by_key: Option<AliasOwnersState>,
    referenced_names: Option<Vec<String>>,
    public_block_ref_counts: Option<Sorted<usize>>,
    derived_cache: Option<DerivedCacheState>,
    query_memo: Option<crate::query::memo::checkpoint::MemoState>,
    query_index: crate::query::index::checkpoint::SlotState,
}

impl DerivedState {
    fn of(read: &ReadSnapshot) -> DerivedState {
        DerivedState {
            block_index: BlockIndexState::Out(Arc::clone(&read.pages)),
            referenced_name_index: read.referenced_name_index.get().cloned(),
            aliases: read
                .alias_relation
                .get()
                .map(|relation| AliasListState::Out(Arc::clone(relation))),
            alias_owner_paths_by_key: read
                .alias_relation
                .get()
                .map(|relation| AliasOwnersState::Out(Arc::clone(relation))),
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

/// The alias list as the checkpoint stores it: `(alias key, owner name, owner
/// rel path)` rows in path order. Written from the generation's alias
/// relation when the file is encoded (off the writer), read back as plain
/// rows; the wire form is the plain row list it has always been.
pub(crate) enum AliasListState {
    Out(Arc<crate::query::AliasRelation>),
    In(Vec<(String, String, String)>),
}

impl serde::Serialize for AliasListState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AliasListState::Out(relation) => relation.list().serialize(s),
            AliasListState::In(rows) => rows.serialize(s),
        }
    }
}

impl<'de> serde::Deserialize<'de> for AliasListState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::deserialize(d).map(AliasListState::In)
    }
}

/// The alias owner map as the checkpoint stores it: each alias key with the
/// rel paths of its owners, in key order (the shape `Sorted` wrote).
pub(crate) enum AliasOwnersState {
    Out(Arc<crate::query::AliasRelation>),
    In(Vec<(String, Vec<String>)>),
}

impl serde::Serialize for AliasOwnersState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AliasOwnersState::Out(relation) => relation.owner_rows().serialize(s),
            AliasOwnersState::In(rows) => rows.serialize(s),
        }
    }
}

impl<'de> serde::Deserialize<'de> for AliasOwnersState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Vec::deserialize(d).map(AliasOwnersState::In)
    }
}

/// A string-keyed map written in key order.
pub(crate) struct Sorted<V>(Arc<HashMap<String, V>>);

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

#[cfg(test)]
#[test]
fn block_index_wire_keeps_runtime_ids_compact() {
    let mut doc = Document {
        pre_block: None,
        roots: (0..2048).map(|_| DocBlock::new("ordinary block")).collect(),
    };
    assign_doc_runtime_ids(&mut doc.roots, "pages/A.md");
    let entry = PageEntry {
        name: "A".into(),
        path: PathBuf::from("pages/A.md"),
        rel_path: Some("pages/A.md".into()),
        kind: PageKind::Page,
        date_key: None,
    };
    let pages = Arc::new(Pages::from(vec![(entry, Arc::new(doc))]));
    let bytes = postcard::to_stdvec(&BlockIndexState::Out(Arc::clone(&pages))).unwrap();
    assert!(
        bytes.len() <= 20 * 2048,
        "{} bytes for 2048 runtime ids",
        bytes.len()
    );
    let decoded: BlockIndexState = postcard::from_bytes(&bytes).unwrap();
    let index = decoded.into_index(&pages).unwrap();
    for block in &pages[0].1.roots {
        assert_eq!(index.first_owner(&block.uuid), Some(0));
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
    /// Which block and derived parts this generation holds: (block index,
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
            read.alias_index.get().is_some(),
            read.referenced_name_index.get().is_some(),
            read.alias_relation.get().is_some(),
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
