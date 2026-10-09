//! Unwired page owner. Runtime code is independent of the page_state oracle.
#![allow(dead_code)]

mod drafts;
mod io;
#[cfg(test)]
mod model_fs;
mod operations;
#[cfg(test)]
mod tests;

use drafts::{Record, Stage, Vehicle};
use io::{ErrorKind, HostIo, Witness};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

type Text = Option<Arc<[u8]>>;
type PageKey = String;

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
    Discard,
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
    Temp,
    Check,
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
    trash_name: String,
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
        pages: BTreeSet<PageKey>,
        failures: u32,
    },
    Unreadable(String),
}

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

struct DraftWorker {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Applied,
    Pending,
    Waiting,
    Refused,
    Disabled,
}

struct Host<F: HostIo> {
    fs: F,
    keys: BTreeSet<PageKey>,
    /// Handles supplied from Graph::page_lock; this is not another registry.
    locks: BTreeMap<PageKey, Arc<Mutex<()>>>,
    lock_ownership: BTreeSet<PageKey>,
    pages: BTreeMap<PageKey, Page>,
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
    fn new(fs: F, locks: BTreeMap<PageKey, Arc<Mutex<()>>>) -> Self {
        Self {
            keys: locks.keys().cloned().collect(),
            fs,
            locks,
            lock_ownership: BTreeSet::new(),
            pages: BTreeMap::new(),
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

    fn with_locks<R>(&mut self, keys: &BTreeSet<PageKey>, apply: impl FnOnce(&mut Self) -> R) -> R {
        assert!(self.lock_ownership.is_empty());
        let handles: Vec<_> = keys.iter().map(|key| self.locks[key].clone()).collect();
        let guards: Vec<_> = handles.iter().map(|lock| lock.lock().unwrap()).collect();
        self.lock_ownership = keys.clone();
        let result = apply(self);
        self.lock_ownership.clear();
        drop(guards);
        result
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
        self.outbox.remove(page)
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
                if self.pages.get(key).is_some_and(Page::clean) {
                    self.set_page(key, None);
                }
                self.subscriptions.remove(key);
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
            RequestKind::Discard => {
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

    fn observe(&mut self, key: &str) -> Disposition {
        if !self.alive || !self.pages.contains_key(key) {
            return Disposition::Disabled;
        }
        if self.busy(key) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        self.with_locks(&BTreeSet::from([key.into()]), |host| {
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
        let mut files = self.fs.draft_files(true);
        if let Some(worker) = &self.worker {
            if worker.application.is_some() && worker.task.bytes.is_some() {
                files.retain(|(name, _)| name != &worker.task.name);
            }
        }
        let mut logical = drafts::scan(files).logical;
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
        if !self.alive || self.worker.is_some() || self.busy(key) {
            return Disposition::Waiting;
        }
        let keys = BTreeSet::from([key.into()]);
        self.with_locks(&keys, |host| {
            let scan = drafts::scan(host.fs.draft_files(true));
            let previous = scan.logical.get(key);
            let page = host.pages.get(key).cloned();
            let desired = page.as_ref().filter(|p| p.risk);
            if let Some(page) = desired {
                if previous.is_some_and(|r| {
                    r.bytes == page.buf && r.base == page.base && r.version == page.version
                }) {
                    return Disposition::Disabled;
                }
                let record = host.record(key, page);
                host.worker = Some(DraftWorker {
                    pages: keys.clone(),
                    before: scan.logical,
                    task: Vehicle::write(drafts::page_name(key), std::slice::from_ref(&record)),
                    application: Some(Application::Refresh(record.clone())),
                    remaining: VecDeque::new(),
                    allocator: false,
                    retry_copy: false,
                    records: vec![record],
                    tidied: false,
                });
            } else {
                if previous.is_none() {
                    return Disposition::Disabled;
                }
                let mut tasks: VecDeque<_> = drafts::older_vehicles(&scan, key, None)
                    .into_iter()
                    .map(Vehicle::remove)
                    .collect();
                let Some(task) = tasks.pop_front() else {
                    return Disposition::Waiting;
                };
                host.worker = Some(DraftWorker {
                    pages: keys.clone(),
                    before: scan.logical,
                    task,
                    application: Some(Application::Removal(key.into())),
                    remaining: tasks,
                    allocator: false,
                    retry_copy: false,
                    records: vec![],
                    tidied: false,
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
        });
    }

    /// Advance exactly one draft I/O phase, or its terminal application.
    fn advance_draft(&mut self) -> Disposition {
        let Some(mut worker) = self.worker.take() else {
            return Disposition::Disabled;
        };
        let terminal = matches!(worker.task.stage, Stage::Present | Stage::Absent);
        if !terminal {
            self.with_locks(&worker.pages.clone(), |host| {
                worker.task.advance(&mut host.fs)
            });
            if worker.task.failures >= 3 {
                self.events.push(Event::DraftError {
                    pages: worker.pages.clone(),
                    failures: worker.task.failures,
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
            let keys = worker.pages.clone();
            self.with_locks(&keys, |host| {
                host.apply_draft_terminal(&mut worker, application, present);
            });
        }
        if worker.remaining.is_empty() && !worker.tidied {
            worker.tidied = true;
            // Launch may find identical highest-sequence copies left by a
            // crash during explosion. Retain one, so repeated crashes cannot
            // accumulate recovery vehicles without bound.
            let scan = drafts::scan(self.fs.draft_files(true));
            for key in &worker.pages {
                let mut vehicles: Vec<_> = scan
                    .files
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
                let scan = drafts::scan(self.fs.draft_files(true));
                worker.remaining.extend(
                    drafts::older_vehicles(&scan, &record.page, Some(record.wseq))
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
                    let scan = drafts::scan(self.fs.draft_files(true));
                    for record in &records {
                        worker.remaining.extend(
                            drafts::older_vehicles(&scan, &record.page, Some(record.wseq))
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
            Application::Refresh(_) | Application::Representation => {}
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
            phase: if page.buf.is_none() {
                SavePhase::Check
            } else {
                SavePhase::Temp
            },
            bytes: page.buf.clone(),
            base: page.base.clone(),
            version: page.version,
            epoch: 0,
            removed: None,
            trash_name: uuid::Uuid::new_v4().simple().to_string(),
        });
        Disposition::Pending
    }

    /// `epoch` is the path's external-write epoch supplied by the event driver;
    /// the 2b adapter will obtain it from the watch boundary, not a file hash.
    fn advance_save(&mut self, epoch: u64) -> Disposition {
        let Some(job) = &self.job else {
            return Disposition::Disabled;
        };
        let keys = BTreeSet::from([job.page.clone()]);
        self.with_locks(&keys, |host| host.advance_save_locked(epoch))
    }

    fn advance_save_locked(&mut self, epoch: u64) -> Disposition {
        let Some(mut job) = self.job.take() else {
            return Disposition::Disabled;
        };
        let key = job.page.clone();
        let result = match job.phase {
            SavePhase::Temp => match self.fs.page_temp(&key, &job.bytes) {
                Ok(()) => {
                    job.phase = SavePhase::Check;
                    None
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Check => match self.fs.read_page(&key) {
                Ok(bytes) if job.base == Base::Known(bytes.clone()) => {
                    job.phase = SavePhase::Rename;
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
                    return Disposition::Applied;
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Rename if job.bytes.is_some() => match self.fs.page_rename(&key) {
                Ok(()) => {
                    job.phase = SavePhase::DirectorySync;
                    job.epoch = epoch;
                    None
                }
                Err(e) if e.completed => Some(Outcome::Uncertain),
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Rename => {
                let movement = self.fs.trash_move(&key, &job.trash_name);
                job.removed = movement.removed;
                if job.removed.is_some() {
                    self.events.push(Event::Removed {
                        page: key.clone(),
                        bytes: job.removed.clone(),
                    });
                }
                match movement.result {
                    Ok(()) => {
                        job.phase = SavePhase::TrashSync;
                        job.epoch = epoch;
                        None
                    }
                    Err(e) if e.kind == ErrorKind::Collision => {
                        job.trash_name = uuid::Uuid::new_v4().simple().to_string();
                        None
                    }
                    Err(e) if e.completed => Some(Outcome::Uncertain),
                    Err(_) => Some(Outcome::Failed),
                }
            }
            SavePhase::TrashSync => match self.fs.trash_sync(&key) {
                Ok(_) => {
                    job.phase = SavePhase::DirectorySync;
                    None
                }
                Err(_) => Some(Outcome::Uncertain),
            },
            SavePhase::DirectorySync => match self.fs.page_sync(&key) {
                Ok(witness) => {
                    if witness == Witness::Durable && job.bytes.is_none() {
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
        if let Some(outcome) = result {
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
        if self.allocator_busy() {
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
        if self.job.is_some() || self.worker.is_some() || !self.retained.is_empty() {
            return Disposition::Waiting;
        }
        let keys = self.pages.keys().cloned().collect();
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
        self.version = scan.max_version;
        self.wseq = scan.max_wseq;
        self.incarnation = self
            .incarnation
            .checked_add(1)
            .expect("incarnation exhausted");
        self.window_crash();
        self.last_admitted = 0;
        self.last_applied = 0;
        let keys = scan.logical.keys().cloned().collect();
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
                drafts::older_vehicles(&scan, key, Some(record.wseq))
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
                pages: keys,
                before: scan.logical,
                application: Some(Application::Representation),
                retry_copy: task.bytes.is_some(),
                records,
                task,
                remaining: tasks,
                allocator: true,
                tidied: false,
            });
            Disposition::Pending
        } else {
            Disposition::Applied
        }
    }
}
