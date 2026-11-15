//! Single-page print document client.

use std::io;
use tine_core::{Corpus, CorpusPage};
use tine_store::{Store, WholeGraph};

use crate::render::{self, RenderGraph, SheetExport, SheetIndex};

/// Options for the single-page print/PDF export.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(default)]
pub struct PrintOpts {
    pub include_comments: bool,
    pub expand_collapsed: bool,
    pub font_px: u32,
    pub margin_mm: u32,
}

impl Default for PrintOpts {
    fn default() -> Self {
        Self {
            include_comments: true,
            expand_collapsed: true,
            font_px: 16,
            margin_mm: 16,
        }
    }
}

/// Render a named page with no frontend: every sheet block stays a plain outline
/// (the named divergence of exports without the app's sheet evaluator).
pub fn page_print_html(store: &Store, name: &str, opts: PrintOpts) -> io::Result<Option<String>> {
    page_print_html_with_sheets(store, name, opts, Vec::new())
}

/// Render a named page using the read-only corpus and bounded asset reads.
/// `sheets` are the app's computed sheets for this page; a sheet block without
/// one keeps its plain outline.
pub fn page_print_html_with_sheets(
    store: &Store,
    name: &str,
    opts: PrintOpts,
    sheets: Vec<SheetExport>,
) -> io::Result<Option<String>> {
    let sheets = SheetIndex::new(sheets);
    let Some((whole, mut corpus)) = fresh_named_pages(store, &[name.to_owned()])? else {
        return Ok(None);
    };
    crate::output_comments::project(&mut corpus.pages, opts.include_comments);
    let mut graph = RenderGraph::for_pages(&corpus, &whole, store, Some(&sheets));
    graph.include_comments = opts.include_comments;
    render::page_print_html(&graph, name, opts)
}

/// The named pages, read fresh, with the view that holds them; `None` when no
/// page has any of these names. I-13: a print is single-page work. Each
/// printed page is read from disk (`Store::page` publishes it if an external
/// editor or sync changed it since the last publication); every other page
/// the render reaches (embeds, block references, queries) comes from the
/// last published view, the one every other read uses, which the watcher
/// keeps current (docs/storage-contract.md). No graph-wide refresh scan and no
/// whole-graph corpus: the pages are found through the view's name table.
/// Cost O(named pages' bytes + claimants x log P).
pub(crate) fn fresh_named_pages(
    store: &Store,
    names: &[String],
) -> io::Result<Option<(WholeGraph, Corpus)>> {
    let load = |store: &Store| {
        store
            .whole_graph()
            .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))
    };
    let mut names: Vec<&String> = names.iter().collect();
    names.sort();
    names.dedup();
    let named = |whole: &WholeGraph| -> Vec<CorpusPage> {
        let mut pages: Vec<CorpusPage> = (names.iter())
            .flat_map(|name| {
                (whole.corpus(Some(name.as_str()), true).pages.into_iter())
                    .filter(move |page| page.name == **name)
            })
            .collect();
        // Name order, page order within a name: what sorting a corpus gives.
        pages.sort_by(|a, b| a.name.cmp(&b.name));
        pages
    };
    let mut whole = load(store)?;
    let pages = named(&whole);
    if pages.is_empty() {
        return Ok(None);
    }
    for page in &pages {
        store.page(&page.id).map_err(crate::store_error)?;
    }
    let fresh = load(store)?;
    let pages = if fresh.rev() != whole.rev() {
        whole = fresh;
        named(&whole)
    } else {
        pages
    };
    Ok(Some((whole, Corpus { pages })))
}
