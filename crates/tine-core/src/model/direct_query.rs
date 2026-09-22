//! The Direct query dispatcher and its projection reads: simple and statement
//! pre-views, dispatch_direct_query, export and publication readers, IR results
//! and explain, statement page rows, failed-read recovery, the reference-family
//! reads, and the projection's test-only accessors.

use super::*;

const BACKLINK_FILTER_EQUIVALENCE_SQL: &str = "WITH RECURSIVE component(name_key) AS (
    SELECT ?1
    UNION
    SELECT alias_name.key
    FROM component
    JOIN names AS owner_name ON owner_name.key = component.name_key
    JOIN pages AS owner INDEXED BY pages_name_idx
      ON owner.name_id = owner_name.name_id
    JOIN reference_alias_declarations AS declaration
      INDEXED BY reference_alias_declarations_source_idx
      ON declaration.source_page_id = owner.page_id
    JOIN names AS alias_name ON alias_name.name_id = declaration.alias_name_id
    UNION
    SELECT owner_name.key
    FROM component
    JOIN names AS alias_name ON alias_name.key = component.name_key
    JOIN reference_alias_declarations AS declaration
      INDEXED BY reference_alias_declarations_name_idx
      ON declaration.alias_name_id = alias_name.name_id
    JOIN pages AS owner ON owner.page_id = declaration.source_page_id
    JOIN names AS owner_name ON owner_name.name_id = owner.name_id
)
SELECT component.name_key, page.path,
       CASE WHEN page.page_id IS NULL THEN NULL ELSE page_name.raw END,
       alias_name.key
FROM component
LEFT JOIN names AS page_name ON page_name.key = component.name_key
LEFT JOIN pages AS page INDEXED BY pages_name_idx
  ON page.name_id = page_name.name_id
LEFT JOIN reference_alias_declarations AS declaration
  INDEXED BY reference_alias_declarations_source_idx
  ON declaration.source_page_id = page.page_id
LEFT JOIN names AS alias_name ON alias_name.name_id = declaration.alias_name_id
ORDER BY component.name_key, page.path, alias_name.key";

const BACKLINK_FILTER_JOURNAL_SQL: &str = "SELECT page_name.raw
FROM names AS page_name
JOIN pages AS page INDEXED BY pages_name_idx ON page.name_id = page_name.name_id
WHERE page_name.key = ?1 AND page.text_kind = 1
ORDER BY page.position
LIMIT 1";

const BACKLINK_FILTER_SOURCE_BATCH: usize = 256;

impl Graph {
    /// **SPEC §5.9's Direct Files dispatch, in ONE place.**
    ///
    /// Every route out of a simple or advanced query goes through here — ready,
    /// not ready, busy, failed, cancelled — so that a future arm cannot answer
    /// a public query with anything but the projection. That is not a stylistic
    /// preference: two earlier arms fell back with a bare
    /// `map_or_else`/`unwrap_or_else` and never called `note_fallback_read`, so
    /// a FAILED read scheduled no recovery and the projection could sit
    /// unusable until the next save.
    ///
    /// **RET2 retired the walk from this route.** Martin, 2026-09-07: the
    /// in-memory walk "needs to be only present as an oracle and needs to be
    /// retired as soon as we are confident the sqlite works". The old Q18
    /// waiting requirement is overridden. What replaces each arm is
    /// [`Graph::dispatch_direct_query`]: one bounded repair, then a typed
    /// `query::QueryExecutionError`, never a second engine and never a
    /// fabricated empty answer.
    pub(super) fn direct_simple_query_pre_view(
        &self,
        query: &crate::query::ir::Query,
        today: crate::date::JournalDate,
        max_rows: usize,
        max_bytes: usize,
        profile: crate::query::ConstructionProfile,
        view: Option<&crate::query::ir::ViewSettings>,
    ) -> Result<crate::query::PreViewGroups, crate::query::QueryExecutionError> {
        self.dispatch_direct_query(|request| {
            self.direct_projection_statement_pre_view(
                request, query, today, max_rows, max_bytes, profile, view,
            )
        })
    }

    /// **The ONE Direct query dispatcher (RET2).**
    ///
    /// It is generic over the answer so that adding a row shape cannot add a
    /// route: `@block` pre-view groups, `@page` rows and an explanation's probe
    /// counts take exactly these arms, once, here.
    ///
    /// `attempt` is a CALLABLE statement attempt rather than an already-computed
    /// state, and that is load-bearing: it returns an owned answer, so by the
    /// time this function can decide to repair, every snapshot handle the failed
    /// attempt opened has been dropped. The old code could not repair-then-retry
    /// for that exact reason and walked instead.
    ///
    /// The arms:
    ///
    /// * **answered** — the statement answers. Full stop: there is no second
    ///   route and no cost test in front of it (§5.9's first line, "exactly
    ///   today's shapes … no third route", and §5.10's refusal of "a new walk
    ///   route" for the transient FTS-building class). A slow unbounded shape is
    ///   a reason to make the statement faster or to add an index, never a
    ///   reason to keep a second engine alive. `SqlQuery::positively_bounded`
    ///   stays §5.7's plan-gate concept and a diagnostic; it is not an input.
    /// * **cancelled** — a drain, a rebuild or a graph close took the snapshot
    ///   away. It NEVER repairs and never retries: the read has no subject any
    ///   more, and repairing on a cancellation would turn every rebuild into a
    ///   second rebuild.
    /// * **unavailable** — no projection is attached (an export, a headless
    ///   tool, a graph opened without one), or the lowering refused the shape.
    ///   No repair can produce a database that was never attached, so it is a
    ///   bounded typed error and not an invitation to parse the graph.
    /// * **busy** — capacity admission refused. Work is progressing; retry.
    /// * **not ready** — [`crate::direct_projection::ProjectionProgress`]
    ///   decides. Working → retryable `NotReady`. Stopped → bounded
    ///   `Unavailable`, because a worker that is gone will never pick up a
    ///   repair and a retry loop against it would never end. Stale → repair.
    /// * **failed read** — repair. **In-scope scenario** (AGENTS §5): a torn or
    ///   truncated projection file after a crash or power loss, a disk error, or
    ///   a projection whose page set has drifted from the parsed cache. The
    ///   projection is disposable derived state (D-3), so the answer is recovery
    ///   — but the recovery is now the SQL retry, not a walk.
    ///
    /// Exactly ONE repair per request. After it, a still-unready projection is
    /// reported as `NotReady(Recovering)` only while the queue is actually
    /// working; a repair that did not take is `Unavailable`, so a permanently
    /// broken projection cannot drive the frontend's retry loop forever.
    pub(super) fn dispatch_direct_query<T>(
        &self,
        attempt: impl Fn(&DirectQueryRequest) -> DirectAttempt<T>,
    ) -> Result<T, crate::query::QueryExecutionError> {
        use crate::direct_projection::ProjectionProgress;
        use crate::query::{
            QueryExecutionError as Error, QueryReadinessReason as Readiness,
            QueryUnavailableReason as Reason,
        };
        // A lifecycle drain invalidates this captured request. Hand a live
        // surface a readiness retry so its NEXT request captures the new
        // incarnation; never recapture a moving target inside this request.
        // Capture the lifecycle identity once, before recovery. Ordinary saves
        // keep the same epoch and do not cancel the request.
        let request = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(|projection| (Arc::clone(projection), projection.query_epoch()));
        let cancelled = || {
            let replaced = request
                .as_ref()
                .is_some_and(|(projection, epoch)| projection.query_epoch() != *epoch);
            if !replaced {
                return Error::Cancelled;
            }
            match self.direct_projection_progress() {
                Some(ProjectionProgress::Ready) => Error::NotReady(Readiness::PendingEdits),
                Some(ProjectionProgress::Working(reason)) => Error::NotReady(reason),
                _ => Error::Cancelled,
            }
        };
        let attempt_captured = || {
            if request
                .as_ref()
                .is_some_and(|(projection, epoch)| projection.query_epoch() != *epoch)
            {
                return DirectAttempt::Cancelled;
            }
            let result = attempt(&request);
            if request
                .as_ref()
                .is_some_and(|(projection, epoch)| projection.query_epoch() != *epoch)
            {
                DirectAttempt::Cancelled
            } else {
                result
            }
        };
        // Whether the repair must ERASE the disposable database before
        // rebuilding it. A failed read owes that; an idle projection that has
        // simply not started yet does not, and erasing it there discarded the
        // whole persisted index on every launch (GH #543).
        let reset_before_rebuild = match attempt_captured() {
            DirectAttempt::Answered(answer) => return Ok(answer),
            DirectAttempt::Cancelled => return Err(cancelled()),
            DirectAttempt::Unavailable(reason) => return Err(Error::Unavailable(reason)),
            DirectAttempt::Busy => return Err(Error::NotReady(Readiness::Busy)),
            DirectAttempt::NotReady => match self.direct_projection_progress() {
                // A save landed between the readiness test and this one.
                Some(ProjectionProgress::Ready) => {
                    return Err(Error::NotReady(Readiness::PendingEdits))
                }
                Some(ProjectionProgress::Working(reason)) => return Err(Error::NotReady(reason)),
                Some(ProjectionProgress::Stopped) | None => {
                    return Err(Error::Unavailable(Reason::ProjectionUnavailable))
                }
                // Idle and stale: nothing is coming, so repair. The repair
                // still resets when the worker actually failed; it validates
                // when the projection is merely not started yet.
                Some(ProjectionProgress::Stale) => false,
            },
            DirectAttempt::FailedRead(_) => true,
        };
        #[cfg(test)]
        if reset_before_rebuild {
            // Bind first: an `if let` scrutinee would hold the guard across
            // the pause.
            let pause = self
                .page_build_test
                .failed_read_repair_pause
                .lock()
                .unwrap()
                .take();
            if let Some(pause) = pause {
                pause.reached.wait();
                pause.release.wait();
            }
        }
        if reset_before_rebuild {
            self.direct_projection_recover_after_failed_read();
        } else {
            self.direct_projection_repair(false);
        }
        match attempt_captured() {
            DirectAttempt::Answered(answer) => Ok(answer),
            DirectAttempt::Cancelled => Err(cancelled()),
            DirectAttempt::Unavailable(reason) => Err(Error::Unavailable(reason)),
            DirectAttempt::Busy => Err(Error::NotReady(Readiness::Busy)),
            DirectAttempt::FailedRead(reason) => Err(Error::Unavailable(reason)),
            DirectAttempt::NotReady => match self.direct_projection_progress() {
                Some(ProjectionProgress::Ready) => Err(Error::NotReady(Readiness::PendingEdits)),
                Some(ProjectionProgress::Working(reason)) => Err(Error::NotReady(reason)),
                Some(ProjectionProgress::Stopped) | None => {
                    Err(Error::Unavailable(Reason::ProjectionUnavailable))
                }
                // The repair did not take: another repair holds the recovery
                // lock, or the enqueue was refused. Retrying cannot be the
                // answer twice in a row for the same request.
                Some(ProjectionProgress::Stale) => Err(Error::Unavailable(Reason::ReadFailed)),
            },
        }
    }

    /// The readiness lifecycle of the attached projection, or `None` when no
    /// projection is attached at all.
    pub(super) fn direct_projection_progress(
        &self,
    ) -> Option<crate::direct_projection::ProjectionProgress> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        Some(projection.progress_at(generation))
    }

    /// The statement half of [`Graph::direct_simple_query_pre_view`]: lower,
    /// run, hydrate. It performs no repair of its own — it only reports which
    /// §5.9 state it reached, so the classification and the recovery live in
    /// exactly one place, and it returns an OWNED answer so the dispatcher can
    /// call it again after every handle it opened is gone.
    fn direct_projection_statement_pre_view(
        &self,
        request: &DirectQueryRequest,
        query: &crate::query::ir::Query,
        today: crate::date::JournalDate,
        max_rows: usize,
        max_bytes: usize,
        profile: crate::query::ConstructionProfile,
        view: Option<&crate::query::ir::ViewSettings>,
    ) -> DirectAttempt<crate::query::PreViewGroups> {
        use crate::query::sql::{lower_query, LoweringInputs, RELATION_RULE, RESULT_SET_RULE};
        if query.is_invalid() {
            // A refused source never executes; this is a semantic answer,
            // independent of database readiness, not an availability failure.
            return DirectAttempt::Answered(crate::query::PreViewGroups::default());
        }
        let Some((projection, _)) = request.as_ref() else {
            return DirectAttempt::Unavailable(
                crate::query::QueryUnavailableReason::ProjectionUnavailable,
            );
        };
        let registry_sensitivity = if query.filter.has_props_leaf() {
            crate::direct_projection::RegistrySensitivity::Required
        } else {
            crate::direct_projection::RegistrySensitivity::Insensitive
        };
        let mut job = match projection.open_current_query_job(registry_sensitivity) {
            crate::direct_projection::QueryJobOpen::Job(job) => job,
            crate::direct_projection::QueryJobOpen::NotReady => return DirectAttempt::NotReady,
            crate::direct_projection::QueryJobOpen::Busy => return DirectAttempt::Busy,
            crate::direct_projection::QueryJobOpen::Failed => {
                return DirectAttempt::FailedRead(crate::query::QueryUnavailableReason::ReadFailed)
            }
            crate::direct_projection::QueryJobOpen::Cancelled => return DirectAttempt::Cancelled,
        };
        let config = Arc::clone(&job.config);
        let registry = if query.filter.has_props_leaf() {
            // §6.2's metadata for THIS answer, read SQL-only from the job's own
            // owned snapshot. A failed read is a failed read: it does not
            // degrade to the cached or the empty registry, which would publish
            // an answer under types nobody declared.
            match self.query_property_registry_at(&mut job) {
                Ok(registry) => registry,
                Err(crate::query::QueryExecutionError::Cancelled) => {
                    return DirectAttempt::Cancelled
                }
                Err(crate::query::QueryExecutionError::Unavailable(reason)) => {
                    return DirectAttempt::FailedRead(reason)
                }
                Err(crate::query::QueryExecutionError::NotReady(_)) => {
                    return DirectAttempt::NotReady
                }
            }
        } else {
            // No property predicate can observe registry types in this plan.
            Arc::new(crate::query::registry::Registry::empty(&config))
        };
        let compiled = crate::query::compiled::CompiledLeaves::for_query(&query.evaluable_filter());
        let inputs = LoweringInputs {
            today,
            registry: &registry,
            // NOT `max_rows`: the walk reports `total` as the number of matches
            // it SAW, not the number it admitted, so a `LIMIT` in the statement
            // would silently truncate the count the user is shown. The bounds
            // are charged during hydration instead, by the same
            // `ConstructionBudget` the walk charges.
            cutoff: None,
            compiled: &compiled,
            result_set_rule: RESULT_SET_RULE,
            relation_rule: RELATION_RULE,
        };
        // §5.9: when the projection is ready, the statement answers. The
        // compiler is TOTAL — it has no "unsupported" answer to return — so
        // there is no shape-based route left to take, and
        // `statement.positively_bounded` is deliberately NOT consulted either:
        // it is §5.7's plan-gate concept, and reading it here would be the
        // fourth route ("ready but unbounded") that §5.9's first line and §5.10
        // both forbid. Should a later relation family stop being lowerable, its
        // route is `DirectAttempt::Unavailable(UnsupportedRelation)` — never a
        // fabricated empty answer and never a switch to the evaluator.
        let statement = lower_query(query, &inputs);
        // A filter that folded to false — an invalid query's zero results
        // (§3.5), or a leaf that can never hold — has an answer already. Reading
        // the projection to be told `0 rows` is the same answer at the price of
        // a round trip (I-15).
        if statement.matches_nothing && view.is_none() {
            return DirectAttempt::Answered(crate::query::PreViewGroups::default());
        }
        // R3: the answer is constructed from ONE owned read snapshot of the
        // projection and nothing else — no page document, no parsed cache
        // (§2B). The job owns capacity, the snapshot pinned at this generation,
        // and the identity capture; `read_results` owns the descriptor read,
        // the budget and the payload batches, and installs the statement's
        // compiled-regex program on the job's own connection.
        // The identity policy, captured with the snapshot: a page THIS process
        // lowered answers with its stored live id; a row reused from an earlier
        // session answers with the structural id the fresh parse assigns it.
        let identity = crate::query::results::ResultIdentity {
            session_pages: Arc::clone(&job.session_pages),
            all_session: false,
        };
        // The recency axis is the WALK'S producer, by the two inputs the
        // projection stores; it runs only for a page the answer admitted.
        let recency = |page: crate::query::results::RecencyPage<'_>| {
            crate::query::page_recency_secs_for(page.journal_day, &self.root.join(page.path))
        };
        let inputs = crate::query::results::ResultReadInputs {
            statement: &statement,
            identity: &identity,
            max_rows,
            max_bytes,
            profile,
            recency: &recency,
        };
        let read = match view {
            // The page-recency programs are the SQL ordering axis's, so they
            // are built only on the branch that orders in SQL.
            Some(view) => {
                let root = self.root.clone();
                let page_recency = crate::query::rank::PageRecencyPrograms::new(
                    |day| {
                        crate::query::page_recency_secs_for(
                            day.parse().ok(),
                            std::path::Path::new(""),
                        )
                    },
                    move |path| crate::query::page_recency_secs_for(None, &root.join(path)),
                );
                crate::query::results::read_ordered_results(
                    &mut job.snapshot,
                    &inputs,
                    view,
                    &page_recency,
                )
            }
            None => crate::query::results::read_results(&mut job.snapshot, &inputs),
        };
        let mut pre = match read {
            Ok(pre) => pre,
            Err(error) => return direct_attempt_from_read(Err(error.into())),
        };
        if !pre.ordered {
            crate::query::base_order_groups(&mut pre.groups);
        }
        if job.snapshot.cancellation().is_cancelled() {
            return DirectAttempt::Cancelled;
        }
        DirectAttempt::Answered(pre)
    }

    /// Export selected query subtrees from the current committed projection.
    /// Preparation and construction are shared with every backend adapter.
    pub fn export_query_subtrees(
        &self,
        specs: &[crate::query::QueryExportSpec],
        max_queries: usize,
        max_roots: usize,
        max_nodes: usize,
        max_bytes: usize,
    ) -> Result<crate::query::QueryExportBatch, crate::query::QueryExecutionError> {
        use crate::query::export_execute::{ExportExecutionInputs, PreparedExportBatch};
        let prepared =
            PreparedExportBatch::prepare(specs, max_queries, crate::date::JournalDate::today());
        if let Some(answer) = prepared.all_refused_result(max_roots) {
            return Ok(answer);
        }
        let sensitivity = if prepared.requires_registry() {
            crate::direct_projection::RegistrySensitivity::Required
        } else {
            crate::direct_projection::RegistrySensitivity::Insensitive
        };
        self.dispatch_direct_query(|request| {
            self.direct_projection_read_job(request, sensitivity, |job| {
                let registry = self.direct_lowering_registry(prepared.requires_registry(), job)?;
                let identity = crate::query::results::ResultIdentity {
                    session_pages: Arc::clone(&job.session_pages),
                    all_session: false,
                };
                let recency = |page: crate::query::results::RecencyPage<'_>| {
                    crate::query::page_recency_secs_for(
                        page.journal_day,
                        &self.root.join(page.path),
                    )
                };
                prepared
                    .execute(
                        &mut job.snapshot,
                        &ExportExecutionInputs {
                            registry: &registry,
                            identity: &identity,
                            recency: &recency,
                            max_roots,
                            max_nodes,
                            max_bytes,
                        },
                    )
                    .map_err(Into::into)
            })
        })
    }

    /// Bind the static publisher's captured source documents to one main image.
    /// A byte/config mismatch is local to this explicit publication command;
    /// ordinary live reads never wait for this correspondence.
    pub(crate) fn with_publication_query_reader<T>(
        &self,
        sources: &[(PageEntry, String)],
        publish: impl FnOnce(&crate::query::read_execute::SnapshotQueryReader<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        use crate::query::rank::PageRecencyPrograms;
        use crate::query::read_execute::{SnapshotQueryInputs, SnapshotQueryReader};
        use crate::query::results::{RecencyPage, ResultIdentity};
        // The dispatcher can repair a failed acquisition before publication
        // starts. Once the callback starts, its IO outcome is final; the writer
        // is never re-entered as a query retry.
        let publish = std::cell::RefCell::new(Some(publish));
        self.dispatch_direct_query(|request| {
            self.direct_projection_read_job(
                request,
                crate::direct_projection::RegistrySensitivity::Required,
                |job| {
                    if !job.publication_sources_match(sources, &self.config.parse_config())? {
                        return Ok(Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "The graph is still updating. Try publishing again shortly.",
                        )));
                    }
                    let registry = self.direct_lowering_registry(true, job)?;
                    let identity = ResultIdentity {
                        session_pages: Arc::new(HashSet::new()),
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
                            today: crate::date::JournalDate::today(),
                        },
                    )?;
                    Ok(publish
                        .borrow_mut()
                        .take()
                        .expect("publication callback runs once")(
                        &reader
                    ))
                },
            )
        })
        .map_err(io::Error::other)?
    }

    /// **SPEC §7.1 `query_run`'s Direct Files execution** (RET1).
    ///
    /// The public IR command used to hand `GraphQueryPages` to the shared
    /// result driver, so a ready warm graph answered a real result by walking
    /// the parsed graph and read no statement at all. It now takes exactly the
    /// same two routes the `{{query …}}` render path takes: `@block` through
    /// the shared pre-view dispatch, `@page` through the page statement and the
    /// shared page read.
    ///
    /// **Post-resolution only.** `resolved` is the bound tree and its ONE
    /// execution-day snapshot (§4.4), so `?current-page` and `:today` are
    /// resolved once before lowering. The support report is NOT attached here:
    /// it is a property of how this source was bound, not of the rows
    /// (`query::run_query_result_ir` attaches it).
    pub(crate) fn direct_ir_query_result(
        &self,
        resolved: &crate::query::ResolvedQuery,
        view: &crate::query::ir::ViewSettings,
        bounds: crate::query::ir::Bounds,
    ) -> Result<crate::query::ir::QueryResult, crate::query::QueryExecutionError> {
        let query = resolved.query();
        let execution_view = crate::query::view::statistics_execution_view(query, view);
        let view = &execution_view;
        let mut result = crate::query::ir::QueryResult {
            statistics: None,
            rows: crate::query::ir::QueryRows::Page { pages: Vec::new() },
            diagnostics: query.diagnostics.clone(),
            report: crate::query::ir::QueryReport {
                ran: Vec::new(),
                ignored: Vec::new(),
                supported: true,
            },
            total: 0,
            matched_total: (query.anchor == crate::query::ir::Anchor::Page).then_some(0),
            exceeded: false,
        };
        if query.anchor == crate::query::ir::Anchor::Page {
            let answer = self.direct_page_rows(query, view, resolved.today(), bounds)?;
            result.total = answer.total;
            result.statistics = answer.statistics;
            result.matched_total = Some(answer.matched_total);
            result.exceeded = answer.exceeded;
            result.rows = crate::query::ir::QueryRows::Page {
                pages: answer.pages,
            };
            return Ok(result);
        }
        // The block-anchored tree both engines evaluate — the same
        // `block_anchored_query` rebase `run_query_bounded` lowers through.
        let query = crate::query::block_anchored_query(query);
        let bounded = self.direct_query_view(
            &query,
            view,
            resolved.today(),
            bounds.max_rows,
            bounds.max_bytes,
        )?;
        result.total = bounded.total;
        result.statistics = bounded.statistics;
        result.matched_total = bounded.matched_total;
        result.exceeded = bounded.exceeded;
        result.rows = crate::query::ir::QueryRows::Block {
            groups: Arc::try_unwrap(bounded.groups)
                .unwrap_or_else(|groups| groups.as_ref().clone()),
        };
        Ok(result)
    }

    /// **SPEC §7.1 `query_explain_empty`'s Direct Files execution** (RET1).
    ///
    /// The decomposition, the printing and the report are `query::view`'s and
    /// are shared with the oracle; only the counting is here. Every probe of one
    /// explanation is counted from ONE owned snapshot, so the conjunct counts
    /// and the whole-query answer describe the same graph state rather than N
    /// separately-timed reads.
    pub(crate) fn direct_ir_explain_empty(
        &self,
        resolved: &crate::query::ResolvedQuery,
        view: &crate::query::ir::ViewSettings,
        bounds: crate::query::ir::Bounds,
    ) -> Result<crate::query::ir::ExplainEmptyResult, crate::query::QueryExecutionError> {
        let plan = crate::query::view::explain_empty_plan(resolved);
        if plan.probes.is_empty() {
            // A refused binding has nothing honest to count. The empty plan and
            // the empty count vector agree, which is what `answer` validates.
            return plan.answer(resolved, &[]).map_err(|_| {
                crate::query::QueryExecutionError::Unavailable(
                    crate::query::QueryUnavailableReason::InvalidSnapshot,
                )
            });
        }
        let today = resolved.today();
        let counts = self.dispatch_direct_query(|request| {
            self.direct_projection_statement_probe_counts(
                request,
                &plan.probes,
                view,
                today,
                bounds,
            )
        })?;
        // The evaluator answered a plan it was not given: that is a projection
        // that contradicts itself, not a row that reads as zero (RET2 §5).
        plan.answer(resolved, &counts).map_err(|_| {
            crate::query::QueryExecutionError::Unavailable(
                crate::query::QueryUnavailableReason::InvalidSnapshot,
            )
        })
    }

    /// §5.9's dispatch for an `@page` answer: the page statement, or a typed
    /// execution error. There is no page walk behind it any more (RET2).
    fn direct_page_rows(
        &self,
        query: &crate::query::ir::Query,
        view: &crate::query::ir::ViewSettings,
        today: crate::date::JournalDate,
        bounds: crate::query::ir::Bounds,
    ) -> Result<crate::query::results::PageAnswer, crate::query::QueryExecutionError> {
        self.dispatch_direct_query(|request| {
            self.direct_projection_statement_page_rows(request, query, view, today, bounds)
        })
    }

    /// Own admission and one current read job; callers supply only their reads.
    pub(super) fn direct_projection_read_job<T>(
        &self,
        request: &DirectQueryRequest,
        registry_sensitivity: crate::direct_projection::RegistrySensitivity,
        read: impl FnOnce(
            &mut crate::direct_projection::DirectQueryJob,
        ) -> Result<T, crate::query::QueryExecutionError>,
    ) -> DirectAttempt<T> {
        let Some((projection, _)) = request.as_ref() else {
            return DirectAttempt::Unavailable(
                crate::query::QueryUnavailableReason::ProjectionUnavailable,
            );
        };
        let mut job = match projection.open_current_query_job(registry_sensitivity) {
            crate::direct_projection::QueryJobOpen::Job(job) => job,
            crate::direct_projection::QueryJobOpen::NotReady => return DirectAttempt::NotReady,
            crate::direct_projection::QueryJobOpen::Busy => return DirectAttempt::Busy,
            crate::direct_projection::QueryJobOpen::Failed => {
                return DirectAttempt::FailedRead(crate::query::QueryUnavailableReason::ReadFailed)
            }
            crate::direct_projection::QueryJobOpen::Cancelled => return DirectAttempt::Cancelled,
        };
        direct_attempt_from_read(read(&mut job))
    }

    /// The §6.2 lowering inputs one Direct execution runs under: ONE registry
    /// snapshot and ONE parse of every `content match` payload, the same values
    /// the oracle reads (I-12). Returned as the owned piece the borrow in
    /// `LoweringInputs` needs.
    ///
    /// SQL-only, from the caller's already-owned job: a failed read is
    /// propagated rather than degraded to the cached or empty table.
    pub(super) fn direct_lowering_registry(
        &self,
        has_properties: bool,
        job: &mut crate::direct_projection::DirectQueryJob,
    ) -> Result<Arc<crate::query::registry::Registry>, crate::query::QueryExecutionError> {
        let config = Arc::clone(&job.config);
        if !has_properties {
            return Ok(Arc::new(crate::query::registry::Registry::empty(&config)));
        }
        self.query_property_registry_at(job)
    }

    fn direct_projection_statement_page_rows(
        &self,
        request: &DirectQueryRequest,
        query: &crate::query::ir::Query,
        view: &crate::query::ir::ViewSettings,
        today: crate::date::JournalDate,
        bounds: crate::query::ir::Bounds,
    ) -> DirectAttempt<crate::query::results::PageAnswer> {
        use crate::query::sql::{lower_query, LoweringInputs, RELATION_RULE, RESULT_SET_RULE};
        if query.is_invalid() {
            return DirectAttempt::Answered(crate::query::results::PageAnswer::default());
        }
        let registry_sensitivity = if query.filter.has_props_leaf() {
            crate::direct_projection::RegistrySensitivity::Required
        } else {
            crate::direct_projection::RegistrySensitivity::Insensitive
        };
        self.direct_projection_read_job(request, registry_sensitivity, |job| {
            let registry = self.direct_lowering_registry(query.filter.has_props_leaf(), job)?;
            let compiled =
                crate::query::compiled::CompiledLeaves::for_query(&query.evaluable_filter());
            let statement = lower_query(
                query,
                &LoweringInputs {
                    today,
                    registry: &registry,
                    // Selection stays complete; the page wrapper owns its
                    // post-order limit and `COUNT(*) OVER()` exact count.
                    cutoff: None,
                    compiled: &compiled,
                    result_set_rule: RESULT_SET_RULE,
                    relation_rule: RELATION_RULE,
                },
            );
            // Valid empty selections still fold requested empty statistics.
            let root = self.root.clone();
            let page_recency = crate::query::rank::PageRecencyPrograms::new(
                |day| {
                    crate::query::page_recency_secs_for(
                        day.parse::<i64>().ok(),
                        std::path::Path::new(""),
                    )
                },
                move |path| crate::query::page_recency_secs_for(None, &root.join(path)),
            );
            crate::query::results::read_page_results(
                &mut job.snapshot,
                &crate::query::results::PageReadInputs {
                    statement: &statement,
                    view,
                    max_rows: bounds.max_rows,
                    max_bytes: bounds.max_bytes,
                    recency: &page_recency,
                },
            )
            .map_err(Into::into)
        })
    }

    /// Every probe of one explanation, counted over ONE snapshot.
    ///
    /// The probes share the anchor of the query they decompose, so a `@page`
    /// explanation counts page rows and a `@block` one counts the same `total`
    /// the block budget reports — the identical two producers the answer itself
    /// uses, never a third counting rule.
    fn direct_projection_statement_probe_counts(
        &self,
        request: &DirectQueryRequest,
        probes: &[crate::query::ir::Query],
        view: &crate::query::ir::ViewSettings,
        today: crate::date::JournalDate,
        bounds: crate::query::ir::Bounds,
    ) -> DirectAttempt<Vec<usize>> {
        use crate::query::sql::{lower_query, LoweringInputs, RELATION_RULE, RESULT_SET_RULE};
        let registry_sensitivity = if probes.iter().any(|probe| probe.filter.has_props_leaf()) {
            crate::direct_projection::RegistrySensitivity::Required
        } else {
            crate::direct_projection::RegistrySensitivity::Insensitive
        };
        self.direct_projection_read_job(request, registry_sensitivity, |job| {
            let profile = crate::query::ConstructionProfile::from_view(view);
            // One lowering per probe, all under ONE registry snapshot: a probe that
            // read a different effective type than its siblings would explain a
            // query nobody ran.
            let registry = self.direct_lowering_registry(
                probes.iter().any(|probe| probe.filter.has_props_leaf()),
                job,
            )?;
            let page_anchored = probes
                .first()
                .is_some_and(|probe| probe.anchor == crate::query::ir::Anchor::Page);
            let lowered = probes
                .iter()
                .map(|probe| {
                    let probe = if page_anchored {
                        probe.clone()
                    } else {
                        crate::query::block_anchored_query(probe)
                    };
                    let compiled = crate::query::compiled::CompiledLeaves::for_query(
                        &probe.evaluable_filter(),
                    );
                    lower_query(
                        &probe,
                        &LoweringInputs {
                            today,
                            registry: &registry,
                            cutoff: None,
                            compiled: &compiled,
                            result_set_rule: RESULT_SET_RULE,
                            relation_rule: RELATION_RULE,
                        },
                    )
                })
                .collect::<Vec<_>>();
            let identity = crate::query::results::ResultIdentity {
                session_pages: Arc::clone(&job.session_pages),
                all_session: false,
            };
            let recency = |page: crate::query::results::RecencyPage<'_>| {
                crate::query::page_recency_secs_for(page.journal_day, &self.root.join(page.path))
            };
            let root = self.root.clone();
            let page_recency = crate::query::rank::PageRecencyPrograms::new(
                |day| {
                    crate::query::page_recency_secs_for(
                        day.parse::<i64>().ok(),
                        std::path::Path::new(""),
                    )
                },
                move |path| crate::query::page_recency_secs_for(None, &root.join(path)),
            );
            let page_count_view = crate::query::ir::ViewSettings::default();
            let mut counts = Vec::with_capacity(lowered.len());
            for statement in &lowered {
                if statement.matches_nothing {
                    counts.push(0);
                    continue;
                }
                counts.push(if page_anchored {
                    crate::query::results::read_page_results(
                        &mut job.snapshot,
                        &crate::query::results::PageReadInputs {
                            statement,
                            view: &page_count_view,
                            max_rows: 0,
                            max_bytes: 0,
                            recency: &page_recency,
                        },
                    )
                    .map_err(crate::query::QueryExecutionError::from)?
                    .matched_total
                } else {
                    crate::query::results::read_results(
                        &mut job.snapshot,
                        &crate::query::results::ResultReadInputs {
                            statement,
                            identity: &identity,
                            max_rows: bounds.max_rows,
                            max_bytes: bounds.max_bytes,
                            profile,
                            recency: &recency,
                        },
                    )
                    .map_err(crate::query::QueryExecutionError::from)?
                    .total
                });
            }
            Ok(counts)
        })
    }

    /// §5.9/M9: schedule the recovery a FAILED projection read owes.
    ///
    /// `mark_stale` alone would strand the projection until another edit. Repair
    /// validates a complete source inventory, then captures one parsed snapshot
    /// for a bounded staged build when there is no parsed cache, or reuses an
    /// already-owned parsed snapshot.
    pub(crate) fn direct_projection_recover_after_failed_read(&self) {
        self.direct_projection_repair(true);
    }

    /// The repair itself. `reset` erases the disposable database first.
    ///
    /// Erasing is the repair a torn or unreadable file owes, and it is never
    /// the repair an intact one owes: `reset()` drops every source stamp, so
    /// the complete inventory that follows re-lowers EVERY page. The app runs
    /// queries before its background warm reaches the projection — the journal
    /// feed, backlinks, Ctrl-K — and such a query finds the worker idle,
    /// unvalidated and not yet failed, which `progress_at` reports as `Stale`.
    /// Resetting there threw away a perfectly good persisted index on every
    /// single launch, so search on a large graph sat on "Indexing — waiting for
    /// search to be ready…" for minutes while the app wrote continuously with
    /// nobody touching it (GH #543). A projection that has not failed is
    /// repaired by validating it against the source revisions it already
    /// stores, never by erasing it.
    fn direct_projection_repair(&self, reset: bool) {
        let Ok(_repair) = self.projection_recovery.try_lock() else {
            return;
        };
        crate::direct_projection::projection_diag(|| {
            format!(
                "repair requested reset={reset} parsed_cache={}",
                self.cache.read().unwrap().is_some()
            )
        });
        let (reset, _in_flight) = {
            let projection = self.direct_projection.lock().unwrap();
            let Some(projection) = projection.as_ref() else {
                return;
            };
            if projection.warm_in_flight() {
                // Another thread is already reading this graph's inventory —
                // the open path's warm, or a warm this repair's own earlier
                // pass started. `projection_recovery` serializes repairs but
                // not that warm, so continuing here would run a second warm of
                // the same graph concurrently: both read every page, both
                // compete for the single `warm_outcome` slot, and the loser
                // used to wait for a producer that no longer existed
                // (GH #543). The warm in flight is the payload; let it finish.
                crate::direct_projection::projection_diag(|| {
                    "repair skipped: a warm is already in flight".to_owned()
                });
                return;
            }
            if projection.fresh_build_owns_image() {
                // Concurrent reads on a damaged image all fail, and the query
                // epoch moves only when the rebuild publishes, so a sibling's
                // failure arrives here while the first one's rebuild runs.
                // That build replaces the image whole; requesting another
                // would queue a second complete build behind it (IT-10).
                crate::direct_projection::projection_diag(|| {
                    "repair skipped: a fresh build already replaces this image".to_owned()
                });
                return;
            }
            let reset = reset || projection.worker_failed();
            if reset {
                projection.request_rebuild();
            }
            // Published BEFORE the payload is computed, which takes seconds on
            // a large graph: a query racing this repair must read it as work in
            // progress, not as an idle stale projection nobody is repairing.
            (reset, projection.begin_repair())
        };
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        // Take the revisions WHILE STILL HOLDING the cache read lock. A
        // publisher inserts into `disk_revs` and bumps `cache_gen` under one
        // held cache WRITE lock (`page_cache.rs`), releasing `disk_revs`
        // between the two — so a reader that lets go of the cache lock in
        // between can read the NEW revision and then re-read the OLD
        // generation. The re-check below passes, and stale documents are
        // published stamped with the hash of the current file, which defeats
        // the very comparison a later warm would use to notice they are stale
        // (re-audit A2-N1). Lock order is cache → disk_revs, the same order
        // the publisher takes, so holding both here cannot deadlock.
        let snapshot = self
            .cache
            .read()
            .unwrap()
            .as_ref()
            .map(|pages| (Arc::clone(pages), self.disk_revs.read().unwrap().clone()));
        let Some((pages, revisions)) = snapshot else {
            // A failed read cannot validate the damaged image. Capture one
            // parsed source snapshot and hand it to the unpublished fresh
            // builder. An ordinary not-yet-started image still gets the cheap
            // revision-only validation first and parses only when that says a
            // cold/stale build is required.
            let owned = if reset {
                self.warm_page_cache_cancellable(&|| false)
            } else {
                match self.warm_projection_cancellable(&|| false) {
                    super::page_cache::WarmProjectionOutcome::Owned => true,
                    super::page_cache::WarmProjectionOutcome::Retry => {
                        self.warm_page_cache_cancellable(&|| false)
                    }
                    super::page_cache::WarmProjectionOutcome::Cancelled
                    | super::page_cache::WarmProjectionOutcome::Unavailable => false,
                }
            };
            if !owned {
                if let Some(projection) = self.direct_projection.lock().unwrap().as_ref() {
                    projection.withdraw_rebuild_request();
                }
            }
            return;
        };
        // The pages and revisions are now coherent with each other, but the
        // generation was read before either. Re-read it: if it moved, this pair
        // is older than the queue and must not be published (GH #543). The
        // obligation a `reset` created goes with it, or it would be latched
        // with no payload behind it.
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
            crate::direct_projection::projection_diag(|| {
                "repair snapshot abandoned: generation moved while it was assembled".to_owned()
            });
            if reset {
                if let Some(projection) = self.direct_projection.lock().unwrap().as_ref() {
                    projection.withdraw_rebuild_request();
                }
            }
            return;
        }
        // A reset must be followed by a payload; see `direct_projection_enqueue_full`.
        let source_complete = self.page_index_failures.read().unwrap().is_empty();
        self.direct_projection_enqueue_full(
            generation,
            pages,
            Arc::new(revisions),
            reset,
            source_complete,
        );
    }

    pub(super) fn direct_projection_note_fallback_read(&self) {
        if let Some(projection) = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
        {
            projection.note_fallback_read();
        }
    }

    pub(super) fn direct_projection_referenced_page_names(&self) -> Option<Vec<String>> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        let names = projection.referenced_page_names(generation)?;
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(names)
    }

    /// Load exactly the named pages from the parsed cache, in the order asked
    /// (source-path order, which is what the reference accumulators charge
    /// their budget in). A dispatched query never comes here (R3): its answer is
    /// read from the projection alone, which the test census records.
    fn direct_projection_pages_for_paths(
        &self,
        generation: u64,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        let snapshot = self.cache.read().unwrap().as_ref().map(Arc::clone);
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
            return None;
        }
        let Some(snapshot) = snapshot else {
            // R6: no parsed cache — hydrate exactly the candidates from disk.
            return self.parse_pages_on_demand(generation, paths.into_iter().collect());
        };
        let cache_index = self.cache_index.read().unwrap();
        let cache_index = cache_index.as_ref()?;
        let mut pages = Vec::new();
        for relative in paths {
            let path = self.root.join(&relative);
            let slot = cache_index.by_path.get(&path).copied()?;
            let page = snapshot.get(slot)?;
            if page.0.path != path || page.0.rel_path != relative.to_string_lossy() {
                return None;
            }
            pages.push(page.clone());
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

    fn direct_projection_pages_for_sources(
        &self,
        generation: u64,
        sources: Vec<(PathBuf, String)>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        let cache = self.cache.read().unwrap();
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
            return None;
        }
        let Some(snapshot) = cache.as_ref().map(Arc::clone) else {
            drop(cache);
            return self.parse_pages_on_demand_with_revisions(generation, sources);
        };
        let revisions = self.disk_revs.read().unwrap();
        let config_digest = self.config.parse_config().digest();
        let mut pages = Vec::with_capacity(sources.len());
        for (relative, projected_revision) in sources {
            let path = self.root.join(&relative);
            let source_revision = revisions.get(&path)?;
            if crate::direct_projection::projection_source_revision(source_revision, config_digest)
                != projected_revision
            {
                return None;
            }
            let slot = self.cached_page_index_for_path(&snapshot, &path)?;
            let page = snapshot.get(slot)?;
            if page.0.path != path || page.0.rel_path != relative.to_string_lossy() {
                return None;
            }
            pages.push(page.clone());
        }
        drop(revisions);
        drop(cache);
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

    pub(crate) fn backlink_filter_scope(
        &self,
        target: &str,
        requested_pages: &[(PageKind, String)],
    ) -> Result<crate::query::BacklinkFilterScope, crate::query::QueryExecutionError> {
        use tine_storage::sqlite::PhysicalQueryValue;

        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        self.dispatch_direct_query(|request| {
            self.direct_projection_read_job(
                request,
                crate::direct_projection::RegistrySensitivity::Insensitive,
                |job| {
                    let rows = crate::query::projection_sql::run(
                        &mut job.snapshot,
                        BACKLINK_FILTER_EQUIVALENCE_SQL,
                        &[PhysicalQueryValue::Text(crate::refs::page_key(target))],
                    )
                    .map_err(|error| -> crate::query::QueryExecutionError {
                        crate::query::results::sql_or_cancelled(&job.snapshot, error).into()
                    })?;
                    let mut real_pages = crate::query::RealPageNames::new();
                    let mut aliases = Vec::new();
                    for row in rows {
                        let [PhysicalQueryValue::Text(_), path, name, alias] = row.as_slice()
                        else {
                            return Err(crate::query::QueryExecutionError::Unavailable(
                                crate::query::QueryUnavailableReason::InvalidSnapshot,
                            ));
                        };
                        match (path, name, alias) {
                            (
                                PhysicalQueryValue::Text(path),
                                PhysicalQueryValue::Text(name),
                                PhysicalQueryValue::Text(alias),
                            ) => {
                                let key = crate::refs::page_key(name);
                                let path = PathBuf::from(path);
                                match real_pages.get_mut(&key) {
                                    Some((winner_path, winner_name)) if path < *winner_path => {
                                        *winner_path = path;
                                        *winner_name = name.clone();
                                    }
                                    Some(_) => {}
                                    None => {
                                        real_pages.insert(key, (path, name.clone()));
                                    }
                                }
                                aliases.push((alias.clone(), name.clone()));
                            }
                            (
                                PhysicalQueryValue::Text(path),
                                PhysicalQueryValue::Text(name),
                                PhysicalQueryValue::Null,
                            ) => {
                                let key = crate::refs::page_key(name);
                                let path = PathBuf::from(path);
                                match real_pages.get_mut(&key) {
                                    Some((winner_path, winner_name)) if path < *winner_path => {
                                        *winner_path = path;
                                        *winner_name = name.clone();
                                    }
                                    Some(_) => {}
                                    None => {
                                        real_pages.insert(key, (path, name.clone()));
                                    }
                                }
                            }
                            (
                                PhysicalQueryValue::Null,
                                PhysicalQueryValue::Null,
                                PhysicalQueryValue::Null,
                            ) => {}
                            _ => {
                                return Err(crate::query::QueryExecutionError::Unavailable(
                                    crate::query::QueryUnavailableReason::InvalidSnapshot,
                                ));
                            }
                        }
                    }
                    let mut resolved = crate::query::equivalent_page_names(
                        &real_pages,
                        &aliases,
                        target,
                    );
                    let format = crate::date::JournalFormat::new(
                        self.config.journal_file_name_format.as_deref(),
                        self.config.journal_page_title_format.as_deref(),
                    );
                    if let Some(target_day) = format.parse(target) {
                        let rows = crate::query::projection_sql::run(
                            &mut job.snapshot,
                            BACKLINK_FILTER_JOURNAL_SQL,
                            &[PhysicalQueryValue::Text(crate::refs::page_key(
                                &format.title(target_day),
                            ))],
                        )
                        .map_err(|error| -> crate::query::QueryExecutionError {
                            crate::query::results::sql_or_cancelled(&job.snapshot, error).into()
                        })?;
                        if let Some(row) = rows.first() {
                            let [PhysicalQueryValue::Text(journal_name)] = row.as_slice() else {
                                return Err(crate::query::QueryExecutionError::Unavailable(
                                    crate::query::QueryUnavailableReason::InvalidSnapshot,
                                ));
                            };
                            crate::query::apply_journal_page_equivalence(
                                &mut resolved,
                                &format,
                                target_day,
                                journal_name,
                            );
                        }
                    }

                    let mut sources = std::collections::BTreeMap::<PathBuf, String>::new();
                    for chunk in requested_pages.chunks(BACKLINK_FILTER_SOURCE_BATCH) {
                        let values = (0..chunk.len())
                            .map(|index| {
                                let first = index * 2 + 1;
                                format!("(?{first}, ?{})", first + 1)
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let sql = format!(
                            "WITH requested(text_kind, name_key) AS (VALUES {values}) \
                             SELECT page.path, revision.revision \
                             FROM requested \
                             JOIN names AS page_name ON page_name.key = requested.name_key \
                             JOIN pages AS page INDEXED BY pages_name_idx \
                               ON page.name_id = page_name.name_id \
                              AND page.text_kind = requested.text_kind \
                             LEFT JOIN direct_source_revisions AS revision \
                               ON revision.path = page.path \
                             ORDER BY page.path"
                        );
                        let mut parameters = Vec::with_capacity(chunk.len() * 2);
                        for (kind, name) in chunk {
                            parameters.push(PhysicalQueryValue::Integer(match kind {
                                PageKind::Page => 0,
                                PageKind::Journal => 1,
                            }));
                            parameters.push(PhysicalQueryValue::Text(name.clone()));
                        }
                        let rows = crate::query::projection_sql::run(
                            &mut job.snapshot,
                            &sql,
                            &parameters,
                        )
                        .map_err(|error| -> crate::query::QueryExecutionError {
                            crate::query::results::sql_or_cancelled(&job.snapshot, error).into()
                        })?;
                        for row in rows {
                            let [PhysicalQueryValue::Text(path), PhysicalQueryValue::Text(revision)] =
                                row.as_slice()
                            else {
                                return Err(crate::query::QueryExecutionError::Unavailable(
                                    crate::query::QueryUnavailableReason::InvalidSnapshot,
                                ));
                            };
                            sources.insert(PathBuf::from(path), revision.clone());
                        }
                    }
                    let pages = self
                        .direct_projection_pages_for_sources(
                            generation,
                            sources.into_iter().collect(),
                        )
                        .ok_or(crate::query::QueryExecutionError::NotReady(
                            crate::query::QueryReadinessReason::PendingEdits,
                        ))?;
                    Ok(crate::query::BacklinkFilterScope {
                        names_norm: resolved.1,
                        pages,
                    })
                },
            )
        })
    }

    pub(super) fn direct_projection_page_aliases_with_owners(
        &self,
    ) -> Option<Vec<(String, String, String)>> {
        self.indexed_read(|projection, generation| projection.page_aliases_with_owners(generation))
    }

    pub(super) fn direct_projection_real_page_names(&self) -> Option<crate::query::RealPageNames> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        let mut names = projection.real_page_names(generation)?;
        for (path, _) in names.values_mut() {
            *path = self.root.join(&*path);
        }
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(names)
    }

    pub(super) fn direct_projection_reference_candidate_pages(
        &self,
        names_norm: &[String],
        self_page: &str,
        kind: ReferenceKind,
        mode: crate::query::candidate::CandidateMode,
    ) -> Option<(
        Vec<(PageEntry, Arc<Document>)>,
        Option<std::collections::HashSet<String>>,
        Option<std::collections::HashSet<PathBuf>>,
    )> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        if !projection.wait_for_reference_generation(generation) {
            return None;
        }
        let candidates = projection.reference_candidates(
            generation,
            names_norm,
            self_page,
            kind,
            mode,
            &self.config,
        )?;
        let pages = self.direct_projection_pages_for_paths(generation, candidates.paths)?;
        Some((pages, candidates.blocks, candidates.page_owners))
    }

    pub(super) fn direct_projection_block_page_hint(&self, uuid: &str) -> Option<Option<String>> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        let hint = projection.block_page_hint(generation, uuid)?;
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(hint)
    }

    pub(super) fn direct_projection_block_ref_counts(
        &self,
    ) -> Option<std::collections::HashMap<String, usize>> {
        self.indexed_read(|projection, generation| projection.block_ref_counts(generation))
    }

    pub(crate) fn direct_projection_block_referrer_candidate_pages(
        &self,
        uuid: &str,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        let paths = projection.block_referrer_candidate_paths(generation, uuid)?;
        self.direct_projection_pages_for_paths(generation, paths)
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_ready_test(&self) -> bool {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|projection| projection.ready_at(generation))
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_mark_stale_test(&self) {
        self.direct_projection_mark_stale();
    }

    /// R3: the attached projection itself, so a test can open a query job on
    /// the same `Arc` the dispatch clones out of this mutex.
    #[cfg(test)]
    pub(crate) fn direct_projection_test(
        &self,
    ) -> Option<Arc<crate::direct_projection::DirectProjection>> {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)
    }

    /// R3: the production rebuild path a failed read takes — request the
    /// rebuild, then enqueue the warm parser snapshot it needs.
    #[cfg(test)]
    pub(crate) fn direct_projection_recover_after_failed_read_test(&self) {
        self.direct_projection_recover_after_failed_read();
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_indexed_reads_test(&self) -> u64 {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |projection| projection.indexed_reads())
    }

    /// §5.9: how many user queries the dispatched statement ANSWERED.
    #[cfg(test)]
    pub(crate) fn direct_projection_statement_reads_test(&self) -> u64 {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |projection| projection.statement_reads())
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_fallback_reads_test(&self) -> u64 {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |projection| projection.fallback_reads())
    }

    /// Reset the two route probes a query test reads: the hydration census and
    /// the full-graph evaluator counter. RET2 retired the third — the candidate
    /// page set — along with the candidate route that populated it.
    #[cfg(test)]
    pub(crate) fn reset_direct_projection_candidate_probe_test(&self) {
        DIRECT_HYDRATED_PAGES.with(|paths| paths.borrow_mut().clear());
        crate::query::reset_full_graph_query_evaluations();
    }

    /// Every page document the projection-side readers loaded from the parsed
    /// cache since the last reset. R3's claim is that a dispatched query
    /// contributes NOTHING here.
    #[cfg(test)]
    pub(crate) fn direct_projection_hydrated_pages_test(&self) -> Vec<std::path::PathBuf> {
        DIRECT_HYDRATED_PAGES.with(|paths| paths.borrow().clone())
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_set_source_revision_test(
        &self,
        path: &std::path::Path,
        revision: &str,
    ) {
        self.disk_revs
            .write()
            .unwrap()
            .insert(path.to_path_buf(), revision.to_string());
    }

    /// §5.9: make the NEXT read through the statement seam fail, as a torn or
    /// truncated projection file, a disk error or a resource limit would.
    #[cfg(test)]
    pub(crate) fn direct_projection_inject_read_failure_test(&self) {
        if let Some(projection) = self.direct_projection.lock().unwrap().as_ref() {
            projection.inject_next_statement_failure();
        }
    }

    #[cfg(test)]
    pub(crate) fn direct_projection_referenced_name_reads_test(&self) -> u64 {
        self.direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |projection| projection.referenced_name_reads())
    }
}
