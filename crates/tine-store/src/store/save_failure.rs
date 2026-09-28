use super::{PageId, SaveOutcome};

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
