//! The page host's publication queries (§4.4): what a barrier waits on
//! (`pages_published`, `pages_recoverable`), the wait itself, the hurry
//! (`save_now`) and the publication debt list (`owed`). Each reads a
//! key's logical state through `Host::logical`, never the applied pages
//! (Q-P2b-4; census `barrier_queries_read_the_logical_state`).
use super::*;

impl PageHost {
    /// `pages_recoverable` (§4.4): every listed page is clean at or past
    /// that version, or its durable draft holds a version at or past it. A
    /// page the host does not hold has nothing to recover; admitted input
    /// not applied yet (Q-P2b-4) is recoverable only through its draft.
    pub(crate) fn pages_recoverable(&self, pages: &[(String, u64)]) -> bool {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let drafts = host.logical_drafts();
        let drafted = |key: &String, version: u64| {
            drafts
                .get(key)
                .is_some_and(|draft| draft.version >= version)
        };
        pages.iter().all(|(key, version)| match host.logical(key) {
            Logical::Absent => true,
            Logical::Settled(page) => {
                (page.clean() && page.version >= *version) || drafted(key, *version)
            }
            Logical::Admitted { .. } => drafted(key, *version),
        })
    }

    /// `pages_published` (§4.4): every listed page the host holds is clean
    /// with its index observation complete (Q5), or the publication
    /// consumer has indexed an own save at or past that version. With a
    /// witness block id (a block reference's target, §8), the page's last
    /// indexed bytes must also hold that block (Q2): a later version without
    /// it does not do, nor does an unsent local restoration. A page the
    /// host does not hold has no save owed, but cannot witness a block. A
    /// page with admitted input not applied yet (Q-P2b-4) is unpublished.
    pub(crate) fn pages_published(&self, pages: &[(String, u64, Option<String>)]) -> bool {
        let mut witnesses = Vec::new();
        {
            let state = self.driver.shared.state.lock().unwrap();
            let host = &state.progress.host;
            for (key, version, witness) in pages {
                let page = match host.logical(key) {
                    Logical::Admitted { .. } => return false,
                    Logical::Absent if witness.is_some() || state.book.owned.contains(key) => {
                        return false;
                    }
                    Logical::Absent => continue,
                    Logical::Settled(page) => page,
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

    /// `page_wait` (§4.4) for window `session` (REVIEW-3b-P1 F1): wait
    /// until `pages_published` holds for `needs` (`Applied`), or `Refused`
    /// once a needed page cannot publish without the user (a conflict, a
    /// third failed save, a third failed index publication) or the session
    /// is not current, so a result never vouches for another session's
    /// pages. `Pending` at `bound`: never success at a bound (S1); the
    /// window asks again.
    pub fn wait_published(
        &self,
        session: u64,
        needs: &[(String, u64, Option<String>)],
        bound: std::time::Duration,
    ) -> PageOperation {
        let deadline = std::time::Instant::now() + bound;
        let current = || self.driver.shared.state.lock().unwrap().book.session == session;
        loop {
            if !current() {
                return PageOperation::Refused;
            }
            if self.pages_published(needs) {
                return if current() {
                    PageOperation::Applied
                } else {
                    PageOperation::Refused
                };
            }
            let shared = &self.driver.shared;
            let state = shared.state.lock().unwrap();
            let progress = &state.progress;
            let stuck = needs.iter().any(|(key, ..)| {
                progress.host.logical(key).conflict()
                    || progress.notice(key).save_error
                    || state.book.index_error(key)
            });
            if stuck {
                return PageOperation::Refused;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return PageOperation::Pending;
            }
            let pause = (deadline - now).min(std::time::Duration::from_millis(100));
            drop(shared.wait(state, pause));
        }
    }

    /// `page_save_now`: make these keys' saves due now (a hint), for the
    /// current window `session` only (F1).
    pub fn save_now(&self, session: u64, keys: &[String]) {
        self.driver.shared.with_state(|state| {
            if state.book.session == session {
                state.progress.save_now(keys)
            }
        });
    }

    /// `page_owed` (§4.4, R2): every held page whose text or index is not
    /// published at its current version, with that version; every key with
    /// admitted input not applied yet (Q-P2b-4: an in-flight deletion,
    /// rename or move, a queued submit), at the version it carries; and each key
    /// whose index the consumer still owes after the host let it go (version
    /// 0). A draft still to retire is not publication debt. `paths` limits
    /// the list to the keys those paths name (the shared entry identity);
    /// None lists every key. Bounded by the held pages and admitted input;
    /// no filesystem scan.
    /// None when `session` is not the current window session (F1): an
    /// empty list would claim no debt.
    pub fn owed(&self, session: u64, paths: Option<&[PageId]>) -> Option<Vec<(PageKey, u64)>> {
        let only: Option<BTreeSet<PageKey>> =
            paths.map(|paths| paths.iter().map(|page| self.identify(page).0).collect());
        let wanted = |key: &PageKey| only.as_ref().is_none_or(|only| only.contains(key));
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let book = &state.book;
        if book.session != session {
            return None;
        }
        let logical = host.logical_keys();
        let held = logical.iter().filter_map(|key| match host.logical(key) {
            Logical::Settled(page) => {
                let indexed = book.index.get(key);
                let published = indexed.is_some_and(|(bytes, watermark)| {
                    (page.clean() && *bytes == page.buf) || *watermark >= page.version
                });
                (!published).then(|| (key.clone(), page.version))
            }
            Logical::Admitted { version, .. } => Some((key.clone(), version)),
            Logical::Absent => None,
        });
        let released = book
            .owned
            .iter()
            .filter(|key| !logical.contains(*key))
            .map(|key| (key.clone(), 0));
        Some(
            held.chain(released)
                .filter(|(key, _)| wanted(key))
                .collect(),
        )
    }
}
