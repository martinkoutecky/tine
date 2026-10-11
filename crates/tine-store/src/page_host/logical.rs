//! A key's logical view: the per-page index over the durable draft census
//! (M4) and the logical state every barrier query reads (Q-P2b-4), split
//! from `mod.rs` along that seam.
use super::*;

impl<F: HostIo> Host<F> {
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
}
