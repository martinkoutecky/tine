//! Phase seam shared by ModelFs and the unwired production adapter.
use super::Text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Phase {
    Read,
    PageTemp,
    PageRename,
    PageSync,
    TrashMove,
    TrashSync,
    CustodyWrite,
    CustodyRetire,
    CustodyList,
    DraftTemp,
    DraftRename,
    DraftUnlink,
    DraftSync,
    Quarantine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Witness {
    Durable,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ErrorKind {
    Io,
    Collision,
}

/// `completed` retains a physical step that succeeded before an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IoFailure {
    pub kind: ErrorKind,
    pub completed: bool,
}

pub(super) type IoResult<T> = Result<T, IoFailure>;

/// A move can remove bytes even when a later phase reports an error.
pub(super) struct MoveResult {
    pub removed: Text,
    pub result: IoResult<()>,
}

/// One invocation exposes one publication phase. No filesystem access escapes
/// this seam. The 2b adapter must extend the existing audited primitives.
pub(super) trait HostIo {
    /// Best effort only; it is not a publication or promise.
    fn graph_launch(&mut self, _pages: &std::collections::BTreeSet<String>) {}
    /// Release only this save's unpublished temporary vehicle at completion.
    fn page_finish(&mut self, _page: &str) {}
    fn read_page(&mut self, page: &str) -> IoResult<Text>;
    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()>;
    fn page_rename(&mut self, page: &str) -> IoResult<()>;
    fn page_sync(&mut self, page: &str) -> IoResult<Witness>;
    /// No-replace move of the page's file to the graph trash as `payload`.
    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult;
    /// Custody phases (A4): (a) the payload's data, then (b) the trash
    /// directory and each existing ancestor up to the graph root. A payload
    /// that no longer exists owes nothing (R-PURGE).
    fn trash_sync(&mut self, page: &str, payload: &str) -> IoResult<Witness>;
    /// Strict new-file write of a custody marker into app data.
    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()>;
    /// Unlink a custody marker and sync its app-data directory.
    fn custody_retire(&mut self, name: &str) -> IoResult<()>;
    /// This device's markers, at launch and on progress's retry while the
    /// listing is unknown; never a listing of the trash. Unpublished marker
    /// temps are unlinked first (no move followed them). The error names the
    /// filesystem's failure (REVIEW-2b-r2 V1).
    fn custody_markers(&mut self) -> Result<Vec<(String, Vec<u8>)>, String>;
    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)>;
    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()>;
    fn draft_rename(&mut self, name: &str) -> IoResult<()>;
    fn draft_unlink(&mut self, name: &str) -> IoResult<()>;
    fn draft_sync(&mut self) -> IoResult<Witness>;
    fn quarantine(&mut self, name: &str) -> IoResult<()>;
}
