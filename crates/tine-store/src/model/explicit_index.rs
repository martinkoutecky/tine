//! Explicit-reference postings sharing paths within each page's insertion batch.
use super::*;

/// Page name → the paths of the pages explicitly referencing it. A page's
/// newly inserted postings share a path allocation; earlier batches can differ.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(from = "ExplicitIndexWire")]
pub(super) struct SnapshotExplicitIndex {
    pub(super) shards: Vec<Arc<ExplicitShard>>,
}

pub(super) type ExplicitShard = SharedMap<String, Arc<SharedMap<Arc<Path>, ()>>>;

/// The decoded form, whose paths are separate allocations until they are shared.
#[derive(serde::Deserialize)]
struct ExplicitIndexWire {
    shards: Vec<ExplicitShard>,
}

impl From<ExplicitIndexWire> for SnapshotExplicitIndex {
    fn from(wire: ExplicitIndexWire) -> Self {
        let mut paths: HashSet<Arc<Path>> = HashSet::new();
        let mut intern = |path: &Arc<Path>| match paths.get(path) {
            Some(shared) => Arc::clone(shared),
            None => {
                paths.insert(Arc::clone(path));
                Arc::clone(path)
            }
        };
        let shards = wire
            .shards
            .into_iter()
            .map(|shard| {
                Arc::new(SharedMap::from_sorted_unique(
                    shard
                        .iter()
                        .map(|(name, postings)| {
                            let postings = SharedMap::from_sorted_unique(
                                postings
                                    .iter()
                                    .map(|(path, ())| (intern(path), ()))
                                    .collect(),
                            );
                            (name.clone(), Arc::new(postings))
                        })
                        .collect(),
                ))
            })
            .collect();
        Self { shards }
    }
}

pub(super) fn names_absent_from<'a, T: Eq + std::hash::Hash>(
    names: &'a [T],
    other: &'a [T],
) -> impl Iterator<Item = &'a T> {
    // Borrow keys once instead of rescanning the opposite vector for each name.
    // Filter the original input so duplicate absent names keep their semantics.
    let other: HashSet<_> = other.iter().collect();
    names.iter().filter(move |name| !other.contains(name))
}

impl SnapshotExplicitIndex {
    pub(super) const SHARDS: usize = 64;

    pub(super) fn shard(name: &str) -> usize {
        snapshot_index_shard(name.as_bytes())
    }

    pub(super) fn update(&mut self, path: &Path, old: &[String], new: &[String]) {
        for name in names_absent_from(old, new) {
            let shard = Arc::make_mut(&mut self.shards[Self::shard(name)]);
            if let Some(paths) = shard.get_mut(name) {
                Arc::make_mut(paths).remove(path);
                if paths.is_empty() {
                    shard.remove(name);
                }
            }
        }
        let mut shared: Option<Arc<Path>> = None;
        for name in names_absent_from(new, old) {
            let shard = Arc::make_mut(&mut self.shards[Self::shard(name)]);
            if !shard.contains_key(name) {
                shard.insert(name.clone(), Arc::new(SharedMap::new()));
            }
            let path = Arc::clone(shared.get_or_insert_with(|| Arc::from(path)));
            Arc::make_mut(shard.get_mut(name).unwrap()).insert(path, ());
        }
    }

    pub(super) fn candidates(&self, names: &[String]) -> std::collections::BTreeSet<PathBuf> {
        let mut paths = std::collections::BTreeSet::new();
        for name in names {
            if let Some(postings) = self.shards[Self::shard(name)].get(name) {
                paths.extend(postings.iter().map(|(path, _)| path.to_path_buf()));
            }
        }
        paths
    }

    pub(super) fn capture(
        old: Option<(&Self, &Arc<Pages>)>,
        pages: &Arc<Pages>,
        changed_paths: &[String],
        positions: Option<(&SharedMap<String, usize>, &SharedMap<String, usize>)>,
        config: &Config,
    ) -> Self {
        if let Some((previous, previous_pages)) = old.filter(|(_, previous_pages)| {
            !changed_paths.is_empty() || Arc::ptr_eq(previous_pages, pages)
        }) {
            let mut index = previous.clone();
            for rel_path in changed_paths {
                let before = snapshot_page_by_rel(
                    previous_pages,
                    positions.map(|(before, _)| before),
                    rel_path,
                );
                let after =
                    snapshot_page_by_rel(pages, positions.map(|(_, after)| after), rel_path);
                let old_names = before
                    .map(|(entry, doc)| {
                        crate::query::document_explicit_reference_names(entry, doc, config)
                    })
                    .unwrap_or_default();
                let new_names = after
                    .map(|(entry, doc)| {
                        crate::query::document_explicit_reference_names(entry, doc, config)
                    })
                    .unwrap_or_default();
                if let Some(path) = after.or(before).map(|(entry, _)| entry.path.as_path()) {
                    index.update(path, &old_names, &new_names);
                }
            }
            index
        } else {
            Self::build(pages, config)
        }
    }

    /// The whole index from `pages`: each page's names on up to
    /// `page_cache_worker_count()` workers, inserted under the name's shard
    /// lock (64 shards, so the workers rarely meet). The same index as
    /// `update` of each page in turn, with no transient copy: per-worker
    /// posting lists measured +14 MB peak RSS on a 1M-block graph. Computing
    /// the names builds every block projection, so this is also the initial
    /// capture's parallel projection pass (GJ5 S1: the capture's index
    /// threads used to wait on the same lazily built projections in page
    /// order, so one core did them all; S4: this index was serial).
    pub(super) fn build(pages: &Pages, config: &Config) -> Self {
        let shards: Vec<std::sync::Mutex<ExplicitShard>> = (0..Self::SHARDS)
            .map(|_| std::sync::Mutex::new(SharedMap::new()))
            .collect();
        let insert = |chunk: &[&(PageEntry, Arc<Document>)]| {
            for (entry, doc) in chunk {
                let names = crate::query::document_explicit_reference_names(entry, doc, config);
                let path: Arc<Path> = Arc::from(entry.path.as_path());
                for name in names {
                    let mut shard = shards[Self::shard(&name)].lock().unwrap();
                    if let Some(paths) = shard.get_mut(&name) {
                        Arc::make_mut(paths).insert(Arc::clone(&path), ());
                    } else {
                        let mut paths = SharedMap::new();
                        paths.insert(Arc::clone(&path), ());
                        shard.insert(name, Arc::new(paths));
                    }
                }
            }
        };
        let rows: Vec<&(PageEntry, Arc<Document>)> = pages.iter().collect();
        #[cfg(test)]
        index_probe::test_full_build_fault(rows.iter().map(|(entry, _)| entry.path.as_path()));
        let workers = page_cache_worker_count();
        if workers <= 1 || rows.len() < 64 {
            insert(&rows);
        } else {
            let insert = &insert;
            std::thread::scope(|scope| {
                for chunk in rows.chunks(rows.len().div_ceil(workers)) {
                    scope.spawn(move || insert(chunk));
                }
            });
        }
        Self {
            shards: shards
                .into_iter()
                .map(|shard| Arc::new(shard.into_inner().unwrap()))
                .collect(),
        }
    }
}
