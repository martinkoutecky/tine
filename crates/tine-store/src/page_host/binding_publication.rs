//! The page host's publication queries (§4.4): what a barrier waits on
//! (`pages_published`, `pages_recoverable`), the wait itself, the hurry
//! (`save_now`) and the publication debt list (`owed`). Each reads a
//! key's logical state through `Host::logical`, never the applied pages
//! (Q-P2b-4; census `barrier_queries_read_the_logical_state`).
use super::*;
use crate::page_host::driver::{Deadline, State, Unanswered, Wait};

/// A barrier's needs: (key, version, witness block id).
type Needs = [(String, u64, Option<String>)];

/// The block witnesses a publication check still owes: (spelling, the
/// page's last indexed bytes, block id).
type Witnesses = Vec<(String, Text, String)>;

/// `pages_published`'s state part, under the state mutex: None when a
/// listed page is not published, else the block witnesses to check against
/// the indexed bytes (outside the mutex; they parse). False on a host that
/// panicked: it vouches for nothing.
fn published(state: &State<ProductionIo, SystemClock>, pages: &Needs) -> Option<Witnesses> {
    let mut witnesses = Vec::new();
    let host = &state.progress.host;
    for (key, version, witness) in pages {
        let page = match host.logical(key) {
            Logical::Admitted { .. } => return None,
            Logical::Absent if witness.is_some() || state.book.owned().contains(key) => {
                return None;
            }
            Logical::Absent => continue,
            Logical::Settled(page) => page,
        };
        let (indexed, watermark) = state.book.index.get(key)?;
        let clean = page.clean() && page.version >= *version && *indexed == page.buf;
        if !clean && *watermark < *version {
            return None;
        }
        if let Some(witness) = witness {
            witnesses.push((host.fs.spelling(key), indexed.clone(), witness.clone()));
        }
    }
    Some(witnesses)
}

/// What a barrier wait's step found (§4.4).
enum Barrier {
    /// Published, once these witnesses hold.
    Check(Witnesses),
    Refused,
    Pending,
}

impl PageHost {
    /// `pages_recoverable` (§4.4): every listed page is clean at or past
    /// that version, or its durable draft holds a version at or past it. A
    /// page the host does not hold has nothing to recover; admitted input
    /// not applied yet (Q-P2b-4) is recoverable only through its draft.
    #[cfg(test)]
    pub(crate) fn pages_recoverable(&self, pages: &[(String, u64)]) -> bool {
        let pages = pages.to_vec();
        let read = move |state: &State<ProductionIo, SystemClock>| {
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
        };
        self.driver.shared.read(read).unwrap()
    }

    /// `pages_published` (§4.4): every listed page the host holds is clean
    /// with its index observation complete (Q5), or the publication
    /// consumer has indexed an own save at or past that version. With a
    /// witness block id (a block reference's target, §8), the page's last
    /// indexed bytes must also hold that block (Q2): a later version without
    /// it does not do, nor does an unsent local restoration. A page the
    /// host does not hold has no save owed, but cannot witness a block. A
    /// page with admitted input not applied yet (Q-P2b-4) is unpublished.
    /// False on a host that ended: it vouches for nothing.
    #[cfg(test)]
    pub(crate) fn pages_published(&self, pages: &Needs) -> bool {
        let pages = pages.to_vec();
        let read = self
            .driver
            .shared
            .read(move |state| published(state, &pages));
        read.ok().flatten().is_some_and(|w| self.witnessed(w))
    }

    /// Each witness block is in its page's last indexed bytes (Q2).
    fn witnessed(&self, witnesses: Witnesses) -> bool {
        let root = &self.store.graph.root;
        witnesses.into_iter().all(|(spelling, bytes, id)| {
            bytes.is_some_and(|bytes| bytes_hold_block_id(&root.join(spelling), &bytes, &id))
        })
    }

    /// `page_wait` (§4.4) for window `session` (REVIEW-3b-P1 F1): wait
    /// until `pages_published` holds for `needs` (`Applied`), or `Refused`
    /// once a needed page cannot publish without the user (a conflict, a
    /// third failed save, a third failed index publication) or the session
    /// is not current, so a result never vouches for another session's
    /// pages. `Refused` too once nothing will publish them (REVIEW-3b-P2b
    /// R3): the driver ended, or the host is down (it dropped its pages,
    /// which would read as owing nothing). `Pending` at `bound`, also when
    /// the driver did not answer by then: never success at a bound (S1);
    /// the window asks again.
    pub fn wait_published(
        &self,
        session: u64,
        needs: &Needs,
        bound: std::time::Duration,
    ) -> PageOperation {
        let deadline = std::time::Instant::now() + bound;
        // After a witness that did not hold, the next ask waits for a step.
        let mut again = false;
        loop {
            let needs = needs.to_vec();
            let mut first = std::mem::take(&mut again);
            let step = move |state: &mut State<ProductionIo, SystemClock>| {
                if state.book.session != session || !state.progress.host.alive {
                    return Wait::Ready(Barrier::Refused);
                }
                let now = std::time::Instant::now();
                if !std::mem::take(&mut first) {
                    if let Some(witnesses) = published(state, &needs) {
                        return Wait::Ready(Barrier::Check(witnesses));
                    }
                    let progress = &state.progress;
                    let stuck = needs.iter().any(|(key, ..)| {
                        progress.host.logical(key).conflict()
                            || progress.notice(key).save_error
                            || state.book.index_error(key)
                    });
                    if stuck {
                        return Wait::Ready(Barrier::Refused);
                    }
                }
                if now >= deadline {
                    return Wait::Ready(Barrier::Pending);
                }
                let pause = now + std::time::Duration::from_millis(100);
                Wait::Until(Some(pause.min(deadline)))
            };
            match self.driver.shared.wait(Deadline::At(deadline), step) {
                Ok(Barrier::Check(witnesses)) => {
                    if !self.witnessed(witnesses) {
                        again = true;
                        continue;
                    }
                    let live = move |state: &State<ProductionIo, SystemClock>| {
                        state.book.session == session && state.progress.host.alive
                    };
                    // Bounded too: a liveness check the driver does not
                    // answer within one more `bound` is pending.
                    let by = Deadline::At(std::time::Instant::now() + bound);
                    return match self.driver.shared.read_by(by, live) {
                        Ok(true) => PageOperation::Applied,
                        Ok(false) | Err(Unanswered::Ended) => PageOperation::Refused,
                        Err(_) => PageOperation::Pending,
                    };
                }
                Ok(Barrier::Refused) | Err(Unanswered::Ended) => return PageOperation::Refused,
                Ok(Barrier::Pending) | Err(_) => return PageOperation::Pending,
            }
        }
    }

    /// `page_save_now`: make these keys' saves due now (a hint), for the
    /// current window `session` only (F1).
    pub fn save_now(&self, session: u64, keys: &[String]) {
        let keys = keys.to_vec();
        // A hint: a host that ended has nothing to save.
        let _ = self.driver.shared.call(move |state| {
            if state.book.session == session {
                state.progress.save_now(&keys)
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
    /// None when `session` is not the current window session (F1), or the
    /// host ended: an empty list would claim no debt.
    pub fn owed(&self, session: u64, paths: Option<&[PageId]>) -> Option<Vec<(PageKey, u64)>> {
        let only: Option<BTreeSet<PageKey>> = match paths {
            Some(paths) => Some(
                (paths.iter())
                    .map(|page| self.try_identify(page).map(|(key, ..)| key))
                    .collect::<Option<_>>()?,
            ),
            None => None,
        };
        let read = move |state: &State<ProductionIo, SystemClock>| {
            let wanted = |key: &PageKey| only.as_ref().is_none_or(|only| only.contains(key));
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
                .owned()
                .iter()
                .filter(|key| !logical.contains(*key))
                .map(|key| (key.clone(), 0));
            Some(
                held.chain(released)
                    .filter(|(key, _)| wanted(key))
                    .collect(),
            )
        };
        self.driver.shared.read(read).ok().flatten()
    }
}
