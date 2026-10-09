//! Unwired page owner. Runtime code is independent of the page_state oracle.
#![allow(dead_code)]

#[cfg(test)]
mod conformance;
mod drafts;
mod driver;
#[cfg(test)]
mod driver_tests;
mod io;
#[cfg(test)]
mod model_fs;
#[cfg(test)]
mod native_cost;
mod operations;
mod production;
#[cfg(test)]
mod production_tests;
mod progress;
#[cfg(test)]
mod tests;

use drafts::{Record, Stage, Vehicle};
use io::{ErrorKind, HostIo, Witness};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

type Text = Option<Arc<[u8]>>;
type PageKey = String;

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct SaveJob {
    page: PageKey,
    phase: SavePhase,
    bytes: Text,
    base: Base,
    version: u64,
    epoch: u64,
    removed: Text,
    /// A deletion's marker name and payload basename.
    marker: Option<(String, String)>,
    trash_durable: bool,
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
}

#[cfg_attr(test, derive(Clone))]
enum Application {
    Refresh(Record),
    Removal(PageKey),
    Operation {
        pages: BTreeMap<PageKey, Page>,
        records: Vec<Record>,
        request: Option<Request>,
        last_version: u64,
        reads: BTreeMap<PageKey, Base>,
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
    keys: BTreeSet<PageKey>,
    /// Handles supplied from Graph::page_lock; this is not another registry.
    locks: BTreeMap<PageKey, Arc<Mutex<()>>>,
    lock_ownership: BTreeSet<PageKey>,
    /// Path locks the driver holds for this step (STEP3 §1: plan, lock,
    /// revalidate). None only under test, where each step takes its own.
    held: Option<BTreeSet<PageKey>>,
    /// The key set a step needs and the driver does not hold: recorded, never
    /// acquired here. The step returned before any side effect.
    lock_request: Option<BTreeSet<PageKey>>,
    /// D-10: decoded durable draft vehicles, kept in step with the adapter's
    /// durable census through `draft_changes`, never rescanned per call.
    drafts: BTreeMap<String, Vec<Record>>,
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
    version: u64,
    wseq: u64,
    incarnation: u64,
    generation: u64,
    last_admitted: u64,
    last_applied: u64,
    admission_open: bool,
    switch_confirmation: Option<u64>,
    alive: bool,
}

impl<F: HostIo> Host<F> {
    fn new(mut fs: F, locks: BTreeMap<PageKey, Arc<Mutex<()>>>) -> Self {
        let drafts = drafts::scan(fs.draft_files(true)).files;
        fs.draft_changes();
        Self {
            keys: locks.keys().cloned().collect(),
            fs,
            locks,
            lock_ownership: BTreeSet::new(),
            held: None,
            lock_request: None,
            drafts,
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
            version: 0,
            wseq: 0,
            incarnation: 1,
            generation: 1,
            last_admitted: 0,
            last_applied: 0,
            admission_open: true,
            switch_confirmation: None,
            alive: true,
        }
    }

    /// Plan step: true when the caller does not hold `keys`. The set is
    /// recorded for the driver, which takes those path locks (writer → paths
    /// → state) and polls again; the caller returns before any side effect.
    fn lacks_locks(&mut self, keys: &BTreeSet<PageKey>) -> bool {
        match &self.held {
            Some(held) if !keys.is_subset(held) => {
                self.lock_request = Some(keys.clone());
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
        if self.busy(&request.page) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        let mut keys = BTreeSet::from([request.page.clone()]);
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
                    self.answer(request, key, false);
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
                self.version = next;
                self.set_page(key, Some(page));
                self.answer(request, key, true);
            }
            RequestKind::Discard { .. } => {
                if let (Some(mut page), Ok(bytes)) =
                    (self.pages.get(key).cloned(), self.fs.read_page(key))
                {
                    let drafts = self.logical_drafts();
                    let hold = page.risk
                        && (page.buf == bytes
                            || drafts.get(key).is_some_and(|draft| draft.bytes == bytes));
                    self.version = self.next_version();
                    page = Self::initial_page(bytes, self.version);
                    page.risk = hold;
                    self.set_page(key, Some(page));
                }
                self.answer(request, key, false);
            }
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
                    Some(request.clone()),
                    vs,
                    BTreeMap::new(),
                );
                return Disposition::Pending;
            }
        }
        self.finish_request(request);
        Disposition::Applied
    }

    fn refuse_move(&mut self, request: &Request, receiver: &str) -> Disposition {
        self.answer(request, receiver, false);
        self.answer(request, &request.page, false);
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
            let mut page = host.pages[key].clone();
            page.observe(bytes, host.next_version());
            if page == host.pages[key] {
                return Disposition::Disabled;
            }
            if page.version == host.next_version() {
                host.version = page.version;
            }
            host.set_page(key, Some(page));
            Disposition::Applied
        })
    }

    /// Logical entries are computed from durable files and the actual worker.
    /// Pending removal retains its previous entry; pending install is excluded.
    fn logical_drafts(&self) -> BTreeMap<PageKey, Record> {
        let installing = self
            .worker
            .as_ref()
            .filter(|w| w.application.is_some() && w.task.bytes.is_some())
            .map(|w| w.task.name.as_str());
        let mut logical = if self.alive {
            let index = drafts::logical(
                self.drafts
                    .iter()
                    .filter(|(name, _)| Some(name.as_str()) != installing)
                    .map(|(_, records)| records),
            );
            // Conformance: the index equals the scan of the durable census.
            #[cfg(test)]
            {
                let mut files = self.fs.draft_files(true);
                files.retain(|(name, _)| Some(name.as_str()) != installing);
                assert_eq!(index, drafts::scan(files).logical, "draft index");
            }
            index
        } else {
            // While stopped, recovery's readable directory is the projection.
            // A process crash may retain a renamed vehicle without a sync
            // witness; launch makes those names durable before retirement.
            let mut files = self.fs.draft_files(false);
            files.retain(|(name, _)| Some(name.as_str()) != installing);
            drafts::scan(files).logical
        };
        if let Some(worker) = &self.worker {
            if worker.application.is_some() {
                for key in &worker.pages {
                    logical.remove(key);
                    if let Some(record) = worker.before.get(key) {
                        logical.insert(key.clone(), record.clone());
                    }
                }
            }
        }
        logical
    }

    fn begin_draft(&mut self, key: &str) -> Disposition {
        if !self.alive || self.worker.is_some() || self.retained.contains(key) {
            return Disposition::Waiting;
        }
        let keys = BTreeSet::from([key.into()]);
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            let durable = drafts::logical(host.drafts.values());
            let previous = durable.get(key);
            let page = host.pages.get(key).cloned();
            let desired = page.as_ref().filter(|p| p.risk);
            if let Some(page) = desired {
                if previous.is_some_and(|r| {
                    r.bytes == page.buf && r.base == page.base && r.version == page.version
                }) {
                    return Disposition::Disabled;
                }
                let record = host.record(key, page);
                let task = Vehicle::write(drafts::page_name(key), std::slice::from_ref(&record));
                host.worker = Some(DraftWorker {
                    effect: task.name.clone(),
                    refresh: Some(key.into()),
                    pages: keys.clone(),
                    before: durable.clone(),
                    task,
                    application: Some(Application::Refresh(record.clone())),
                    remaining: VecDeque::new(),
                    allocator: false,
                    retry_copy: false,
                    records: vec![record],
                    tidied: false,
                    failures: 0,
                    recover_notice: true,
                });
            } else {
                if previous.is_none() {
                    return Disposition::Disabled;
                }
                let mut tasks: VecDeque<_> = drafts::older_vehicles(&host.drafts, key, None)
                    .into_iter()
                    .map(Vehicle::remove)
                    .collect();
                let Some(task) = tasks.pop_front() else {
                    return Disposition::Waiting;
                };
                host.worker = Some(DraftWorker {
                    effect: task.name.clone(),
                    refresh: None,
                    pages: keys.clone(),
                    before: durable.clone(),
                    task,
                    application: Some(Application::Removal(key.into())),
                    remaining: tasks,
                    allocator: false,
                    retry_copy: false,
                    records: vec![],
                    tidied: false,
                    failures: 0,
                    recover_notice: true,
                });
            }
            Disposition::Pending
        })
    }

    fn install_operation(
        &mut self,
        pages: BTreeMap<PageKey, Page>,
        records: Vec<Record>,
        request: Option<Request>,
        last_version: u64,
        reads: BTreeMap<PageKey, Base>,
    ) {
        let keys = pages.keys().cloned().collect();
        let name = if request.is_some() {
            drafts::page_name(&records[0].page)
        } else {
            drafts::op_name()
        };
        self.worker = Some(DraftWorker {
            effect: name.clone(),
            refresh: None,
            pages: keys,
            before: self.logical_drafts(),
            task: Vehicle::write(name, &records),
            remaining: VecDeque::new(),
            application: Some(Application::Operation {
                pages,
                records: records.clone(),
                request,
                last_version,
                reads,
            }),
            allocator: true,
            retry_copy: false,
            records,
            tidied: false,
            failures: 0,
            recover_notice: true,
        });
    }

    /// Advance exactly one draft I/O phase, or its terminal application.
    fn advance_draft(&mut self) -> Disposition {
        let Some(keys) = self.worker.as_ref().map(|w| w.pages.clone()) else {
            return Disposition::Disabled;
        };
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        let mut worker = self.worker.take().unwrap();
        let terminal = matches!(worker.task.stage, Stage::Present | Stage::Absent);
        if !terminal {
            let failures = worker.task.failures;
            self.with_locks(&keys, |host| {
                worker.task.advance(&mut host.fs);
                host.sync_drafts();
            });
            worker.failures = worker
                .failures
                .checked_add(worker.task.failures - failures)
                .expect("draft failure count exhausted");
            // A failed fresh refresh that is already durably absent can be
            // reported immediately. Retryable sync failures stay silent until
            // the third failure; their physical obligation is still pending.
            if worker.task.failures != failures
                && (worker.failures >= 3
                    || (worker.task.stage == Stage::Absent && worker.refresh.is_some()))
            {
                self.events.push(Event::DraftError {
                    effect: worker.effect.clone(),
                    pages: worker.pages.clone(),
                    refresh: worker.refresh.clone(),
                    failures: worker.failures,
                });
            }
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        // Removal becomes abstractly effective only after its last unlink.
        if worker.task.bytes.is_none() && !worker.remaining.is_empty() {
            worker.task = worker.remaining.pop_front().unwrap();
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        if worker.retry_copy && worker.task.stage == Stage::Absent && worker.task.bytes.is_some() {
            worker.task =
                Vehicle::write(drafts::page_name(&worker.records[0].page), &worker.records);
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        if let Some(application) = worker.application.take() {
            let present = worker.task.stage == Stage::Present;
            self.with_locks(&keys, |host| {
                host.apply_draft_terminal(&mut worker, application, present);
            });
        }
        if worker.remaining.is_empty() && !worker.tidied {
            worker.tidied = true;
            // Launch may find identical highest-sequence copies left by a
            // crash during explosion. Retain one, so repeated crashes cannot
            // accumulate recovery vehicles without bound.
            for key in &worker.pages {
                let mut vehicles: Vec<_> = self
                    .drafts
                    .iter()
                    .filter(|(name, records)| name.starts_with("p-") && records[0].page == *key)
                    .map(|(name, records)| (records[0].wseq, name.clone()))
                    .collect();
                vehicles.sort();
                vehicles.pop();
                worker
                    .remaining
                    .extend(vehicles.into_iter().map(|(_, name)| Vehicle::remove(name)));
            }
        }
        if let Some(task) = worker.remaining.pop_front() {
            worker.records = task
                .bytes
                .as_ref()
                .and_then(|b| drafts::decode(b).ok())
                .unwrap_or_default();
            worker.retry_copy = task.bytes.is_some();
            worker.task = task;
            self.worker = Some(worker);
            Disposition::Pending
        } else {
            self.events.push(Event::DraftFinished {
                effect: worker.effect,
                pages: worker.pages,
                refresh: worker.refresh,
                recovered: worker.recover_notice,
            });
            Disposition::Applied
        }
    }

    fn apply_draft_terminal(
        &mut self,
        worker: &mut DraftWorker,
        application: Application,
        present: bool,
    ) {
        match application {
            Application::Refresh(record) if present => {
                self.events.push(Event::Draft(record.clone()));
                worker.remaining.extend(
                    drafts::older_vehicles(&self.drafts, &record.page, Some(record.wseq))
                        .into_iter()
                        .map(Vehicle::remove),
                );
            }
            Application::Removal(key) => self.events.push(Event::DraftRemoved(key)),
            Application::Operation {
                pages,
                records,
                request,
                last_version,
                reads,
            } => {
                if present {
                    self.version = last_version;
                    for (key, page) in pages {
                        self.set_page(&key, Some(page));
                    }
                    for (key, base) in reads {
                        self.events.push(Event::OperationRead { page: key, base });
                    }
                    self.events
                        .extend(records.iter().cloned().map(Event::Draft));
                    if worker.task.name.starts_with("op-") {
                        for record in &records {
                            worker.remaining.push_back(Vehicle::write(
                                drafts::page_name(&record.page),
                                std::slice::from_ref(record),
                            ));
                        }
                        worker
                            .remaining
                            .push_back(Vehicle::remove(worker.task.name.clone()));
                    }
                    for record in &records {
                        worker.remaining.extend(
                            drafts::older_vehicles(&self.drafts, &record.page, Some(record.wseq))
                                .into_iter()
                                .map(Vehicle::remove),
                        );
                    }
                }
                if let Some(request) = request {
                    if let RequestKind::Move { receiver, .. } = &request.kind {
                        self.answer(&request, receiver, present);
                    }
                    self.answer(&request, &request.page, present);
                    self.finish_request(&request);
                }
                // Keep the allocator through explosion (§4), never acquire
                // another page or wait for a version allocator while holding it.
            }
            Application::Refresh(_) => worker.recover_notice = false,
            Application::Representation => {}
        }
    }

    fn start_save(&mut self, key: &str) -> Disposition {
        if !self.alive || self.job.is_some() {
            return Disposition::Disabled;
        }
        if self.busy(key) {
            return Disposition::Waiting;
        }
        let Some(page) = self.pages.get(key) else {
            return Disposition::Disabled;
        };
        if page.clean() || page.conflict {
            return Disposition::Disabled;
        }
        self.job = Some(SaveJob {
            page: key.into(),
            phase: if self.custody.contains_key(key) {
                SavePhase::Custody
            } else {
                Self::first_phase(&page.buf)
            },
            bytes: page.buf.clone(),
            base: page.base.clone(),
            version: page.version,
            epoch: 0,
            removed: None,
            marker: None,
            trash_durable: true,
        });
        Disposition::Pending
    }

    fn first_phase(bytes: &Text) -> SavePhase {
        if bytes.is_none() {
            SavePhase::Check
        } else {
            SavePhase::Temp
        }
    }

    /// `epoch` is the path's external-write epoch supplied by the event driver;
    /// the 2b adapter will obtain it from the watch boundary, not a file hash.
    fn advance_save(&mut self, epoch: u64) -> Disposition {
        let Some(job) = &self.job else {
            return Disposition::Disabled;
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.pages.contains(&job.page))
        {
            return Disposition::Waiting;
        }
        let keys = BTreeSet::from([job.page.clone()]);
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.advance_save_locked(epoch))
    }

    fn advance_save_locked(&mut self, epoch: u64) -> Disposition {
        let Some(mut job) = self.job.take() else {
            return Disposition::Disabled;
        };
        let key = job.page.clone();
        let result = match job.phase {
            // A failure is saveFail (L442-445): nothing was renamed. After three
            // consecutive filesystem errors the save goes ahead, the error stays
            // visible and the marker is retried at the next launch (R-STORAGE-ERROR).
            // While the custody listing is unknown (REVIEW-2b-r2 V1) a save
            // with no known debt never enters this phase and proceeds: the
            // listing error is the filesystem's report, the sticky
            // custody-unknown notice keeps it visible, and A4 rule 4's barrier
            // covers known markers only. That is R-STORAGE-ERROR.
            SavePhase::Custody => {
                if self.settle(&key) {
                    job.phase = Self::first_phase(&job.bytes);
                    None
                } else {
                    let debt = self.custody.get_mut(&key).unwrap();
                    debt.failures += 1;
                    if debt.failures >= 3 {
                        for (marker, payload) in &debt.markers {
                            self.custody_errors.insert(marker.clone());
                            self.events.push(Event::CustodyError {
                                page: key.clone(),
                                payload: payload.clone(),
                            });
                        }
                        job.phase = Self::first_phase(&job.bytes);
                        None
                    } else {
                        Some(Outcome::Failed)
                    }
                }
            }
            SavePhase::Temp => match self.fs.page_temp(&key, &job.bytes) {
                Ok(()) => {
                    job.phase = SavePhase::Check;
                    None
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Check => match self.fs.read_page(&key) {
                Ok(bytes) if job.base == Base::Known(bytes.clone()) => {
                    job.phase = if job.bytes.is_none() {
                        SavePhase::Marker
                    } else {
                        SavePhase::Rename
                    };
                    None
                }
                Ok(bytes) => {
                    if self.allocator_busy() {
                        self.job = Some(job);
                        return Disposition::Waiting;
                    }
                    let mut page = self.pages[&key].clone();
                    page.observe(bytes, self.next_version());
                    if page.version == self.next_version() {
                        self.version = page.version;
                    }
                    self.set_page(&key, Some(page));
                    self.fs.page_finish(&key);
                    return Disposition::Applied;
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Rename if job.bytes.is_some() => match self.fs.page_rename(&key) {
                Ok(()) => {
                    self.events.push(Event::Renamed {
                        page: key.clone(),
                        bytes: job.bytes.clone(),
                        version: job.version,
                    });
                    job.phase = SavePhase::DirectorySync;
                    job.epoch = epoch;
                    None
                }
                Err(e) if e.completed => {
                    self.events.push(Event::Renamed {
                        page: key.clone(),
                        bytes: job.bytes.clone(),
                        version: job.version,
                    });
                    Some(Outcome::Uncertain)
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Marker => {
                // One fresh identity names both the marker and its payload.
                let id = uuid::Uuid::new_v4().simple().to_string();
                let filename = std::path::Path::new(&key).file_name().unwrap();
                let marker = drafts::Marker {
                    page: key.clone(),
                    payload: crate::atomic_file::prefixed_name(
                        &format!("{id}__"),
                        &filename.to_string_lossy(),
                    ),
                };
                let name = format!("{id}.tcm");
                match self
                    .fs
                    .custody_write(&name, &drafts::encode_marker(&marker))
                {
                    Ok(()) => {
                        let debt = self.custody.entry(key.clone()).or_default();
                        debt.markers.insert(name.clone(), marker.payload.clone());
                        job.marker = Some((name, marker.payload));
                        job.phase = SavePhase::Rename;
                        None
                    }
                    Err(_) => Some(Outcome::Failed),
                }
            }
            SavePhase::Rename => {
                let (marker, payload) = job.marker.clone().expect("deletion marker");
                let movement = self.fs.trash_move(&key, &payload);
                job.removed = movement.removed;
                if job.removed.is_some() {
                    self.events.push(Event::Removed {
                        page: key.clone(),
                        bytes: job.removed.clone(),
                    });
                }
                match movement.result {
                    Ok(()) => {
                        self.events.push(Event::Renamed {
                            page: key.clone(),
                            bytes: job.bytes.clone(),
                            version: job.version,
                        });
                        // A restored deletion moved nothing; its marker owes nothing.
                        job.phase = if job.removed.is_some() {
                            SavePhase::TrashSync
                        } else {
                            SavePhase::DirectorySync
                        };
                        job.epoch = epoch;
                        None
                    }
                    // The occupied target is never adopted: retire the unused
                    // marker (it owes no custody; a failed unlink is
                    // retire-only debt), then retry under a fresh one.
                    Err(e) if e.kind == ErrorKind::Collision => {
                        self.retire_marker(&key, &marker, payload);
                        job.marker = None;
                        job.phase = SavePhase::Marker;
                        None
                    }
                    Err(e) if e.completed => {
                        self.events.push(Event::Renamed {
                            page: key.clone(),
                            bytes: job.bytes.clone(),
                            version: job.version,
                        });
                        Some(Outcome::Uncertain)
                    }
                    Err(_) => Some(Outcome::Failed),
                }
            }
            SavePhase::TrashSync => {
                let (_, payload) = job.marker.as_ref().expect("deletion marker");
                match self.fs.trash_sync(&key, payload) {
                    Ok(witness) => {
                        job.trash_durable = witness == Witness::Durable;
                        job.phase = SavePhase::DirectorySync;
                        None
                    }
                    Err(_) => Some(Outcome::Uncertain),
                }
            }
            SavePhase::DirectorySync => match self.fs.page_sync(&key) {
                Ok(witness) => {
                    if witness == Witness::Durable && job.trash_durable && job.bytes.is_none() {
                        self.events.push(Event::DeleteDurable {
                            page: key.clone(),
                            bytes: job.removed.clone(),
                        });
                    }
                    Some(Outcome::Published)
                }
                Err(_) => Some(Outcome::Uncertain),
            },
        };
        if result.is_some() && job.phase == SavePhase::DirectorySync {
            // Rule 2.5: custody (a)+(b) completed before this phase.
            if let Some((marker, payload)) = job.marker.clone() {
                self.retire_marker(&key, &marker, payload);
            }
        }
        if let Some(outcome) = result {
            self.fs.page_finish(&key);
            let mut page = self.pages[&key].clone();
            if outcome == Outcome::Published {
                page.base = Base::Known(job.bytes.clone());
                page.typed = false;
                page.risk = false;
                self.events.push(Event::Published {
                    page: key.clone(),
                    bytes: job.bytes,
                    version: job.version,
                    epoch: job.epoch,
                });
            } else {
                page.risk = true;
            }
            self.set_page(&key, Some(page));
            self.events.push(Event::SaveOutcome { page: key, outcome });
            Disposition::Applied
        } else {
            self.job = Some(job);
            Disposition::Pending
        }
    }

    fn reserve(&mut self, keys: &BTreeSet<PageKey>) -> Disposition {
        if !keys.is_subset(&self.keys) {
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
                if let Some(mut page) = host.pages.get(&key).cloned() {
                    page.observe(bytes, host.next_version());
                    if page.version == host.next_version() {
                        host.version = page.version;
                    }
                    host.set_page(&key, Some(page));
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
        self.switch_confirmation = None;
    }

    fn launch(&mut self) -> Disposition {
        if self.alive {
            return Disposition::Disabled;
        }
        let scan = drafts::scan(self.fs.draft_files(false));
        for name in &scan.unreadable {
            self.events.push(Event::Unreadable(name.clone()));
            if self.fs.quarantine(name).is_err() {
                return Disposition::Pending;
            }
        }
        if !scan.logical.keys().all(|key| self.keys.contains(key)) {
            // Caller must provide shared locks for every recovered key.
            return Disposition::Refused;
        }
        let keys = scan.logical.keys().cloned().collect();
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
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
        self.fs.graph_launch(&self.keys);
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
        // Make every readable recovered entry durable before retirement. This
        // is essential for process-crash survivors not previously directory synced.
        if !matches!(self.fs.draft_sync(), Ok(Witness::Durable)) {
            return Disposition::Pending;
        }
        self.drafts = scan.files.clone();
        self.fs.draft_changes();
        let mut tasks = VecDeque::new();
        for (name, records) in &scan.files {
            if name.starts_with("op-") {
                let mut superseded = vec![];
                for record in records {
                    let copy = drafts::page_name(&record.page);
                    tasks.push_back(Vehicle::write(copy.clone(), std::slice::from_ref(record)));
                    if record.wseq < scan.logical[&record.page].wseq {
                        superseded.push(Vehicle::remove(copy));
                    }
                }
                tasks.push_back(Vehicle::remove(name.clone()));
                tasks.extend(superseded);
            }
        }
        for (key, record) in &scan.logical {
            tasks.extend(
                drafts::older_vehicles(&scan.files, key, Some(record.wseq))
                    .into_iter()
                    .map(Vehicle::remove),
            );
        }
        self.alive = true;
        if let Some(task) = tasks.pop_front() {
            let records = task
                .bytes
                .as_ref()
                .and_then(|b| drafts::decode(b).ok())
                .unwrap_or_default();
            self.worker = Some(DraftWorker {
                effect: task.name.clone(),
                refresh: None,
                pages: keys,
                before: scan.logical,
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
        } else {
            Disposition::Applied
        }
    }
}
