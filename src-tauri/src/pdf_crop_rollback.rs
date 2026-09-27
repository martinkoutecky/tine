//! Forward a crop rollback after the caller confirms sidecar refusal and no
//! persisted highlight reference. One bound
//! graph lookup plus O(crop bytes) per Store attempt; binding or Store refusal
//! returns an error string. The command does not inspect the sidecar.

use crate::state::{slot_for_context, GraphContext};

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
