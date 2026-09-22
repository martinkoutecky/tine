//! Graph's page cache: building, warming and streaming the parsed-page cache,
//! repairing it, invalidation, and cache_upsert / cache_remove.

use super::*;

/// What `warm_projection_cancellable` achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WarmProjectionOutcome {
    /// The projection (or a parsed snapshot already queued to it) owns
    /// readiness at the generation the warm observed.
    Owned,
    /// The warm observed a generation that moved under it, or its enqueue was
    /// refused by work the worker will drain shortly: run it again.
    Retry,
    /// No projection can own readiness: none attached, the graph text scope
    /// unreadable, the writer lease held elsewhere, or the worker failed.
    Unavailable,
    Cancelled,
}

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

    /// The page inventory plus the entries the read walk skipped (GH #332).
    /// Skipped entries become page index failures, so the source is never
    /// reported complete while a page may be missing from it.
    fn page_build_entries(
        &self,
        permit: &GraphTextWritePermit,
    ) -> Result<(Vec<PageEntry>, Vec<String>), String> {
        #[cfg(test)]
        self.page_build_test
            .enumerations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.graph_text_entries_and_skipped(permit)
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
        let (entries, skipped) = match self.page_build_entries(permit) {
            Ok(inventory) => inventory,
            Err(failure) => {
                return PageCacheBuild {
                    pages: Vec::new(),
                    failures: vec![failure],
                };
            }
        };
        let mut built = self.parse_page_entries_with_permit(permit, entries);
        built.failures.extend(skipped);
        built
    }

    fn parse_page_entries_with_permit(
        &self,
        permit: &GraphTextWritePermit,
        entries: Vec<PageEntry>,
    ) -> PageCacheBuild {
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
        let (completed, structural) = {
            let cache = self.cache.read().unwrap();
            let current_generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            let structural = self
                .cache_structural_gen
                .load(std::sync::atomic::Ordering::Acquire);
            let completed = if current_generation != expected_generation {
                Some(PageBuildOutcome::GenerationDrift)
            } else if cache.is_some() {
                Some(PageBuildOutcome::AlreadyAvailable)
            } else {
                None
            };
            (completed, structural)
        };
        if let Some(outcome) = completed {
            let flight = Arc::new(PageBuildFlight::new(expected_generation, structural));
            flight.complete(outcome);
            return (flight, false);
        }
        let flight = Arc::new(PageBuildFlight::new(expected_generation, structural));
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
        let outcome = PageBuildOutcome::from(self.install_built(&flight, built));
        self.finish_page_build(&flight, outcome);
        outcome
    }

    /// Install a freshly-built whole-graph snapshot atomically: the parsed pages
    /// into the cache, their on-disk revs into `disk_revs`. Cache set BEFORE
    /// disk_revs so a reader never observes a fresh rev paired with a stale cache.
    pub(super) fn install_built(
        &self,
        flight: &PageBuildFlight,
        built: PageCacheBuild,
    ) -> PageCacheInstallOutcome {
        let expected_generation = flight.expected_generation;
        #[cfg(test)]
        if self
            .page_build_test
            .drift_before_install
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            // Real drift: a structural move, which no revision match excuses.
            self.cache_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            self.cache_structural_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        let PageCacheBuild {
            pages: built,
            mut failures,
        } = built;
        failures.sort();
        failures.dedup();
        let source_complete = failures.is_empty();
        let revs: std::collections::HashMap<PathBuf, String> = built
            .iter()
            .map(|(e, _, r)| (e.path.clone(), r.clone()))
            .collect();
        let pages: Vec<(PageEntry, Arc<Document>)> = built
            .into_iter()
            .map(|(e, d, _)| (e, Arc::new(d)))
            .collect();
        let index = build_page_cache_index(&pages);
        let page_list: Vec<PageEntry> = pages.iter().map(|(entry, _)| entry.clone()).collect();
        // Publish cache + revs atomically under the cache lock (cache → disk_revs
        // order), but only at a generation that still describes what the owner
        // parsed. A cache that another publisher already supplied is likewise
        // never overwritten.
        let mut guard = self.cache.write().unwrap();
        // Opening a page publishes it even when its bytes are unchanged; a
        // cold open does that for today's journal while this parse runs, and
        // discarding the parse for it sent the next reader into a second
        // whole-graph parse (GH #543). Such a move leaves the parse current.
        let Some(generation) = self.generation_after_benign_drift(
            expected_generation,
            flight.expected_structural,
            |path| revs.get(path).map(String::as_str),
        ) else {
            return PageCacheInstallOutcome::GenerationDrift;
        };
        let expected_generation = generation;
        let effective_index = Arc::new(build_effective_identity_index(
            expected_generation,
            &pages,
            failures.clone(),
        ));
        if guard.is_some() && self.page_index_failures.read().unwrap().is_empty() {
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
        self.direct_projection_enqueue_full(
            expected_generation,
            pages,
            Arc::new(revs),
            false,
            source_complete,
        );
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
                let built = self.load_all_pages_with_permit(&permit);
                let outcome = PageBuildOutcome::from(self.install_built(&flight, built));
                self.finish_page_build(&flight, outcome);
            } else {
                let _ = flight.wait();
            }
        }
    }

    /// Read one already-captured parsed snapshot without starting a page build.
    /// Pre-ready interactive search is allowed to use this owner, but must not
    /// turn an unavailable projection into a cold query-thread graph parse.
    pub(super) fn with_captured_pages<T>(
        &self,
        f: impl FnOnce(&[(PageEntry, Arc<Document>)]) -> T,
    ) -> Option<T> {
        let snapshot = self.cache.read().unwrap().as_ref().map(Arc::clone)?;
        Some(f(snapshot.as_slice()))
    }

    /// Eagerly build the page cache plus graph-open derived maps (call once after
    /// opening, off the hot path).
    pub fn warm_cache(&self) {
        let _ = self.warm_cache_cancellable(|| false);
    }

    /// Build graph-open caches while allowing a revoked window binding to stop
    /// between files and derived-map phases. Returns false when cancelled.
    pub fn warm_cache_cancellable(&self, cancelled: impl Fn() -> bool) -> bool {
        // Validate a clean warm image without parsing. A stale, cold, damaged,
        // or raced image falls through to the existing parsed-page build
        // flight, whose captured snapshot is built unpublished and published
        // atomically by the projection worker.
        let _retrying = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(|projection| projection.begin_warm());
        let warmed = match self.warm_projection_cancellable(&cancelled) {
            WarmProjectionOutcome::Owned => true,
            WarmProjectionOutcome::Cancelled => return false,
            WarmProjectionOutcome::Retry | WarmProjectionOutcome::Unavailable => false,
        };
        if !warmed && (!self.warm_page_cache_cancellable(&cancelled) || cancelled()) {
            return false;
        }
        if warmed {
            // The validation/build turn may still be running; the
            // derived-map reads below use a bounded wait and would otherwise
            // parse the whole graph (GH #543). Nothing is owed if readiness is
            // no longer coming at this generation: the reads then take their
            // ordinary route.
            if let Some(projection) = self
                .direct_projection
                .lock()
                .unwrap()
                .as_ref()
                .map(Arc::clone)
            {
                let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
                projection.wait_until_ready_at(generation, &cancelled);
            }
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
    /// unless the worker names replacements.
    ///
    /// `Owned` means the projection (or a full snapshot already queued to it)
    /// owns readiness at the generation this warm observed. `Retry` means the
    /// attempt accomplished nothing and MUST be run again — a mutation raced
    /// it, or the queue was held by work that will drain shortly; the caller
    /// owns that retry, and dropping the outcome is how an obligation ends up
    /// with no producer (GH #543). `Unavailable` means no projection can own
    /// readiness at all: none attached, the graph text scope unreadable, the
    /// lease held by another instance, or the worker gone. `Cancelled` is the
    /// binding being revoked under us.
    pub(super) fn warm_projection_cancellable(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> WarmProjectionOutcome {
        use WarmProjectionOutcome as Outcome;
        if cancelled() {
            return Outcome::Cancelled;
        }
        let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        else {
            // No projection to own readiness; a parsed cache is all there is.
            return if self.cache.read().unwrap().is_some() {
                Outcome::Owned
            } else {
                Outcome::Unavailable
            };
        };
        if self.cache.read().unwrap().is_some() {
            let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            // A complete cache installation queues its exact snapshot. A
            // partial cache does not own a stale rebuild: retry the inventory
            // so an unreadable page can recover without replacing the older
            // complete SQL image with an omission.
            if projection.ready_at(generation)
                || self.page_index_failures.read().unwrap().is_empty()
            {
                return Outcome::Owned;
            }
        }
        // GH #543: announce this warm before the inventory read below. On a
        // 10k-page graph that read takes seconds (tens on Windows), and a
        // query landing inside it must see Indexing, not an idle projection
        // it should repair — its repair would be a second warm racing this
        // one on the query thread.
        let in_flight = projection.begin_warm();
        let warm_started = std::time::Instant::now();
        crate::direct_projection::projection_diag(|| {
            "warm announced; reading inventory".to_owned()
        });
        #[cfg(test)]
        {
            // Bind first: an `if let` scrutinee would hold the guard across
            // the pause and block the very second warm the test provokes.
            let pause = self
                .page_build_test
                .warm_validation_pause
                .lock()
                .unwrap()
                .take();
            if let Some(pause) = pause {
                pause.reached.wait();
                pause.release.wait();
            }
        }
        let Ok(permit) = self.admit_retained_graph_text_writer() else {
            return Outcome::Unavailable;
        };
        let structural = self
            .cache_structural_gen
            .load(std::sync::atomic::Ordering::Acquire);
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let Ok((entries, skipped)) = self.page_build_entries(&permit) else {
            return Outcome::Unavailable;
        };
        let walk_order = entries
            .iter()
            .map(|entry| entry.rel_path.clone())
            .collect::<Vec<_>>();
        let mut sources = Vec::with_capacity(entries.len());
        let mut retained = Vec::new();
        let mut failures = skipped;
        let mut text_bytes = 0u64;
        let progress = self.indexing_progress.begin(
            crate::indexing_progress::IndexingPhase::Checking,
            entries.len(),
        );
        for (i, entry) in entries.into_iter().enumerate() {
            if cancelled() {
                return Outcome::Cancelled;
            }
            progress.advance(1);
            match self.graph_text_read_optional_text_with_identity(&permit, &entry.path) {
                Ok(Some((content, _))) => {
                    let revision = content_rev(&content);
                    text_bytes += content.len() as u64;
                    sources.push((entry, revision));
                }
                // The file is genuinely gone: omitting it from `sources` is
                // how the walk says "delete its rows".
                Ok(None) => failures.push(entry.rel_path),
                // The file EXISTS and could not be read. Omitting it would
                // delete a live page's rows over a transient disk error or a
                // Windows sharing violation (GH #543), so name it retained:
                // validation keeps its rows and its old stored revision, and
                // the next warm re-reads it.
                Err(_) => {
                    retained.push(entry.clone());
                    failures.push(entry.rel_path);
                }
            }
            if i % 24 == 23 {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        #[cfg(test)]
        {
            let pause = self
                .page_build_test
                .warm_read_done_pause
                .lock()
                .unwrap()
                .take();
            if let Some(pause) = pause {
                pause.reached.wait();
                pause.release.wait();
            }
        }
        let read_at = generation;
        let abandoned = || {
            crate::direct_projection::projection_diag(|| {
                format!(
                    "warm abandoned on drift after {}ms",
                    warm_started.elapsed().as_millis()
                )
            });
            Outcome::Retry
        };
        crate::direct_projection::projection_diag(|| {
            format!(
                "warm inventory read in {}ms pages={} retained={} failures={} text_mib={:.1}",
                warm_started.elapsed().as_millis(),
                sources.len(),
                retained.len(),
                failures.len(),
                text_bytes as f64 / (1024.0 * 1024.0),
            )
        });
        let parse_config = Arc::new(self.config.parse_config());
        // A stale image is repaired page by page at most this many times
        // before the complete rebuild takes over: each repair re-validates,
        // and a graph still changing underneath it gets the rebuild.
        let mut repairs_left = 2usize;
        let mut repair = None;
        loop {
            let Some((generation, published)) =
                self.warm_generation_after_drift(read_at, structural, &sources)
            else {
                return abandoned();
            };
            // Validation leaves these pages as the image holds them, and the
            // updates they published replace them after it (audit IT-03).
            let mut published_pages = Vec::with_capacity(published.len());
            for path in &published {
                let rel_path = match sources.iter().find(|(entry, _)| &entry.path == path) {
                    Some((entry, _)) => entry.rel_path.clone(),
                    // A page created after the read: the walk never listed it.
                    None => match self.entry_for_path(path) {
                        Some(entry) => entry.rel_path,
                        None => return abandoned(),
                    },
                };
                published_pages.push(rel_path);
            }
            if generation != read_at {
                // Each of those publications queued an update; the queue takes
                // the warm beside them and applies them after validating it.
                crate::direct_projection::projection_diag(|| {
                    format!(
                        "warm kept across generations {read_at}..{generation}; \
                         {} page(s) published since the read follow as updates",
                        published.len()
                    )
                });
            }
            let attempt = projection.enqueue_warm_with_repair(
                generation,
                sources.clone(),
                retained.clone(),
                published_pages,
                walk_order.clone(),
                repair.take(),
                Arc::clone(&parse_config),
                text_bytes,
            );
            let Some(attempt) = attempt else {
                // Refused: the worker is gone or failed without a rebuild
                // queued (nothing a retry changes), or the queue outranks this
                // generation.
                return if projection.worker_failed() || !projection.worker_available() {
                    Outcome::Unavailable
                } else {
                    Outcome::Retry
                };
            };
            let outcome_started = std::time::Instant::now();
            // Waiting on THIS attempt's verdict: a warm admitted after this
            // one owns the queue, and its verdict is not ours to take (GH #543).
            let outcome = projection.wait_warm_outcome(attempt);
            crate::direct_projection::projection_diag(|| {
                format!(
                    "warm validation returned {} after {}ms",
                    match &outcome {
                        crate::direct_projection::WarmOutcome::Clean => "clean".to_owned(),
                        crate::direct_projection::WarmOutcome::Superseded =>
                            "superseded".to_owned(),
                        crate::direct_projection::WarmOutcome::Failed => "failed".to_owned(),
                        crate::direct_projection::WarmOutcome::FreshBuildRequired =>
                            "fresh-build-required".to_owned(),
                        crate::direct_projection::WarmOutcome::Changed {
                            replacements,
                            deletions,
                        } => format!(
                            "changed replacements={} deletions={}",
                            replacements.len(),
                            deletions.len()
                        ),
                    },
                    outcome_started.elapsed().as_millis()
                )
            });
            match outcome {
                crate::direct_projection::WarmOutcome::Clean => {
                    drop(in_flight);
                    self.publish_page_index_failures(generation, failures);
                    return Outcome::Owned;
                }
                crate::direct_projection::WarmOutcome::Superseded => {
                    // A captured parsed snapshot already owns readiness.
                    return Outcome::Owned;
                }
                crate::direct_projection::WarmOutcome::Failed => return Outcome::Unavailable,
                crate::direct_projection::WarmOutcome::FreshBuildRequired => {
                    return Outcome::Retry;
                }
                crate::direct_projection::WarmOutcome::Changed {
                    replacements,
                    deletions,
                } => {
                    if repairs_left == 0 {
                        return Outcome::Retry;
                    }
                    repairs_left -= 1;
                    if cancelled() {
                        return Outcome::Cancelled;
                    }
                    let Some(parsed) = self.parse_warm_repair(&permit, &mut sources, &replacements)
                    else {
                        return Outcome::Retry;
                    };
                    repair = Some(crate::direct_projection::WarmRepair {
                        replacements: parsed,
                        deletions,
                    });
                }
            }
        }
    }

    /// Parse exactly the pages a warm validation named `Changed`, updating
    /// their revisions in `sources` to the bytes parsed. `None` when one of
    /// them cannot be read or parsed: the complete rebuild then owns it.
    fn parse_warm_repair(
        &self,
        permit: &GraphTextWritePermit,
        sources: &mut [(PageEntry, String)],
        replacements: &[String],
    ) -> Option<Vec<(PageEntry, Arc<Document>, String)>> {
        let index = sources
            .iter()
            .enumerate()
            .map(|(i, (entry, _))| (entry.rel_path.clone(), i))
            .collect::<std::collections::HashMap<_, _>>();
        let mut parsed = Vec::with_capacity(replacements.len());
        for rel_path in replacements {
            let i = *index.get(rel_path)?;
            let entry = sources[i].0.clone();
            let Ok(Some((content, _))) =
                self.graph_text_read_optional_text_with_identity(permit, &entry.path)
            else {
                return None;
            };
            #[cfg(test)]
            self.page_build_test
                .repair_parses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let Ok(Some((effective, document, revision))) =
                isolate_page_parse(entry, &self.journal_format, |entry| {
                    Some(self.parse_session_page_content(entry, &content))
                })
            else {
                return None;
            };
            sources[i].1 = revision.clone();
            parsed.push((effective, Arc::new(document), revision));
        }
        Some(parsed)
    }

    /// The generation a finished warm inventory read may be queued at, and
    /// the pages published since the read at other bytes than it saw; `None`
    /// when it must be read again.
    ///
    /// Opening a page publishes it even when its bytes are unchanged, and a
    /// launch opens today's journal while the warm is still reading. On a
    /// 10,000-page Windows graph that read takes seconds, and throwing it away
    /// for that publication sent every warm reopen to a whole-graph parse
    /// (GH #543). An edited or new page published during the read did the
    /// same (audit IT-03), although its publication queued the update that
    /// brings the index to its current bytes. So the read survives any move
    /// that is not structural: a publication at the revision the warm read is
    /// benign, and every other published page is returned for validation to
    /// leave as the image holds it, with its update applied after. A removal
    /// or invalidation (structural) still discards the read: nothing queued
    /// describes what it took away.
    fn warm_generation_after_drift(
        &self,
        read_at: u64,
        structural: u64,
        sources: &[(PageEntry, String)],
    ) -> Option<(u64, std::collections::HashSet<PathBuf>)> {
        use std::sync::atomic::Ordering;
        let read: std::collections::HashMap<&Path, &str> = sources
            .iter()
            .map(|(entry, revision)| (entry.path.as_path(), revision.as_str()))
            .collect();
        // Every mover publishes its session record and bumps both counters
        // under the cache write lock, so this read lock sees them together.
        let _cache = self.cache.read().unwrap();
        let current = self.cache_gen.load(Ordering::Acquire);
        if current == read_at {
            return Some((current, std::collections::HashSet::new()));
        }
        if self.cache_structural_gen.load(Ordering::Acquire) != structural {
            return None;
        }
        let config = self.config.parse_config().digest();
        let published = self
            .session_page_ids
            .read()
            .unwrap()
            .iter()
            .filter(|(path, ids)| {
                ids.config != config || read.get(path.as_path()) != Some(&ids.revision.as_str())
            })
            .map(|(path, _)| path.clone())
            .collect();
        Some((current, published))
    }

    /// The shared rule behind `warm_generation_after_drift` and cold
    /// installation. The caller holds the cache lock (read or write), which
    /// every mover takes to publish its session record and bump the counters.
    fn generation_after_benign_drift<'a>(
        &self,
        read_at: u64,
        structural: u64,
        read: impl Fn(&Path) -> Option<&'a str>,
    ) -> Option<u64> {
        use std::sync::atomic::Ordering;
        let current = self.cache_gen.load(Ordering::Acquire);
        if current == read_at {
            return Some(current);
        }
        if self.cache_structural_gen.load(Ordering::Acquire) != structural {
            return None;
        }
        let config = self.config.parse_config().digest();
        self.session_page_ids
            .read()
            .unwrap()
            .iter()
            .all(|(path, ids)| {
                ids.config == config && read(path.as_path()) == Some(ids.revision.as_str())
            })
            .then_some(current)
    }

    fn publish_page_index_failures(&self, generation: u64, mut failures: Vec<String>) {
        failures.sort();
        failures.dedup();
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation {
            *self.page_index_failures.write().unwrap() = failures;
        }
    }

    pub(super) fn warm_page_cache_cancellable(&self, cancelled: &impl Fn() -> bool) -> bool {
        if cancelled() {
            return false;
        }
        if self.cache.read().unwrap().is_some()
            && self.page_index_failures.read().unwrap().is_empty()
        {
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
        let (entries, skipped) = match self.page_build_entries(&permit) {
            Ok(inventory) => inventory,
            Err(failure) => {
                let built = PageCacheBuild {
                    pages: Vec::new(),
                    failures: vec![failure],
                };
                let outcome = PageBuildOutcome::from(self.install_built(&flight, built));
                self.finish_page_build(&flight, outcome);
                return outcome.installed() && !cancelled();
            }
        };
        let mut built = PageCacheBuild::with_capacity(entries.len());
        built.failures.extend(skipped);
        let mut baselines: Vec<(PathBuf, ContentDigest, String)> =
            Vec::with_capacity(entries.len());
        let progress = self.indexing_progress.begin(
            crate::indexing_progress::IndexingPhase::Reading,
            entries.len(),
        );
        for (i, e) in entries.into_iter().enumerate() {
            progress.advance(1);
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
        let outcome = PageBuildOutcome::from(self.install_built(&flight, built));
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
    pub(crate) fn warm_repair_parses_test(&self) -> usize {
        self.page_build_test
            .repair_parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn page_build_parses_test(&self) -> usize {
        self.page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Move the cache generation as a save would, without a save: a warm
    /// validation that observes it abandons, exactly as on a racing edit.
    /// There is no page to check the move against, so it counts as structural.
    #[cfg(test)]
    pub(crate) fn drift_generation_test(&self) {
        self.cache_structural_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        self.cache_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// The walk inventory as the warm sees it, parsing nothing.
    #[cfg(test)]
    pub(crate) fn walk_entries_test(&self) -> Vec<PageEntry> {
        let permit = self.admit_retained_graph_text_writer().unwrap();
        self.page_build_entries(&permit).unwrap().0
    }

    #[cfg(test)]
    pub(crate) fn on_demand_parses_test(&self) -> usize {
        self.page_build_test
            .on_demand_parses
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// GH #543 test hook: pause the NEXT warm validation after it has
    /// announced itself and before it reads page bytes.
    #[cfg(test)]
    pub(crate) fn pause_next_warm_validation_test(&self) -> Arc<PageBuildTestPause> {
        let pause = Arc::new(PageBuildTestPause::new());
        *self.page_build_test.warm_validation_pause.lock().unwrap() = Some(Arc::clone(&pause));
        pause
    }

    /// GH #543 test hook: pause the NEXT warm validation after it has read
    /// every page and before it checks whether the generation moved.
    #[cfg(test)]
    pub(crate) fn pause_next_page_publication_test(&self) -> Arc<PageBuildTestPause> {
        let pause = Arc::new(PageBuildTestPause::new());
        *self.page_build_test.upsert_published_pause.lock().unwrap() = Some(Arc::clone(&pause));
        pause
    }

    #[cfg(test)]
    pub(crate) fn pause_next_warm_after_read_test(&self) -> Arc<PageBuildTestPause> {
        let pause = Arc::new(PageBuildTestPause::new());
        *self.page_build_test.warm_read_done_pause.lock().unwrap() = Some(Arc::clone(&pause));
        pause
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
        self.cache_structural_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
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
        // Taken before the cache lock: attaching holds the projection slot
        // while it reads the cache, so the slot is never locked under it.
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone);
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
        // A partial parsed snapshot may have left a healthy older SQL image in
        // AwaitingFullInventory. When the last unreadable page is reconciled,
        // this cache already is the normal complete parsed-snapshot producer:
        // publish that captured Arc and its exact revisions instead of sending
        // a delta the worker must refuse while it still owes a full inventory.
        let recovered_projection_snapshot =
            (cache_built && failures_changed && page_inventory_complete).then(|| {
                (
                    Arc::clone(guard.as_ref().expect("a built cache has pages")),
                    Arc::new(self.disk_revs.read().unwrap().clone()),
                )
            });
        *failures_guard = resulting_failures;
        // The update is queued before the new generation can be observed. A
        // warm that reads this generation keeps its inventory and leaves this
        // page to its update (audit IT-03); queued after the lock, the warm
        // could publish readiness at this generation with the page's old
        // rows, and an answer cached at it would outlive the update.
        let queued_under_lock = recovered_projection_snapshot.is_none()
            && projection.as_ref().is_some_and(|projection| {
                projection.enqueue_replace(
                    newgen,
                    evict_entry.clone(),
                    Arc::clone(&evict_doc),
                    projection_revision.clone(),
                    Arc::new(self.config.parse_config()),
                );
                true
            });
        drop(failures_guard);
        drop(guard);
        #[cfg(test)]
        {
            let pause = self
                .page_build_test
                .upsert_published_pause
                .lock()
                .unwrap()
                .take();
            if let Some(pause) = pause {
                pause.reached.wait();
                pause.release.wait();
            }
        }
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
        if let Some((pages, revisions)) = recovered_projection_snapshot {
            self.direct_projection_enqueue_full(newgen, pages, revisions, false, true);
        } else if !queued_under_lock {
            self.direct_projection_enqueue_replace(
                newgen,
                evict_entry,
                evict_doc,
                projection_revision,
            );
        }
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
                    Some(("U" | "UI", rest)) => rest.splitn(3, '\0').nth(2).is_none_or(|target| {
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
        self.cache_structural_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
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
        self.cache_structural_gen
            .fetch_add(1, std::sync::atomic::Ordering::Release);
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
