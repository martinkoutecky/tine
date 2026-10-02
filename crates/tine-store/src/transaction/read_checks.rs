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
    /// Queue recoverable trash of an unreferenced asset. Commit checks the
    /// latest published graph under the writer, refusing partial inventories
    /// and referenced assets. Cost O(B + source bytes); unobserved external
    /// arrivals can still race after this check. Generic trash has no such
    /// orphan requirement (for example intentional PDF annotation removal).
    pub fn trash_orphan_asset(&mut self, file: &FileId, expected: FileRev) -> &mut Self {
        self.steps.push(Step::Trash {
            file: file.clone(),
            expected,
            orphan_only: true,
        });
        self
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
            return Err(Why::Refused(Refusal::ReadOnly(
                "asset is referenced; refresh the orphan inventory".into(),
            )));
        }
        Ok(())
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
