//! Clock-driven policy over the same physical phases. No model rules, timers,
//! threads or production adapters live here; the caller supplies a monotonic
//! clock, invokes with_host at mutation boundaries, and polls on wakeup.
use super::*;

pub(super) trait Clock {
    fn now_ms(&self) -> u64;
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct Notice {
    pub failures: u32,
    pub save_error: bool,
    pub draft_error: bool,
    pub conflict_reported: bool,
}

struct Timing {
    page: Page,
    first: Option<u64>,
    last: u64,
    retry: Option<u64>,
    last_draft: Option<u64>,
    notice: Notice,
}

impl Timing {
    fn deadline(&self) -> Option<u64> {
        self.retry.or_else(|| {
            self.first.map(|first| {
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
        self.times.get(key).map_or(Notice::default(), |t| t.notice)
    }

    fn reconcile(&mut self) {
        let now = self.clock.now_ms();
        if self.incarnation != self.host.incarnation {
            self.times.clear();
            self.incarnation = self.host.incarnation;
        }
        self.times
            .retain(|key, _| self.host.pages.contains_key(key));
        for (key, page) in &self.host.pages {
            let t = self.times.entry(key.clone()).or_insert_with(|| Timing {
                page: page.clone(),
                first: None,
                last: now,
                retry: None,
                last_draft: None,
                notice: Notice::default(),
            });
            let changed = t.page.buf != page.buf || t.page.version != page.version;
            if page.clean() {
                t.first = None;
                t.retry = None;
                // An unapplied operation can fail while its old pages remain
                // clean. Keep its surfaced draft error across unrelated calls.
                t.notice = Notice {
                    draft_error: t.notice.draft_error,
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
                } => {
                    let t = self.times.get_mut(page).unwrap();
                    t.notice.failures = t
                        .notice
                        .failures
                        .checked_add(1)
                        .expect("failure count exhausted");
                    let backoff = [100, 300, 1000, 3000, 10000, 30000];
                    let delay = backoff[(t.notice.failures as usize - 1).min(backoff.len() - 1)];
                    t.retry = Some(now.checked_add(delay).expect("clock exhausted"));
                    t.notice.save_error = t.notice.failures >= 3;
                }
                Event::DraftError { pages, .. } => {
                    for key in pages {
                        if let Some(t) = self.times.get_mut(key) {
                            t.notice.draft_error = true;
                        }
                    }
                }
                _ => {}
            }
        }
        self.events_seen = self.host.events.len();
        let drafts = self.host.logical_drafts();
        for (key, t) in &mut self.times {
            let applied = drafts.get(key).is_some_and(|r| {
                r.bytes == t.page.buf && r.base == t.page.base && r.version == t.page.version
            });
            if applied {
                t.notice.draft_error = false;
            }
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
        if let Some(w) = &self.host.worker {
            let failed = w.task.stage == Stage::Absent
                && matches!(w.application, Some(Application::Refresh(_)));
            let refreshing = w.task.stage == Stage::Temp
                && matches!(w.application, Some(Application::Refresh(_)));
            let keys = w.pages.clone();
            let result = self.host.advance_draft();
            if refreshing {
                let written_at = self.clock.now_ms();
                for key in &keys {
                    if let Some(t) = self.times.get_mut(key) {
                        t.last_draft = Some(written_at);
                    }
                }
            }
            if failed {
                for key in keys {
                    if let Some(t) = self.times.get_mut(&key) {
                        t.notice.draft_error = true;
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
        let now = self.clock.now_ms();
        let drafts = self.host.logical_drafts();
        for key in &self.host.keys.clone() {
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
            if differs && due && self.host.begin_draft(key) == Disposition::Pending {
                if let Some(t) = self.times.get_mut(key) {
                    t.last_draft = Some(now);
                }
                return Disposition::Pending;
            }
        }
        for key in &self.host.keys.clone() {
            if self.times.get(key).is_some_and(|t| {
                !t.page.clean() && !t.page.conflict && t.deadline().is_some_and(|due| now >= due)
            }) && self.host.start_save(key) == Disposition::Pending
            {
                return Disposition::Pending;
            }
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
