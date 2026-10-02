//! The forced full rebuild behind the Settings "Rescan graph" button
//! (`Store::rebuild_graph`). The focus-return rescan stays the cheap stat
//! diff in `scan_refresh`; only this path ignores every stamp.
//!
//! Unit cost: one stat plus one full read per graph file for the hash pass,
//! then the cold-launch parse (one read plus one parse per file, parallel);
//! no bytes are written and nothing is persisted. It holds the writer lock
//! only for the swap and the publication, or for the whole build once
//! concurrent saves have made two optimistic builds decline.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use super::WatchHandle;
use crate::launch_diag::DiffTrigger;
use crate::store::{LoadError, LoadStatus};

/// Optimistic rebuild attempts that build beside concurrent saves before the
/// build runs under the writer lock; the total bounds a graph that never settles.
const REBUILD_ATTEMPTS: u32 = 8;

impl WatchHandle {
    /// The forced rebuild behind `Store::rebuild_graph` (Settings "Rescan
    /// graph"). Step 1 hashes every file ignoring every stamp and publishes
    /// the ones whose bytes moved, with their page events, through the
    /// ordinary reconcile. Step 2 re-reads and re-parses every file through
    /// the launch build and swaps the result in; saves racing the parse make
    /// the swap decline, so after two declined attempts the build runs under
    /// the writer lock (saves wait, nothing is lost). Step 3 publishes a
    /// snapshot whose derived state is recomputed from the new cache.
    pub(crate) fn rebuild_all(&self) -> Result<(), LoadError> {
        let mut status = self.core.load.status.lock().unwrap();
        while matches!(*status, LoadStatus::Loading) {
            status = self.core.load.ready.wait(status).unwrap();
        }
        match &*status {
            LoadStatus::Closed => return Err(LoadError::Closed),
            LoadStatus::Failed(_) => {
                drop(status);
                return self.scan_refresh();
            }
            LoadStatus::Ready => drop(status),
            LoadStatus::Loading => unreachable!(),
        }
        self.core
            .reconcile(None, true, true, DiffTrigger::Rebuild)?;
        let cancelled = || self.core.closed.load(Ordering::Acquire);
        let mut attempt = 0u32;
        let writer = loop {
            if cancelled() {
                return Err(LoadError::Closed);
            }
            if attempt >= REBUILD_ATTEMPTS {
                return Err(LoadError::Failed {
                    reason: "graph kept changing during the rescan".into(),
                });
            }
            let locked = (attempt >= 2).then(|| self.core.writer.lock().unwrap());
            if self.core.graph.rebuild_cache_cancellable(cancelled) {
                break locked.unwrap_or_else(|| self.core.writer.lock().unwrap());
            }
            attempt += 1;
        };
        if cancelled() {
            return Err(LoadError::Closed);
        }
        // The install replaced the unreadable rows with the build's own;
        // the watcher's walk errors belong beside them.
        let walk_errors = self.core.unreadable_dirs.lock().unwrap().clone();
        self.core
            .graph
            .replace_unreadable_walk_errors(&HashMap::new(), &walk_errors);
        self.core.changes.publish_rebuilt(Vec::new());
        drop(writer);
        self.core.observe_assets(&HashSet::new(), true);
        let _ = self.wake.send(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime};

    use crate::launch_diag::DiffTrigger;
    use crate::store::{OpenOptions, Store, WatchMode};
    use crate::{ChangeKind, Origin, PageId, SaveBase, TxOutcome, Why};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tine-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        root
    }

    /// Rewrite `path` in place with same-length `text` and put its mtime back:
    /// what a sync client or restore-from-backup tool does, and the one shape a
    /// stat diff cannot see (length and modification time unchanged).
    fn rewrite_keeping_stamp(path: &Path, text: &str) {
        let before = fs::metadata(path).unwrap();
        assert_eq!(
            before.len(),
            text.len() as u64,
            "the rewrite must keep the length"
        );
        fs::write(path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(before.modified().unwrap())
            .unwrap();
        let after = fs::metadata(path).unwrap();
        assert_eq!(
            (after.len(), after.modified().unwrap()),
            (before.len(), before.modified().unwrap())
        );
    }

    fn open(root: &Path) -> Store {
        let store = Store::open(
            root,
            OpenOptions {
                approved_external_assets: None,
                watch: WatchMode::Poll,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        store
    }

    fn cached_first_block(store: &Store, page: &str) -> String {
        let graph = store.whole_graph().unwrap();
        let corpus = graph.corpus();
        let found = corpus.pages.iter().find(|p| p.name == page).unwrap();
        found.document.roots[0].raw().to_string()
    }

    /// The Settings "Rescan graph" button must catch what the focus-return
    /// stat diff cannot: a same-length rewrite with the mtime restored. The
    /// stat diff (`scan_refresh`'s own reconcile) leaves the stale document; the
    /// forced rebuild replaces it and publishes the file as an external change.
    #[test]
    fn rebuild_catches_a_same_size_rewrite_that_a_stat_diff_misses() {
        let root = temp_root("rebuild-same-size");
        let path = root.join("pages/A.md");
        fs::write(&path, "- old one\n").unwrap();
        let store = open(&root);
        let subscription = store.subscribe();
        assert_eq!(cached_first_block(&store, "A"), "old one");

        {
            // Holding the writer pauses the poller, so the only observer is the
            // explicit stat-diff reconcile below.
            let writer = store.writer.lock().unwrap();
            rewrite_keeping_stamp(&path, "- new one\n");
            let core = store.watch.core_for_load();
            core.reconcile_locked(None, true, true, DiffTrigger::Rescan)
                .unwrap();
            assert_eq!(
                cached_first_block(&store, "A"),
                "old one",
                "the stat diff was expected to miss this rewrite (test premise)"
            );
            drop(writer);
        }

        store.rebuild_graph().unwrap();
        assert_eq!(cached_first_block(&store, "A"), "new one");
        let mut seen = false;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !seen && Instant::now() < deadline {
            match subscription.try_recv().unwrap() {
                Some(change) => {
                    seen = change.origin == Origin::External
                        && change.files.iter().any(|(id, kind, _)| {
                            id.as_str() == "pages/A.md" && *kind == ChangeKind::Modified
                        });
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(
            seen,
            "the rebuilt file was never published as an external change"
        );
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    /// An open editor keeps its base revision across the rebuild: a save made
    /// over a file the rebuild found changed is refused as a stale save.
    #[test]
    fn rebuild_leaves_an_open_editors_base_revision_stale_not_overwritten() {
        let root = temp_root("rebuild-stale-base");
        let path = root.join("pages/A.md");
        fs::write(&path, "- old one\n").unwrap();
        let store = open(&root);
        let id = PageId::from("pages/A.md");
        let open_editor = store.page(&id).unwrap();

        {
            let writer = store.writer.lock().unwrap();
            rewrite_keeping_stamp(&path, "- new one\n");
            drop(writer);
        }
        store.rebuild_graph().unwrap();

        let mut doc = open_editor.doc.clone();
        doc.blocks[0].raw = "editor text".into();
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.save_page(
            &[crate::EditKind::ReplacePage],
            &id,
            SaveBase::Existing(open_editor.rev),
            &doc,
        );
        match tx.commit() {
            TxOutcome::NotCommitted {
                why: Why::Conflict { .. },
                ..
            } => {}
            other => panic!("a stale save over the rebuilt file was not refused: {other:?}"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "- new one\n");
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    /// Files created and removed behind the stat diff's back (the rebuild must
    /// not leave a ghost page nor miss a new one).
    #[test]
    fn rebuild_drops_a_removed_page_and_adds_a_created_one() {
        let root = temp_root("rebuild-add-remove");
        fs::write(root.join("pages/Gone.md"), "- gone\n").unwrap();
        let store = open(&root);
        {
            let writer = store.writer.lock().unwrap();
            fs::remove_file(root.join("pages/Gone.md")).unwrap();
            fs::write(root.join("pages/Fresh.md"), "- fresh\n").unwrap();
            drop(writer);
        }
        store.rebuild_graph().unwrap();
        let graph = store.whole_graph().unwrap();
        let corpus = graph.corpus();
        let names: Vec<&str> = corpus.pages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Fresh"), "created page missing: {names:?}");
        assert!(!names.contains(&"Gone"), "removed page lingers: {names:?}");
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
