//! Immutable publication capture: changed paths and names patch persistent roots.
use super::*;
use crate::model::{derived::Named, Claim, OpeningName};
use std::collections::HashSet;

impl Store {
    /// Use the latest published identity index for a move's same-name check.
    /// Before the first publication, live discovery still supplies the index;
    /// never wait for the load worker while holding the transaction writer lock.
    pub(crate) fn move_claimant(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        if let Some(snapshot) = self.changes.snapshot.read().unwrap().as_ref() {
            return OpeningName::new(&snapshot.journal_format, name, kind == PageKind::Journal)
                .claims(&snapshot.claimants)
                .iter()
                .find(|entry| entry.kind == kind)
                .cloned();
        }
        self.graph.find_entry(name, kind)
    }

    /// Whether `entry`, read by path, is the winning claimant of its name
    /// (GH #623 BR3). An ordinary page is answered from the published name
    /// index while that index describes the current cache generation, so no
    /// other file is opened; a file the index does not list under this name
    /// yet (new, or retitled before the watcher saw it) is ranked against
    /// the listed claimants by the one claimant order. A journal, or a stale
    /// or absent publication, asks the live claimant index. Erring towards
    /// "not canonical" is safe: the
    /// page is then parsed directly and its revision still comes from the
    /// bytes served.
    pub(crate) fn canonical_claim(&self, entry: &PageEntry) -> bool {
        if entry.kind == PageKind::Page {
            if let Some(snapshot) = self
                .changes
                .snapshot
                .read()
                .unwrap()
                .as_ref()
                .filter(|snapshot| snapshot.cache_generation == self.graph.cache_generation())
            {
                let listed = snapshot
                    .claimants
                    .get(&Claim::of(entry))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if listed.iter().any(|claimant| claimant.path == entry.path) {
                    return listed[0].path == entry.path;
                }
                let journals_dir = &snapshot.config.config.journals_dir;
                return listed.iter().all(|claimant| {
                    crate::model::compare_page_claimants(journals_dir, entry, claimant).is_lt()
                });
            }
        }
        self.graph
            .find_entry(&entry.name, entry.kind)
            .is_some_and(|found| found.path == entry.path)
    }
}

/// Test-only hook on snapshot publication: a pause before a snapshot is
/// published.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct PublishHooks {
    pub(crate) pause: Mutex<Option<super::TestPause>>,
}

#[cfg(test)]
impl PublishHooks {
    pub(super) fn before_publish(&self) {
        super::pause_at_hook(&self.pause);
    }
}

/// What one capture patches: the publication's files and the rows the cache
/// changed that no publication named. Held outside the capture because taking
/// those rows is not repeatable, and a capture that panics is retried.
#[derive(Default)]
pub(super) struct PendingFiles {
    files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    /// Paths whose rows a hold or respelling retired since the last capture
    /// (E105): each is renamed from what it names now.
    retired: HashSet<FileId>,
    /// Rows the cache changed that no publication named (a host save while
    /// the graph is Failed publishes nothing, and the recovery's diff names
    /// only what it found): every index patches them now.
    unnamed: bool,
}

impl PendingFiles {
    pub(super) fn of(files: &[(FileId, ChangeKind, Option<FileRev>)], graph: &Graph) -> Self {
        let mut pending = Self {
            files: files.to_vec(),
            ..Self::default()
        };
        pending.take_rows(graph);
        pending
    }

    pub(super) fn take_rows(&mut self, graph: &Graph) {
        for path in graph.take_retired() {
            let id = FileId::from(graph.rel_path(&path));
            if !self.files.iter().any(|(file, _, _)| *file == id) {
                self.files.push((id.clone(), ChangeKind::Modified, None));
                self.retired.insert(id);
            }
        }
        for path in graph.take_touched() {
            let id = FileId::from(graph.rel_path(&path));
            if !self.files.iter().any(|(file, _, _)| *file == id) {
                self.files.push((id.clone(), ChangeKind::Modified, None));
                self.retired.insert(id);
                self.unnamed = true;
            }
        }
    }

    /// One of these files alone, to learn which one a capture panics on.
    pub(super) fn only(&self, file: &(FileId, ChangeKind, Option<FileRev>)) -> Self {
        Self {
            files: vec![file.clone()],
            retired: self.retired.clone(),
            unnamed: self.unnamed,
        }
    }

    pub(super) fn files(&self) -> &[(FileId, ChangeKind, Option<FileRev>)] {
        &self.files
    }
}

/// Multi-row patch failure that no individual row explains. Root-scoped like
/// the full-index fault; solo probes pass, so containment cannot blame a page.
#[cfg(any(test, feature = "test-faults"))]
pub(super) static TEST_PATCH_PANIC_ROOTS: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());

impl Snapshot {
    /// The entry this generation holds for `path`: its name, its kind and
    /// the day it actually claims (CM SPEC §2.1: a `title::` may name a day
    /// other than its file name's, and a title that is no date names none), as the
    /// claimant index recorded it from the file's bytes. An incremental
    /// update takes the day a file held from here, never from its path or
    /// its displayed name. O(claimants of that name).
    fn held_entry(&self, id: &PageId) -> Option<&PageEntry> {
        let key = self.claim_by_path.get(id)?;
        self.claimants
            .get(key)?
            .iter()
            .find(|entry| entry.rel_path.as_ref() == Some(id))
    }

    /// The active pages changed by this publication, including opening
    /// siblings that no physical write touched. Physical evidence stays in
    /// Change::files for conflict refresh and mutation bookkeeping.
    fn page_changes(
        &self,
        old: Option<&Self>,
        origin: Origin,
        files: &[(FileId, ChangeKind, Option<FileRev>)],
    ) -> Vec<(FileId, ChangeKind, PageKind, String)> {
        if origin == Origin::Own {
            return Vec::new();
        }
        let kinds: HashMap<_, _> = files
            .iter()
            .map(|(id, kind, _)| (id.as_str(), *kind))
            .collect();
        let mut paths: Vec<_> = files
            .iter()
            .map(|(id, _, _)| id.as_str().to_owned())
            .collect();
        paths.extend(self.graph.semantic_changed_paths.iter().cloned());
        let mut seen = HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        let mut pages = Vec::new();
        fn get<'a>(
            snapshot: &'a Snapshot,
            path: &str,
        ) -> Option<&'a (PageEntry, Arc<tine_core::doc::Document>)> {
            snapshot
                .graph
                .pages
                .positions
                .get(path)
                .and_then(|&slot| snapshot.graph.pages.get(slot))
        }
        for path in paths {
            let before = old.and_then(|snapshot| get(snapshot, &path));
            let after = get(self, &path);
            if let Some((entry, doc)) = after {
                if before.is_some_and(|(previous, previous_doc)| {
                    previous.name == entry.name
                        && previous.kind == entry.kind
                        && (Arc::ptr_eq(previous_doc, doc) || previous_doc == doc)
                }) {
                    continue;
                }
                let change = kinds
                    .get(path.as_str())
                    .copied()
                    .unwrap_or(ChangeKind::Modified);
                pages.push((FileId::from(path), change, entry.kind, entry.name.clone()));
            } else if let Some((entry, _)) = before {
                // A demoted file or removed opening with a replacement is
                // represented by the new opening file's event, not deletion
                // of a logical page that still exists.
                if self
                    .graph
                    .real_page_owners()
                    .opening_entry(&tine_core::refs::page_key(&entry.name))
                    .is_none()
                {
                    pages.push((
                        FileId::from(path),
                        ChangeKind::Removed,
                        entry.kind,
                        entry.name.clone(),
                    ));
                }
            }
        }
        pages
    }

    pub(super) fn capture(
        graph: &Graph,
        config: &RwLock<ConfigState>,
        old: Option<&Snapshot>,
        pending: &PendingFiles,
        config_changed: bool,
        rebuild: bool,
        rev: GraphRev,
        published: &HashMap<FileId, PageEntry>,
        targets: &super::change_summary::RegisteredTargets,
    ) -> Self {
        // The publication caller holds the store writer lock. A load worker
        // publishes only after its initial parse has finished.
        graph.with_pages(|_| ());
        let config = config.read().unwrap().clone();
        let journal_format = graph.current_journal_format();
        let PendingFiles {
            files,
            retired,
            unnamed,
        } = pending;
        let unnamed = *unnamed;
        let changed_names: Vec<_> = files
            .iter()
            .filter_map(|(id, kind, _)| {
                let path = graph.root.join(id.as_str());
                if !crate::model::graph_text_eligible(&graph.root, &path, &graph.current_config())
                    || !matches!(
                        kind,
                        ChangeKind::Created | ChangeKind::Modified | ChangeKind::Removed
                    )
                {
                    return None;
                }
                let page = PageId::from(id.as_str());
                let old_entry = old.and_then(|snapshot| snapshot.held_entry(&page));
                // A removal is the entry the previous generation held (R2):
                // no file is read for it, and it keeps the day it held.
                let removed = || {
                    let held = old_entry?;
                    Some((ChangeKind::Removed, held.clone()))
                };
                if *kind == ChangeKind::Removed {
                    return removed();
                }
                // A transaction's own publication named the page from the
                // bytes it cached, under the same writer lock and config; an
                // external write since then is the watcher's next change.
                let entry = match published.get(id) {
                    Some(entry) if !config_changed => entry.clone(),
                    _ => match graph.named_entry(&path) {
                        Named::Entry(entry) => entry,
                        Named::Absent => return removed(),
                        Named::NotAPage => return None,
                    },
                };
                let kind = match retired.contains(id) {
                    true if old_entry.is_none() => ChangeKind::Created,
                    _ => *kind,
                };
                // A journals-directory file is named by its file name alone; a
                // day elsewhere is named by its bytes (CM SPEC §2.1).
                if kind == ChangeKind::Modified && path.starts_with(graph.journals_path()) {
                    return None;
                }
                // A journal title names one day (D-20261119-YL), so kind and
                // name settle whether the claim changed.
                if kind == ChangeKind::Modified
                    && old_entry.is_some_and(|old| {
                        old.kind == entry.kind
                            && old.name == entry.name
                            && old.date_key == entry.date_key
                    })
                {
                    return None;
                }
                Some((kind, entry))
            })
            .collect();
        // `rebuild` (the Settings "Rescan graph") recomputes every derived answer
        // from the cache instead of carrying the previous generation's.
        let name_set_changed =
            config_changed || rebuild || old.is_none() || !changed_names.is_empty();
        // The files whose name came, left or moved, for the precise summary.
        let named_paths: Vec<PathBuf> = changed_names
            .iter()
            .map(|(_, entry)| entry.path.clone())
            .collect();
        let mut claim_by_path = old
            .map(|old| Arc::clone(&old.claim_by_path))
            .unwrap_or_default();
        let (list, claimants) = if config_changed || rebuild || old.is_none() {
            let (list, claimants) = graph.snapshot_name_roots();
            claim_by_path = Arc::new(
                claimants
                    .iter()
                    .flat_map(|(claim, entries)| {
                        entries
                            .iter()
                            .filter_map(|entry| Some((entry.rel_path.clone()?, claim.clone())))
                    })
                    .collect(),
            );
            (list, claimants)
        } else if !changed_names.is_empty() {
            let previous = old.expect("name index from old generation");
            let mut list = Arc::clone(&previous.list);
            let mut claimants = Arc::clone(&previous.claimants);
            let journals_dir = graph.current_config().journals_dir.clone();
            for (kind, entry) in changed_names {
                let Some(id) = &entry.rel_path else { continue };
                let path = entry.path.clone();
                let buckets = Arc::make_mut(&mut claimants);
                let old_key = claim_by_path.get(id).cloned();
                if let Some(old_key) = &old_key {
                    Arc::make_mut(&mut claim_by_path).remove(id);
                    if let Some(bucket) = buckets.get_mut(old_key) {
                        bucket.retain(|candidate| candidate.path != path);
                        if bucket.is_empty() {
                            buckets.remove(old_key);
                        }
                    }
                }
                let new_key = (kind != ChangeKind::Removed).then(|| Claim::of(&entry));
                if let Some(key) = &new_key {
                    Arc::make_mut(&mut claim_by_path).insert(id.clone(), key.clone());
                    if !buckets.contains_key(key) {
                        buckets.insert(key.clone(), Vec::new());
                    }
                    let bucket = buckets.get_mut(key).unwrap();
                    bucket.push(entry.clone());
                    bucket
                        .sort_by(|a, b| crate::model::compare_page_claimants(&journals_dir, a, b));
                }
                let list = Arc::make_mut(&mut list);
                list.remove_path(&path);
                for key in old_key
                    .as_ref()
                    .filter(|key| Some(*key) != new_key.as_ref())
                    .into_iter()
                    .chain(new_key.as_ref())
                {
                    if let Claim::Day(day) = key {
                        list.remove_day(*day);
                        if let Some(winner) = buckets.get(key).and_then(|bucket| bucket.first()) {
                            list.push(winner.clone());
                        }
                    }
                }
                if matches!(new_key, Some(Claim::Name(_))) {
                    list.push(entry);
                }
            }
            (list, claimants)
        } else {
            let old = old.expect("name index from old generation");
            (Arc::clone(&old.list), Arc::clone(&old.claimants))
        };
        // A file arrival can displace an indexed journal; a removal can promote
        // a previously set-aside file. Patch only the affected day buckets,
        // reusing the same complete claimant index that resolve() exposes.
        let journal_keys: HashSet<_> = if config_changed || rebuild || old.is_none() {
            claimants
                .iter()
                .filter(|(key, entries)| matches!(key, Claim::Day(_)) && entries.len() > 1)
                .map(|(key, _)| key.clone())
                .collect()
        } else {
            // The day a file holds now and the one it held: a retitle off a
            // day promotes that day's next claimant.
            files
                .iter()
                .flat_map(|(id, _, _)| {
                    let page = PageId::from(id.as_str());
                    [
                        claim_by_path.get(&page),
                        old.and_then(|old| old.claim_by_path.get(&page)),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|key| matches!(key, Claim::Day(_)))
                    .cloned()
                    .collect::<Vec<_>>()
                })
                .collect()
        };
        let journal_paths: Vec<_> = journal_keys
            .iter()
            .filter_map(|key| claimants.get(key))
            .flat_map(|entries| graph.reconcile_journal_claimants(entries))
            .collect();
        let cache_generation = graph.cache_generation();
        let mut changed_paths: Vec<String> = files
            .iter()
            .filter(|(id, _, _)| {
                let root = &graph.root;
                crate::model::graph_text_eligible(
                    root,
                    &root.join(id.as_str()),
                    &graph.current_config(),
                )
            })
            .map(|(id, _, _)| id.as_str().to_owned())
            .collect();
        changed_paths.extend(journal_paths.iter().map(|path| graph.rel_path(path)));
        changed_paths.sort();
        changed_paths.dedup();
        let mut folds_only = false;
        // Share one old/new document lookup between memo carry and the wire
        // summary. Do not retain these previous documents in the new snapshot.
        // Anything but "nothing text-eligible changed" or a computed edit list
        // stays None, which the summary publishes as `full` (ADR 0014: over-
        // invalidate, never stale).
        let mut edited_pages =
            (changed_paths.is_empty() && !config_changed && !rebuild).then(Vec::new);
        // A publication that moved a page name (create, delete, rename) and
        // nothing graph-wide: each changed path's page on both sides, for a
        // precise summary instead of `full` (D-20261102-PERF row 3).
        let mut page_changes = None;
        let evaluator = if let Some(old) = old.filter(|old| {
            old.cache_generation == cache_generation && !config_changed && !rebuild && !unnamed
        }) {
            Arc::clone(&old.graph)
        } else {
            let evaluator = ReadSnapshot::capture(
                graph,
                (*config.config).clone(),
                Arc::clone(&list),
                old.filter(|_| !config_changed && !rebuild)
                    .map(|old| old.graph.as_ref()),
                &changed_paths,
            );
            if !name_set_changed {
                if let Some(old) = old {
                    let edits = evaluator.edited_pages(&old.graph, &changed_paths);
                    let folds = edits
                        .as_deref()
                        .is_some_and(|edits| evaluator.carry_from(&old.graph, edits));
                    if !config_changed && !rebuild {
                        edited_pages = edits;
                    }
                    folds_only = folds
                        && files
                            .iter()
                            .all(|(_, kind, _)| *kind == ChangeKind::Modified);
                }
            } else if let Some(old) = old.filter(|_| !config_changed && !rebuild && !unnamed) {
                // The semantic paths add a twin the change promoted or demoted.
                let mut paths = changed_paths.clone();
                paths.extend(evaluator.semantic_changed_paths.iter().cloned());
                paths.sort();
                paths.dedup();
                // A batch over the bound publishes `full` (the summary's same
                // rule): decided first, so it reads no page (review F2).
                if paths.len() <= super::change_summary::PRECISE_PATHS {
                    let changes = evaluator.page_changes(&old.graph, &paths);
                    // A named file that is the active page on neither side (a
                    // losing twin) has no document to name its readers by:
                    // `full`. One set, so the check is linear in the batch.
                    let active: HashSet<&std::path::Path> = changes
                        .iter()
                        .flat_map(|(before, after)| [before, after].into_iter().flatten())
                        .map(|(entry, _)| entry.path.as_path())
                        .collect();
                    page_changes = named_paths
                        .iter()
                        .all(|path| {
                            #[cfg(feature = "test-faults")]
                            crate::cost_counters::lifecycle_probe();
                            active.contains(path.as_path())
                        })
                        .then_some(changes);
                }
            }
            Arc::new(evaluator)
        };
        changed_paths.extend(evaluator.semantic_changed_paths.iter().cloned());
        changed_paths.sort();
        changed_paths.dedup();
        let mut snapshot = Self {
            graph: evaluator,
            rev,
            cache_generation,
            config,
            journal_format,
            list,
            claimants,
            claim_by_path,
            unreadable: graph.reported_unreadable_pages(),
            answers: Default::default(),
            folds_only,
            unparsed: std::sync::OnceLock::new(),
        };
        // A config change or rescan renames the whole table: rebuilt on demand.
        snapshot.carry_unparsed(
            graph,
            old.filter(|_| !config_changed && !rebuild),
            &changed_paths,
        );
        #[cfg(any(test, feature = "test-faults"))]
        if old.is_some()
            && !rebuild
            && files.len() > 1
            && TEST_PATCH_PANIC_ROOTS.lock().unwrap().contains(&graph.root)
        {
            panic!("test: patch capture failed with no individual suspect");
        }
        let (inventory, delta, counts, moved) =
            snapshot.answer_changes(old, &changed_paths, name_set_changed);
        let summary = snapshot.change_summary(
            old,
            &changed_paths,
            name_set_changed,
            inventory,
            edited_pages.as_deref(),
            page_changes.as_deref(),
            &targets.unlinked,
            &targets.queries,
        );
        let metrics = snapshot.metric_changes(old, &changed_paths, &moved);
        snapshot.answers = (inventory, counts, summary, delta, metrics);
        snapshot
    }
}

impl Store {
    /// Another chance for pages a panic retired (`derived_contained.rs`): a
    /// focus or a rescan comes here, the watcher's own ticks do not. Nothing
    /// to do when no retired page has a retry left.
    pub(super) fn retry_contained_rows(&self) {
        if !self.graph.contained_retry_due() {
            return;
        }
        let _writer = self.writer.lock().unwrap();
        let (mut files, mut pages) = (Vec::new(), Vec::new());
        for (path, entry, rev) in self.graph.retry_contained_rows() {
            let id = FileId::from(self.graph.rel_path(&path));
            if let Some(entry) = entry {
                pages.push((id.clone(), entry.kind, entry.name));
            }
            files.push((id, ChangeKind::Modified, Some(rev)));
        }
        if !files.is_empty() {
            self.changes.publish(Origin::External, files, false, pages);
        }
    }
}

impl ChangeFeed {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_inner(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        config_changed: bool,
        _pages: Vec<(FileId, PageKind, String)>,
        before_notify: impl FnOnce(),
        watch: Option<WatchBatch>,
        rebuild: bool,
        published: &HashMap<FileId, PageEntry>,
    ) -> GraphRev {
        let old = self.snapshot.read().unwrap().clone();
        // Only the settings the page build and indexes read
        // (`checkpoint::config_key`) rebuild names and indexes; the others
        // install with the config state and publish no index change (the
        // config contract). Compared with the previous publication's config,
        // so a reload since then that moved the key is never missed.
        let config_changed = config_changed
            && old.as_ref().is_none_or(|old| {
                super::checkpoint::config_key(&old.config.config)
                    != super::checkpoint::config_key(&self.config.read().unwrap().config)
            });
        let rev = GraphRev(self.rev().0 + 1);
        let targets = self.registered_targets();
        let capture = |pending: &PendingFiles, config_changed: bool| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Snapshot::capture(
                    &self.graph,
                    &self.config,
                    old.as_deref(),
                    pending,
                    config_changed,
                    rebuild,
                    rev,
                    published,
                    &targets,
                )
            }))
            .ok()
        };
        let mut pending = PendingFiles::of(&files, &self.graph);
        let captured = capture(&pending, config_changed).or_else(|| {
            // Parses are isolated per page before this, so this is a Rust
            // index stage. Patch each page alone to learn which one it is. A
            // file that is no page would build every index, so it is no suspect.
            let mut suspects = Vec::new();
            if old.is_some() && !rebuild {
                suspects = (pending.files().iter())
                    .map(|file| (file, self.graph.root.join(file.0.as_str())))
                    .filter(|(_, path)| {
                        crate::model::graph_text_eligible(
                            &self.graph.root,
                            path,
                            &self.graph.current_config(),
                        )
                    })
                    .filter(|(file, _)| capture(&pending.only(file), false).is_none())
                    .map(|(_, path)| path)
                    .collect();
            }
            // A capture that builds every index names no page: find its rows.
            let probed = suspects.is_empty() && (old.is_none() || rebuild || config_changed);
            if probed {
                suspects = self.graph.rows_that_panic_an_index_build();
            }
            if suspects.is_empty() {
                return None;
            }
            for path in &suspects {
                self.graph.retire_index_stage_failure(path);
            }
            pending.take_rows(&self.graph);
            let without = capture(&pending, config_changed);
            // Rows that panic built alone but do not explain this build's panic
            // are not shown to be bad: the graph fails with no page blamed.
            if without.is_none() && probed {
                suspects.iter().for_each(|path| self.graph.unblame(path));
            }
            without
        });
        let Some(mut snapshot) = captured else {
            // Under the writer, so a panic that reached it would poison every
            // later save: the graph fails instead, `refresh()` retries, and
            // the retry patches these files.
            self.graph.touch_rows(
                (pending.files().iter()).map(|(id, _, _)| self.graph.root.join(id.as_str())),
            );
            self.load.fail_publication(&self.graph);
            return self.rev();
        };
        self.graph.adopt_name_index(
            old.as_ref().map(|old| old.cache_generation),
            snapshot.cache_generation,
            (&snapshot.list, &snapshot.claimants),
        );
        // A fold leaves the launch checkpoint's derived state true; its file
        // stamp is stale, so the next launch diff rereads that one file
        // (ADR 0070) instead of the whole checkpoint being rewritten for it.
        if (old.is_none() || config_changed || rebuild || !files.is_empty()) && !snapshot.folds_only
        {
            self.checkpoint.get().inspect(|signal| signal.published());
        }
        #[cfg(test)]
        self.publish_hooks.before_publish();
        let pages = snapshot.page_changes(old.as_deref(), origin, &files);
        let mut state = self.state.lock().unwrap();
        state.rev += 1;
        let rev = GraphRev(state.rev);
        debug_assert_eq!(snapshot.rev, rev);
        // Holding the snapshot write lock orders a later registration's first
        // read after this install; one made during the capture fires here.
        let mut current = self.snapshot.write().unwrap();
        self.fire_registered_since(&targets, &mut snapshot);
        let snapshot = Arc::new(snapshot);
        let answers = snapshot.answers.clone();
        *current = Some(snapshot);
        drop(current);
        before_notify();
        if !state.closed {
            state.queue.push_back(Change {
                graph_rev: rev,
                origin,
                files,
                pages,
                answers,
                watch,
            });
            self.ready.notify_all();
        }
        rev
    }
}

/// What final publication already observed of the files it published, so
/// the own-write stamp and the name index need not reopen them (GH #623).
#[derive(Default)]
pub(crate) struct PublishedObservations {
    /// Per file: metadata stamped before the publication read, with the
    /// revision of the bytes that read returned.
    pub(crate) stamps: HashMap<FileId, crate::watch::Stamp>,
    /// Per page file: the cacheable entry named from those same bytes.
    pub(crate) entries: HashMap<FileId, PageEntry>,
}

impl Store {
    /// Publish what ownership transitions settled that no publication has
    /// captured yet (REVIEW-3a4 #2): the current snapshot stops showing a
    /// pre-transition row, also when the owner's first publication fails.
    /// The caller holds the writer, under which the transitions ran.
    pub(crate) fn publish_retired(&self) {
        // After a failed load nothing publishes until a successful refresh;
        // that refresh's first view takes the rows.
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_)) {
            return;
        }
        if self.graph.has_retired() && self.defers_publication(false) {
            self.defer_retired();
        } else if self.graph.has_retired() {
            self.changes
                .publish(Origin::External, Vec::new(), false, Vec::new());
        }
    }

    /// Whether the published snapshot names a page at `path`.
    pub(crate) fn snapshot_knows(&self, path: &Path) -> bool {
        let snapshot = self.changes.snapshot.read().unwrap();
        snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .claim_by_path
                .contains_key(&PageId::from(self.graph.rel_path(path)))
        })
    }

    pub(crate) fn publish_own(
        &self,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        observations: PublishedObservations,
    ) -> (GraphRev, Option<Change>) {
        self.publish_transaction_change(Origin::Own, files, Vec::new(), observations)
    }

    pub(crate) fn publish_transaction_change(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        pages: Vec<(FileId, PageKind, String)>,
        observations: PublishedObservations,
    ) -> (GraphRev, Option<Change>) {
        let observed: Vec<_> = files
            .iter()
            .map(|(id, _, rev)| (id.clone(), rev.clone()))
            .collect();
        let raced = self
            .watch
            .note_own_observed(&observed, &observations.stamps);
        let config_changed = observed
            .iter()
            .any(|(id, _)| id.as_str() == "logseq/config.edn");
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_)) {
            self.watch.reconcile_raced(&raced);
            return (self.changes.rev(), None);
        }
        // Before the first complete view, the publication waits for it
        // (`first_view.rs`): the current revision, no change yet.
        if self.defers_publication(config_changed) {
            self.changes.defer_publication(origin, files, pages, raced);
            return (self.changes.rev(), None);
        }
        // A publication that does not wait while changes still do (a config
        // write during loading publishes the first view early): the waiting
        // changes go first, so own changes keep save order (review R1).
        if self.changes.has_deferred_changes() {
            self.changes
                .publish_deferred_early(&self.watch.core_for_load());
        }
        let previous = (origin == Origin::External)
            .then(|| self.changes.snapshot.read().unwrap().clone())
            .flatten();
        let rev = self.changes.publish_observed(
            origin,
            files.clone(),
            config_changed,
            pages.clone(),
            &observations.entries,
        );
        // No snapshot: a publication during the load whose capture panicked.
        // The write landed; only its answers are unknown.
        let (answers, pages) = (self.changes.snapshot.read().unwrap().as_ref())
            .map(|snapshot| {
                (
                    snapshot.answers.clone(),
                    snapshot.page_changes(previous.as_deref(), origin, &files),
                )
            })
            .unwrap_or_default();
        let change = Change {
            graph_rev: rev,
            origin,
            files,
            pages,
            answers,
            watch: None,
        };
        self.watch.reconcile_raced(&raced);
        (rev, Some(change))
    }
}
