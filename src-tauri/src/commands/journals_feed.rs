//! Journals feed bridge: file-backed bodies and virtual referenced days.
use super::*;

use tine_graph_features::journals::FeedPage;

/// Feed-only pagination by ordinal day. Viewing virtual days writes no files.
#[tauri::command]
pub(crate) async fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    wait_for_references: Option<bool>,
    through_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<FeedPage<PageWire>, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        tine_graph_features::page_read_wire::journal_feed_page(
            PageSource::Live(&slot.store),
            limit,
            before_day,
            through_day,
            wait_for_references.unwrap_or(false),
        )
        .map_err(|error| {
            crate::flight::journal_feed_io_failure(error.kind());
            error.to_string()
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
