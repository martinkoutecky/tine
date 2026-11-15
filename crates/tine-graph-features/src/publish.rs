//! The sheet blocks a static export or print hands the app to compute.
//! The export itself is `publish_query` (live app, static site, query export).

use std::io;
use tine_store::Store;

use crate::render::{self, RenderGraph, SheetInput};

/// Which pages the export that will consume these inputs publishes. A query
/// sheet's rows on any other page are never handed to the app, so no cell, count
/// or aggregate can carry them. Print has no page admission boundary.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SheetScope {
    /// `publish_live`: the public pages, or every page when `all_pages`.
    #[serde(rename_all = "camelCase")]
    Live {
        all_pages: bool,
        #[serde(default)]
        include_comments: Option<bool>,
    },
    /// Single-page print, with the same content projection as its renderer.
    #[serde(rename_all = "camelCase")]
    Print {
        #[serde(default)]
        include_comments: Option<bool>,
    },
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
    let include_comments = match scope {
        Some(
            SheetScope::Live {
                include_comments, ..
            }
            | SheetScope::Print { include_comments },
        ) => include_comments.unwrap_or(true),
        Some(SheetScope::Query { request }) => {
            crate::output_comments::included(request.include_comments, &request.properties)
        }
        None => true,
    };
    if let (Some(names), None | Some(SheetScope::Print { .. })) = (pages, scope) {
        // A print's sheets: the named pages only, read fresh as the print
        // reads them, with no graph-wide refresh or corpus (I-13, D-10).
        let Some((whole, mut corpus)) = crate::print::fresh_named_pages(store, names)? else {
            return Ok(Vec::new());
        };
        crate::output_comments::project(&mut corpus.pages, include_comments);
        let mut graph = RenderGraph::for_pages(&corpus, &whole, store, None);
        graph.include_comments = include_comments;
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
    crate::output_comments::project(&mut corpus.pages, include_comments);
    let mut graph = RenderGraph::new(&corpus, &whole, store, None);
    graph.include_comments = include_comments;
    let published = match scope {
        None | Some(SheetScope::Print { .. }) => None,
        Some(SheetScope::Live { all_pages, .. }) => {
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
