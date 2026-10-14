//! Journals feed bridge: file-backed bodies and virtual referenced days.
use super::*;

#[derive(Serialize)]
pub(crate) struct JournalFeedPage {
    pages: Vec<PageWire>,
    upcoming: Vec<PageWire>,
    next_before_day: Option<i64>,
    done: bool,
    as_of_day: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unreadable: Vec<String>,
}

fn feed_wire(read: tine_graph_features::journals::FeedRead) -> PageWire {
    PageWire {
        id: read.id.into(),
        doc: read.doc,
    }
}

/// Feed-only pagination by ordinal day. Viewing virtual days writes no files.
#[tauri::command]
pub(crate) async fn journal_feed_page(
    limit: usize,
    before_day: Option<i64>,
    state: GraphContext<'_>,
) -> Result<JournalFeedPage, String> {
    let slot = slot_for_context(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let feed = tine_graph_features::journals::feed_page(&slot.store, limit, before_day)
            .map_err(|error| {
                crate::flight::journal_feed_io_failure(error.kind());
                error.to_string()
            })?;
        Ok(JournalFeedPage {
            pages: feed.pages.into_iter().map(feed_wire).collect(),
            upcoming: feed.upcoming.into_iter().map(feed_wire).collect(),
            next_before_day: feed.next_before_day,
            done: feed.done,
            as_of_day: feed.as_of_day,
            unreadable: feed.unreadable,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
