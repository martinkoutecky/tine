//! Immutable publication capture: changed paths and names patch persistent roots.
use super::*;
use crate::model::derived::Named;
use std::collections::HashSet;

fn name_claimants<'a>(
    index: &'a SharedMap<(bool, String), Vec<PageEntry>>,
    name: &str,
    kind: PageKind,
) -> &'a [PageEntry] {
    index
        .get(&(kind == PageKind::Journal, tine_core::refs::page_key(name)))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

impl Store {
    /// Use the latest published identity index for a move's same-name check.
    /// Before the first publication, live discovery still supplies the index;
    /// never wait for the load worker while holding the transaction writer lock.
    pub(crate) fn move_claimant(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        if let Some(snapshot) = self.changes.snapshot.read().unwrap().as_ref() {
            return name_claimants(&snapshot.claimants, name, kind)
                .first()
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
    /// or absent publication, asks the live index (a journal walk lists
    /// journal file names only). Erring towards "not canonical" is safe: the
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
                let listed = name_claimants(&snapshot.claimants, &entry.name, entry.kind);
                if listed.iter().any(|claimant| claimant.path == entry.path) {
                    return listed[0].path == entry.path;
                }
                let format = self.graph.current_journal_format();
                let name_format = self.graph.current_config().file_name_format;
                return listed.iter().all(|claimant| {
                    crate::model::compare_page_claimants(entry, claimant, &format, name_format)
                        .is_lt()
                });
            }
        }
        self.graph
            .find_entry(&entry.name, entry.kind)
            .is_some_and(|found| found.path == entry.path)
    }
}

impl WholeGraph {
    pub(super) fn name_claimants(&self, name: &str, kind: PageKind) -> &[PageEntry] {
        name_claimants(&self.claimants, name, kind)
    }
}

/// Test-only hooks on snapshot publication: a pause before a snapshot is
/// published, and every published snapshot in order (tests compare each
/// generation).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct PublishHooks {
    pub(crate) pause: Mutex<Option<super::TestPause>>,
    log: Mutex<Vec<Arc<Snapshot>>>,
}

#[cfg(test)]
impl PublishHooks {
    pub(super) fn before_publish(&self) {
        super::pause_at_hook(&self.pause);
    }

    pub(super) fn published(&self, snapshot: &Arc<Snapshot>) {
        self.log.lock().unwrap().push(Arc::clone(snapshot));
    }
}

#[cfg(test)]
impl ChangeFeed {
    /// How many snapshots this feed has published.
    pub(crate) fn published_count(&self) -> usize {
        self.publish_hooks.log.lock().unwrap().len()
    }

    /// For each snapshot published from index `from` on: the cache
    /// generation it was captured from, its image, and the image of a
    /// parent-free capture of the cache as it is now (comparable only at the
    /// current generation).
    pub(crate) fn published_images(
        &self,
        from: usize,
        image: &dyn Fn(&crate::model::ReadSnapshot) -> String,
    ) -> Vec<(u64, String, String)> {
        let log: Vec<_> = self.publish_hooks.log.lock().unwrap()[from..].to_vec();
        log.iter()
            .map(|snapshot| {
                let fresh = crate::model::ReadSnapshot::capture(
                    &self.graph,
                    (*snapshot.config.config).clone(),
                    Arc::clone(&snapshot.list),
                    None,
                    &[],
                );
                (
                    snapshot.cache_generation,
                    image(&snapshot.graph),
                    image(&fresh),
                )
            })
            .collect()
    }
}

impl Snapshot {
    pub(super) fn capture(
        graph: &Graph,
        config: &RwLock<ConfigState>,
        old: Option<&Snapshot>,
        files: &[(FileId, ChangeKind, Option<FileRev>)],
        config_changed: bool,
        rebuild: bool,
        rev: GraphRev,
        published: &HashMap<FileId, PageEntry>,
        unlinked_targets: &[String],
    ) -> Self {
        // The publication caller holds the store writer lock. A load worker
        // publishes only after its initial parse has finished.
        graph.with_pages(|_| ());
        let config = config.read().unwrap().clone();
        let journal_format = graph.current_journal_format();
        let cache_generation = graph.cache_generation();
        // Paths whose rows a hold or respelling retired since the last
        // capture (E105): each is renamed from what it names now.
        let mut files = files.to_vec();
        let mut retired = HashSet::new();
        for path in graph.take_retired() {
            let id = FileId::from(graph.rel_path(&path));
            if !files.iter().any(|(file, _, _)| *file == id) {
                files.push((id.clone(), ChangeKind::Modified, None));
                retired.insert(id);
            }
        }
        // Rows the cache changed that no publication named (a failed
        // transaction publishes its own files and an outside writer's
        // separately, after caching both): every index patches them now.
        let mut unnamed = false;
        for path in graph.take_touched() {
            let id = FileId::from(graph.rel_path(&path));
            if !files.iter().any(|(file, _, _)| *file == id) {
                files.push((id.clone(), ChangeKind::Modified, None));
                retired.insert(id);
                unnamed = true;
            }
        }
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
                let old_name = old.and_then(|snapshot| snapshot.name_by_path.get(&path));
                // A removal is named by the path and its previous name alone
                // (R2): no file is read for it.
                let removed = || Some((ChangeKind::Removed, graph.tombstone(&path, old_name?)));
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
                    true if old_name.is_none() => ChangeKind::Created,
                    _ => *kind,
                };
                if kind == ChangeKind::Modified && entry.kind == PageKind::Journal {
                    return None;
                }
                if kind == ChangeKind::Modified
                    && old_name
                        .is_some_and(|(kind, name)| *kind == entry.kind && *name == entry.name)
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
        let mut name_by_path = old
            .map(|old| Arc::clone(&old.name_by_path))
            .unwrap_or_default();
        let (list, claimants) = if config_changed || rebuild || old.is_none() {
            let (list, claimants) = graph.snapshot_name_index();
            name_by_path = Arc::new(
                claimants
                    .values()
                    .flatten()
                    .map(|entry| (entry.path.clone(), (entry.kind, entry.name.clone())))
                    .collect(),
            );
            (
                Arc::new(EntryList::from(list.as_slice())),
                Arc::new(
                    claimants
                        .into_iter()
                        .map(|((kind, name), rows)| ((kind == PageKind::Journal, name), rows))
                        .collect(),
                ),
            )
        } else if !changed_names.is_empty() {
            let previous = old.expect("name index from old generation");
            let mut list = Arc::clone(&previous.list);
            let mut claimants = Arc::clone(&previous.claimants);
            for (kind, entry) in changed_names {
                let path = entry.path.clone();
                let buckets = Arc::make_mut(&mut claimants);
                if let Some((old_kind, old_name)) = previous.name_by_path.get(&path) {
                    let old_key = (
                        *old_kind == PageKind::Journal,
                        tine_core::refs::page_key(old_name),
                    );
                    if let Some(bucket) = buckets.get_mut(&old_key) {
                        bucket.retain(|candidate| candidate.path != path);
                        if bucket.is_empty() {
                            buckets.remove(&old_key);
                        }
                    }
                }
                Arc::make_mut(&mut name_by_path).remove(&path);
                if kind != ChangeKind::Removed {
                    Arc::make_mut(&mut name_by_path)
                        .insert(path.clone(), (entry.kind, entry.name.clone()));
                    let key = (
                        entry.kind == PageKind::Journal,
                        tine_core::refs::page_key(&entry.name),
                    );
                    if !buckets.contains_key(&key) {
                        buckets.insert(key.clone(), Vec::new());
                    }
                    let bucket = buckets.get_mut(&key).unwrap();
                    bucket.push(entry.clone());
                    bucket.sort_by(|a, b| {
                        crate::model::compare_page_claimants(
                            a,
                            b,
                            &journal_format,
                            config.config.file_name_format,
                        )
                    });
                }
                let list = Arc::make_mut(&mut list);
                list.remove_path(&path);
                if entry.kind == PageKind::Journal && entry.date_key.is_some() {
                    list.remove_day(entry.date_key.unwrap());
                    if let Some(winner) = buckets
                        .get(&(
                            entry.kind == PageKind::Journal,
                            tine_core::refs::page_key(&entry.name),
                        ))
                        .and_then(|bucket| bucket.first())
                    {
                        list.push(winner.clone());
                    }
                } else if kind != ChangeKind::Removed {
                    list.push(entry);
                }
            }
            (list, claimants)
        } else {
            let old = old.expect("name index from old generation");
            (Arc::clone(&old.list), Arc::clone(&old.claimants))
        };
        let changed_paths: Vec<String> = files
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
        let mut folds_only = false;
        // Share one old/new document lookup between memo carry and the wire
        // summary. Do not retain these previous documents in the new snapshot.
        // Anything but "nothing text-eligible changed" or a computed edit list
        // stays None, which the summary publishes as `full` (ADR 0014: over-
        // invalidate, never stale).
        let mut edited_pages =
            (changed_paths.is_empty() && !config_changed && !rebuild).then(Vec::new);
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
            }
            Arc::new(evaluator)
        };
        let mut snapshot = Self {
            graph: evaluator,
            rev,
            cache_generation,
            config,
            journal_format,
            list,
            claimants,
            name_by_path,
            unreadable: graph.reported_unreadable_pages(),
            answers: Default::default(),
            folds_only,
        };
        let (inventory, counts) = snapshot.answer_changes(old, &changed_paths, name_set_changed);
        let summary = snapshot.change_summary(
            old,
            &changed_paths,
            name_set_changed,
            inventory,
            edited_pages.as_deref(),
            unlinked_targets,
        );
        snapshot.answers = (inventory, counts, summary);
        snapshot
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
        (snapshot.as_ref()).is_some_and(|snapshot| snapshot.name_by_path.contains_key(path))
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
        let journal_set_changed = config_changed
            || files.iter().any(|(id, kind, _)| {
                id.as_str()
                    .starts_with(&format!("{}/", self.config().journals_dir))
                    && matches!(kind, ChangeKind::Created | ChangeKind::Removed)
            });
        if matches!(*self.load.status.lock().unwrap(), LoadStatus::Failed(_)) {
            if journal_set_changed {
                self.refresh_journal_ids();
            }
            return (self.changes.rev(), None);
        }
        // Before the first complete view, the publication waits for it
        // (`first_view.rs`): the current revision, no change yet.
        if self.defers_publication(config_changed) {
            self.changes
                .defer_publication(origin, files, pages, (raced, journal_set_changed));
            return (self.changes.rev(), None);
        }
        // A publication that does not wait while changes still do (a config
        // write during loading publishes the first view early): the waiting
        // changes go first, so own changes keep save order (review R1).
        if self.changes.has_deferred_changes() {
            self.changes
                .publish_deferred_early(&self.watch.core_for_load());
        }
        let rev = self.changes.publish_observed(
            origin,
            files.clone(),
            config_changed,
            pages.clone(),
            &observations.entries,
        );
        let answers = self
            .changes
            .snapshot
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .answers
            .clone();
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
