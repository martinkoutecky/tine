//! Concord (og family 8) IPC: sync-copy and VCS-marker conflicts and the
//! derived conflict queue. Every write goes through the feature clients in
//! `tine_graph_features::conflicts`, which commit one guarded store
//! transaction; nothing here touches a file.

use crate::commands::sync_conflict_error;
use crate::state::{slot_for_context, GraphContext};

/// Sync-tool conflict copies (Syncthing/Dropbox) sitting in the graph — for the
/// user to review + reconcile instead of them showing as garbage pages.
#[tauri::command]
pub(crate) async fn list_sync_conflicts(
    state: GraphContext<'_>,
) -> Result<Vec<tine_core::model::SyncConflict>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::list_sync_conflicts(&slot.store)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Block-level diff of a sync-conflict copy against its winner (both graph-root-
/// relative paths) — the data behind the two-column merge UI. Read-only.
#[tauri::command]
pub(crate) fn sync_conflict_diff(
    winner: String,
    conflict: String,
    state: GraphContext<'_>,
) -> Result<Option<tine_core::sync_diff::SyncConflictDiff>, String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::sync_conflict_diff(&slot.store, &winner, &conflict)
        .map_err(|e| e.to_string())
}

/// Resolve a sync-conflict copy: merge it into its winner per the user's per-row
/// `decisions` (row id → "mine"/"theirs"/"both") via the normal save path, then
/// trash the conflict copy. `base_rev` guards against the winner changing under
/// the merge; returns "conflict" if it did. `pre_choice`: "mine"/"theirs"/"union".
#[tauri::command]
pub(crate) fn resolve_sync_conflict(
    winner: String,
    conflict: String,
    decisions: std::collections::HashMap<String, String>,
    base_rev: String,
    conflict_rev: String,
    pre_choice: Option<String>,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::resolve_sync_conflict(
        &slot.store,
        &winner,
        &conflict,
        &decisions,
        &base_rev,
        &conflict_rev,
        pre_choice.as_deref().unwrap_or("union"),
    )
    .map_err(sync_conflict_error)
}

/// Discard a sync-conflict copy without merging (move it to the recoverable
/// trash). Refuses anything that isn't a conflict copy.
#[tauri::command]
pub(crate) fn trash_sync_conflict(conflict: String, state: GraphContext<'_>) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::conflicts::trash_sync_conflict(&slot.store, &conflict)
        .map_err(|e| e.to_string())
}

/// The derived conflict queue: sync-tool copies paired with their winner and
/// marker-bearing pages, recomputed from disk on every call (never stored).
/// Reads every page file, so it runs on the blocking pool.
#[tauri::command]
pub(crate) async fn conflict_inventory(
    state: GraphContext<'_>,
) -> Result<tine_core::concord_queue::ConflictInventory, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::conflicts::conflict_inventory(&slot.store)
    })
    .await
    .map_err(|error| error.to_string())
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
        tine_graph_features::conflicts::resolve_vcs_marker_conflict(
            &slot.store,
            &path,
            &decisions,
            &base_rev,
            pre_choice.as_deref().unwrap_or("union"),
        )
        .map_err(sync_conflict_error)
    })
    .await
    .map_err(|error| error.to_string())?
}
