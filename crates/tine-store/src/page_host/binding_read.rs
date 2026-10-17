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
    /// load. Not `Store::page`, whose indexed bytes can lag the buffer. The
    /// held-or-file choice, the spelling, the bytes and the revision come
    /// from one host read, so an open, a save or a respelling cannot fall
    /// between them (review CODE-1 F2, CODE-2 #4); the caller parses outside
    /// it (A-K3).
    pub fn read(&self, page: &PageId) -> Result<HostRead, HostReadError> {
        let (key, spelling, ..) = self.try_identify(page).ok_or(HostReadError::Ended)?;
        let store = self.store.clone();
        let asked = page.clone();
        self.driver
            .shared
            .read(move |state: &retained::HostState| {
                let host = &state.progress.host;
                if let Some(held) = host.pages.get(&key) {
                    // The held key's spelling now: a case-only rename since
                    // `try_identify` respelled it (review CODE-2 #4).
                    return Ok(HostRead {
                        page: PageId::from(host.fs.spelling(&key)),
                        bytes: held.buf.clone(),
                        revision: Some((host.incarnation, held.version)),
                    });
                }
                // Unheld: its spelling and bytes as the host would load them now.
                let spelling = store.disk_spelling(&asked).unwrap_or(spelling);
                let cap = Some(crate::PARSE_INPUT_MAX_BYTES);
                let bytes = match store.read(&spelling.file(), cap) {
                    Ok((bytes, _)) => Some(Arc::from(bytes)),
                    Err(StoreError::NotFound) => None,
                    Err(error) => return Err(HostReadError::Store(error)),
                };
                Ok(HostRead {
                    page: spelling,
                    bytes,
                    revision: None,
                })
            })
            .map_err(|_| HostReadError::Ended)?
    }
}
