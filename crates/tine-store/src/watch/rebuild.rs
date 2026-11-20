//! The forced full rebuild behind the Settings "Rescan graph" button
//! (`Store::refresh(Depth::Rebuild)`). The focus-return rescan stays the cheap stat
//! diff in `refresh(Depth::Stamps)`; only this path ignores every stamp.
//!
//! Unit cost: one stat plus one full read per graph file for the hash pass,
//! then the cold-launch parse (one read plus one parse per file, parallel);
//! no bytes are written and nothing is persisted. It holds the writer lock
//! for the hash diff and final publication. Build and name certification run
//! beside the writer.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use super::WatchHandle;
use crate::launch_diag::DiffTrigger;
use crate::store::{LoadError, LoadStatus, Origin};

/// Bound retries when files or ownership keep changing during Rescan.
const REBUILD_ATTEMPTS: u32 = 8;

#[cfg(test)]
thread_local! {
    pub(crate) static REBUILD_WRITER_TIME: std::cell::Cell<std::time::Duration> = const { std::cell::Cell::new(std::time::Duration::ZERO) };
}

impl WatchHandle {
    /// The forced rebuild behind `Store::refresh(Depth::Rebuild)` (Settings "Rescan
    /// graph"). Step 1 hashes every file ignoring every stamp and publishes
    /// the ones whose bytes moved, with their page events, through the
    /// ordinary reconcile. Step 2 re-reads and re-parses every file through
    /// the launch build and swaps the result in, retrying if the observed
    /// state moved. Step 3 publishes a snapshot recomputed from that cache.
    pub(crate) fn rebuild_all(&self) -> Result<(), LoadError> {
        let _rebuild = self.rebuild.lock().unwrap();
        #[cfg(test)]
        REBUILD_WRITER_TIME.with(|time| time.set(std::time::Duration::ZERO));
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
        self.reconcile_refresh(DiffTrigger::Rebuild)?;
        #[cfg(test)]
        crate::store::pause_at_hook(&self.core.graph.rebuild_before_build_pause);
        let cancelled = || self.core.closed.load(Ordering::Acquire);
        let mut writer = None;
        for _ in 0..REBUILD_ATTEMPTS {
            if cancelled() {
                return Err(LoadError::Closed);
            }
            if self.core.graph.rebuild_cache_cancellable(cancelled) {
                let mut guard = self.core.writer.lock().unwrap();
                if !self.core.graph.rebuild_names_are_current() {
                    drop(guard);
                    self.core.graph.prepare_rebuilt_name_listing();
                    guard = self.core.writer.lock().unwrap();
                }
                if self.core.graph.rebuild_names_are_current() {
                    writer = Some(guard);
                    break;
                }
            }
        }
        let writer = writer.ok_or_else(|| LoadError::Failed {
            reason: "graph kept changing during the rescan".into(),
        })?;
        #[cfg(test)]
        let writer_at = std::time::Instant::now();
        if cancelled() {
            return Err(LoadError::Closed);
        }
        // Rebuilt indexes from the whole cache: contained as the first
        // publication is (an index-stage panic costs its rows; one no row
        // explains fails the graph, and never poisons the writer).
        let published =
            crate::store::contain_index_publication(&self.core.graph, &self.core.load, || {
                self.core
                    .changes
                    .publish_rebuilt(Origin::External, Vec::new(), false);
            });
        #[cfg(test)]
        crate::store::pause_at_hook(&self.core.graph.rebuild_after_publish_pause);
        #[cfg(test)]
        REBUILD_WRITER_TIME.with(|time| time.set(time.get() + writer_at.elapsed()));
        drop(writer);
        if !published {
            return Err(LoadError::Failed {
                reason: "graph publication stopped: an index build panicked".into(),
            });
        }
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
    use crate::{ChangeKind, Origin, PageId, SaveBase, Why};

    fn temp_root(tag: &str) -> PathBuf {
        let root = crate::test_support::fresh_dir(&format!("tine-{tag}-"));
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
                launch_checkpoint: None,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        store
    }

    fn cached_first_block(store: &Store, page: &str) -> String {
        let graph = store.whole_graph().unwrap();
        let corpus = graph.corpus(None, false);
        let found = corpus.pages.iter().find(|p| p.name == page).unwrap();
        found.document.roots[0].raw().to_string()
    }

    #[test]
    fn an_edit_after_the_rescan_hash_is_named_on_watch_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pages/Old.md");
        fs::create_dir(path.parent().unwrap()).unwrap();
        let write_title = |title: &str, seconds: u64| {
            fs::write(&path, format!("title:: {title}\n- text\n")).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        };
        write_title("Recovered", 1_000_000_000);
        let store = open(dir.path());
        store.watch.stop_background_for_test();
        let page = PageId::from("pages/Old.md");
        store.hold_page(&page).unwrap();
        let resolves = |title| {
            matches!(store.whole_graph().unwrap().resolve(title, false),
                crate::Resolved::Existing { id, .. } if id == page)
        };

        // Drive the production Rescan stages so the file changes between the
        // hash pass and the build, without the watcher delivering it yet.
        store.watch.reconcile_refresh(DiffTrigger::Rebuild).unwrap();
        write_title("Disk name", 1_000_000_001);
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        {
            let _writer = store.writer.lock().unwrap();
            let generation = store.graph.cache_generation();
            let held = store.graph.held.epoch();
            store.watch.release_hold(page.as_str());
            assert_ne!(store.graph.held.epoch(), held);
            assert_eq!(store.graph.cache_generation(), generation);
            store
                .changes
                .publish_rebuilt(Origin::External, Vec::new(), false);
        }
        assert!(resolves("Recovered"));
        assert!(!resolves("Disk name"));

        let subscription = store.subscribe();
        {
            let _writer = store.writer.lock().unwrap();
            let paths = [store.graph.root.join(page.as_str())].into_iter().collect();
            store.watch.reconcile_raced(&paths);
        }
        let event = subscription
            .try_recv()
            .unwrap()
            .expect("external title delivery");
        assert_eq!(event.origin, Origin::External);
        assert!(event.files.iter().any(|(id, kind, _)| {
            id.as_str() == page.as_str() && *kind == ChangeKind::Modified
        }));
        assert!(resolves("Disk name"));
        assert!(!resolves("Recovered"));
        store.close();
    }

    #[test]
    fn releasing_an_unreadable_page_restores_its_listed_name() {
        let dir = tempfile::tempdir().unwrap();
        let page = PageId::from("pages/Unreadable.md");
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join(page.as_str()), b"- \xff\n").unwrap();
        let store = open(dir.path());
        store.watch.stop_background_for_test();
        {
            let _writer = store.writer.lock().unwrap();
            store.graph.hold_unhosted(page.as_str());
        }
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        {
            let _writer = store.writer.lock().unwrap();
            let generation = store.graph.cache_generation();
            store.watch.release_hold(page.as_str());
            assert_eq!(store.graph.cache_generation(), generation);
            store
                .changes
                .publish_rebuilt(Origin::External, Vec::new(), false);
        }
        assert!(
            matches!(store.whole_graph().unwrap().resolve("Unreadable", false),
            crate::Resolved::Existing { id, .. } if id == page)
        );
        store.close();
    }

    #[test]
    fn releasing_an_owner_only_claim_removes_it_from_the_rebuilt_listing() {
        owner_only_release(false);
    }

    #[test]
    fn shutting_down_removes_an_owner_only_claim_from_the_rebuilt_listing() {
        owner_only_release(true);
    }

    fn owner_only_release(shutdown: bool) {
        let dir = tempfile::tempdir().unwrap();
        let page = PageId::from("pages/Old.md");
        let path = dir.path().join(page.as_str());
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, "title:: Recovered\n- text\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
            .unwrap();
        let store = open(dir.path());
        store.watch.stop_background_for_test();
        store.hold_page(&page).unwrap();
        store.watch.reconcile_refresh(DiffTrigger::Rebuild).unwrap();
        fs::remove_file(path).unwrap();
        assert!(store.graph.rebuild_cache_cancellable(|| false));
        let generation = store.graph.cache_generation();
        {
            let _writer = store.writer.lock().unwrap();
            if shutdown {
                store.watch.release_holds();
            } else {
                store.watch.release_hold(page.as_str());
            }
            store
                .changes
                .publish_rebuilt(Origin::External, Vec::new(), false);
        }
        let view = store.whole_graph().unwrap();
        assert!(matches!(
            view.resolve("Recovered", false),
            crate::Resolved::Absent { .. }
        ));
        assert!(
            view.corpus(None, false)
                .pages
                .iter()
                .all(|page| page.name != "Recovered"),
            "the removed page is still in the corpus"
        );
        assert!(
            store.graph.cache_generation() > generation,
            "a retired row moves the generation"
        );
        store.close();
    }

    struct Pause(crate::store::TestPause);

    impl Pause {
        fn at(hook: &std::sync::Mutex<Option<crate::store::TestPause>>) -> Self {
            let pause = Self(Default::default());
            *hook.lock().unwrap() = Some(pause.0.clone());
            pause
        }

        fn reached(&self) -> bool {
            let (state, ready) = &*self.0;
            ready
                .wait_timeout_while(state.lock().unwrap(), Duration::from_secs(2), |s| !s.0)
                .unwrap()
                .0
                 .0
        }

        fn resume(&self) {
            let (state, ready) = &*self.0;
            state.lock().unwrap().1 = true;
            ready.notify_all();
        }
    }

    impl Drop for Pause {
        fn drop(&mut self) {
            self.resume();
        }
    }

    #[test]
    fn a_rejected_name_walk_cannot_restore_a_repaired_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pages/Old.md");
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, b"title:: \xff\n- unreadable\n").unwrap();
        let store = std::sync::Arc::new(open(dir.path()));
        store.watch.stop_background_for_test();
        assert!(!store.graph.reported_unreadable_pages().is_empty());
        let pause = Pause::at(&store.graph.discovery_walk_pause);
        let walking = store.clone();
        let walk = std::thread::spawn(move || walking.graph.prepare_rebuilt_name_listing());
        assert!(pause.reached());
        *store.graph.discovery_walk_pause.lock().unwrap() = None;
        let generation = store.graph.cache_generation();
        fs::write(&path, "title:: Repaired\n- readable again\n").unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(store.graph.cache_generation() > generation);
        assert!(store.graph.reported_unreadable_pages().is_empty());
        pause.resume();
        walk.join().unwrap();
        assert!(
            store.graph.reported_unreadable_pages().is_empty(),
            "the rejected walk restored the repaired error"
        );
        store.close();
    }

    #[test]
    fn overlapping_rescans_publish_consistent_views() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pages/Named.md");
        fs::create_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, "title:: First\n- text\n").unwrap();
        let store = std::sync::Arc::new(open(dir.path()));
        store.watch.stop_background_for_test();

        let first_install = Pause::at(&store.graph.warm_after_install_pause);
        let first_capture = Pause::at(&store.graph.rebuild_names_captured_pause);
        let first_publication = Pause::at(&store.graph.rebuild_after_publish_pause);
        let first = {
            let store = store.clone();
            std::thread::spawn(move || store.refresh(crate::Depth::Rebuild))
        };
        assert!(first_install.reached());
        let second_install = Pause::at(&store.graph.warm_after_install_pause);
        let second_start = Pause::at(&store.graph.rebuild_before_build_pause);
        let second = {
            let store = store.clone();
            std::thread::spawn(move || store.refresh(crate::Depth::Rebuild))
        };
        let overlapping_build = second_start.reached();
        first_install.resume();
        assert!(first_capture.reached());
        fs::write(&path, "title:: Second\n- text\n").unwrap();
        if overlapping_build {
            second_start.resume();
            assert!(second_install.reached());
        }
        first_capture.resume();
        assert!(first_publication.reached());
        let before = store.whole_graph().unwrap();
        first_publication.resume();
        first.join().unwrap().unwrap();
        second_start.resume();
        second_install.resume();
        second.join().unwrap().unwrap();
        let after = store.whole_graph().unwrap();
        for (view, title) in [(before, "First"), (after, "Second")] {
            assert!(matches!(
                view.resolve(title, false),
                crate::Resolved::Existing { .. }
            ));
            assert_eq!(
                view.corpus(None, false).pages[0].name,
                title,
                "lookup and corpus are from different rebuilds"
            );
        }
        store.close();
    }

    /// The Settings "Rescan graph" button must catch what the focus-return
    /// stat diff cannot: a same-length rewrite with the mtime restored. The
    /// stat diff (`refresh(Depth::Stamps)`'s own reconcile) leaves the stale document; the
    /// forced rebuild replaces it and publishes the file as an external change.
    #[test]
    fn rebuild_catches_a_same_size_rewrite_that_a_stat_diff_misses() {
        let root = temp_root("rebuild-same-size");
        let path = root.join("pages/A.md");
        fs::write(&path, "- old one\n").unwrap();
        // Outside the racy window (storage spec §5.4): a racy stamp is reread
        // by every full diff, so the stat diff would catch a fresh file's
        // rewrite. The accepted gap (R5) is the rewrite of a settled file.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
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

        store.refresh(crate::Depth::Rebuild).unwrap();
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

    /// The `Depth::Bytes` diff (an export's freshness check) catches the
    /// same-length, stamp-restored rewrite the `Depth::Stamps` diff misses,
    /// and publishes it as an external change, without the rebuild's re-parse.
    /// Both diffs run under the writer, as `refresh` runs them, so the poller
    /// (which can notice the rewrite through other metadata) never observes
    /// it first; graph-features' `publish_sees_a_same_size_rewrite_with_its_
    /// stamp_restored` covers the public `refresh(Depth::Bytes)` door.
    #[test]
    fn a_bytes_refresh_catches_a_same_size_rewrite_that_stamps_miss() {
        let root = temp_root("bytes-same-size");
        let path = root.join("pages/A.md");
        fs::write(&path, "- old one\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        let store = open(&root);
        let subscription = store.subscribe();
        {
            // Holding the writer pauses the poller, so the two diffs below are
            // the only observers of the rewrite.
            let writer = store.writer.lock().unwrap();
            rewrite_keeping_stamp(&path, "- new one\n");
            let core = store.watch.core_for_load();
            core.reconcile_locked(None, true, true, DiffTrigger::Rescan)
                .unwrap();
            assert_eq!(
                cached_first_block(&store, "A"),
                "old one",
                "a stamps diff was expected to miss this rewrite (test premise)"
            );
            core.reconcile_locked(None, true, true, DiffTrigger::Bytes)
                .unwrap();
            assert_eq!(cached_first_block(&store, "A"), "new one");
            drop(writer);
        }
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
            "the refreshed file was never published as an external change"
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
        store.refresh(crate::Depth::Rebuild).unwrap();

        let mut doc = open_editor.doc.clone();
        doc.blocks[0].raw = "editor text".into();
        let store = std::sync::Arc::new(store);
        let host_writer = crate::test_host_writer::HostWriter::start(&store);
        match host_writer.save(
            crate::EditKind::ReplacePage,
            &id,
            SaveBase::Existing(open_editor.rev),
            &doc,
        ) {
            Err(crate::test_host_writer::SaveRefused::Planned(Why::Conflict { .. })) => {}
            other => panic!("a stale save over the rebuilt file was not refused: {other:?}"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "- new one\n");
        drop(host_writer);
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
        store.refresh(crate::Depth::Rebuild).unwrap();
        let graph = store.whole_graph().unwrap();
        let corpus = graph.corpus(None, false);
        let names: Vec<&str> = corpus.pages.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Fresh"), "created page missing: {names:?}");
        assert!(!names.contains(&"Gone"), "removed page lingers: {names:?}");
        store.close();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
