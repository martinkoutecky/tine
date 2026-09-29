//! Page-save adapters over the guarded transaction path. A single save returns
//! one revision or a refusal; a group save keeps failed rollback and publication
//! file locations for recovery. Existing-page writes can copy O(P) page pointers
//! while a graph view is held. Callers retain unsaved edits on every failure.

use super::*;

impl Store {
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
    /// generation until a successful `scan_refresh()`. Cost includes reading
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
        match self.save_pages(&[(id.clone(), base, doc.clone(), vec![kind])]) {
            SavePagesOutcome::Ok(mut outcomes) => outcomes.remove(0),
            SavePagesOutcome::Failed {
                outcome,
                undo_failed,
                publication_errors,
                ..
            } => single_page_failure(outcome, &undo_failed, &publication_errors, id),
        }
    }

    /// Save page snapshots in input order through one guarded transaction.
    /// Preflight checks all entries before writing; duplicate file IDs, an
    /// empty request, empty edit kinds, and Guide pages refuse. Success returns
    /// one Saved or Unchanged file revision per entry. A preflight refusal writes
    /// nothing. An apply failure attempts undo; inspect `undo_failed` and
    /// `publication_errors` before retrying. If publication fails after all disk
    /// steps, Failed uses index 0 as a placeholder, not a failed entry, and the
    /// writes may stand. Cost includes reading/hashing every guarded page,
    /// writing changed pages, possible undo/final-state reads and writes, and
    /// graph publication that may copy O(P) page pointers.
    pub fn save_pages(
        &self,
        entries: &[(PageId, SaveBase, PageDto, Vec<crate::EditKind>)],
    ) -> SavePagesOutcome {
        if entries.is_empty() {
            return SavePagesOutcome::Failed {
                index: 0,
                outcome: SaveOutcome::InvalidTarget("empty page save".into()),
                undo_failed: Vec::new(),
                publication_errors: Vec::new(),
            };
        }
        if let Some(index) = entries.iter().position(|(_, _, _, kinds)| kinds.is_empty()) {
            return SavePagesOutcome::Failed {
                index,
                outcome: SaveOutcome::InvalidTarget(
                    "OG-RULES Rule 8: page save needs a kind".into(),
                ),
                undo_failed: Vec::new(),
                publication_errors: Vec::new(),
            };
        }
        if self.is_closed() {
            return SavePagesOutcome::Failed {
                index: 0,
                outcome: SaveOutcome::Closed,
                undo_failed: Vec::new(),
                publication_errors: Vec::new(),
            };
        }
        if let Some(index) = entries.iter().position(|(_, _, doc, _)| doc.guide) {
            return SavePagesOutcome::Failed {
                index,
                outcome: SaveOutcome::GuideEphemeral,
                undo_failed: Vec::new(),
                publication_errors: Vec::new(),
            };
        }
        let mut tx = self.transaction(Some(entries[0].3[0]));
        for (id, base, doc, kinds) in entries {
            tx.save_page(kinds, id, base.clone(), doc);
        }
        match tx.commit() {
            crate::TxOutcome::Committed { steps, .. } => SavePagesOutcome::Ok(
                steps
                    .into_iter()
                    .map(|step| match step {
                        crate::StepResult::Written { rev, .. } => SaveOutcome::Saved(rev),
                        crate::StepResult::Unchanged { rev, .. } => SaveOutcome::Unchanged(rev),
                        _ => unreachable!("save_page result"),
                    })
                    .collect(),
            ),
            crate::TxOutcome::NotCommitted {
                step,
                why,
                rollback,
                publication_errors,
                ..
            } => {
                let undo_failed = rollback.undo_failed.into_iter().map(|(file, _)| file).collect();
                SavePagesOutcome::Failed {
                    index: step,
                    outcome: SaveOutcome::from_failed_step(why, &entries[step].0),
                    undo_failed,
                    publication_errors: publication_errors.into_iter().map(|(file, _)| file).collect(),
                }
            }
            crate::TxOutcome::PublicationIncomplete { files, .. } => SavePagesOutcome::Failed {
                index: 0,
                outcome: SaveOutcome::Io(crate::IoError {
                    kind: std::io::ErrorKind::Other,
                    message: "disk steps applied but publication incomplete; inspect disk before retrying".into(),
                    operation: None,
                    os_error: None,
                }),
                undo_failed: Vec::new(),
                publication_errors: files.into_iter().map(|(file, _)| file).collect(),
            },
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
        operation: None,
        os_error: None,
    })
}
