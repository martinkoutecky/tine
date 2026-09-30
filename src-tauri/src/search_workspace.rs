//! Closing a query workspace releases its native cancellation lanes. This
//! graph-bound, O(1) operation never writes content; a stale binding is refused.
use crate::state::{slot_for_context, GraphContext};

#[tauri::command]
pub(crate) fn close_search_workspace(
    workspace: String,
    state: GraphContext<'_>,
) -> Result<(), String> {
    slot_for_context(&state)?
        .block_search_lanes
        .close_workspace(&workspace);
    Ok(())
}
