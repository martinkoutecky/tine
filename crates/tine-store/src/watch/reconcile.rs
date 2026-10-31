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

fn retain_unreadable_stamps<'a>(
    previous: impl Iterator<Item = (&'a PathBuf, &'a Stamp)>,
    now: &mut HashMap<PathBuf, Stamp>,
    errors: &HashMap<PathBuf, String>,
    incremental: bool,
) {
    if errors.is_empty() {
        return;
    }
    for (path, value) in previous {
        if path
            .ancestors()
            // An incremental self-error must not restore an exact removal.
            .skip(usize::from(incremental))
            .any(|ancestor| errors.contains_key(ancestor))
        {
            now.entry(path.clone()).or_insert_with(|| value.clone());
        }
    }
}

#[test]
fn empty_errors_do_not_visit_prior_stamps() {
    for size in [100, 1000] {
        let visits = std::cell::Cell::new(0);
        let path = PathBuf::from("pages/Page.md");
        let stamp = Stamp {
            modified: None,
            len: 0,
            identity: 0,
            changed: 0,
            rev: None,
        };
        let previous = (0..size).map(|_| {
            visits.set(visits.get() + 1);
            (&path, &stamp)
        });
        retain_unreadable_stamps(previous, &mut HashMap::new(), &HashMap::new(), true);
        assert_eq!(visits.get(), 0, "prior stamp visits at N={size}");
    }
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
            let (files, errors, times) = self.collect_full(&config);
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
            // Byte-identity gate: only a changed revision needs a reload
            // and publication (build-key changes also discard parsed pages).
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
                    .filter_map(|path| stamp_metadata(path).map(|value| (path.clone(), value)))
                    .collect(),
                Some(self.unreadable_dirs.lock().unwrap().clone()),
            )
        } else {
            let (files, errors, times) = match prepared {
                Some(walk) => (walk.files, walk.errors, walk.times),
                None => self.collect_full(&self.graph.current_config()),
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
            retain_unreadable_stamps(snapshot.iter(), &mut now, errors, paths.is_some());
        }
        let mut racy = self.racy.lock().unwrap();
        let mut refusals = self.read_refusals.lock().unwrap();
        // A full listing reports directory errors afresh; unchanged file
        // refusals must survive it without being read or published again.
        for (path, refusal) in &*refusals {
            unreadable
                .as_mut()
                .unwrap()
                .insert(path.clone(), refusal.reason.clone());
        }
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
                    let refusal_moved = refusals
                        .get(path)
                        .is_some_and(|refusal| !refusal.stamp.same_metadata(new));
                    if same && !racy.contains(path) && !refusal_moved {
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
            // A new in-flux file has no committed baseline yet. A later full
            // scan must also clear its uncertainty if it disappears.
            names.extend(self.graph.unsettled_read_paths());
            // Failed first reads can have neither a page entry nor a committed
            // stamp. Their disappearance is still a change to the notice.
            names.extend(
                self.graph
                    .reported_unreadable_pages()
                    .iter()
                    .map(|(id, _)| self.graph.root.join(id.as_str()))
                    .chain(unreadable.as_ref().unwrap().keys().cloned())
                    .filter(|path| !now.contains_key(path)),
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
        let mut retired_errors = false;
        // One identity pass for the cycle (REVIEW-3a4 #6).
        let mut identities = crate::model::entry_identity::Memo::default();
        for path in names {
            let before = snapshot.get(&path);
            if before.is_some() && !now.contains_key(&path) {
                if let Some(value) = stamp_metadata(&path) {
                    now.insert(path.clone(), value);
                }
            }
            if before.is_none() && !now.contains_key(&path) {
                self.graph.note_unsettled_read(&path, None);
                // Do not mistake an inaccessible ancestor for deletion. This
                // also rechecks a replacement arriving after the listing.
                if matches!(fs::symlink_metadata(&path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                {
                    retired_errors |= self.graph.forget_unreadable(&path);
                    unreadable.as_mut().unwrap().remove(&path);
                    refusals.remove(&path);
                    racy.remove(&path);
                }
                continue;
            }
            if !force
                && !config_changed
                && refusals.get(&path).is_some_and(|refusal| {
                    now.get(&path)
                        .is_some_and(|new| refusal.stamp.same_metadata(new))
                })
            {
                if let Some(new) = now.get_mut(&path) {
                    new.rev = refusals[&path].stamp.rev.clone();
                }
                continue;
            }
            let refused = refusals.remove(&path).is_some();
            if refused {
                unreadable.as_mut().unwrap().remove(&path);
            }
            let mut hash_error = None;
            if paths.is_none() {
                if let (Some(old), Some(new)) = (before, now.get_mut(&path)) {
                    let same = !force
                        && old.modified == new.modified
                        && old.len == new.len
                        && (scan_semantics
                            || (old.identity == new.identity && old.changed == new.changed));
                    if same && !racy.contains(&path) && !refused {
                        new.rev = old.rev.clone();
                        continue;
                    }
                    let hashed = racy::hash_settled(&path, new);
                    if same && matches!(hashed, Ok(false)) && !refused {
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
                    hash_error = hashed.err();
                } else if let Some(new) = now.get_mut(&path) {
                    hash_error = racy::hash_settled(&path, new).err();
                }
            } else if let Some(new) = now.get_mut(&path) {
                // Keep exact notifications on their existing one-stamp path:
                // sync_file_internal compares this hash with the bounded read.
                // Only a failed read needs the extra metadata observation below.
                match racy::hash_revision(&path) {
                    Ok(rev) => new.rev = Some(rev),
                    Err(error) => hash_error = Some(error),
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
                (Some(_), Some(_)) if refused || hash_error.is_some() => Some(ChangeKind::Modified),
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
            let owners = match self.graph.held_identity_in(&mut identities, &path) {
                crate::model::entry_identity::Identity::Key(key) => vec![key],
                crate::model::entry_identity::Identity::Unknown { candidates, .. } => {
                    // Reread from disk when they release it (REVIEW-3a4 #3).
                    self.graph.note_withheld(&candidates, &path);
                    candidates
                }
                _ => Vec::new(),
            };
            if !owners.is_empty() {
                self.graph.note_unsettled_read(&path, None);
                forwarded.extend(owners);
                retry_baseline(&mut now, &path, before);
                continue;
            }
            if tine_core::model::path_is_sync_conflict(&path) {
                // Conflict copies are in the file feed so the window adapter can
                // refresh the conflicts panel, but never enter the page cache.
            } else if matches!(kind, ChangeKind::Removed) {
                self.graph.note_unsettled_read(&path, None);
                unreadable.as_mut().unwrap().remove(&path);
                if let Some(entry) = self.graph.forget_file_internal(&path) {
                    pages.push((id.clone(), entry.kind, entry.name));
                }
            } else if matches!(kind, ChangeKind::Touched) {
                self.graph.note_unsettled_read(&path, None);
                unreadable.as_mut().unwrap().remove(&path);
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
                let result = match observed_rev.as_ref() {
                    Some(expected) => self.graph.sync_file_internal(&path, Some(expected)),
                    None => SyncFileResult::ReadFailed(
                        hash_error.unwrap_or_else(|| std::io::Error::other("file hash failed")),
                    ),
                };
                match result {
                    SyncFileResult::Reconciled { entry, rev } => {
                        self.graph.note_unsettled_read(&path, None);
                        debug_assert_eq!(Some(&rev), observed_rev.as_ref());
                        unreadable.as_mut().unwrap().remove(&path);
                        if let Some(entry) = entry {
                            pages.push((id.clone(), entry.kind, entry.name));
                        }
                    }
                    SyncFileResult::ChangedDuringRead => {
                        self.graph
                            .note_unsettled_read(&path, Some("file changed during read".into()));
                        unreadable.as_mut().unwrap().remove(&path);
                        self.retry_read_later();
                        retry_baseline(&mut now, &path, before);
                        continue;
                    }
                    SyncFileResult::ReadFailed(error) => {
                        let error = crate::IoError::from(error);
                        let value = after.unwrap();
                        // A delivered partial UTF-8 sequence or a permission
                        // change during the read is still in flux. Use §5.4's
                        // existing settling window, and re-stat before treating
                        // any error as belonging to this observation.
                        let settled = !value.racy_at(observed)
                            && stamp_metadata(&path)
                                .is_some_and(|fresh| fresh.same_metadata(value));
                        // Access can recover without a stamp change (Windows
                        // ACLs, mobile scopes, minifilters). Retry its cheap
                        // failed open on the normal event/poll cadence.
                        let terminal = error.kind() == std::io::ErrorKind::InvalidData;
                        if settled {
                            self.graph.note_unsettled_read(&path, None);
                            if let Some(error) = crate::model::unreadable_error(
                                error.clone(),
                                "Tine unreadable file producer=watch-read",
                            ) {
                                unreadable
                                    .as_mut()
                                    .unwrap()
                                    .insert(path.clone(), error.to_string());
                            }
                        } else {
                            self.graph
                                .note_unsettled_read(&path, Some(error.to_string()));
                            unreadable.as_mut().unwrap().remove(&path);
                        }
                        if settled && terminal {
                            refusals.insert(
                                path.clone(),
                                ReadRefusal {
                                    stamp: value.clone(),
                                    reason: error.to_string(),
                                },
                            );
                            racy.remove(&path);
                        } else {
                            if !settled {
                                self.retry_read_later();
                            }
                            retry_baseline(&mut now, &path, before);
                        }
                        continue;
                    }
                    SyncFileResult::Excluded => {
                        self.graph.note_unsettled_read(&path, None);
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
        let unreadable_changed = unreadable_changed || retired_errors;
        drop(racy);
        drop(refusals);
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
        } else if !config_changed && self.changes.first_view_pending(&self.load) {
            // Before the first complete view (a hold released while the
            // graph loads), the observation waits for it as an own save does
            // (`store/first_view.rs`, card hB0p3s): capturing a view now
            // would build the whole graph here, under the writer. The page
            // already reads from its file; the launch pass takes the row in.
            if !files.is_empty() || unreadable_changed {
                let journals = format!("{}/", self.graph.current_config().journals_dir);
                let journal_set_changed = files.iter().any(|(id, kind, _)| {
                    id.as_str().starts_with(&journals)
                        && matches!(kind, ChangeKind::Created | ChangeKind::Removed)
                });
                self.changes.defer_publication(
                    Origin::External,
                    files,
                    pages,
                    (HashSet::new(), journal_set_changed),
                );
            }
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

    /// Notify mode retries in-flux reads without waiting for another OS event.
    /// Settled I/O failures use the normal event/poll cadence. Keep the
    /// existing racy follow-up deadline when one is pending.
    fn retry_read_later(&self) {
        self.follow_up
            .lock()
            .unwrap()
            .get_or_insert_with(|| Instant::now() + RACY_FOLLOW_UP);
    }

    fn collect_full(
        &self,
        config: &tine_core::Config,
    ) -> (
        HashMap<PathBuf, Stamp>,
        HashMap<PathBuf, String>,
        CollectTimes,
    ) {
        let found = collect_with_errors(&self.graph.root, config);
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

    #[cfg(unix)]
    fn unreadable_without_a_page_entry(exact_notification: bool, repair: bool) {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        fs::create_dir(dir.path().join("journals")).unwrap();
        let path = dir.path().join("journals/2026_10_26.md");
        fs::write(&path, "- day\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            eprintln!("permission probe skipped: process bypasses access controls");
            return;
        }
        let store = crate::Store::open(
            dir.path(),
            crate::OpenOptions {
                watch: crate::WatchMode::Poll,
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        store.whole_graph_reconciled().unwrap();
        store.watch.stop_background_for_test();
        assert_eq!(store.whole_graph().unwrap().unreadable_files().len(), 1);
        if repair {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        if exact_notification {
            store
                .watch
                .core_for_load()
                .reconcile(
                    Some(&HashSet::from([path])),
                    false,
                    false,
                    DiffTrigger::Test,
                )
                .unwrap();
        } else {
            store.refresh(crate::Depth::Stamps).unwrap();
        }
        let view = store.whole_graph().unwrap();
        assert!(
            view.unreadable_files().is_empty(),
            "{:?}",
            view.unreadable_files()
        );
        if repair {
            assert_eq!(
                store
                    .page(&PageId::from("journals/2026_10_26.md"))
                    .unwrap()
                    .doc
                    .blocks[0]
                    .raw,
                "day"
            );
        }
        store.close();
    }

    #[cfg(unix)]
    #[test]
    fn stamps_retires_a_deleted_unreadable_file_without_a_page_entry() {
        unreadable_without_a_page_entry(false, false);
    }

    #[cfg(unix)]
    #[test]
    fn notification_retires_a_deleted_unreadable_file_without_a_page_entry() {
        unreadable_without_a_page_entry(true, false);
    }

    #[cfg(unix)]
    #[test]
    fn stamps_clears_a_repaired_unreadable_file_without_a_page_entry() {
        unreadable_without_a_page_entry(false, true);
    }

    #[cfg(unix)]
    #[test]
    fn notification_clears_a_repaired_unreadable_file_without_a_page_entry() {
        unreadable_without_a_page_entry(true, true);
    }

    fn open() -> (tempfile::TempDir, Arc<crate::Store>) {
        // The store keys watcher state by its canonical root, as the watcher's
        // own events are. On a Windows runner the temp dir is an 8.3 spelling
        // (`C:\Users\RUNNER~1\...`) and the canonical root a `\\?\` long
        // name, so path lookups (snapshot, refusals, hints, injected faults)
        // must start from the canonical temp dir.
        let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
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

    fn read_failure_is_retained_until_repair(break_file: impl Fn(&Path), repair: impl Fn(&Path)) {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        let file = fs::File::options().write(true).open(&path).unwrap();
        break_file(&path);
        file.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        racy::HASH_ATTEMPTS.with(|count| count.set(0));
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert_eq!(racy::HASH_ATTEMPTS.with(|count| count.get()), 1);
        let view = store.whole_graph().unwrap();
        let errors = view.unreadable_files();
        assert!(
            errors
                .iter()
                .any(|(id, reason)| { id.as_str() == "pages/P0.md" && !reason.is_empty() }),
            "a settled refusal must name the file and reason: {errors:?}"
        );
        let generation = store.graph.cache_generation();
        let changes = store.subscribe();
        // Both a poll and a duplicate notification must preserve the refusal's
        // real stamp, rather than invalidate the baseline to force another read.
        for paths in [None, Some(HashSet::from([path.clone()])), None] {
            core.reconcile(paths.as_ref(), false, false, DiffTrigger::Test)
                .unwrap();
            assert!(
                core.snapshot.lock().unwrap()[&path].same_metadata(&stamp_metadata(&path).unwrap()),
                "terminal failure was queued for another read"
            );
            assert_eq!(store.graph.cache_generation(), generation);
            assert!(changes.try_recv().unwrap().is_none());
            assert_eq!(
                racy::HASH_ATTEMPTS.with(|count| count.get()),
                1,
                "unchanged terminal file was read again"
            );
        }
        // A metadata-only touch must retry even if the bytes/revision still
        // match the previous refused observation (including denied opens).
        file.set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        core.reconcile(None, false, true, DiffTrigger::Test)
            .unwrap();
        assert_eq!(racy::HASH_ATTEMPTS.with(|count| count.get()), 2);
        repair(&path);
        fs::write(&path, "- repaired\n").unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(!store
            .whole_graph()
            .unwrap()
            .unreadable_files()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/P0.md"));
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "repaired"
        );
        store.close();
    }

    #[test]
    fn gh543_r1_settled_io_failure_does_not_renew_full_walks() {
        for kind in [
            std::io::ErrorKind::Other,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::Interrupted,
        ] {
            let (dir, store) = open();
            store.watch.stop_background_for_test();
            let core = store.watch.core_for_load();
            *core.follow_up.lock().unwrap() = None;
            let path = dir.path().join("pages/P0.md");
            fs::write(&path, "- unavailable cloud delivery\n").unwrap();
            age(&path);
            let _fault = OpenFailure::new(&path, kind);
            let walks = store.graph.diag.snapshot("ready")["fullDiffs"]["total"]
                .as_u64()
                .unwrap();
            core.reconcile(None, false, false, DiffTrigger::Test)
                .unwrap();
            let generation = store.graph.cache_generation();
            let changes = store.subscribe();
            // Advance pending deadlines without sleeping: the real watcher entry
            // point must perform no follow-up walks for this settled failure.
            for _ in 0..3 {
                if let Some(at) = core.follow_up.lock().unwrap().as_mut() {
                    *at = Instant::now();
                }
                core.follow_up_if_due();
            }
            assert_eq!(
                store.graph.diag.snapshot("ready")["fullDiffs"]["total"]
                    .as_u64()
                    .unwrap()
                    - walks,
                1,
                "a settled failure must not self-renew full directory walks"
            );
            assert!(core.follow_up.lock().unwrap().is_none());
            assert_eq!(store.graph.cache_generation(), generation);
            assert!(changes.try_recv().unwrap().is_none());
            // A normal later poll still retries and retains the refusal quietly.
            racy::HASH_ATTEMPTS.with(|count| count.set(0));
            core.reconcile(None, false, false, DiffTrigger::Test)
                .unwrap();
            assert_eq!(racy::HASH_ATTEMPTS.with(|count| count.get()), 1);
            assert_eq!(store.graph.cache_generation(), generation);
            assert!(changes.try_recv().unwrap().is_none());
            store.close();
        }
    }

    struct OpenFailure;

    impl OpenFailure {
        fn new(path: &Path, kind: std::io::ErrorKind) -> Self {
            racy::HASH_OPEN_ERROR.with(|fault| *fault.borrow_mut() = Some((path.into(), kind)));
            Self
        }
    }

    impl Drop for OpenFailure {
        fn drop(&mut self) {
            racy::HASH_OPEN_ERROR.with(|fault| *fault.borrow_mut() = None);
        }
    }

    fn age(path: &Path) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
    }

    #[test]
    fn restore_retires_read_uncertainty_and_terminal_refusals() {
        for terminal in [false, true] {
            let (dir, store) = open();
            fs::create_dir(dir.path().join("assets")).unwrap();
            store.watch.stop_background_for_test();
            let core = store.watch.core_for_load();
            let path = dir.path().join("pages/P0.md");
            fs::write(&path, b"\xff").unwrap();
            if terminal {
                age(&path);
            }
            core.reconcile(None, false, false, DiffTrigger::Test)
                .unwrap();
            assert!(store.referenced_assets().is_err());
            if terminal {
                assert!(core.read_refusals.lock().unwrap().contains_key(&path));
            } else {
                assert!(store.graph.unsettled_read_paths().contains(&path));
            }
            store
                .restore(
                    crate::EditKind::SaveBlock,
                    vec![crate::RestoreFile {
                        area: crate::Area::Pages,
                        rel: "Restored.md".into(),
                        bytes: b"- restored\n".to_vec(),
                    }],
                    None,
                )
                .unwrap();
            assert!(
                store.referenced_assets().is_ok(),
                "restore left the inventory refused"
            );
            assert!(store.graph.unsettled_read_paths().is_empty());
            assert!(core.read_refusals.lock().unwrap().is_empty());
            core.reconcile(None, false, true, DiffTrigger::Test)
                .unwrap();
            assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
            store.close();
        }
    }

    #[test]
    fn stamps_retries_an_unchanged_denied_open_but_not_invalid_content() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        let original = stamp_metadata(&path).unwrap();
        let paths = HashSet::from([path.clone()]);
        let fault = OpenFailure::new(&path, std::io::ErrorKind::PermissionDenied);
        core.reconcile(Some(&paths), false, false, DiffTrigger::Test)
            .unwrap();
        assert!(!store.whole_graph().unwrap().unreadable_files().is_empty());
        drop(fault);
        assert!(stamp_metadata(&path).unwrap().same_metadata(&original));
        store.refresh(crate::Depth::Stamps).unwrap();
        assert!(
            store.whole_graph().unwrap().unreadable_files().is_empty(),
            "stamps skipped an access repair"
        );
        fs::write(&path, b"\xff").unwrap();
        age(&path);
        store.refresh(crate::Depth::Stamps).unwrap();
        racy::HASH_ATTEMPTS.with(|count| count.set(0));
        for _ in 0..3 {
            store.refresh(crate::Depth::Stamps).unwrap();
        }
        assert_eq!(
            racy::HASH_ATTEMPTS.with(|count| count.get()),
            0,
            "stamps reread invalid content"
        );
        store.close();
    }

    #[test]
    fn an_identical_restore_revalidates_content_before_retiring_uncertainty() {
        for valid in [false, true] {
            let (dir, store) = open();
            fs::create_dir(dir.path().join("assets")).unwrap();
            store.watch.stop_background_for_test();
            let core = store.watch.core_for_load();
            let path = dir.path().join("pages/P0.md");
            if valid {
                fs::write(&path, "title:: Before\nicon:: 📚\n\n- original\n").unwrap();
                store.refresh(crate::Depth::Bytes).unwrap();
                // Populate the previous generation's lazy icon index, too.
                store.whole_graph().unwrap().page_icons(&["Before".into()]);
                fs::write(
                    &path,
                    "title:: Delivered\nicon:: 🚀\n\n- delivered before restore\n",
                )
                .unwrap();
                store
                    .graph
                    .fail_sync_read_once
                    .store(true, Ordering::Release);
            } else {
                fs::write(&path, b"\xff").unwrap();
            }
            core.reconcile(None, false, false, DiffTrigger::Test)
                .unwrap();
            assert!(store.referenced_assets().is_err());
            let files = (0..20)
                .map(|n| crate::RestoreFile {
                    area: crate::Area::Pages,
                    rel: format!("P{n}.md"),
                    bytes: fs::read(dir.path().join(format!("pages/P{n}.md"))).unwrap(),
                })
                .collect();
            // Restore writes the exact same complete file set. The file-feed
            // byte diff is empty, but its old read uncertainty still must be
            // replaced by an authoritative parse, including a real refusal.
            store
                .restore(crate::EditKind::SaveBlock, files, None)
                .unwrap();
            assert!(store.graph.unsettled_read_paths().is_empty());
            assert_eq!(store.referenced_assets().is_ok(), valid);
            assert_eq!(
                store.whole_graph().unwrap().unreadable_files().is_empty(),
                valid
            );
            if valid {
                let copy = tempfile::tempdir().unwrap();
                for area in ["pages", "logseq"] {
                    fs::create_dir(copy.path().join(area)).unwrap();
                    for entry in fs::read_dir(dir.path().join(area)).unwrap() {
                        let entry = entry.unwrap();
                        if !entry.file_type().unwrap().is_file() {
                            continue;
                        }
                        fs::copy(entry.path(), copy.path().join(area).join(entry.file_name()))
                            .unwrap();
                    }
                }
                let fresh = crate::Store::open(copy.path(), Default::default())
                    .unwrap()
                    .0;
                let live = store.whole_graph().unwrap();
                let cold = fresh.whole_graph_reconciled().unwrap();
                let claim = |view: &crate::WholeGraph, name: &str| match view.resolve(name, false) {
                    crate::Resolved::Existing { id, others } => (true, id, others),
                    crate::Resolved::Absent { id } => (false, id, Vec::new()),
                    crate::Resolved::Alias { .. } => panic!("unexpected alias"),
                };
                for name in ["Before", "Delivered", "P0"] {
                    assert_eq!(claim(&live, name), claim(&cold, name), "resolve {name}");
                }
                let inventory = |view: &crate::WholeGraph| {
                    view.inventory(crate::InventoryScope::All)
                        .0
                        .iter()
                        .map(|entry| (entry.name.clone(), claim(view, &entry.name)))
                        .collect::<Vec<_>>()
                };
                assert_eq!(inventory(&live), inventory(&cold));
                let names = ["Before".into(), "Delivered".into(), "P0".into()];
                assert_eq!(live.page_icons(&names), cold.page_icons(&names));
                assert_eq!(live.page_icons(&names).get("Delivered").unwrap(), "🚀");
                fresh.close();
                assert_eq!(
                    store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
                    "delivered before restore"
                );
            }
            store.close();
        }
    }

    #[test]
    fn a_panicking_promoted_journal_leaves_other_pages_usable() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        fs::create_dir(dir.path().join("journals")).unwrap();
        let winner = dir.path().join("journals/2026_10_02.md");
        let loser = dir.path().join("journals/2026-10-02.md");
        fs::write(&winner, "- winning journal [[P1]]\n").unwrap();
        fs::write(
            &loser,
            format!("- {}\n", crate::model::TEST_PAGE_PARSE_PANIC_SENTINEL),
        )
        .unwrap();
        store.refresh(crate::Depth::Bytes).unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        fs::remove_file(&winner).unwrap();
        store
            .graph
            .fail_sync_parse_once
            .store(true, Ordering::Release);
        store.refresh(crate::Depth::Bytes).unwrap();
        assert!(!store.graph.fail_sync_parse_once.load(Ordering::Acquire));
        let view = store.whole_graph().unwrap();
        let unreadable: Vec<_> = view
            .unreadable_files()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(
            unreadable,
            [crate::FileId::from("journals/2026-10-02.md".to_owned())]
        );
        assert!(view.explicit_referrers(&["P1".into()]).is_empty());
        assert_eq!(
            store.page(&PageId::from("pages/P1.md")).unwrap().doc.blocks[0].raw,
            "original"
        );
        // The next writer operation must remain usable, too.
        fs::write(dir.path().join("pages/P1.md"), "- unrelated repair\n").unwrap();
        store.refresh(crate::Depth::Bytes).unwrap();
        assert_eq!(
            store.page(&PageId::from("pages/P1.md")).unwrap().doc.blocks[0].raw,
            "unrelated repair"
        );
        let fresh = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let cold = fresh.whole_graph_reconciled().unwrap();
        let cold_unreadable: Vec<_> = cold
            .unreadable_files()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(unreadable, cold_unreadable);
        assert!(cold.explicit_referrers(&["P1".into()]).is_empty());
        fresh.close();
        // Deleting the failed journal retires its record, as a fresh start has none.
        // Both the watcher's path-hinted reconcile and a full refresh.
        fs::remove_file(&loser).unwrap();
        let core = store.watch.core_for_load();
        core.reconcile(
            Some(&HashSet::from([loser.clone()])),
            false,
            false,
            DiffTrigger::Test,
        )
        .unwrap();
        let view = store.whole_graph().unwrap();
        assert!(
            view.unreadable_files().is_empty(),
            "{:?}",
            view.unreadable_files()
        );
        assert!(store.referenced_assets().is_ok());
        store.refresh(crate::Depth::Bytes).unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        store.close();
    }

    #[test]
    fn gh543_r1_permission_only_repair_with_identical_stamp_recovers() {
        // Models Windows ACL/minifilter and mobile security-scope repairs:
        // availability changes while every metadata field stays identical.
        for hinted in [false, true] {
            let (dir, store) = open();
            store.watch.stop_background_for_test();
            let core = store.watch.core_for_load();
            let path = dir.path().join("pages/P0.md");
            fs::write(&path, "- delivered before access was denied\n").unwrap();
            age(&path);
            let original = stamp_metadata(&path).unwrap();
            let fault = OpenFailure::new(&path, std::io::ErrorKind::PermissionDenied);
            core.reconcile(None, false, false, DiffTrigger::Test)
                .unwrap();
            let generation = store.graph.cache_generation();
            let changes = store.subscribe();
            racy::HASH_ATTEMPTS.with(|count| count.set(0));
            let paths = hinted.then(|| HashSet::from([path.clone()]));
            core.reconcile(paths.as_ref(), false, false, DiffTrigger::Test)
                .unwrap();
            assert_eq!(
                racy::HASH_ATTEMPTS.with(|count| count.get()),
                1,
                "an unchanged access failure must retry its open on the next cycle"
            );
            assert_eq!(store.graph.cache_generation(), generation);
            assert!(changes.try_recv().unwrap().is_none());
            drop(fault);
            assert!(stamp_metadata(&path).unwrap().same_metadata(&original));
            core.reconcile(paths.as_ref(), false, false, DiffTrigger::Test)
                .unwrap();
            assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
            assert_eq!(
                store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
                "delivered before access was denied"
            );
            store.close();
        }
    }

    #[test]
    fn gh543_r1_full_walk_file_error_keeps_the_last_good_page() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        let (mut files, mut errors, times) = core.collect_full(&store.graph.current_config());
        files.remove(&path);
        errors.insert(path.clone(), "injected file_type/lstat failure".into());
        // Make the fallback path stat unavailable as it is for EACCES/ESTALE;
        // the prepared walk's explicit file error must retain its baseline.
        fs::rename(&path, dir.path().join("inaccessible.md")).unwrap();
        {
            let _writer = core.writer.lock().unwrap();
            core.reconcile_prepared(
                None,
                false,
                false,
                None,
                DiffTrigger::Test,
                None,
                Some(PreparedWalk {
                    began: Instant::now(),
                    observed: SystemTime::now(),
                    files,
                    errors,
                    times,
                }),
            )
            .unwrap();
        }
        let view = store.whole_graph().unwrap();
        assert!(
            matches!(view.resolve("P0", false), crate::Resolved::Existing { .. }),
            "a failed file observation must retain its last good page"
        );
        assert!(view
            .unreadable_files()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/P0.md"));
        assert!(core.snapshot.lock().unwrap().contains_key(&path));
        store.close();
    }

    #[test]
    fn gh543_r1_save_racing_append_or_truncation_remains_a_conflict() {
        for external in ["- original\n- external append\n", "- cut\n"] {
            let (dir, store) = open();
            store.watch.stop_background_for_test();
            let path = dir.path().join("pages/P0.md");
            let id = PageId::from("pages/P0.md");
            let mut read = store.page(&id).unwrap();
            read.doc.blocks[0].raw = "unsaved typing".into();
            let delivering = path.clone();
            crate::model::READ_PARSE_BYTES_HOOK.with(|hook| {
                *hook.borrow_mut() =
                    Some(Box::new(move || fs::write(delivering, external).unwrap()));
            });
            let outcome = store.save(
                crate::EditKind::SaveBlock,
                &id,
                crate::SaveBase::Existing(read.rev),
                &read.doc,
            );
            assert!(
                matches!(outcome, crate::SaveOutcome::Conflict { .. }),
                "external delivery during save staging must use the conflict flow: {outcome:?}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), external);
            assert_eq!(read.doc.blocks[0].raw, "unsaved typing");
            store.close();
        }
    }

    #[test]
    fn gh543_a_short_read_during_delivery_is_quiet_and_recovered() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, "- a settled file replaced during its read\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        let delivering = path.clone();
        crate::model::READ_PARSE_BYTES_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(delivering, b"- partial \xf0").unwrap()
            }));
        });
        let changes = store.subscribe();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert!(changes.try_recv().unwrap().is_none());
        fs::write(&path, "- finished delivery\n").unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "finished delivery"
        );
        store.close();
    }

    #[test]
    fn gh543_revision_changed_during_read_is_already_quiet_and_retried() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, "- changed between hash and read\n").unwrap();
        core.force_mismatched_rev_once
            .store(true, Ordering::Release);
        let changes = store.subscribe();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert!(changes.try_recv().unwrap().is_none());
        assert!(
            core.follow_up.lock().unwrap().is_some(),
            "notify mode needs a retry without another event"
        );
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "changed between hash and read"
        );
        store.close();
    }

    #[test]
    fn gh543_partial_utf8_retries_quietly_then_reports_when_settled() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, b"- partial \xf0\x9f").unwrap();
        core.reconcile(
            Some(&HashSet::from([path.clone()])),
            false,
            false,
            DiffTrigger::Test,
        )
        .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert!(core.follow_up.lock().unwrap().is_some());
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store
            .whole_graph()
            .unwrap()
            .unreadable_files()
            .iter()
            .any(|(id, reason)| id.as_str() == "pages/P0.md" && reason.contains("utf-8")));
        fs::write(&path, "- complete 🐈\n").unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        store.close();
    }

    #[test]
    fn gh543_quiet_uncertainty_still_guards_asset_trash_and_clears_after_removal() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/New.md");
        fs::write(&path, b"- ![](../assets/kept.png) \xf0").unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert!(store
            .graph
            .unreadable_pages()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/New.md"));
        assert!(
            store.referenced_assets().is_err(),
            "a quiet retry must not authorize orphan trash"
        );
        fs::remove_file(&path).unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.graph.unreadable_pages().is_empty());
        assert!(store.referenced_assets().is_ok());
        store.close();
    }

    #[test]
    fn gh543_oversize_read_stops_until_the_file_changes() {
        read_failure_is_retained_until_repair(
            |path| {
                fs::File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_len(crate::model::PARSE_INPUT_MAX_BYTES + 1)
                    .unwrap();
            },
            |_| {},
        );
    }

    #[test]
    fn gh543_invalid_utf8_read_stops_until_the_file_changes() {
        read_failure_is_retained_until_repair(
            |path| {
                fs::write(path, b"\xff\xfe").unwrap();
            },
            |_| {},
        );
    }

    #[test]
    fn gh543_removing_a_refused_file_clears_its_notice_incrementally() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, b"\xff").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(!store.whole_graph().unwrap().unreadable_files().is_empty());
        fs::remove_file(&path).unwrap();
        core.reconcile(
            Some(&HashSet::from([path.clone()])),
            false,
            false,
            DiffTrigger::Test,
        )
        .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert!(!core.read_refusals.lock().unwrap().contains_key(&path));
        assert!(!core.snapshot.lock().unwrap().contains_key(&path));
        store.close();
    }

    #[test]
    fn gh543_explicit_byte_refresh_retries_an_unchanged_refusal() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, b"\xff").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        racy::HASH_ATTEMPTS.with(|count| count.set(0));
        core.reconcile(
            Some(&HashSet::from([path.clone()])),
            false,
            false,
            DiffTrigger::Bytes,
        )
        .unwrap();
        assert_eq!(racy::HASH_ATTEMPTS.with(|count| count.get()), 1);
        assert!(!store.whole_graph().unwrap().unreadable_files().is_empty());
        store.close();
    }

    #[cfg(unix)]
    #[test]
    fn gh543_permission_denied_retries_quietly_and_recovers_after_chmod_only() {
        use std::os::unix::fs::PermissionsExt;
        let probe = tempfile::NamedTempFile::new().unwrap();
        fs::set_permissions(probe.path(), fs::Permissions::from_mode(0o000)).unwrap();
        let denied = fs::File::open(probe.path()).is_err();
        fs::set_permissions(probe.path(), fs::Permissions::from_mode(0o600)).unwrap();
        if !denied {
            eprintln!("permission probe skipped: process bypasses access controls");
            return;
        }
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, "- delivered before chmod\n").unwrap();
        age(&path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(!store.whole_graph().unwrap().unreadable_files().is_empty());
        let generation = store.graph.cache_generation();
        let changes = store.subscribe();
        racy::HASH_ATTEMPTS.with(|count| count.set(0));
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert_eq!(racy::HASH_ATTEMPTS.with(|count| count.get()), 1);
        assert_eq!(store.graph.cache_generation(), generation);
        assert!(changes.try_recv().unwrap().is_none());
        // No rewrite or mtime touch accompanies this access repair.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "delivered before chmod"
        );
        store.close();
    }

    #[test]
    fn gh543_in_flux_read_retries_without_an_unreadable_notice() {
        let (dir, store) = open();
        store.watch.stop_background_for_test();
        let core = store.watch.core_for_load();
        let path = dir.path().join("pages/P0.md");
        fs::write(&path, "- external delivery\n").unwrap();
        store
            .graph
            .fail_sync_read_once
            .store(true, Ordering::Release);
        let changes = store.subscribe();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(
            store.whole_graph().unwrap().unreadable_files().is_empty(),
            "an unsettled read was announced as unreadable"
        );
        assert!(changes.try_recv().unwrap().is_none());
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert_eq!(
            store.page(&PageId::from("pages/P0.md")).unwrap().doc.blocks[0].raw,
            "external delivery"
        );
        fs::write(&path, "- another unfinished delivery\n").unwrap();
        store
            .graph
            .fail_sync_read_once
            .store(true, Ordering::Release);
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(!store.graph.unreadable_pages().is_empty());
        // Restoring the last good bytes takes the Touched path, which must
        // also clear internal uncertainty rather than leave storage blocked.
        fs::write(&path, "- external delivery\n").unwrap();
        core.reconcile(None, false, false, DiffTrigger::Test)
            .unwrap();
        assert!(store.graph.unreadable_pages().is_empty());
        assert!(store.referenced_assets().is_ok());
        store.close();
    }

    #[test]
    fn focus_full_walk_inspects_no_files_under_the_page_save_writer() {
        let (_dir, store) = open();
        let core = store.watch.core_for_load();
        // REG-LOAD-FLAKY-FOCUS-WRITER: the background watcher's own full
        // walks (its install diff, the launch's racy follow-up) and its
        // writer-held cycles share this core's counter and writer; under load
        // they overlapped the measured refresh and were billed to it.
        store.watch.stop_background_for_test();
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
        // REG-LOAD-FLAKY-FOCUS-WRITER: only the refresh may reach the walk
        // pause, and only this test may hold the writer while it waits there.
        // Under load the background watcher's own diffs can reach the same
        // hook or hold the writer (an install diff's config check, an asset
        // pass), so the probe below judged the watcher, not the refresh
        // (failed at load ~14, beta.5 og gate); with it stopped,
        // the late external changes are also the refresh's own to observe.
        store.watch.stop_background_for_test();
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
