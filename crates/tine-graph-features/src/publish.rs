//! Static site client. The renderer emits bytes through Store's publish protocol.

use std::io;
use tine_store::{IoError, Store};

use crate::render::{self, RenderGraph, SheetInput};

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
    let graph = RenderGraph::new(&corpus, &whole, store, None);
    let mut count = 0;
    let receipt = store
        .publish(tine_store::PublishDest::GraphSite, &mut |writer| {
            count = render::publish_graph(
                &graph,
                render::PageSelection::every_page(config.all_pages_public),
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

/// Which pages the export that will consume these inputs publishes. A query
/// sheet's rows on any other page are never handed to the app, so no cell, count
/// or aggregate can carry them. Print has no boundary and passes no scope.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SheetScope {
    /// `publish_live`: the public pages, or every page when `all_pages`.
    #[serde(rename_all = "camelCase")]
    Live { all_pages: bool },
    /// `publish_query`: the pages the reviewed query selects.
    Query {
        request: crate::publish_query::QueryExportRequest,
    },
}

fn page_keys<'a>(
    pages: impl Iterator<Item = &'a tine_core::CorpusPage>,
) -> std::collections::HashSet<String> {
    pages
        .map(|page| tine_core::refs::page_key(&page.name))
        .collect()
}

/// The sheet blocks of the named pages (all pages when `None`), each with the
/// data the app needs to compute it for a static export. Cost O(blocks of the
/// pages); bounded per `render_sheets` limits.
pub fn sheet_export_inputs(
    store: &Store,
    pages: Option<&[String]>,
    scope: Option<&SheetScope>,
) -> io::Result<Vec<SheetInput>> {
    store
        .scan_refresh()
        .map_err(|error| io::Error::other(format!("graph refresh failed: {error:?}")))?;
    let whole = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?;
    let mut corpus = whole.corpus();
    // Page order comes from directory enumeration, which differs by platform;
    // hand sheets over by page name so the handoff and its MAX_SHEETS cut are stable.
    corpus.pages.sort_by(|a, b| a.name.cmp(&b.name));
    let graph = RenderGraph::new(&corpus, &whole, store, None);
    let published = match scope {
        None => None,
        Some(SheetScope::Live { all_pages }) => {
            let selection = render::PageSelection::every_page(*all_pages);
            Some(page_keys(
                corpus.pages.iter().filter(|page| selection.includes(page)),
            ))
        }
        Some(SheetScope::Query { request }) => {
            Some(crate::publish_query::planned_page_keys(store, request)?)
        }
    };
    Ok(render::sheet_inputs(
        &corpus,
        pages,
        Some(&graph),
        published.as_ref(),
    ))
}
