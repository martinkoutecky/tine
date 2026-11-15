//! Page read wire answers retain provenance even when the page is absent.
use super::*;

#[derive(Serialize)]
pub(crate) struct PageWire {
    pub(super) id: String,
    #[serde(flatten)]
    pub(super) doc: PageDto,
}

impl std::ops::Deref for PageWire {
    type Target = PageDto;

    fn deref(&self) -> &PageDto {
        &self.doc
    }
}

pub(super) fn page_dto(read: tine_store::PageRead) -> PageWire {
    let mut doc = read.doc;
    doc.rev = Some(read.rev.into());
    PageWire {
        id: read.id.into(),
        doc,
    }
}

#[derive(Serialize)]
/// Internal path-read answer: absence retains the selected read source;
/// a rejected target has no selected source. It performs no additional I/O.
pub(crate) struct PagePathAnswer {
    pub(super) page: Option<PageWire>,
    pub(super) source: Option<tine_store::PageReadSource>,
}
