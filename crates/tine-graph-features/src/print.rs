//! Single-page print document client.

use std::io;
use tine_core::doc;
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
    store
        .scan_refresh()
        .map_err(|error| io::Error::other(format!("graph refresh failed: {error:?}")))?;
    let whole = store
        .whole_graph()
        .map_err(|error| io::Error::other(format!("graph load failed: {error:?}")))?;
    let corpus = whole.corpus();
    let Some(page) = corpus.pages.iter().find(|page| page.name == name) else {
        return Ok(None);
    };
    store.page(&page.id).map_err(crate::store_error)?;
    // The old print path parses Org source as Markdown. Keep that behavior in
    // the print client, within the shared parse input byte limit.
    let org_document = if page.id.as_str().to_ascii_lowercase().ends_with(".org") {
        let file = tine_store::FileId::from(page.id.as_str().to_owned());
        let (source, _) = crate::parsed_text::read(store, &file)?;
        Some(doc::parse(&source))
    } else {
        None
    };
    render::page_print_html(
        &RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store,
            sheets: Some(&sheets),
        },
        name,
        opts,
        org_document.as_ref(),
    )
}
