//! PDF crop rollback command.

use crate::state::{slot_for_context, GraphContext};

/// After the caller confirms sidecar refusal and no persisted highlight reference,
/// trash the current crop in the window's bound graph. This does not inspect the
/// sidecar. Cost O(crop bytes) per attempt, up to four attempts; binding, target,
/// I/O, and exhausted conflict failures return error strings.
#[tauri::command]
pub(crate) fn rollback_pdf_area_image(
    pdf: String,
    page: i64,
    id: String,
    stamp: i64,
    state: GraphContext<'_>,
) -> Result<(), String> {
    let slot = slot_for_context(&state)?;
    tine_graph_features::pdf::rollback_pdf_area_image(&slot.store, &pdf, page, &id, stamp)
        .map_err(|error| error.to_string())
}
