//! The page host's publication queries (§4.4): what a barrier waits on
//! (`pages_published`, `pages_recoverable`), the wait itself, the hurry
//! (`save_now`) and the publication debt list (`owed`).
use super::*;

impl PageHost {
    /// `pages_recoverable` (§4.4): every listed page is clean at or past
    /// that version, or its durable draft holds a version at or past it. A
    /// page the host does not hold has nothing to recover.
    pub(crate) fn pages_recoverable(&self, pages: &[(String, u64)]) -> bool {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let drafts = host.logical_drafts();
        pages
            .iter()
            .all(|(key, version)| match host.pages.get(key) {
                None => true,
                Some(page) => {
                    (page.clean() && page.version >= *version)
                        || drafts
                            .get(key)
                            .is_some_and(|draft| draft.version >= *version)
                }
            })
    }

    /// `pages_published` (§4.4): every listed page the host holds is clean
    /// with its index observation complete (Q5), or the publication
    /// consumer has indexed an own save at or past that version. With a
    /// witness block id (a block reference's target, §8), the page's last
    /// indexed bytes must also hold that block (Q2): a later version without
    /// it does not do, nor does an unsent local restoration. A page the
    /// host does not hold has no save owed, but cannot witness a block.
    pub(crate) fn pages_published(&self, pages: &[(String, u64, Option<String>)]) -> bool {
        let mut witnesses = Vec::new();
        {
            let state = self.driver.shared.state.lock().unwrap();
            let host = &state.progress.host;
            for (key, version, witness) in pages {
                let Some(page) = host.pages.get(key) else {
                    if witness.is_some() || state.book.owned.contains(key) {
                        return false;
                    }
                    continue;
                };
                let Some((indexed, watermark)) = state.book.index.get(key) else {
                    return false;
                };
                let clean = page.clean() && page.version >= *version && *indexed == page.buf;
                if !clean && *watermark < *version {
                    return false;
                }
                if let Some(witness) = witness {
                    witnesses.push((host.fs.spelling(key), indexed.clone(), witness));
                }
            }
        }
        let root = &self.store.graph.root;
        witnesses.into_iter().all(|(spelling, bytes, id)| {
            bytes.is_some_and(|bytes| bytes_hold_block_id(&root.join(spelling), &bytes, id))
        })
    }

    /// `page_wait` (§4.4): wait until `pages_published` holds for `needs`
    /// (true), or false once a needed page cannot publish without the user
    /// (a conflict, a third failed save, a third failed index publication)
    /// or at `bound`. Never success at a bound (S1).
    pub(crate) fn wait_published(
        &self,
        needs: &[(String, u64, Option<String>)],
        bound: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + bound;
        loop {
            if self.pages_published(needs) {
                return true;
            }
            let shared = &self.driver.shared;
            let state = shared.state.lock().unwrap();
            let progress = &state.progress;
            let stuck = needs.iter().any(|(key, ..)| {
                progress
                    .host
                    .pages
                    .get(key)
                    .is_some_and(|page| page.conflict)
                    || progress.notice(key).save_error
                    || state.book.index_error(key)
            });
            let now = std::time::Instant::now();
            if stuck || now >= deadline {
                return false;
            }
            let pause = (deadline - now).min(std::time::Duration::from_millis(100));
            drop(shared.wait(state, pause));
        }
    }

    /// `page_save_now` (§4.4): make these keys' saves due at once, for a
    /// barrier or a block reference; a failing save keeps its backoff.
    pub(crate) fn save_now(&self, keys: &[String]) {
        self.driver
            .shared
            .with_state(|state| state.progress.save_now(keys));
    }

    /// `page_owed` (§4.4, R2): every held page whose text or index is not
    /// published at its current version, with that version, and each key
    /// whose index the consumer still owes after the host let it go (version
    /// 0). A draft still to retire is not publication debt. `paths` limits
    /// the list to the keys those paths name (the shared entry identity);
    /// None lists every key. Bounded by the held pages; no filesystem scan.
    pub(crate) fn owed(&self, paths: Option<&[PageId]>) -> Vec<(PageKey, u64)> {
        let only: Option<BTreeSet<PageKey>> =
            paths.map(|paths| paths.iter().map(|page| self.identify(page).0).collect());
        let wanted = |key: &PageKey| only.as_ref().is_none_or(|only| only.contains(key));
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let book = &state.book;
        let held = host.pages.iter().filter_map(|(key, page)| {
            let indexed = book.index.get(key);
            let published = indexed.is_some_and(|(bytes, watermark)| {
                (page.clean() && *bytes == page.buf) || *watermark >= page.version
            });
            (!published).then(|| (key.clone(), page.version))
        });
        let released = book
            .owned
            .iter()
            .filter(|key| !host.pages.contains_key(*key))
            .map(|key| (key.clone(), 0));
        held.chain(released)
            .filter(|(key, _)| wanted(key))
            .collect()
    }
}
