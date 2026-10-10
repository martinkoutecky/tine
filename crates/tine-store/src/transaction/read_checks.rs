//! Read-dependent safety checks at the Store writer boundary.
use super::*;
impl Transaction<'_> {
    // Initial parsing can need the writer; wait before commit acquires it.
    pub(super) fn await_reference_publication(&self) {
        if self.steps.iter().any(|step| {
            matches!(
                step,
                Step::Trash {
                    orphan_only: true,
                    ..
                }
            )
        }) {
            let _ = self.store.whole_graph_reconciled();
        }
    }
    pub(super) fn config_write_failure(&self) -> Option<TxOutcome> {
        self.store
            .config()
            .problem
            .map(|problem| TxOutcome::NotCommitted {
                step: 0,
                why: Why::Failed(problem),
                rollback: Rollback::default(),
                publication_errors: Vec::new(),
                graph_rev: self.store.changes.rev(),
            })
    }
    /// The [`Transaction::expect_view`] precondition, under the writer: a
    /// publication or a config delivery since the plan's view is a conflict
    /// (in-scope scenario: an external editor or sync service changes a
    /// referrer or `config.edn` between plan and commit; the caller replans).
    pub(super) fn stale_view(&self) -> Option<TxOutcome> {
        let (rev, config) = self.view.as_ref()?;
        let current = self.store.changes.rev();
        if *rev == current && std::sync::Arc::ptr_eq(config, &self.store.config().config) {
            return None;
        }
        let file = FileId::from("logseq/config.edn".to_owned());
        let disk = FileRev::from_file(&self.store.graph.root.join(file.as_str())).ok();
        Some(TxOutcome::NotCommitted {
            step: 0,
            why: Why::Conflict { file, disk },
            rollback: Rollback::default(),
            publication_errors: Vec::new(),
            graph_rev: current,
        })
    }
    // Disk/permission failures or a published external reference invalidate an
    // orphan claim. The caller holds the writer, so publication cannot race
    // this check and trash; unobserved external edits remain outside this lock.
    pub(super) fn check_orphan_asset(&self, file: &FileId) -> Result<(), Why> {
        let Some(name) = file.as_str().strip_prefix("assets/") else {
            return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
        };
        // Never the launch checkpoint before its diff (ADR 0070): an edit
        // made while Tine was closed may reference this asset. Under the
        // writer a served-but-unreconciled state cannot be observed, since
        // `checkpoint::launch_from` holds the writer from serving until
        // Ready, so the plain view here is the reconciled one.
        let view = self.store.whole_graph().map_err(|error| {
            Why::Failed(
                io::Error::other(format!("asset reference inventory unavailable: {error:?}"))
                    .into(),
            )
        })?;
        if !view.unreadable_files().is_empty() {
            return Err(Why::Failed(
                io::Error::other("asset reference inventory is partial").into(),
            ));
        }
        if view.referenced_assets().contains(name) {
            return Err(Why::Refused(Refusal::AssetReferenced));
        }
        Ok(())
    }
}
impl Transaction<'_> {
    /// Stage-2 test injections for `file`, then its path.
    pub(super) fn stage2_faults(
        &self,
        file: &FileId,
        old: Option<&[u8]>,
        index: usize,
    ) -> Result<PathBuf, Why> {
        let path = self.path(file)?;
        if fault(self.store, FaultPoint::Stage2ConfigExternal) {
            atomic_write(&path, b"{:external true :start-of-week 1}\n").map_err(failed)?;
        }
        if fault(self.store, FaultPoint::Stage2ValidSidecar) {
            let external = b"{:highlights [] :foreign \"external\"}";
            let result = if old.is_some() {
                atomic_write(&path, external)
            } else {
                atomic_write_new(&path, external)
            };
            result.map_err(failed)?;
        }
        if fault(self.store, FaultPoint::Stage2Mismatch)
            || fault(self.store, FaultPoint::Stage2MismatchAt(index))
        {
            let result = if old.is_some() {
                atomic_write(&path, b"external stage-2")
            } else {
                atomic_write_new(&path, b"external stage-2")
            };
            result.map_err(failed)?;
        }
        if fault(self.store, FaultPoint::Stage2ExternalDelete) {
            #[cfg(any(test, feature = "test-faults"))]
            inject_external_delete(&path).map_err(failed)?;
        }
        Ok(path)
    }

    /// Stage-2 base-revision check: `file` still holds `old` (absent for
    /// `None`). Scenario: an external editor or sync service wrote the file
    /// after preflight staged it (refusal table: base-revision conflict).
    pub(super) fn verify(
        &self,
        file: &FileId,
        old: Option<&[u8]>,
        index: usize,
    ) -> Result<(), Why> {
        let path = self.stage2_faults(file, old, index)?;
        match if self.page(file) {
            crate::model::read_parse_bytes(&path)
        } else {
            fs::read(path)
        } {
            Ok(now) if old == Some(now.as_slice()) => Ok(()),
            Ok(now) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(FileRev::from_bytes(&now)),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound && old.is_none() => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Why::Conflict {
                file: file.clone(),
                disk: None,
            }),
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    /// Whether `apply` may skip the separate stage-2 read. A reference rewrite that
    /// changes an existing page skips the separate read: `apply` writes it
    /// with `atomic_write_with_check`, whose final pre-rename guard reads the
    /// same file through the same `read_parse_bytes` and compares it with the
    /// same `old` bytes before the rename, refusing a mismatch or absence as
    /// the same `Why::Conflict` (via `collision`). The external-editor/sync
    /// race this check defends against is therefore caught later in the same
    /// step, before anything becomes visible; only a temp file is written
    /// and withdrawn first (GH #623: one fewer open per referrer). Saves and
    /// replacements keep both checks: an unchanged or new file has no final
    /// guard, and their callers' contracts are not narrowed here.
    pub(super) fn final_guard_covers(step: &Step, old: Option<&[u8]>, new: &[u8]) -> bool {
        matches!(step, Step::Rewrite { .. }) && old.is_some_and(|old| old != new)
    }

    pub(super) fn verify_opaque(&self, file: &FileId, expected: &FileRev) -> Result<(), Why> {
        let path = self.path(file)?;
        match FileRev::from_file(&path) {
            Ok(rev) if rev == *expected => Ok(()),
            Ok(rev) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(rev),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Why::Conflict {
                file: file.clone(),
                disk: None,
            }),
            Err(error) => Err(failed(error)),
        }
    }
}
impl Store {
    /// Whether a streaming descriptor still names the live validated file.
    /// Cost O(1) open/identity checks, no content read. IO/path failures are
    /// errors; false means external editor/sync replacement. A later external
    /// replacement can still occur after this observation.
    pub fn read_is_current(&self, file: &FileId, input: &File) -> Result<bool, StoreError> {
        let (live, _) = self.open_read(file)?;
        let held = same_file::Handle::from_file(input.try_clone().map_err(StoreError::from_io)?)
            .map_err(StoreError::from_io)?;
        let current = same_file::Handle::from_file(live).map_err(StoreError::from_io)?;
        Ok(held == current)
    }
}
