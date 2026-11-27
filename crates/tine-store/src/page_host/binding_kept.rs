//! Kept originals at the binding boundary (GH #540): the recovery panel's
//! list and its Copy original. Both only read the host's state: they write
//! nothing and record nothing.
use super::retained::HostState;
use super::*;
use io::{KeptOriginal, KeptText};

impl PageHost {
    /// `page_kept_originals`: every original a cancelled rename kept
    /// (`Order::kept`), for window `session`; None when not answered (the
    /// driver ended, the host is down, or `session` is not the current one).
    /// One walk over the durable pairs under the state mutex; then, per row,
    /// its text check (`as_text`), off the driver. No row is parsed or
    /// named: the panel shows its path.
    pub fn kept_originals(&self, session: u64) -> Option<Vec<KeptOriginal>> {
        let read = move |state: &HostState| {
            let host = &state.progress.host;
            if state.book.session != session || !host.alive {
                return None;
            }
            let rows = (host.order.kept()).map(|(_, p)| {
                let path = host.fs.spelling(&p.source);
                (p.source.clone(), p.origin, path, p.preimage.clone())
            });
            Some(rows.collect::<Vec<_>>())
        };
        let rows = self.driver.shared.read(read).ok()??;
        let rows = rows.into_iter().map(|(key, origin, path, bytes)| {
            let bytes = bytes.as_deref().unwrap_or_default();
            KeptOriginal {
                key,
                origin,
                path,
                len: bytes.len(),
                text: as_text(bytes).is_some(),
            }
        });
        Some(rows.collect())
    }

    /// `page_kept_text`: the kept original (`key`, `origin`) for the panel's
    /// Copy original: its exact text, or `NotText` for bytes that are not
    /// text (`as_text`); `Missing` when the current session's host keeps no such
    /// original; `Unavailable` when not answered. One pair search.
    pub fn kept_text(&self, session: u64, key: &str, origin: u64) -> KeptText {
        let key = key.to_owned();
        let read = move |state: &HostState| {
            let host = &state.progress.host;
            if state.book.session != session || !host.alive {
                return KeptText::Unavailable;
            }
            let Some(pair) = host.order.kept_at(&key, origin) else {
                return KeptText::Missing;
            };
            let bytes = pair.preimage.as_deref().unwrap_or_default();
            match as_text(bytes) {
                Some(text) => KeptText::Text { text: text.into() },
                None => KeptText::NotText { len: bytes.len() },
            }
        };
        (self.driver.shared.read(read)).unwrap_or(KeptText::Unavailable)
    }
}

/// A kept original's text, when Copy original can hand it over exactly:
/// UTF-8 with no NUL. A Windows clipboard ends text at the first NUL, and
/// Logseq and git take such a file for binary.
fn as_text(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
}
