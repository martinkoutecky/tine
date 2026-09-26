//! Static site client. The renderer emits bytes through Store's publish protocol.

use std::io;
use tine_store::{IoError, Store};

use crate::render::{self, RenderGraph};

/// Export public pages and return the published folder and page count.
pub fn publish_html(store: &Store) -> io::Result<(String, usize)> {
    let whole = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?;
    for file in whole.parsed_page_ids() {
        store.page(&file).map_err(crate::store_error)?;
    }
    store
        .scan_refresh()
        .map_err(|error| io::Error::other(format!("graph refresh failed: {error:?}")))?;
    let whole = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?;
    let corpus = whole.corpus();
    let config = store.config();
    let graph = RenderGraph {
        corpus: &corpus,
        whole: &whole,
        store,
    };
    let mut count = 0;
    let receipt = store
        .publish_site(&mut |writer| {
            count = render::publish_graph(
                &graph,
                config.all_pages_public,
                &config.favorites,
                &mut |name, bytes| {
                    writer
                        .write(name, bytes)
                        .map_err(|error| io::Error::new(error.kind, error.message))
                },
            )
            .map_err(IoError::from)?;
            Ok(())
        })
        .map_err(|failed| io::Error::new(failed.cause.kind, failed.cause.message))?;
    Ok((receipt.site.display().to_string(), count))
}
