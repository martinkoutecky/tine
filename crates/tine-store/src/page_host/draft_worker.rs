//! The host's draft worker (STEP2 §4): the logical draft view, draft
//! installation and the worker's step and terminal handling.
use super::*;

impl<F: HostIo> Host<F> {
    /// Logical entries are computed from durable files and the actual worker.
    /// Pending removal retains its previous entry; pending install is excluded.
    pub(super) fn logical_drafts(&self) -> BTreeMap<PageKey, Record> {
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

    pub(super) fn begin_draft(&mut self, key: &str) -> Disposition {
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

    pub(super) fn install_operation(
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
    pub(super) fn advance_draft(&mut self) -> Disposition {
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

    pub(super) fn apply_draft_terminal(
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
                    let mut pages = vec![request.page.clone()];
                    if let RequestKind::Move { receiver, .. } = &request.kind {
                        pages.insert(0, receiver.clone());
                    }
                    for page in pages {
                        if present {
                            self.answer(&request, &page, true);
                        } else {
                            self.refused(&request, &page, Refusal::DraftFailed);
                        }
                    }
                    self.finish_request(&request);
                }
                // Keep the allocator through explosion (§4), never acquire
                // another page or wait for a version allocator while holding it.
            }
            Application::Refresh(_) => worker.recover_notice = false,
            Application::Representation => {}
        }
    }
}
