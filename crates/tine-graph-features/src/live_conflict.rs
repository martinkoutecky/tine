//! Concord live-draft conflicts (og family 8e): an editor draft whose ordinary
//! guarded save was refused because the file changed on disk. The review
//! compares the draft ("mine") with the file as it is NOW ("theirs"); the
//! resolve recomputes the same comparison and writes the chosen result through
//! one guarded store transaction. Nothing here is persisted; the draft itself
//! lives in the editor, or in the app-private draft store across a restart.
//!
//! Base: the Concord ledger's retained text whose revision equals the draft's
//! `base_rev` (the bytes the editor loaded or last saved). With it the review is
//! 3-way and its rows carry suggestions; without it (evicted, unreadable, never
//! recorded) it is 2-way and nothing is pre-selected. Never an authority: only
//! a `"merged"` decision reads the base at resolve time.
//!
//! A missing file is its own disk revision, `"absent"`: the resolve then
//! creates the file and refuses when anything (even an empty file) appeared.

use std::collections::HashMap;
use std::io;

use sha2::{Digest, Sha256};
use tine_core::doc::Document;
use tine_core::model::{Format, PageDto};
use tine_core::projection::page_dto_document;
use tine_core::sync_diff::{self, SyncConflictDiff};
use tine_store::{EditKind, FileId, FileRev, SaveBase, Store};

use crate::conflicts::{choose_pre, dto, format, id, invalid_path, merge_refused, parse, read_text};

/// The disk revision of a missing file.
pub const ABSENT: &str = "absent";

fn changed_on_disk() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "live conflict changed on disk")
}

/// The file as it is now: its text and revision, or `None` when it is absent.
fn disk(store: &Store, file: &FileId) -> io::Result<Option<(String, FileRev)>> {
    match read_text(store, file) {
        Ok(read) => Ok(Some(read)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn disk_rev(read: &Option<(String, FileRev)>) -> String {
    read.as_ref()
        .map_or_else(|| ABSENT.to_owned(), |(_, rev)| rev.clone().into())
}

/// The retained text the draft was edited from, and its identity token
/// (sha256 hex). `None` when no candidate has the draft's revision.
fn base_for<'a>(bases: &'a [String], base_rev: Option<&str>) -> Option<(&'a str, String)> {
    let wanted = base_rev?;
    let base = bases
        .iter()
        .find(|text| String::from(FileRev::from_bytes(text.as_bytes())) == wanted)?;
    Some((base.as_str(), format!("{:x}", Sha256::digest(base.as_bytes()))))
}

fn sides(path: &FileId, draft: &PageDto, now: &Option<(String, FileRev)>) -> (Format, Document, Document) {
    let fmt = format(path);
    let mine = page_dto_document(draft, fmt == Format::Org);
    let theirs = now.as_ref().map_or_else(
        || Document {
            pre_block: None,
            roots: Vec::new(),
        },
        |(text, _)| parse(text, fmt),
    );
    (fmt, mine, theirs)
}

/// Review `draft` (the editor's page, loaded at `base_rev`) against the file at
/// `path` as it is now. `bases` are the ledger's retained texts of that page.
/// `conflict_rev` is the disk revision the review shows (or [`ABSENT`]); the
/// resolve requires it back. Read-only. Cost O(file + draft + base bytes).
pub fn live_conflict_diff(
    store: &Store,
    path: &str,
    draft: &PageDto,
    base_rev: Option<&str>,
    bases: &[String],
) -> io::Result<SyncConflictDiff> {
    let file = id(store, path)?;
    let now = disk(store, &file)?;
    let (fmt, mine, theirs) = sides(&file, draft, &now);
    let mut diff = match base_for(bases, base_rev) {
        Some((base, token)) => {
            let mut diff = sync_diff::diff3_docs(&parse(base, fmt), &mine, &theirs);
            diff.merge_base_rev = Some(token);
            diff
        }
        None => sync_diff::diff_docs(&mine, &theirs),
    };
    diff.base_rev = base_rev.unwrap_or_default().to_owned();
    diff.conflict_rev = disk_rev(&now);
    Ok(diff)
}

/// Apply the user's per-row `decisions` for the reviewed live conflict and
/// write the result in one guarded transaction: `SaveBase::Existing` at the
/// reviewed disk revision, or `CreateNew` when the review showed the file
/// absent. Returns the written page with its new revision, which the editor
/// installs. Refusals (each writes nothing):
/// - the disk moved since the review (`live conflict changed on disk`,
///   scenario: an external-editor race, sync-service delivery, or an honest
///   concurrent instance writing after the review) — the UI refreshes the
///   review;
/// - a `"merged"` row whose reviewed base is gone or different (`merge base
///   changed since the review`, same scenarios moving the ledger);
/// - an Org file on disk that does not round-trip (malformed imported content).
///
/// Cost O(file + draft + base bytes) plus one page commit.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_conflict(
    store: &Store,
    path: &str,
    draft: &PageDto,
    base_rev: Option<&str>,
    conflict_rev: &str,
    merge_base_rev: Option<&str>,
    bases: &[String],
    decisions: &HashMap<String, String>,
    pre_choice: &str,
) -> io::Result<PageDto> {
    let file = id(store, path)?;
    let page = store.as_page(&file).ok_or_else(invalid_path)?;
    let now = disk(store, &file)?;
    if disk_rev(&now) != conflict_rev {
        return Err(changed_on_disk());
    }
    if let Some((text, _)) = &now {
        if format(&file) == Format::Org && !tine_core::org::org_editable(text) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the org file does not round-trip; not merging",
            ));
        }
    }
    let (fmt, mine, theirs) = sides(&file, draft, &now);
    let base_doc = match (merge_base_rev, decisions.values().any(|d| d == "merged")) {
        (Some(token), true) => match base_for(bases, base_rev) {
            Some((base, current)) if current == token => Some(parse(base, fmt)),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "merge base changed since the review",
                ))
            }
        },
        _ => None,
    };
    let roots = sync_diff::merge_blocks3(
        base_doc.as_ref().map(|doc| doc.roots.as_slice()),
        &mine.roots,
        &theirs.roots,
        None,
        decisions,
    )
    .map_err(merge_refused)?;
    let pre_block = choose_pre(pre_choice, fmt, &mine, &theirs);
    let mut merged = dto(store, &page, Document { pre_block, roots });
    let (kind, base) = match &now {
        Some((_, rev)) => (EditKind::ReplacePage, SaveBase::Existing(rev.clone())),
        None => (EditKind::CreatePage, SaveBase::CreateNew),
    };
    let mut tx = store.transaction(Some(kind));
    tx.save_page(&[kind], &page, base, &merged);
    let outcome = tx.commit();
    // A revision guard that fails at commit is the same race as a moved
    // revision before it: refresh the review, never retry against unseen bytes.
    if crate::is_conflict(&outcome) {
        return Err(changed_on_disk());
    }
    let rev = crate::tx_error(outcome)?
        .into_iter()
        .find_map(|step| match step {
            tine_store::StepResult::Written { rev, .. }
            | tine_store::StepResult::Unchanged { rev, .. } => Some(rev),
            _ => None,
        })
        .ok_or_else(|| io::Error::other("the resolved page was not written"))?;
    merged.rev = Some(rev.into());
    Ok(merged)
}
