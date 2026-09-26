//! Single-page print document client.

use std::io;
use tine_core::doc;
use tine_store::Store;

use crate::render::{self, RenderGraph};

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

/// Render a named page using the read-only corpus and bounded asset reads.
pub fn page_print_html(store: &Store, name: &str, opts: PrintOpts) -> io::Result<Option<String>> {
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
    // the print client, with its original unbounded file read.
    let org_document = if page.id.as_str().to_ascii_lowercase().ends_with(".org") {
        let file = tine_store::FileId::from(page.id.as_str().to_owned());
        let (bytes, _) = store
            .read(&file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
            .map_err(crate::store_error)?;
        let source = String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if !tine_store::parse_input_depth_within_limit(&source) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "I-22: input nesting exceeds 512 levels",
            ));
        }
        Some(doc::parse(&source))
    } else {
        None
    };
    render::page_print_html(
        &RenderGraph {
            corpus: &corpus,
            whole: &whole,
            store,
        },
        name,
        opts,
        org_document.as_ref(),
    )
}
