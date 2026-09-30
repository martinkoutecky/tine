//! Read-dependent safety checks at the Store writer boundary.
use super::*;
impl Transaction<'_> {
    // Disk/permission failures or a published external reference invalidate an
    // orphan claim. The caller holds the writer, so publication cannot race
    // this check and trash; unobserved external edits remain outside this lock.
    pub(super) fn check_orphan_asset(&self, file: &FileId) -> Result<(), Why> {
        let Some(name) = file.as_str().strip_prefix("assets/") else {
            return Ok(());
        };
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
