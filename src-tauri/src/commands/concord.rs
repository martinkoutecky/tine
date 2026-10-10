//! Concord (og family 8) IPC: sync-copy and VCS-marker conflicts and the
//! derived conflict queue. Every write goes through the feature clients in
//! `tine_graph_features::conflicts`, which commit one guarded store
//! transaction; nothing here touches a file.

use crate::commands::sync_conflict_error;
use crate::state::{slot_for_context, GraphContext, GraphSlot};

/// Candidate common ancestors for one copy from the Concord base ledger;
/// empty (2-way review) when there is no ledger or it cannot answer.
fn ledger_bases(slot: &GraphSlot, winner: &str, conflict: &str) -> Vec<String> {
    slot.concord_ledger
        .get()
        .map(|ledger| ledger.conflict_bases(conflict, winner))
        .unwrap_or_default()
}

/// A settled resolve or trash re-derives its two queue entries before the
/// command returns, so the next inventory cannot resurrect them.
fn settle_queue(slot: &GraphSlot, paths: &[&str]) {
    let files: Vec<_> = paths
        .iter()
        .map(|path| tine_store::FileId::from((*path).to_owned()))
        .collect();
    if let Err(error) = slot.conflict_queue.refresh_files(&slot.store, &files) {
        crate::debug::diag_private("conflict-refresh-failed", error.to_string());
    }
}

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) async fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::list_sync_conflicts(&slot.store)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI; 3-way with
/// suggestions when the Concord base ledger holds an ancestor. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    let slot = slot_for_context(&state)?;
    let bases = ledger_bases(&slot, &winner, &conflict);
    tine_graph_features::conflicts::sync_conflict_diff(&slot.store, &winner, &conflict, &bases)
        .map_err(|e| e.to_string())
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both"/"merged") via the normal save
/// path, then trash the conflict copy. `base_rev` guards against the winner
/// changing under the merge; returns "conflict" if it did. `merge_base_rev` is
/// the reviewed diff's ledger-base token (needed only by "merged" rows).
/// `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    merge_base_rev: Option<String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    // The resolution takes the store writer and the ledger fsyncs (R3).
    crate::state::off_ui(move || {
        let bases = if merge_base_rev.is_some() {
            ledger_bases(&slot, &winner, &conflict)
        } else {
            Vec::new()
        };
        let outcome = tine_graph_features::conflicts::resolve_sync_conflict(
            &slot.store,
            slot.host_slot().running(),
            &winner,
            &conflict,
            &decisions,
            &base_rev,
            &conflict_rev,
            merge_base_rev.as_deref(),
            &bases,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(sync_conflict_error);
        settle_queue(&slot, &[&winner, &conflict]);
        outcome
    })
    .await
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) async fn trash_sync_conflict(
    conflict: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    crate::state::off_ui(move || {
        let outcome = tine_graph_features::conflicts::trash_sync_conflict(
            &slot.store,
            slot.host_slot().running(),
            &conflict,
        )
        .map_err(|e| e.to_string());
        settle_queue(&slot, &[&conflict]);
        outcome
    })
    .await
}

/// Two-way diff of a duplicate journal day's canonical file against one stray
/// (master 9dc54e4a7); `None` for a cross-format pair, which the UI shows as
/// file rows without row choices. Read-only.
#[tauri::command]
pub(crate) async fn duplicate_journal_diff(
    canonical: String,
    stray: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::duplicate_journal_diff(&slot.store, &canonical, &stray)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Fold one stray of a duplicate journal day into the day's canonical file per
/// the reviewed row decisions and trash the stray recoverably. Guarded to two
/// files of ONE duplicate day; "conflict" when either file changed since the
/// review (nothing written).
#[tauri::command]
pub(crate) async fn resolve_duplicate_journal_day(
    canonical: String,
    stray: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    stray_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = tine_graph_features::conflicts::resolve_duplicate_journal_day(
            &slot.store,
            slot.host_slot().running(),
            &canonical,
            &stray,
            &decisions,
            &base_rev,
            &stray_rev,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(sync_conflict_error);
        settle_queue(&slot, &[&canonical, &stray]);
        outcome
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The derived conflict queue: sync-tool copies paired with their winner and
/// marker-bearing pages (memory only, never stored). The first call per graph
/// walks every page file, so it runs on the blocking pool; later calls answer
/// from the queue the change feed keeps current.
#[tauri::command]
pub(crate) async fn conflict_inventory(
    state: GraphContext<'_>,
) -> Result<tine_core::concord_queue::ConflictInventory, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        slot.conflict_queue
            .inventory(&slot.store)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Block diff of a marker-bearing page's own sides (3-way when the markers
/// carry a common ancestor). Read-only; `None` when the page has no markers.
#[tauri::command]
pub(crate) async fn vcs_marker_conflict_diff(
    path: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::concord_queue::MarkerConflictDiff>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::vcs_marker_conflict_diff(&slot.store, &path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Resolve a marker-bearing page per the user's per-row decisions. `base_rev`
/// is the marker file's rev from the diff; returns "conflict" if the file
/// changed since, and writes nothing.
#[tauri::command]
pub(crate) async fn resolve_vcs_marker_conflict(
    path: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = tine_graph_features::conflicts::resolve_vcs_marker_conflict(
            &slot.store,
            slot.host_slot().running(),
            &path,
            &decisions,
            &base_rev,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(sync_conflict_error);
        settle_queue(&slot, &[&path]);
        outcome
    })
    .await
    .map_err(|error| error.to_string())?
}

/// The Concord ledger's retained texts of one page: the candidates a live-draft
/// review looks up its base in (by the draft's revision). Empty when there is
/// no ledger or it cannot answer (2-way review).
pub(crate) fn page_bases(slot: &GraphSlot, path: &str) -> Vec<String> {
    slot.concord_ledger
        .get()
        .map(|ledger| ledger.page_bases(path))
        .unwrap_or_default()
}

/// Review a live editor draft whose guarded save was refused against the file
/// at `path` as it is now (og 8e). `base_rev` is the revision the editor
/// loaded; the ledger text with that revision makes the review 3-way. The
/// answer's `conflict_rev` is the disk revision shown (`"absent"` for a missing
/// file). Read-only.
#[tauri::command]
pub(crate) async fn live_conflict_diff(
    path: String,
    page: tine_core::model::PageDto,
    base_rev: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::sync_diff::SyncConflictDiff, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bases = page_bases(&slot, &path);
        tine_graph_features::live_conflict::live_conflict_diff(
            &slot.store,
            &path,
            &page,
            base_rev.as_deref(),
            &bases,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Resolve a reviewed live-draft conflict: recompute the review from the same
/// draft and the disk at `conflict_rev`, apply `decisions`, and write the page
/// in one guarded transaction. Returns the written page (with its revision)
/// for the editor to install; "conflict" when the disk or the reviewed base
/// moved since the review (nothing written).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_live_conflict(
    path: String,
    page: tine_core::model::PageDto,
    base_rev: Option<String>,
    conflict_rev: String,
    merge_base_rev: Option<String>,
    decisions: std::collections::HashMap<String, String>,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<tine_core::model::PageDto, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bases = if merge_base_rev.is_some() {
            page_bases(&slot, &path)
        } else {
            Vec::new()
        };
        tine_graph_features::live_conflict::resolve_live_conflict(
            &slot.store,
            slot.host_slot().running(),
            &path,
            &page,
            base_rev.as_deref(),
            &conflict_rev,
            merge_base_rev.as_deref(),
            &bases,
            &decisions,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(sync_conflict_error)
    })
    .await
    .map_err(|error| error.to_string())?
}
