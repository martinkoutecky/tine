//! The sheet blocks a static export or print hands the app to compute.
//! The export itself is `publish_query` (live app, static site, query export).

use std::io;
use tine_store::Store;

use crate::render::{self, RenderGraph, SheetInput};

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
/// data the app needs to compute it for a static export; bounded per
/// `render_sheets` limits. A print (names, no scope) reads only the named pages
/// (fresh) and resolves the rest by key from the published view: cost O(those
/// pages + what they embed). Publish scopes refresh and copy the corpus.
pub fn sheet_export_inputs(
    store: &Store,
    pages: Option<&[String]>,
    scope: Option<&SheetScope>,
) -> io::Result<Vec<SheetInput>> {
    if let (Some(names), None) = (pages, scope) {
        // A print's sheets: the named pages only, read fresh as the print
        // reads them, with no graph-wide refresh or corpus (I-13, D-10).
        let Some((whole, corpus)) = crate::print::fresh_named_pages(store, names)? else {
            return Ok(Vec::new());
        };
        let graph = RenderGraph::for_pages(&corpus, &whole, store, None);
        return Ok(render::sheet_inputs(&corpus, pages, Some(&graph), None));
    }
    store
        .refresh(tine_store::Depth::Stamps)
        .map_err(|error| io::Error::other(format!("graph refresh failed: {error:?}")))?;
    let whole = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?;
    let mut corpus = whole.corpus(None, true);
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
