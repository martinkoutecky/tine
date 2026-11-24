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

    /// One page's draft step, the model's single-page `draftSync`: never
    /// retires an operation vehicle whole.
    #[cfg(test)]
    pub(super) fn begin_draft(&mut self, key: &str) -> Disposition {
        self.test.single_page_drafts = true;
        let result = self.begin_draft_retiring(key);
        self.test.single_page_drafts = false;
        result
    }

    /// One page's draft step; a removal retires its page's operation vehicle
    /// whole when it can (RENAME-COST L1): each of its pages' removal steps
    /// at one terminal.
    pub(super) fn begin_draft_retiring(&mut self, key: &str) -> Disposition {
        if !self.alive || self.worker.is_some() {
            return Disposition::Waiting;
        }
        let keys = BTreeSet::from([key.into()]);
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            // No worker runs, so the live logical draft is the durable one;
            // `before` keeps only this worker's page (M4).
            let durable: BTreeMap<_, _> = host
                .durable
                .get(key)
                .map(|record| (key.to_owned(), record.clone()))
                .into_iter()
                .collect();
            let previous = durable.get(key);
            let page = host.pages.get(key).cloned();
            let desired = page.as_ref().filter(|p| p.risk);
            if let Some(page) = desired {
                if previous.is_some_and(|r| r.holds(page)) {
                    return Disposition::Disabled;
                }
                let record = host.record(key, page);
                let task = Vehicle::write(drafts::page_name(key), std::slice::from_ref(&record));
                // L2 (RENAME-COST): the new record supersedes this page's
                // record in its operation vehicle, which then retires.
                let absorb = host.live_op(key).map(|op| host.absorb(&op, &keys));
                host.worker = Some(DraftWorker {
                    effect: task.name.clone(),
                    refresh: Some(key.into()),
                    pages: keys.clone(),
                    before: durable.clone(),
                    task,
                    application: Some(Application::Refresh(record.clone())),
                    remaining: absorb.unwrap_or_default(),
                    allocator: false,
                    retry_copy: false,
                    records: vec![record],
                    tidied: false,
                    failures: 0,
                    recover_notice: true,
                    rename: false,
                });
            } else {
                if previous.is_none() {
                    return Disposition::Disabled;
                }
                // A paired operation's records leave only through its
                // settlement: the whole draft once nothing is owed, else a
                // rewrite without the resolved pair whose source is `key`.
                let ops: Vec<String> = (host.vehicles_of.get(key).into_iter().flatten())
                    .filter(|name| name.starts_with("op-"))
                    .cloned()
                    .collect();
                for op in &ops {
                    if host.retire_whole(op) || host.rewrite_pair(op, key) {
                        return Disposition::Pending;
                    }
                }
                // The model's `retained`: a member of a paired draft that
                // still owes a pair keeps its draft, never retired alone;
                // it may go once that pair settles.
                if host.pair_retains(key) {
                    return Disposition::Waiting;
                }
                let op = host.live_op(key);
                // L2 (RENAME-COST): this page's record leaves with its
                // operation vehicle, after the other pages' copies.
                let mut tasks = op.map(|op| host.absorb(&op, &keys)).unwrap_or_default();
                tasks.extend(
                    host.older_vehicles(key, None)
                        .into_iter()
                        .map(Vehicle::remove),
                );
                let Some(task) = tasks.pop_front() else {
                    return Disposition::Waiting;
                };
                // A first copy locks and retries like every later one.
                let records = vehicle_records(&task);
                host.worker = Some(DraftWorker {
                    effect: task.name.clone(),
                    refresh: None,
                    pages: keys.clone(),
                    before: durable.clone(),
                    retry_copy: task.bytes.is_some(),
                    task,
                    application: Some(Application::Removal),
                    remaining: tasks,
                    allocator: false,
                    records,
                    tidied: false,
                    failures: 0,
                    recover_notice: true,
                    rename: false,
                });
            }
            Disposition::Pending
        })
    }

    /// RENAME-COST L1: retire operation vehicle `op` whole when none of its
    /// pages owes its publication and each wants its record gone (not at
    /// risk, the record its winner, not retained). False, with nothing
    /// done, otherwise.
    fn retire_whole(&mut self, op: &str) -> bool {
        #[cfg(test)]
        if self.test.single_page_drafts {
            return false;
        }
        let live = &self.live_ops[op];
        let records = &self.drafts[op];
        // A paired operation's record that a newer vehicle superseded is
        // history: that vehicle holds the page's text (call 20's rewrite
        // drops the settled pair with its own records only).
        let paired = self.order.paired(op).is_some();
        let whole = !live.waits()
            && records.iter().all(|r| {
                let winner = self.durable.get(&r.page).is_some_and(|w| w.wseq == r.wseq);
                let superseded = self.durable.get(&r.page).is_some_and(|w| w.wseq > r.wseq);
                (winner && !self.pages.get(&r.page).is_some_and(|p| p.risk))
                    || (paired && superseded)
            });
        if !whole {
            return false;
        }
        let pages: BTreeSet<PageKey> = live.versions.keys().cloned().collect();
        let records = records.clone();
        let mut tasks: VecDeque<_> = records
            .iter()
            .flat_map(|r| self.older_vehicles(&r.page, Some(r.wseq)))
            .map(Vehicle::remove)
            .collect();
        tasks.push_back(Vehicle::remove(op.into()));
        // A paired operation's decisions leave after it: one left behind by
        // a crash once the operation is durably absent is cleanup only.
        let decisions = self.order.paired(op).map(|p| p.decisions.clone());
        tasks.extend((decisions.into_iter().flatten()).map(|(name, _)| Vehicle::remove(name)));
        let task = tasks.pop_front().unwrap();
        self.worker = Some(DraftWorker {
            effect: op.into(),
            refresh: None,
            pages,
            before: records.into_iter().map(|r| (r.page.clone(), r)).collect(),
            task,
            application: Some(Application::Retire),
            remaining: tasks,
            allocator: false,
            retry_copy: false,
            records: vec![],
            tidied: false,
            failures: 0,
            recover_notice: true,
            rename: false,
        });
        true
    }

    /// The resolved pair of paired operation `op` whose source is `key`
    /// that a settlement rewrite may drop now (its index in the pairing),
    /// the model's `pairRewrite` target: the operation keeps another pair,
    /// every decision of it is one this code acts on, and neither endpoint
    /// still owes the publication of its record (that record is then the
    /// only copy of the page's text, so the host keeps it; the model drops
    /// an endpoint's record whatever its page holds).
    pub(super) fn rewrite_target(&self, op: &str, key: &str) -> Option<usize> {
        let paired = self.order.paired(op)?;
        let live = self.live_ops.get(op)?;
        if paired.unreadable || paired.pairs.len() < 2 || !self.drafts.contains_key(op) {
            return None;
        }
        let at = (paired.pairs.iter()).rposition(|p| p.source == key && !p.phase.unresolved())?;
        let pair = &paired.pairs[at];
        let owed = live.owed.contains(&pair.receiver) || live.owed.contains(&pair.source);
        (!owed).then_some(at)
    }

    /// `key` is the source of a resolved pair a settlement rewrite drops
    /// now: a draft candidate although its operation still waits. Only once
    /// no page of the operation owes its publication: while the operation
    /// is still being saved its pairs settle in turn and the draft retires
    /// whole, so a rewrite per settled pair would only add writes (PC-3).
    pub(super) fn rewritable(&self, key: &str) -> bool {
        let names = self.vehicles_of.get(key).into_iter().flatten();
        names.filter(|name| name.starts_with("op-")).any(|name| {
            self.live_ops.get(name).is_some_and(|op| op.owed.is_empty())
                && self.rewrite_target(name, key).is_some()
        })
    }

    /// The settlement rewrite (the model's `pairRewrite`): replace paired
    /// operation draft `op` by a fresh one without the resolved pair whose
    /// source is `key` and that pair's endpoint records, keeping every other
    /// pair, its decisions (rewritten for the new draft's request id and
    /// member indexes) and the shared referrer records. The new draft and
    /// those decisions are written first (fresh no-replace names, temp,
    /// fsync, rename, private-directory sync); `op` and its decisions are
    /// unlinked only after, so a failure or crash before that unlink leaves
    /// `op` byte-identical, and launch ignores the uncommitted copy
    /// (`drafts::Scan::rewrites`). The operation's pages are the worker's
    /// until it ends. False, with nothing done, when no pair may go.
    fn rewrite_pair(&mut self, op: &str, key: &str) -> bool {
        #[cfg(test)]
        if self.test.single_page_drafts {
            return false;
        }
        let Some(at) = self.rewrite_target(op, key) else {
            return false;
        };
        let paired = &self.order.paired(op).unwrap();
        let target = &paired.pairs[at];
        let dropped = [target.member, target.source_member];
        let records = &self.drafts[op];
        let mut index = BTreeMap::new();
        let mut kept = vec![];
        for (i, record) in records.iter().enumerate() {
            let i = i as u32;
            if !dropped.contains(&i) {
                index.insert(i, kept.len() as u32);
                kept.push(record.clone());
            }
        }
        let pairs: drafts::Pairs = (paired.pairs.iter().enumerate())
            .filter(|(i, _)| *i != at)
            .map(|(_, p)| (index[&p.member], index[&p.source_member]))
            .collect();
        let id = uuid::Uuid::new_v4().into_bytes();
        let new = drafts::op_vehicle(&id);
        let mut tasks = VecDeque::from([Vehicle::write_operation(new.clone(), &kept, &pairs)]);
        let mut decisions = BTreeMap::new();
        for decision in paired.decisions.values() {
            // `rewrite_target` admits only decisions this code acts on.
            let Some(d) = decision else {
                return false;
            };
            if dropped.contains(&d.member) {
                continue;
            }
            let Some(&member) = index.get(&d.member) else {
                return false;
            };
            let moved = drafts::Decision {
                op: id,
                member,
                wseq: d.wseq,
                choice: d.choice.clone(),
            };
            let file = drafts::decision_vehicle(&records[d.member as usize].page, &id, member);
            tasks.push_back(Vehicle::write_decision(file.clone(), &moved));
            decisions.insert(file, Some(moved));
        }
        tasks.push_back(Vehicle::remove(op.into()));
        tasks.extend(paired.decisions.keys().cloned().map(Vehicle::remove));
        let pages: BTreeSet<PageKey> = self.live_ops[op].versions.keys().cloned().collect();
        let before = (pages.iter())
            .filter_map(|page| Some((page.clone(), self.durable.get(page)?.clone())))
            .collect();
        let task = tasks.pop_front().unwrap();
        self.worker = Some(DraftWorker {
            effect: op.into(),
            refresh: None,
            pages,
            before,
            records: vehicle_records(&task),
            task,
            application: Some(Application::Rewrite(Box::new(Rewrite {
                old: op.into(),
                new,
                id,
                records: kept,
                pairs,
                decisions,
                written: vec![],
                failed: false,
            }))),
            remaining: tasks,
            allocator: false,
            retry_copy: false,
            tidied: false,
            failures: 0,
            recover_notice: true,
            rename: false,
        });
        true
    }

    /// A settlement rewrite's vehicle reached its terminal: record what it
    /// made durable, abort on a failed write (unlinking what it wrote, so
    /// the original stays the operation), and move the surviving pairs to
    /// the new draft once every write is durable, before the original's
    /// unlink. Then the next vehicle, or the application terminal.
    fn rewrite_step(&mut self, worker: &mut DraftWorker) -> bool {
        let Some(Application::Rewrite(rewrite)) = &mut worker.application else {
            return false;
        };
        if worker.task.bytes.is_some() && !rewrite.failed {
            if worker.task.stage == Stage::Present {
                rewrite.written.push(worker.task.name.clone());
            } else {
                rewrite.failed = true;
                worker.remaining = (rewrite.written.iter().rev().cloned())
                    .map(Vehicle::remove)
                    .collect();
            }
        }
        let Some(next) = worker.remaining.pop_front() else {
            return false;
        };
        if !rewrite.failed && next.bytes.is_none() && next.name == rewrite.old {
            self.order.rewrite(
                &rewrite.old,
                rewrite.new.clone(),
                rewrite.id,
                &rewrite.records,
                &rewrite.pairs,
                std::mem::take(&mut rewrite.decisions),
            );
            let new = rewrite.new.clone();
            self.repair_op(&new);
        }
        worker.records = vehicle_records(&next);
        worker.retry_copy = false;
        worker.task = next;
        true
    }

    /// L2 (RENAME-COST): the vehicles that retire live operation vehicle
    /// `op` once an effect supersedes or removes its records of `except`:
    /// a durable copy of each other page's record, then its unlink. For
    /// those pages a representation stutter (§2a rule 3).
    pub(super) fn absorb(&self, op: &str, except: &BTreeSet<PageKey>) -> VecDeque<Vehicle> {
        // A paired operation draft is immutable: a newer page draft wins for
        // editor text without erasing the original obligation, and a pair's
        // records never leave their pair as standalone records (a settled
        // pair leaves only through its whole retirement, `retire_whole`).
        if self.order.paired(op).is_some() {
            return VecDeque::new();
        }
        let records = self.drafts[op].iter().filter(|r| !except.contains(&r.page));
        let mut tasks: VecDeque<_> = records
            .map(|r| Vehicle::write(drafts::page_name(&r.page), std::slice::from_ref(r)))
            .collect();
        tasks.push_back(Vehicle::remove(op.into()));
        tasks
    }

    pub(super) fn install_operation(
        &mut self,
        pages: BTreeMap<PageKey, Page>,
        records: Vec<Record>,
        reply: Reply,
        last_version: u64,
        gates: Option<Gates>,
    ) {
        let keys: BTreeSet<PageKey> = pages.keys().cloned().collect();
        // The model's ghost `opRead`: a caller's operation read each page's
        // base at capture; a window move reads none.
        #[cfg(test)]
        let reads = match reply {
            Reply::Caller(_) => (pages.iter())
                .map(|(key, page)| (key.clone(), page.base.clone()))
                .collect(),
            Reply::Window(_) => BTreeMap::new(),
        };
        // M4: the live logical drafts of this worker's pages only; a pending
        // install exposes nothing else (`logical_drafts`).
        let before = keys
            .iter()
            .filter_map(|key| Some((key.clone(), self.draft_of(key)?.clone())))
            .collect();
        let name = match reply {
            Reply::Window(_) => drafts::page_name(&records[0].page),
            Reply::Caller(_) => drafts::op_name(),
        };
        // Every rename writes a page, and so does an apply that is not only
        // deletions; a deletion's records have no bytes (M6 `renaming`).
        let rename = matches!(reply, Reply::Caller(_)) && records.iter().any(|r| r.bytes.is_some());
        // A caller's operation with moves writes the paired operation draft
        // (its pairing survives relaunch); everything else the legacy body.
        let pairs = gates
            .as_ref()
            .filter(|_| matches!(reply, Reply::Caller(_)))
            .and_then(|gates| drafts::pairing(&records, gates.moves()));
        let task = match &pairs {
            Some(pairs) => Vehicle::write_operation(name.clone(), &records, pairs),
            None => Vehicle::write(name.clone(), &records),
        };
        self.worker = Some(DraftWorker {
            effect: name.clone(),
            refresh: None,
            pages: keys,
            before,
            task,
            remaining: VecDeque::new(),
            application: Some(Application::Operation {
                pages,
                records: records.clone(),
                reply,
                last_version,
                #[cfg(test)]
                reads,
                gates,
            }),
            allocator: true,
            retry_copy: false,
            records,
            tidied: false,
            failures: 0,
            recover_notice: true,
            rename,
        });
    }

    /// Advance exactly one draft I/O phase, or its terminal application.
    pub(super) fn advance_draft(&mut self) -> Disposition {
        let Some(worker) = &self.worker else {
            return Disposition::Disabled;
        };
        #[cfg(any(test, feature = "test-faults"))]
        if !self.paused_for_tests.is_disjoint(&worker.pages) {
            return Disposition::Waiting;
        }
        #[cfg(test)]
        if self.test.paused {
            return Disposition::Waiting;
        }
        // M4 (REVIEW-progress-plan B3): a physical phase locks only the pages
        // its vehicle names (an operation's write: all of them; a copy: its
        // page; an unlink: none), and the terminal application locks the
        // whole set it applies at once. The worker's custody of its other
        // pages stays logical (`busy`), so no phase copies or relocks the
        // whole operation.
        let terminal = matches!(worker.task.stage, Stage::Present | Stage::Absent);
        let keys: BTreeSet<PageKey> = match (terminal, &worker.application) {
            (true, Some(_)) => worker.pages.clone(),
            (true, None) => BTreeSet::new(),
            (false, _) => worker.records.iter().map(|r| r.page.clone()).collect(),
        };
        work(Work::WorkerKeys, keys.len());
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        let mut worker = self.worker.take().unwrap();
        if !terminal {
            let failures = worker.task.failures;
            self.with_locks(&keys, |host| {
                worker.task.advance(&mut host.fs);
                // Count a durable vehicle write once, on transition to Present.
                // Cleanup/removal and unsuccessful writes never count.
                if worker.task.stage == Stage::Present {
                    host.storage_diag
                        .drafts_written
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
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
                    || (worker.task.stage == Stage::Absent
                        && worker.refresh.is_some()
                        && worker.application.is_some()))
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
        // A settlement rewrite runs its own vehicle sequence; with none
        // left, its application terminal follows below.
        if self.rewrite_step(&mut worker) {
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        if worker.retry_copy && worker.task.stage == Stage::Absent && worker.task.bytes.is_some() {
            worker.task =
                Vehicle::write(drafts::page_name(&worker.records[0].page), &worker.records);
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        // Removal becomes abstractly effective only after its last vehicle
        // step: its unlinks, and the copies an operation vehicle it retires
        // owes its other pages (RENAME-COST L2).
        let removal = matches!(
            worker.application,
            Some(Application::Removal | Application::Retire | Application::Rewrite(_))
        );
        if (removal || worker.task.bytes.is_none()) && !worker.remaining.is_empty() {
            let task = worker.remaining.pop_front().unwrap();
            worker.records = vehicle_records(&task);
            worker.retry_copy = task.bytes.is_some();
            worker.task = task;
            self.worker = Some(worker);
            return Disposition::Pending;
        }
        if let Some(application) = worker.application.take() {
            let present = worker.task.stage == Stage::Present;
            self.with_locks(&keys, |host| {
                host.apply_draft_terminal(&mut worker, application, present);
            });
            // The pending install's overlay ends: its pages' live logical
            // drafts are the durable ones from here on (M4).
            for key in &worker.pages {
                self.mark(key);
            }
        }
        if worker.remaining.is_empty() && !worker.tidied {
            worker.tidied = true;
            // Launch may find identical highest-sequence copies left by a
            // crash during explosion. Retain one, so repeated crashes cannot
            // accumulate recovery vehicles without bound.
            for key in &worker.pages {
                let mut vehicles = self.page_vehicles(key);
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
                self.bench_draft_durable(&record);
                #[cfg(test)]
                self.events.push(Event::Draft(record.clone()));
                worker.remaining.extend(
                    self.older_vehicles(&record.page, Some(record.wseq))
                        .into_iter()
                        .map(Vehicle::remove),
                );
            }
            #[cfg(test)]
            Application::Removal | Application::Retire => {
                for key in &worker.pages {
                    self.events.push(Event::DraftRemoved(key.clone()));
                }
            }
            #[cfg(not(test))]
            Application::Removal | Application::Retire => {}
            // Committed at the original's durable unlink: the surviving
            // pairs moved before it (`rewrite_step`), the retired pair left
            // with it. A failed rewrite changed nothing.
            Application::Rewrite(_) => {}
            Application::Operation {
                pages,
                records,
                reply,
                last_version,
                #[cfg(test)]
                reads,
                gates,
            } => {
                let versions: BTreeMap<PageKey, u64> =
                    pages.iter().map(|(k, p)| (k.clone(), p.version)).collect();
                if present {
                    let paired = drafts::op_id(&worker.task.name).and_then(|id| {
                        let moves = gates.as_ref()?.moves();
                        let pairs = drafts::pairing(&records, moves)?;
                        Some(order::PairedOp::new(id, &records, &pairs))
                    });
                    if let Some(paired) = paired {
                        self.order.adopt(worker.task.name.clone(), paired);
                        self.repair_op(&worker.task.name.clone());
                    }
                    self.order.install(gates);
                    self.version = last_version;
                    for (key, page) in pages {
                        self.set_page(&key, Some(page));
                    }
                    #[cfg(test)]
                    for (key, base) in reads {
                        self.events.push(Event::OperationRead { page: key, base });
                    }
                    for record in &records {
                        self.bench_draft_durable(record);
                    }
                    #[cfg(test)]
                    self.events
                        .extend(records.iter().cloned().map(Event::Draft));
                    // L2 (RENAME-COST): an older operation vehicle naming a
                    // page this one writes retires now, after copies of its
                    // other pages' records.
                    let written: BTreeSet<PageKey> =
                        records.iter().map(|r| r.page.clone()).collect();
                    let older: BTreeSet<String> = written
                        .iter()
                        .flat_map(|key| self.vehicles_of.get(key).into_iter().flatten())
                        .filter(|name| name.starts_with("op-") && **name != worker.task.name)
                        .cloned()
                        .collect();
                    for op in older {
                        worker.remaining.extend(self.absorb(&op, &written));
                    }
                    for record in &records {
                        worker.remaining.extend(
                            self.older_vehicles(&record.page, Some(record.wseq))
                                .into_iter()
                                .map(Vehicle::remove),
                        );
                    }
                }
                if let Reply::Caller(id) = reply {
                    let pages: BTreeSet<_> = versions.keys().cloned().collect();
                    // Its caller waits for these pages to publish: their
                    // saves skip the edit debounce (`Progress::reconcile`).
                    // A window Move is an ordinary edit and keeps it.
                    for key in pages.iter().filter(|_| present) {
                        self.changed.insert(key.clone(), true);
                    }
                    let reply = if present {
                        OperationReply::Applied(versions)
                    } else {
                        OperationReply::DraftFailed
                    };
                    // Q2: a caller already told Uncertain learns the outcome
                    // from its pages' notice.
                    if !self.order.answer(id, reply) && !present {
                        self.events.push(Event::OperationDropped(pages));
                    }
                } else if let Reply::Window(request) = reply {
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
                // Keep the allocator until the worker ends; never acquire
                // another page or wait for a version allocator while holding it.
            }
            Application::Refresh(_) => {
                worker.recover_notice = false;
                // The operation vehicle this refresh would have retired
                // still holds the page's newest record (RENAME-COST L2).
                worker.remaining.clear();
            }
            Application::Representation => {}
        }
    }

    /// Bench events: `record`'s vehicle is durable (renamed and its
    /// directory synced) and the host installs it.
    fn bench_draft_durable(&self, record: &Record) {
        crate::bench_events::emit(|| crate::bench_events::Ev::DraftDurable {
            path: self.fs.spelling(&record.page),
            text: record.bytes.clone(),
        });
    }
}

/// The records a queued vehicle writes (none for an unlink): a copy's phases
/// lock their pages and a failed copy is rewritten from them.
fn vehicle_records(task: &Vehicle) -> Vec<Record> {
    task.bytes
        .as_ref()
        .and_then(|b| drafts::decode(b).ok())
        .unwrap_or_default()
}
