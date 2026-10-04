//! Deterministic test fault hooks for graph transactions. Production builds
//! compile `fault` to a constant `false`; the enum stays so call sites need no
//! cfg of their own.

#[cfg(any(test, feature = "test-faults"))]
use std::fs;
#[cfg(any(test, feature = "test-faults"))]
use std::io;
#[cfg(any(test, feature = "test-faults"))]
use std::path::Path;

use crate::store::Store;

#[cfg(any(test, feature = "test-faults"))]
/// Deterministic one-shot failure hooks for tests. Indexed points use a
/// zero-based transaction step index; an unreachable point stays armed in this
/// Store. Multiple distinct points can be armed and each fires once. The two
/// move-abort points require `test-faults` even in a plain unit-test build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaultPoint {
    /// Simulate a changed file at the second revision guard.
    Stage2Mismatch,
    /// Simulate a changed file at the indexed step's second guard.
    Stage2MismatchAt(usize),
    /// Simulate an external deletion at the second revision guard.
    Stage2ExternalDelete,
    /// Remove a copied stream stage after its successful write/revision check.
    RemoveStreamStageAfterWrite,
    /// Simulate an external write after the replacement temp file is synced.
    AfterTempSync,
    /// Simulate a changed but valid sidecar at the second guard.
    Stage2ValidSidecar,
    /// Simulate an external config edit at the second guard.
    Stage2ConfigExternal,
    /// Simulate a no-replace destination collision.
    NoReplaceCollision,
    /// Simulate an I/O error after a step starts.
    MidStepIo,
    /// Simulate a real directory sync error after a rename.
    DirectorySyncIo,
    /// Simulate an I/O error at the indexed step.
    MidStepIoAt(usize),
    /// Simulate a read error while determining the post-apply publication.
    PublicationReadIo,
    /// Simulate an external write while undoing a live file.
    UndoLiveWrite,
    /// Simulate failure to withdraw bytes written by this transaction during undo.
    UndoWithdrawalIo,
    /// Simulate a twin appearing after publication.
    TwinAfterPublish,
    /// Abort the process immediately after the indexed step has reached disk.
    AbortAfterStep(usize),
    /// Abort the process after a rewritten move has renamed and synced the old
    /// bytes at the destination, before its trash copy; requires `test-faults`.
    AbortAfterMoveRename,
    /// Abort with the case-only source safely staged in existing graph trash.
    AbortAfterCaseMoveStage,
    /// Abort after the second case-only move and its directory sync.
    AbortAfterCaseMovePublish,
    /// Fail between the two case-only moves to exercise rollback.
    CaseMoveStageIo,
    /// Create a competing destination between the two case-only moves.
    CaseMoveStageCollision,
    /// Abort the process after a rewritten move has copied the old bytes to
    /// trash and published new destination bytes; requires `test-faults`.
    AbortAfterMoveRewrite,
    /// Fail a rewritten move after its old-byte trash copy, to exercise rollback.
    MoveAfterTrashCopyIo,
    /// Abort the process after a marker resolution staged the pre-resolution
    /// bytes in conflict trash, before the replacement; requires `test-faults`.
    AbortAfterMarkerStage,
    /// Abort the process after undo withdrew a replaced file's new bytes to
    /// conflict trash, before it rewrote the old bytes; requires `test-faults`.
    AbortAfterUndoWithdraw,
}

#[cfg(not(any(test, feature = "test-faults")))]
#[allow(dead_code)] // The production fault check is a no-op.
pub(crate) enum FaultPoint {
    Stage2Mismatch,
    Stage2MismatchAt(usize),
    Stage2ExternalDelete,
    RemoveStreamStageAfterWrite,
    AfterTempSync,
    Stage2ValidSidecar,
    Stage2ConfigExternal,
    NoReplaceCollision,
    MidStepIo,
    DirectorySyncIo,
    MidStepIoAt(usize),
    PublicationReadIo,
    UndoLiveWrite,
    UndoWithdrawalIo,
    TwinAfterPublish,
    AbortAfterStep(usize),
    AbortAfterMoveRename,
    /// Abort with the case-only source safely staged in existing graph trash.
    AbortAfterCaseMoveStage,
    /// Abort after the second case-only move and its directory sync.
    AbortAfterCaseMovePublish,
    /// Fail between the two case-only moves to exercise rollback.
    CaseMoveStageIo,
    /// Create a competing destination between the two case-only moves.
    CaseMoveStageCollision,
    AbortAfterMoveRewrite,
    MoveAfterTrashCopyIo,
    AbortAfterMarkerStage,
    AbortAfterUndoWithdraw,
}

#[cfg(any(test, feature = "test-faults"))]
impl Store {
    /// Arm one deterministic, one-shot fault in this store instance.
    pub fn inject_fault(&self, point: FaultPoint) {
        self.faults.lock().unwrap().insert(point);
    }
}

#[cfg(any(test, feature = "test-faults"))]
pub(super) fn fault(store: &Store, point: FaultPoint) -> bool {
    store.faults.lock().unwrap().remove(&point)
}

#[cfg(not(any(test, feature = "test-faults")))]
pub(super) fn fault(_store: &Store, _point: FaultPoint) -> bool {
    false
}

#[cfg(any(test, feature = "test-faults"))]
pub(super) fn inject_external_delete(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}
