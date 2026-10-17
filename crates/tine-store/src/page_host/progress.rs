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

/// A held page's save policy. The page itself is the host's; Timing keeps
/// only the version it last saw, for "the buffer changed" (M4).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Timing {
    version: u64,
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    fn new(page: &Page, now: u64) -> Self {
        Self {
            version: page.version,
            first: None,
            last: now,
            urgent: false,
            retry: None,
            last_draft: None,
            notice: Notice::default(),
        }
    }

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

    /// A held page's policy update at a reconcile visit: dirty onset stamps
    /// first and last, a changed buffer advances last, clean ends the save's
    /// timing and its diagnosis.
    fn visit(&mut self, page: &Page, changed: bool, now: u64) {
        if page.clean() {
            self.first = None;
            self.urgent = false;
            self.retry = None;
            // An unapplied operation can fail while its old pages remain
            // clean. Keep its surfaced draft error across unrelated calls.
            self.notice = Notice {
                draft_error: self.notice.draft_error,
                dropped: self.notice.dropped,
                ..Notice::default()
            };
        } else if self.first.is_none() {
            self.first = Some(now);
            self.last = now;
        } else if changed {
            self.last = now;
        }
        if !page.conflict {
            self.notice.conflict_reported = false;
        }
    }

    /// The draft error notice and conflict progress, after events; whether
    /// the page is stuck (in conflict, or past its third failed save).
    fn conclude(
        &mut self,
        key: &str,
        page: &Page,
        draft_errors: &BTreeMap<String, DraftFailure>,
        draft: Option<&Record>,
    ) -> bool {
        self.notice.draft_error = draft_errors.values().any(|f| f.pages.contains(key));
        // The core's model-mandated conflict mailbox push stays immediate.
        // This separate persistent progress indicator waits for a draft
        // application or a surfaced failure of that draft.
        if page.conflict && (draft.is_some_and(|r| r.holds(page)) || self.notice.draft_error) {
            self.notice.conflict_reported = true;
        }
        page.conflict || self.notice.save_error
    }
}

impl Record {
    /// This draft is exactly the page's buffer at its version.
    pub(super) fn holds(&self, page: &Page) -> bool {
        self.bytes == page.buf && self.base == page.base && self.version == page.version
    }
}

/// Apply this reconcile's events to the save policy and the surfaced draft
/// failures, in order; returns every page whose Timing or failure
/// membership they touched.
fn apply_events(
    events: &[Event],
    times: &mut BTreeMap<PageKey, Timing>,
    pages: &BTreeMap<PageKey, Page>,
    draft_errors: &mut BTreeMap<String, DraftFailure>,
    mut failed: Option<&mut BTreeSet<PageKey>>,
    now: u64,
) -> BTreeSet<PageKey> {
    let mut touched = BTreeSet::new();
    for event in events {
        match event {
            Event::SaveOutcome {
                page,
                outcome: Outcome::Published,
                ..
            } => {
                touched.insert(page.clone());
                let t = times.get_mut(page).unwrap();
                t.notice = Notice::default();
                t.retry = None;
                t.first = (!pages[page].clean()).then_some(now);
                t.last = now;
            }
            Event::SaveOutcome {
                page,
                outcome: Outcome::Failed | Outcome::Uncertain,
                cause,
            } => {
                touched.insert(page.clone());
                let t = times.get_mut(page).unwrap();
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
                if let Some(failed) = failed.as_deref_mut() {
                    failed.insert(page.clone());
                }
            }
            Event::OperationDropped(dropped) => {
                for page in dropped {
                    touched.insert(page.clone());
                    if let Some(t) = times.get_mut(page) {
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
                touched.extend(pages.iter().cloned());
                let failure = DraftFailure {
                    pages: pages.clone(),
                    refresh: refresh.clone(),
                    terminal: false,
                };
                if let Some(old) = draft_errors.insert(effect.clone(), failure) {
                    touched.extend(old.pages);
                }
            }
            Event::DraftFinished {
                effect,
                pages,
                refresh,
                recovered,
            } => {
                touched.extend(pages.iter().cloned());
                if let Some(old) = draft_errors.remove(effect) {
                    touched.extend(old.pages);
                }
                if let Some(key) = refresh {
                    // A completed refresh replaces earlier terminal failed
                    // attempts at that desired record. Pending physical
                    // obligations, and unrelated operations, remain distinct.
                    draft_errors.retain(|_, failure| {
                        let keep = !failure.terminal || failure.refresh.as_ref() != Some(key);
                        if !keep {
                            touched.extend(failure.pages.iter().cloned());
                        }
                        keep
                    });
                    if !recovered {
                        draft_errors.insert(
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
    // Discard/resolution can abandon a failed refresh's desired record.
    // Only terminal work can disappear here; logical equality alone never
    // recovers a physical failure (in particular, explosion/retirement).
    draft_errors.retain(|_, failure| {
        let keep = !failure.terminal
            || failure
                .refresh
                .as_ref()
                .is_none_or(|key| pages.get(key).is_some_and(|page| page.risk));
        if !keep {
            touched.extend(failure.pages.iter().cloned());
        }
        keep
    });
    touched
}

pub(super) struct Progress<F: HostIo, C: Clock> {
    pub host: Host<F>,
    pub clock: C,
    /// Every held page's save policy (M4: kept per visited key).
    times: BTreeMap<PageKey, Timing>,
    /// Held pages that are dirty and not in conflict: the save candidates,
    /// whatever currently blocks them (busy, gated, reserved, not due).
    saves: BTreeSet<PageKey>,
    /// Keys, held or not, whose desired draft (a page at risk, else none)
    /// differs from the live logical draft: the draft candidates, whatever
    /// currently blocks them. Empty while the host is stopped.
    drafted: BTreeSet<PageKey>,
    /// Held clean pages: the retirement candidates.
    clean: BTreeSet<PageKey>,
    /// Every key a reconcile visited since Book last took them: ownership
    /// changes for the publication consumer (B1, B2).
    moved: BTreeSet<PageKey>,
    /// The host incarnation and liveness, and whether a stop ran, that the
    /// sets were built for; a change rebuilds them.
    seen: Option<(u64, bool, bool)>,
    events_seen: usize,
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
    /// Held pages that cannot publish until someone acts: in conflict, or
    /// past their third failed save. Kept by `reconcile`, so a caller
    /// waiting for an operation's witnesses reads it per wake instead of
    /// rescanning every answered page (REVIEW-3b-P2b R5).
    stuck: BTreeSet<PageKey>,
    /// The full-pass reconcile this one replaced, run beside it and
    /// compared field by field (M4, REVIEW-progress-plan R3).
    #[cfg(test)]
    reference: Reference,
}

/// The key after `after` in `set` (the first with None).
fn next_after(set: &BTreeSet<PageKey>, after: Option<&PageKey>) -> Option<PageKey> {
    use std::ops::Bound::{Excluded, Unbounded};
    let from = after.map_or(Unbounded, Excluded);
    set.range::<PageKey, _>((from, Unbounded)).next().cloned()
}

impl<F: HostIo, C: Clock> Progress<F, C> {
    pub fn new(host: Host<F>, clock: C) -> Self {
        let events_seen = host.events.len();
        let mut result = Self {
            host,
            clock,
            times: BTreeMap::new(),
            saves: BTreeSet::new(),
            drafted: BTreeSet::new(),
            clean: BTreeSet::new(),
            moved: BTreeSet::new(),
            seen: None,
            events_seen,
            draft_retry: None,
            draft_retries: 0,
            draft_errors: BTreeMap::new(),
            custody_retry: None,
            custody_retries: 0,
            save_cursor: None,
            stopping: None,
            stuck: BTreeSet::new(),
            #[cfg(test)]
            reference: Reference::default(),
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

    /// The held pages in conflict or with a save error, as of the last
    /// host step.
    pub fn stuck(&self) -> impl Iterator<Item = &PageKey> {
        self.stuck.iter()
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
    /// is skipped. A clean page has nothing to hurry, so it is left as it
    /// is: urgency lasts while the page stays dirty (R1).
    pub fn save_now(&mut self, keys: &[PageKey]) {
        for key in keys {
            let dirty = self.host.pages.get(key).is_some_and(|page| !page.clean());
            if let Some(t) = self.times.get_mut(key).filter(|_| dirty) {
                t.urgent = true;
            }
            #[cfg(test)]
            self.reference.save_now(key);
        }
    }

    /// Pages a surfaced draft failure names (§6 step 5).
    pub fn draft_error_pages(&self) -> BTreeSet<PageKey> {
        self.draft_errors
            .values()
            .flat_map(|failure| failure.pages.iter().cloned())
            .collect()
    }

    /// Keys whose holding changed, or that a reconcile visited otherwise,
    /// since the last call: for the publication consumer's ownership (B2).
    pub fn take_moved(&mut self) -> BTreeSet<PageKey> {
        std::mem::take(&mut self.moved)
    }

    /// Graph-level sticky error while trash custody cannot be listed (V1).
    #[cfg(test)]
    pub fn custody_unknown(&self) -> Option<&str> {
        self.host.custody_unknown.as_deref()
    }

    /// Wakeup required for a pending draft or custody retry; repeated early
    /// polls stutter.
    #[cfg(test)]
    pub fn draft_retry_at(&self) -> Option<u64> {
        self.draft_retry.into_iter().chain(self.custody_retry).min()
    }

    /// The earliest time an idle host has timed work: a save or draft
    /// refresh deadline, or a retry. A page whose save or draft is blocked
    /// (conflict, reservation, a busy worker or job) contributes nothing:
    /// whatever unblocks it wakes the driver, so an expired deadline is
    /// never spun on (STEP3 §1). Walks the save and draft candidates only.
    pub fn next_deadline(&self) -> Option<u64> {
        let host = &self.host;
        let saves = self
            .saves
            .iter()
            .inspect(|_| work(Work::Candidates, 1))
            .filter(|key| !host.busy(key) && !host.order.gated(key))
            .filter_map(|key| self.times[key].deadline());
        let refreshes = self
            .drafted
            .iter()
            .filter(|_| host.worker.is_none())
            .inspect(|_| work(Work::Candidates, 1))
            .filter_map(|key| {
                let page = host.pages.get(key).filter(|page| page.risk)?;
                let t = &self.times[key];
                t.last_draft
                    .filter(|_| !host.busy(key) && !self.held_back(key, page))
                    .map(|last| last.checked_add(500).expect("clock exhausted"))
            });
        saves
            .chain(refreshes)
            .chain(self.draft_retry.filter(|_| host.worker.is_some()))
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

    /// Bring the policy up to date with what changed since the last call
    /// (M4): the host's changed keys and the pages events name. A page's
    /// policy changes only when it does (a save's outcome comes with its
    /// page's change; nothing changes a page while its job runs), so the
    /// reference tracker's full pass agrees. Work is proportional to those, plus
    /// the surfaced draft failures (E) per visited key; a new incarnation
    /// or a liveness change rebuilds from every held page and draft.
    fn reconcile(&mut self) {
        self.host.sync_drafts();
        let now = self.clock.now_ms();
        let mut visit = std::mem::take(&mut self.host.changed);
        if self.host.switch_confirmation.is_none() {
            self.stopping = None;
        }
        // A stop starting or ending moves every operation page's draft
        // candidacy (`draft_membership`).
        let stopping = self.stopping.is_some();
        let current = (self.host.incarnation, self.host.alive, stopping);
        if self.seen != Some(current) {
            if self
                .seen
                .is_some_and(|(incarnation, _, _)| incarnation != current.0)
            {
                self.times.clear();
                self.draft_retry = None;
                self.draft_retries = 0;
                self.draft_errors.clear();
                self.custody_retry = None;
                self.custody_retries = 0;
            }
            self.seen = Some(current);
            let host = &self.host;
            let known = self.saves.iter().chain(&self.clean).chain(&self.drafted);
            let known = known.chain(&self.stuck).chain(self.times.keys());
            let working = host.worker.iter().flat_map(|w| w.pages.iter());
            let keys = host.pages.keys().chain(host.durable.keys()).chain(working);
            for key in known.chain(keys).cloned().collect::<Vec<_>>() {
                visit.entry(key).or_insert(false);
            }
            self.drafted.clear();
        }
        for (key, urgent) in &visit {
            work(Work::Reconcile, 1);
            self.moved.insert(key.clone());
            let Some(page) = self.host.pages.get(key) else {
                self.times.remove(key);
                self.saves.remove(key);
                self.clean.remove(key);
                continue;
            };
            let t = self
                .times
                .entry(key.clone())
                .or_insert_with(|| Timing::new(page, now));
            let changed = t.version != page.version;
            t.visit(page, changed, now);
            t.version = page.version;
            // An operation's caller applied it: its save skips the debounce.
            if *urgent && !page.clean() {
                t.urgent = true;
            }
            let (dirty, clean) = (!page.clean() && !page.conflict, page.clean());
            for (set, member) in [(&mut self.saves, dirty), (&mut self.clean, clean)] {
                if member {
                    set.insert(key.clone());
                } else {
                    set.remove(key);
                }
            }
        }
        let events = &self.host.events[self.events_seen..];
        let failed = self.stopping.as_mut().map(|s| &mut s.failed);
        let touched = apply_events(
            events,
            &mut self.times,
            &self.host.pages,
            &mut self.draft_errors,
            failed,
            now,
        );
        #[cfg(test)]
        self.reference.reconcile(&self.host, events, &visit, now);
        self.events_seen = self.host.events.len();
        // The stopped host's drafts are its readable directory (R2): read
        // once, only if a held page needs them.
        let mut stopped = None;
        for key in visit.keys().chain(&touched) {
            work(Work::Reconcile, 1 + self.draft_errors.len());
            let Some(t) = self.times.get_mut(key) else {
                self.stuck.remove(key);
                if self.host.alive {
                    Self::draft_membership(&mut self.drafted, &self.host, stopping, key);
                }
                continue;
            };
            let page = &self.host.pages[key];
            let draft = if self.host.alive {
                self.host.draft_of(key)
            } else {
                stopped
                    .get_or_insert_with(|| self.host.logical_drafts())
                    .get(key)
            };
            if t.conclude(key, page, &self.draft_errors, draft) {
                self.stuck.insert(key.clone());
            } else {
                self.stuck.remove(key);
            }
            if self.host.alive {
                Self::draft_membership(&mut self.drafted, &self.host, stopping, key);
            }
        }
        #[cfg(test)]
        self.reference.compare(self);
    }

    /// `key` is a draft candidate iff its desired draft (the page at risk,
    /// else none) differs from its live logical draft (B1: absent keys too).
    /// RENAME-COST L1: a page with no desired draft keeps its record in its
    /// operation vehicle while that still owes a publication, except during
    /// a stop: the stop saves what it can and drafts the rest, so no owe is
    /// paid in this incarnation, and the vehicle explodes as before the
    /// change (REVIEW-rename-cost F1: a failing or gated page's owe otherwise
    /// kept a published page's record, and the stop waited forever).
    fn draft_membership(
        drafted: &mut BTreeSet<PageKey>,
        host: &Host<F>,
        stopping: bool,
        key: &str,
    ) {
        let draft = host.draft_of(key);
        let differs = match host.pages.get(key).filter(|page| page.risk) {
            Some(page) => !draft.is_some_and(|r| r.holds(page)),
            None => draft.is_some() && (stopping || !host.op_waits(key)),
        };
        if differs {
            drafted.insert(key.into());
        } else {
            drafted.remove(key);
        }
    }

    /// `poll` with the model's ghost external-write epoch for a save it
    /// advances (`Host::advance_save_at`).
    #[cfg(test)]
    pub(super) fn poll_at(&mut self, epoch: u64) -> Disposition {
        self.host.test.epoch = epoch;
        let result = self.poll();
        self.host.test.epoch = 0;
        result
    }

    /// Advance one real phase.
    pub fn poll(&mut self) -> Disposition {
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
            // A refresh's one page; an operation's set is not copied (B3).
            let refreshed = w.refresh.clone().filter(|_| refreshing);
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
            if let Some(key) = refreshed {
                let written_at = self.clock.now_ms();
                if let Some(t) = self.times.get_mut(&key) {
                    t.last_draft = Some(written_at);
                }
                #[cfg(test)]
                self.reference.drafted_at(&key, written_at);
            }
            self.reconcile();
            return result;
        }
        if self.host.job.is_some() {
            let result = self.host.advance_save();
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
        // Draft candidates, held or with draft vehicles only (D-10), in key
        // order. A stop explodes an operation vehicle only once no
        // save-first is owed (§6 step 3: save before any draft); the saves
        // usually retire it whole (REVIEW-rename-cost R3-1).
        let deferring = self.stopping.is_some() && self.owes_save_first();
        let mut next = next_after(&self.drafted, None);
        while let Some(key) = next {
            work(Work::Candidates, 1);
            let desired = self.host.pages.get(&key).filter(|page| page.risk);
            let due = desired.is_none_or(|_| {
                self.times[&key]
                    .last_draft
                    .is_none_or(|last| now >= last.checked_add(500).expect("clock exhausted"))
            });
            let later = desired.is_none() && deferring && self.host.op_waits(&key);
            if due && !later && !desired.is_some_and(|page| self.held_back(&key, page)) {
                let result = self.host.begin_draft_retiring(&key);
                if self.host.lock_request.is_some() {
                    return Disposition::Waiting;
                }
                if result == Disposition::Pending {
                    if let Some(t) = self.times.get_mut(&key) {
                        t.last_draft = Some(now);
                    }
                    #[cfg(test)]
                    self.reference.drafted_at(&key, now);
                    return Disposition::Pending;
                }
            }
            next = next_after(&self.drafted, Some(&key));
        }
        // Due saves are served round-robin, from the page after the one
        // started last, so a page that is due again cannot starve another.
        let cursor = self.save_cursor.clone();
        let mut next = next_after(&self.saves, cursor.as_ref());
        // Without a cursor, one pass from the first key.
        let mut wrapped = cursor.is_none();
        loop {
            let key = match next {
                Some(key) => key,
                None if !wrapped => {
                    wrapped = true;
                    let Some(key) = next_after(&self.saves, None) else {
                        break;
                    };
                    key
                }
                None => break,
            };
            if wrapped && cursor.as_ref().is_some_and(|c| key > *c) {
                break;
            }
            work(Work::Candidates, 1);
            let t = &self.times[&key];
            let due = self.save_first(&key, &self.host.pages[&key])
                || t.deadline().is_some_and(|deadline| now >= deadline);
            if due && self.host.start_save(&key) == Disposition::Pending {
                self.save_cursor = Some(key);
                return Disposition::Pending;
            }
            next = next_after(&self.saves, Some(&key));
        }
        // Model close: retire clean operation/recovery slots once their draft
        // cleanup completes, keeping window and admitted-request custody.
        let host = &self.host;
        let requests = || host.applying.iter().chain(&host.queue);
        let retire =
            self.clean
                .iter()
                .find(|key| {
                    work(Work::Candidates, 1 + host.queue.len());
                    host.draft_of(key).is_none() && !host.busy(key)
                && !host.subscriptions.contains(*key)
                && !requests().any(|r| r.page == **key
                    || matches!(&r.kind, RequestKind::Move { receiver, .. } if receiver == *key))
                })
                .cloned();
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

/// The full-pass reconcile M4 replaced (REVIEW-progress-plan R3), kept as
/// the oracle: its own Timing, page snapshots, surfaced failures and stuck
/// set, rebuilt over every held page with the logical drafts rebuilt from
/// the whole census (`logical_drafts`, the stopped host's readable
/// directory) at each reconcile. `compare` asserts the delta reconcile's
/// policy, candidate sets and per-key draft index agree with it.
#[cfg(test)]
#[derive(Default)]
struct Reference {
    times: BTreeMap<PageKey, Timing>,
    pages: BTreeMap<PageKey, Page>,
    draft_errors: BTreeMap<String, DraftFailure>,
    stuck: BTreeSet<PageKey>,
    logical: BTreeMap<PageKey, Record>,
    incarnation: Option<u64>,
}

#[cfg(test)]
impl Reference {
    /// R1: only a dirty held page is hurried.
    fn save_now(&mut self, key: &str) {
        let dirty = self.pages.get(key).is_some_and(|page| !page.clean());
        if let Some(t) = self.times.get_mut(key).filter(|_| dirty) {
            t.urgent = true;
        }
    }

    fn drafted_at(&mut self, key: &str, at: u64) {
        if let Some(t) = self.times.get_mut(key) {
            t.last_draft = Some(at);
        }
    }

    fn reconcile<F: HostIo>(
        &mut self,
        host: &Host<F>,
        events: &[Event],
        visit: &BTreeMap<PageKey, bool>,
        now: u64,
    ) {
        if self.incarnation != Some(host.incarnation) {
            if self.incarnation.is_some() {
                self.times.clear();
                self.pages.clear();
                self.draft_errors.clear();
            }
            self.incarnation = Some(host.incarnation);
        }
        self.times.retain(|key, _| host.pages.contains_key(key));
        self.pages.retain(|key, _| host.pages.contains_key(key));
        for (key, page) in &host.pages {
            let t = self
                .times
                .entry(key.clone())
                .or_insert_with(|| Timing::new(page, now));
            let old = self.pages.insert(key.clone(), page.clone());
            let changed = old.is_some_and(|old| old.buf != page.buf || old.version != page.version);
            t.visit(page, changed, now);
            t.version = page.version;
            if visit.get(key) == Some(&true) && !page.clean() {
                t.urgent = true;
            }
        }
        apply_events(
            events,
            &mut self.times,
            &host.pages,
            &mut self.draft_errors,
            None,
            now,
        );
        self.logical = host.logical_drafts();
        self.stuck.clear();
        for (key, t) in &mut self.times {
            if t.conclude(
                key,
                &self.pages[key],
                &self.draft_errors,
                self.logical.get(key),
            ) {
                self.stuck.insert(key.clone());
            }
        }
    }

    fn compare<F: HostIo, C: Clock>(&self, p: &Progress<F, C>) {
        let host = &p.host;
        assert_eq!(p.times, self.times, "M4 reference: Timing");
        assert_eq!(
            p.draft_errors, self.draft_errors,
            "M4 reference: draft errors"
        );
        assert_eq!(p.stuck, self.stuck, "M4 reference: stuck");
        let pages = self.pages.iter();
        let saves = pages
            .clone()
            .filter(|(_, page)| !page.clean() && !page.conflict);
        let clean = pages.filter(|(_, page)| page.clean());
        let keys = |pages: &mut dyn Iterator<Item = (&PageKey, &Page)>| {
            pages.map(|(key, _)| key.clone()).collect::<BTreeSet<_>>()
        };
        assert_eq!(
            p.saves,
            keys(&mut { saves }),
            "M4 reference: save candidates"
        );
        assert_eq!(p.clean, keys(&mut { clean }), "M4 reference: clean pages");
        // RENAME-COST L1, rebuilt from the census and the pages: a page
        // waits while an operation vehicle naming it owes a publication.
        let mut live_ops = BTreeMap::new();
        for (name, records) in host.drafts.iter().filter(|(n, _)| n.starts_with("op-")) {
            let versions: BTreeMap<PageKey, u64> = records
                .iter()
                .map(|r| (r.page.clone(), r.version))
                .collect();
            let owed = versions
                .iter()
                .filter(|(key, version)| logical::owes(host.pages.get(*key), **version))
                .map(|(key, _)| key.clone())
                .collect();
            live_ops.insert(name.clone(), LiveOp { versions, owed });
        }
        assert_eq!(
            host.live_ops, live_ops,
            "RENAME-COST reference: live operation vehicles"
        );
        let op_waits = |key: &PageKey| {
            live_ops
                .values()
                .any(|op| op.versions.contains_key(key) && !op.owed.is_empty())
        };
        let drafted: BTreeSet<_> = if host.alive {
            let candidates = self.pages.keys().chain(self.logical.keys());
            let differs = |key: &&PageKey| match self.pages.get(*key).filter(|page| page.risk) {
                Some(page) => !self.logical.get(*key).is_some_and(|r| r.holds(page)),
                None => self.logical.contains_key(*key) && (p.stopping.is_some() || !op_waits(key)),
            };
            candidates.filter(differs).cloned().collect()
        } else {
            BTreeSet::new()
        };
        assert_eq!(p.drafted, drafted, "M4 reference: draft candidates");
        assert_eq!(
            host.durable,
            drafts::logical(host.drafts.values()),
            "M4 reference: per-key draft index"
        );
        let mut vehicles_of: BTreeMap<PageKey, BTreeSet<String>> = BTreeMap::new();
        for (name, records) in &host.drafts {
            for record in records {
                vehicles_of
                    .entry(record.page.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
        assert_eq!(host.vehicles_of, vehicles_of, "M4 reference: vehicle index");
    }
}
