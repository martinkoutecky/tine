//! Shared command wires for page bodies and the journal feed. Each source
//! selects its own bytes; captured documents have no save revision or live
//! read provenance.

use serde::Serialize;
use tine_core::model::{PageDto, PageKind};
#[cfg(not(target_arch = "wasm32"))]
use tine_store::Store;
use tine_store::{PageId, PageRead, PageReadSource, StoreError, WholeGraph};

use crate::journals::{self, FeedPage};

pub enum PageSource<'a> {
    #[cfg(not(target_arch = "wasm32"))]
    Live(&'a Store),
    View(&'a WholeGraph),
}

#[derive(Serialize)]
pub struct PageWire {
    pub id: PageId,
    #[serde(flatten)]
    pub doc: PageDto,
}

impl From<PageRead> for PageWire {
    fn from(read: PageRead) -> Self {
        let mut doc = read.doc;
        doc.rev = Some(read.rev.into());
        Self { id: read.id, doc }
    }
}

impl PageWire {
    pub(crate) fn from_view(view: &WholeGraph, id: PageId) -> Option<Self> {
        let mut doc = view.page(&id)?;
        doc.read_only = true;
        Some(Self { id, doc })
    }
}

#[derive(Serialize)]
pub struct PagePathAnswer {
    pub page: Option<PageWire>,
    pub source: Option<PageReadSource>,
}

/// Resolve and read a name, including aliases. One owner query and one
/// page read; the live store can reconcile a newly delivered journal.
pub fn get_page(
    source: PageSource<'_>,
    name: &str,
    kind: PageKind,
) -> Result<Option<PageWire>, StoreError> {
    match source {
        #[cfg(not(target_arch = "wasm32"))]
        PageSource::Live(store) => Ok(store.page_named(name, kind)?.map(PageWire::from)),
        PageSource::View(view) => Ok(view
            .page_owner(name, kind)
            .and_then(|id| PageWire::from_view(view, id))),
    }
}

/// Read one exact physical path. Live absence preserves any selected read
/// source. The view needs one indexed lookup and O(page bytes + blocks).
pub fn get_page_by_path(source: PageSource<'_>, id: PageId) -> Result<PagePathAnswer, StoreError> {
    let (page, source) = match source {
        #[cfg(not(target_arch = "wasm32"))]
        PageSource::Live(store) => match store.page(&id) {
            Ok(read) => {
                let source = read.source;
                (Some(read.into()), Some(source))
            }
            Err(StoreError::NotFound(source)) => (None, Some(source)),
            Err(StoreError::InvalidTarget(_)) => (None, None),
            Err(error) => return Err(error),
        },
        PageSource::View(view) => (PageWire::from_view(view, id), None),
    };
    Ok(PagePathAnswer { page, source })
}

/// Descending journal page with an exclusive cursor and optional extension
/// through a displayed day. Reads each examined body at most once.
pub fn journal_feed_page(
    source: PageSource<'_>,
    limit: usize,
    before: Option<i64>,
    through: Option<i64>,
    include_references: bool,
) -> std::io::Result<FeedPage<PageWire>> {
    match source {
        #[cfg(not(target_arch = "wasm32"))]
        PageSource::Live(store) => {
            journals::feed_page_through(store, limit, before, through, include_references)
        }
        PageSource::View(view) => {
            journals::feed_page_from_view(view, limit, before, through, include_references)
        }
    }
}
