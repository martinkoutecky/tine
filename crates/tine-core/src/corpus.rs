//! Read-only parsed input for graph-wide evaluators and renderers.

use std::sync::Arc;

use crate::doc::Document;
use crate::model::{PageId, PageKind};

/// One physical page file. Duplicate logical page names stay separate so an
/// evaluator can decide whether a name has an unambiguous source.
#[derive(Clone)]
pub struct CorpusPage {
    pub id: PageId,
    pub name: String,
    pub kind: PageKind,
    pub document: Arc<Document>,
}

/// Parsed pages with their file identity and full preamble, properties and
/// block tree. This value performs no I/O and is independent of its store.
#[derive(Clone, Default)]
pub struct Corpus {
    pub pages: Vec<CorpusPage>,
}
