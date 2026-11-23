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
        doc.read_only = view.page_read_only(&id);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A view's page reads are read-only when the view is (a published
    /// export) or the page is (an Org file Tine cannot round-trip); an
    /// editable browser graph's other pages read writable.
    #[test]
    fn view_page_reads_follow_the_view_and_the_page() {
        let files = || {
            vec![
                ("pages/Plain.md".to_owned(), b"- text\n".to_vec()),
                ("pages/Skipped.org".to_owned(), b"* a\n*** c\n".to_vec()),
                ("journals/2000_01_01.md".to_owned(), b"- day\n".to_vec()),
            ]
        };
        let read_only = |view: &WholeGraph, path: &str| {
            let answer = get_page_by_path(PageSource::View(view), PageId::from(path)).unwrap();
            answer.page.expect(path).doc.read_only
        };
        let feed_read_only = |view: &WholeGraph| {
            let page = journal_feed_page(PageSource::View(view), 1, Some(20000102), None, false);
            page.unwrap().pages[0].doc.read_only
        };
        let (editable, _) = WholeGraph::from_files(files(), Vec::new(), true).unwrap();
        assert!(!read_only(&editable, "pages/Plain.md"));
        assert!(!feed_read_only(&editable));
        let plain = get_page(PageSource::View(&editable), "plain", PageKind::Page);
        assert!(!plain.unwrap().expect("by name").doc.read_only);
        assert!(read_only(&editable, "pages/Skipped.org"));

        let (published, _) = WholeGraph::from_files(files(), Vec::new(), false).unwrap();
        assert!(read_only(&published, "pages/Plain.md"));
        assert!(feed_read_only(&published));
        assert!(read_only(&published, "pages/Skipped.org"));
    }
}
