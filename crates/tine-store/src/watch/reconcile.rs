//! Full and path-hinted graph-text reconciliation, ordered with store publication.
use super::*;

/// An ephemeral directory observation, never a cache or persisted record.
struct PreparedWalk {
    began: Instant,
    observed: SystemTime,
    files: HashMap<PathBuf, Stamp>,
    errors: HashMap<PathBuf, String>,
    times: CollectTimes,
}

impl Core {
    pub(super) fn reconcile(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        trigger: DiffTrigger,
    ) -> Result<(), LoadError> {
        if paths.is_none() {
            return self.reconcile_full(include_config, scan_semantics, None, trigger);
        }
        let _writer = self.writer.lock().unwrap();
        self.reconcile_locked(paths, include_config, scan_semantics, trigger)
    }

    /// One watcher cycle; its publication carries `batch` for latency receipts.
    pub(super) fn reconcile_batch(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        batch: WatchBatch,
    ) -> Result<(), LoadError> {
        let trigger = if batch.poll {
            DiffTrigger::Poll
        } else {
            DiffTrigger::WatchEvent
        };
        if paths.is_none() {
            return self.reconcile_full(include_config, false, Some(batch), trigger);
        }
        let _writer = self.writer.lock().unwrap();
        self.reconcile_inner(paths, include_config, false, Some(batch), trigger)
    }

    /// Enumerate unchanged-config metadata outside the page/save writer. Config
    /// changes retain full enumeration and reparse under the writer; failed-load
    /// recovery uses the separate writer-ordered reconciliation entry point. A
    /// publication or cache change while collecting invalidates the observation:
    /// retry rather than replacing a newer own-write baseline with older metadata.
    fn reconcile_full(
        &self,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
        trigger: DiffTrigger,
    ) -> Result<(), LoadError> {
        if include_config {
            let _writer = self.writer.lock().unwrap();
            // An actual config change retains its existing full reparse protocol.
            // Otherwise this bounded check visits no page paths.
            self.reconcile_inner(Some(&HashSet::new()), true, scan_semantics, batch, trigger)?;
        }
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Err(LoadError::Closed);
            }
            let generation = self.graph.cache_generation();
            let revision = self.changes.rev();
            let config = self.graph.current_config();
            let began = Instant::now();
            let observed = SystemTime::now();
            let dirs = self.dirs.read().unwrap().clone();
            let (files, errors, times) = self.collect_full(&dirs, &config);
            let prepared = PreparedWalk {
                began,
                observed,
                files,
                errors,
                times,
            };
            let _writer = self.writer.lock().unwrap();
            if generation != self.graph.cache_generation() || revision != self.changes.rev() {
                continue;
            }
            return self.reconcile_prepared(
                None,
                false,
                scan_semantics,
                batch,
                trigger,
                None,
                Some(prepared),
            );
        }
    }

    // Caller holds writer through reconciliation and any recovery publication.
    pub(super) fn reconcile_locked(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        trigger: DiffTrigger,
    ) -> Result<(), LoadError> {
        self.reconcile_inner(paths, include_config, scan_semantics, None, trigger)
    }

    /// Times the cycle and records it when it was a full stat diff (a config
    /// change widens a path-scoped cycle into one, so that is known only
    /// after the body ran). Instants only: nothing is allocated per file.
    fn reconcile_inner(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
        trigger: DiffTrigger,
    ) -> Result<(), LoadError> {
        self.reconcile_timed(paths, include_config, scan_semantics, batch, trigger, None)
    }

    pub(super) fn reconcile_timed(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
        trigger: DiffTrigger,
        deferred: Option<&mut Deferred>,
    ) -> Result<(), LoadError> {
        self.reconcile_prepared(
            paths,
            include_config,
            scan_semantics,
            batch,
            trigger,
            deferred,
            None,
        )
    }

    fn reconcile_prepared(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
        trigger: DiffTrigger,
        deferred: Option<&mut Deferred>,
        prepared: Option<PreparedWalk>,
    ) -> Result<(), LoadError> {
        let began = prepared
            .as_ref()
            .map_or_else(Instant::now, |walk| walk.began);
        let mut walk = CollectTimes::default();
        // A rebuild or a bytes refresh ignores the stamp shortcut: every
        // file is hashed.
        let force = matches!(trigger, DiffTrigger::Rebuild | DiffTrigger::Bytes);
        let result = self.reconcile_walk(
            paths,
            include_config,
            scan_semantics,
            batch,
            force,
            &mut walk,
            deferred,
            prepared,
        );
        if walk.full {
            self.graph
                .diag
                .diff(DiffStats::new(trigger, began.elapsed(), &walk));
        }
        result
    }

    fn reconcile_walk(
        &self,
        paths: Option<&HashSet<PathBuf>>,
        include_config: bool,
        scan_semantics: bool,
        batch: Option<WatchBatch>,
        force: bool,
        walk: &mut CollectTimes,
        deferred: Option<&mut Deferred>,
        prepared: Option<PreparedWalk>,
    ) -> Result<(), LoadError> {
        // §5.4: observation time of this cycle's stamps, taken before any is.
        let observed = prepared
            .as_ref()
            .map_or_else(SystemTime::now, |walk| walk.observed);
        if self.closed.load(Ordering::Acquire) {
            return Err(LoadError::Closed);
        }
        if !self.graph.root.is_dir() {
            return Err(LoadError::Failed {
                reason: "graph root is unavailable".into(),
            });
        }
        let mut config_changed = false;
        let mut config_file = None;
        if include_config {
            let path = self.graph.root.join("logseq/config.edn");
            let mut current = stamp(&path);
            let mut previous = self.config_stamp.lock().unwrap();
            // A stamp without a hash means the file vanished (or failed to
            // read) between its metadata and its bytes: a sync delivery or
            // external editor removing it mid-cycle. Look again so a removal
            // reads as one; otherwise it passed as a hashless "modification"
            // and the later real removal compared hashless to absent, unseen.
            if (current.is_none() && previous.is_some())
                || current.as_ref().is_some_and(|value| value.rev.is_none())
            {
                current = stamp(&path);
            }
            // Byte-identity gate: taking in a config discards every parsed
            // page, so only a changed revision is read.
            let moved = previous.as_ref().and_then(|value| value.rev.as_ref())
                != current.as_ref().and_then(|value| value.rev.as_ref());
            match moved.then(|| self.read_config(&path)) {
                None => *previous = current,
                Some(Ok(())) => {
                    config_changed = true;
                    let kind = match (previous.as_ref(), current.as_ref()) {
                        (None, Some(_)) => ChangeKind::Created,
                        (Some(_), None) => ChangeKind::Removed,
                        _ => ChangeKind::Modified,
                    };
                    config_file = Some((
                        FileId::from("logseq/config.edn".to_owned()),
                        kind,
                        current.as_ref().and_then(|value| value.rev.clone()),
                    ));
                    *previous = current;
                }
                Some(Err(error)) if scan_semantics => return Err(error),
                // Refusal (sync delivery / external-editor race): the delivered
                // config names a page or journal directory that escapes the
                // graph. A watcher cycle keeps serving the last good config,
                // keeps the old stamp so a later cycle re-checks, and still
                // observes page files: a bad config must not blind the
                // watcher. Contract `docs/contracts/config-live-reload.md` §5.
                Some(Err(_)) => {}
            }
        }
        // A changed config can change which files are graph text (`:hidden`,
        // page and journal directories), so it takes a full scan.
        let paths = if config_changed { None } else { paths };
        let dirs = self.dirs.read().unwrap().clone();
        let mut snapshot = self.snapshot.lock().unwrap();
        let (mut now, mut unreadable) = if let Some(paths) = paths {
            (
                paths
                    .iter()
                    .filter(|path| {
                        if path.starts_with(self.graph.assets_path()) {
                            self.graph.ensure_asset_write_target(path).is_ok()
                        } else {
                            self.graph.ensure_write_target(path).is_ok()
                        }
                    })
                    .filter_map(|path| stamp(path).map(|value| (path.clone(), value)))
                    .collect(),
                Some(self.unreadable_dirs.lock().unwrap().clone()),
            )
        } else {
            let (files, errors, times) = match prepared {
                Some(walk) => (walk.files, walk.errors, walk.times),
                None => self.collect_full(&dirs, &self.graph.current_config()),
            };
            *walk = times;
            walk.full = true;
            (files, Some(errors))
        };
        #[cfg(test)]
        if snapshot.keys().any(|path| !now.contains_key(path)) {
            crate::store::pause_at_hook(&self.after_collect_pause);
        }
        if let Some(errors) = unreadable.as_ref() {
            for (path, value) in &*snapshot {
                if path
                    .ancestors()
                    .any(|ancestor| errors.contains_key(ancestor))
                {
                    now.entry(path.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        let mut racy = self.racy.lock().unwrap();
        // Stable paths need neither union allocation nor graph-wide sorting.
        // Copy their known revisions and sort only the paths requiring work.
        let names: HashSet<PathBuf> = if let Some(paths) = paths {
            paths.clone()
        } else {
            let mut names: HashSet<_> = now
                .iter_mut()
                .filter_map(|(path, new)| {
                    let old = snapshot.get(path);
                    let same = !force
                        && old.is_some_and(|old| {
                            old.modified == new.modified
                                && old.len == new.len
                                && (scan_semantics
                                    || (old.identity == new.identity && old.changed == new.changed))
                        });
                    if same && !racy.contains(path) {
                        new.rev = old.unwrap().rev.clone();
                        None
                    } else {
                        Some(path.clone())
                    }
                })
                .collect();
            names.extend(
                snapshot
                    .keys()
                    .filter(|path| !now.contains_key(*path))
                    .cloned(),
            );
            names
        };
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        let mut files = Vec::new();
        if let Some(config_file) = config_file {
            files.push(config_file);
        }
        let mut pages = Vec::new();
        let mut forwarded = Vec::new();
        for path in names {
            let before = snapshot.get(&path);
            if before.is_some() && !now.contains_key(&path) {
                if let Some(value) = stamp(&path) {
                    now.insert(path.clone(), value);
                }
            }
            if paths.is_none() {
                if let (Some(old), Some(new)) = (before, now.get_mut(&path)) {
                    let same = !force
                        && old.modified == new.modified
                        && old.len == new.len
                        && (scan_semantics
                            || (old.identity == new.identity && old.changed == new.changed));
                    if same && !racy.contains(&path) {
                        new.rev = old.rev.clone();
                        continue;
                    }
                    let moved = racy::hash_settled(&path, new);
                    if same && !moved {
                        // §5.4: a racy stamp cannot vouch for the bytes, so
                        // they were reread; it stops being racy once observed
                        // outside the window.
                        if !new.racy_at(observed) {
                            racy.remove(&path);
                        }
                        // A baseline entry recorded without a revision (a
                        // file the load pass did not read) only learns it.
                        if old.rev.is_none() || old.rev == new.rev {
                            if old.rev.is_none() && new.rev.is_none() {
                                racy.insert(path.clone());
                            }
                            continue;
                        }
                    }
                } else if let Some(new) = now.get_mut(&path) {
                    racy::hash_settled(&path, new);
                }
            }
            match now.get(&path) {
                Some(value) if value.rev.is_some() && value.racy_at(observed) => {
                    racy.insert(path.clone());
                }
                Some(value) if value.rev.is_some() => {
                    racy.remove(&path);
                }
                Some(_) => {}
                None => {
                    racy.remove(&path);
                }
            }
            let after = now.get(&path);
            let kind = match (before, after) {
                (None, Some(_)) => Some(ChangeKind::Created),
                (Some(_), None) => Some(ChangeKind::Removed),
                (Some(a), Some(b)) if a.rev != b.rev => Some(ChangeKind::Modified),
                (Some(a), Some(b)) if a.modified != b.modified => Some(ChangeKind::Touched),
                _ => None,
            };
            let Some(kind) = kind else {
                continue;
            };
            let Some(id) = self.file_id(&path) else {
                continue;
            };
            // STEP3 §5: the owner observes, indexes and publishes this read.
            // B1: a path of unknown identity against the held keys is never
            // installed from disk; each candidate's owner observes its page.
            let owners = match self.graph.held_identity(&path) {
                crate::model::entry_identity::Identity::Key(key) => vec![key],
                crate::model::entry_identity::Identity::Unknown { candidates, .. } => candidates,
                _ => Vec::new(),
            };
            if !owners.is_empty() {
                forwarded.extend(owners);
                retry_baseline(&mut now, &path, before);
                continue;
            }
            if tine_core::model::path_is_sync_conflict(&path) {
                // Conflict copies are in the file feed so the window adapter can
                // refresh the conflicts panel, but never enter the page cache.
            } else if matches!(kind, ChangeKind::Removed) {
                if let Some(entry) = self.graph.forget_file_internal(&path) {
                    pages.push((id.clone(), entry.kind, entry.name));
                }
            } else if matches!(kind, ChangeKind::Touched) {
                self.graph
                    .observe_page_mtime(&path, after.and_then(|value| value.modified));
            } else {
                let observed_rev = after.and_then(|value| value.rev.clone());
                #[cfg(test)]
                let observed_rev = if self.force_mismatched_rev_once.swap(false, Ordering::AcqRel) {
                    Some(FileRev::from_bytes(b"injected mismatched hash"))
                } else {
                    observed_rev
                };
                let Some(expected_rev) = observed_rev.as_ref() else {
                    unreadable
                        .as_mut()
                        .unwrap()
                        .insert(path.clone(), "file hash failed".into());
                    retry_baseline(&mut now, &path, before);
                    continue;
                };
                match self.graph.sync_file_internal(&path, Some(expected_rev)) {
                    SyncFileResult::Reconciled { entry, rev } => {
                        debug_assert_eq!(&rev, expected_rev);
                        unreadable.as_mut().unwrap().remove(&path);
                        if let Some(entry) = entry {
                            pages.push((id.clone(), entry.kind, entry.name));
                        }
                    }
                    SyncFileResult::ChangedDuringRead => {
                        unreadable.as_mut().unwrap().remove(&path);
                        retry_baseline(&mut now, &path, before);
                        continue;
                    }
                    SyncFileResult::ReadFailed(error) => {
                        unreadable
                            .as_mut()
                            .unwrap()
                            .insert(path.clone(), error.to_string());
                        retry_baseline(&mut now, &path, before);
                        continue;
                    }
                    SyncFileResult::Excluded => {
                        unreadable.as_mut().unwrap().remove(&path);
                    }
                }
            }
            files.push((id, kind, after.and_then(|value| value.rev.clone())));
        }
        if paths.is_none() {
            racy.retain(|path| now.contains_key(path));
            *snapshot = now;
        } else {
            for path in paths.unwrap() {
                if let Some(value) = now.get(path) {
                    snapshot.insert(path.clone(), value.clone());
                } else {
                    snapshot.remove(path);
                }
            }
        }
        let unreadable_changed = if let Some(errors) = unreadable {
            let mut previous = self.unreadable_dirs.lock().unwrap();
            let changed = *previous != errors;
            if changed {
                self.graph
                    .replace_unreadable_walk_errors(&previous, &errors);
                *previous = errors;
            }
            changed
        } else {
            false
        };
        drop(racy);
        drop(snapshot);
        if !forwarded.is_empty() {
            if let Some(forward) = self.forward.lock().unwrap().as_ref() {
                forward(forwarded);
            }
        }
        walk.changed = files.len() as u64;
        if let Some(deferred) = deferred {
            // The launch diff: its findings ride the Ready publication.
            deferred.files.extend(files);
            deferred.config_changed |= config_changed;
            deferred.pages.extend(pages);
        } else if !files.is_empty() || config_changed || unreadable_changed {
            self.changes.publish_watched(
                Origin::External,
                files,
                config_changed,
                pages,
                || {},
                batch,
            );
        }
        Ok(())
    }

    fn collect_full(
        &self,
        dirs: &[PathBuf; 1],
        config: &tine_core::Config,
    ) -> (
        HashMap<PathBuf, Stamp>,
        HashMap<PathBuf, String>,
        CollectTimes,
    ) {
        let found = collect_with_errors(dirs, config);
        #[cfg(test)]
        crate::store::pause_at_hook(&self.full_walk_pause);
        #[cfg(test)]
        if self.writer.try_lock().is_err() {
            self.full_walk_locked_files
                .fetch_add(found.0.len(), Ordering::Relaxed);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, Arc<crate::Store>) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("logseq")).unwrap();
        fs::write(dir.path().join("logseq/config.edn"), "{}\n").unwrap();
        for n in 0..20 {
            let path = dir.path().join(format!("pages/P{n}.md"));
            fs::write(&path, "- original\n").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(3600))
                .unwrap();
        }
        let store = Arc::new(
            crate::Store::open(dir.path(), Default::default())
                .unwrap()
                .0,
        );
        store.whole_graph().unwrap();
        (dir, store)
    }

    #[test]
    fn focus_full_walk_inspects_no_files_under_the_page_save_writer() {
        let (_dir, store) = open();
        let core = store.watch.core_for_load();
        core.full_walk_locked_files.store(0, Ordering::Relaxed);
        store.refresh(crate::Depth::Stamps).unwrap();
        let files = core.full_walk_locked_files.load(Ordering::Relaxed);
        store.close();
        assert_eq!(files, 0,
            "I-13/I-25: focus metadata enumeration must not block page reads/saves; exemplar watch/reconcile.rs");
    }

    #[test]
    fn focus_observation_retries_an_intervening_save_and_keeps_external_changes() {
        let (dir, store) = open();
        let core = store.watch.core_for_load();
        let pause: crate::store::TestPause =
            Arc::new((Mutex::new((false, false)), std::sync::Condvar::new()));
        *core.full_walk_pause.lock().unwrap() = Some(Arc::clone(&pause));
        let reader = Arc::clone(&store);
        let scan = std::thread::spawn(move || reader.refresh(crate::Depth::Stamps).unwrap());
        let (state, ready) = &*pause;
        let mut waiting = state.lock().unwrap();
        while !waiting.0 {
            let (next, timed) = ready
                .wait_timeout(waiting, Duration::from_secs(10))
                .unwrap();
            assert!(!timed.timed_out(), "full observation was not reached");
            waiting = next;
        }
        drop(waiting);
        // This is a lock/work assertion, not a wall-clock response threshold.
        let available = core.writer.try_lock().is_ok();
        if !available {
            state.lock().unwrap().1 = true;
            ready.notify_all();
            scan.join().unwrap();
            store.close();
            panic!("the full observation held the page/save writer");
        }
        let id = PageId::from("pages/P0.md");
        let mut read = store.page(&id).unwrap();
        read.doc.blocks[0].raw = "saved during observation".into();
        assert!(matches!(
            store.save(
                crate::EditKind::SaveBlock,
                &id,
                crate::SaveBase::Existing(read.rev),
                &read.doc
            ),
            crate::SaveOutcome::Saved(_)
        ));
        // A concurrent config publication changes eligibility as well as the
        // generation. Its writer must not wait for a directory-read guard.
        let config = dir.path().join("logseq/config.edn");
        let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
        tx.replace(
            &FileId::from("logseq/config.edn".to_owned()),
            FileRev::from_file(&config).unwrap(),
            b"{:hidden [\"pages/P2.md\"]}\n".to_vec(),
        );
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
        // Include changes delivered after the stale directory observation.
        fs::write(dir.path().join("pages/New.md"), "- late external\n").unwrap();
        fs::remove_file(dir.path().join("pages/P1.md")).unwrap();
        *core.full_walk_pause.lock().unwrap() = None;
        state.lock().unwrap().1 = true;
        ready.notify_all();
        scan.join().unwrap();
        assert_eq!(
            store.page(&id).unwrap().doc.blocks[0].raw,
            "saved during observation"
        );
        let graph = store.whole_graph().unwrap();
        assert!(matches!(
            graph.resolve("New", false),
            crate::Resolved::Existing { .. }
        ));
        assert!(matches!(
            graph.resolve("P1", false),
            crate::Resolved::Absent { .. }
        ));
        assert!(matches!(
            graph.resolve("P2", false),
            crate::Resolved::Absent { .. }
        ));
        store.close();
    }

    #[test]
    fn focus_still_observes_external_content_and_header_config_changes() {
        let (dir, store) = open();
        fs::write(dir.path().join("pages/P0.md"), "- delivered content\n").unwrap();
        fs::write(
            dir.path().join("logseq/config.edn"),
            "{:hidden [\"pages/P2.md\"]}\n",
        )
        .unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "delivered content"
        );
        assert!(matches!(
            store.whole_graph().unwrap().resolve("P2", false),
            crate::Resolved::Absent { .. }
        ));
        store.close();
    }
}
