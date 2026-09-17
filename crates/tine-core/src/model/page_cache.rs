//! Graph's page cache: building, warming and streaming the parsed-page cache,
//! repairing it, invalidation, and cache_upsert / cache_remove.

use super::*;

impl Graph {
    fn parse_page_entry_with_permit(
        &self,
        permit: &GraphTextWritePermit,
        entry: PageEntry,
    ) -> PageParseResult {
        let content = match self.graph_text_read_optional_text_with_identity(&permit, &entry.path) {
            Ok(Some((content, _))) => content,
            _ => return Err(entry.rel_path),
        };
        #[cfg(test)]
        self.page_build_test
            .parses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        isolate_page_parse(entry, &self.journal_format, |entry| {
            Some(self.parse_session_page_content(entry, &content))
        })
    }

    fn page_build_entries(&self, permit: &GraphTextWritePermit) -> Result<Vec<PageEntry>, String> {
        #[cfg(test)]
        self.page_build_test
            .enumerations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.graph_text_entries(permit)
            .map_err(|error| format!("graph-text-scope: {error}"))
    }

    /// Enumerate, read and parse every page from disk once (recording unreadable
    /// files as failures). Used by the fast owner of the shared page-build flight;
    /// the paced warmer has the same inventory/parse boundary below.
    ///
    /// The per-file work (read → content_rev → parse → assign uuids) is independent,
    /// so on a large graph we fan it across cores. Result order is irrelevant: the
    /// cache is searched by `(kind, name)`, never by position.
    pub(super) fn load_all_pages_with_permit(
        &self,
        permit: &GraphTextWritePermit,
    ) -> PageCacheBuild {
        let entries = match self.page_build_entries(permit) {
            Ok(entries) => entries,
            Err(failure) => {
                return PageCacheBuild {
                    pages: Vec::new(),
                    failures: vec![failure],
                };
            }
        };
        let entry_count = entries.len();
        let workers = page_cache_worker_count();
        // Small graphs (or a single core): serial — the parse is fast and thread
        // spawn isn't worth it. Big graphs: split across `workers` threads.
        if workers <= 1 || entries.len() < 64 {
            let mut built = PageCacheBuild::with_capacity(entries.len());
            for entry in entries {
                built.collect(self.parse_page_entry_with_permit(permit, entry));
            }
            return built;
        }
        let per = (entries.len() + workers - 1) / workers;
        // Drain into owned contiguous chunks (no clone of PageEntry).
        let mut chunks: Vec<Vec<PageEntry>> = Vec::with_capacity(workers);
        let mut it = entries.into_iter();
        loop {
            let chunk: Vec<PageEntry> = it.by_ref().take(per).collect();
            if chunk.is_empty() {
                break;
            }
            chunks.push(chunk);
        }
        std::thread::scope(|s| {
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    s.spawn(move || {
                        let mut built = PageCacheBuild::with_capacity(chunk.len());
                        for entry in chunk {
                            built.collect(self.parse_page_entry_with_permit(permit, entry));
                        }
                        built
                    })
                })
                .collect();
            let mut built = PageCacheBuild::with_capacity(entry_count);
            for (worker, handle) in handles.into_iter().enumerate() {
                match handle.join() {
                    Ok(shard) => built.append(shard),
                    Err(_) => eprintln!(
                        "Tine search index worker {worker} panicked after per-page isolation; its shard was not indexed"
                    ),
                }
            }
            built
        })
    }

    pub(super) fn claim_page_build(
        &self,
        expected_generation: u64,
    ) -> (Arc<PageBuildFlight>, bool) {
        let mut active = self.page_build_flight.lock().unwrap();
        if let Some(flight) = active.as_ref() {
            #[cfg(test)]
            {
                let mut joined = self.page_build_test.joined.lock().unwrap();
                *joined += 1;
                self.page_build_test.joined_changed.notify_all();
            }
            return (Arc::clone(flight), false);
        }
        // Close the gap between a caller's earlier cold-cache observation and
        // flight publication. Holding the flight registry makes this decision
        // atomic with a finishing owner clearing its active flight. The nested
        // order is flight -> cache; installation holds only cache and releases it
        // before `finish_page_build` takes flight, so there is no inverse nesting.
        let completed = {
            let cache = self.cache.read().unwrap();
            let current_generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            if current_generation != expected_generation {
                Some(PageBuildOutcome::GenerationDrift)
            } else if cache.is_some() {
                Some(PageBuildOutcome::AlreadyAvailable)
            } else {
                None
            }
        };
        if let Some(outcome) = completed {
            let flight = Arc::new(PageBuildFlight::new(expected_generation));
            flight.complete(outcome);
            return (flight, false);
        }
        let flight = Arc::new(PageBuildFlight::new(expected_generation));
        *active = Some(Arc::clone(&flight));
        drop(active);
        #[cfg(test)]
        if let Some(pause) = self.page_build_test.owner_pause.lock().unwrap().clone() {
            pause.reached.wait();
            pause.release.wait();
        }
        (flight, true)
    }

    fn finish_page_build(&self, flight: &Arc<PageBuildFlight>, outcome: PageBuildOutcome) {
        flight.complete(outcome);
        let mut active = self.page_build_flight.lock().unwrap();
        if active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, flight))
        {
            *active = None;
        }
    }

    pub(super) fn repair_page_cache_once(&self, permit: &GraphTextWritePermit) -> PageBuildOutcome {
        let expected_generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let (flight, owner) = self.claim_page_build(expected_generation);
        if !owner {
            return flight.wait();
        }
        let built = self.load_all_pages_with_permit(permit);
        let outcome = PageBuildOutcome::from(self.install_built(flight.expected_generation, built));
        self.finish_page_build(&flight, outcome);
        outcome
    }

    /// Install a freshly-built whole-graph snapshot atomically: the parsed pages
    /// into the cache, their on-disk revs into `disk_revs`. Cache set BEFORE
    /// disk_revs so a reader never observes a fresh rev paired with a stale cache.
    pub(super) fn install_built(
        &self,
        expected_generation: u64,
        built: PageCacheBuild,
    ) -> PageCacheInstallOutcome {
        #[cfg(test)]
        if self
            .page_build_test
            .drift_before_install
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.cache_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        let PageCacheBuild {
            pages: built,
            mut failures,
        } = built;
        failures.sort();
        failures.dedup();
        let revs: std::collections::HashMap<PathBuf, String> = built
            .iter()
            .map(|(e, _, r)| (e.path.clone(), r.clone()))
            .collect();
        let pages: Vec<(PageEntry, Arc<Document>)> = built
            .into_iter()
            .map(|(e, d, _)| (e, Arc::new(d)))
            .collect();
        let index = build_page_cache_index(&pages);
        let effective_index = Arc::new(build_effective_identity_index(
            expected_generation,
            &pages,
            failures.clone(),
        ));
        let page_list = pages.iter().map(|(entry, _)| entry.clone()).collect();
        // Publish cache + revs atomically under the cache lock (cache → disk_revs
        // order), but only at the exact generation the owner parsed. A cache that
        // another publisher already supplied is likewise never overwritten.
        let mut guard = self.cache.write().unwrap();
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != expected_generation {
            return PageCacheInstallOutcome::GenerationDrift;
        }
        if guard.is_some() {
            return PageCacheInstallOutcome::AlreadyAvailable;
        }
        let pages = Arc::new(pages);
        *guard = Some(Arc::clone(&pages));
        *self.cache_index.write().unwrap() = Some(index);
        *self.disk_revs.write().unwrap() = revs.clone();
        *self.effective_identity_index.write().unwrap() = Some(effective_index);
        *self.page_index_failures.write().unwrap() = failures;
        *self.page_list_cache.write().unwrap() = Some((expected_generation, page_list));
        #[cfg(test)]
        self.page_build_test
            .installs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        self.direct_projection_enqueue_full(expected_generation, pages, Arc::new(revs), false);
        PageCacheInstallOutcome::Installed
    }

    /// Run `f` over every parsed page, building the cache on first use.
    ///
    /// `f` scans a consistent snapshot: a concurrent save/delete may or may not be
    /// visible depending on whether it published before this method cloned the
    /// snapshot Arc, but the scan never sees torn or partially-mutated cache
    /// contents. The cache read lock is held only while cloning the Arc; mutations
    /// use copy-on-write under `cache.write()` when a scan still holds an older
    /// snapshot.
    #[track_caller]
    pub fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Arc<Document>)]) -> T) -> T {
        loop {
            let snapshot = {
                let guard = self.cache.read().unwrap();
                guard.as_ref().map(Arc::clone)
            };
            if let Some(snapshot) = snapshot {
                return f(snapshot.as_slice());
            }
            // Admission precedes flight ownership. Query callers retain their
            // historical retry semantics, while Direct creation uses the bounded
            // `repair_page_cache_once` entry point instead.
            let permit = match self.admit_retained_graph_text_writer() {
                Ok(permit) => permit,
                Err(_) => return f(&[]),
            };
            let expected_generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            let (flight, owner) = self.claim_page_build(expected_generation);
            if owner {
                if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
                    eprintln!(
                        "DIAG543 FULL_PARSE with_pages caller={} generation={expected_generation}",
                        std::panic::Location::caller()
                    );
                }
                let built = self.load_all_pages_with_permit(&permit);
                let outcome =
                    PageBuildOutcome::from(self.install_built(flight.expected_generation, built));
                self.finish_page_build(&flight, outcome);
            } else {
                let _ = flight.wait();
            }
        }
    }

    /// Eagerly build the page cache plus graph-open derived maps (call once after
    /// opening, off the hot path).
    pub fn warm_cache(&self) {
        let _ = self.warm_cache_cancellable(|| false);
    }

    /// Build graph-open caches while allowing a revoked window binding to stop
    /// between files and derived-map phases. Returns false when cancelled.
    pub fn warm_cache_cancellable(&self, cancelled: impl Fn() -> bool) -> bool {
        // R6: validate the projection from file bytes first. An unchanged graph
        // is READY after that with nothing parsed and nothing retained; a
        // changed or cold one streams only its replacement pages. The full
        // parse below is the fallback when no projection can own readiness.
        if !self.warm_projection_cancellable(&cancelled)
            && (!self.warm_page_cache_cancellable(&cancelled) || cancelled())
        {
            return false;
        }
        if cancelled() {
            return false;
        }
        // Warm the derived maps the frontend fetches right after `warm-cache-done`
        // (aliases + block-ref counts), so those fetches are pure cache hits.
        let _ = self.page_aliases();
        if cancelled() {
            return false;
        }
        if self.block_ref_counts().is_err() {
            return false;
        }
        !cancelled()
    }

    /// R6 warm validation: publish Direct Files projection readiness from the
    /// walk inventory and each page's exact content revision, parsing nothing
    /// unless the worker names replacements. `true` means the projection owns
    /// readiness at the generation this warm observed (or a full snapshot
    /// superseded it); `false` means the caller must fall back to the full
    /// parse — no projection, a lease held elsewhere, a failed worker,
    /// cancellation, or a mutation that raced the warm (generation drift).
    pub(super) fn warm_projection_cancellable(&self, cancelled: &impl Fn() -> bool) -> bool {
        let diagnostic = std::env::var_os("TINE_DIAGNOSE_543").is_some();
        if diagnostic {
            eprintln!(
                "DIAG543 WARM begin thread={:?}",
                std::thread::current().id()
            );
        }
        if cancelled() {
            return false;
        }
        if self.cache.read().unwrap().is_some() {
            // The full-snapshot path owns readiness while a parsed cache exists.
            return false;
        }
        let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        else {
            return false;
        };
        let Ok(permit) = self.admit_retained_graph_text_writer() else {
            return false;
        };
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let Ok(entries) = self.page_build_entries(&permit) else {
            return false;
        };
        if diagnostic {
            eprintln!(
                "DIAG543 WARM inventory={} thread={:?}",
                entries.len(),
                std::thread::current().id()
            );
        }
        let mut sources = Vec::with_capacity(entries.len());
        let mut failures = Vec::new();
        for (i, entry) in entries.into_iter().enumerate() {
            if cancelled() {
                return false;
            }
            match self.graph_text_read_optional_text_with_identity(&permit, &entry.path) {
                Ok(Some((content, _))) => {
                    let revision = content_rev(&content);
                    sources.push((entry, revision));
                }
                _ => failures.push(entry.rel_path),
            }
            if i % 24 == 23 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
            if diagnostic {
                eprintln!(
                    "DIAG543 WARM drift captured={generation} current={} thread={:?}",
                    self.cache_generation(),
                    std::thread::current().id()
                );
            }
            return false;
        }
        let parse_config = Arc::new(self.config.parse_config());
        if diagnostic {
            eprintln!(
                "DIAG543 WARM sources={} thread={:?}",
                sources.len(),
                std::thread::current().id()
            );
        }
        if !projection.enqueue_warm(generation, sources, Arc::clone(&parse_config)) {
            if diagnostic {
                eprintln!(
                    "DIAG543 WARM enqueue refused thread={:?}",
                    std::thread::current().id()
                );
            }
            return false;
        }
        if diagnostic {
            eprintln!(
                "DIAG543 WARM waiting outcome thread={:?}",
                std::thread::current().id()
            );
        }
        let outcome = projection.wait_warm_outcome();
        if diagnostic {
            eprintln!(
                "DIAG543 WARM received outcome thread={:?}",
                std::thread::current().id()
            );
        }
        match outcome {
            crate::direct_projection::WarmOutcome::Clean => {
                self.publish_page_index_failures(generation, failures);
                true
            }
            crate::direct_projection::WarmOutcome::Superseded => true,
            crate::direct_projection::WarmOutcome::Failed => false,
            crate::direct_projection::WarmOutcome::Replacements(pages) => self
                .stream_warm_replacements(
                    &projection,
                    &permit,
                    generation,
                    pages,
                    parse_config,
                    failures,
                    cancelled,
                ),
        }
    }

    /// R6 warm stream: parse ONLY the pages the worker named, in bounded
    /// batches, retaining nothing beyond the batch in flight. Every batch is
    /// admitted against the queue's high-water mark, so a cold open never pins
    /// the parsed graph. Any refusal (drift, supersession, worker failure,
    /// cancellation) abandons the stream; a superseding full snapshot owns
    /// readiness instead and nothing is owed.
    #[allow(clippy::too_many_arguments)]
    fn stream_warm_replacements(
        &self,
        projection: &crate::direct_projection::DirectProjection,
        permit: &GraphTextWritePermit,
        generation: u64,
        pages: Vec<PageEntry>,
        parse_config: Arc<crate::config::ParseConfig>,
        mut failures: Vec<String>,
        cancelled: &impl Fn() -> bool,
    ) -> bool {
        use crate::direct_projection::WarmStreamItem;
        const BATCH: usize = 16;
        let total = pages.len();
        let mut batch = Vec::with_capacity(BATCH.min(total));
        for (i, entry) in pages.into_iter().enumerate() {
            if cancelled() {
                return projection.abandon_warm_stream(generation);
            }
            let fallback = entry.clone();
            let item = match self.graph_text_read_optional_text_with_identity(permit, &entry.path) {
                Ok(Some((content, _))) => {
                    #[cfg(test)]
                    self.page_build_test
                        .warm_stream_parses
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    match isolate_page_parse(entry, &self.journal_format, |entry| {
                        Some(parse_page_content(entry, &content))
                    }) {
                        Ok(Some((effective, mut document, revision))) => {
                            let identity = if self.restore_session_page_ids(
                                &effective,
                                &revision,
                                &mut document,
                            ) {
                                crate::direct_projection::DeltaIdentity::Live
                            } else {
                                crate::direct_projection::DeltaIdentity::Structural
                            };
                            WarmStreamItem::Replace {
                                entry: effective,
                                document: Arc::new(document),
                                revision,
                                identity,
                            }
                        }
                        Ok(None) | Err(_) => {
                            failures.push(fallback.rel_path.clone());
                            WarmStreamItem::Delete { entry: fallback }
                        }
                    }
                }
                _ => {
                    failures.push(fallback.rel_path.clone());
                    WarmStreamItem::Delete { entry: fallback }
                }
            };
            batch.push(item);
            if batch.len() == BATCH || i + 1 == total {
                if !projection.warm_stream_admit(generation, batch.len())
                    || self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation
                    || !projection.enqueue_warm_stream(
                        generation,
                        std::mem::take(&mut batch),
                        Arc::clone(&parse_config),
                    )
                {
                    return projection.abandon_warm_stream(generation);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation
            || !projection.finish_warm_stream(generation)
        {
            return projection.abandon_warm_stream(generation);
        }
        self.publish_page_index_failures(generation, failures);
        true
    }

    fn publish_page_index_failures(&self, generation: u64, mut failures: Vec<String>) {
        failures.sort();
        failures.dedup();
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation {
            *self.page_index_failures.write().unwrap() = failures;
        }
    }

    pub(super) fn warm_page_cache_cancellable(&self, cancelled: &impl Fn() -> bool) -> bool {
        if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
            eprintln!(
                "DIAG543 FULL_PARSE warm_page_cache generation={}",
                self.cache_generation()
            );
        }
        if cancelled() {
            return false;
        }
        if self.cache.read().unwrap().is_some() {
            return true; // already built (e.g. by a query) — nothing to warm
        }
        let permit = match self.admit_retained_graph_text_writer() {
            Ok(permit) => permit,
            Err(_) => return false,
        };
        let expected_generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let (flight, owner) = self.claim_page_build(expected_generation);
        if !owner {
            return flight.wait().installed() && !cancelled();
        }
        if cancelled() {
            self.finish_page_build(&flight, PageBuildOutcome::Cancelled);
            return false;
        }
        // Build PACED without holding the flight mutex during the parse: on a
        // thermally throttled laptop the warm would otherwise peg a core in one
        // burst right after launch, competing with first scrolling/typing/the
        // first agenda query. Joiners wait for this same generation instead of
        // duplicating its parse.
        let entries = match self.page_build_entries(&permit) {
            Ok(entries) => entries,
            Err(failure) => {
                let built = PageCacheBuild {
                    pages: Vec::new(),
                    failures: vec![failure],
                };
                let outcome =
                    PageBuildOutcome::from(self.install_built(flight.expected_generation, built));
                self.finish_page_build(&flight, outcome);
                return outcome.installed() && !cancelled();
            }
        };
        let mut built = PageCacheBuild::with_capacity(entries.len());
        let mut baselines: Vec<(PathBuf, ContentDigest, String)> =
            Vec::with_capacity(entries.len());
        for (i, e) in entries.into_iter().enumerate() {
            if cancelled() {
                self.finish_page_build(&flight, PageBuildOutcome::Cancelled);
                return false;
            }
            match self.graph_text_read_optional_text_with_identity(&permit, &e.path) {
                Ok(Some((content, identity))) => {
                    let path = e.path.clone();
                    let revision = content_rev(&content);
                    #[cfg(test)]
                    self.page_build_test
                        .parses
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let indexed =
                        built.collect(isolate_page_parse(e, &self.journal_format, |entry| {
                            Some(self.parse_session_page_content(entry, &content))
                        }));
                    if indexed {
                        baselines.push((path, identity, revision));
                    }
                }
                _ => built.failures.push(e.rel_path),
            }
            if i % 24 == 23 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        #[cfg(test)]
        if self
            .page_build_test
            .force_warm_failure
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.finish_page_build(&flight, PageBuildOutcome::Failed);
            return false;
        }
        for (path, identity, revision) in &baselines {
            match self.graph_text_read_optional_text_with_identity(&permit, path) {
                Ok(Some((content, current_identity)))
                    if current_identity == *identity && content_rev(&content) == *revision => {}
                _ => {
                    self.finish_page_build(&flight, PageBuildOutcome::Failed);
                    return false;
                }
            }
        }
        if cancelled() {
            self.finish_page_build(&flight, PageBuildOutcome::Cancelled);
            return false;
        }
        let outcome = PageBuildOutcome::from(self.install_built(flight.expected_generation, built));
        self.finish_page_build(&flight, outcome);
        outcome.installed() && !cancelled()
    }

    /// Discard the cache; it rebuilds on the next whole-graph query. Use when an
    /// external change may have touched many files.
    pub fn invalidate_cache(&self) {
        self.invalidate_guarded_graph_text_identity(
            "broad external cache invalidation has no exact path generation",
        );
        self.invalidate_cache_after_tine_mutation();
    }

    #[cfg(test)]
    pub(crate) fn page_build_parses_test(&self) -> usize {
        self.page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn warm_stream_parses_test(&self) -> usize {
        self.page_build_test
            .warm_stream_parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn on_demand_parses_test(&self) -> usize {
        self.page_build_test
            .on_demand_parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn has_parsed_cache_test(&self) -> bool {
        self.cache.read().unwrap().is_some()
    }

    pub(super) fn invalidate_cache_after_tine_mutation(&self) {
        self.direct_projection_mark_stale();
        // Compatible IDs are owned by session_page_ids, independently of the
        // parsed cache. Reconciliation invalidates incompatible source revisions.
        let mut guard = self.cache.write().unwrap();
        *guard = None;
        self.page_index_failures.write().unwrap().clear();
        *self.cache_index.write().unwrap() = None;
        *self.effective_identity_index.write().unwrap() = None;
        self.disk_revs.write().unwrap().clear(); // under the cache lock (cache → disk_revs)
                                                 // Bump the generation AFTER discarding the cache (under the cache lock), so
                                                 // a reader that loads the new gen then reads the cache sees None (and
                                                 // rebuilds from disk) rather than the stale pre-invalidation content — same
                                                 // gen-after-content ordering as cache_upsert.
        self.cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        drop(guard);
    }

    /// Publish one page delta after a write or external reconciliation, without
    /// building an absent parsed cache. `disk_rev` is `content_rev` of the
    /// exact on-disk bytes `doc` was produced from (the freshness key — see
    /// `disk_revs`).
    pub(super) fn cache_upsert(&self, entry: PageEntry, doc: Document, disk_rev: String) {
        // An exact one-page publication must not make an ordinary warm save pay
        // for graph-wide derived-cache maintenance. Content-only updates retain
        // their exact candidate index delta and conservatively clear dependent
        // query caches; an identity-changing update still rebuilds the identity
        // index before it can be used for name-only admission.
        self.cache_upsert_inner(entry, doc, disk_rev, true);
    }

    fn cache_upsert_inner(
        &self,
        entry: PageEntry,
        mut doc: Document,
        disk_rev: String,
        bounded_foreground: bool,
    ) {
        // Fill runtime ids for any block that lacks one (e.g. PDF-highlight writes)
        // from this physical owner. Blocks saved from the frontend already carry
        // live ids, which are deliberately kept through the in-memory save path.
        assign_doc_runtime_ids(&mut doc.roots, &entry.rel_path);
        // Only the alias map needs dropping when an `alias::` was added/changed/
        // removed — invalidating on every save would make a normal edit an O(P)
        // alias rescan on the next navigation.
        let new_aliases = crate::query::document_aliases(&doc);
        let mut alias_touched = !new_aliases.is_empty();
        let path_key = entry.path.clone();
        let doc = Arc::new(doc);
        // Keep the new content + identity for the scoped derived-cache pass below
        // (the original is moved into the cache slot; this clone is a refcount bump).
        let evict_doc = Arc::clone(&doc);
        let evict_entry = entry.clone();
        let projection_revision = disk_rev.clone();
        let mut previous_doc: Option<Arc<Document>> = None;
        let mut is_new_page = false;
        let mut identity_changed = false;
        let mut guard = self.cache.write().unwrap();
        let mut failures_guard = self.page_index_failures.write().unwrap();
        self.session_page_ids.write().unwrap().insert(
            evict_entry.path.clone(),
            SessionPageIds::capture(
                &projection_revision,
                self.config.parse_config().digest(),
                &evict_doc,
            ),
        );
        let mut resulting_failures = failures_guard.clone();
        resulting_failures.retain(|failure| failure != &evict_entry.rel_path);
        let failures_changed = resulting_failures != *failures_guard;
        let cache_built = guard.is_some();
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            match self.cached_page_index_for_path(pages, &entry.path) {
                Some(i) => {
                    let slot = &mut pages[i];
                    alias_touched = new_aliases != crate::query::document_aliases(&slot.1);
                    previous_doc = Some(Arc::clone(&slot.1));
                    identity_changed = slot.0.kind != entry.kind
                        || !crate::refs::same_page(&slot.0.name, &entry.name)
                        || slot.0.date_key != entry.date_key;
                    alias_touched |= identity_changed;
                    slot.0 = entry;
                    slot.1 = doc;
                    if identity_changed {
                        *self.cache_index.write().unwrap() = Some(build_page_cache_index(pages));
                    }
                }
                None => {
                    is_new_page = true;
                    let name_key = page_cache_key(entry.kind, &entry.name);
                    pages.push((entry, doc));
                    if let Some(index) = self.cache_index.write().unwrap().as_mut() {
                        let slot = pages.len() - 1;
                        index.by_path.insert(path_key.clone(), slot);
                        // Exact-path additions must never repoint the stable
                        // logical duplicate winner.
                        index.by_name.entry(name_key).or_insert(slot);
                    }
                }
            }
            // Update disk_revs WHILE STILL HOLDING the cache write lock, so the
            // cached doc and its freshness rev are published atomically and can
            // never diverge across concurrent same-page writers (e.g. an editor
            // save racing a PDF write_highlights on an hls__ page). If they could
            // diverge, the sync_file_content fast-path could match disk against a
            // rev that isn't the cached doc's and serve a stale doc. Lock order is
            // always cache → disk_revs; readers never hold disk_revs while taking
            // the cache lock, so this nesting can't deadlock. Sets only when the
            // page is actually cached (preserves "entry exists IFF cached").
            self.disk_revs.write().unwrap().insert(path_key, disk_rev);
        }
        // Bump cache_gen AFTER publishing the new content (and disk_revs), still
        // under the cache write lock. A reader loads cache_gen (Acquire) then takes
        // the cache read lock; because the bump (Release) happens-after the slot
        // write and before the lock is dropped, observing the new gen guarantees
        // the new doc is visible. So any derived result computed at gen G reflects
        // every edit whose gen is <= G — it can never be a stale whole-graph scan
        // that reads the OLD doc yet gets tagged (and served) at the fresh gen.
        // (Bumping FIRST left a window where the gen was new but the doc still old.)
        // The bump is unconditional — even on a cold cache (no slot to update) — so
        // a concurrent lock-free with_pages build still detects the race and retries.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if bounded_foreground
            && cache_built
            && !identity_changed
            && !is_new_page
            && !failures_changed
        {
            // Exact content-only publication does not alter effective identity.
            // Advance the shared generation tag in O(1): creation requires warm
            // parsed semantic evidence and must not rebuild it merely because
            // unrelated bytes changed.
            let mut identity_guard = self.effective_identity_index.write().unwrap();
            let advanced = identity_guard
                .as_ref()
                .is_some_and(|index| index.retag_generation(newgen - 1, newgen));
            if !advanced {
                *identity_guard = None;
            }
        } else if let Some(pages) = guard.as_ref() {
            *self.effective_identity_index.write().unwrap() = Some(Arc::new(
                build_effective_identity_index(newgen, pages, resulting_failures.clone()),
            ));
        } else {
            self.advance_effective_identity_after_upsert(
                newgen,
                &evict_entry,
                resulting_failures.clone(),
            );
        }
        let page_inventory_complete = resulting_failures.is_empty();
        *failures_guard = resulting_failures;
        drop(failures_guard);
        drop(guard);
        // Same treatment for the physical page inventory, and for the same
        // reason as other generation-bound derived inventories. `list_pages` is keyed on raw
        // `cache_gen` equality, and its ONLY way to rebuild is to walk the graph
        // text scope, re-read every file from disk and re-parse each one.
        // Measured at ~35 µs/file, dead linear from 506 to 8,006 pages: 243 ms on
        // a real 5,225-file graph, and `find_entry` rebuilds straight from it. So
        // every ordinary save made the next navigation or `[[` autocomplete stall
        // for a quarter of a second, repeating after every typing pause.
        // (Direct Files perf audit, 2026-08-09, F1.)
        //
        // The inventory is a function of the file set plus each file's parsed
        // identity, so it survives exactly the conditions the effective-identity
        // index above already tests: an existing page whose content changed but
        // whose identity, page-ness and failure set did not. Deliberately NOT
        // re-tagged more broadly — a `title::` edit IS an identity change and
        // does move an entry. `referenced_page_names` is likewise left to
        // rebuild: it is genuinely derived from block content, so a content edit
        // really can change it, and it recomputes from the warm in-memory cache
        // rather than from disk.
        //
        // The `generation + 1 == newgen` guard is load-bearing, and its absence
        // was a real regression: re-tagging is only sound for a list that was
        // CURRENT as of the previous generation. A list cached before an
        // unrelated page was created is stale at an older generation, and
        // stamping it as current republishes that staleness permanently —
        // `list_pages` is keyed on generation equality, so it never rebuilds and
        // the missing page becomes unloadable.
        let mut page_list_advanced = false;
        if cache_built && !failures_changed {
            if let Some((generation, entries)) = self.page_list_cache.write().unwrap().as_mut() {
                if *generation + 1 == newgen {
                    if let Some(existing) = entries
                        .iter_mut()
                        .find(|entry| entry.path == evict_entry.path)
                    {
                        *existing = evict_entry.clone();
                    } else {
                        entries.push(evict_entry.clone());
                    }
                    *generation = newgen;
                    page_list_advanced = true;
                }
            }
        }
        // Creation paths and watcher reconciliation may intentionally have no
        // prior list memo to retag. The warm parsed cache already contains the
        // exact newly parsed entry and every survivor, so rebuild the in-memory
        // inventory from it rather than reopening and reparsing the graph.
        if cache_built && !page_list_advanced && page_inventory_complete {
            self.publish_warm_page_inventory(newgen);
        }
        // Scoped query/backlink invalidation (#52): a content edit to one page
        // can't change a derived result the page doesn't participate in, so keep
        // those (advancing their generation) and recompute only the entries this
        // page is in or now matches. An alias change, a new page, or a cold cache
        // has graph-wide effects → drop everything. Guarded by the differential
        // fuzz oracle in tests/derived_cache_fuzz.rs.
        let scoped = cache_built && !alias_touched && !is_new_page && !identity_changed;
        self.scope_derived_invalidation(
            &evict_entry,
            previous_doc.as_deref(),
            &evict_doc,
            newgen,
            scoped,
        );
        // R6: the projection receives the delta whether or not a parsed cache
        // exists. A warm session has no cache at all, so gating the delta on
        // it would leave every save unprojected until some whole-graph
        // consumer happened to build one. Readiness still needs this
        // session's inventory validated first (the projection's own rule).
        self.direct_projection_enqueue_replace(newgen, evict_entry, evict_doc, projection_revision);
    }

    /// See `cache_upsert`. When `scoped`, evict only derived entries the edited
    /// page (`entry`, `doc`) participates in and re-tag the survivors to `newgen`;
    /// otherwise drop the whole derived cache.
    fn scope_derived_invalidation(
        &self,
        entry: &PageEntry,
        previous_doc: Option<&Document>,
        doc: &Document,
        newgen: u64,
        scoped: bool,
    ) {
        // Most edits have no derived query state to invalidate. Avoid building
        // graph-wide alias and real-page inputs merely to rediscover that both
        // memo families are absent. A memo installed after this observation is
        // computed against the already-published cache generation and content.
        if self.derived_cache.read().unwrap().is_none() {
            return;
        }
        // Resolve aliases BEFORE taking the derived lock (page_aliases may take the
        // cache lock); never hold derived while taking cache.
        let (aliases, real_pages) = if scoped {
            (self.page_aliases(), crate::query::real_page_names(self))
        } else {
            (Vec::new(), crate::query::RealPageNames::new())
        };
        let today = crate::date::JournalDate::today().ordinal_key();
        // Hold the derived write lock across the WHOLE prune+re-tag. This is
        // deliberately atomic: the keep/evict test (page_affects_*) is re-evaluated
        // against whatever entry is CURRENTLY in the map, so a result a concurrent
        // query inserted (possibly from an older page-doc) is re-judged and evicted
        // if this edit affects it — never kept on pointer identity. Combined with
        // the gen-after-content bump (cache_upsert), an entry that survives the
        // prune is provably unaffected by this edit and consistent at `newgen`.
        // (An earlier version evaluated off the lock and kept entries by Arc
        // ptr_eq; that could bless a stale concurrent recompute — reverted.)
        let pname = &entry.name;
        {
            let mut g = self.derived_cache.write().unwrap();
            let Some(dc) = g.as_mut() else {
                drop(g);
                return;
            };
            if !scoped || dc.today != today {
                *g = None; // full invalidate (alias/page-set/cold-cache, or day rollover)
                drop(g);
                return;
            }
            let mut removed_bytes = 0usize;
            dc.results.retain(|key, (result, result_bytes)| {
                // Evict iff this page is already in the result OR matches the key's
                // predicate in either the old or new page; keep (still correct)
                // otherwise. Comparing both parsed documents makes omitted overflow
                // matches visible without retaining a graph-sized membership set.
                if result
                    .result
                    .groups
                    .iter()
                    .any(|grp| crate::refs::same_page(&grp.page, pname))
                {
                    removed_bytes = removed_bytes.saturating_add(*result_bytes);
                    return false;
                }
                let page_affects = |candidate: &Document| match key.split_once('\0') {
                    Some(("b", target)) => crate::query::page_affects_backlinks(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        candidate,
                    ),
                    Some(("u", target)) => crate::query::page_affects_unlinked(
                        &real_pages,
                        &aliases,
                        target,
                        entry,
                        candidate,
                    ),
                    Some(("br", uuid)) => {
                        crate::query::page_affects_block_referrers(uuid, candidate)
                    }
                    Some(("B", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|target| {
                        crate::query::page_affects_backlinks(
                            &real_pages,
                            &aliases,
                            target,
                            entry,
                            candidate,
                        )
                    }),
                    Some(("U", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|target| {
                        crate::query::page_affects_unlinked(
                            &real_pages,
                            &aliases,
                            target,
                            entry,
                            candidate,
                        )
                    }),
                    Some(("R", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|uuid| {
                        crate::query::page_affects_block_referrers(uuid, candidate)
                    }),
                    _ => true, // unknown key shape → evict to stay safe
                };
                let affects = page_affects(doc) || previous_doc.is_some_and(&page_affects);
                if affects {
                    removed_bytes = removed_bytes.saturating_add(*result_bytes);
                }
                !affects
            });
            dc.bytes = dc.bytes.saturating_sub(removed_bytes);
            dc.lru.retain(|key| dc.results.contains_key(key));
            dc.gen = newgen; // survivors are valid for the post-bump generation
        }
    }

    /// Drop one page from the cache after deleting its file.
    /// `known` is the exact entry the caller just removed from disk, so the
    /// projection delete can be named in a session that holds no parsed cache
    /// (R6). Without it and without a cache, the current page-list memo is the
    /// remaining inventory; failing both, the projection is only marked stale.
    pub(super) fn cache_remove(&self, name: &str, kind: PageKind, known: Option<PageEntry>) {
        // A page delete is a page-set change that can affect every backlink
        // and reference result, so drop the whole derived-reference cache.
        *self.derived_cache.write().unwrap() = None;
        let mut guard = self.cache.write().unwrap();
        let mut removed_entries = Vec::new();
        let cache_built = guard.is_some();
        if !cache_built {
            match known {
                Some(entry) => removed_entries.push(entry),
                None => {
                    let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
                    if let Some((memo_generation, entries)) =
                        self.page_list_cache.read().unwrap().as_ref()
                    {
                        if *memo_generation == generation {
                            removed_entries.extend(
                                entries
                                    .iter()
                                    .filter(|entry| {
                                        entry.kind == kind
                                            && crate::refs::same_page(&entry.name, name)
                                    })
                                    .cloned(),
                            );
                        }
                    }
                }
            }
        }
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            removed_entries.extend(
                pages
                    .iter()
                    .filter(|(entry, _)| {
                        entry.kind == kind && crate::refs::same_page(&entry.name, name)
                    })
                    .map(|(entry, _)| entry.clone()),
            );
            let removed_paths = pages
                .iter()
                .filter(|(e, _)| e.kind == kind && crate::refs::same_page(&e.name, name))
                .map(|(e, _)| e.path.clone())
                .collect::<Vec<_>>();
            pages.retain(|(e, _)| !(e.kind == kind && crate::refs::same_page(&e.name, name)));
            // Drop all exact revisions removed by this ambiguity-validated logical
            // delete under the cache lock (same cache → disk_revs order as
            // cache_upsert) so the two never diverge.
            let mut revs = self.disk_revs.write().unwrap();
            for path in removed_paths {
                revs.remove(&path);
            }
        }
        *self.cache_index.write().unwrap() =
            guard.as_ref().map(|pages| build_page_cache_index(pages));
        // Bump AFTER the removal is published (under the cache lock), so a reader
        // that loads the new gen is guaranteed to see the page gone — see the
        // gen-after-content note in cache_upsert.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if let Some(pages) = guard.as_ref() {
            *self.effective_identity_index.write().unwrap() =
                Some(Arc::new(build_effective_identity_index(
                    newgen,
                    pages,
                    self.page_index_failures.read().unwrap().clone(),
                )));
        } else {
            *self.effective_identity_index.write().unwrap() = None;
        }
        drop(guard);
        let mut page_list_advanced = false;
        if let Some((generation, entries)) = self.page_list_cache.write().unwrap().as_mut() {
            if *generation + 1 == newgen {
                entries.retain(|entry| {
                    !removed_entries
                        .iter()
                        .any(|removed| removed.path == entry.path)
                });
                *generation = newgen;
                page_list_advanced = true;
            }
        }
        if !page_list_advanced && self.page_index_failures.read().unwrap().is_empty() {
            self.publish_warm_page_inventory(newgen);
        }
        if removed_entries.is_empty() && !cache_built {
            // The deleted file could not be named; the next warm validation or
            // full snapshot re-derives the inventory.
            self.direct_projection_mark_stale();
        }
        for entry in removed_entries {
            self.direct_projection_enqueue_delete(newgen, entry);
        }
    }

    /// Drop one physical page from the cache after its file disappears. Unlike
    /// `cache_remove`, this preserves same-name siblings and rebuilds the logical
    /// first-wins index from the surviving entries.
    pub(super) fn cache_remove_path(&self, entry: &PageEntry) {
        // A page delete is a page-set change (affects namespaces, exists-by-ref,
        // every backlink/query) — drop the whole derived cache.
        *self.derived_cache.write().unwrap() = None;
        let mut guard = self.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            let pages = Arc::make_mut(pages);
            if let Some(i) = self.cached_page_index_for_path(pages, &entry.path) {
                pages.remove(i);
                // Drop the rev under the cache lock (same cache → disk_revs order
                // as cache_upsert) so the two never diverge.
                self.disk_revs.write().unwrap().remove(&entry.path);
            }
            *self.cache_index.write().unwrap() = Some(build_page_cache_index(pages));
        }
        // Bump AFTER the removal is published (under the cache lock), so a reader
        // that loads the new gen is guaranteed to see the page gone — see the
        // gen-after-content note in cache_upsert.
        let newgen = self
            .cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release)
            + 1;
        if let Some(pages) = guard.as_ref() {
            *self.effective_identity_index.write().unwrap() =
                Some(Arc::new(build_effective_identity_index(
                    newgen,
                    pages,
                    self.page_index_failures.read().unwrap().clone(),
                )));
        } else {
            *self.effective_identity_index.write().unwrap() = None;
        }
        drop(guard);
        let mut page_list_advanced = false;
        if let Some((generation, entries)) = self.page_list_cache.write().unwrap().as_mut() {
            if *generation + 1 == newgen {
                entries.retain(|candidate| candidate.path != entry.path);
                *generation = newgen;
                page_list_advanced = true;
            }
        }
        if !page_list_advanced && self.page_index_failures.read().unwrap().is_empty() {
            self.publish_warm_page_inventory(newgen);
        }
        self.direct_projection_enqueue_delete(newgen, entry.clone());
    }
}
