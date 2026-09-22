//! Graph's query and reference read surface: the property registry, the derived
//! memo, advanced and simple queries, backlinks, block referrers, unlinked
//! references, reference diagnostics, and publish/print HTML.

use super::*;

impl Graph {
    /// The walk oracle's property registry, built from the current rows on
    /// every call: the ready projection's raw stream when it answers, the
    /// parsed documents otherwise. Test-only since K2 (2026-09-15): the product
    /// reads the projection's committed registry
    /// ([`Graph::query_property_registry_current`]) and has no other.
    #[cfg(test)]
    pub(crate) fn property_registry(&self) -> Arc<crate::query::registry::Registry> {
        let config = self.config.parse_config();
        let (rows, pages) = self
            .direct_projection_property_owner_rows()
            .unwrap_or_else(|| crate::query::property_owner_rows(self));
        let registry = crate::query::registry::build_registry(
            rows.into_iter(),
            &|page_id: &str| pages.get(page_id).cloned(),
            &config,
        )
        .expect("every row names a page of its own snapshot");
        Arc::new(registry)
    }

    /// A strict registry for an already-acquired query snapshot. The old UI
    /// debounce and document fallback are not eligible sources for this read.
    pub(crate) fn query_property_registry_at(
        &self,
        job: &mut crate::direct_projection::DirectQueryJob,
    ) -> Result<Arc<crate::query::registry::Registry>, crate::query::QueryExecutionError> {
        if job.snapshot.cancellation().is_cancelled() {
            return Err(crate::query::QueryExecutionError::Cancelled);
        }
        let config = Arc::clone(&job.config);
        job.read_registry(&config)
    }

    /// §6.2's registry for the CURRENT source generation, SQL-only and
    /// fallible (RET2).
    ///
    /// One owned job captures the committed registry owner and SQL image.
    /// Its cache can reuse unchanged metadata without borrowing the editor
    /// registry or its debounce state.
    ///
    /// It is the product's only registry: the document-built one
    /// (`Graph::property_registry`) exists only for the test-only walk oracle.
    pub(crate) fn query_property_registry_current(
        &self,
        _source_generation: u64,
    ) -> Result<Arc<crate::query::registry::Registry>, crate::query::QueryExecutionError> {
        self.dispatch_direct_query(|request| {
            self.direct_projection_read_job(
                request,
                crate::direct_projection::RegistrySensitivity::Required,
                |job| self.query_property_registry_at(job),
            )
        })
    }

    /// **SPEC §7.1 `query_registry`'s Direct Files answer** (RET2).
    ///
    /// The command used to read `Graph::property_registry().snapshot()`, whose
    /// not-ready branch iterates every parsed document. It is the projection's
    /// answer or a typed failure now, exactly like every other public query
    /// read on this backend.
    pub fn query_registry_snapshot_ready(
        &self,
    ) -> Result<crate::query::ir::RegistrySnapshot, crate::query::QueryExecutionError> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        Ok(self.query_property_registry_current(generation)?.snapshot())
    }

    fn derived_memo_bounded(
        &self,
        key: String,
        compute: impl FnOnce() -> crate::query::BoundedGroups,
    ) -> BoundedRefGroups {
        self.derived_memo_entry(key, || DerivedEntry::plain(bounded_ref_groups(compute())))
            .result
    }

    fn derived_memo_bounded_fallible<E>(
        &self,
        key: String,
        compute: impl FnOnce() -> Result<crate::query::BoundedGroups, E>,
    ) -> Result<BoundedRefGroups, E> {
        Ok(self
            .derived_memo_entry_fallible(key, || {
                compute().map(|computed| DerivedEntry::plain(bounded_ref_groups(computed)))
            })?
            .result)
    }

    fn derived_memo_bounded_fallible_if_eligible<E>(
        &self,
        key: String,
        compute: impl FnOnce() -> Result<(crate::query::BoundedGroups, bool), E>,
    ) -> Result<BoundedRefGroups, E> {
        Ok(self
            .derived_memo_entry_fallible_if_eligible(key, || {
                compute().map(|(computed, memo_eligible)| {
                    (
                        DerivedEntry::plain(bounded_ref_groups(computed)),
                        memo_eligible,
                    )
                })
            })?
            .result)
    }

    /// Execute the selection into operation-scoped PRE-VIEW groups, apply this
    /// request's view, and wrap the final groups at the public transport edge.
    pub(super) fn direct_query_view(
        &self,
        query: &crate::query::ir::Query,
        view: &crate::query::ir::ViewSettings,
        today: crate::date::JournalDate,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<BoundedRefGroups, crate::query::QueryExecutionError> {
        let execution_view = crate::query::view::statistics_execution_view(query, view);
        let view = &execution_view;
        let profile = crate::query::ConstructionProfile::from_view(view);
        let pre = self.direct_simple_query_pre_view(
            query,
            today,
            max_rows,
            max_bytes,
            profile,
            Some(view),
        )?;
        let bounded = crate::query::apply_view(pre, view);
        Ok(BoundedRefGroups {
            statistics: bounded.statistics,
            matched_total: bounded.matched_total,
            groups: Arc::new(bounded.groups),
            total: bounded.total,
            exceeded: bounded.exceeded,
        })
    }

    /// The reference-cache identity: `(cache_gen, today, ParseConfig::digest())`.
    fn derived_memo_entry(
        &self,
        key: String,
        compute: impl FnOnce() -> DerivedEntry,
    ) -> DerivedEntry {
        match self.derived_memo_entry_fallible::<std::convert::Infallible>(key, || Ok(compute())) {
            Ok(entry) => entry,
            Err(never) => match never {},
        }
    }

    /// The one memo body. A `compute` that REFUSES (a projection that is only
    /// mid-turn, say) must not be cached: the same key answers normally a
    /// moment later, and a stored refusal would outlive the condition that
    /// produced it until the next cache generation.
    fn derived_memo_entry_fallible<E>(
        &self,
        key: String,
        compute: impl FnOnce() -> Result<DerivedEntry, E>,
    ) -> Result<DerivedEntry, E> {
        self.derived_memo_entry_fallible_if_eligible(key, || compute().map(|result| (result, true)))
    }

    /// The existing memo boundary with result-derived admission. A successful
    /// answer may still be ineligible when it came from a transient fallback;
    /// decide that from the source the query actually used, after the read,
    /// rather than from a racy readiness observation before it.
    fn derived_memo_entry_fallible_if_eligible<E>(
        &self,
        key: String,
        compute: impl FnOnce() -> Result<(DerivedEntry, bool), E>,
    ) -> Result<DerivedEntry, E> {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        let today = crate::date::JournalDate::today().ordinal_key();
        let config_digest = self.config.parse_config().digest();
        {
            let mut g = self.derived_cache.write().unwrap();
            if let Some(dc) = g.as_mut() {
                if dc.gen == gen && dc.today == today && dc.config_digest == config_digest {
                    if let Some((r, _)) = dc.results.get(&key) {
                        let result = r.clone();
                        touch_lru(&mut dc.lru, &key);
                        return Ok(result);
                    }
                }
            }
        }
        let (result, memo_eligible) = compute()?;
        if !memo_eligible || !self.answer_is_complete() {
            return Ok(result);
        }
        let result_bytes = ref_groups_estimated_bytes(result.result.groups.as_slice())
            .saturating_add(result_cache_key_estimated_bytes(&key));
        if result_bytes > DERIVED_CACHE_MAX_ENTRY_BYTES {
            return Ok(result);
        }
        let mut g = self.derived_cache.write().unwrap();
        match g.as_mut() {
            Some(dc) if dc.gen == gen && dc.today == today && dc.config_digest == config_digest => {
                if let Some((_, old_bytes)) = dc
                    .results
                    .insert(key.clone(), (result.clone(), result_bytes))
                {
                    dc.bytes = dc.bytes.saturating_sub(old_bytes);
                }
                dc.bytes = dc.bytes.saturating_add(result_bytes);
                touch_lru(&mut dc.lru, &key);
                prune_result_cache(&mut dc.results, &mut dc.lru, &mut dc.bytes);
            }
            _ => {
                let mut results = std::collections::HashMap::new();
                results.insert(key.clone(), (result.clone(), result_bytes));
                *g = Some(DerivedCache {
                    gen,
                    today,
                    config_digest,
                    results,
                    lru: std::collections::VecDeque::from([key]),
                    bytes: result_bytes,
                });
            }
        }
        Ok(result)
    }

    pub(super) fn derived_memo(
        &self,
        key: String,
        compute: impl FnOnce() -> Vec<RefGroup>,
    ) -> Arc<Vec<RefGroup>> {
        self.derived_memo_bounded(key, || {
            let groups = compute();
            let total = groups.iter().map(|group| group.blocks.len()).sum();
            crate::query::BoundedGroups {
                matched_total: None,
                statistics: None,
                groups,
                total,
                exceeded: false,
            }
        })
        .groups
    }

    pub(super) fn run_advanced_query_cached(
        &self,
        query_src: &str,
        current_page: Option<&str>,
    ) -> Result<Arc<crate::query::AdvancedResult>, crate::query::QueryExecutionError> {
        let (result, _, _) =
            self.advanced_query_cached(query_src, current_page, usize::MAX, usize::MAX)?;
        Ok(Arc::new(result))
    }

    pub fn run_advanced_query_bounded_cached(
        &self,
        query_src: &str,
        current_page: Option<&str>,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<(crate::query::AdvancedResult, bool, usize), crate::query::QueryExecutionError>
    {
        self.advanced_query_cached(query_src, current_page, max_rows, max_bytes)
    }

    /// **SPEC §5.9's Direct Files entry point for an advanced (datalog) query.**
    /// Resolve once, dispatch the resolved normalized IR, and attach the source
    /// clause report after the rows return.
    ///
    /// The advanced route carries no view directives, so its pre-view result IS
    /// its result and the construction profile is the default one.
    fn advanced_query_cached(
        &self,
        query_src: &str,
        current_page: Option<&str>,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<(crate::query::AdvancedResult, bool, usize), crate::query::QueryExecutionError>
    {
        let (query, today, ran, ignored) =
            match crate::query::resolve_advanced_source(query_src, current_page) {
                // A refused source is a SEMANTIC answer (§3.5): it never
                // executes, so it is independent of index availability.
                crate::query::ResolvedAdvanced::Refused(result) => return Ok((result, false, 0)),
                crate::query::ResolvedAdvanced::Executable {
                    query,
                    today,
                    ran,
                    ignored,
                } => (query, today, ran, ignored),
            };
        let query = crate::query::block_anchored_query(&query);
        let profile = crate::query::ConstructionProfile::default();
        let pre =
            self.direct_simple_query_pre_view(&query, today, max_rows, max_bytes, profile, None)?;
        Ok((
            crate::query::AdvancedResult {
                groups: pre.groups,
                ran,
                ignored,
                supported: true,
            },
            pre.exceeded,
            pre.total,
        ))
    }

    /// Backlinks for a page: blocks across the graph that reference it,
    /// grouped by source page. Delegates to the query module (memoized).
    pub fn backlinks(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("b\0{}", crate::refs::normalize(target)), || {
            crate::query::backlinks(self, target)
        })
    }

    pub fn backlinks_bounded(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded(format!("B\0{max_rows}\0{max_bytes}\0{normalized}"), || {
            crate::query::backlinks_bounded(self, target, max_rows, max_bytes)
        })
    }

    /// Linked references for an interactive panel. Same rows, same memo key as
    /// [`Graph::backlinks_bounded`] -- the only difference is that a projection
    /// which is merely mid-turn is reported instead of answered by parsing
    /// every page in the graph.
    pub fn backlinks_bounded_indexed(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<BoundedRefGroups, crate::query::QueryExecutionError> {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded_fallible(
            format!("B\0{max_rows}\0{max_bytes}\0{normalized}"),
            || crate::query::backlinks_bounded_indexed(self, target, max_rows, max_bytes),
        )
    }

    /// Block-level referrers for a block uuid: every block across the graph that
    /// references it, grouped by source page (memoized). Includes same-page
    /// referrers (see `query::block_referrers`).
    pub fn block_referrers(&self, uuid: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("br\0{}", uuid.trim()), || {
            crate::query::block_referrers(self, uuid)
        })
    }

    pub fn block_referrers_bounded(
        &self,
        uuid: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let uuid = uuid.trim();
        self.derived_memo_bounded(format!("R\0{max_rows}\0{max_bytes}\0{uuid}"), || {
            crate::query::block_referrers_bounded(self, uuid, max_rows, max_bytes)
        })
    }

    /// Evaluate a `{{query ...}}` body over the current projection.
    pub fn run_query(
        &self,
        query_src: &str,
    ) -> Result<Arc<Vec<RefGroup>>, crate::query::QueryExecutionError> {
        Ok(self
            .run_query_bounded(query_src, usize::MAX, usize::MAX)?
            .groups)
    }

    /// **SPEC §5.9's Direct Files entry point.** Parse once, dispatch the same
    /// normalized IR to SQL, and apply the view to the operation-scoped result.
    pub fn run_query_bounded(
        &self,
        query_src: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<BoundedRefGroups, crate::query::QueryExecutionError> {
        if !crate::query::query_source_within_limit(query_src)
            || !crate::query::query_nesting_within_limit(query_src)
        {
            // A refused INPUT (§3.5) is a semantic answer, not an availability
            // failure: it is the same empty result at every readiness state.
            return Ok(BoundedRefGroups {
                matched_total: None,
                statistics: None,
                groups: Arc::new(Vec::new()),
                total: 0,
                exceeded: false,
            });
        }
        // §4.4: the ONE execution-day snapshot for this answer, taken here and
        // handed to both the lowering and the walk. A rollover between two
        // halves of one answer is not a thing that can happen.
        let today = crate::date::JournalDate::today();
        let (query, view) = crate::query::parse_query_source(query_src, today);
        // The block-anchored tree both engines evaluate (`block_anchored_query`
        // is the one producer of that rebase).
        let query = crate::query::block_anchored_query(&query);
        self.direct_query_view(&query, &view, today, max_rows, max_bytes)
    }

    /// Evaluate an advanced (datalog-subset) query, returning the matched groups
    /// plus which clauses ran vs were ignored.
    pub fn run_advanced_query(
        &self,
        query_src: &str,
        current_page: Option<&str>,
    ) -> Result<crate::query::AdvancedResult, crate::query::QueryExecutionError> {
        Ok(self
            .run_advanced_query_cached(query_src, current_page)?
            .as_ref()
            .clone())
    }

    /// Unlinked references: plain-text mentions of a page that aren't links
    /// (memoized).
    pub fn unlinked_refs(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("u\0{}", crate::refs::normalize(target)), || {
            crate::query::unlinked_refs(self, target)
        })
    }

    pub fn unlinked_refs_bounded(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> BoundedRefGroups {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded(format!("U\0{max_rows}\0{max_bytes}\0{normalized}"), || {
            crate::query::unlinked_refs_bounded(self, target, max_rows, max_bytes)
        })
    }

    /// Unlinked references for an interactive panel. See
    /// [`Graph::backlinks_bounded_indexed`].
    pub fn unlinked_refs_bounded_indexed(
        &self,
        target: &str,
        max_rows: usize,
        max_bytes: usize,
    ) -> Result<BoundedRefGroups, crate::query::QueryExecutionError> {
        let normalized = crate::refs::normalize(target);
        self.derived_memo_bounded_fallible_if_eligible(
            format!("UI\0{max_rows}\0{max_bytes}\0{normalized}"),
            || {
                crate::query::unlinked_refs_bounded_indexed_with_source(
                    self, target, max_rows, max_bytes,
                )
                .map(|answer| (answer.groups, answer.memo_eligible))
            },
        )
    }

    /// Explicit, uncached target-scoped trace of the exact reference engine.
    /// Intended for local diagnostics; callers must anonymize before export.
    pub fn reference_diagnostics(&self, target: &str) -> ReferenceDiagnostics {
        crate::query::reference_diagnostics(self, target)
    }

    /// Export the whole graph to static HTML under `<root>/publish/`.
    pub fn publish_html(&self) -> io::Result<(String, usize)> {
        crate::publish::publish_graph(self)
    }

    /// Render a single page to a self-contained HTML document for print-to-PDF
    /// (assets inlined, no sidebar/scripts). `Ok(None)` if the page doesn't exist.
    pub fn page_print_html(
        &self,
        name: &str,
        opts: crate::publish::PrintOpts,
    ) -> Result<Option<String>, crate::publish::PrintPreparationError> {
        crate::publish::page_print_html(self, name, opts)
    }

    pub(crate) fn with_print_query_reader<T>(
        &self,
        render: impl FnOnce(
            &crate::query::read_execute::SnapshotQueryReader<'_>,
        ) -> Result<T, crate::publish::PrintPreparationError>,
    ) -> Result<T, crate::publish::PrintPreparationError> {
        use crate::query::rank::PageRecencyPrograms;
        use crate::query::read_execute::{SnapshotQueryInputs, SnapshotQueryReader};
        use crate::query::results::{RecencyPage, ResultIdentity};
        let render = std::cell::RefCell::new(Some(render));
        let today = crate::date::JournalDate::today();
        self.dispatch_direct_query(|request| {
            self.direct_projection_read_job(
                request,
                crate::direct_projection::RegistrySensitivity::Required,
                |job| {
                    let registry = self.direct_lowering_registry(true, job)?;
                    let identity = ResultIdentity {
                        session_pages: Arc::clone(&job.session_pages),
                        all_session: false,
                    };
                    let recency = |page: RecencyPage<'_>| {
                        crate::query::page_recency_secs_for(
                            page.journal_day,
                            &self.root.join(page.path),
                        )
                    };
                    let root = self.root.clone();
                    let page_recency = PageRecencyPrograms::new(
                        |day| {
                            crate::query::page_recency_secs_for(
                                day.parse::<i64>().ok(),
                                Path::new(""),
                            )
                        },
                        move |path| crate::query::page_recency_secs_for(None, &root.join(path)),
                    );
                    let reader = SnapshotQueryReader::new(
                        &mut job.snapshot,
                        SnapshotQueryInputs {
                            registry: &registry,
                            identity: &identity,
                            recency: &recency,
                            page_recency: &page_recency,
                            today,
                        },
                    )?;
                    Ok(render
                        .borrow_mut()
                        .take()
                        .expect("Print renderer runs once")(
                        &reader
                    ))
                },
            )
        })?
    }
}
