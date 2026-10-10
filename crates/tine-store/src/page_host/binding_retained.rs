//! Retained writers' reservations, the single-page rename, and the
//! switch/restore stop (STEP3 §6–§7): the binding commands that fence
//! pages away from the host or run a page operation for a writer.
use super::*;
use crate::transaction::validation::{rewrite as rewrite_refs, rewrite_move};
use crate::RenameMap;

/// Why a host rename did not run (§7, F10), or did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameRefusal {
    /// `target` is another spelling of `source`'s own directory entry (a
    /// case-folding volume, Q4): the caller moves it as a retained
    /// transaction under a reservation of the page and `respell`s it.
    Alias,
    /// A page the rename would rewrite has unsaved input.
    Unsaved(PageId),
    /// A page the rename would change cannot be rewritten: an Org file
    /// that does not round-trip, or bytes that are not UTF-8.
    Unwritable(PageId),
    /// The operation was admitted but this page could not be written (a
    /// conflict or a persistent save error); the operation keeps its
    /// custody and completes once the page can be written.
    Unwritten(PageId),
    /// The target has a file, or the host is stopped.
    Refused,
}

/// What the rename policy recorded for one attempt (pages by spelling).
#[derive(Default)]
struct Rewrites {
    changed: BTreeSet<String>,
    skipped: BTreeSet<String>,
    unwritable: Option<String>,
}

impl PageHost {
    /// Reserve the complete page set `discover` names for a retained
    /// writer (§7 steps 2–3, R6). Waiting reservations hold no lock. Under
    /// the reservation discovery runs again; a grown set starts over. Then,
    /// under that final reservation, the writer's input contract is checked
    /// on every page in it (Q3): unsaved input is a buffer that is not
    /// clean (typed, at risk or on an unknown base) or a submit or move the
    /// host admitted but has not applied. `Refuse` returns those pages;
    /// `Flush` releases, waits for their saves holding nothing, and retries,
    /// returning the pages whose save cannot complete.
    pub fn reserve(
        &self,
        mut discover: impl FnMut() -> Vec<PageId>,
        input: Input,
    ) -> Result<Reservation, BTreeSet<PageKey>> {
        loop {
            let keys = self.register_all(discover());
            self.fence(&keys);
            if !self.register_all(discover()).is_subset(&keys) {
                self.unreserve(&keys);
                continue;
            }
            let unsaved = self.unsaved(&keys);
            if unsaved.is_empty() {
                return Ok(Reservation { keys });
            }
            self.unreserve(&keys);
            if input == Input::Refuse {
                return Err(unsaved);
            }
            self.flush(&unsaved)?;
        }
    }

    /// End a retained transaction (§7 step 5), returning at once: the
    /// driver unreserves the keys and observes each, retrying a failed
    /// read with the save backoff. A save before that observation is safe:
    /// its guard check reads the writer's change. The transaction's index
    /// stands until that observation succeeds (V2).
    pub fn release(&self, reservation: Reservation) {
        self.driver.shared.with_state(|state| {
            for key in reservation.keys {
                state.progress.host.retained.remove(&key);
                state.book.handover.insert(key.clone());
                state.observe.entry(key).or_default();
            }
        });
    }

    fn register_all(&self, pages: Vec<PageId>) -> BTreeSet<PageKey> {
        pages
            .iter()
            .map(|page| {
                let (key, spelling, _) = self.identify(page);
                self.register(&key, &spelling);
                key
            })
            .collect()
    }

    /// Reserve `keys` under the state mutex only, waiting on the host
    /// condition while any is busy (§7 step 3).
    fn fence(&self, keys: &BTreeSet<PageKey>) {
        let shared = &self.driver.shared;
        let mut state = shared.state.lock().unwrap();
        while state.progress.host.reserve(keys) == Disposition::Waiting {
            state = shared.wait(state, std::time::Duration::from_secs(1));
        }
    }

    /// Withdraw a reservation under which nothing was written.
    fn unreserve(&self, keys: &BTreeSet<PageKey>) {
        self.driver.shared.with_state(|state| {
            let retained = &mut state.progress.host.retained;
            retained.retain(|key| !keys.contains(key));
        });
    }

    /// `page_rename` (§7, F10): rename `source`'s page to the absent
    /// `target` as one host operation, rewriting `referrers` (the index's
    /// explicit referrers; held buffers are scanned too) with `map`. The
    /// policy is the rename transaction's own: the moving page's refs and
    /// own title are rebound (`rewrite_move`), every other page's refs are
    /// rewritten, a changed Org file that does not round-trip refuses the
    /// whole operation, and a page carrying VCS conflict markers is left
    /// byte-identical (moved verbatim) and reported. A `source` with no file
    /// rewrites references only. Waits while a page is busy, then until the
    /// operation's writes complete. Returns the rewritten referrers and the
    /// pages left for their markers.
    pub fn rename(
        &self,
        source: &PageId,
        target: &PageId,
        referrers: &[PageId],
        map: &RenameMap,
    ) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
        let (src, src_spelling, _) = self.identify(source);
        let (dst, dst_spelling, _) = self.identify(target);
        if src == dst {
            return Err(RenameRefusal::Alias);
        }
        self.register(&src, &src_spelling);
        self.register(&dst, &dst_spelling);
        let mut refs = self.register_all(referrers.to_vec());
        refs.remove(&src);
        refs.remove(&dst);
        let root = self.store.graph.root.clone();
        let format = self.store.config().file_name_format;
        let destination = root.join(dst_spelling.as_str());
        let seen = loop {
            let outcome = self.driver.shared.locked_step(|state| {
                let (disposition, seen, written) = state.progress.with_host(|host| {
                    let spellings: BTreeMap<PageKey, String> = host
                        .keys
                        .iter()
                        .map(|key| (key.clone(), host.fs.spelling(key).to_owned()))
                        .collect();
                    let record = Mutex::new(Rewrites::default());
                    let policy = |bytes: &Text, key: &str, moving: bool| {
                        let Some(old) = bytes else { return Ok(None) };
                        let spelling = &spellings[key];
                        let path = root.join(spelling);
                        let rewritten = if moving {
                            rewrite_move(old, &destination, map, format)
                        } else {
                            rewrite_refs(old, &path, map, format)
                        };
                        let mut record = record.lock().unwrap();
                        let Ok(new) = rewritten else {
                            record.unwritable = Some(spelling.clone());
                            return Err(());
                        };
                        if new.as_slice() == &old[..] {
                            return Ok(bytes.clone());
                        }
                        let syntax = tine_core::model::Format::from_path(&path);
                        let marked = std::str::from_utf8(old).is_ok_and(|text| {
                            !tine_core::concord_queue::vcs_conflict_markers(text, syntax).is_empty()
                        });
                        if marked {
                            record.skipped.insert(spelling.clone());
                            return Ok(bytes.clone());
                        }
                        if !moving {
                            record.changed.insert(spelling.clone());
                        }
                        Ok(Some(Arc::from(new)))
                    };
                    let disposition = host.rename_with(&src, &dst, &refs, policy);
                    let written = host.worker.as_ref().map(|w| w.pages.clone());
                    (disposition, record.into_inner().unwrap(), written)
                });
                // OG-RULES Rule 8 (A-K1): the operation's writes are renames.
                if disposition == Disposition::Pending {
                    let written = written.unwrap_or_default();
                    state.book.took(written, EditKind::RenamePage);
                }
                (disposition, seen)
            });
            let Some((disposition, seen)) = outcome else {
                return Err(RenameRefusal::Refused);
            };
            match disposition {
                Disposition::Pending => break seen,
                Disposition::Waiting => {
                    let shared = &self.driver.shared;
                    let state = shared.state.lock().unwrap();
                    drop(shared.wait(state, std::time::Duration::from_millis(100)));
                }
                _ => {
                    if let Some(page) = seen.unwritable {
                        return Err(RenameRefusal::Unwritable(PageId::from(page)));
                    }
                    let mut touched = refs.clone();
                    touched.extend(seen.changed.iter().cloned());
                    touched.extend([src.clone(), dst.clone()]);
                    return Err(match self.unsaved(&touched).into_iter().next() {
                        Some(page) => RenameRefusal::Unsaved(self.spelling(&page)),
                        None => RenameRefusal::Refused,
                    });
                }
            }
        };
        let mut written = refs;
        written.extend(seen.changed.iter().cloned());
        written.extend([src, dst]);
        if let Err(stuck) = self.flush(&written) {
            let page = stuck.into_iter().next().unwrap();
            return Err(RenameRefusal::Unwritten(self.spelling(&page)));
        }
        let pages = |set: BTreeSet<String>| set.into_iter().map(PageId::from).collect();
        Ok((pages(seen.changed), pages(seen.skipped)))
    }

    /// The alias spelling move (§2, Q4): under its reservation the caller
    /// moved `page`'s directory entry to `to`, another spelling of the same
    /// entry. The page keeps its key, buffer, drafts and queued requests;
    /// its I/O, path lock and held index follow `to`.
    pub fn respell(&self, page: &PageId, to: &PageId) {
        let graph = &self.store.graph;
        let lock = graph.page_lock(&graph.root.join(to.as_str()));
        let key = self.driver.shared.with_state(|state| {
            let host = &mut state.progress.host;
            let key = host
                .keys
                .iter()
                .find(|key| host.fs.spelling(key) == page.as_str())
                .cloned()?;
            host.respell(&key, to.as_str(), lock);
            Some(key)
        });
        if key.is_some() {
            let _writer = self.store.writer.lock().unwrap();
            graph.held.respell(
                &graph.root.join(page.as_str()),
                graph.root.join(to.as_str()),
            );
        }
    }

    /// A key's current spelling, as the page it names.
    fn spelling(&self, key: &str) -> PageId {
        let state = self.driver.shared.state.lock().unwrap();
        PageId::from(state.progress.host.fs.spelling(key))
    }

    /// The pages among `keys` with unsaved input (Q3), including an
    /// admitted operation's pages before its draft applies (a retiring
    /// draft is not input: its page is saved).
    fn unsaved(&self, keys: &BTreeSet<PageKey>) -> BTreeSet<PageKey> {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let queued = host.abstract_queue();
        keys.iter()
            .filter(|key| {
                host.pages.get(*key).is_some_and(|page| !page.clean())
                    || host.worker.as_ref().is_some_and(|w| {
                        matches!(&w.application,
                            Some(Application::Operation { pages, .. }) if pages.contains_key(*key))
                    })
                    || queued.iter().any(|request| match &request.kind {
                        RequestKind::Submit { .. } => &request.page == *key,
                        RequestKind::Move { receiver, .. } => {
                            &request.page == *key || receiver == *key
                        }
                        _ => false,
                    })
            })
            .cloned()
            .collect()
    }

    /// Wait, holding nothing, until no page in `pages` has unsaved input;
    /// a page in conflict or with a persistent save error is returned.
    fn flush(&self, pages: &BTreeSet<PageKey>) -> Result<(), BTreeSet<PageKey>> {
        let shared = &self.driver.shared;
        loop {
            let stuck: BTreeSet<_> =
                {
                    let state = shared.state.lock().unwrap();
                    let progress = &state.progress;
                    pages
                        .iter()
                        .filter(|key| {
                            progress.host.pages.get(*key).is_some_and(|page| {
                                page.conflict || progress.notice(key).save_error
                            })
                        })
                        .cloned()
                        .collect()
                };
            if !stuck.is_empty() {
                return Err(stuck);
            }
            if self.unsaved(pages).is_empty() {
                return Ok(());
            }
            let state = shared.state.lock().unwrap();
            drop(shared.wait(state, std::time::Duration::from_millis(100)));
        }
    }

    /// Stop the host (see `Drop`).
    pub(crate) fn stop(self) {}

    /// Begin a switch or restore stop (§6 step 3, §7 step 3) once the
    /// window has consumed every answer up to `consumed_last_id`: admission
    /// closes and the driver saves every dirty page it can at once, before
    /// any draft of it. A failed save puts the page at risk, which drafts it
    /// in a switch (the model's switchReq, needed by no page a save-first
    /// order leaves dirty). False: the window has more to drain.
    pub(crate) fn stop_begin(&self, consumed_last_id: u64, mode: StopMode) -> bool {
        self.driver.shared.with_state(|state| {
            let closed = state
                .progress
                .with_host(|host| host.switch_ready(consumed_last_id));
            let closed = closed == Disposition::Applied;
            if closed {
                state.progress.stopping = Some(Stopping {
                    restore: mode == StopMode::Restore,
                    failed: BTreeSet::new(),
                });
            }
            closed
        })
    }

    /// The stop's barrier (§6 step 4) and its abort condition (step 5).
    pub(crate) fn stop_state(&self) -> StopState {
        let state = self.driver.shared.state.lock().unwrap();
        stop_state(&state.progress, &state.book)
    }

    /// Abort the stop: admission reopens and the pages keep their state.
    pub(crate) fn stop_abort(&self) {
        self.driver
            .shared
            .with_state(|state| state.progress.with_host(|host| host.switch_abort()));
    }

    /// Stop the host if the stop is ready, then join the driver (§7 step
    /// 4): a delivery in flight finishes first (with admission closed, every
    /// host step runs on the driver and is delivered with it), and the
    /// watcher indexes every page again. The binding holds no host until
    /// `Stopped::relaunch` (the "restoring" state). Otherwise the host is
    /// handed back.
    pub(crate) fn stop_finish(self) -> Result<Stopped, Self> {
        let stopped = self.driver.shared.with_state(|state| {
            stop_state(&state.progress, &state.book) == StopState::Ready
                && state.progress.with_host(|host| host.switch_finish()) == Disposition::Applied
        });
        if stopped {
            let launch = self.launch.clone();
            drop(self);
            Ok(Stopped { launch })
        } else {
            Err(self)
        }
    }

    /// A whole stop for a backup restore or a switch (§6 steps 3–5, §7
    /// steps 3–4): begin it once the window consumed every answer up to
    /// `consumed_last_id`, wait for its barrier, then stop. When it cannot
    /// complete without losing custody, admission reopens and the host
    /// comes back with the affected pages (none: the window had more to
    /// drain, or trash custody cannot be listed); today's restore likewise
    /// stops when its flush fails.
    pub fn stop_saved(
        self,
        consumed_last_id: u64,
        mode: StopMode,
    ) -> Result<Stopped, (Box<Self>, BTreeSet<PageKey>)> {
        if !self.stop_begin(consumed_last_id, mode) {
            return Err((Box::new(self), BTreeSet::new()));
        }
        let mut host = self;
        loop {
            match host.stop_state() {
                StopState::Waiting => {
                    let shared = &host.driver.shared;
                    let state = shared.state.lock().unwrap();
                    drop(shared.wait(state, std::time::Duration::from_millis(100)));
                }
                StopState::Ready => match host.stop_finish() {
                    Ok(stopped) => return Ok(stopped),
                    Err(back) => host = back,
                },
                StopState::Aborted(pages) => {
                    host.stop_abort();
                    return Err((Box::new(host), pages));
                }
            }
        }
    }
}
