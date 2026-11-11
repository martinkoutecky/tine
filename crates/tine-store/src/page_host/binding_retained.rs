//! Save-first input handling and switch/restore stop.
use super::driver::{Deadline, Shared, State, Wait};
use super::*;
use crate::model::entry_identity::Identity;

/// Why a host operation's input could not be saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::page_host) enum FlushRefusal {
    /// These pages have unsaved input (`Input::Refuse`), or their save
    /// cannot complete (`Input::Flush`), named by current spelling.
    Unsaved(BTreeSet<PageId>),
    /// A rename still finishing held one of the pages for `OPERATION_WAIT`
    /// from the caller's start (M6); nothing was admitted.
    Busy,
}

type HostShared = Shared<ProductionIo, SystemClock>;
pub(super) type HostState = State<ProductionIo, SystemClock>;

/// The stall bound of a host operation's caller (Finding B, R1, L2-06), not
/// the longest it waits. From the caller's start it limits how long a page
/// busy in another operation delays admission (`Busy`); a save-first flush
/// past it refuses `Busy` only for a rename still finishing, and otherwise
/// keeps waiting (`bounded`), after which admission gets a new budget. Once
/// admitted, `settle` moves its deadline to this long after the answer and
/// after each page's newly counted publication, so an operation that keeps
/// progressing is waited for beyond it. A wait the driver is mid-ask on at
/// its deadline gets `ASK_GRACE` more. `Deadline::Never` reads (naming a
/// page, `register_all`) are in no bound.
///
/// Enforced by: `an_operation_over_a_running_order_is_busy_until_the_order_ends`
/// (admission `Busy`), `a_flush_handed_to_a_rename_is_told_busy_after_the_bound`
/// (the flush refusal), `a_moving_deadline_keeps_the_caller_waiting_only_while_the_step_moves_it`
/// (a moved deadline outlives the first bound) and
/// `an_answer_given_across_the_deadline_reaches_the_caller` (`ASK_GRACE`).
/// A held page's read waits as long for its owner (`watch::OBSERVED_WAIT`).
pub const OPERATION_WAIT: std::time::Duration = crate::watch::OBSERVED_WAIT;

/// What a caller's admitted operation came to, by precedence: Superseded,
/// Applied, Unwritten, Uncertain (STEP3-DESIGN option 2; plan v3 N6).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Settled {
    Applied,
    DraftFailed,
    Unwritten(PageKey),
    Superseded(PageKey),
    Uncertain,
}

/// A caller's reply slot. Only its holder removes it, here, on every exit
/// (an unwind included), so a late answer creates nothing (R2). Made on
/// the driver with the operation's admission: a slot whose caller already
/// left is dropped there, and removed the same way.
pub(super) struct ReplySlot {
    shared: std::sync::Weak<HostShared>,
    id: ReplyId,
}

impl ReplySlot {
    pub(super) fn new(shared: std::sync::Weak<HostShared>, id: ReplyId) -> Self {
        Self { shared, id }
    }
}

impl Drop for ReplySlot {
    fn drop(&mut self) {
        // A notice: a driver that ended has no slot left to answer.
        let id = self.id;
        if let Some(shared) = self.shared.upgrade() {
            shared.notify(move |state| {
                let _ = state.progress.host.order.remove(id);
            });
        }
    }
}

/// One step of save-first input handling, decided on the driver
/// ([`PageHost::bounded`]).
enum Step<R> {
    Done(R),
    /// Waiting on a busy page; `renaming`: on a rename still finishing.
    Wait {
        renaming: bool,
    },
}

/// The pages among `keys` with unsaved input (Q3), including an admitted
/// operation's pages before its draft applies (a retiring draft is not
/// input: its page is saved), each with its current spelling.
pub(in crate::page_host) fn unsaved<F: HostIo>(
    host: &Host<F>,
    keys: &BTreeSet<PageKey>,
) -> BTreeMap<PageKey, PageId> {
    keys.iter()
        .filter(|key| match host.logical(key) {
            Logical::Settled(page) => !page.clean(),
            Logical::Admitted { .. } => true,
            Logical::Absent => false,
        })
        .map(|key| (key.clone(), PageId::from(host.fs.spelling(key))))
        .collect()
}

impl PageHost {
    /// `pages`' keys, each registered; None when the host state is
    /// poisoned.
    pub(super) fn register_all(&self, pages: Vec<PageId>) -> Option<BTreeSet<PageKey>> {
        pages
            .iter()
            .map(|page| {
                let (key, spelling, ..) = self.try_identify(page)?;
                self.try_register(&key, &spelling)?;
                Some(key)
            })
            .collect()
    }

    /// Every save-first input wait and check (M6,
    /// REVIEW-3b-P2b-2 N1): the driver runs `step` until it is done, again
    /// after each of its steps. With each step: a host that cannot answer
    /// refuses `Busy` (the driver ended, or the host is down; in-scope
    /// scenario: a host that failed on a disk error), and past `deadline` a
    /// step waiting on a rename still finishing refuses `Busy` (in-scope
    /// scenario: a disk error keeps failing its drafts or writes, or a sync
    /// service or external editor conflicted one of its pages). A step
    /// waiting on any other busy page keeps waiting.
    fn bounded<R: Send + 'static>(
        &self,
        deadline: std::time::Instant,
        mut step: impl FnMut(&mut HostState) -> Step<R> + Send + 'static,
    ) -> Result<R, FlushRefusal> {
        let wait = move |state: &mut HostState| {
            if !state.progress.host.alive {
                return Wait::Ready(Err(FlushRefusal::Busy));
            }
            match step(state) {
                Step::Done(done) => Wait::Ready(Ok(done)),
                Step::Wait { renaming } => {
                    let now = std::time::Instant::now();
                    if now >= deadline && renaming {
                        return Wait::Ready(Err(FlushRefusal::Busy));
                    }
                    Wait::Until((now < deadline).then_some(deadline))
                }
            }
        };
        let answer = self.driver.shared.wait(Deadline::Never, wait);
        answer.unwrap_or(Err(FlushRefusal::Busy))
    }

    /// Wait for a caller's admitted operation (Finding B): its answer, then
    /// each answered page's own publication at the answered version or later.
    /// Never success without both; Uncertain at `deadline`, once the driver
    /// ended or the state is poisoned, once the host stopped or relaunched,
    /// or once window `session` is not current (F1: a result never vouches
    /// for another session's pages). The deadline bounds a stall, not the
    /// operation: the answer and each newly published page move it to
    /// `OPERATION_WAIT` from then, so a long operation that keeps publishing
    /// (a rename with a thousand referrers on Windows) is waited for. The
    /// caller waits on the same moving deadline, so a driver stalled in a
    /// disk call answers `Uncertain` too.
    pub(super) fn settle(
        &self,
        slot: &ReplySlot,
        session: Option<u64>,
        deadline: std::time::Instant,
    ) -> Settled {
        let deadline = Arc::new(Mutex::new(deadline));
        let moving = deadline.clone();
        let id = slot.id;
        // (answered, pages synced, pages indexed) at the last ask.
        let mut progress = (false, 0, 0);
        let step = move |state: &mut HostState| {
            let host = &state.progress.host;
            let Some(reply) = host.order.slot(id) else {
                return Wait::Ready(Settled::Uncertain);
            };
            if session.is_some_and(|session| session != state.book.session) {
                return Wait::Ready(Settled::Uncertain);
            }
            match &reply.reply {
                Some(OperationReply::DraftFailed) => return Wait::Ready(Settled::DraftFailed),
                Some(OperationReply::Applied(pages)) => {
                    if let Some(page) = &reply.superseded {
                        return Wait::Ready(Settled::Superseded(page.clone()));
                    }
                    let now = (true, reply.witnessed.len(), reply.indexed.len());
                    if now != progress {
                        progress = now;
                        let mut at = moving.lock().unwrap();
                        *at = (*at).max(std::time::Instant::now() + OPERATION_WAIT);
                    }
                    // Both sets hold answered pages only. A disk sync can
                    // precede a deferred or failed index delivery, so neither
                    // alone proves the operation complete to its caller.
                    if reply.witnessed.len() == pages.len() && reply.indexed.len() == pages.len() {
                        return Wait::Ready(Settled::Applied);
                    }
                    // Per ask, the host's few stuck pages, not every answered
                    // one: an operation's asks grow with its pages (R5).
                    let mut stuck = state.progress.stuck().inspect(|_| {
                        #[cfg(feature = "test-faults")]
                        crate::cost_counters::host_publication_probes(1);
                    });
                    let open = |key: &&PageKey| {
                        pages.contains_key(*key) && !reply.witnessed.contains(*key)
                    };
                    if let Some(page) = stuck.find(open) {
                        return Wait::Ready(Settled::Unwritten(page.clone()));
                    }
                    #[cfg(feature = "test-faults")]
                    crate::cost_counters::host_publication_probes(1);
                }
                None => {}
            }
            let now = std::time::Instant::now();
            let at = *moving.lock().unwrap();
            if !host.alive || host.incarnation != id.incarnation || now >= at {
                return Wait::Ready(Settled::Uncertain);
            }
            Wait::Until(Some(at))
        };
        let settled = self.driver.shared.wait(Deadline::Moving(deadline), step);
        settled.unwrap_or(Settled::Uncertain)
    }

    /// The key naming `page`'s directory entry (§2), by the one identity
    /// rule (B1, [`crate::model::Graph::identify`]): a registered key the
    /// path names, or whose entry it reaches as an alias (Q4's test: the
    /// same file, not listed apart); else a key for the new entry
    /// ([`super::Host::key_for`]: never a live key spelling another entry).
    /// Also whether the host holds that page, and whether the path is proved
    /// to name that key's entry. None when the driver ended.
    pub(super) fn try_identify(&self, page: &PageId) -> Option<(PageKey, PageId, bool, bool)> {
        let graph = &self.store.graph;
        let ended = std::cell::Cell::new(false);
        let registered = |fold: &str| {
            let fold = fold.to_owned();
            let read = move |state: &HostState| state.progress.host.candidates(&fold);
            self.driver.shared.read(read).unwrap_or_else(|_| {
                ended.set(true);
                Vec::new()
            })
        };
        let identity = graph.identify(&graph.root.join(page.as_str()), &registered);
        // The path names the key's entry: its own leaf, Q4's same-file alias,
        // or no registered collision. An unproved collision or a path outside
        // the root does not (step 3b P1: `page_open`'s baseline entry).
        let proved = !matches!(
            identity,
            Identity::Outside | Identity::Unknown { alias: None, .. }
        );
        let named = match identity {
            Identity::Key(key)
            | Identity::Unknown {
                alias: Some(key), ..
            } => Some(key),
            _ => None,
        };
        let disk = self
            .store
            .disk_spelling(page)
            .unwrap_or_else(|| page.clone());
        if ended.get() {
            return None;
        }
        let read = move |state: &HostState| {
            let host = &state.progress.host;
            let (key, spelling) = match named {
                Some(key) => (key.clone(), PageId::from(host.fs.spelling(&key))),
                None => (host.key_for(disk.as_str()), disk),
            };
            let held = host.pages.contains_key(&key);
            (key, spelling, held, proved)
        };
        self.driver.shared.read(read).ok()
    }

    /// Register `key` at `spelling` with its page lock; None when the
    /// driver ended.
    pub(super) fn try_register(&self, key: &str, spelling: &PageId) -> Option<()> {
        let graph = &self.store.graph;
        let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
        let (key, spelling) = (key.to_owned(), spelling.clone());
        let register = move |state: &mut HostState| {
            state
                .progress
                .with_host(|host| host.register(key, spelling.as_str(), lock))
        };
        self.driver.shared.call(register).ok()
    }

    /// `keys`' current spellings, the pages a refusal names (REVIEW-3a5):
    /// a key is opaque, and need not be its page's spelling.
    fn spellings(&self, keys: &BTreeSet<PageKey>) -> BTreeSet<PageId> {
        let keys = keys.clone();
        let read = move |state: &HostState| {
            let fs = &state.progress.host.fs;
            keys.iter()
                .map(|key| PageId::from(fs.spelling(key)))
                .collect()
        };
        // A driver that ended meanwhile names none (`StopState::Ended`).
        self.driver.shared.read(read).unwrap_or_default()
    }

    /// A key's current spelling, as the page it names.
    pub(super) fn spelling(&self, key: &str) -> PageId {
        let key = key.to_owned();
        let read = move |state: &HostState| PageId::from(state.progress.host.fs.spelling(&key));
        self.driver
            .shared
            .read(read)
            .expect("page host driver ended")
    }

    /// Wait, holding nothing, until no page in `pages` has unsaved input,
    /// within the writer's `deadline` (N1); a page in conflict or with a
    /// persistent save error refuses the writer, by its spelling. The rename
    /// dependency is classified over the writer's whole set, `reserving`, not
    /// only the pages it waits to see saved (REVIEW-3b-P2b-3): a rename
    /// holding a clean page of the set bounds the wait too.
    pub(super) fn flush(
        &self,
        pages: &BTreeSet<PageKey>,
        reserving: &BTreeSet<PageKey>,
        deadline: std::time::Instant,
    ) -> Result<(), FlushRefusal> {
        #[cfg(test)]
        crate::store::pause_at_hook(&self.driver.shared.flush_pause);
        let (pages, reserving) = (pages.clone(), reserving.clone());
        self.bounded(deadline, move |state| {
            let progress = &state.progress;
            let host = &progress.host;
            let stuck: BTreeSet<PageId> = pages
                .iter()
                .filter(|key| host.logical(key).conflict() || progress.notice(key).save_error)
                .map(|key| PageId::from(host.fs.spelling(key)))
                .collect();
            if !stuck.is_empty() {
                return Step::Done(Err(FlushRefusal::Unsaved(stuck)));
            }
            if unsaved(host, &pages).is_empty() {
                return Step::Done(Ok(()));
            }
            Step::Wait {
                renaming: host.renaming(&reserving),
            }
        })?
    }

    /// Stop the host (see `Drop`).
    #[cfg(test)]
    pub(crate) fn stop(self) {}

    /// Begin a switch or restore stop (§6 step 3, §7 step 3) once the
    /// window has consumed every answer up to `consumed_last_id`: admission
    /// closes and the driver saves every dirty page it can at once, before
    /// any draft of it. A failed save puts the page at risk, which drafts it
    /// in a switch (the model's switchReq, needed by no page a save-first
    /// order leaves dirty). False: the window has more to drain, or the
    /// driver ended (nothing is left to stop).
    pub(crate) fn stop_begin(&self, consumed_last_id: u64, mode: StopMode) -> bool {
        let begin = self.driver.shared.call(move |state| {
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
        });
        begin.unwrap_or(false)
    }

    /// The stop's barrier (§6 step 4) and its abort condition (step 5);
    /// `Ended` once the driver ended.
    pub(crate) fn stop_state(&self) -> StopState {
        let read = |state: &HostState| stop_state(&state.progress, &state.book);
        self.driver.shared.read(read).unwrap_or(StopState::Ended)
    }

    /// Stop a host no window owns any more (plan v3 §3, `HostRetirement`;
    /// legal as the model's `windowCrash`, `storage-s3.qnt:588-590`). Unless
    /// a stop already began (the window's own, kept), the window's session
    /// ends, once per abandoned owner: its mail is dropped, nothing more is
    /// admitted, and every admitted request keeps applying with custody.
    /// Once they have all applied, a switch stop begins over that watermark
    /// (a stale request's answer never enters the outbox, so none is owed).
    /// Returns where the stop stands: the caller calls `stop_finish` once
    /// the stop may complete, or `stop_abort` on `Aborted` and asks again
    /// later. While known draft vehicles are cleanup debt (REVIEW-3b-P1
    /// R2), each call first re-probes draft I/O, the Retry no window is
    /// left to press, so a repaired filesystem lets the stop complete. A
    /// driver that ended (a host bug) answers `Ended`: the caller ends the
    /// host with `stop_finish`.
    pub fn orphan_stop(&self) -> StopState {
        let debt = |state: &HostState| !state.progress.host.vehicle_debt.is_empty();
        match self.driver.shared.read(debt) {
            Ok(true) => {
                let _ = self.drafts_retry();
            }
            Ok(false) => {}
            Err(_) => return StopState::Ended,
        }
        let stopped = self.driver.shared.call(|state| {
            if state.progress.host.switch_confirmation.is_none() {
                if !state.book.orphaned {
                    state.book.orphaned = true;
                    state.book.session = next_session();
                    state.progress.with_host(|host| host.window_crash());
                }
                let ready = state.progress.with_host(|host| {
                    let last = host.last_admitted;
                    host.switch_ready(last)
                });
                if ready == Disposition::Applied {
                    state.progress.stopping = Some(Stopping {
                        restore: false,
                        failed: BTreeSet::new(),
                    });
                }
            }
            stop_state(&state.progress, &state.book)
        });
        stopped.unwrap_or(StopState::Ended)
    }

    /// Whether the driver ended (a host bug panicked it): the host answers
    /// nothing more, and only `stop_finish` is left for it. A reopen of
    /// its retiring root never adopts it.
    pub fn ended(&self) -> bool {
        self.driver.shared.ended()
    }

    /// Point page mail at the window that now owns the binding (plan v3 §3,
    /// S2: an adopting window). Mail already collected keeps the session it
    /// was collected under.
    pub fn retarget(&self, mail: impl FnMut(PageMail) + Send + 'static) {
        *self.launch.mail.lock().unwrap() = Box::new(mail);
    }

    /// Abort the stop: admission reopens and the pages keep their state.
    pub fn stop_abort(&self) {
        let abort = |state: &mut HostState| state.progress.with_host(|host| host.switch_abort());
        // A host that ended has no stop left to abort.
        let _ = self.driver.shared.call(abort);
    }

    /// Stop the host if its stop may complete now, then join the driver (§7 step
    /// 4): a delivery in flight finishes first (with admission closed, every
    /// host step runs on the driver and is delivered with it), and the
    /// watcher indexes every page again. The binding holds no host until
    /// `Stopped::relaunch`. Otherwise the host is handed back.
    ///
    /// A host whose driver ended (`StopState::Ended`, a host bug) stops here
    /// for good, which never fails: the driver is joined and every page it
    /// held goes back to the watcher, which rereads it from disk (s3b O6);
    /// every draft it wrote stays on disk, where the next host of this graph
    /// recovers it. The driver's panic was reported as it happened and does
    /// not go on here, unlike the host's drop.
    pub fn stop_finish(self) -> Result<Stopped, Self> {
        let stopped = self.driver.shared.call(|state| {
            stop_state(&state.progress, &state.book) == StopState::Ready
                && state.progress.with_host(|host| host.switch_finish()) == Disposition::Applied
        });
        let launch = self.launch.clone();
        match stopped {
            Ok(true) => {
                drop(self);
                Ok(Stopped { launch })
            }
            Ok(false) => Err(self),
            // Unanswered with no deadline: the driver ended.
            Err(_) => {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(self)));
                Ok(Stopped { launch })
            }
        }
    }

    /// A whole stop for a backup restore or a switch (§6 steps 3–5, §7
    /// steps 3–4): begin it once the window consumed every answer up to
    /// `consumed_last_id`, wait for its barrier, then stop. When it cannot
    /// complete without losing custody, admission reopens and the host
    /// comes back with the affected pages (none: the window had more to
    /// drain, trash custody cannot be listed, or the driver ended), named by
    /// their current spellings; today's restore likewise stops when its
    /// flush fails.
    pub fn stop_saved(
        self,
        consumed_last_id: u64,
        mode: StopMode,
    ) -> Result<Stopped, (Box<Self>, BTreeSet<PageId>)> {
        if !self.stop_begin(consumed_last_id, mode) {
            return Err((Box::new(self), BTreeSet::new()));
        }
        let mut host = self;
        loop {
            match host.stop_state() {
                StopState::Waiting => {
                    // Until the barrier moves: asked after each driver step.
                    let moved =
                        |state: &mut HostState| match stop_state(&state.progress, &state.book) {
                            StopState::Waiting => Wait::Until(None),
                            _ => Wait::Ready(()),
                        };
                    let _ = host.driver.shared.wait(Deadline::Never, moved);
                }
                StopState::Ready => match host.stop_finish() {
                    Ok(stopped) => return Ok(stopped),
                    Err(back) => host = back,
                },
                StopState::Aborted(pages) => {
                    let pages = host.spellings(&pages);
                    host.stop_abort();
                    return Err((Box::new(host), pages));
                }
                // Nothing closes over a dead host's lost work; its window's
                // close retires it (`stop_finish`).
                StopState::Ended => return Err((Box::new(host), BTreeSet::new())),
            }
        }
    }
}
