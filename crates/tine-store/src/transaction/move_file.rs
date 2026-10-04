//! Guarded move application, including recoverable case-only spelling changes.
use super::*;
use unicode_normalization::UnicodeNormalization;

impl Transaction<'_> {
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

    // Preserve case while treating filesystem NFC/NFD spellings as equivalent.
    pub(super) fn listed_path(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let parent = path.parent().unwrap();
        let mut entries = self.spelling_entries.borrow_mut();
        if !entries.contains_key(parent) {
            let names = fs::read_dir(parent)?
                .map(|entry| {
                    entry.map(|entry| {
                        let name = entry.file_name();
                        (spelling_key(&name), name)
                    })
                })
                .collect::<io::Result<BTreeMap<_, _>>>()?;
            entries.insert(parent.to_path_buf(), names);
        }
        Ok(entries[parent]
            .get(&spelling_key(path.file_name().unwrap()))
            .map(|name| parent.join(name)))
    }

    /// Same-file equality alone also accepts two hard links. Only an absent
    /// destination directory entry can be an alternate spelling of the source.
    fn destination_is_source_spelling(&self, from: &FileId, to: &FileId) -> Result<bool, Why> {
        let src = self.path(from)?;
        let dst = self.spelled_path(to)?;
        if src.parent() != dst.parent() || !case_only(&src, &dst) {
            return Ok(false);
        }
        match same_file::is_same_file(&src, &dst) {
            Ok(true) => self
                .listed_path(&dst)
                .map(|listed| listed.is_none())
                .map_err(failed),
            Ok(false) => Ok(false),
            Err(error) if crate::atomic_file::names_nothing(&error) => Ok(false),
            Err(error) => Err(failed(error)),
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

    /// Use the existing trash name and audited platform move for both legs.
    /// No replace onto an alias, no new journal or recovery format.
    fn publish_move(
        &self,
        plan: &Prepared,
        undo: &mut Undo,
        src: &Path,
        dst: &Path,
    ) -> io::Result<()> {
        if !case_only(src, dst) {
            return move_file_noreplace(src, dst);
        }
        let trash_id = self.trash_id(&plan.src);
        let trash = self
            .path(&trash_id)
            .map_err(|why| io::Error::other(format!("{why:?}")))?;
        fs::create_dir_all(trash.parent().unwrap())?;
        // Persist each newly created directory's entry before source withdrawal.
        // Reuse the directory-durability owner, including its platform policy.
        let mut ancestor = trash.parent();
        while let Some(dir) = ancestor {
            crate::directory_durability::sync_directory_entry(dir)?;
            if dir == self.store.graph.root {
                break;
            }
            ancestor = dir.parent();
        }
        undo.trash = Some(trash_id);
        move_file_noreplace(src, &trash)?;
        undo.moved = true;
        sync_move_dirs(self.store, src, &trash)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterCaseMoveStage) {
            std::process::abort();
        }
        if fault(self.store, FaultPoint::CaseMoveStageIo) {
            return Err(io::Error::other("injected failure after case move stage"));
        }
        #[cfg(any(test, feature = "test-faults"))]
        if fault(self.store, FaultPoint::CaseMoveStageCollision) {
            self.store.inject_fault(FaultPoint::NoReplaceCollision);
            self.fault_collision(dst);
        }
        // An external writer may have replaced the source just before staging.
        // Preserve those bytes in recovery and refuse to publish stale rewrites.
        let expected = plan
            .opaque_rev
            .clone()
            .unwrap_or_else(|| FileRev::from_bytes(plan.old.as_deref().unwrap()));
        if FileRev::from_file(&trash)? != expected {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "source changed during case rename",
            ));
        }
        move_file_noreplace(&trash, dst)?;
        undo.moved = false;
        undo.trash = None;
        undo.created = true;
        sync_move_dirs(self.store, &trash, dst)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterCaseMovePublish) {
            std::process::abort();
        }
        Ok(())
    }

    pub(super) fn undo_opaque_move(
        &self,
        record: &Undo,
        dst_id: &FileId,
        dst: &Path,
        live: &Path,
    ) -> io::Result<()> {
        let plan = Prepared {
            src: dst_id.clone(),
            dst: Some(record.src.clone()),
            old: None,
            new: None,
            saved_page: None,
            opaque_rev: record.opaque_rev.clone(),
        };
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
        self.publish_move(&plan, &mut undo, dst, live)
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
                fs::create_dir_all(parent).map_err(failed)?;
            }
            undo.kind = UndoKind::Rename;
            undo.opaque_rev = Some(rev.clone());
            self.fault_collision(&dst);
            self.publish_move(plan, undo, &src, &dst)
                .map_err(|error| collision(dst_id, error, &dst))?;
            undo.created = true;
            sync_move_dirs(self.store, &src, &dst).map_err(failed)?;
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
            fs::create_dir_all(parent).map_err(failed)?;
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
            self.publish_move(plan, undo, &src, &dst)
                .map_err(|error| collision(dst_id, error, &dst))?;
            undo.created = true;
            sync_move_dirs(self.store, &src, &dst).map_err(failed)?;
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
        // the audited save primitive. Case-only staging may temporarily leave
        // only the recoverable trash name; other moves keep one live name.
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
        self.publish_move(plan, undo, &src, &dst)
            .map_err(|error| collision(dst_id, error, &dst))?;
        undo.created = true;
        sync_move_dirs(self.store, &src, &dst).map_err(failed)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterMoveRename) {
            std::process::abort();
        }
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

pub(super) fn case_only(src: &Path, dst: &Path) -> bool {
    src != dst
        && src.parent() == dst.parent()
        && src
            .file_name()
            .and_then(|s| s.to_str())
            .zip(dst.file_name().and_then(|s| s.to_str()))
            .is_some_and(|(from, to)| tine_core::refs::same_page(from, to))
}

fn spelling_key(name: &std::ffi::OsStr) -> std::ffi::OsString {
    name.to_str()
        .map(|name| name.nfc().collect::<String>().into())
        .unwrap_or_else(|| name.to_os_string())
}

#[cfg(test)]
mod spelling_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn destination_spelling_never_redirects_into_a_final_symlinks_parent() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pages/sub")).unwrap();
        fs::write(dir.path().join("pages/sub/old.md"), "- body\n").unwrap();
        std::os::unix::fs::symlink("sub/old.md", dir.path().join("pages/Old.md")).unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let tx = store.transaction(None);
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
    fn directory_spelling_preserves_case_but_accepts_normalized_unicode() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/CAFE\u{301}.md"), "- body\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let tx = store.transaction(None);
        let desired = dir.path().join("pages/CAFÉ.md");
        assert_eq!(
            tx.listed_path(&desired).unwrap(),
            Some(dir.path().join("pages/CAFE\u{301}.md"))
        );
        assert_eq!(
            tx.listed_path(&dir.path().join("pages/café.md")).unwrap(),
            None
        );
        store.close();
    }
}
