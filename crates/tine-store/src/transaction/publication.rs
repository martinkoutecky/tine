//! Final disk observation and cache publication after apply/undo.
//! Plan and undo records are indexed once; each file inspects only its records.
use super::*;
use std::collections::HashMap;

pub(super) struct FilePublication {
    pub(super) changed_any: bool,
    pub(super) spelling_moves: HashSet<FileId>,
    pub(super) publication_errors: Vec<(FileId, IoError)>,
    pub(super) published_own: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    pub(super) published_external: Vec<(FileId, ChangeKind, Option<FileRev>)>,
    /// Own-publication observations handed to the stamp and name index.
    pub(super) observations: crate::store::PublishedObservations,
}

impl Transaction<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_final_files(
        &self,
        plans: &[Prepared],
        done: &[Undo],
        before: &BTreeMap<String, Option<Vec<u8>>>,
        failed: bool,
        rollback: &mut Rollback,
        kept_old: &[FileId],
        steps: &[Step],
    ) -> FilePublication {
        let spelling_moves: HashSet<FileId> = plans
            .iter()
            .filter(|plan| Self::spelling_move(plan))
            .flat_map(|plan| [plan.src.clone(), plan.dst.as_ref().unwrap().clone()])
            .collect();
        // Index each record once, preserving first-plan and ordered undo matching.
        let mut source_plans = HashMap::new();
        let mut opaque_plans = HashMap::new();
        for (index, plan) in plans.iter().enumerate() {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::transaction_record_probe();
            source_plans.entry(&plan.src).or_insert(index);
            if plan.opaque_rev.is_some() {
                opaque_plans.entry(&plan.src).or_insert(plan);
                if let Some(dst) = &plan.dst {
                    opaque_plans.entry(dst).or_insert(plan);
                }
            }
        }
        let mut file_undos: HashMap<&FileId, Vec<&Undo>> = HashMap::new();
        for record in done {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::transaction_record_probe();
            file_undos.entry(&record.src).or_default().push(record);
            if let Some(dst) = &record.dst {
                if dst != &record.src {
                    file_undos.entry(dst).or_default().push(record);
                }
            }
        }
        let mut changed_any = false;
        let mut publication_errors = Vec::new();
        let mut published_own = Vec::new();
        let mut published_external = Vec::new();
        let mut observations = crate::store::PublishedObservations::default();
        for (name, baseline) in before {
            let id = FileId::from(name.clone());
            let spelling_move = spelling_moves.contains(&id);
            let mut path = match if spelling_move {
                self.spelled_path(&id)
            } else {
                self.path(&id)
            } {
                Ok(path) => path,
                Err(error) => {
                    publication_errors.push((id.clone(), publication_path_error(&error)));
                    self.store.graph.invalidate_cache();
                    continue;
                }
            };
            let missing_spelling = if spelling_move {
                match self.listed_path(&path) {
                    Ok(Some(actual)) => {
                        path = actual;
                        false
                    }
                    Ok(None) => true,
                    Err(error) => {
                        publication_errors.push((id.clone(), error.into()));
                        self.store.graph.invalidate_cache();
                        continue;
                    }
                }
            } else {
                false
            };
            if let Some(plan) = opaque_plans.get(&id) {
                let before_rev = (plan.src == id).then(|| plan.opaque_rev.clone()).flatten();
                let now_rev = match if missing_spelling {
                    Err(io::Error::from(io::ErrorKind::NotFound))
                } else {
                    FileRev::from_file(&path)
                } {
                    Ok(rev) => Some(rev),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => {
                        publication_errors.push((id.clone(), error.into()));
                        self.store.graph.invalidate_cache();
                        continue;
                    }
                };
                if before_rev != now_rev {
                    let kind = match (&before_rev, &now_rev) {
                        (None, Some(_)) => ChangeKind::Created,
                        (Some(_), None) => ChangeKind::Removed,
                        _ => ChangeKind::Modified,
                    };
                    let own = file_undos.get(&id).into_iter().flatten().any(|record| {
                        record.opaque_rev.is_some()
                            && ((record.src == id
                                && now_rev.is_none()
                                && (record.moved || record.created))
                                || (record.dst.as_ref() == Some(&id)
                                    && record.created
                                    && record.opaque_rev == now_rev))
                    });
                    let tuple = (id.clone(), kind, now_rev);
                    if failed && !own {
                        published_external.push(tuple);
                    } else {
                        published_own.push(tuple);
                    }
                    changed_any = true;
                    self.store.graph.invalidate_cache();
                }
                continue;
            }
            // A page's metadata comes from the read's own handle, before its
            // bytes, so the own-write stamp can later prove these bytes
            // unchanged without another open or re-hash (GH #623). Handle
            // metadata follows a symlink where the watcher's does not; the
            // stamp is reused only when a later `symlink_metadata` of a
            // regular file matches it, so a symlinked page takes the full
            // stamp as before.
            let mut stamped = None;
            let now = match if fault(self.store, FaultPoint::PublicationReadIo) {
                Err(io::Error::other("injected publication read error"))
            } else if missing_spelling {
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else if self.page(&id) {
                crate::model::read_parse_bytes_observed(&path).map(|(bytes, metadata)| {
                    stamped = crate::watch::stamp_from_metadata(&metadata);
                    bytes
                })
            } else {
                fs::read(&path)
            } {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => {
                    publication_errors.push((id.clone(), error.into()));
                    self.store.graph.invalidate_cache();
                    continue;
                }
            };
            if let (Some(stamp), Some(bytes)) = (stamped, &now) {
                let rev = FileRev::from_bytes(bytes);
                observations
                    .stamps
                    .insert(id.clone(), stamp.with_rev(Some(rev)));
            }
            let own_final = file_undos.get(&id).into_iter().flatten().any(|record| {
                #[cfg(feature = "test-faults")]
                crate::cost_counters::transaction_record_probe();
                if record.src != id && record.dst.as_ref() != Some(&id) {
                    return false;
                }
                match (&record.new, &now) {
                    (Some(Expected::Bytes(written)), Some(bytes)) => written == bytes,
                    (Some(Expected::File(stage)), Some(bytes)) => {
                        let now_rev = FileRev::from_bytes(bytes);
                        record.new_rev.as_ref().is_some_and(|rev| *rev == now_rev)
                            || (record.new_rev.is_none()
                                && FileRev::from_file(stage).is_ok_and(|rev| rev == now_rev))
                    }
                    (_, None) => record.moved,
                    _ => false,
                }
            });
            if failed && baseline.is_none() && now.is_some() && !own_final {
                if !rollback.kept_external.iter().any(|(kept, _)| *kept == id) {
                    rollback.kept_external.push((id.clone(), None));
                }
            }
            if failed
                && baseline
                    .as_ref()
                    .is_some_and(|old| now.as_ref() != Some(old))
            {
                if now.is_some()
                    && !own_final
                    && !rollback.kept_external.iter().any(|(kept, _)| *kept == id)
                {
                    rollback.kept_external.push((id.clone(), None));
                }
                if !kept_old.contains(&id) {
                    self.preserve_old(&id, baseline.as_ref().unwrap(), rollback);
                }
            }
            self.store.watch.settle_asset(&path);
            if now.as_ref() != baseline.as_ref() {
                let kind = match (baseline, &now) {
                    (None, Some(_)) => ChangeKind::Created,
                    (Some(_), None) => ChangeKind::Removed,
                    _ => ChangeKind::Modified,
                };
                let tuple = (
                    id.clone(),
                    kind,
                    now.as_ref().map(|bytes| FileRev::from_bytes(bytes)),
                );
                let external_after_undo = failed
                    && (rollback
                        .kept_external
                        .iter()
                        .any(|(kept, recovery)| kept == &id && recovery.is_none())
                        || !own_final);
                if external_after_undo {
                    published_external.push(tuple);
                } else {
                    published_own.push(tuple);
                }
            }
            if self.page(&id) {
                if now.as_ref() != baseline.as_ref() {
                    changed_any = true;
                    let saved_page = if !failed {
                        source_plans
                            .get(&id)
                            .map(|&index| &plans[index])
                            .filter(|plan| plan.new.as_deref() == now.as_deref())
                            .and_then(|plan| plan.saved_page.as_ref())
                    } else {
                        None
                    };
                    let own_rename = !failed
                        && own_final
                        && source_plans
                            .get(&id)
                            .is_some_and(|&index| matches!(steps[index], Step::Rewrite { .. }));
                    // A held page's reservation publishes its index (A-V4).
                    let held = || now.as_deref().map(std::sync::Arc::from);
                    if let Some(key) = self.store.graph.page_path_of(&path) {
                        self.store.graph.held.indexed(&key, held);
                    }
                    if let Some(entry) = self.store.graph.transaction_publish_page_inner(
                        &path,
                        now.as_deref(),
                        saved_page,
                        baseline.is_none() || now.is_none(),
                        own_rename,
                    ) {
                        observations.entries.insert(id.clone(), entry);
                    }
                } else {
                    self.store.graph.transaction_clear_page_marker(&path);
                }
            } else if now.as_ref() != baseline.as_ref() {
                changed_any = true;
            }
        }
        FilePublication {
            changed_any,
            spelling_moves,
            publication_errors,
            published_own,
            published_external,
            observations,
        }
    }
}
