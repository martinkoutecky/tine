use super::{PageId, SaveOutcome};

/// Return outcome unchanged when undo succeeded. Incomplete rollback returns
/// Io naming the page and asking the caller to inspect disk before retrying,
/// regardless of the original refusal. No I/O; O(1).
pub(super) fn single_page_failure(
    outcome: SaveOutcome,
    undo_failed: &[usize],
    id: &PageId,
) -> SaveOutcome {
    if undo_failed.is_empty() {
        return outcome;
    }
    SaveOutcome::Io(crate::IoError {
        kind: std::io::ErrorKind::Other,
        message: format!(
            "rollback incomplete for {}: {outcome:?}; inspect disk before retrying",
            id.as_str()
        ),
    })
}
