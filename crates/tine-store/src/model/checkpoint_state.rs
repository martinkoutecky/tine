//! Launch checkpoint state (storage spec §7.6, ADR 0070): the graph's half of
//! one published generation — the parsed page cache with its content revisions
//! and observed mtimes, plus the read evaluator's eager indexes — captured for
//! the checkpoint file and installed back at launch without rebuilding any of
//! it. The file format, the writer and the launch path live in
//! `store/checkpoint.rs`. The dump is deliberately dumb: whatever the published
//! generation holds is written and loaded whole; lazily built answers (block
//! index, referenced names, query memos) are left cold, exactly as a cold build
//! leaves them at Ready.
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
        }
    }

    /// The alias index with every entry in shard 0. Its shards are chosen by a
    /// hash of the absolute page path, so a golden image over a temporary root
    /// would otherwise move between runs (the loader's header root check keeps
    /// real lookups consistent). Unix only, like the golden test.
    #[cfg(all(test, unix))]
    pub(crate) fn with_alias_shards_merged(mut self) -> Self {
        if let Some(index) = self.alias_index.as_mut() {
            let mut all = SharedMap::new();
            for shard in &index.shards {
                for (path, names) in shard.iter() {
                    all.insert(path.clone(), names.clone());
                }
            }
            let count = index.shards.len();
            index.shards = std::iter::once(Arc::new(all))
                .chain((1..count).map(|_| Arc::new(SharedMap::new())))
                .collect();
        }
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
            block_index: std::sync::OnceLock::new(),
            alias_index: cell(state.alias_index),
            referenced_name_index: std::sync::OnceLock::new(),
            real_page_names: state.real_page_names,
            aliases: std::sync::OnceLock::new(),
            alias_owner_paths_by_key: std::sync::OnceLock::new(),
            referenced_names: std::sync::OnceLock::new(),
            block_ref_counts: cell(state.block_ref_counts),
            public_block_ref_counts: std::sync::OnceLock::new(),
            icon_index: cell(state.icon_index),
            memos: SnapshotMemos::default(),
            query_index: Default::default(),
            #[cfg(test)]
            block_full_builds: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            referenced_name_full_builds: std::sync::atomic::AtomicUsize::new(0),
        };
        Some((Arc::new(read), list))
    }
}

fn cell<T>(value: Option<T>) -> std::sync::OnceLock<T> {
    let cell = std::sync::OnceLock::new();
    if let Some(value) = value {
        let _ = cell.set(value);
    }
    cell
}

/// The 4096-bit reference signature as a fixed tuple of words (serde's array
/// support stops at 32 elements).
impl serde::Serialize for ReferenceTokenSignature {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let mut out = s.serialize_tuple(REFERENCE_SIGNATURE_WORDS)?;
        for word in &self.0 {
            out.serialize_element(word)?;
        }
        out.end()
    }
}

impl<'de> serde::Deserialize<'de> for ReferenceTokenSignature {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Words;
        impl<'de> serde::de::Visitor<'de> for Words {
            type Value = ReferenceTokenSignature;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "{REFERENCE_SIGNATURE_WORDS} signature words")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut words = [0u64; REFERENCE_SIGNATURE_WORDS];
                for word in words.iter_mut() {
                    *word = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::custom("short reference signature"))?;
                }
                Ok(ReferenceTokenSignature(words))
            }
        }
        d.deserialize_tuple(REFERENCE_SIGNATURE_WORDS, Words)
    }
}
