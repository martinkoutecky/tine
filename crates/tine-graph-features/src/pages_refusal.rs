//! Typed context for a merge refusal, and the ordinary unchanged save used by
//! the interface's explicit save-then-rename helper (D-20261116-P3H).
use super::*;

/// Pre-write refusals shared by name renames and physical file merges.
/// The file context and legal next step survive both window and CLI boundaries;
/// formatting costs O(message bytes) and performs no I/O.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RewriteBlocked {
    /// O3: the confirmed survivor no longer owns the requested name.
    StaleMerge { file: String },
    /// Imported or synced content exceeds the reader's safe domain.
    Unindexed { file: String },
    /// O4: the Org save cannot preserve this file's content.
    ReadOnlyOrg { file: String },
    /// O4: a merge pair must use one format.
    DifferentFormats { source: String, target: String },
    /// O4: neither merge member may contain conflict markers.
    MergeConflictMarkers { file: String },
    /// File merge needs two different physical files.
    SameFile { file: String },
    /// External changes invalidated the source selection.
    StaleSource { file: String },
    /// O4: more than one physical file claims the selected source page.
    DuplicateFiles { files: Vec<String> },
}

impl RewriteBlocked {
    pub(super) fn error(self) -> io::Error {
        let kind = match self {
            Self::DuplicateFiles { .. } => io::ErrorKind::AlreadyExists,
            Self::DifferentFormats { .. } | Self::SameFile { .. } => io::ErrorKind::InvalidInput,
            Self::Unindexed { .. } | Self::MergeConflictMarkers { .. } => {
                io::ErrorKind::InvalidData
            }
            Self::StaleMerge { .. } | Self::StaleSource { .. } => io::ErrorKind::NotFound,
            Self::ReadOnlyOrg { .. } => io::ErrorKind::PermissionDenied,
        };
        io::Error::new(kind, self)
    }

    /// This boundary's refusal before renaming or merging files.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref().and_then(|cause| cause.downcast_ref())
    }
}

impl std::fmt::Display for RewriteBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleMerge { file } => write!(f, "cannot merge: {file} no longer holds the name you chose; start the rename again to choose the remaining page"),
            Self::Unindexed { file } => write!(f, "cannot rename or merge: Tine cannot read {file} safely; repair its encoding or simplify the file in another editor, then try again"),
            Self::ReadOnlyOrg { file } => write!(f, "cannot rename or merge: Tine cannot save {file} without changing its content; edit it in another Org editor or move the notes into a new page, then try again"),
            Self::DifferentFormats { source, target } => write!(f, "cannot merge: {source} and {target} use different formats; move the notes into pages with the same format, then try again"),
            Self::MergeConflictMarkers { file } => write!(f, "cannot merge: resolve the conflict markers in {file}, then try again; nothing was merged"),
            Self::SameFile { file } => write!(f, "cannot merge: {file} is both the source and the remaining file; choose two different files"),
            Self::StaleSource { file } => write!(f, "cannot rename: {file} no longer identifies the page you chose; reopen the page and start the rename again"),
            Self::DuplicateFiles { files } => write!(f, "cannot rename: these files claim the same page: {}; merge one file into the other or remove one, then try again", files.join(", ")),
        }
    }
}
impl std::error::Error for RewriteBlocked {}

/// The file whose rename or merge was refused, retaining the writer's
/// reason for diagnostics. Ordinary limitations have their own explanations;
/// internal writer faults use the P2 explanation.
#[derive(Debug)]
pub struct MergeRefused {
    pub file: String,
    pub reason: merge_payload::RealizationError,
}

impl MergeRefused {
    pub(crate) fn trace(reason: tine_core::text_trace::TraceError, file: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            Self {
                file: file.into(),
                reason: merge_payload::RealizationError::Trace(reason),
            },
        )
    }

    pub(super) fn error(
        failure: merge_payload::RealizationFailure,
        src: &PageId,
        dst: &PageId,
    ) -> io::Error {
        let file = match failure.part {
            merge_payload::MergePart::Source => src,
            merge_payload::MergePart::Survivor | merge_payload::MergePart::Output => dst,
        };
        io::Error::new(
            io::ErrorKind::InvalidData,
            Self {
                file: file.as_str().into(),
                reason: failure.reason,
            },
        )
    }

    /// A refusal before merging files; preparation may have saved pending edits.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref().and_then(|cause| cause.downcast_ref())
    }

    /// An internal writer fault; ordinary limitations and model refusals have
    /// their own explanations.
    /// O(file path bytes), no I/O; names the input the user can inspect.
    pub fn check_refused(&self) -> Option<CheckRefused> {
        use merge_payload::RealizationError as M;
        match self.reason {
            M::HeaderChildren
            | M::HiddenOccurrence
            | M::SurvivorIdentity
            | M::BodyLaw
            | M::IndentedBody
            | M::DirectiveEscape
            | M::MissingNativeNameRange { .. }
            | M::Trace(tine_core::text_trace::TraceError::TitleIdentity) => None,
            M::Trace(_) | M::FusedLineEnding => Some(CheckRefused {
                guarantee: tine_store::Guarantee::P2,
                file: self.file.clone(),
            }),
        }
    }
}

impl std::fmt::Display for MergeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(check) = self.check_refused() {
            return check.fmt(f);
        }
        use merge_payload::RealizationError as M;
        let file = &self.file;
        match &self.reason {
            M::HeaderChildren => write!(f, "cannot merge: blocks under the header in {file} would lose their parent; move them out from under the header and try again; nothing was merged"),
            M::HiddenOccurrence => write!(f, "cannot merge: references in the page properties of {file} would stop working in the merged page; move those references into ordinary blocks and try again; nothing was merged"),
            M::SurvivorIdentity => write!(f, "cannot merge: the title in the first block of {file} would give the remaining page a different name; save that file once with its intended page title and try again; nothing was merged"),
            M::Trace(tine_core::text_trace::TraceError::TitleIdentity) => write!(f, "cannot rename or merge: changing the references inside the title of {file} would give the page a different name; edit that title so its references do not name the page being renamed, then try again; nothing was renamed or merged"),
            M::BodyLaw => write!(f, "cannot merge: Tine cannot preserve the block structure of {file} when joining the files; align the top-level blocks with the left edge in that file, save it and try again; nothing was merged"),
            M::IndentedBody => write!(f, "cannot merge: the first block of {file} is indented, so Tine cannot keep its blocks at the top level when joining the files; align the top-level blocks with the left edge in that file, save it and try again; nothing was merged"),
            M::DirectiveEscape => write!(f, "cannot merge: {file} contains a #+END_QUOTE line outside a quote, which would end the merged page's quote early; move that line into a quote or remove it in that file, then try again; nothing was merged"),
            M::MissingNativeNameRange { declaration } => write!(f, "cannot merge: Tine cannot move the alias declaration “{declaration}” in {file} without losing its spelling; replace that declaration with plain alias names separated by commas in that file, save it and try again; nothing was merged"),
            _ => unreachable!("writer faults use the check explanation"),
        }
    }
}
impl std::error::Error for MergeRefused {}

/// Refuse unsaved input only on pages the rename would rewrite.
pub(super) fn unsaved_error(graph: &tine_store::WholeGraph, path: &str) -> io::Error {
    let inventory = graph.inventory(tine_store::InventoryScope::All);
    let name = inventory
        .0
        .iter()
        .find(|entry| physical(&entry.target).iter().any(|id| id.as_str() == path))
        .map_or(path, |entry| entry.name.as_str());
    error(
        io::ErrorKind::WouldBlock,
        &format!(
            "“{name}” has changes Tine could not save, and this rename would rewrite it. \
             Save or discard those changes, then rename again."
        ),
    )
}

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
