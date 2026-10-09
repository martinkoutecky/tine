//! Phase seam; production adapters belong to step 2b.
use super::Text;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Phase {
    Read,
    PageTemp,
    PageRename,
    PageSync,
    TrashMove,
    TrashSync,
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
    fn read_page(&mut self, page: &str) -> IoResult<Text>;
    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()>;
    fn page_rename(&mut self, page: &str) -> IoResult<()>;
    fn page_sync(&mut self, page: &str) -> IoResult<Witness>;
    fn trash_move(&mut self, page: &str, name: &str) -> MoveResult;
    fn trash_sync(&mut self, page: &str) -> IoResult<Witness>;
    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)>;
    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()>;
    fn draft_rename(&mut self, name: &str) -> IoResult<()>;
    fn draft_unlink(&mut self, name: &str) -> IoResult<()>;
    fn draft_sync(&mut self) -> IoResult<Witness>;
    fn quarantine(&mut self, name: &str) -> IoResult<()>;
}
