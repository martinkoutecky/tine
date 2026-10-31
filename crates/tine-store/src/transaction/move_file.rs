//! Guarded atomic moves, including case-only spelling changes.
use super::*;

impl PagePlan<'_> {
    /// The spelling preflight holds the write set's path locks and writer.
    pub(crate) fn verify_spelling_input(
        &self,
        file: &FileId,
        bytes: Option<&[u8]>,
    ) -> Result<(), Why> {
        self.verify(file, bytes, 0)
    }

    /// The host driver holds the page's path lock and store writer. Keep the
    /// same old-byte recovery copy as a rewritten transaction rename.
    pub(crate) fn retain_spelling_bytes(&self, from: &FileId, bytes: &[u8]) -> Result<(), Why> {
        self.verify(from, Some(bytes), 0)?;
        self.write_trash_copy(&self.trash_id(from), bytes)?;
        Ok(())
    }

    /// Host driver's byte-preserving spelling move. The caller holds the
    /// store writer and the source key's path lock, and has refused unsaved
    /// input. Reuse the transaction's target checks and durable move recipe;
    /// no content is written and no transaction is committed here.
    pub(crate) fn respell_entry(
        &self,
        from: &FileId,
        to: &FileId,
        bytes: &[u8],
    ) -> (bool, Result<(), Why>) {
        let mut undo = Undo {
            kind: UndoKind::Rename,
            src: from.clone(),
            dst: Some(to.clone()),
            trash: None,
            old: None,
            new: None,
            new_rev: None,
            opaque_rev: None,
            created: false,
            moved: false,
        };
        let result = (|| {
            let src = self.path(from)?;
            let dst = self.spelled_path(to)?;
            // Same-file equality alone accepts symlinks and hard links;
            // only an unlisted folded spelling or an absent case-only
            // destination on a sensitive volume is eligible.
            let absent_case_destination =
                case_only(Path::new(from.as_str()), Path::new(to.as_str()))
                    && matches!(fs::symlink_metadata(&dst),
                    Err(error) if error.kind() == io::ErrorKind::NotFound);
            if !self.destination_is_source_spelling(from, to)? && !absent_case_destination {
                return Err(Why::Conflict {
                    file: to.clone(),
                    disk: disk_rev(&dst),
                });
            }
            self.verify(from, Some(bytes), 0)?;
            self.publish_move(&mut undo, &src, &dst).map_err(failed)
        })();
        // The provider may accept rename while retaining the source casing.
        // Only a fresh directory listing can justify following the target;
        // a sync failure can still leave that observed move live.
        let moved = self.spelling_moved(from, to);
        let result = result.and_then(|()| {
            if moved {
                Ok(())
            } else {
                Err(failed(io::Error::other(
                    "could not confirm the destination spelling",
                )))
            }
        });
        (moved, result)
    }

    fn spelling_moved(&self, from: &FileId, to: &FileId) -> bool {
        self.spelled_path(to).ok().is_some_and(|dst| {
            let src = self.store.graph.root.join(from.as_str());
            self.spelling_entries
                .borrow_mut()
                .remove(dst.parent().unwrap());
            matches!(self.listed_path(&dst), Ok(Some(_)))
                && matches!(self.listed_path(&src), Ok(None))
        })
    }

    /// Publish an unheld entry's spelling change after the driver moved it.
    /// Reuse final-file observation, including its exact listing checks:
    /// on a folding volume reading the old spelling still opens the new
    /// entry, and must not publish a ghost source row. No writes or undo.
    pub(crate) fn publish_respelled_entry(
        &self,
        from: &FileId,
        to: &FileId,
        bytes: &[u8],
    ) -> Result<(), Why> {
        let plan = Prepared {
            src: from.clone(),
            dst: Some(to.clone()),
            old: Some(bytes.to_vec()),
            new: Some(bytes.to_vec()),
            saved_page: None,
            opaque_rev: None,
        };
        let before = BTreeMap::from([
            (from.as_str().to_owned(), Some(bytes.to_vec())),
            (to.as_str().to_owned(), None),
        ]);
        let starting = self.store.graph.cache_generation();
        let publication = self.publish_final_files(
            &[plan],
            &[],
            &before,
            false,
            &mut Rollback::default(),
            &[],
            &[],
        );
        if publication.changed_any && self.store.graph.cache_generation() == starting {
            let root = &self.store.graph.root;
            self.store
                .graph
                .transaction_bump_generation(&[&root.join(from.as_str()), &root.join(to.as_str())]);
        }
        self.store
            .publish_own(publication.published_own, publication.observations);
        match publication.publication_errors.into_iter().next() {
            Some((_, error)) => Err(Why::Failed(error)),
            None => Ok(()),
        }
    }

    // OS handoff canonicalizes an existing final component. A move must keep
    // the requested spelling while retaining that door's target validation.
    pub(super) fn spelled_path(&self, file: &FileId) -> Result<PathBuf, Why> {
        self.path(file)?;
        let candidate = self.store.graph.root.join(file.as_str());
        let parent = candidate.parent().unwrap();
        // Resolve the parent, never a final symlink's target: a separately
        // listed destination must remain occupied at its requested location.
        let (existing, resolved) =
            crate::model::canonical_existing_ancestor(parent).map_err(failed)?;
        Ok(resolved
            .join(parent.strip_prefix(existing).unwrap())
            .join(candidate.file_name().unwrap()))
    }

    pub(super) fn spelling_move(plan: &Prepared) -> bool {
        plan.dst
            .as_ref()
            .is_some_and(|dst| case_only(Path::new(plan.src.as_str()), Path::new(dst.as_str())))
    }

    // Physical directory spellings are exact. Folding these keys would merge
    // coexisting NFC/NFD files on sensitive volumes and hide normalization-only
    // self-alias renames on APFS. The resolver/same_file probe decides aliases.
    pub(super) fn listed_path(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let parent = path.parent().unwrap();
        let mut entries = self.spelling_entries.borrow_mut();
        if !entries.contains_key(parent) {
            let names = fs::read_dir(parent)?
                .map(|entry| {
                    entry.map(|entry| {
                        let name = entry.file_name();
                        (name.clone(), name)
                    })
                })
                .collect::<io::Result<BTreeMap<_, _>>>()?;
            entries.insert(parent.to_path_buf(), names);
        }
        Ok(entries[parent]
            .get(path.file_name().unwrap())
            .map(|name| parent.join(name)))
    }

    /// Same-file equality alone also accepts two hard links. Only an absent
    /// destination directory entry can be an alternate spelling of the source.
    fn destination_is_source_spelling(&self, from: &FileId, to: &FileId) -> Result<bool, Why> {
        let src = self.path(from)?;
        let dst = self.spelled_path(to)?;
        self.source_spelling_alias(&src, &dst, &dst).map_err(failed)
    }

    fn source_spelling_alias(
        &self,
        src: &Path,
        dst: &Path,
        resolved_dst: &Path,
    ) -> io::Result<bool> {
        if !case_only(src, dst) || self.listed_path(dst)?.is_some() {
            return Ok(false);
        }
        match same_file::is_same_file(src, resolved_dst) {
            Ok(same) => Ok(same),
            Err(error) if crate::atomic_file::names_nothing(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn available_move_destination(&self, from: &FileId, to: &FileId) -> Result<(), Why> {
        if self.destination_is_source_spelling(from, to)? {
            Ok(())
        } else {
            self.absent(to)
        }
    }

    fn verify_move_destination(&self, from: &FileId, to: &FileId, index: usize) -> Result<(), Why> {
        if self.destination_is_source_spelling(from, to)? {
            Ok(())
        } else {
            self.verify(to, None, index)
        }
    }

    /// First try the shared no-replace primitive. Only the source's own
    /// unlisted folded alias permits the atomic-write path's plain rename.
    /// Distinct entries (including hard links) remain collisions (GH #609).
    fn publish_move(&self, undo: &mut Undo, src: &Path, dst: &Path) -> io::Result<()> {
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortBeforeMoveRename) {
            std::process::abort();
        }
        // Linux test fixtures model a folding filesystem's refusal and alias
        // resolution while still executing the real plain rename and listing.
        let alias_refusal =
            case_only(src, dst) && fault(self.store, FaultPoint::CaseMoveAliasRefusal);
        if case_only(src, dst) {
            // A folding resolver returns the source's existing spelling for
            // dst. Check the bound parent and source identity, then retain
            // the requested leaf. Only the source's unlisted alias may differ
            // from that leaf; ordinary target containment remains strict.
            self.check_write_target(src)?;
            self.check_write_target(dst.parent().unwrap())?;
            let (existing, resolved) = crate::model::canonical_existing_ancestor(dst)?;
            if existing == dst
                && resolved != dst
                && !self.source_spelling_alias(src, dst, &resolved)?
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "spelling destination is not the source alias",
                ));
            }
        } else {
            self.check_write_target(dst)?;
        }
        let result = if alias_refusal {
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        } else {
            move_file_noreplace(src, dst)
        };
        if let Err(error) = result {
            if error.kind() != io::ErrorKind::AlreadyExists || !case_only(src, dst) {
                return Err(error);
            }
            self.spelling_entries
                .borrow_mut()
                .remove(src.parent().unwrap());
            let resolved_dst = if alias_refusal { src } else { dst };
            if !self.source_spelling_alias(src, dst, resolved_dst)? {
                return Err(error);
            }
            if !fault(self.store, FaultPoint::CaseMoveAliasNoop) {
                rename_spelling_replace(src, dst).map_err(crate::platform_step::at(
                    "rename source over its folded alias",
                ))?;
            }
            // The fault models a provider returning Ok from rename_replace
            // while silently keeping the old directory spelling.
            undo.created = true;
        } else {
            undo.created = true;
        }
        if case_only(src, dst) {
            self.spelling_entries
                .borrow_mut()
                .remove(src.parent().unwrap());
            if self.listed_path(dst)?.is_none() && self.listed_path(src)?.is_some() {
                // The provider returned success but kept the source's case
                // (Android shared storage, GS1): move through a name that
                // differs by more than case.
                self.respell_through_temporary(src, dst)?;
                self.spelling_entries
                    .borrow_mut()
                    .remove(src.parent().unwrap());
            }
            if self.listed_path(dst)?.is_none() || self.listed_path(src)?.is_some() {
                return Err(io::Error::other(
                    "case rename did not publish the destination spelling",
                ));
            }
        }
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortBeforeMoveSync) {
            std::process::abort();
        }
        sync_move_dirs(self.store, src, dst)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterMoveRename) {
            std::process::abort();
        }
        Ok(())
    }

    /// The two halves of a case move on a provider that keeps the old case
    /// (GS1): `src` to its temporary entry `logseq/.tine-respell/<dst's
    /// graph-relative path>`, then to `dst`, each a no-replace move with its
    /// directories synced. A failed second half moves the entry back; if that
    /// fails too, the bytes stay at the temporary entry and the next host
    /// launch finishes the move ([`Store::complete_case_moves`]). An existing
    /// temporary entry (an earlier unfinished move) refuses, moving nothing.
    fn respell_through_temporary(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let temp = case_move_temporary(&self.store.graph.root, dst)?;
        let parent = temp.parent().unwrap();
        crate::directory_durability::create_dir_all_durable(
            parent,
            self.store.graph.creation_bound(parent),
            &self.store.graph.root_binding,
        )?;
        self.check_write_target(&temp)?;
        move_file_noreplace(src, &temp)?;
        sync_move_dirs(self.store, src, &temp)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortBetweenCaseMoveSteps) {
            std::process::abort();
        }
        let second = self.check_write_target(dst).and_then(|()| {
            if fault(self.store, FaultPoint::CaseMoveTemporaryIo) {
                Err(io::Error::other("injected case move temporary failure"))
            } else {
                move_file_noreplace(&temp, dst)
            }
        });
        match second {
            Ok(()) => sync_move_dirs(self.store, &temp, dst),
            Err(error) => {
                if self.check_write_target(src).is_ok() && move_file_noreplace(&temp, src).is_ok() {
                    let _ = sync_move_dirs(self.store, &temp, src);
                }
                Err(error)
            }
        }
    }

    /// Finish case moves a stopped process left at their temporary entry
    /// (GS1). The caller holds the store writer, before any host page is
    /// registered. Each regular file `logseq/.tine-respell/<rel>` moves to
    /// `<rel>` when no entry of that folder spells `<rel>`'s name (exactly or
    /// folded), and is published at once; otherwise it moves into page trash
    /// as `<stamp>__respell__<name>`, never over the listed file. Failures
    /// leave the entry for the next launch; none stops the launch.
    pub(crate) fn complete_case_moves(&self) {
        let root = self.store.graph.root.clone();
        let base = root.join(CASE_MOVE_DIR);
        if !fs::symlink_metadata(&base).is_ok_and(|meta| meta.is_dir()) {
            return;
        }
        let mut pending = vec![base.clone()];
        let mut found = Vec::new();
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                    Ok(kind) if kind.is_file() => found.push(entry.path()),
                    _ => {}
                }
            }
        }
        found.sort();
        for temp in found {
            let rel = temp.strip_prefix(&base).unwrap().to_path_buf();
            let Some(rel_str) = rel.to_str().map(|rel| rel.replace('\\', "/")) else {
                continue;
            };
            let dst = root.join(&rel);
            match self.complete_case_move(&temp, &dst, &FileId::from(rel_str)) {
                Ok(true) => {
                    self.store
                        .graph
                        .storage_diag
                        .case_moves_completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Ok(false) => {
                    self.store
                        .graph
                        .storage_diag
                        .case_moves_kept_in_trash
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => {}
            }
        }
        // Empty folders are left from finished moves; a non-empty one keeps
        // an entry for the next launch and refuses removal.
        let mut dirs = vec![base];
        let mut all = Vec::new();
        while let Some(dir) = dirs.pop() {
            if let Ok(entries) = fs::read_dir(&dir) {
                dirs.extend(
                    entries
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|kind| kind.is_dir()))
                        .map(|e| e.path()),
                );
            }
            all.push(dir);
        }
        for dir in all.into_iter().rev() {
            if self.check_write_target(&dir).is_ok() {
                let _ = crate::model::remove_graph_dir(&dir);
            }
        }
    }

    /// One temporary entry: `Ok(true)` moved to `dst` and published,
    /// `Ok(false)` kept in trash because `dst`'s spelling is taken.
    fn complete_case_move(&self, temp: &Path, dst: &Path, id: &FileId) -> io::Result<bool> {
        self.check_write_target(temp)?;
        let parent = dst.parent().unwrap();
        let leaf = dst.file_name().unwrap();
        let taken = self.check_write_target(parent).is_err()
            || !fs::symlink_metadata(parent).is_ok_and(|meta| meta.is_dir())
            || fs::read_dir(parent)?.flatten().any(|entry| {
                let name = entry.file_name();
                name == leaf || case_only(Path::new(&name), Path::new(leaf))
            });
        if !taken {
            self.check_write_target(dst)?;
            move_file_noreplace(temp, dst)?;
            sync_move_dirs(self.store, temp, dst)?;
            let bytes = fs::read(dst)?;
            let plan = Prepared {
                src: id.clone(),
                dst: None,
                old: None,
                new: Some(bytes),
                saved_page: None,
                opaque_rev: None,
            };
            let before = BTreeMap::from([(id.as_str().to_owned(), None)]);
            let starting = self.store.graph.cache_generation();
            let publication = self.publish_final_files(
                &[plan],
                &[],
                &before,
                false,
                &mut Rollback::default(),
                &[],
                &[],
            );
            if publication.changed_any && self.store.graph.cache_generation() == starting {
                self.store.graph.transaction_bump_generation(&[dst]);
            }
            self.store
                .publish_own(publication.published_own, publication.observations);
            return Ok(true);
        }
        let name = leaf.to_string_lossy();
        let trash = crate::model::trash_root(&self.store.graph.root)
            .join("pages")
            .join(crate::atomic_file::prefixed_name(
                &format!("{}__respell__", crate::model::trash_stamp()),
                &name,
            ));
        crate::directory_durability::create_dir_all_durable(
            trash.parent().unwrap(),
            self.store.graph.creation_bound(trash.parent().unwrap()),
            &self.store.graph.root_binding,
        )?;
        self.check_write_target(&trash)?;
        move_file_noreplace(temp, &trash)?;
        sync_move_dirs(self.store, temp, &trash)?;
        Ok(false)
    }

    pub(super) fn undo_opaque_move(
        &self,
        record: &Undo,
        dst_id: &FileId,
        dst: &Path,
        live: &Path,
    ) -> io::Result<()> {
        let mut undo = Undo {
            kind: UndoKind::Rename,
            src: dst_id.clone(),
            dst: Some(record.src.clone()),
            trash: None,
            old: None,
            new: None,
            new_rev: None,
            opaque_rev: record.opaque_rev.clone(),
            created: false,
            moved: false,
        };
        self.publish_move(&mut undo, dst, live)
    }

    pub(super) fn apply_move(
        &self,
        plan: &Prepared,
        undo: &mut Undo,
        index: usize,
    ) -> Result<StepResult, Why> {
        let src = self.path(&plan.src)?;
        let dst_id = plan.dst.as_ref().expect("move destination");
        let dst = self.spelled_path(dst_id)?;
        if let Some(rev) = &plan.opaque_rev {
            self.verify_opaque(&plan.src, rev)?;
            self.verify_move_destination(&plan.src, dst_id, index)?;
            if let Some(parent) = dst.parent() {
                crate::directory_durability::create_dir_all_durable(
                    parent,
                    self.store.graph.creation_bound(parent),
                    &self.store.graph.root_binding,
                )
                .map_err(failed)?;
            }
            undo.kind = UndoKind::Rename;
            undo.opaque_rev = Some(rev.clone());
            self.fault_collision(&dst);
            self.publish_move(undo, &src, &dst)
                .map_err(|error| collision(dst_id, error, &dst))?;
            undo.created = true;
            self.fault_mid_step(index)?;
            self.fault_twin(dst_id);
            if let Some(twin) = self.disk_twin(dst_id)? {
                return Err(Why::Conflict {
                    file: twin.clone(),
                    disk: disk_rev(&self.path(&twin)?),
                });
            }
            self.verify_opaque(dst_id, rev)?;
            return Ok(StepResult::Moved {
                to: dst_id.clone(),
                rev: rev.clone(),
            });
        }
        let old = plan.old.as_deref().expect("move baseline");
        let new = plan.new.as_ref().expect("move bytes");
        self.verify(&plan.src, Some(old), index)?;
        self.verify_move_destination(&plan.src, dst_id, index)?;
        if let Some(parent) = dst.parent() {
            crate::directory_durability::create_dir_all_durable(
                parent,
                self.store.graph.creation_bound(parent),
                &self.store.graph.root_binding,
            )
            .map_err(failed)?;
        }
        if new.as_slice() == old {
            // Content unchanged: a guarded no-replace rename, so no copy
            // of the source is left in the trash. Undo withdraws the
            // destination and writes the baseline back under `src`.
            undo.kind = UndoKind::Rename;
            undo.new = Some(Expected::Bytes(old.to_vec()));
            if self.page(dst_id) {
                self.store.graph.transaction_note_page(&dst, old);
            }
            if self.page(&plan.src) {
                self.store.graph.transaction_note_delete(&src);
            }
            self.fault_collision(&dst);
            self.publish_move(undo, &src, &dst)
                .map_err(|error| collision(dst_id, error, &dst))?;
            undo.created = true;
            self.fault_mid_step(index)?;
            self.fault_twin(dst_id);
            if let Some(twin) = self.disk_twin(dst_id)? {
                return Err(Why::Conflict {
                    file: twin.clone(),
                    disk: disk_rev(&self.path(&twin)?),
                });
            }
            if fs::read(&dst).map_err(failed)? != old {
                return Err(Why::Conflict {
                    file: plan.src.clone(),
                    disk: disk_rev(&dst),
                });
            }
            return Ok(StepResult::Moved {
                to: dst_id.clone(),
                rev: FileRev::from_bytes(old),
            });
        }
        // Publish the original bytes at the destination, then replace through
        // the audited save primitive. Every move keeps one live name.
        // Before replacement the destination has old bytes; after it, new bytes.
        undo.kind = UndoKind::Rename;
        undo.new = Some(Expected::Bytes(old.to_vec()));
        self.fault_collision(&dst);
        if self.page(dst_id) {
            self.store.graph.transaction_note_page(&dst, new);
        }
        if self.page(&plan.src) {
            self.store.graph.transaction_note_delete(&src);
        }
        self.publish_move(undo, &src, &dst)
            .map_err(|error| collision(dst_id, error, &dst))?;
        undo.created = true;
        self.fault_mid_step(index)?;

        // Preserve the original bytes for the same recovery affordance
        // as the old destination-first move.
        let trash_id = self.trash_id(&plan.src);
        let trash = self.write_trash_copy(&trash_id, old)?;
        undo.trash = Some(trash_id);
        if fault(self.store, FaultPoint::MoveAfterTrashCopyIo) {
            return Err(failed(io::Error::other(
                "injected failure after move trash copy",
            )));
        }
        self.arm_directory_sync_fault();
        self.check_write_target(&dst).map_err(failed)?;
        if let Err(error) = atomic_write_with_check(&dst, new, || {
            if fs::read(&dst)? == old {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "moved page changed before rewrite",
                ))
            }
        }) {
            if crate::directory_durability::is_directory_sync_failure(&error) {
                undo.new = Some(Expected::Bytes(new.clone()));
            }
            return Err(failed(error));
        }
        undo.new = Some(Expected::Bytes(new.clone()));
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterMoveRewrite) {
            std::process::abort();
        }
        self.fault_mid_step(index)?;
        self.fault_twin(dst_id);
        if let Some(twin) = self.disk_twin(dst_id)? {
            return Err(Why::Conflict {
                file: twin.clone(),
                disk: disk_rev(&self.path(&twin)?),
            });
        }
        if fs::read(&trash).map_err(failed)? != old {
            return Err(Why::Conflict {
                file: plan.src.clone(),
                disk: disk_rev(&trash),
            });
        }
        Ok(StepResult::Moved {
            to: dst_id.clone(),
            rev: FileRev::from_bytes(new),
        })
    }
}

fn rename_spelling_replace(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(test)]
    crate::model::assert_checked_target(dst);
    crate::atomic_file::rename_replace(src, dst)
}

pub(super) use crate::file_kind::case_only_paths as case_only;

/// Tine's per-device folder for case moves in progress (GS1): never staged by
/// git, pulled, indexed or watched as pages (`logseq/.tine-*`).
pub(crate) const CASE_MOVE_DIR: &str = "logseq/.tine-respell";

/// A case move's temporary entry for destination `dst` inside `root`.
pub(crate) fn case_move_temporary(root: &Path, dst: &Path) -> io::Result<PathBuf> {
    let rel = dst
        .strip_prefix(root)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "case move outside the graph"))?;
    Ok(root.join(CASE_MOVE_DIR).join(rel))
}

#[cfg(test)]
mod spelling_tests {
    use super::*;

    #[test]
    fn rg7_unchecked_folded_spelling_replace_panics_before_rename() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("old.md");
        let dst = dir.path().join("Old.md");
        fs::write(&src, b"kept").unwrap();
        crate::model::write_target::clear_checked_target();
        assert!(std::panic::catch_unwind(|| rename_spelling_replace(&src, &dst)).is_err());
        assert_eq!(fs::read(src).unwrap(), b"kept");
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            [std::ffi::OsString::from("old.md")]
        );
    }

    #[test]
    fn a_case_move_keeps_the_requested_leaf_when_its_alias_resolves_to_the_source() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let src = dir.path().join("pages/old.md");
        fs::write(&src, b"- body\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let src = store.graph.root.join("pages/old.md");
        let dst = store.graph.root.join("pages/Old.md");
        let tx = store.page_plan(None);
        let mut undo = Undo {
            kind: UndoKind::Rename,
            src: FileId::from("pages/old.md".to_owned()),
            dst: Some(FileId::from("pages/Old.md".to_owned())),
            trash: None,
            old: None,
            new: None,
            new_rev: None,
            opaque_rev: None,
            created: false,
            moved: false,
        };
        crate::model::write_target::with_folding_alias(&dst, &src, || {
            assert_eq!(
                crate::model::canonical_existing_ancestor(&dst).unwrap().1,
                src
            );
            assert!(
                store.graph.ensure_write_target(&dst).is_err(),
                "ordinary containment must stay strict"
            );
            store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
            tx.publish_move(&mut undo, &src, &dst).unwrap();
        });
        assert_eq!(fs::read(&dst).unwrap(), b"- body\n");
        assert!(tx.listed_path(&src).unwrap().is_none());
        assert_eq!(tx.listed_path(&dst).unwrap(), Some(dst));
    }

    fn noop_case_move(temp_fails: bool) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "- body\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let from = FileId::from("pages/old.md".to_owned());
        let to = FileId::from("pages/Old.md".to_owned());
        let tx = store.page_plan(None);
        let mut undo = Undo {
            kind: UndoKind::Rename,
            src: from.clone(),
            dst: Some(to.clone()),
            trash: None,
            old: None,
            new: None,
            new_rev: None,
            opaque_rev: None,
            created: false,
            moved: false,
        };
        store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
        store.inject_fault(FaultPoint::CaseMoveAliasNoop);
        if temp_fails {
            store.inject_fault(FaultPoint::CaseMoveTemporaryIo);
        }
        // The store's canonical root, as production passes it: on Windows the
        // tempdir path can be an 8.3 short name (`RUNNER~1`) outside the binding.
        let src = store.graph.root.join(from.as_str());
        let dst = store.graph.root.join(to.as_str());
        let published = tx.publish_move(&mut undo, &src, &dst);
        assert!(
            undo.created,
            "the provider returned success from rename_replace; publish_move: {published:?}"
        );
        let temporary = store.graph.root.join("logseq/.tine-respell/pages/Old.md");
        assert!(!temporary.exists());
        // Exercise the same post-call observation used by respell_entry.
        if temp_fails {
            assert!(published.is_err());
            assert!(!tx.spelling_moved(&from, &to));
            assert!(tx.listed_path(&src).unwrap().is_some());
            assert!(tx.listed_path(&dst).unwrap().is_none());
            assert_eq!(fs::read(&src).unwrap(), b"- body\n");
        } else {
            published.unwrap();
            assert!(tx.spelling_moved(&from, &to));
            assert_eq!(fs::read(&dst).unwrap(), b"- body\n");
        }
        store.close();
    }

    /// GS1: Android shared storage returns success from a case-only rename
    /// and keeps the old name; the move goes through a temporary entry.
    #[test]
    fn a_provider_keeping_the_old_case_moves_through_a_temporary_entry() {
        noop_case_move(false);
    }

    /// A failed second half moves the entry back: the host keeps the source.
    #[test]
    fn a_failed_temporary_case_move_puts_the_source_back() {
        noop_case_move(true);
    }

    fn launched(files: &[(&str, &str)]) -> (tempfile::TempDir, std::sync::Arc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let app_data = tempfile::tempdir().unwrap();
        let host = crate::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        drop(host);
        (dir, store)
    }

    fn listed(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// GS1: a process stopped between the two halves left the page at its
    /// temporary entry; the next host launch finishes the move and indexes it.
    #[test]
    fn a_launch_finishes_a_case_move_left_at_its_temporary_entry() {
        let (dir, store) = launched(&[
            ("pages/Ref.md", "- [[Old]]\n"),
            ("logseq/.tine-respell/pages/Old.md", "- body\n"),
        ]);
        assert_eq!(listed(&dir.path().join("pages")), ["Old.md", "Ref.md"]);
        assert_eq!(
            fs::read(dir.path().join("pages/Old.md")).unwrap(),
            b"- body\n"
        );
        assert!(!dir.path().join("logseq/.tine-respell").exists());
        // Starting a host recovers the bytes, but the initial graph load is
        // asynchronous. Inspect its snapshot only after that load reconciles.
        store.whole_graph_reconciled().unwrap();
        assert!(store.snapshot_knows(&store.graph.root.join("pages/Old.md")));
        assert_eq!(
            store.graph.storage_diag.snapshot()["caseMovesFinishedAtLaunch"]["completed"],
            1
        );
        store.close();
    }

    /// A launch never writes over a listed file: when the destination name
    /// (or its case fold) is taken, the temporary entry goes to page trash.
    #[test]
    fn a_launch_keeps_a_temporary_entry_in_trash_when_its_name_is_taken() {
        for taken in ["Old.md", "old.md"] {
            let (dir, store) = launched(&[
                (&format!("pages/{taken}"), "- other\n"),
                ("logseq/.tine-respell/pages/Old.md", "- body\n"),
            ]);
            assert_eq!(listed(&dir.path().join("pages")), [taken]);
            assert_eq!(
                fs::read(dir.path().join("pages").join(taken)).unwrap(),
                b"- other\n"
            );
            let trash = dir.path().join("logseq/.tine-trash/pages");
            let kept = listed(&trash);
            assert_eq!(kept.len(), 1, "{kept:?}");
            assert!(kept[0].ends_with("__respell__Old.md"), "{kept:?}");
            assert_eq!(fs::read(trash.join(&kept[0])).unwrap(), b"- body\n");
            assert!(!dir.path().join("logseq/.tine-respell").exists());
            store.close();
        }
    }

    #[test]
    fn spelling_retains_verified_old_bytes_before_any_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let from = FileId::from("pages/old.md".to_owned());
        let tx = store.page_plan(None);
        assert!(tx.retain_spelling_bytes(&from, b"- stale\n").is_err());
        assert!(!dir.path().join("logseq/.tine-trash").exists());
        tx.retain_spelling_bytes(&from, b"- original\n").unwrap();
        assert_eq!(
            fs::read(dir.path().join("pages/old.md")).unwrap(),
            b"- original\n"
        );
        let copies: Vec<_> = fs::read_dir(dir.path().join("logseq/.tine-trash/pages"))
            .unwrap()
            .map(|e| fs::read(e.unwrap().path()).unwrap())
            .collect();
        assert_eq!(copies, [b"- original\n"]);
        store.close();
    }

    /// The host spelling command may run after an unopened source returned
    /// to disk ownership. Its final-file publication must update the paths
    /// synchronously, without waiting for a watcher or a fresh metadata scan.
    #[test]
    fn spelling_entry_publication_updates_unheld_paths_before_reply() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "title:: Old\n- body\n").unwrap();
        let store = Store::open(
            dir.path(),
            crate::OpenOptions {
                watch: crate::WatchMode::Poll,
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let root = &store.graph.root;
        let src = root.join("pages/old.md");
        let dst = root.join("pages/Old.md");
        assert!(store.snapshot_knows(&src));
        {
            let _writer = store.writer.lock().unwrap();
            let lock = store.graph.page_lock(&src);
            let _guard = lock.lock().unwrap();
            // Fixture for the byte-preserving entry syscall: on Linux the
            // two spellings are distinct, so respell admission itself skips.
            fs::rename(&src, &dst).unwrap();
            store
                .page_plan(None)
                .publish_respelled_entry(
                    &FileId::from("pages/old.md".to_owned()),
                    &FileId::from("pages/Old.md".to_owned()),
                    b"title:: Old\n- body\n",
                )
                .unwrap();
        }
        assert!(store.snapshot_knows(&dst));
        assert!(!store.snapshot_knows(&src));
        store.close();
    }

    #[cfg(unix)]
    #[test]
    fn destination_spelling_never_redirects_into_a_final_symlinks_parent() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pages/sub")).unwrap();
        fs::write(dir.path().join("pages/sub/old.md"), "- body\n").unwrap();
        std::os::unix::fs::symlink("sub/old.md", dir.path().join("pages/Old.md")).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let tx = store.page_plan(None);
        let dest = store.file_id(Area::Pages, "Old.md").unwrap();
        assert_eq!(
            tx.spelled_path(&dest).unwrap(),
            fs::canonicalize(dir.path().join("pages"))
                .unwrap()
                .join("Old.md")
        );
        store.close();
    }

    #[test]
    fn sb1_directory_spelling_keeps_normalization_distinct_entries_separate() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let nfd = dir.path().join("pages/Ku\u{30a}n\u{30c}.md");
        let nfc = dir.path().join("pages/Kůň.md");
        fs::write(&nfd, "- nfd\n").unwrap();
        if nfc.exists() {
            assert!(same_file::is_same_file(&nfd, &nfc).unwrap());
            return; // This volume cannot hold normalization-distinct entries.
        }
        fs::write(&nfc, "- nfc\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let tx = store.page_plan(None);
        assert_eq!(tx.listed_path(&nfd).unwrap(), Some(nfd.clone()));
        assert_eq!(tx.listed_path(&nfc).unwrap(), Some(nfc.clone()));
        assert_eq!(fs::read(nfd).unwrap(), b"- nfd\n");
        assert_eq!(fs::read(nfc).unwrap(), b"- nfc\n");
        store.close();
    }

    #[test]
    fn sb1_unicode_self_aliases_allow_spelling_moves() {
        for (from, to) in [
            ("Kůň", "kůň"),
            ("Kůň", "Ku\u{30a}n\u{30c}"),
            ("ß", "SS"),
            ("I", "ı"),
            ("İ", "i\u{307}"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("pages")).unwrap();
            let source = format!("pages/{from}.md");
            let target = format!("pages/{to}.md");
            fs::write(dir.path().join(&source), b"- body\n").unwrap();
            let store = Store::open(dir.path(), Default::default()).unwrap().0;
            let src = store.graph.root.join(&source);
            let dst = store.graph.root.join(&target);
            let tx = store.page_plan(None);
            // Linux fixture for an actual folding resolver: the unlisted
            // destination resolves to the source, independently of page_key.
            assert!(
                tx.source_spelling_alias(&src, &dst, &src).unwrap(),
                "{from} -> {to}"
            );
            let mut undo = Undo {
                kind: UndoKind::Rename,
                src: FileId::from(source.clone()),
                dst: Some(FileId::from(target.clone())),
                trash: None,
                old: None,
                new: None,
                new_rev: None,
                opaque_rev: None,
                created: false,
                moved: false,
            };
            crate::model::write_target::with_folding_alias(&dst, &src, || {
                store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
                tx.publish_move(&mut undo, &src, &dst).unwrap();
            });
            assert!(tx.spelling_moved(&FileId::from(source), &FileId::from(target)));
            assert_eq!(fs::read(&dst).unwrap(), b"- body\n");
            store.close();
        }
    }

    #[test]
    fn sb1_unicode_listed_hard_link_is_not_a_self_alias() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let src = dir.path().join("pages/ß.md");
        let dst = dir.path().join("pages/SS.md");
        fs::write(&src, b"- kept\n").unwrap();
        if dst.exists() {
            return; // A full-folding volume cannot hold both directory entries.
        }
        fs::hard_link(&src, &dst).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let tx = store.page_plan(None);
        assert_eq!(tx.source_spelling_alias(&src, &dst, &dst).unwrap(), false);
        assert_eq!(fs::read(src).unwrap(), b"- kept\n");
        assert_eq!(fs::read(dst).unwrap(), b"- kept\n");
        store.close();
    }
}
