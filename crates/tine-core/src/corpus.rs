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
