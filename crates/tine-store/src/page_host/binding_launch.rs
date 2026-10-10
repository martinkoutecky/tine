//! The page host's launch (§4, B-Q1): binding a host to a graph, its one
//! launch, which never fails on draft I/O, and crash-recovery status and
//! Retry (S3).
use super::*;

impl PageHost {
    /// Bind a page host to `store`'s graph: drafts under
    /// `app_data/drafts-v2/<graph_id>`, recovered draft keys registered
    /// before launch (§2), then the driver, which owns the index of every
    /// page it holds (§5). `mail` runs on the driver thread. Each launch
    /// draws a fresh session (E101). The error says why no host could
    /// start (another live host holds this graph's draft locks); the open
    /// then fails. Production starts one per graph-window binding
    /// (`src-tauri/src/graph.rs` `load_graph_for_label`, step 3b P2b).
    pub fn start(
        store: &Arc<Store>,
        app_data: &Path,
        graph_id: &str,
        mail: impl FnMut(PageMail) + Send + 'static,
    ) -> Result<Self, String> {
        Self::launch(Launch {
            store: store.clone(),
            app_data: app_data.to_path_buf(),
            graph_id: graph_id.into(),
            mail: Arc::new(Mutex::new(Box::new(mail))),
        })
    }

    /// A host for tests outside this crate (retained writers in
    /// tine-graph-features, the app's slot fixtures) under `app_data`, mail
    /// discarded.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn start_for_tests(store: &Arc<Store>, app_data: &Path) -> Result<Self, String> {
        Self::start(store, app_data, "test-graph", |_| {})
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
        #[cfg(test)]
        let published_kinds = Arc::new(Mutex::new(Vec::new()));
        let bridge = Bridge {
            store: store.clone(),
            mail: launch.mail.clone(),
            #[cfg(test)]
            index_faults: index_faults.clone(),
            #[cfg(test)]
            published_kinds: published_kinds.clone(),
        };
        let driver = Driver::spawn(host, SystemClock::new(), bridge);
        driver
            .shared
            .with_state(|state| state.book.session = next_session());
        // A watcher read of a held page (§5): the driver owes the host an
        // observation of it, taken under its path lock once the page is idle.
        let shared = Arc::downgrade(&driver.shared);
        let forward: crate::watch::Forward = Box::new(move |keys| {
            if let Some(shared) = shared.upgrade() {
                shared.with_state(|state| {
                    for key in keys {
                        state.observe.entry(key).or_default();
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
            #[cfg(test)]
            published_kinds,
        };
        // Recovered pages are held before launch reads them (§5), by the
        // host's keys and spelling table (B1).
        {
            let _writer = store.writer.lock().unwrap();
            store.watch.forward_held(forward, spellings);
            for (key, _) in &recovered {
                store.watch.hold(key.clone());
            }
            store.publish_retired();
            this.driver.shared.with_state(|state| {
                state
                    .book
                    .owned
                    .extend(recovered.into_iter().map(|(k, _)| k))
            });
        }
        // Draft I/O failures never stop a launch (§4, B-Q1): they leave it
        // down, and `draft_status` says why.
        this.locked(|host| host.launch());
        let (alive, recovered) = this.driver.shared.with_state(|state| {
            let host = &state.progress.host;
            let recovered = host
                .pages
                .iter()
                .filter(|(key, _)| keys.contains(*key))
                .map(|(key, page)| (key.clone(), page.version))
                .collect();
            (host.alive, recovered)
        });
        this.recovered = recovered;
        if alive {
            return Ok(this);
        }
        drop(this);
        Err("the page host did not launch".into())
    }

    /// Crash-recovery availability (§4, B-Q1), and the pages whose text is
    /// not on disk yet (SPEC-s2 §4.11, S9), for the `load_graph` reply: a
    /// launch names what it recovered, an adopted host (B-QA) what it still
    /// holds. Bounded by the held pages and admitted input; no I/O.
    pub fn draft_status(&self) -> DraftStatus {
        self.driver.shared.with_state(|state| {
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
                        recovered: self.recovered.get(&key) == Some(&version),
                        conflict,
                        failing: state.progress.notice(&key).save_error,
                    })
                })
                .collect();
            status
        })
    }

    /// `page_drafts_retry` (S3): re-probe down draft I/O in the running
    /// host and finish the cleanup launch skipped. A draft effect in flight
    /// while I/O is down fails at once; this waits for it, 5 s at most. The
    /// error says why draft I/O is still down.
    pub fn drafts_retry(&self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match self.locked(|host| host.drafts_retry()) {
                None => return Err("the page host is stopping".into()),
                Some(Ok(Disposition::Waiting)) if std::time::Instant::now() < deadline => {
                    let shared = &self.driver.shared;
                    let pause = std::time::Duration::from_millis(100);
                    drop(shared.wait(shared.state.lock().unwrap(), pause));
                }
                Some(Ok(Disposition::Waiting)) => return Err("draft work is still running".into()),
                Some(result) => return result.map(drop),
            }
        }
    }
}
