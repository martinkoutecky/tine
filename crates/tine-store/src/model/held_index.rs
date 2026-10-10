//! Pages a page host holds (STEP3 §5, R13, amendment A-V4). While a host
//! holds a page, its publication consumer (or a reservation's transaction)
//! is the page's only index writer. A read or a whole-graph build answers a
//! held page from the bytes that writer last indexed, never from disk: a
//! newer disk read indexed there could be overwritten by an older host
//! event still on its way to the consumer (REVIEW-3a V4). With no host
//! running nothing is held, and both paths read the file as before.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// A held path's owner key and what its index writer last indexed since
/// the hold began: `None` until the first publication, `Some(None)` for no
/// file. The bytes are the consumer's own buffer, shared, not copied.
type Entry = (String, Option<Option<Arc<[u8]>>>);

#[derive(Default)]
pub(crate) struct HeldPages {
    /// Changed only under the store writer (holds, releases, and every
    /// index publication), so a reconcile holding it sees a fixed set.
    paths: RwLock<HashMap<PathBuf, Entry>>,
    /// Bumped by every new hold. A whole-graph build that read its files
    /// before a hold began declines its install, as for a cache mutation.
    epoch: AtomicU64,
}

/// What a held page reads as.
pub(crate) enum HeldBytes {
    NotHeld,
    /// Held, and its owner has not yet indexed it.
    Unindexed,
    /// Held: the bytes last indexed for it, `None` for no file.
    Indexed(Option<Arc<[u8]>>),
}

impl HeldPages {
    /// Hand `path`'s index to `key`'s owner. Holding it again for the same
    /// key keeps what that owner already indexed.
    pub(crate) fn hold(&self, path: PathBuf, key: String) {
        let mut paths = self.paths.write().unwrap();
        if paths.get(&path).is_some_and(|(held, _)| *held == key) {
            return;
        }
        paths.insert(path, (key, None));
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// Return `path` to the watcher; true when it was held.
    pub(crate) fn release(&self, path: &Path) -> bool {
        self.paths.write().unwrap().remove(path).is_some()
    }

    /// Return every held path to the watcher.
    pub(crate) fn release_all(&self) -> Vec<PathBuf> {
        let mut paths = self.paths.write().unwrap();
        paths.drain().map(|(path, _)| path).collect()
    }

    /// The owner key of a held path.
    pub(crate) fn key(&self, path: &Path) -> Option<String> {
        let paths = self.paths.read().unwrap();
        paths.get(path).map(|(key, _)| key.clone())
    }

    /// Record the bytes an index writer is about to publish for `path`, if
    /// it is held. Called before the publication moves the cache
    /// generation, so a build that read the earlier bytes declines.
    pub(crate) fn indexed(&self, path: &Path, bytes: impl FnOnce() -> Option<Arc<[u8]>>) {
        if let Some((_, indexed)) = self.paths.write().unwrap().get_mut(path) {
            *indexed = Some(bytes());
        }
    }

    pub(crate) fn bytes(&self, path: &Path) -> HeldBytes {
        match self.paths.read().unwrap().get(path) {
            None => HeldBytes::NotHeld,
            Some((_, None)) => HeldBytes::Unindexed,
            Some((_, Some(bytes))) => HeldBytes::Indexed(bytes.clone()),
        }
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

impl Graph {
    /// A whole-graph build's input for `path`, and whether a host holds
    /// it: the file, or for a held page the bytes its owner last indexed.
    /// None for a held page with no file, or one its owner has not yet
    /// indexed: that owner's pending publication indexes it.
    pub(super) fn build_input(&self, path: &Path) -> io::Result<(Option<String>, bool)> {
        match self.held.bytes(path) {
            HeldBytes::NotHeld => read_parse_input(path).map(|content| (Some(content), false)),
            HeldBytes::Unindexed | HeldBytes::Indexed(None) => Ok((None, true)),
            HeldBytes::Indexed(Some(bytes)) => {
                validate_parse_bytes_for_path(&bytes, path)?;
                let content = String::from_utf8(bytes.to_vec())
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                Ok((Some(content), true))
            }
        }
    }

    /// One page's DTO for a read (`Store::page`): the canonical claimant
    /// through the cache (which it reconciles), any other file directly.
    /// A held page answers its owner's indexed bytes and touches no index;
    /// one its owner has not indexed yet is parsed from disk, unpublished.
    pub(crate) fn read_page(
        &self,
        path: &Path,
        entry: &PageEntry,
        canonical: bool,
    ) -> io::Result<Option<PageDto>> {
        match self.held.bytes(path) {
            HeldBytes::Indexed(Some(bytes)) => self.page_dto_for_bytes(path, &bytes),
            HeldBytes::Indexed(None) => Ok(None),
            HeldBytes::Unindexed => self.load_by_validated_path(path),
            HeldBytes::NotHeld if canonical => self.load_page(entry).map(Some),
            HeldBytes::NotHeld => self.load_by_validated_path(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{Store, TestPause};
    use crate::PageId;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    /// A build that read a page before a host held it declines its
    /// install even when the owner has not published yet (A-V4): its disk
    /// bytes could be newer than the owner's pending first observation.
    #[test]
    fn a_build_that_read_a_page_before_its_hold_declines() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        let path = temp.path().join("pages/a.md");
        std::fs::write(&path, "- a\n").unwrap();
        let store = Arc::new(Store::open(temp.path(), Default::default()).unwrap().0);
        store.page(&PageId::from("pages/a.md")).unwrap();
        store.whole_graph().unwrap();
        let pause: TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
        *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let build = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.graph.rebuild_cache_cancellable(|| false))
        };
        {
            let (state, ready) = &*pause;
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut state = state.lock().unwrap();
            while !state.0 {
                assert!(Instant::now() < deadline, "the build never read its page");
                state = ready
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        }
        store.graph.held.hold(path.clone(), "pages/a.md".into());
        pause.0.lock().unwrap().1 = true;
        pause.1.notify_all();
        assert!(
            !build.join().unwrap(),
            "A-V4: a build installed a page it read before the page's hold began"
        );
        *store.graph.warm_after_first_page_pause.lock().unwrap() = None;
        // Held and indexed: the next build parses the owner's bytes.
        store
            .graph
            .held
            .indexed(&path, || Some(Arc::from(&b"- owner\n"[..])));
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        assert_eq!(
            store.graph.cached_rev(&path),
            Some(crate::model::content_rev("- owner\n"))
        );
        store.close();
    }
}
