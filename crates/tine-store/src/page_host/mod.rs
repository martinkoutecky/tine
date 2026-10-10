//! Unwired page owner. Runtime code is independent of the page_state oracle.
#![allow(dead_code)]

mod binding;
pub use binding::{
    DiskToken, Input, Opened, PageHost, PageMail, PageOperation, PageRefusal, Reloaded,
    RenameRefusal, Reservation, StopMode, StopState, Stopped,
};
pub use io::DraftStatus;
#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod conformance;
mod draft_worker;
mod drafts;
mod driver;
#[cfg(test)]
mod driver_tests;
#[cfg(feature = "test-faults")]
pub mod faults;
mod io;
#[cfg(test)]
mod model_fs;
#[cfg(test)]
mod native_cost;
mod operations;
mod order;
mod production;
#[cfg(test)]
mod production_tests;
mod progress;
mod save;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod writer_census_tests;

use drafts::{Record, Stage, Vehicle};
use io::{ErrorKind, HostIo, IoFailure, Witness};
use order::{Gates, OperationReply, Reply, ReplyId};
use save::SaveJob;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

type Text = Option<Arc<[u8]>>;
type PageKey = String;

/// The registered keys (STEP3 §2). Every visit goes through `iter`, which a
/// cost test counts: an operation visits no registered key it does not touch
/// (A-R5, D-10). Membership is a lookup, not a visit.
#[derive(Clone, Default)]
struct Keys(BTreeSet<PageKey>);

// `Keys::iter` visits on this thread (A-R5's counting test).
#[cfg(test)]
thread_local! {
    static KEY_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Keys {
    fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }

    fn insert(&mut self, key: PageKey) -> bool {
        self.0.insert(key)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether every key of `set` is registered, by lookup.
    fn includes(&self, set: &BTreeSet<PageKey>) -> bool {
        set.iter().all(|key| self.0.contains(key))
    }

    fn iter(&self) -> impl Iterator<Item = &PageKey> {
        self.0.iter().inspect(|_| {
            #[cfg(test)]
            KEY_VISITS.with(|n| n.set(n.get() + 1));
        })
    }
}

/// A4 trash custody one page owes: its markers on disk (name → payload
/// basename) whose phases (a) and (b) have not completed in this incarnation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Debt {
    markers: BTreeMap<String, String>,
    /// Consecutive filesystem-reported custody failures before a save.
    failures: u32,
}

/// REVIEW-2b-r2 V2: a marker whose custody completed but whose unlink and
/// directory sync failed. Only retirement is owed; it never blocks a save.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Retire {
    page: PageKey,
    payload: String,
    /// Filesystem-reported retirement failures.
    failures: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Base {
    Known(Text),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Page {
    buf: Text,
    base: Base,
    version: u64,
    typed: bool,
    risk: bool,
    conflict: bool,
    /// Outer None means no observation; inner None means observed no file.
    obs: Option<Text>,
}

impl Page {
    fn clean(&self) -> bool {
        !self.typed && !self.risk && self.base == Base::Known(self.buf.clone())
    }

    fn submit(&mut self, bytes: Text, base_version: u64, version: u64) {
        if self.version != base_version {
            self.base = Base::Unknown;
            self.conflict = true;
            self.risk = true;
        }
        self.buf = bytes;
        self.typed = true;
        self.version = version;
    }

    /// SPEC read table: compare base, cleanliness, then buffer, in that order.
    fn observe(&mut self, bytes: Text, version: u64) {
        self.obs = Some(bytes.clone());
        if self.base == Base::Known(bytes.clone()) {
            self.conflict = false;
        } else if self.clean() {
            self.buf = bytes.clone();
            self.base = Base::Known(bytes);
            self.version = version;
            self.conflict = false;
            self.typed = false;
        } else if self.buf == bytes {
            self.base = Base::Known(bytes);
            self.conflict = false;
        } else {
            self.conflict = true;
            self.risk = true;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RequestKind {
    Open,
    Close,
    Submit {
        bytes: Text,
        version: u64,
        resolve: Option<Text>,
    },
    Discard {
        version: u64,
    },
    Move {
        receiver: PageKey,
        source_text: Text,
        receiver_text: Text,
        source_version: u64,
        receiver_version: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Request {
    id: u64,
    generation: u64,
    page: PageKey,
    kind: RequestKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Answer {
    id: u64,
    generation: u64,
    version: u64,
    took: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Mail {
    page: Option<Page>,
    answer: Option<Answer>,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SavePhase {
    /// A4 rule 4: the page's earlier trash custody, before anything else.
    Custody,
    Temp,
    Check,
    /// A4 rule 2: a deletion's custody marker, after the guard and before the move.
    Marker,
    Rename,
    TrashSync,
    DirectorySync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Published,
    Uncertain,
    Failed,
}

/// Events originate here, never from an oracle's expected successor.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Loaded {
        page: PageKey,
        version: u64,
    },
    Answer {
        page: PageKey,
        answer: Answer,
    },
    Draft(Record),
    DraftRemoved(PageKey),
    Published {
        page: PageKey,
        bytes: Text,
        version: u64,
        epoch: u64,
    },
    Renamed {
        page: PageKey,
        bytes: Text,
        version: u64,
    },
    SaveOutcome {
        page: PageKey,
        outcome: Outcome,
        /// A failed step's I/O diagnosis, for the notice (Q-P2b-3).
        cause: Option<IoFailure>,
    },
    Removed {
        page: PageKey,
        bytes: Text,
    },
    DeleteDurable {
        page: PageKey,
        bytes: Text,
    },
    OperationRead {
        page: PageKey,
        base: Base,
    },
    DraftError {
        effect: String,
        pages: BTreeSet<PageKey>,
        refresh: Option<PageKey>,
        failures: u32,
    },
    DraftFinished {
        effect: String,
        pages: BTreeSet<PageKey>,
        refresh: Option<PageKey>,
        recovered: bool,
    },
    Unreadable(String),
    /// A4 escape: a save goes ahead past this trash file's unfinished custody.
    CustodyError {
        page: PageKey,
        payload: String,
    },
    /// A driver observation (watcher read or released reservation) failed a
    /// third time; it keeps retrying with the save backoff (STEP3 §7).
    ObserveError(PageKey),
    /// The page's observed disk state changed (an Open read, an adopted or
    /// recorded external change): the publication consumer indexes it (§5).
    Observed {
        page: PageKey,
        bytes: Text,
    },
    /// An operation whose caller stopped waiting (Uncertain) ended with its
    /// draft durably absent: it did not happen (PLAN-P2b-AB v2 Q2).
    OperationDropped(BTreeSet<PageKey>),
    /// Why request `id` was answered without taking it (STEP3 §3.3).
    Refused {
        page: PageKey,
        id: u64,
        reason: Refusal,
    },
    /// Another page file claims this page's name (the alternate-extension
    /// twin, STEP3 §3.2): before the rename it failed the save; after it, a
    /// notice beside the save's own outcome (Q9).
    Twin {
        page: PageKey,
        existing: String,
        /// The save's version.
        version: u64,
        /// False: the twin failed the save; true: found after the rename.
        saved: bool,
    },
}

/// Typed reasons for an answer that did not take its request (STEP3 §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Refusal {
    /// A submit or discard for a page the host does not hold.
    NotHeld,
    /// A discard whose disk read failed (SPEC-s2 §4.11 "discard failed").
    ReadFailed,
    /// A move whose versions were no longer current.
    Stale,
    /// A move whose receiver draft could not be written.
    DraftFailed,
}

#[cfg_attr(test, derive(Clone))]
enum Application {
    Refresh(Record),
    Removal(PageKey),
    Operation {
        pages: BTreeMap<PageKey, Page>,
        records: Vec<Record>,
        reply: Reply,
        last_version: u64,
        reads: BTreeMap<PageKey, Base>,
        gates: Option<Gates>,
    },
    Representation,
}

#[cfg_attr(test, derive(Clone))]
struct DraftWorker {
    /// The initial vehicle names this obligation, even when copies are replaced.
    effect: String,
    refresh: Option<PageKey>,
    pages: BTreeSet<PageKey>,
    before: BTreeMap<PageKey, Record>,
    application: Option<Application>,
    task: Vehicle,
    remaining: VecDeque<Vehicle>,
    allocator: bool,
    /// Representation writes that fail must retry, never count as copies.
    retry_copy: bool,
    records: Vec<Record>,
    tidied: bool,
    /// Pending-effect failures outlive replacement representation vehicles.
    failures: u32,
    recover_notice: bool,
}

/// A key's logical state (`Host::logical`).
#[derive(Clone, Copy, Debug)]
enum Logical<'a> {
    /// No page held and no admitted input for it.
    Absent,
    /// The applied page; nothing admitted for it is still to apply.
    Settled(&'a Page),
    /// Admitted input not applied yet, at the version it carries; never
    /// clean or published. `conflict` is the in-flight page's.
    Admitted { version: u64, conflict: bool },
}

impl Logical<'_> {
    fn conflict(&self) -> bool {
        match self {
            Logical::Absent => false,
            Logical::Settled(page) => page.conflict,
            Logical::Admitted { conflict, .. } => *conflict,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Applied,
    Pending,
    Waiting,
    Refused,
    Disabled,
}

#[cfg_attr(test, derive(Clone))]
struct Host<F: HostIo> {
    fs: F,
    keys: Keys,
    /// Handles supplied from Graph::page_lock; this is not another registry.
    locks: BTreeMap<PageKey, Arc<Mutex<()>>>,
    lock_ownership: BTreeSet<PageKey>,
    /// Path locks the driver holds for this step (STEP3 §1: plan, lock,
    /// revalidate). None only under test, where each step takes its own.
    held: Option<BTreeSet<PageKey>>,
    /// The key set a step needs and the driver does not hold: recorded, never
    /// acquired here. The step returned before any side effect.
    lock_request: Option<BTreeSet<PageKey>>,
    /// Keys whose path lock another thread held when the driver last tried
    /// it, until its retry (A4, REVIEW-3a): a step needing one waits without
    /// a lock request, so the driver plans other work. Set only for a
    /// driver step; empty otherwise.
    contended: BTreeSet<PageKey>,
    /// D-10: decoded durable draft vehicles, kept in step with the adapter's
    /// durable census through `draft_changes`, never rescanned per call.
    drafts: BTreeMap<String, Vec<Record>>,
    /// REVIEW-3b-P1 R2: pages named by draft vehicles on disk whose census
    /// launch could not make durable (M2). Cleanup debt, never recovery
    /// evidence: no stop finishes and no key is evicted over it until a
    /// re-probe syncs the census (`drafts_retry`); a stop aborts naming
    /// them meanwhile, so the next host never recovers a copy the stop
    /// should have retired (a restore's in particular).
    vehicle_debt: BTreeSet<PageKey>,
    pages: BTreeMap<PageKey, Page>,
    custody: BTreeMap<PageKey, Debt>,
    retire: BTreeMap<String, Retire>,
    /// Markers with a sticky custody error (the escape, or a third failed
    /// retirement); each clears only when that marker is retired (V3).
    custody_errors: BTreeSet<String>,
    /// REVIEW-2b-r2 V1: the custody listing failed with this error. Never an
    /// empty listing: it clears only when a later listing succeeds.
    custody_unknown: Option<String>,
    queue: VecDeque<Request>,
    applying: Option<Request>,
    outbox: BTreeMap<PageKey, Mail>,
    subscriptions: BTreeSet<PageKey>,
    events: Vec<Event>,
    job: Option<SaveJob>,
    worker: Option<DraftWorker>,
    retained: BTreeSet<PageKey>,
    order: order::Order,
    version: u64,
    wseq: u64,
    incarnation: u64,
    generation: u64,
    last_admitted: u64,
    last_applied: u64,
    admission_open: bool,
    switch_confirmation: Option<u64>,
    alive: bool,
    /// Test pause (Q-P2b-4's deterministic schedules): no admitted request
    /// applies and the draft worker does not advance while set.
    #[cfg(test)]
    paused: bool,
}

impl<F: HostIo> Host<F> {
    fn new(mut fs: F, locks: BTreeMap<PageKey, Arc<Mutex<()>>>) -> Self {
        let drafts = drafts::scan(fs.draft_files(true)).files;
        fs.draft_changes();
        Self {
            keys: Keys(locks.keys().cloned().collect()),
            fs,
            locks,
            lock_ownership: BTreeSet::new(),
            held: None,
            lock_request: None,
            contended: BTreeSet::new(),
            drafts,
            vehicle_debt: BTreeSet::new(),
            pages: BTreeMap::new(),
            custody: BTreeMap::new(),
            retire: BTreeMap::new(),
            custody_errors: BTreeSet::new(),
            custody_unknown: None,
            queue: VecDeque::new(),
            applying: None,
            outbox: BTreeMap::new(),
            subscriptions: BTreeSet::new(),
            events: vec![],
            job: None,
            worker: None,
            retained: BTreeSet::new(),
            order: Default::default(),
            version: 0,
            wseq: 0,
            incarnation: 1,
            generation: 1,
            last_admitted: 0,
            last_applied: 0,
            admission_open: true,
            switch_confirmation: None,
            alive: true,
            #[cfg(test)]
            paused: false,
        }
    }

    /// Plan step: true when the caller does not hold `keys`. The set is
    /// recorded for the driver, which takes those path locks (writer → paths
    /// → state) and polls again; the caller returns before any side effect.
    fn lacks_locks(&mut self, keys: &BTreeSet<PageKey>) -> bool {
        match &self.held {
            Some(held) if !keys.is_subset(held) => {
                let contended = keys
                    .iter()
                    .any(|key| !held.contains(key) && self.contended.contains(key));
                if !contended {
                    self.lock_request = Some(keys.clone());
                }
                true
            }
            _ => false,
        }
    }

    /// Runs `apply` with page-buffer ownership of `keys`. The driver holds
    /// the path locks already (asserted); only tests let a step take its own.
    fn with_locks<R>(&mut self, keys: &BTreeSet<PageKey>, apply: impl FnOnce(&mut Self) -> R) -> R {
        assert!(self.lock_ownership.is_empty());
        let handles: Vec<Arc<Mutex<()>>> = match &self.held {
            Some(held) => {
                assert!(keys.is_subset(held), "path locks not held by the driver");
                vec![]
            }
            #[cfg(test)]
            None => keys.iter().map(|key| self.locks[key].clone()).collect(),
            #[cfg(not(test))]
            None => panic!("the page host runs only under its driver"),
        };
        let guards: Vec<_> = handles.iter().map(|lock| lock.lock().unwrap()).collect();
        self.lock_ownership = keys.clone();
        self.sync_drafts();
        let result = apply(self);
        self.lock_ownership.clear();
        drop(guards);
        result
    }

    /// Apply the adapter's durable census changes to the draft index.
    fn sync_drafts(&mut self) {
        for (name, bytes) in self.fs.draft_changes() {
            match bytes.and_then(|bytes| drafts::vehicle(&name, &bytes)) {
                Some(records) => self.drafts.insert(name, records),
                None => self.drafts.remove(&name),
            };
        }
    }

    fn set_page(&mut self, key: &str, page: Option<Page>) {
        assert!(
            self.lock_ownership.contains(key),
            "buffer application without page lock"
        );
        let previous = self.pages.get(key).cloned();
        if let Some(page) = page {
            self.pages.insert(key.into(), page);
        } else {
            self.pages.remove(key);
        }
        let current = self.pages.get(key).cloned();
        if let Some(obs) = current.as_ref().and_then(|p| p.obs.clone()) {
            if previous.as_ref().and_then(|p| p.obs.as_ref()) != Some(&obs) {
                self.events.push(Event::Observed {
                    page: key.into(),
                    bytes: obs,
                });
            }
        }
        // The model pushes only fields visible to the client, not risk/base.
        let visible = |p: &Option<Page>| {
            p.as_ref()
                .map(|p| (p.buf.clone(), p.version, p.obs.clone(), p.conflict))
        };
        if visible(&previous) != visible(&current) && self.subscriptions.contains(key) {
            let answer = self.outbox.get(key).and_then(|m| m.answer.clone());
            self.outbox.insert(
                key.into(),
                Mail {
                    page: current,
                    answer,
                    generation: self.generation,
                },
            );
        }
    }

    fn next_version(&self) -> u64 {
        self.version.checked_add(1).expect("host version exhausted")
    }

    fn record(&mut self, key: &str, page: &Page) -> Record {
        self.wseq = self.wseq.checked_add(1).expect("draft sequence exhausted");
        Record {
            page: key.into(),
            wseq: self.wseq,
            version: page.version,
            base: page.base.clone(),
            bytes: page.buf.clone(),
        }
    }

    /// Custody phases (a) and (b) for every marker the page owes; each
    /// completed marker is retired. True when nothing is owed.
    fn settle(&mut self, key: &str) -> bool {
        let markers = self.custody.get(key).map(|d| d.markers.clone());
        for (marker, payload) in markers.unwrap_or_default() {
            if self.fs.trash_sync(key, &payload).is_ok() {
                self.retire_marker(key, &marker, payload);
            }
        }
        !self.custody.contains_key(key)
    }

    /// A4 rule 2.5, once no custody is owed for `marker`: unlink it and sync
    /// the custody directory. A failure keeps it as retire-only debt that
    /// progress retries (REVIEW-2b-r2 V2); the page owes nothing more.
    fn retire_marker(&mut self, key: &str, marker: &str, payload: String) {
        if let Some(debt) = self.custody.get_mut(key) {
            debt.markers.remove(marker);
            if debt.markers.is_empty() {
                self.custody.remove(key);
            }
        }
        if self.fs.custody_retire(marker).is_ok() {
            self.custody_errors.remove(marker);
        } else {
            let (page, failures) = (key.into(), 1);
            self.retire.insert(
                marker.into(),
                Retire {
                    page,
                    payload,
                    failures,
                },
            );
        }
    }

    /// Progress's backoff retry (V1, V2): relist custody while it is unknown,
    /// adopting and settling its debts as launch does, then retry every
    /// retire-only marker; the third failed retirement is a sticky error.
    fn recover_custody(&mut self) {
        if self.custody_unknown.is_some() && self.job.is_none() {
            let before: BTreeSet<_> = self.custody.keys().cloned().collect();
            self.list_custody();
            let adopted: Vec<_> = self
                .custody
                .keys()
                .filter(|key| !before.contains(*key))
                .cloned()
                .collect();
            for key in adopted {
                self.settle(&key);
            }
        }
        for (marker, retire) in self.retire.clone() {
            if self.fs.custody_retire(&marker).is_ok() {
                self.retire.remove(&marker);
                self.custody_errors.remove(&marker);
            } else {
                let failures = retire.failures + 1;
                self.retire.get_mut(&marker).unwrap().failures = failures;
                if failures >= 3 {
                    self.custody_errors.insert(marker);
                }
            }
        }
    }

    /// A4 rule 3's listing. A marker owing only retirement stays there; a
    /// malformed one is quarantined, never acted on. A failed listing is
    /// recorded, never read as "no debt" (REVIEW-2b-r2 V1).
    fn list_custody(&mut self) {
        let markers = match self.fs.custody_markers() {
            Ok(markers) => markers,
            Err(error) => {
                self.custody_unknown = Some(error);
                return;
            }
        };
        self.custody_unknown = None;
        for (name, bytes) in markers {
            if self.retire.contains_key(&name) {
                continue;
            }
            if let Ok(marker) = drafts::decode_marker(&bytes) {
                let debt = self.custody.entry(marker.page).or_default();
                debt.markers.insert(name, marker.payload);
            } else {
                let name = format!("trash-custody/{name}");
                self.events.push(Event::Unreadable(name.clone()));
                let _ = self.fs.quarantine(&name);
            }
        }
    }

    /// Payloads with a sticky custody error on this page (V3).
    fn custody_errors(&self, key: &str) -> BTreeSet<String> {
        self.custody_errors
            .iter()
            .filter_map(|marker| {
                let owed = self.custody.get(key).and_then(|d| d.markers.get(marker));
                owed.or_else(|| {
                    self.retire
                        .get(marker)
                        .filter(|r| r.page == key)
                        .map(|r| &r.payload)
                })
                .cloned()
            })
            .collect()
    }

    fn allocator_busy(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| w.allocator)
    }

    /// The rename's effective referrers (STEP3 A-W1, R3/R4): the caller's,
    /// less a held one that is dirty or busy and whose buffer the rewrite
    /// leaves unchanged (the rename has nothing to write there, and its input
    /// is not the rename's to flush or refuse), plus every other held buffer
    /// the rewrite changes, including pages no index lists. Err when the
    /// rewrite fails on a held buffer. Model refinement: `conformance.rs`.
    pub(super) fn rename_refs(
        &self,
        source: &str,
        target: &str,
        referrers: &BTreeSet<PageKey>,
        rewrite: &impl Fn(&Text, &str, bool) -> Result<Text, ()>,
    ) -> Result<BTreeSet<PageKey>, ()> {
        let unchanged = |key: &PageKey, page: &Page| {
            (!page.clean() || self.busy(key))
                && rewrite(&page.buf, key, false).is_ok_and(|bytes| bytes == page.buf)
        };
        let mut refs: BTreeSet<PageKey> = referrers
            .iter()
            .filter(|key| {
                !self
                    .pages
                    .get(*key)
                    .is_some_and(|page| unchanged(key, page))
            })
            .cloned()
            .collect();
        for (key, page) in &self.pages {
            if key != source && key != target && rewrite(&page.buf, key, false)? != page.buf {
                refs.insert(key.clone());
            }
        }
        Ok(refs)
    }

    fn busy(&self, page: &str) -> bool {
        self.retained.contains(page)
            || self.job.as_ref().is_some_and(|job| job.page == page)
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.pages.contains(page))
    }

    fn admit(&mut self, request: Request) -> Disposition {
        if !self.alive
            || !self.admission_open
            || request.generation != self.generation
            || request.id <= self.last_admitted
            || !self.keys.contains(&request.page)
        {
            return Disposition::Refused;
        }
        if let RequestKind::Move { receiver, .. } = &request.kind {
            if receiver == &request.page || !self.keys.contains(receiver) {
                return Disposition::Refused;
            }
        }
        self.last_admitted = request.id;
        if matches!(request.kind, RequestKind::Open) {
            self.subscriptions.insert(request.page.clone());
        }
        if matches!(request.kind, RequestKind::Close) {
            self.subscriptions.remove(&request.page);
        }
        self.queue.push_back(request);
        Disposition::Applied
    }

    fn abstract_queue(&self) -> Vec<Request> {
        self.applying
            .iter()
            .chain(self.queue.iter())
            .cloned()
            .collect()
    }

    /// A key's logical state, the one source of every barrier query
    /// (`owed`, `pages_published`, `pages_recoverable`, `wait_published`,
    /// `unsaved`; STEP3-DESIGN "Manager decisions on the P2b checkpoint
    /// questions", Q-P2b-4). Admitted input not applied yet wins over the
    /// applied page: the draft worker's in-flight operation (a deletion,
    /// rename or move, at the version it installs), else a queued or
    /// applying submit or move (at its version, at least the held one).
    /// A barrier that read `pages` alone vouched for a deletion before its
    /// file moved. A census in `tests.rs` keeps barrier code off `pages`.
    fn logical(&self, key: &str) -> Logical<'_> {
        let in_flight = self
            .worker
            .as_ref()
            .and_then(|worker| match &worker.application {
                Some(Application::Operation { pages, .. }) => pages.get(key),
                _ => None,
            });
        if let Some(page) = in_flight {
            return Logical::Admitted {
                version: page.version,
                conflict: page.conflict,
            };
        }
        let held = self.pages.get(key);
        let queued = self
            .applying
            .iter()
            .chain(&self.queue)
            .filter_map(|request| match &request.kind {
                RequestKind::Submit { version, .. } if request.page == key => Some(*version),
                RequestKind::Move {
                    receiver,
                    source_version,
                    receiver_version,
                    ..
                } => (request.page == key)
                    .then_some(*source_version)
                    .or((receiver == key).then_some(*receiver_version)),
                _ => None,
            })
            .max();
        match (queued, held) {
            (Some(version), held) => Logical::Admitted {
                version: version.max(held.map_or(0, |page| page.version)),
                conflict: false,
            },
            (None, Some(page)) => Logical::Settled(page),
            (None, None) => Logical::Absent,
        }
    }

    /// Every key whose logical state is not `Absent`: the held pages and
    /// the keys admitted input names. Bounded by those; no registry walk.
    fn logical_keys(&self) -> BTreeSet<PageKey> {
        let mut keys: BTreeSet<PageKey> = self.pages.keys().cloned().collect();
        if let Some(Application::Operation { pages, .. }) =
            self.worker.as_ref().and_then(|w| w.application.as_ref())
        {
            keys.extend(pages.keys().cloned());
        }
        for request in self.applying.iter().chain(&self.queue) {
            match &request.kind {
                RequestKind::Submit { .. } => {
                    keys.insert(request.page.clone());
                }
                RequestKind::Move { receiver, .. } => {
                    keys.extend([request.page.clone(), receiver.clone()]);
                }
                _ => {}
            }
        }
        keys
    }

    fn answer(&mut self, request: &Request, key: &str, took: bool) {
        if request.generation != self.generation {
            return;
        }
        let version = self.pages.get(key).map_or(0, |p| p.version);
        let answer = Answer {
            id: request.id,
            generation: request.generation,
            version,
            took,
        };
        self.events.push(Event::Answer {
            page: key.into(),
            answer: answer.clone(),
        });
        self.outbox.insert(
            key.into(),
            Mail {
                page: self.pages.get(key).cloned(),
                answer: Some(answer),
                generation: self.generation,
            },
        );
    }

    /// An answer that did not take `request`, with its typed reason.
    fn refused(&mut self, request: &Request, key: &str, reason: Refusal) {
        if request.generation == self.generation {
            self.events.push(Event::Refused {
                page: key.into(),
                id: request.id,
                reason,
            });
        }
        self.answer(request, key, false);
    }

    fn finish_request(&mut self, request: &Request) {
        self.last_applied = request.id;
        self.applying = None;
    }

    fn window_crash(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("generation exhausted");
        self.outbox.clear();
        self.subscriptions.clear();
        self.switch_abort();
        // Admitted requests, including the applying worker, retain custody.
    }

    fn receive(&mut self, page: &str) -> Option<Mail> {
        let mail = self.outbox.remove(page)?;
        // Until the failed-open answer is consumed the window is still sent,
        // so intervening page applications must coalesce into that answer.
        if mail.page.is_none() && mail.answer.is_some() {
            self.subscriptions.remove(page);
        }
        Some(mail)
    }

    fn initial_page(bytes: Text, version: u64) -> Page {
        Page {
            buf: bytes.clone(),
            base: Base::Known(bytes.clone()),
            version,
            typed: false,
            risk: false,
            conflict: false,
            obs: Some(bytes),
        }
    }

    fn load(&mut self, key: &str) -> Disposition {
        if !self.alive || !self.keys.contains(key) || self.pages.contains_key(key) {
            return Disposition::Disabled;
        }
        let keys = BTreeSet::from([key.into()]);
        if self.busy(key) || self.allocator_busy() || self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.load_locked(key))
    }

    fn load_locked(&mut self, key: &str) -> Disposition {
        if self.pages.contains_key(key) {
            return Disposition::Disabled;
        }
        let Ok(bytes) = self.fs.read_page(key) else {
            return Disposition::Refused;
        };
        self.version = self.next_version();
        self.set_page(key, Some(Self::initial_page(bytes, self.version)));
        self.events.push(Event::Loaded {
            page: key.into(),
            version: self.version,
        });
        Disposition::Applied
    }

    /// Dequeue is a distinct barrier, retaining the entry in abstract_queue.
    fn dequeue(&mut self) -> Disposition {
        if !self.alive || self.applying.is_some() {
            return Disposition::Disabled;
        }
        let Some(request) = self.queue.front() else {
            return Disposition::Disabled;
        };
        if self.busy(&request.page)
            || self.allocator_busy()
            || matches!(&request.kind, RequestKind::Move { receiver, .. } if self.busy(receiver))
        {
            return Disposition::Waiting;
        }
        self.applying = self.queue.pop_front();
        Disposition::Pending
    }

    fn apply_request(&mut self) -> Disposition {
        let Some(request) = self.applying.clone() else {
            return Disposition::Disabled;
        };
        #[cfg(test)]
        if self.paused {
            return Disposition::Waiting;
        }
        if self.busy(&request.page) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        let mut keys = BTreeSet::from([request.page.clone()]);
        // A Discard of a running rename's destination reverts its source too.
        if let (RequestKind::Discard { .. }, Some(src)) =
            (&request.kind, self.order.partner(&request.page))
        {
            if self.busy(src) {
                return Disposition::Waiting;
            }
            keys.insert(src.clone());
        }
        if let RequestKind::Move { receiver, .. } = &request.kind {
            if self.busy(receiver) || self.worker.is_some() {
                return Disposition::Waiting;
            }
            keys.insert(receiver.clone());
        }
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.apply_locked_request(&request))
    }

    fn apply_locked_request(&mut self, request: &Request) -> Disposition {
        let key = &request.page;
        match &request.kind {
            RequestKind::Open => {
                if !self.pages.contains_key(key) {
                    if let Ok(bytes) = self.fs.read_page(key) {
                        self.version = self.next_version();
                        self.set_page(key, Some(Self::initial_page(bytes, self.version)));
                    }
                }
                self.answer(request, key, false);
            }
            RequestKind::Close => {
                self.close_clean(key);
            }
            RequestKind::Submit {
                bytes,
                version,
                resolve,
            } => {
                let Some(mut page) = self.pages.get(key).cloned() else {
                    self.refused(request, key, Refusal::NotHeld);
                    self.finish_request(request);
                    return Disposition::Refused;
                };
                let next = self.next_version();
                if let Some(base) = resolve {
                    page.buf = bytes.clone();
                    page.base = Base::Known(base.clone());
                    page.version = next;
                    page.typed = true;
                    page.conflict = page.obs != Some(base.clone());
                    page.risk |= page.conflict;
                } else {
                    page.submit(bytes.clone(), *version, next);
                }
                if page.buf.is_some() && self.pages[key].buf.is_none() {
                    self.order.supersede(key);
                }
                self.version = next;
                self.set_page(key, Some(page));
                self.answer(request, key, true);
            }
            RequestKind::Discard { .. } => self.discard(request),
            RequestKind::Move {
                receiver,
                source_text,
                receiver_text,
                source_version,
                receiver_version,
            } => {
                let Some(a) = self.pages.get(key).cloned() else {
                    return self.refuse_move(request, receiver);
                };
                let Some(b) = self.pages.get(receiver).cloned() else {
                    return self.refuse_move(request, receiver);
                };
                if a.version != *source_version || b.version != *receiver_version {
                    return self.refuse_move(request, receiver);
                }
                let vd = self.next_version();
                let vs = vd.checked_add(1).expect("host version exhausted");
                let mut source = a;
                let mut target = b;
                source.submit(source_text.clone(), *source_version, vs);
                target.submit(receiver_text.clone(), *receiver_version, vd);
                target.risk = true;
                let record = self.record(receiver, &target);
                self.install_operation(
                    BTreeMap::from([(key.clone(), source), (receiver.clone(), target)]),
                    vec![record],
                    Reply::Window(request.clone()),
                    vs,
                    BTreeMap::new(),
                    None,
                );
                return Disposition::Pending;
            }
        }
        self.finish_request(request);
        Disposition::Applied
    }

    fn refuse_move(&mut self, request: &Request, receiver: &str) -> Disposition {
        self.refused(request, receiver, Refusal::Stale);
        self.refused(request, &request.page, Refusal::Stale);
        self.finish_request(request);
        Disposition::Refused
    }

    fn close_clean(&mut self, key: &str) {
        if self.pages.get(key).is_some_and(Page::clean) {
            self.set_page(key, None);
        }
    }

    fn observe(&mut self, key: &str) -> Disposition {
        if !self.alive || !self.pages.contains_key(key) {
            return Disposition::Disabled;
        }
        let keys = BTreeSet::from([key.into()]);
        if self.busy(key) || self.allocator_busy() || self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            let Ok(bytes) = host.fs.read_page(key) else {
                return Disposition::Refused;
            };
            if host.adopt_read(key, bytes) {
                Disposition::Applied
            } else {
                Disposition::Disabled
            }
        })
    }

    /// Apply a disk read of `key` to its page (SPEC read table); false when
    /// the page is unchanged. A read that changes the page publishes its
    /// bytes even when they equal the last observation: an own save may
    /// have moved the index past them since (V3, REVIEW-3a).
    fn adopt_read(&mut self, key: &str, bytes: Text) -> bool {
        let mut page = self.pages[key].clone();
        page.observe(bytes.clone(), self.next_version());
        if page == self.pages[key] {
            return false;
        }
        if page.version == self.next_version() {
            self.version = page.version;
        }
        let again = self.pages[key].obs.as_ref() == Some(&bytes);
        self.set_page(key, Some(page));
        if again {
            self.events.push(Event::Observed {
                page: key.into(),
                bytes,
            });
        }
        true
    }

    /// STEP3 §2 registration: a key with no page, the model's state of a
    /// path with no file and no buffer, so no transition; it widens the key
    /// set the proof quantifies over. Keys are never unregistered within a
    /// binding, and a registered key keeps its spelling and lock handle.
    /// `spelling` names the key's directory entry (the store's case-alias
    /// resolution) and `lock` is `Graph::page_lock` of that spelling.
    fn register(&mut self, key: PageKey, spelling: &str, lock: Arc<Mutex<()>>) {
        if self.keys.insert(key.clone()) {
            self.fs.spell(&key, spelling);
            self.locks.insert(key, lock);
        }
    }

    /// The alias spelling move (STEP3 §2, Q4): on a folding volume the
    /// retained writer moved the key's entry to `spelling`, which the old
    /// spelling still names. The key, its page, drafts and every queued
    /// request stay; I/O and the path lock follow the new spelling. A move
    /// to an absent distinct path is a host rename between two keys instead.
    fn respell(&mut self, key: &str, spelling: &str, lock: Arc<Mutex<()>>) {
        assert!(
            self.retained.contains(key),
            "a spelling move runs under the key's reservation"
        );
        self.fs.spell(key, spelling);
        self.locks.insert(key.into(), lock);
    }

    /// The keys `launch` recovers from the readable draft census; the
    /// binding registers them first. Custody markers name exact key strings
    /// and their settling takes no page lock, so they need no registration.
    fn recovered_keys(&self) -> BTreeSet<PageKey> {
        drafts::scan(self.fs.draft_files(false))
            .logical
            .into_keys()
            .collect()
    }

    fn reserve(&mut self, keys: &BTreeSet<PageKey>) -> Disposition {
        if !self.keys.includes(keys) {
            return Disposition::Refused;
        }
        if keys.iter().any(|key| self.busy(key)) {
            return Disposition::Waiting;
        }
        self.retained.extend(keys.iter().cloned());
        Disposition::Applied
    }

    fn release(&mut self, keys: &BTreeSet<PageKey>) -> Disposition {
        if !keys.is_subset(&self.retained) {
            return Disposition::Refused;
        }
        if self.allocator_busy() || self.lacks_locks(keys) {
            return Disposition::Waiting;
        }
        // Reconcile even an undone transaction; failed reads retain custody.
        let result = self.with_locks(keys, |host| {
            let mut observed = BTreeMap::new();
            for key in keys {
                let Ok(bytes) = host.fs.read_page(key) else {
                    return Disposition::Waiting;
                };
                observed.insert(key.clone(), bytes);
            }
            for (key, bytes) in observed {
                if host.pages.contains_key(&key) {
                    host.adopt_read(&key, bytes);
                }
            }
            Disposition::Applied
        });
        if result == Disposition::Applied {
            for key in keys {
                self.retained.remove(key);
            }
        }
        result
    }

    fn switch_ready(&mut self, consumed_last_id: u64) -> Disposition {
        if !self.alive {
            return Disposition::Disabled;
        }
        if consumed_last_id != self.last_admitted
            || consumed_last_id != self.last_applied
            || self.outbox.values().any(|mail| mail.answer.is_some())
        {
            return Disposition::Waiting;
        }
        self.admission_open = false;
        self.switch_confirmation = Some(consumed_last_id);
        Disposition::Applied
    }

    fn switch_request(&mut self) -> Disposition {
        if self.worker.is_some() || !self.retained.is_empty() {
            return Disposition::Waiting;
        }
        let keys = self.pages.keys().cloned().collect();
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            for key in &keys {
                let mut page = host.pages[key].clone();
                if !page.clean() {
                    page.risk = true;
                }
                host.set_page(key, Some(page));
            }
        });
        Disposition::Applied
    }

    fn can_switch(&self) -> bool {
        if !self.alive
            || !self.vehicle_debt.is_empty()
            || self.admission_open
            || self.switch_confirmation != Some(self.last_applied)
            || !self.queue.is_empty()
            || self.applying.is_some()
            || self.job.is_some()
            || self.worker.is_some()
            || !self.retained.is_empty()
        {
            return false;
        }
        let drafts = self.logical_drafts();
        self.keys.iter().all(|key| {
            let page = self.pages.get(key);
            page.is_none_or(|p| p.clean() || p.risk)
                && match page.filter(|p| p.risk) {
                    Some(p) => drafts.get(key).is_some_and(|r| {
                        r.bytes == p.buf && r.base == p.base && r.version == p.version
                    }),
                    None => !drafts.contains_key(key),
                }
        })
    }

    fn switch_abort(&mut self) {
        self.admission_open = true;
        self.switch_confirmation = None;
    }

    fn switch_finish(&mut self) -> Disposition {
        if !self.can_switch() {
            return Disposition::Waiting;
        }
        self.stop();
        Disposition::Applied
    }

    fn stop(&mut self) {
        // Fault teardown ends an incarnation; it does not apply page mutations.
        self.alive = false;
        self.pages.clear();
        self.custody.clear();
        self.retire.clear();
        self.custody_errors.clear();
        self.custody_unknown = None;
        self.queue.clear();
        self.applying = None;
        self.outbox.clear();
        self.subscriptions.clear();
        self.job = None;
        self.worker = None;
        self.retained.clear();
        self.order.stop();
        self.switch_confirmation = None;
    }

    fn launch(&mut self) -> Disposition {
        if self.alive {
            return Disposition::Disabled;
        }
        let scan = drafts::scan(self.fs.draft_files(false));
        if !scan.logical.keys().all(|key| self.keys.contains(key)) {
            // Caller must provide shared locks for every recovered key.
            return Disposition::Refused;
        }
        let keys = scan.logical.keys().cloned().collect();
        // Plan before the first side effect (STEP3 §1): quarantine included.
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        for name in &scan.unreadable {
            self.events.push(Event::Unreadable(name.clone()));
            // One that fails stays in place and is never touched (§4): the
            // graph opens anyway, and `draft_status` names it.
            let _ = self.fs.quarantine(name);
        }
        self.version = scan.max_version;
        self.wseq = scan.max_wseq;
        self.incarnation = self
            .incarnation
            .checked_add(1)
            .expect("incarnation exhausted");
        self.window_crash();
        self.last_admitted = 0;
        self.last_applied = 0;
        // A4 rule 3, destination first: every marker's custody runs before the
        // graph's best-effort source-directory syncs. Neither debt nor a failed
        // listing blocks opening (V1).
        self.list_custody();
        for key in self.custody.keys().cloned().collect::<Vec<_>>() {
            self.settle(&key);
        }
        self.fs.graph_launch(&self.keys.iter().cloned().collect());
        self.with_locks(&keys, |host| {
            for (key, record) in &scan.logical {
                host.version = host.next_version();
                host.set_page(
                    key,
                    Some(Page {
                        buf: record.bytes.clone(),
                        base: record.base.clone(),
                        version: host.version,
                        typed: true,
                        risk: true,
                        conflict: false,
                        obs: None,
                    }),
                );
            }
        });
        // Make every readable recovered entry durable before retirement.
        // This is essential for process-crash survivors not previously
        // directory synced. When that fails, the recovered buffers stay held
        // at risk and draft I/O goes down, so nothing retires (M2).
        let synced = matches!(self.fs.draft_sync(), Ok(Witness::Durable));
        if !synced {
            self.fs.drafts_unsynced();
        }
        self.drafts = drafts::scan(self.fs.draft_files(true)).files;
        self.vehicle_debt = if synced {
            BTreeSet::new()
        } else {
            let known = drafts::scan(self.fs.draft_files(false)).files;
            known.into_values().flatten().map(|r| r.page).collect()
        };
        self.fs.draft_changes();
        self.alive = true;
        if synced {
            self.tidy_drafts()
        } else {
            Disposition::Applied
        }
    }

    /// Retry (S3): re-probe down draft I/O in place, keeping the census,
    /// the vehicles and every page, then run the cleanup launch skipped:
    /// quarantine what it left in place, and tidy. The error says why draft
    /// I/O is still down.
    pub(super) fn drafts_retry(&mut self) -> Result<Disposition, String> {
        let status = self.fs.draft_status();
        if status == DraftStatus::default() {
            return Ok(Disposition::Applied);
        }
        if self.worker.is_some() {
            return Ok(Disposition::Waiting);
        }
        self.fs.drafts_reprobe()?;
        // The census is durable now: the vehicles are ordinary drafts.
        self.vehicle_debt.clear();
        for name in &status.unreadable {
            let _ = self.fs.quarantine(name);
        }
        self.sync_drafts();
        Ok(self.tidy_drafts())
    }

    /// Launch's representation cleanup over the durable index (§4):
    /// explode operation vehicles into page copies and retire superseded
    /// vehicles. Only once the census is durable (M2).
    fn tidy_drafts(&mut self) -> Disposition {
        let logical = drafts::logical(self.drafts.values());
        let mut tasks = VecDeque::new();
        for (name, records) in &self.drafts {
            if name.starts_with("op-") {
                let mut superseded = vec![];
                for record in records {
                    let copy = drafts::page_name(&record.page);
                    tasks.push_back(Vehicle::write(copy.clone(), std::slice::from_ref(record)));
                    if record.wseq < logical[&record.page].wseq {
                        superseded.push(Vehicle::remove(copy));
                    }
                }
                tasks.push_back(Vehicle::remove(name.clone()));
                tasks.extend(superseded);
            }
        }
        for (key, record) in &logical {
            tasks.extend(
                drafts::older_vehicles(&self.drafts, key, Some(record.wseq))
                    .into_iter()
                    .map(Vehicle::remove),
            );
        }
        let Some(task) = tasks.pop_front() else {
            return Disposition::Applied;
        };
        let records = task
            .bytes
            .as_ref()
            .and_then(|b| drafts::decode(b).ok())
            .unwrap_or_default();
        self.worker = Some(DraftWorker {
            effect: task.name.clone(),
            refresh: None,
            pages: logical.keys().cloned().collect(),
            before: logical,
            application: Some(Application::Representation),
            retry_copy: task.bytes.is_some(),
            records,
            task,
            remaining: tasks,
            allocator: true,
            tidied: false,
            failures: 0,
            recover_notice: true,
        });
        Disposition::Pending
    }
}
