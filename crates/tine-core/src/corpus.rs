//! Read-only parsed input for graph-wide evaluators and renderers.

use std::sync::Arc;

use crate::doc::Document;
use crate::model::{PageId, PageKind};

/// One physical page file. Duplicate logical page names stay separate so an
/// evaluator can decide whether a name has an unambiguous source.
#[derive(Clone)]
/// One immutable parsed page in an evaluator corpus.
#[deny(missing_docs)]
pub struct CorpusPage {
    /// Physical page identity.
    pub id: PageId,
    /// Decoded page name.
    pub name: String,
    /// Journal or ordinary page.
    pub kind: PageKind,
    /// Parsed page document.
    pub document: Arc<Document>,
}

/// Parsed pages with their file identity and full preamble, properties and
/// block tree. This value performs no I/O and is independent of its store.
#[derive(Clone, Default)]
/// Owned list of parsed pages for pure evaluators.
#[deny(missing_docs)]
pub struct Corpus {
    /// Pages included in the captured graph view.
    pub pages: Vec<CorpusPage>,
}

impl Corpus {
    /// Block-reference counts over exactly these pages, by the same
    /// per-document counter as the graph's badges. Cost O(blocks); no I/O.
    pub fn block_ref_counts(&self) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();
        for page in &self.pages {
            for (id, count) in crate::page_properties::document_block_ref_counts(&page.document) {
                *counts.entry(id).or_default() += count;
            }
        }
        counts
    }

    /// Asset names these pages mention, by the same collector as orphan-asset
    /// detection: raw and percent-decoded spellings, plus the first segment of
    /// a nested reference. Cost O(page text bytes); no I/O.
    pub fn asset_refs(&self) -> std::collections::HashSet<String> {
        let mut names = std::collections::HashSet::new();
        for page in &self.pages {
            crate::asset_refs::collect_document_asset_refs(&page.document, &mut names);
        }
        names
    }
}
