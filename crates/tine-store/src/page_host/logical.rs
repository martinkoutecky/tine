//! A key's logical view: the per-page index over the durable draft census
//! (M4) and the logical state every barrier query reads (Q-P2b-4), split
//! from `mod.rs` along that seam.
use super::*;

/// RENAME-COST L1: whether a page held as `page` still owes the publication
/// of an operation record at `version`.
pub(super) fn owes(page: Option<&Page>, version: u64) -> bool {
    page.is_some_and(|p| p.risk && !p.conflict && p.version == version)
}

/// RENAME-COST L1: a durable operation vehicle retires whole once none of
/// its pages still owes the publication of its record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LiveOp {
    /// Each page the vehicle names, at its record's version.
    pub(super) versions: BTreeMap<PageKey, u64>,
    /// Pages held at risk, without a conflict, at their record's version:
    /// the operation's saves still to publish.
    pub(super) owed: BTreeSet<PageKey>,
    /// A paired operation whose `Order` still owes a source settlement:
    /// its vehicle neither retires nor splits (`Order::holds`).
    pub(super) held: bool,
}

impl LiveOp {
    /// Its pages keep their records in it: a publication or a pair's
    /// settlement is still owed.
    pub(super) fn waits(&self) -> bool {
        !self.owed.is_empty() || self.held
    }
}

/// A host's test-only schedule knobs.
#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct TestKnobs {
    /// Q-P2b-4's deterministic schedules: no admitted request applies and the
    /// draft worker does not advance while set.
    pub(super) paused: bool,
    /// A window Submit keeps the edit debounce, as before PERF-FIXES D1, for
    /// a test whose scenario needs a submitted page to stay dirty (unsaved
    /// or closed before its save). Production submits save at once.
    pub(super) submits_wait: bool,
    /// `begin_draft`: the model's single-page `draftSync`, which never
    /// retires an operation vehicle whole.
    pub(super) single_page_drafts: bool,
    /// `delete_loaded`: the s2 model's `opDelete`, which refuses a page the
    /// host does not hold instead of loading it.
    pub(super) loaded_deletes: bool,
    /// `advance_save_at`: the path's external-write epoch the model's
    /// conformance promise reads from `Published` (a ghost; production has
    /// none).
    pub(super) epoch: u64,
}

impl<F: HostIo> Host<F> {
    /// A window Submit's page (PERF-FIXES D1): the window already waited for
    /// the end of the typing burst (400 ms, 1 s at most), so its save skips
    /// the host's edit debounce. One owner of "when to save" per edit kind.
    pub(super) fn set_submitted(&mut self, key: &str, page: Page) {
        self.set_page(key, Some(page));
        #[cfg(test)]
        if self.test.submits_wait {
            return;
        }
        self.changed.insert(key.into(), true);
    }

    /// RENAME-COST L1, at `stop`: no page is held, so none owes an
    /// operation's publication.
    pub(super) fn release_owed(&mut self) {
        for op in self.live_ops.values_mut() {
            op.owed.clear();
        }
    }

    /// Apply the adapter's durable census changes to the draft index.
    pub(super) fn sync_drafts(&mut self) {
        for (name, bytes) in self.fs.draft_changes() {
            let records = bytes.and_then(|bytes| drafts::vehicle(&name, &bytes));
            self.index_vehicle(name, records);
        }
    }

    /// Install or remove vehicle `name` in `drafts`, keeping the per-page
    /// index exact: a removed winning record is recomputed from the page's
    /// remaining vehicles. Every page either version names is marked.
    pub(super) fn index_vehicle(&mut self, name: String, records: Option<Vec<Record>>) {
        let old = match records {
            Some(records) => self.drafts.insert(name.clone(), records),
            None => self.drafts.remove(&name),
        };
        let mut stale = BTreeSet::new();
        for record in old.iter().flatten() {
            work(Work::DraftRecords, 1);
            self.mark(&record.page);
            if let Some(names) = self.vehicles_of.get_mut(&record.page) {
                names.remove(&name);
                if names.is_empty() {
                    self.vehicles_of.remove(&record.page);
                }
            }
            if self
                .durable
                .get(&record.page)
                .is_some_and(|winner| winner.wseq == record.wseq)
            {
                stale.insert(record.page.clone());
            }
        }
        let new = self.drafts.get(&name).cloned().unwrap_or_default();
        for record in new {
            work(Work::DraftRecords, 1);
            self.mark(&record.page);
            let names = self.vehicles_of.entry(record.page.clone()).or_default();
            names.insert(name.clone());
            if !stale.contains(&record.page)
                && self
                    .durable
                    .get(&record.page)
                    .is_none_or(|winner| record.wseq > winner.wseq)
            {
                self.durable.insert(record.page.clone(), record);
            }
        }
        for page in stale {
            self.reindex(&page);
        }
        self.index_op(&name);
    }

    /// RENAME-COST L1: recompute operation vehicle `name`'s entry in
    /// `live_ops` from `drafts` and `pages` (dropped once it is gone).
    pub(super) fn index_op(&mut self, name: &str) {
        let records = self.drafts.get(name).filter(|_| name.starts_with("op-"));
        let Some(records) = records else {
            self.live_ops.remove(name);
            // Its vehicle is durably gone: the pairing owes nothing more.
            self.order.forget(name);
            return;
        };
        let versions: BTreeMap<PageKey, u64> = records
            .iter()
            .inspect(|_| work(Work::DraftRecords, 1))
            .map(|r| (r.page.clone(), r.version))
            .collect();
        let owed = versions
            .iter()
            .filter(|(key, version)| owes(self.pages.get(*key), **version))
            .map(|(key, _)| key.clone())
            .collect();
        let held = self.order.holds(name);
        self.live_ops.insert(
            name.into(),
            LiveOp {
                versions,
                owed,
                held,
            },
        );
    }

    /// `Order` changed paired operation `name`: refresh its entry, marking
    /// its pages when whether it waits changed (their draft candidacy).
    pub(super) fn repair_op(&mut self, name: &str) {
        let before = self.live_ops.get(name).map(LiveOp::waits);
        self.index_op(name);
        let after = self.live_ops.get(name).map(LiveOp::waits);
        if before != after {
            let pages: Vec<PageKey> = (self.live_ops.get(name))
                .map(|op| op.versions.keys().cloned().collect())
                .unwrap_or_default();
            for page in pages {
                self.mark(&page);
            }
        }
    }

    /// RENAME-COST L1: keep the owed sets of `key`'s operation vehicles in
    /// step with its page. A vehicle whose owed set empties or fills marks
    /// all its pages: their draft candidacy depends on it (`op_waits`).
    pub(super) fn reindex_owed(&mut self, key: &str) {
        let ops = self.vehicles_of.get(key).into_iter().flatten();
        let ops: Vec<String> = ops.filter(|n| n.starts_with("op-")).cloned().collect();
        for name in ops {
            let Some(op) = self.live_ops.get_mut(&name) else {
                continue;
            };
            let settled = !op.waits();
            if owes(self.pages.get(key), op.versions[key]) {
                op.owed.insert(key.into());
            } else {
                op.owed.remove(key);
            }
            if settled != !op.waits() {
                let pages: Vec<PageKey> = op.versions.keys().cloned().collect();
                for page in pages {
                    self.mark(&page);
                }
            }
        }
    }

    /// RENAME-COST L1: the operation vehicle holding a record of `key`.
    /// Outside a running worker there is at most one unpaired one: an
    /// operation install absorbs its pages' older unpaired vehicle before it
    /// finishes, and launch explodes every vehicle (pinned by
    /// `rename_cost_an_op_absorbing_an_older_op_keeps_published_bytes_at_every_cut`).
    /// A paired operation vehicle is never absorbed (it leaves whole once its
    /// pair settled), so it may share a page with that one: the unpaired
    /// vehicle is preferred.
    pub(super) fn live_op(&self, key: &str) -> Option<String> {
        let names = self.vehicles_of.get(key).into_iter().flatten();
        let mut ops = names.filter(|n| n.starts_with("op-"));
        let ops: Vec<&String> = ops.by_ref().collect();
        (ops.iter().find(|n| self.order.paired(n).is_none()))
            .or(ops.first())
            .map(|n| (*n).clone())
    }

    /// RENAME-COST L1: a page whose desired draft is none keeps its record
    /// in its operation vehicle, rather than retiring it alone, while the
    /// vehicle still owes another page's publication.
    pub(super) fn op_waits(&self, key: &str) -> bool {
        let names = self.vehicles_of.get(key).into_iter().flatten();
        names
            .filter_map(|n| self.live_ops.get(n))
            .any(LiveOp::waits)
    }

    /// `key`'s records stay in a paired operation draft that still owes a
    /// settlement: no stop or retirement removes them alone.
    pub(super) fn pair_retains(&self, key: &str) -> bool {
        let names = self.vehicles_of.get(key).into_iter().flatten();
        names
            .filter(|n| self.order.paired(n).is_some())
            .filter_map(|n| self.live_ops.get(n))
            .any(LiveOp::waits)
    }

    /// Recompute `page`'s highest-sequence record from its vehicles.
    pub(super) fn reindex(&mut self, page: &str) {
        let mut best: Option<&Record> = None;
        for name in self.vehicles_of.get(page).into_iter().flatten() {
            for record in &self.drafts[name] {
                work(Work::DraftRecords, 1);
                if record.page == page && best.is_none_or(|b| record.wseq > b.wseq) {
                    best = Some(record);
                }
            }
        }
        match best.cloned() {
            Some(record) => self.durable.insert(page.into(), record),
            None => self.durable.remove(page),
        };
    }

    /// Rebuild the per-page index from the whole of `drafts` (a host's
    /// construction, launch's census rebuild), marking every page it names
    /// before and after.
    pub(super) fn index_drafts(&mut self) {
        let before: Vec<_> = self.durable.keys().cloned().collect();
        for key in before {
            self.mark(&key);
        }
        self.vehicles_of.clear();
        for (name, records) in &self.drafts {
            for record in records {
                let names = self.vehicles_of.entry(record.page.clone()).or_default();
                names.insert(name.clone());
            }
        }
        self.durable = drafts::logical(self.drafts.values());
        self.live_ops.clear();
        let ops: Vec<String> = self
            .drafts
            .keys()
            .filter(|n| n.starts_with("op-"))
            .cloned()
            .collect();
        for name in ops {
            self.index_op(&name);
        }
        let after: Vec<_> = self.durable.keys().cloned().collect();
        for key in after {
            self.mark(&key);
        }
    }

    /// `page`'s page vehicles (`p-`), by the index: (write sequence, name).
    pub(super) fn page_vehicles(&self, page: &str) -> Vec<(u64, String)> {
        let names = self.vehicles_of.get(page).into_iter().flatten();
        let mut vehicles: Vec<_> = names
            .filter(|name| name.starts_with("p-"))
            .inspect(|_| work(Work::DraftRecords, 1))
            .map(|name| (self.drafts[name][0].wseq, name.clone()))
            .collect();
        vehicles.sort();
        vehicles
    }

    /// `page`'s page vehicles below write sequence `keep` (all with None),
    /// oldest first.
    pub(super) fn older_vehicles(&self, page: &str, keep: Option<u64>) -> Vec<String> {
        let vehicles = self.page_vehicles(page).into_iter();
        vehicles
            .filter(|(wseq, _)| keep.is_none_or(|keep| *wseq < keep))
            .map(|(_, name)| name)
            .collect()
    }

    /// `key`'s live logical draft (M4): what `logical_drafts` holds for it
    /// while the host runs, by lookup. A pending install exposes the worker's
    /// `before` for its pages; a stopped host has no live projection.
    pub(super) fn draft_of(&self, key: &str) -> Option<&Record> {
        debug_assert!(
            self.alive,
            "a stopped host's drafts are its readable directory"
        );
        let worker = self.worker.as_ref().filter(|w| w.application.is_some());
        match worker {
            Some(worker) if worker.pages.contains(key) => worker.before.get(key),
            _ => self.durable.get(key),
        }
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
    pub(super) fn logical(&self, key: &str) -> Logical<'_> {
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
    pub(super) fn logical_keys(&self) -> BTreeSet<PageKey> {
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

    /// Launch: adopt every readable paired operation into `Order` with its
    /// decisions. Returns the keys whose logical record is a cancelled
    /// pair's operation record (not recovered).
    pub(super) fn adopt_pairing(&mut self, scan: &drafts::Scan) -> BTreeSet<PageKey> {
        let mut skipped = BTreeSet::new();
        for (name, pairs) in &scan.pairs {
            let Some(id) = drafts::op_id(name) else {
                continue;
            };
            let records = &scan.files[name];
            self.order
                .adopt(name.clone(), order::PairedOp::new(id, records, pairs));
        }
        // A receiver's decision outranks its source's own Use Disk.
        for (decision, d) in &scan.decisions {
            if d.choice == drafts::Choice::TakeDisk {
                let name = drafts::op_vehicle(&d.op);
                self.order
                    .decide(&name, d.member, None, Some(decision.clone()));
            }
        }
        // Where each decided pair's original text stands now: one read of
        // each endpoint, for decided pairs only (none on a clean launch).
        let reads: BTreeMap<PageKey, Option<Text>> = (self.order.decided_pages())
            .into_iter()
            .map(|key| {
                let bytes = self.fs.read_page(&key).ok();
                (key, bytes)
            })
            .collect();
        self.order.decided_phases(&reads);
        for (decision, name) in &scan.blocking {
            self.order.block(name, decision.clone());
            // The existing draft error: the decision stays in place and
            // the operation's destructive step waits for it.
            let pages = self.order.paired(name).map(|op| op.pages.clone());
            self.events.push(Event::DraftError {
                effect: decision.clone(),
                pages: pages.unwrap_or_default(),
                refresh: None,
                failures: 1,
            });
        }
        for (name, records) in &scan.files {
            let Some(op) = self.order.paired(name) else {
                continue;
            };
            for pair in &op.pairs {
                let keys = match pair.choice {
                    order::PairChoice::None => vec![],
                    order::PairChoice::SourceDisk => vec![&pair.source],
                    order::PairChoice::ReceiverDisk => vec![&pair.receiver, &pair.source],
                };
                for key in keys {
                    let own = records.iter().find(|r| &r.page == key).map(|r| r.wseq);
                    if scan.logical.get(key).map(|r| r.wseq) == own {
                        skipped.insert(key.clone());
                    }
                }
            }
        }
        skipped
    }

    /// A successful Retry made the draft store durable again: pairs the
    /// down store blocked resume, and their sources may save.
    pub(super) fn unblock_pairs(&mut self) {
        for name in self.order.unblock_store() {
            self.repair_op(&name);
            let sources: Vec<PageKey> = (self.order.paired(&name))
                .map(|op| op.pairs.iter().map(|p| p.source.clone()).collect())
                .unwrap_or_default();
            for source in sources {
                self.mark(&source);
            }
        }
    }

    /// Launch: each recovered paired operation's order over its recovered
    /// pages, receivers first, sources last, as while Tine ran.
    pub(super) fn install_recovered_gates(&mut self) {
        for gates in self.order.recovered_gates(&self.pages) {
            self.order.install(Some(gates));
        }
    }
}
