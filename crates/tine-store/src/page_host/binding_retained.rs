//! Retained writers' reservations and the switch/restore stop (STEP3
//! §6–§7): the binding commands that fence pages away from the host.
use super::*;

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
    /// its guard check reads the writer's change.
    pub fn release(&self, reservation: Reservation) {
        self.driver.shared.with_state(|state| {
            for key in reservation.keys {
                state.progress.host.retained.remove(&key);
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

    /// The pages among `keys` with unsaved input (Q3).
    fn unsaved(&self, keys: &BTreeSet<PageKey>) -> BTreeSet<PageKey> {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let queued = host.abstract_queue();
        keys.iter()
            .filter(|key| {
                host.pages.get(*key).is_some_and(|page| !page.clean())
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
    pub fn stop(self) {}

    /// Begin a switch or restore stop (§6 step 3, §7 step 3) once the
    /// window has consumed every answer up to `consumed_last_id`: admission
    /// closes and the driver saves every dirty page it can at once, before
    /// any draft of it. A failed save puts the page at risk, which drafts it
    /// in a switch (the model's switchReq, needed by no page a save-first
    /// order leaves dirty). False: the window has more to drain.
    pub fn stop_begin(&self, consumed_last_id: u64, mode: StopMode) -> bool {
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
    pub fn stop_state(&self) -> StopState {
        let state = self.driver.shared.state.lock().unwrap();
        stop_state(&state.progress, &state.book)
    }

    /// Abort the stop: admission reopens and the pages keep their state.
    pub fn stop_abort(&self) {
        self.driver
            .shared
            .with_state(|state| state.progress.with_host(|host| host.switch_abort()));
    }

    /// Stop the host if the stop is ready, then join the driver (§7 step
    /// 4): a delivery in flight finishes first (with admission closed, every
    /// host step runs on the driver and is delivered with it), and the
    /// watcher indexes every page again. The binding holds no host until a
    /// fresh `start` (the "restoring" state). Otherwise the host is handed
    /// back.
    pub fn stop_finish(self) -> Result<(), Self> {
        let stopped = self.driver.shared.with_state(|state| {
            stop_state(&state.progress, &state.book) == StopState::Ready
                && state.progress.with_host(|host| host.switch_finish()) == Disposition::Applied
        });
        if stopped {
            drop(self);
            Ok(())
        } else {
            Err(self)
        }
    }
}
