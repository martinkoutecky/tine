//! Quick Capture's narrow asset import (GH #835). The capture window has no
//! graph slot of its own and, by design, no `GraphContext` access to the
//! generic asset commands (`capture_binding_never_grants_generic_graphcontext_
//! mutation_access`). These two commands resolve the graph its show was bound
//! to instead (window label and binding generation checked in
//! `capture_asset_slot`, like `capture_quick_switch`) and then run the same
//! asset transaction as `import_asset` / `save_asset`. The capture editor
//! writes the returned name into its block text; the graph window appends that
//! text to the journal or page as before (single writer).
use crate::commands::{decode_asset_b64, feature_asset_error};
use crate::state::{capture_asset_slot, AppState, GraphSlot};
use tauri::{State, WebviewWindow};

/// Import the file at `path` into the slot's graph assets (one store-writer
/// transaction). Shared by the graph window's `import_asset` and
/// `capture_import_asset`.
pub(crate) fn import_asset_blocking(
    slot: &GraphSlot,
    path: &str,
    name: Option<&str>,
) -> Result<String, String> {
    crate::device_io::import_asset_from_path(&slot.store, path, name).map_err(|error| match error {
        crate::device_io::DeviceAssetImportError::Name(message) => message,
        crate::device_io::DeviceAssetImportError::Io(error) => feature_asset_error(error, slot),
    })
}

/// Save pasted bytes (base64, bounded) into the slot's graph assets. Shared by
/// `save_asset` and `capture_save_asset`.
pub(crate) fn save_asset_blocking(
    slot: &GraphSlot,
    name: &str,
    bytes_b64: &str,
) -> Result<String, String> {
    let bytes = decode_asset_b64(bytes_b64)?;
    tine_graph_features::assets::save_asset(&slot.store, name, &bytes)
        .map_err(|error| feature_asset_error(error, slot))
}

#[tauri::command]
pub(crate) async fn capture_import_asset(
    path: String,
    name: Option<String>,
    binding_generation: Option<u64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let slot = capture_asset_slot(&state, window.label(), binding_generation)?;
    crate::state::off_ui(move || import_asset_blocking(&slot, &path, name.as_deref())).await
}

#[tauri::command]
pub(crate) async fn capture_save_asset(
    name: String,
    bytes_b64: String,
    binding_generation: Option<u64>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let slot = capture_asset_slot(&state, window.label(), binding_generation)?;
    crate::state::off_ui(move || save_asset_blocking(&slot, &name, &bytes_b64)).await
}
