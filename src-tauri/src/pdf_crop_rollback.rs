//! PDF crop rollback command.

use crate::state::{slot_for_context, GraphContext};

/// Trash a crop in the bound graph only while the current primary sidecar has no
/// reference to it. A whitespace-only sidecar rewrite and crop trash share one
/// revision-guarded transaction.
/// Cost O(sidecar + crop bytes) per attempt, up to four attempts; binding,
/// missing/malformed sidecar, target, I/O and exhausted conflicts reject.
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
