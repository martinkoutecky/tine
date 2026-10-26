//! The test-only one-page save oracle over the guarded transaction path. It
//! returns one revision or a refusal; a failed rollback or publication becomes
//! an `Io` refusal naming the page. Existing-page writes can copy O(P) page
//! pointers while a graph view is held. Production page saves are the page
//! host's (STEP3 §12: `Store::save_pages` is deleted, not hidden).

use super::*;

impl Store {
    /// Test oracle (og-surface rule 4): production saves go through the page
    /// host; this one-entry adapter over [`Store::transaction`] exists for
    /// tests only.
    ///
    /// Save one page with a raw-byte [`SaveBase`] guard. Revalidates the
    /// caller-constructible identity, reads current disk bytes, and uses
    /// temporary-file replacement; the temp file is synced before rename and
    /// a create uses a no-clobber rename. An existing-page replacement uses
    /// an ordinary rename after its final revision guard, then syncs the
    /// directory. A directory-sync failure after rename returns an error and
    /// attempts undo: new bytes may have been visible briefly, while a clean
    /// undo restores the starting bytes. Inspect disk if undo is incomplete.
    /// A stale base returns `Conflict` even if the proposed bytes equal current disk
    /// bytes. With a matching base, equal bytes return `Unchanged` without a
    /// publication. A changed save publishes
    /// before returning as its own `Origin::Own` change when the initial load
    /// has not failed. A save begun during parsing may wait for the full parse
    /// while capturing its publication view. The bytes have already been
    /// written and synced to the temporary file and renamed into place before
    /// this wait; directory-sync errors return after attempted undo instead of
    /// entering the wait. A successful save can finish before or after
    /// the separate load-completion event. The save's own generation contains
    /// its write, and a later load-completion view contains it too. `Saved`
    /// returns a file revision, not a graph revision; compare the matching
    /// `Origin::Own` change with a newly acquired view when needed. After a failed
    /// initial load it still writes on a matching guard, but publishes no
    /// generation until a successful `refresh()`. Cost includes reading
    /// and hashing the page and writing its new bytes. Updating an existing
    /// page can copy O(P) in-memory page pointers when a snapshot is held;
    /// creation can additionally walk O(P) file-list metadata for twin checks
    /// and update the name index. It can
    /// wait for graph snapshot capture and other writers without a timeout.
    /// If a caller abandons that wait, it must re-read the file: cancellation
    /// outside this call does not reveal whether the bytes reached disk.
    /// Missing target parent directories are created during apply. A
    /// separate process can still write between the final guard check and
    /// rename. Serialization and temp-file sync precede that final check.
    /// `doc.rev` does not replace `base`; `doc.format`, `doc.name`, and
    /// `doc.title` do not override the target file identity or extension. Only
    /// `doc.pre_block` and the block tree's `raw` and children become page
    /// text; `doc.name`, `kind`, `title`, `format`, and derived block facets do
    /// not inject page properties or select the serializer. The target
    /// extension selects Markdown or Org.
    /// Serialize saves for one editor page, passing each returned `Saved(rev)`
    /// as the next `SaveBase::Existing`; overlapping saves from the same base
    /// can conflict with each other. A concurrent external write overwritten
    /// in the remaining check-to-rename window may never appear as a separate
    /// `Change`.
    /// `CreateNew` checks the exact destination and alternate extension on
    /// disk; other same-name claims use the file-list index built before
    /// `open` returns and updated by later observations, even before parsing
    /// or after a failed parse. A newly
    /// delivered, unobserved journal twin can still be missed. `CreateNew` on
    /// an existing target returns `Conflict` with its disk revision unless
    /// an alternate-extension twin is present; that twin takes precedence.
    /// Other indexed same-name or same-day twins are checked after the exact
    /// target, so an existing exact target wins with `Conflict`. A twin first
    /// found after writing is reported as `Twin`, never as a target revision.
    /// Re-read with
    /// `page(id)` for parsed content or
    /// `read(id.file(), None)` for raw bytes; both revisions hash the same
    /// bytes as `Conflict::disk` if the file has not changed again. That revision is a technically valid new
    /// base, but inspect the current bytes before choosing to overwrite. A
    /// guard conflict does not publish an external change. This call does not preserve a separate
    /// conflict copy of bytes it replaces. A caller choosing "keep mine"
    /// must preserve the other bytes separately if they are needed. Its own observed write is not
    /// republished as an external watcher echo. Keep unsaved edits on every
    /// refusal.
    /// An incomplete rollback or publication returns `Io` naming the page; inspect disk before retrying.
    pub fn save(
        &self,
        kind: crate::EditKind,
        id: &PageId,
        base: SaveBase,
        doc: &PageDto,
    ) -> SaveOutcome {
        if self.is_closed() {
            return SaveOutcome::Closed;
        }
        if doc.guide {
            return SaveOutcome::GuideEphemeral;
        }
        let mut tx = self.transaction(Some(kind));
        tx.save_page(&[kind], id, base, doc);
        match tx.commit() {
            crate::TxOutcome::Committed { mut steps, .. } => match steps.remove(0) {
                crate::StepResult::Written { rev, .. } => SaveOutcome::Saved(rev),
                crate::StepResult::Unchanged { rev, .. } => SaveOutcome::Unchanged(rev),
                _ => unreachable!("save_page result"),
            },
            crate::TxOutcome::NotCommitted {
                why,
                rollback,
                publication_errors,
                ..
            } => {
                let undo_failed: Vec<FileId> = rollback
                    .undo_failed
                    .into_iter()
                    .map(|(file, _)| file)
                    .collect();
                let publication_errors: Vec<FileId> = publication_errors
                    .into_iter()
                    .map(|(file, _)| file)
                    .collect();
                single_page_failure(
                    SaveOutcome::from_failed_step(why, id),
                    &undo_failed,
                    &publication_errors,
                    id,
                )
            }
            crate::TxOutcome::PublicationIncomplete { files, .. } => {
                let files: Vec<FileId> = files.into_iter().map(|(file, _)| file).collect();
                single_page_failure(
                    SaveOutcome::Io(crate::IoError {
                        kind: std::io::ErrorKind::Other,
                        message: "disk steps applied but publication incomplete; inspect disk before retrying".into(),
                    }),
                    &[],
                    &files,
                    id,
                )
            }
        }
    }
}

/// Return outcome unchanged when no undo step or publication failed. Otherwise
/// return Io naming the page and what is incomplete, asking the caller to inspect
/// disk before retrying, regardless of the original refusal. No I/O; O(1).
fn single_page_failure(
    outcome: SaveOutcome,
    undo_failed: &[FileId],
    publication_errors: &[FileId],
    id: &PageId,
) -> SaveOutcome {
    if undo_failed.is_empty() && publication_errors.is_empty() {
        return outcome;
    }
    SaveOutcome::Io(crate::IoError {
        kind: std::io::ErrorKind::Other,
        message: format!(
            "{} for {}: {outcome:?}; inspect disk before retrying",
            match (undo_failed.is_empty(), publication_errors.is_empty()) {
                (false, false) => "rollback and publication incomplete",
                (false, true) => "rollback incomplete",
                (true, false) => "publication incomplete",
                (true, true) => unreachable!(),
            },
            id.as_str(),
        ),
    })
}
