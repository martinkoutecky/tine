//! Checkpoint admission and capture: whether a loaded checkpoint's rows may
//! install, installing them (folding the seeds a read or save left before
//! the install, S6), and the cache rows a checkpoint captures. A child of
//! `derived.rs`, so the derived state stays private.

use super::*;

/// What [`Graph::checkpoint_rows`] returns: failures, sorted revisions and
/// sorted anchored paths.
pub(in crate::model) type CheckpointRows = (Vec<String>, Vec<(PathBuf, String)>, Vec<PathBuf>);

impl Graph {
    /// Why a checkpoint's rows would not install now (the checks
    /// [`Self::install_checkpoint_rows`] makes under the cache lock), or
    /// None. Asked early, without the writer (`ChangeFeed::launch_race`).
    pub(crate) fn checkpoint_race(&self) -> Option<crate::store::checkpoint::Fallback> {
        use crate::store::checkpoint::Fallback;
        if self.cache_built() {
            Some(Fallback::CacheBuilt)
        } else if !self.held.is_empty() {
            Some(Fallback::PageHeld)
        } else {
            None
        }
    }

    /// Install a loaded checkpoint's page cache: refused when a cache exists
    /// or any page is held. A checkpoint's rows, indexes and claimants carry
    /// no owner provenance, so a held page makes the launch a full build
    /// (R2): the whole checkpoint is declined. A seed (a page read or saved
    /// before the install, S6) folds into its row when the checkpoint holds
    /// its bytes' revision and keyed outline, so the snapshot and block index
    /// serve the keys already served. Any other seed declines the checkpoint
    /// (`Seeded`): the build adopts it. The caller holds the store writer,
    /// under which holds change.
    pub(in crate::model) fn install_checkpoint_rows(
        &self,
        pages: &Arc<Pages>,
        observed_mtimes: &Arc<SharedMap<String, std::time::SystemTime>>,
        failures: Vec<String>,
        (disk_revs, vcs_anchored): (Vec<(PathBuf, String)>, Vec<PathBuf>),
        (cache_generation, list): (u64, Arc<EntryList>),
    ) -> Result<(), crate::store::checkpoint::Fallback> {
        use crate::store::checkpoint::Fallback;
        let index = build_page_cache_index(pages);
        let mut guard = self.derived.cache.write().unwrap();
        if guard.pages.is_some() {
            return Err(Fallback::CacheBuilt);
        }
        if !self.held.is_empty() {
            return Err(Fallback::PageHeld);
        }
        let mut claims = self.held.claims.lock().unwrap();
        if !claims.is_empty() {
            let revs: HashMap<&Path, &str> = (disk_revs.iter())
                .map(|(path, rev)| (path.as_path(), rev.as_str()))
                .collect();
            for (path, (_, seed, disk)) in claims.iter() {
                // The row the snapshot and block index are built from holds
                // the seed's keys: nothing to fold.
                let same = (revs.get(path.as_path()) == Some(&disk.rev.as_str()))
                    .then(|| index.by_path.get(path).and_then(|&slot| pages.get(slot)))
                    .flatten()
                    .is_some_and(|(_, row)| row.same_keyed_outline(seed));
                if !same {
                    return Err(Fallback::Seeded);
                }
            }
            claims.clear();
        }
        drop(claims);
        guard.pages = Some(Arc::clone(pages));
        *self.derived.observed_mtimes.write().unwrap() = Arc::clone(observed_mtimes);
        *self.derived.page_index_failures.write().unwrap() = failures;
        self.derived.index_retired.write().unwrap().clear();
        *self.derived.unreadable_pages.write().unwrap() = Arc::new(Vec::new());
        *self.derived.cache_index.write().unwrap() = Some(index);
        *self.derived.disk_revs.write().unwrap() = disk_revs.into_iter().collect();
        *self.derived.vcs_anchored.write().unwrap() = vcs_anchored.into_iter().collect();
        // The loaded generation keeps its number, so the evaluator, the
        // reference index and the generation-keyed listing agree with it.
        self.cache_gen.store(cache_generation, Ordering::Release);
        self.forget_cold_moves();
        *self.derived.page_list_cache.write().unwrap() = Some((cache_generation, list));
        drop(guard);
        Ok(())
    }

    /// The cache rows a checkpoint captures with `pages`, the published
    /// generation's: failures, sorted revisions and anchors.
    pub(in crate::model) fn checkpoint_rows(
        &self,
        pages: &Arc<Pages>,
        cache_generation: u64,
    ) -> Result<CheckpointRows, checkpoint_state::NotCaptured> {
        use checkpoint_state::NotCaptured;
        let cache = self.derived.cache.read().unwrap();
        if !cache
            .pages
            .as_ref()
            .is_some_and(|built| Arc::ptr_eq(built, pages))
            || cache_generation != self.cache_generation()
        {
            return Err(NotCaptured::Unpublished);
        }
        if !self.derived.unreadable_pages.read().unwrap().is_empty() || !cache.discovery.is_empty()
        {
            return Err(NotCaptured::Unreadable);
        }
        let mut disk_revs: Vec<(PathBuf, String)> = self
            .derived
            .disk_revs
            .read()
            .unwrap()
            .iter()
            .map(|(path, rev)| (path.clone(), rev.clone()))
            .collect();
        disk_revs.sort();
        let mut anchored: Vec<PathBuf> = self
            .derived
            .vcs_anchored
            .read()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        anchored.sort();
        let failures = self.derived.page_index_failures.read().unwrap().clone();
        Ok((failures, disk_revs, anchored))
    }
}
