//! The page host's launch (§4, B-Q1): binding a host to a graph, its one
//! launch, which never fails on draft I/O, and crash-recovery status and
//! Retry (S3).
use super::*;
use crate::page_host::driver::{Deadline, State, Wait};

/// Why no host could start: the defensive launch error.
const DOWN: &str = "the page host did not launch";

impl PageHost {
    /// Bind a page host to `store`'s graph: drafts under
    /// `app_data/drafts-v2/<graph_id>`, recovered draft keys registered
    /// before launch (§2), then the driver, which owns the index of every
    /// page it holds (§5). `mail` runs on the driver thread, and so does
    /// `kinds`, which takes each published page's edit kinds (OG-RULES
    /// Rule 8; the app logs them). Each launch
    /// draws a fresh session (E101). No draft lock exists: a graph opened
    /// again in this process routes to its existing binding, so production
    /// starts one host per graph (`src-tauri/src/graph.rs`
    /// `load_graph_for_label` → `route_bound_root`, step 3b P2b). Draft I/O
    /// failures leave the host up with draft I/O down; the error is only
    /// the defensive "did not launch", when the host is not alive after
    /// its launch.
    pub fn start(
        store: &Arc<Store>,
        app_data: &Path,
        graph_id: &str,
        kinds: impl FnMut(&str, &[EditKind]) + Send + 'static,
        mail: impl FnMut(PageMail) + Send + 'static,
    ) -> Result<Self, String> {
        Self::launch(Launch {
            store: store.clone(),
            app_data: app_data.to_path_buf(),
            graph_id: graph_id.into(),
            mail: Arc::new(Mutex::new(Box::new(mail))),
            kinds: Arc::new(Mutex::new(Box::new(kinds))),
        })
    }

    /// A host for tests outside this crate (retained writers in
    /// tine-graph-features, the app's slot fixtures) under `app_data`, mail
    /// discarded.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn start_for_tests(store: &Arc<Store>, app_data: &Path) -> Result<Self, String> {
        Self::start(store, app_data, "test-graph", |_, _| {}, |_| {})
    }

    /// The host holds nothing and owes nothing: no page, job, draft worker,
    /// durable draft, queued request, owed observation or index ownership. A cost test waits
    /// for it so it measures an operation through its cleanup (M4).
    #[cfg(feature = "test-faults")]
    pub fn quiescent_for_tests(&self) -> bool {
        // A read does not wake the driver, so polling adds no host work.
        let read = self.driver.shared.read(|state| {
            let host = &state.progress.host;
            host.pages.is_empty()
                && host.job.is_none()
                && host.worker.is_none()
                && host.drafts.is_empty()
                && host.queue.is_empty()
                && host.applying.is_none()
                && state.observe.is_empty()
                && state.book.owned().is_empty()
        });
        read.unwrap_or(false)
    }

    /// A cost test's measurement boundary (REVIEW-3b-P2b-2 N3): the watcher
    /// drained (every notification of a write so far reconciled,
    /// `watch/test_drain.rs`), then the host quiescent, every forwarded
    /// observation processed; repeated until a whole round leaves the
    /// driver's wake sequence unchanged (nothing forwarded, submitted or
    /// delivered meanwhile; per host, so other tests' work cannot hold it
    /// open). False at `timeout`.
    #[cfg(feature = "test-faults")]
    pub fn quiesce_for_tests(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let left = || deadline.saturating_duration_since(std::time::Instant::now());
        let mut last = None;
        loop {
            if !self.store.watcher_drained_for_tests(left()) {
                return false;
            }
            while !self.quiescent_for_tests() {
                if left().is_zero() {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let Ok(now) = self.driver.shared.read(|state| state.wake_seq()) else {
                return false;
            };
            if last == Some(now) {
                return true;
            }
            last = Some(now);
        }
    }

    pub(super) fn launch(launch: Launch) -> Result<Self, String> {
        let Launch {
            store,
            app_data,
            graph_id,
            ..
        } = &launch;
        let (app_data, graph_id) = (app_data.as_path(), graph_id.as_str());
        let graph = store.graph.clone();
        let trash = crate::model::trash_root(&graph.root).join("pages");
        let mut io = ProductionIo::attach(&graph.root, app_data, graph_id, &trash);
        let spellings = io.spellings().clone();
        io.marks = Some(graph.clone());
        let mut host = Host::new(io, BTreeMap::new());
        host.stop();
        let mut recovered = Vec::new();
        let keys = host.recovered_keys();
        for key in keys.iter().cloned() {
            let id = PageId::from(super::super::production::key_base(&key));
            let spelling = store.disk_spelling(&id).unwrap_or(id);
            let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
            host.register(key.clone(), spelling.as_str(), lock);
            recovered.push((key, spelling));
        }
        #[cfg(test)]
        let index_faults = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let bridge = Bridge {
            store: store.clone(),
            mail: launch.mail.clone(),
            kinds: launch.kinds.clone(),
            #[cfg(test)]
            index_faults: index_faults.clone(),
        };
        let driver = Driver::spawn(host, SystemClock::new(), bridge);
        let session = next_session();
        (driver
            .shared
            .call(move |state| state.book.session = session))
        .map_err(|_| DOWN)?;
        // A watcher read of a held page (§5): the driver owes the host an
        // observation of it, taken under its path lock once the page is idle.
        let shared = Arc::downgrade(&driver.shared);
        let forward: crate::watch::Forward = Box::new(move |keys| {
            if let Some(shared) = shared.upgrade() {
                // A notice: the watcher calls this holding the writer.
                shared.notify(|state| {
                    for key in keys {
                        state.observe.entry(key).or_default().forwarded = true;
                    }
                });
            }
        });
        let mut this = Self {
            driver,
            store: store.clone(),
            launch: launch.clone(),
            recovered: BTreeMap::new(),
            #[cfg(test)]
            index_faults,
        };
        // Recovered pages are held before launch reads them (§5), by the
        // host's keys and spelling table (B1).
        let watched = store.clone();
        let held = this.driver.shared.call_writer(&store.writer, move |state| {
            watched.watch.forward_held(forward, spellings);
            for (key, _) in &recovered {
                watched.watch.hold(key.clone());
            }
            watched.publish_retired();
            let keys = recovered.into_iter().map(|(k, _)| k);
            state.book.own(keys, &state.progress.host)
        });
        held.map_err(|_| DOWN)?;
        // Draft I/O failures never stop a launch (§4, B-Q1): they leave it
        // down, and `draft_status` says why.
        this.locked(|host| host.launch());
        let launched = this.driver.shared.read(move |state| {
            let host = &state.progress.host;
            let recovered = host
                .pages
                .iter()
                .filter(|(key, _)| keys.contains(*key))
                .map(|(key, page)| (key.clone(), page.version))
                .collect();
            (host.alive, recovered)
        });
        let (alive, recovered) = launched.map_err(|_| DOWN)?;
        this.recovered = recovered;
        if alive {
            let drafts = this.recovered.len();
            crate::bench_events::emit(|| crate::bench_events::Ev::LaunchRecovered { drafts });
            return Ok(this);
        }
        drop(this);
        Err(DOWN.into())
    }

    /// Crash-recovery availability (§4, B-Q1), and the pages whose text is
    /// not on disk yet (SPEC-s2 §4.11, S9), for the `load_graph` reply: a
    /// launch names what it recovered, an adopted host (B-QA) what it still
    /// holds. Bounded by the held pages and admitted input; no I/O.
    pub fn draft_status(&self) -> DraftStatus {
        let recovered = self.recovered.clone();
        let status = self.driver.shared.read(move |state| {
            let host = &state.progress.host;
            let mut status = host.fs.draft_status();
            status.unsaved = host
                .logical_keys()
                .into_iter()
                .filter_map(|key| {
                    let (version, conflict) = match host.logical(&key) {
                        Logical::Settled(page) if !page.clean() || page.conflict => {
                            (page.version, page.conflict)
                        }
                        Logical::Admitted { version, conflict } => (version, conflict),
                        _ => return None,
                    };
                    Some(io::UnsavedPage {
                        path: host.fs.spelling(&key),
                        recovered: recovered.get(&key) == Some(&version),
                        conflict,
                        failing: state.progress.notice(&key).save_error,
                    })
                })
                .collect();
            status
        });
        status.expect("page host driver ended")
    }

    /// `page_drafts_retry` (S3): re-probe down draft I/O in the running
    /// host and finish the cleanup launch skipped. A draft effect in flight
    /// while I/O is down fails at once; this waits for it, 5 s at most. The
    /// error says why draft I/O is still down.
    pub fn drafts_retry(&self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Probes at most every 100 ms, as the caller's pause did.
        let mut next = std::time::Instant::now();
        let step = move |state: &mut State<ProductionIo, SystemClock>| {
            let now = std::time::Instant::now();
            if now < next {
                return Wait::Until(Some(next));
            }
            let retried = state.progress.with_host(|host| host.drafts_retry());
            if state.progress.host.lock_request.is_some() {
                // A plan: it runs again under its path locks.
                return Wait::Until(None);
            }
            match retried {
                Ok(Disposition::Waiting) if now < deadline => {
                    next = (now + std::time::Duration::from_millis(100)).min(deadline);
                    Wait::Until(Some(next))
                }
                Ok(Disposition::Waiting) => Wait::Ready(Err("draft work is still running".into())),
                result => Wait::Ready(result.map(drop)),
            }
        };
        match self.driver.shared.locked_wait(Deadline::Never, step) {
            Ok(result) => result,
            Err(_) => Err("the page host is stopping".into()),
        }
    }
}
