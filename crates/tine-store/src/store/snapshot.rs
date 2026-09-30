//! Immutable publication capture: changed paths and names patch persistent roots.
use super::*;

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
}

impl WholeGraph {
    pub(super) fn name_claimants(&self, name: &str, kind: PageKind) -> &[PageEntry] {
        name_claimants(&self.claimants, name, kind)
    }
}

impl Snapshot {
    pub(super) fn capture(
        graph: &Graph,
        config: &RwLock<ConfigState>,
        old: Option<&Snapshot>,
        files: &[(FileId, ChangeKind, Option<FileRev>)],
        config_changed: bool,
        rev: GraphRev,
    ) -> Self {
        // The publication caller holds the store writer lock. A load worker
        // publishes only after its initial parse has finished.
        graph.with_pages(|_| ());
        let config = config.read().unwrap().clone();
        let journal_format = graph.current_journal_format();
        let cache_generation = graph.cache_generation();
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
                let entry = graph.entry_for_path(&path)?;
                if *kind == ChangeKind::Modified && entry.kind == PageKind::Journal {
                    return None;
                }
                let old_name = old.and_then(|snapshot| snapshot.name_by_path.get(&path));
                if *kind == ChangeKind::Modified
                    && old_name
                        .is_some_and(|(kind, name)| *kind == entry.kind && *name == entry.name)
                {
                    return None;
                }
                Some((*kind, entry))
            })
            .collect();
        let name_set_changed = config_changed || old.is_none() || !changed_names.is_empty();
        let mut name_by_path = old
            .map(|old| Arc::clone(&old.name_by_path))
            .unwrap_or_default();
        let (list, claimants) = if config_changed || old.is_none() {
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
        let evaluator = if let Some(old) =
            old.filter(|old| old.cache_generation == cache_generation && !config_changed)
        {
            Arc::clone(&old.graph)
        } else {
            let evaluator = ReadSnapshot::capture(
                graph,
                (*config.config).clone(),
                Arc::clone(&list),
                old.filter(|_| !config_changed)
                    .map(|old| old.graph.as_ref()),
                &changed_paths,
            );
            if !name_set_changed {
                if let Some(old) = old {
                    evaluator.carry_memos_from(&old.graph, &changed_paths);
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
            unreadable: graph.unreadable_pages(),
            answers: Default::default(),
        };
        snapshot.answers = snapshot.answer_changes(old, &changed_paths, name_set_changed);
        snapshot
    }
}

impl Store {
    pub(crate) fn publish_own(
        &self,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    ) -> (GraphRev, Option<Change>) {
        self.publish_transaction_change(Origin::Own, files, Vec::new())
    }

    pub(crate) fn publish_transaction_change(
        &self,
        origin: Origin,
        files: Vec<(FileId, ChangeKind, Option<FileRev>)>,
        pages: Vec<(FileId, PageKind, String)>,
    ) -> (GraphRev, Option<Change>) {
        let observed: Vec<_> = files
            .iter()
            .map(|(id, _, rev)| (id.clone(), rev.clone()))
            .collect();
        let raced = self.watch.note_own(&observed);
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
        let rev = self
            .changes
            .publish(origin, files.clone(), config_changed, pages.clone());
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
