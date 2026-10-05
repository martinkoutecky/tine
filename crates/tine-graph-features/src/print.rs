//! Single-page print document client.

use std::io;
use tine_store::Store;

use crate::render::{self, RenderGraph, SheetExport, SheetIndex};

/// Options for the single-page print/PDF export.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(default)]
pub struct PrintOpts {
    pub expand_collapsed: bool,
    pub font_px: u32,
    pub margin_mm: u32,
}

impl Default for PrintOpts {
    fn default() -> Self {
        Self {
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
    // I-13: this is a single-page export. The printed page is read fresh
    // (`Store::page` publishes it if it changed); every other page the render
    // reaches (embeds, queries) comes from the last published graph view, the
    // view every other read uses. No graph-wide refresh scan.
    let load = |store: &Store| {
        store
            .whole_graph()
            .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))
    };
    let mut whole = load(store)?;
    let mut corpus = whole.corpus();
    let Some(id) = (corpus.pages.iter())
        .find(|page| page.name == name)
        .map(|page| page.id.clone())
    else {
        return Ok(None);
    };
    store.page(&id).map_err(crate::store_error)?;
    let fresh = load(store)?;
    if fresh.rev() != whole.rev() {
        corpus = fresh.corpus();
        whole = fresh;
    }
    render::page_print_html(
        &RenderGraph::new(&corpus, &whole, store, Some(&sheets)),
        name,
        opts,
    )
}
