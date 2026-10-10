//! Clock-driven policy over the same physical phases. No model rules, timers,
//! threads or production adapters live here; the caller supplies a monotonic
//! clock, invokes with_host at mutation boundaries, and polls on wakeup.
use super::*;

pub(super) trait Clock {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(super) struct Notice {
    pub failures: u32,
    pub save_error: bool,
    /// The latest failed save's platform step and OS error code, when the
    /// filesystem named them (GH #538, Q-P2b-3).
    pub operation: Option<&'static str>,
    pub os_error: Option<i32>,
    pub draft_error: bool,
    /// An operation reported Uncertain did not happen (Q2); sticky until the
    /// page next publishes or is released.
    pub dropped: bool,
    pub conflict_reported: bool,
    /// Trash payloads whose custody error is sticky (REVIEW-2b-r2 V3): set by
    /// the escape or a third failed retirement, kept across later saves and
    /// Discard, cleared only when that marker is retired.
    pub custody_error: BTreeSet<String>,
}

struct Timing {
    page: Page,
    first: Option<u64>,
    last: u64,
    /// `save_now` asked for this page's save at once (a barrier or a block
    /// reference, §4.4): until the page is clean the debounce is skipped; a
    /// failure's backoff is not.
    urgent: bool,
    retry: Option<u64>,
    last_draft: Option<u64>,
    notice: Notice,
}

struct DraftFailure {
    pages: BTreeSet<PageKey>,
    refresh: Option<PageKey>,
    terminal: bool,
}

/// A stop in progress (STEP3 §6, §7): admission closed, and each dirty
/// page that can be saved is saved at once, before any draft of it.
pub(super) struct Stopping {
    /// Restore mode: no fallback draft is ever written (§7 step 3).
    pub restore: bool,
    /// Pages whose save failed during this stop: drafted now (a switch).
    pub failed: BTreeSet<PageKey>,
}

pub(super) fn backoff(failures: u32) -> u64 {
    let delays = [100, 300, 1000, 3000, 10000, 30000];
    delays[(failures as usize - 1).min(delays.len() - 1)]
}

impl Timing {
    fn deadline(&self) -> Option<u64> {
        self.retry.or_else(|| {
            self.first.map(|first| {
                if self.urgent {
                    return first;
                }
                (first.checked_add(1000).expect("clock exhausted"))
                    .min(self.last.checked_add(400).expect("clock exhausted"))
            })
        })
    }
}

pub(super) struct Progress<F: HostIo, C: Clock> {
    pub host: Host<F>,
    pub clock: C,
    times: BTreeMap<PageKey, Timing>,
    events_seen: usize,
    incarnation: u64,
    draft_retry: Option<u64>,
    draft_retries: u32,
    draft_errors: BTreeMap<String, DraftFailure>,
    /// Backoff for the custody listing and retire-only markers (V1, V2).
    custody_retry: Option<u64>,
    custody_retries: u32,
    /// Round-robin among due saves (STEP3 §1): the page started last.
    save_cursor: Option<PageKey>,
    /// Set by the binding when a switch or restore closes admission; ends
    /// with the switch confirmation (abort or stop).
    pub stopping: Option<Stopping>,
}

impl<F: HostIo, C: Clock> Progress<F, C> {
    pub fn new(host: Host<F>, clock: C) -> Self {
        let incarnation = host.incarnation;
        let events_seen = host.events.len();
        let mut result = Self {
            host,
            clock,
            times: BTreeMap::new(),
            events_seen,
            incarnation,
            draft_retry: None,
            draft_retries: 0,
            draft_errors: BTreeMap::new(),
            custody_retry: None,
            custody_retries: 0,
            save_cursor: None,
            stopping: None,
        };
        result.reconcile();
        result
    }

    /// Capture first/last change time at the action boundary, not at a later
    /// timer wakeup. This is also the entry point for launch and watcher reads.
    pub fn with_host<R>(&mut self, action: impl FnOnce(&mut Host<F>) -> R) -> R {
        self.reconcile();
        let result = action(&mut self.host);
        self.reconcile();
        result
    }

    pub fn notice(&self, key: &str) -> Notice {
        Notice {
            custody_error: self.host.custody_errors(key),
            ..self
                .times
                .get(key)
                .map_or(Notice::default(), |t| t.notice.clone())
        }
    }

    /// Make these pages' saves due now (`page_save_now`): only the debounce
    /// is skipped. A clean page has nothing to hurry: the next reconcile
    /// ends its urgency.
    pub fn save_now(&mut self, keys: &[PageKey]) {
        for key in keys {
            if let Some(t) = self.times.get_mut(key) {
                t.urgent = true;
            }
        }
    }

    /// Pages a surfaced draft failure names (§6 step 5).
    pub fn draft_error_pages(&self) -> BTreeSet<PageKey> {
        self.draft_errors
            .values()
            .flat_map(|failure| failure.pages.iter().cloned())
            .collect()
    }

    /// Graph-level sticky error while trash custody cannot be listed (V1).
    pub fn custody_unknown(&self) -> Option<&str> {
        self.host.custody_unknown.as_deref()
    }

    /// Wakeup required for a pending draft or custody retry; repeated early
    /// polls stutter.
    pub fn draft_retry_at(&self) -> Option<u64> {
        self.draft_retry.into_iter().chain(self.custody_retry).min()
    }

    /// The earliest time an idle host has timed work: a save or draft
    /// refresh deadline, or a retry. A page whose save or draft is blocked
    /// (conflict, reservation, a busy worker or job) contributes nothing:
    /// whatever unblocks it wakes the driver, so an expired deadline is
    /// never spun on (STEP3 §1).
    pub fn next_deadline(&self) -> Option<u64> {
        let drafts = self.host.logical_drafts();
        let pages = self.times.iter().filter(|(key, _)| !self.host.busy(key));
        let saves = pages
            .clone()
            .filter(|(key, _)| !self.host.order.gated(key))
            .filter_map(|(_, t)| t.deadline().filter(|_| !t.page.clean() && !t.page.conflict));
        let refreshes = pages.filter_map(|(key, t)| {
            let differs = t.page.risk
                && !drafts.get(key).is_some_and(|r| {
                    r.bytes == t.page.buf && r.base == t.page.base && r.version == t.page.version
                });
            t.last_draft
                .filter(|_| differs && self.host.worker.is_none() && !self.held_back(key, &t.page))
                .map(|last| last.checked_add(500).expect("clock exhausted"))
        });
        saves
            .chain(refreshes)
            .chain(self.draft_retry.filter(|_| self.host.worker.is_some()))
            .chain(self.custody_retry)
            .min()
    }

    /// Every event since the last call, removed from the host after this
    /// progress tracker has read them (D-10: the vector does not grow).
    /// During a stop, a dirty page that can be saved is saved at once and
    /// before any draft of it; it is drafted only if that save fails, so a
    /// page that cannot be saved (conflict, reserved) never waits (§6 step 3).
    fn save_first(&self, key: &str, page: &Page) -> bool {
        self.stopping
            .as_ref()
            .is_some_and(|s| !s.failed.contains(key))
            && !page.clean()
            && !page.conflict
            && !self.host.retained.contains(key)
            // R3: a page a running rename gates is carried by its draft.
            && !self.host.order.gated(key)
    }

    /// A page the stop must still try to save first: its readiness waits
    /// for that attempt even when the page already has an exact draft
    /// (V1, REVIEW-3a).
    pub fn owes_save_first(&self) -> bool {
        let pages = &self.host.pages;
        pages.iter().any(|(key, page)| self.save_first(key, page))
    }

    /// A stop drafts a page only once it cannot be saved, and a restore
    /// never writes a fallback draft (§6 step 3, §7 step 3).
    fn held_back(&self, key: &str, page: &Page) -> bool {
        self.save_first(key, page) || self.stopping.as_ref().is_some_and(|s| s.restore)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        self.reconcile();
        self.events_seen = 0;
        std::mem::take(&mut self.host.events)
    }

    fn reconcile(&mut self) {
        self.host.sync_drafts();
        let now = self.clock.now_ms();
        if self.incarnation != self.host.incarnation {
            self.times.clear();
            self.incarnation = self.host.incarnation;
            self.draft_retry = None;
            self.draft_retries = 0;
            self.draft_errors.clear();
            self.custody_retry = None;
            self.custody_retries = 0;
        }
        if self.host.switch_confirmation.is_none() {
            self.stopping = None;
        }
        self.times
            .retain(|key, _| self.host.pages.contains_key(key));
        for (key, page) in &self.host.pages {
            let t = self.times.entry(key.clone()).or_insert_with(|| Timing {
                page: page.clone(),
                first: None,
                last: now,
                urgent: false,
                retry: None,
                last_draft: None,
                notice: Notice::default(),
            });
            let changed = t.page.buf != page.buf || t.page.version != page.version;
            if page.clean() {
                t.first = None;
                t.urgent = false;
                t.retry = None;
                // An unapplied operation can fail while its old pages remain
                // clean. Keep its surfaced draft error across unrelated calls.
                t.notice = Notice {
                    draft_error: t.notice.draft_error,
                    dropped: t.notice.dropped,
                    ..Notice::default()
                };
            } else if t.first.is_none() {
                t.first = Some(now);
                t.last = now;
            } else if changed {
                t.last = now;
            }
            if !page.conflict {
                t.notice.conflict_reported = false;
            }
            t.page = page.clone();
        }
        for event in &self.host.events[self.events_seen..] {
            match event {
                Event::SaveOutcome {
                    page,
                    outcome: Outcome::Published,
                    ..
                } => {
                    let t = self.times.get_mut(page).unwrap();
                    t.notice = Notice::default();
                    t.retry = None;
                    t.first = (!t.page.clean()).then_some(now);
                    t.last = now;
                }
                Event::SaveOutcome {
                    page,
                    outcome: Outcome::Failed | Outcome::Uncertain,
                    cause,
                } => {
                    let t = self.times.get_mut(page).unwrap();
                    t.notice.operation = cause.and_then(|cause| cause.operation);
                    t.notice.os_error = cause.and_then(|cause| cause.os_error);
                    t.notice.failures = t
                        .notice
                        .failures
                        .checked_add(1)
                        .expect("failure count exhausted");
                    let delay = backoff(t.notice.failures);
                    t.retry = Some(now.checked_add(delay).expect("clock exhausted"));
                    t.notice.save_error = t.notice.failures >= 3;
                    if let Some(stopping) = &mut self.stopping {
                        stopping.failed.insert(page.clone());
                    }
                }
                Event::OperationDropped(pages) => {
                    for page in pages {
                        if let Some(t) = self.times.get_mut(page) {
                            t.notice.dropped = true;
                        }
                    }
                }
                Event::DraftError {
                    effect,
                    pages,
                    refresh,
                    ..
                } => {
                    self.draft_errors.insert(
                        effect.clone(),
                        DraftFailure {
                            pages: pages.clone(),
                            refresh: refresh.clone(),
                            terminal: false,
                        },
                    );
                }
                Event::DraftFinished {
                    effect,
                    pages,
                    refresh,
                    recovered,
                } => {
                    self.draft_errors.remove(effect);
                    if let Some(key) = refresh {
                        // A completed refresh replaces earlier terminal failed
                        // attempts at that desired record. Pending physical
                        // obligations, and unrelated operations, remain distinct.
                        self.draft_errors.retain(|_, failure| {
                            !failure.terminal || failure.refresh.as_ref() != Some(key)
                        });
                        if !recovered {
                            self.draft_errors.insert(
                                effect.clone(),
                                DraftFailure {
                                    pages: pages.clone(),
                                    refresh: refresh.clone(),
                                    terminal: true,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        self.events_seen = self.host.events.len();
        // Discard/resolution can abandon a failed refresh's desired record.
        // Only terminal work can disappear here; logical equality alone never
        // recovers a physical failure (in particular, explosion/retirement).
        self.draft_errors.retain(|_, failure| {
            !failure.terminal
                || failure
                    .refresh
                    .as_ref()
                    .is_none_or(|key| self.host.pages.get(key).is_some_and(|page| page.risk))
        });
        let drafts = self.host.logical_drafts();
        for (key, t) in &mut self.times {
            t.notice.draft_error = self.draft_errors.values().any(|f| f.pages.contains(key));
            let applied = drafts.get(key).is_some_and(|r| {
                r.bytes == t.page.buf && r.base == t.page.base && r.version == t.page.version
            });
            // The core's model-mandated conflict mailbox push stays immediate.
            // This separate persistent progress indicator waits for a draft
            // application or a surfaced failure of that draft.
            if t.page.conflict && (applied || t.notice.draft_error) {
                t.notice.conflict_reported = true;
            }
        }
    }

    /// Advance one real phase. epoch is supplied by the watcher boundary,
    /// exactly as for Host::advance_save; it is never inferred from a hash.
    pub fn poll(&mut self, epoch: u64) -> Disposition {
        self.reconcile();
        if !self.host.alive {
            return Disposition::Disabled;
        }
        let now = self.clock.now_ms();
        let draft_ready = self.draft_retry.is_none_or(|due| now >= due);
        if let Some(w) = self.host.worker.as_ref().filter(|_| draft_ready) {
            self.draft_retry = None;
            let failures = w.failures;
            let stage = w.task.stage;
            let refreshing = w.task.stage == Stage::Temp
                && matches!(w.application, Some(Application::Refresh(_)));
            let keys = w.pages.clone();
            let result = self.host.advance_draft();
            if self.host.lock_request.is_some() {
                return Disposition::Waiting;
            }
            if let Some(w) = &self.host.worker {
                if w.failures > failures && !(stage == Stage::Sync && w.task.stage == Stage::Sync) {
                    self.draft_retries = self
                        .draft_retries
                        .checked_add(1)
                        .expect("retry count exhausted");
                    self.draft_retry = Some(
                        now.checked_add(backoff(self.draft_retries))
                            .expect("clock exhausted"),
                    );
                }
            } else {
                self.draft_retries = 0;
            }
            if refreshing {
                let written_at = self.clock.now_ms();
                for key in &keys {
                    if let Some(t) = self.times.get_mut(key) {
                        t.last_draft = Some(written_at);
                    }
                }
            }
            self.reconcile();
            return result;
        }
        if self.host.job.is_some() {
            let result = self.host.advance_save(epoch);
            self.reconcile();
            return result;
        }
        if self.host.custody_unknown.is_none() && self.host.retire.is_empty() {
            self.custody_retry = None;
            self.custody_retries = 0;
        } else if self.custody_retry.is_some_and(|due| now >= due) {
            self.custody_retry = None;
            self.host.recover_custody();
            self.reconcile();
            return Disposition::Applied;
        } else if self.custody_retry.is_none() {
            self.custody_retries = self
                .custody_retries
                .checked_add(1)
                .expect("retry count exhausted");
            let delay = backoff(self.custody_retries);
            self.custody_retry = Some(now.checked_add(delay).expect("clock exhausted"));
        }
        let now = self.clock.now_ms();
        let drafts = self.host.logical_drafts();
        // Held pages and pages with draft vehicles only (D-10), in key order.
        let keys: BTreeSet<_> = self.times.keys().chain(drafts.keys()).cloned().collect();
        for key in &keys {
            let t = self.times.get(key);
            let desired = t.filter(|t| t.page.risk);
            let differs = match desired {
                Some(t) => !drafts.get(key).is_some_and(|r| {
                    r.bytes == t.page.buf && r.base == t.page.base && r.version == t.page.version
                }),
                None => drafts.contains_key(key),
            };
            let due = desired.is_none_or(|t| {
                t.last_draft
                    .is_none_or(|last| now >= last.checked_add(500).expect("clock exhausted"))
            });
            if differs && due && !desired.is_some_and(|t| self.held_back(key, &t.page)) {
                let result = self.host.begin_draft(key);
                if self.host.lock_request.is_some() {
                    return Disposition::Waiting;
                }
                if result == Disposition::Pending {
                    if let Some(t) = self.times.get_mut(key) {
                        t.last_draft = Some(now);
                    }
                    return Disposition::Pending;
                }
            }
        }
        // Due saves are served round-robin, from the page after the one
        // started last, so a page that is due again cannot starve another.
        let due: Vec<_> = self
            .times
            .iter()
            .filter(|(key, t)| {
                let due = self.save_first(key, &t.page)
                    || t.deadline().is_some_and(|deadline| now >= deadline);
                due && !t.page.clean() && !t.page.conflict
            })
            .map(|(key, _)| key.clone())
            .collect();
        let after = self.save_cursor.as_ref();
        let (later, earlier): (Vec<_>, Vec<_>) = due
            .into_iter()
            .partition(|key| after.is_none_or(|c| key > c));
        for key in later.into_iter().chain(earlier) {
            if self.host.start_save(&key) == Disposition::Pending {
                self.save_cursor = Some(key);
                return Disposition::Pending;
            }
        }
        // Model close: retire clean operation/recovery slots once their draft
        // cleanup completes, keeping window and admitted-request custody.
        let retire = self
            .host
            .pages
            .iter()
            .find(|(key, page)| {
                page.clean() && !drafts.contains_key(*key) && !self.host.busy(key)
                && !self.host.subscriptions.contains(*key)
                && !self.host.abstract_queue().iter().any(|r| r.page == **key
                    || matches!(&r.kind, RequestKind::Move { receiver, .. } if receiver == *key))
            })
            .map(|(key, _)| key.clone());
        if let Some(key) = retire {
            let keys = BTreeSet::from([key.clone()]);
            if self.host.lacks_locks(&keys) {
                return Disposition::Waiting;
            }
            self.host.with_locks(&keys, |h| h.close_clean(&key));
            self.reconcile();
            return Disposition::Applied;
        }
        // The worker's wakeup delays its effect alone. Independent graph jobs
        // above use the existing busy/allocator guards and remain serialized.
        // Keep request execution deferred while the physical worker is waiting.
        if self.host.worker.is_some() {
            return Disposition::Disabled;
        }
        // A stream of admitted requests must not postpone an overdue save.
        // Starting it preserves any already-dequeued request's custody; that
        // request resumes after the job and its draft effects reach terminal.
        if self.host.applying.is_some() {
            let result = self.host.apply_request();
            self.reconcile();
            return result;
        }
        if !self.host.queue.is_empty() {
            return self.host.dequeue();
        }
        Disposition::Disabled
    }
}
