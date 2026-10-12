//! The host read (CLI PROTOCOL §5): a page's current text for a client that
//! is not a window. One request on the driver, holding nothing on the
//! caller's thread; a page the host does not hold is read from its file.
use super::*;
use crate::StoreError;

/// A page's current text as the host sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRead {
    /// The page's spelling on disk.
    pub page: PageId,
    /// The host's buffer for a held page (unsaved input included), else the
    /// file's bytes. `None`: no file (and, for a held page, a deletion).
    pub bytes: Option<Arc<[u8]>>,
    /// `(incarnation, version)` of a held page: informational and scoped
    /// to this host incarnation. `None` for a page the host does not hold;
    /// no version is invented for it.
    pub revision: Option<(u64, u64)>,
}

/// Why a host read has no answer.
#[derive(Debug)]
pub enum HostReadError {
    /// The host ended.
    Ended,
    /// The page's file could not be read (too large included).
    Store(StoreError),
}

impl PageHost {
    /// `page`'s current text: the held buffer, or the file the host would
    /// load. Not `Store::page`, whose indexed bytes can lag the buffer.
    pub fn read(&self, page: &PageId) -> Result<HostRead, HostReadError> {
        let (key, spelling, ..) = self.try_identify(page).ok_or(HostReadError::Ended)?;
        let held = self
            .driver
            .shared
            .read(move |state: &retained::HostState| {
                let host = &state.progress.host;
                host.pages
                    .get(&key)
                    .map(|page| (page.buf.clone(), (host.incarnation, page.version)))
            })
            .map_err(|_| HostReadError::Ended)?;
        if let Some((bytes, revision)) = held {
            return Ok(HostRead {
                page: spelling,
                bytes,
                revision: Some(revision),
            });
        }
        let cap = Some(crate::PARSE_INPUT_MAX_BYTES);
        let bytes = match self.store.read(&spelling.file(), cap) {
            Ok((bytes, _)) => Some(Arc::from(bytes)),
            Err(StoreError::NotFound) => None,
            Err(error) => return Err(HostReadError::Store(error)),
        };
        Ok(HostRead {
            page: spelling,
            bytes,
            revision: None,
        })
    }
}
