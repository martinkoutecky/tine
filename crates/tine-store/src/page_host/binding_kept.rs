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
    /// its UTF-8 check and the page its text names
    /// (`Graph::page_entry_of_text`: a preamble scan of text in hand, no
    /// read, no discovery record).
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
        let graph = &self.store.graph;
        let rows = rows.into_iter().map(|(key, origin, path, bytes)| {
            let bytes = bytes.as_deref().unwrap_or_default();
            let text = std::str::from_utf8(bytes).ok();
            // Bytes that are not text name the page by its file name.
            let entry = graph.page_entry_of_text(&graph.root.join(&path), text.unwrap_or(""));
            KeptOriginal {
                key,
                origin,
                path,
                kind: entry.as_ref().map(|entry| entry.kind),
                name: entry.map(|entry| entry.name),
                len: bytes.len(),
                text: text.is_some(),
            }
        });
        Some(rows.collect())
    }

    /// `page_kept_text`: the kept original (`key`, `origin`) for the panel's
    /// Copy original: its exact text, or `NotText` for bytes that are not
    /// UTF-8; `Missing` when the current session's host keeps no such
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
            match std::str::from_utf8(bytes) {
                Ok(text) => KeptText::Text { text: text.into() },
                Err(_) => KeptText::NotText { len: bytes.len() },
            }
        };
        (self.driver.shared.read(read)).unwrap_or(KeptText::Unavailable)
    }
}
