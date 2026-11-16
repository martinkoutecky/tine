//! Typed context for a merge refusal, and the ordinary unchanged save used by
//! the interface's explicit save-then-rename helper (D-20261116-P3H).
use super::*;

/// The file whose merge representation was refused. The writer's reason is
/// preserved as data, including reasons which still use backend wording.
#[derive(Debug)]
pub struct MergeRefused {
    pub file: String,
    pub reason: merge_payload::RealizationError,
}

impl MergeRefused {
    pub(super) fn error(
        reason: merge_payload::RealizationError,
        src: &PageId,
        dst: &PageId,
    ) -> io::Error {
        if !matches!(
            reason,
            merge_payload::RealizationError::HeaderChildren
                | merge_payload::RealizationError::HiddenOccurrence
                | merge_payload::RealizationError::SurvivorIdentity
        ) {
            return io::Error::new(io::ErrorKind::InvalidData, reason);
        }
        let file = if reason == merge_payload::RealizationError::SurvivorIdentity {
            dst
        } else {
            src
        };
        io::Error::new(
            io::ErrorKind::InvalidData,
            Self {
                file: file.as_str().into(),
                reason,
            },
        )
    }

    /// A merge's refusal before any write, if this error carries one.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref().and_then(|cause| cause.downcast_ref())
    }
}

impl std::fmt::Display for MergeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.reason)
    }
}
impl std::error::Error for MergeRefused {}

/// Save exactly `path` without an edit, using the normal save preparation and
/// the page host's guarded publication. Pending input is flushed first; the
/// saved revision is read afterwards and guards the transaction. A failure
/// prevents the caller's subsequent rename. Cost: one page read and one save,
/// plus the host's flush of pending input on that page; no graph walk.
pub fn save_page_unchanged(store: &Store, host: &PageHost, path: &str) -> io::Result<()> {
    let file = text_file(store, path)?;
    let page = store
        .as_page(&file)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid page path"))?;
    crate::retained::operation(
        host,
        Input::Flush,
        std::slice::from_ref(&page),
        "the save",
        "page changed repeatedly during save",
        crate::retained::unsaved,
        || {
            let parsed = store.page(&page).map_err(store_error)?;
            let mut tx = store.page_plan(Some(tine_store::EditKind::SaveBlock));
            tx.save_page(
                &[tine_store::EditKind::SaveBlock],
                &page,
                tine_store::SaveBase::Existing(parsed.rev),
                &parsed.doc,
            );
            crate::retained::plan(tx).map(Some)
        },
    )
}
