//! Graph's Direct projection lifetime: attaching and detaching it, the Concord
//! ledger, enqueueing replacements and deletes, marking it stale, and the
//! projection-backed page inventory, on-demand parses and property facets.

use super::*;

impl Graph {
    /// Attach Direct Files' app-private disposable SQLite projection.
    ///
    /// This never reads or writes graph files. If the parsed cache is already
    /// warm, its exact snapshot is queued; otherwise `install_built` supplies it
    /// when the ordinary background warm completes.
    pub fn attach_direct_projection(&self, path: PathBuf) -> io::Result<()> {
        let projection = Arc::new(crate::direct_projection::DirectProjection::start(path)?);
        let mut slot = self.direct_projection.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        *slot = Some(Arc::clone(&projection));
        let cache = self.cache.read().unwrap();
        if let Some(snapshot) = cache.as_ref().map(Arc::clone) {
            let revisions = Arc::new(self.disk_revs.read().unwrap().clone());
            projection.enqueue_full(
                self.cache_gen.load(std::sync::atomic::Ordering::Acquire),
                snapshot,
                revisions,
                Arc::new(self.config.parse_config()),
                self.page_index_failures.read().unwrap().is_empty(),
            );
        }
        Ok(())
    }

    /// Detach the Direct Files projection and wait for its writer to exit.
    ///
    /// A configuration refresh reopens the same root and attaches a projection
    /// at the SAME path. A second `DirectProjection::start` while this worker
    /// still holds the exclusive writer lease races it (observed as a 15 s
    /// "did not converge" under load), so the old worker is retired first.
    /// Returns whether it exited within `timeout`; `false` is reported by the
    /// caller, never treated as fatal — the replacement attach then decides.
    pub fn detach_direct_projection(&self, timeout: std::time::Duration) -> bool {
        let projection = self.direct_projection.lock().unwrap().take();
        match projection {
            Some(projection) => projection.close_and_wait_for_worker(timeout),
            None => true,
        }
    }

    /// Register the application's existing watcher wake channel and observe the
    /// last committed-image notification. This is notification state only; query reads
    /// never compare it with an edit or wait for it to advance.
    pub fn observe_direct_projection_commits(
        &self,
        wake: std::sync::mpsc::Sender<()>,
    ) -> Option<u64> {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(|projection| projection.observe_commits(wake))
    }

    /// What the graph-sized index work is doing, for the indexing progress bar
    /// (GH #543). `None` once search is answered by a current index, or when
    /// nothing graph-sized is running. Presentation only.
    pub fn indexing_progress(&self) -> Option<crate::indexing_progress::IndexingProgress> {
        use crate::direct_projection::ProjectionProgress;
        use crate::indexing_progress::{IndexingPhase, IndexingProgress};
        use crate::query::QueryReadinessReason as Reason;
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone);
        let Some(projection) = projection else {
            return self.indexing_progress.snapshot();
        };
        if let Some(build) = projection.build_progress() {
            return Some(build);
        }
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if projection.ready_at(generation) {
            return None;
        }
        if let Some(pass) = self.indexing_progress.snapshot() {
            return Some(pass);
        }
        // Between passes: a queued snapshot or an announced warm is still
        // graph-sized work, so keep the bar up rather than flicker it off.
        match projection.progress_at(generation) {
            ProjectionProgress::Working(Reason::Recovering | Reason::Indexing) => {
                Some(IndexingProgress::unmeasured(IndexingPhase::Indexing))
            }
            _ => None,
        }
    }

    /// Test barrier for ordinary producer progression. Production queries read
    /// the current coherent image and never wait for a saved-edit generation.
    #[cfg(test)]
    pub(crate) fn wait_for_direct_projection_for_test(
        &self,
        timeout: std::time::Duration,
    ) -> io::Result<()> {
        use crate::direct_projection::ProjectionProgress;
        let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "snapshot query projection is not attached",
            ));
        };
        let started = std::time::Instant::now();
        loop {
            let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            match projection.progress_at(generation) {
                ProjectionProgress::Ready => return Ok(()),
                ProjectionProgress::Working(_) => {}
                ProjectionProgress::Stale => {
                    return Err(io::Error::other(
                        "test projection stopped before the producer generation",
                    ))
                }
                ProjectionProgress::Stopped => {
                    return Err(io::Error::other(
                        "snapshot query projection worker is unavailable",
                    ))
                }
            }
            if started.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "snapshot query projection did not finish indexing in time",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Attach the Concord base ledger (ADR 0056) rooted at `dir` (an
    /// app-private directory OUTSIDE the graph tree). Idempotent; the first
    /// attach wins. Queues a background prune of unreferenced blobs.
    pub fn attach_concord_ledger(&self, dir: PathBuf) {
        let ledger = Arc::new(crate::concord_ledger::ConcordLedger::new(dir));
        if self.concord_ledger.set(Arc::clone(&ledger)).is_ok() {
            ledger.queue_prune();
        }
    }

    /// The attached ledger, if any (None ⇒ every Concord hook no-ops).
    pub fn concord_ledger(&self) -> Option<&Arc<crate::concord_ledger::ConcordLedger>> {
        self.concord_ledger.get()
    }

    /// Quitting: wait until `deadline` for this graph's queued ledger updates.
    /// True when there was nothing to wait for or the queue drained in time.
    pub fn drain_concord_ledger_for_exit(&self, deadline: std::time::Instant) -> bool {
        self.concord_ledger
            .get()
            .is_none_or(|ledger| ledger.drain_for_exit(deadline))
    }

    /// Best-effort ledger update: `content` is now the exact text Tine and the
    /// disk agree on for `path`. Called after a successful save commit and
    /// after an external-change admission. Foreground cost is one channel send;
    /// non-page paths (config, assets) are filtered out here.
    pub(super) fn concord_record_agreed(&self, path: &Path, content: &str) {
        let Some(ledger) = self.concord_ledger.get() else {
            return;
        };
        if self.entry_for_path(path).is_none() || path_is_sync_conflict(path) {
            return;
        }
        ledger.record(&self.rel_path(path), content);
    }

    /// The winner page a conflict copy shadows (same dir, same extension, base
    /// stem), as a graph-relative path — the identity the ledger pins under.
    pub(super) fn conflict_winner_rel(&self, conflict_path: &Path) -> Option<String> {
        let ext = text_extension_from_path(conflict_path)?;
        let stem = conflict_path.file_stem()?.to_str()?;
        let base_stem = sync_conflict_base(stem)?;
        let winner = conflict_path.parent()?.join(format!("{base_stem}.{ext}"));
        Some(self.rel_path(&winner))
    }

    /// `force` skips the readiness shortcut below. A repair that has latched
    /// `pending.rebuild` MUST force: the worker consumes that flag only
    /// together with a full or warm payload, so a skipped snapshot would leave
    /// the rebuild latched with nothing to ride in on and every later capture
    /// refused for the lifetime of the graph.
    pub(super) fn direct_projection_enqueue_full(
        &self,
        generation: u64,
        pages: Arc<Vec<(PageEntry, Arc<Document>)>>,
        revisions: Arc<std::collections::HashMap<PathBuf, String>>,
        force: bool,
        source_complete: bool,
    ) {
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            // R6: a projection already READY at this generation was validated
            // from the same bytes this snapshot was parsed from; a redundant
            // snapshot would only open a NotReady window while it re-validates.
            if !force && projection.ready_at(generation) {
                return;
            }
            projection.enqueue_full(
                generation,
                pages,
                revisions,
                Arc::new(self.config.parse_config()),
                source_complete,
            );
        }
    }

    pub(super) fn direct_projection_enqueue_replace(
        &self,
        generation: u64,
        entry: PageEntry,
        document: Arc<Document>,
        revision: String,
    ) {
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            projection.enqueue_replace(
                generation,
                entry,
                document,
                revision,
                Arc::new(self.config.parse_config()),
            );
        }
    }

    /// The ONE way a mutation that changes the page SET tells the index what it
    /// did. Renames, merges and file rescues all move or retire physical page
    /// paths, and each one that skipped this left the index holding rows for a
    /// file that no longer exists — with nothing queued, no progress shown, and
    /// search answering from it indefinitely, because a successful answer never
    /// reaches the repair a refusal would start (GH #543, third and fourth
    /// audits). `cache_upsert`/`cache_remove` are the one-page equivalents for
    /// an ordinary save and delete.
    ///
    /// The whole change goes in one call: published one delta at a time, the
    /// queue can empty between them and readiness is announced for a generation
    /// that is only half enqueued.
    pub(super) fn direct_projection_publish_page_set(
        &self,
        generation: u64,
        changes: Vec<crate::direct_projection::PageSetChange>,
    ) {
        for change in &changes {
            if let crate::direct_projection::PageSetChange::Delete { entry } = change {
                self.session_page_ids.write().unwrap().remove(&entry.path);
            }
        }
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            projection.enqueue_page_set(generation, changes, Arc::new(self.config.parse_config()));
        }
    }

    pub(super) fn direct_projection_enqueue_delete(&self, generation: u64, entry: PageEntry) {
        self.session_page_ids.write().unwrap().remove(&entry.path);
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            // A delete lowers nothing, so it carries no parse config (F11).
            projection.enqueue_delete(generation, entry);
        }
    }

    pub(super) fn direct_projection_mark_stale(&self) {
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            projection.mark_stale();
        }
    }

    /// Announce the launch warm when it is scheduled rather than when its
    /// thread reaches the first page read. The app delays that thread so the
    /// first journal paint goes first, and the page list requested by that
    /// same paint used to find no warm announced, parse every page, and queue
    /// a full snapshot ahead of the warm (GH #543). Hold the returned value
    /// for the life of the thread that runs `warm_cache_cancellable`; a
    /// thread that is cancelled or finishes drops it, so a reader can never
    /// wait on a warm nobody runs.
    pub fn announce_launch_warm(&self) -> LaunchWarmAnnouncement {
        LaunchWarmAnnouncement(
            self.direct_projection
                .lock()
                .unwrap()
                .as_ref()
                .map(|projection| projection.begin_warm()),
        )
    }

    /// Readiness for a whole-graph derived read (page list, aliases, property
    /// owners, block-ref counts). Beyond the short delta wait, a read that
    /// finds a warm validation or queued edits in flight and no parsed cache
    /// keeps waiting for them: its only alternative is parsing every page, and on a
    /// warm reopen that parse queued a full snapshot which outranked the warm
    /// and doubled the time to a working search (GH #543). The warm finishes
    /// no later than such a parse would; if it gives up, the read falls back
    /// as before. With a parsed cache present the fallback is cheap, so no
    /// extra wait (this also keeps the warm thread from waiting on itself).
    /// Returns the generation the projection is ready at, which is newer than
    /// `generation` when a page was published during the wait.
    pub(super) fn wait_for_derived_read(
        &self,
        projection: &crate::direct_projection::DirectProjection,
        mut generation: u64,
    ) -> Option<u64> {
        use crate::direct_projection::ProjectionProgress;
        use crate::query::QueryReadinessReason as Reason;
        loop {
            if projection.wait_ready_at(generation) {
                return Some(generation);
            }
            // A replaced graph's reads are no longer anyone's to wait for.
            if self.is_retired() {
                return None;
            }
            let coming = match projection.progress_at(generation) {
                // `Busy`: the worker has taken the queued warm or edit and is
                // applying it.
                ProjectionProgress::Working(Reason::Indexing | Reason::Busy) => true,
                // An edit queued behind a turn -- today's journal and a save at
                // launch, behind a slow disk -- lands on a validated image and
                // readiness follows. Parsing the graph instead reads every page
                // to answer what one delta settles. Before validation the edit
                // waits for an inventory, so it is not by itself coming.
                ProjectionProgress::Working(Reason::PendingEdits) => projection.validated(),
                _ => false,
            };
            if !coming || self.cache.read().unwrap().is_some() {
                return projection.ready_at(generation).then_some(generation);
            }
            // Opening today's journal publishes it and moves the generation;
            // the warm keeps going across such moves, so follow it.
            generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// R6: the page inventory from the ready projection, rebuilt into the
    /// walk's `PageEntry` shape. `pages.name` is the effective (title::-aware)
    /// name because the producer lowers the effective entry; kind comes from
    /// the row and a journal's sort key from its name.
    pub(super) fn direct_projection_page_inventory(&self) -> Option<(u64, Vec<PageEntry>)> {
        self.indexed_read(|projection, generation| {
            self.direct_projection_page_inventory_at(projection, generation)
        })
    }

    fn direct_projection_page_inventory_at(
        &self,
        projection: &Arc<crate::direct_projection::DirectProjection>,
        generation: u64,
    ) -> Option<(u64, Vec<PageEntry>)> {
        let rows = projection.page_inventory(generation)?;
        let mut entries = Vec::with_capacity(rows.len());
        for (name, rel_path, kind) in rows {
            let kind = crate::direct_projection::page_kind_from_sql(kind)?;
            let date_key = match kind {
                PageKind::Journal => Some(self.journal_format.parse(&name)?.ordinal_key()),
                PageKind::Page => None,
            };
            entries.push(PageEntry {
                name,
                kind,
                date_key,
                path: self.root.join(&rel_path),
                rel_path,
            });
        }
        entries.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        Some((generation, entries))
    }

    /// R6: parse exactly the named pages for reference/fuzzy hydration when no
    /// parsed cache exists. The documents are returned to the caller and
    /// dropped after use — nothing is installed or retained.
    pub(super) fn parse_pages_on_demand(
        &self,
        generation: u64,
        paths: Vec<PathBuf>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        self.parse_pages_on_demand_inner(
            generation,
            paths.into_iter().map(|path| (path, None)).collect(),
        )
    }

    pub(super) fn parse_pages_on_demand_with_revisions(
        &self,
        generation: u64,
        sources: Vec<(PathBuf, String)>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        self.parse_pages_on_demand_inner(
            generation,
            sources
                .into_iter()
                .map(|(path, revision)| (path, Some(revision)))
                .collect(),
        )
    }

    fn parse_pages_on_demand_inner(
        &self,
        generation: u64,
        sources: Vec<(PathBuf, Option<String>)>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        let permit = self.admit_retained_graph_text_writer().ok()?;
        let mut pages = Vec::with_capacity(sources.len());
        let config_digest = self.config.parse_config().digest();
        for (relative, projected_revision) in sources {
            let absolute = self.root.join(&relative);
            let entry = self.graph_inventory_entry(&absolute).ok()??;
            if entry.rel_path != relative.to_string_lossy() {
                return None;
            }
            let (content, _) = self
                .graph_text_read_optional_text_with_identity(&permit, &entry.path)
                .ok()??;
            if projected_revision.is_some_and(|expected| {
                crate::direct_projection::projection_source_revision(
                    &content_rev(&content),
                    config_digest,
                ) != expected
            }) {
                return None;
            }
            #[cfg(test)]
            self.page_build_test
                .on_demand_parses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (effective, document, _) =
                isolate_page_parse(entry, &self.journal_format, |entry| {
                    Some(self.parse_session_page_content(entry, &content))
                })
                .ok()??;
            pages.push((effective, Arc::new(document)));
        }
        #[cfg(test)]
        DIRECT_HYDRATED_PAGES.with(|recorded| {
            recorded.borrow_mut().extend(
                pages
                    .iter()
                    .map(|(entry, _)| PathBuf::from(&entry.rel_path)),
            );
        });
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(pages)
    }

    pub(super) fn direct_projection_property_facets(
        &self,
        autocomplete: bool,
        max_items: usize,
        max_bytes: usize,
    ) -> Option<(Vec<(String, Vec<String>)>, bool)> {
        self.indexed_read(|projection, generation| {
            projection.property_facets(
                generation,
                autocomplete,
                &self.config.block_hidden_properties,
                max_items,
                max_bytes,
            )
        })
    }

    /// The §6.2 registry row source when the Direct Files projection is READY
    /// (CLOSURE §4): the shared raw property stream and its same-snapshot page
    /// map, or `None` when the projection is not ready or the read refused.
    ///
    /// The generation is re-checked after the read for the same reason every
    /// other projection reader re-checks it: a snapshot that straddles a
    /// rebuild is not a snapshot.
    #[cfg(test)]
    pub(super) fn direct_projection_property_owner_rows(
        &self,
    ) -> Option<(
        Vec<crate::query::registry::OwnerRow>,
        std::collections::HashMap<String, crate::query::registry::PageMeta>,
    )> {
        self.indexed_read(|projection, generation| projection.property_owner_rows(generation))
    }
}

/// A scheduled launch warm; see `Graph::announce_launch_warm`.
pub struct LaunchWarmAnnouncement(
    #[allow(dead_code)] Option<crate::direct_projection::WarmInFlight>,
);
