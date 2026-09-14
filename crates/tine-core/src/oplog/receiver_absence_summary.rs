use std::collections::{BTreeMap, BTreeSet};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use tine_storage::sealed_accepted_index::{
    AuthenticatedMapKey, AuthenticatedMapLinkV1, AuthenticatedMapRootV1, SealedAcceptedIndexError,
};

use super::receiver_absence_map::{MapNodeObjects, MapReader, MapWriter, MAP_NODE_KIND_CODE};

use super::absence_decision::{
    merge_anchor, restored_generation_relation, AbsenceCompletionAnchor, AbsenceDecisionMap,
    ReceiverAbsenceSummaryEntry, ReceiverHistoryRead, ReceiverHistoryUnavailable,
};
use super::current_action_roots::{
    CurrentActionRoots, CursorConsumer, CursorCoverage, ProjectionActionCursor, UncoveredMark,
    ACTION_NAMESPACE as ABSENCE_NAMESPACE, CURSOR_RESUME_CHUNK_MARKS,
};
use super::object_store::{
    ensure_reconstructible_directory_nofollow, open_dir_nofollow, read_optional_regular,
    require_regular_entry, sync_dir_required,
};
use super::projection_store::ProjectionCatalogEntry;
use super::{
    ContentDigest, ManagedPath, ObjectStore, ProjectionIntent, ProjectionIntentId,
    ProjectionReceiptStore, ProjectionStoreError, WorkspaceId,
};

pub(crate) const SUMMARY_NAMESPACE: &str = "receiver-absence-summary-v1";
const SUMMARY_PREFIX: &str = "receiver-absence-summary-";
const SUMMARY_SUFFIX: &str = ".summary";
/// Schema 4 keeps only *current* state in the roots object: the cursor
/// incarnation and folded sequence, the unfinished receiver intents, and the
/// immutable authenticated root of the point-addressable page history. Completed
/// receiver decisions are no longer serialized here, so neither an ordinary open
/// nor a receipt update materializes history. There is one current format: an
/// older object is not read by a second decoder, it simply fails to decode and
/// the disposable summary rebuilds from retained receipts (D-1, D-3).
const SUMMARY_SCHEMA_VERSION: u32 = 4;
#[cfg(test)]
const SUFFIX_COMPLETION: &str = ".completion";
const MAX_SUMMARY_OBJECT_BYTES: u64 = 512 * 1024 * 1024;

/// Core-initiated durability barriers one accepted RECEIVER receipt
/// publication performs end to end: intent publication, roots fold,
/// attempt reservation, mutation authority, graph write, completion
/// publication and roots fold.
///
/// Own-endpoint saves publish no receipt artifacts, so this budget is
/// independent of `MANAGED_SAVE_BARRIER_BUDGET` and does not move it. Four of
/// these barriers are the current-action cursor's two coalesced reservations;
/// the receipt for this packet records the journal seam that would remove
/// them entirely.
///
/// Two of them are the point-addressable page row and its authenticated-map
/// nodes, published as ONE coalesced group into the row namespace before the
/// roots object that names them. That ordering is what makes a crash leave
/// unreferenced objects instead of a root pointing at bytes that were never
/// written, so it cannot be folded into the roots publication by reordering.
/// It could be folded by *coalescing*: one `ArchiveBatchPublication` already
/// supports several namespaces, but `ObjectStore::publish_coalesced_private_derived`
/// exposes only one, and `object_store.rs` is outside this packet's write set.
/// The exact seam is in the receipt.
#[cfg(test)]
const RECEIVER_RECEIPT_BARRIER_BUDGET: u64 = 31;

#[cfg(test)]
static SKIP_NEXT_SUMMARY_UPDATE: std::sync::LazyLock<std::sync::Mutex<BTreeSet<WorkspaceId>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeSet::new()));

#[cfg(test)]
pub(crate) fn skip_next_receiver_absence_summary_update_for_test(workspace_id: WorkspaceId) {
    SKIP_NEXT_SUMMARY_UPDATE
        .lock()
        .expect("summary fault mutex")
        .insert(workspace_id);
}

/// Diagnostic-only attribution carried by the roots object.
///
/// Coverage is *proven* by the current-action cursor being empty, not by these
/// counters; they exist so a receipt can quote how much history one bounded
/// roots object stands for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverageAttribution {
    folded_receipts: u64,
    repairs: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverAbsenceSummaryObject {
    schema_version: u32,
    workspace_id: WorkspaceId,
    generation: u64,
    previous_digest: Option<ContentDigest>,
    coverage: CoverageAttribution,
    /// The exact cursor incarnation and reservation sequence these roots have
    /// folded through. Coverage is *this*, never "the cursor directory looked
    /// empty": recreating a lost cursor mints a new incarnation, so a stale
    /// claim can never match and can never hide preserved receipt work.
    cursor_coverage: Option<CursorCoverage>,
    /// Immutable root of the authenticated page map holding every completed
    /// receiver decision. Historical bytes stay on disk and are read one page
    /// at a time; this root is the only history term in the roots object.
    history_root: PersistedHistoryRoot,
    incomplete_intents: Vec<ProjectionIntent>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ReceiverAbsenceSummaryOpenStats {
    /// Receipt evidence filenames enumerated. Zero on every healthy open:
    /// the bounded current-action cursor replaced the lifetime name walk, so
    /// a non-zero value here means the instrumented repair ran.
    pub(crate) evidence_names_observed: usize,
    pub(crate) receipt_content_reads: usize,
    pub(crate) full_catalog_passes: usize,
    pub(crate) summary_content_reads: usize,
    pub(crate) rebuilt: bool,
    pub(crate) delta_completions: usize,
    pub(crate) delta_intents: usize,
    /// Uncovered receipt marks the durable cursor offered this open. Bounded
    /// by unfinished work plus one crash window, never by retained history.
    pub(crate) cursor_marks_observed: usize,
    /// The cursor could not prove bounded coverage for these roots — absent,
    /// re-incarnated, torn, or missing a mark — so the roots were rebuilt from
    /// retained receipts instead of resumed. Named repair, never a refusal.
    pub(crate) cursor_unavailable: bool,
    /// Why the repair ran, for the open trace. Diagnostic only.
    pub(crate) repair_cause: Option<String>,
    /// Durable roots installations the catch-up performed. One per resolved
    /// chunk, so a crash mid-catch-up resumes instead of restarting.
    pub(crate) cursor_resume_installs: usize,
    /// Point reads into the durable page history this open performed. Bounded
    /// by the roots probe plus unfinished work; never by retained history.
    pub(crate) history_point_reads: usize,
    /// Receipt obligations the current-action roots still owe: durable
    /// receiver intents with no completion. This is the receipt half of the
    /// one actionable-state producer, and it is bounded by unfinished work.
    pub(crate) actionable_intents: usize,
}

pub(crate) struct ReceiverAbsenceSummaryOpen {
    pub(crate) map: AbsenceDecisionMap,
    pub(crate) cache: Option<ReceiverAbsenceSummary>,
    pub(crate) stats: ReceiverAbsenceSummaryOpenStats,
}

pub(crate) struct ReceiverAbsenceSummary {
    store: ObjectStore,
    directory: Dir,
    workspace_id: WorkspaceId,
    generation: u64,
    tail_digest: Option<ContentDigest>,
    names: BTreeMap<u64, SummaryName>,
    coverage: CoverageAttribution,
    history: std::sync::Arc<ReceiverAbsenceHistory>,
    incomplete_intents: BTreeMap<ProjectionIntentId, ProjectionIntent>,
    cursor_coverage: Option<CursorCoverage>,
    cursor: Option<std::sync::Arc<ProjectionActionCursor>>,
}

#[derive(Clone, Debug)]
struct SummaryName {
    name: String,
    digest: ContentDigest,
}

impl ReceiverAbsenceSummary {
    /// Open the bounded current-action roots for receiver receipt work.
    ///
    /// A healthy open reads the roots object and the durable current-action
    /// cursor. It enumerates no receipt namespace, decodes no historical
    /// receipt, and resolves only the marks the cursor still holds — normally
    /// none. Missing or damaged derived state takes one named, counted repair
    /// through the retained catalog; that repair is never disguised as
    /// ordinary work and never becomes a refusal (D-3, I-10).
    pub(crate) fn open(
        store: &ObjectStore,
        receipts: &ProjectionReceiptStore,
    ) -> Result<ReceiverAbsenceSummaryOpen, ProjectionStoreError> {
        let mut stats = ReceiverAbsenceSummaryOpenStats::default();
        let cursor = receipts.action_cursor();
        if let Some(handle) = cursor.as_deref() {
            handle.register(CursorConsumer::ReceiverAbsenceRoots);
            let opened = Self::open_cache(store, cursor.clone(), &mut stats);
            match opened {
                Ok(Some(mut cache)) => match handle.uncovered_for(cache.cursor_coverage) {
                    Ok(marks) => {
                        stats.cursor_marks_observed = marks.len();
                        match cache.resume_from_cursor(receipts, handle, &marks, None, &mut stats) {
                            Ok(()) => {
                                stats.actionable_intents = cache.incomplete_intents.len();
                                let map = cache.materialized_map()?;
                                return Ok(ReceiverAbsenceSummaryOpen {
                                    map,
                                    cache: Some(cache),
                                    stats,
                                });
                            }
                            Err(error) => {
                                stats.repair_cause = Some(error.to_string());
                            }
                        }
                    }
                    Err(error) => {
                        stats.cursor_unavailable = true;
                        stats.repair_cause = Some(error.to_string());
                    }
                },
                Ok(None) => {
                    stats.repair_cause = Some("no receiver absence roots object".into());
                }
                Err(error) => {
                    stats.repair_cause = Some(error);
                }
            }
        } else {
            stats.cursor_unavailable = true;
            stats.repair_cause = Some("no current-action cursor is attached".into());
        }

        Self::repair(store, receipts, cursor, stats)
    }

    /// The named repair: one validated pass over retained receipt truth.
    fn repair(
        store: &ObjectStore,
        receipts: &ProjectionReceiptStore,
        cursor: Option<std::sync::Arc<ProjectionActionCursor>>,
        mut stats: ReceiverAbsenceSummaryOpenStats,
    ) -> Result<ReceiverAbsenceSummaryOpen, ProjectionStoreError> {
        stats.rebuilt = true;
        stats.full_catalog_passes = 1;
        let evidence_names = receipts.absence_summary_evidence_names()?;
        stats.evidence_names_observed = evidence_names.len();
        let catalog = receipts.validated_catalog()?;
        stats.receipt_content_reads = catalog
            .iter()
            .map(|row| usize::from(row.completion.is_some()) + 1)
            .sum();
        let (rows, incomplete) = rebuild_rows(catalog)?;
        let mut cache = Self::fresh_cache(store, cursor.clone()).ok();
        if let Some(candidate) = cache.as_mut() {
            candidate.coverage.folded_receipts = evidence_names.len() as u64;
            candidate.coverage.repairs = 1;
            // Retained receipts are strictly more complete than the cursor, so
            // the rebuilt roots may claim everything reserved so far. The claim
            // is published with the roots themselves, so a crash before the
            // install leaves the old (unmatched) coverage and repairs again.
            candidate.cursor_coverage = cursor
                .as_deref()
                .map(ProjectionActionCursor::coverage_of_everything_reserved);
            candidate.incomplete_intents = incomplete
                .iter()
                .map(|intent| Ok((intent.id()?, intent.clone())))
                .collect::<Result<_, super::ReceiptError>>()?;
            let rebuilt = rows
                .values()
                .cloned()
                .collect::<Vec<_>>()
                .chunks(ROW_REBUILD_PUBLISH_ROWS)
                .try_fold((), |(), group| {
                    let mut batch = candidate.begin_batch();
                    for row in group {
                        candidate.stage_row(&mut batch, row.clone())?;
                    }
                    candidate.commit_batch(batch)
                });
            if rebuilt.is_err() || candidate.install().is_err() {
                cache = None;
            }
        }
        if cache.is_some() {
            // The rebuild covers strictly more than any mark could, so every
            // outstanding reservation is now redundant.
            if let (Some(handle), Some(coverage)) = (
                cursor.as_deref(),
                cache.as_ref().and_then(|c| c.cursor_coverage),
            ) {
                handle.commit_coverage(CursorConsumer::ReceiverAbsenceRoots, coverage);
            }
        }
        stats.actionable_intents = incomplete.len();
        let mut map = AbsenceDecisionMap::default();
        match cache.as_ref() {
            // The rebuilt index is durable: history stays point-addressable.
            Some(candidate) => map.attach_history(candidate.history.clone()),
            // No durable derived state could be installed at all. Correctness
            // outranks residency, so this open keeps the rebuilt receiver rows
            // resident for its own lifetime and the next open repairs again.
            None => {
                for row in rows.values() {
                    map.record_receiver_summary_entry(row.clone());
                }
            }
        }
        for intent in incomplete {
            map.record_incomplete_receiver_intent(&intent)?;
        }
        Ok(ReceiverAbsenceSummaryOpen { map, cache, stats })
    }

    /// Fold exactly the receipts the cursor still points at.
    ///
    /// Each mark costs one point read of its intent and, when the receipt
    /// completed, one of its completion. A mark whose receipt never landed
    /// (crash between reservation and publication) resolves to nothing and is
    /// dropped; a receipt that landed and was never summarized is recovered
    /// here.
    ///
    /// Work is streamed in chunks, and each chunk's progress is installed
    /// durably before the next one starts. A legitimately large uncovered
    /// window is therefore ordinary paged work, never a reason to reconstruct
    /// from history (D-5) and never a reason to refuse; a crash in the middle
    /// resumes at the last installed chunk with no pending receipt lost.
    fn resume_from_cursor(
        &mut self,
        receipts: &ProjectionReceiptStore,
        cursor: &ProjectionActionCursor,
        marks: &[UncoveredMark],
        max_chunks: Option<usize>,
        stats: &mut ReceiverAbsenceSummaryOpenStats,
    ) -> Result<(), ProjectionStoreError> {
        if marks.is_empty() {
            // Nothing to fold, but the reservations at or below the durable
            // watermark are now reclaimable by every registered consumer.
            if let Some(coverage) = self.cursor_coverage {
                cursor.commit_coverage(CursorConsumer::ReceiverAbsenceRoots, coverage);
            }
            return Ok(());
        }
        let mut resolved = BTreeSet::new();
        for chunk in marks
            .chunks(CURSOR_RESUME_CHUNK_MARKS)
            .take(max_chunks.unwrap_or(usize::MAX))
        {
            let mut batch = self.begin_batch();
            for mark in chunk {
                if !resolved.insert(mark.intent_id) {
                    // The intent and completion halves of one receipt are two
                    // reservations; one resolution covers both.
                    continue;
                }
                let Some(intent) = receipts.load_intent(mark.intent_id)? else {
                    // The reservation outlived a publication that never landed.
                    continue;
                };
                stats.receipt_content_reads = stats.receipt_content_reads.saturating_add(1);
                if receipts.load_completion(&intent)?.is_some() {
                    stats.receipt_content_reads = stats.receipt_content_reads.saturating_add(1);
                    stats.delta_completions = stats.delta_completions.saturating_add(1);
                    self.incomplete_intents.remove(&mark.intent_id);
                    self.fold_completion(&mut batch, &intent, stats)
                        .map_err(ProjectionStoreError::Encode)?;
                } else {
                    stats.delta_intents = stats.delta_intents.saturating_add(1);
                    self.incomplete_intents.insert(mark.intent_id, intent);
                }
                self.coverage.folded_receipts = self.coverage.folded_receipts.saturating_add(1);
            }
            // The chunk's page rows become durable before the roots object that
            // names them, so a crash here leaves unreferenced objects rather
            // than a root pointing at bytes that were never written.
            self.commit_batch(batch)
                .map_err(ProjectionStoreError::Encode)?;
            let last = chunk.last().expect("chunks are non-empty").sequence;
            let coverage = cursor.coverage_through(last);
            self.cursor_coverage = Some(coverage);
            self.install().map_err(ProjectionStoreError::Encode)?;
            stats.cursor_resume_installs = stats.cursor_resume_installs.saturating_add(1);
            cursor.commit_coverage(CursorConsumer::ReceiverAbsenceRoots, coverage);
        }
        Ok(())
    }

    /// Fold exactly `chunks` catch-up chunks and stop, modelling a process
    /// that died mid-catch-up. Every chunk it did finish is durable.
    #[cfg(test)]
    pub(crate) fn resume_partially_for_test(
        store: &ObjectStore,
        receipts: &ProjectionReceiptStore,
        chunks: usize,
    ) -> Result<usize, ProjectionStoreError> {
        let mut stats = ReceiverAbsenceSummaryOpenStats::default();
        let cursor = receipts
            .action_cursor()
            .expect("the fixture attaches a cursor");
        cursor.register(CursorConsumer::ReceiverAbsenceRoots);
        let mut cache = Self::open_cache(store, receipts.action_cursor(), &mut stats)
            .map_err(ProjectionStoreError::Encode)?
            .expect("a roots object exists");
        let marks = cursor
            .uncovered_for(cache.cursor_coverage)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
        cache.resume_from_cursor(receipts, &cursor, &marks, Some(chunks), &mut stats)?;
        Ok(stats.cursor_resume_installs)
    }

    /// The decision map the engine consumes: durable completions plus the
    /// still-actionable intents, which are roots rather than history.
    fn materialized_map(&self) -> Result<AbsenceDecisionMap, ProjectionStoreError> {
        let mut map = AbsenceDecisionMap::default();
        map.attach_history(self.history.clone());
        for intent in self.incomplete_intents.values() {
            map.record_incomplete_receiver_intent(intent)?;
        }
        Ok(map)
    }

    /// The shared point-addressable receiver history this roots object names.
    pub(crate) fn history(&self) -> &std::sync::Arc<ReceiverAbsenceHistory> {
        &self.history
    }

    /// Unfinished receipt work, as the one current actionable-state producer
    /// reports it. Completed chains are deliberately absent.
    pub(crate) fn current_action_roots(&self) -> CurrentActionRoots {
        CurrentActionRoots {
            actionable_intents: self.incomplete_intents.values().cloned().collect(),
            sweep_pins: Vec::new(),
        }
    }

    pub(crate) fn record_completion(&mut self, intent: &ProjectionIntent) -> Result<(), String> {
        #[cfg(test)]
        if SKIP_NEXT_SUMMARY_UPDATE
            .lock()
            .expect("summary fault mutex")
            .remove(&self.workspace_id)
        {
            return Ok(());
        }
        let intent_id = intent.id().map_err(|error| error.to_string())?;
        let mut changed = self.incomplete_intents.remove(&intent_id).is_some();
        let mut batch = self.begin_batch();
        let mut stats = ReceiverAbsenceSummaryOpenStats::default();
        changed |= self.fold_completion(&mut batch, intent, &mut stats)?;
        self.commit_batch(batch)?;
        if changed {
            self.coverage.folded_receipts = self.coverage.folded_receipts.saturating_add(1);
        }
        self.advance_coverage(intent_id, changed)
    }

    pub(crate) fn record_intent(&mut self, intent: &ProjectionIntent) -> Result<(), String> {
        #[cfg(test)]
        if SKIP_NEXT_SUMMARY_UPDATE
            .lock()
            .expect("summary fault mutex")
            .remove(&self.workspace_id)
        {
            return Ok(());
        }
        let intent_id = intent.id().map_err(|error| error.to_string())?;
        let superseded = self.receiver_completion_recorded(intent_id, intent)?;
        let already_known = self.incomplete_intents.get(&intent_id) == Some(intent);
        let changed = if superseded || already_known {
            false
        } else {
            self.incomplete_intents.insert(intent_id, intent.clone());
            self.coverage.folded_receipts = self.coverage.folded_receipts.saturating_add(1);
            true
        };
        self.advance_coverage(intent_id, changed)
    }

    /// Publish the roots when this receipt changed them OR when covering it
    /// advances the durable cursor watermark, then let the cursor reclaim what
    /// every registered consumer has now covered.
    ///
    /// The watermark is published *with* the roots, in the one audited write
    /// that also publishes the state it claims to cover. A crash before that
    /// write leaves the older watermark, so the receipt is simply re-folded;
    /// a crash after it leaves a consistent pair. There is no window in which
    /// coverage is claimed for state that is not durable.
    fn advance_coverage(
        &mut self,
        intent_id: ProjectionIntentId,
        changed: bool,
    ) -> Result<(), String> {
        let advanced = self.cursor.as_deref().map(|cursor| {
            cursor.coverage_after(
                CursorConsumer::ReceiverAbsenceRoots,
                self.cursor_coverage,
                intent_id,
            )
        });
        let coverage_moved = advanced.is_some() && advanced != self.cursor_coverage;
        if !changed && !coverage_moved {
            return Ok(());
        }
        if let Some(coverage) = advanced {
            self.cursor_coverage = Some(coverage);
        }
        self.install()?;
        if let (Some(cursor), Some(coverage)) = (self.cursor.as_deref(), advanced) {
            cursor.commit_coverage(CursorConsumer::ReceiverAbsenceRoots, coverage);
        }
        Ok(())
    }

    /// Is this receipt already a durable completion? One point read of the
    /// page's own row, never a scan of the summarized history.
    fn receiver_completion_recorded(
        &self,
        intent_id: ProjectionIntentId,
        intent: &ProjectionIntent,
    ) -> Result<bool, String> {
        Ok(self
            .history
            .row(intent.page_id(), intent.path())?
            .is_some_and(|row| {
                row.anchors
                    .iter()
                    .any(|anchor| anchor.intent_id == intent_id)
            }))
    }

    fn begin_batch(&self) -> RowBatch {
        RowBatch {
            staged: BTreeMap::new(),
            root: self.history.root(),
        }
    }

    /// Fold one completed receipt into the batch: one exact (page, path) point
    /// read, then one exact row install. Returns whether the row changed.
    fn fold_completion(
        &self,
        batch: &mut RowBatch,
        intent: &ProjectionIntent,
        stats: &mut ReceiverAbsenceSummaryOpenStats,
    ) -> Result<bool, String> {
        let anchor = AbsenceCompletionAnchor::from_intent(intent).map_err(|e| e.to_string())?;
        stats.history_point_reads = stats.history_point_reads.saturating_add(1);
        let mut row = lookup_row(
            &self.history.rows,
            Some(&batch.staged),
            batch.root,
            self.workspace_id,
            anchor.page_id,
            &anchor.path,
            &self.history.counters,
        )?
        .unwrap_or_else(|| ReceiverAbsenceSummaryEntry {
            page_id: anchor.page_id,
            path: anchor.path.clone(),
            anchors: Vec::new(),
            restored_generation_requires_deferral: false,
        });
        let relation = restored_generation_relation(&row.anchors, &anchor);
        let mut changed = merge_anchor(&mut row.anchors, anchor);
        if relation && !row.restored_generation_requires_deferral {
            row.restored_generation_requires_deferral = true;
            changed = true;
        }
        if !changed {
            return Ok(false);
        }
        self.stage_row(batch, row)?;
        Ok(true)
    }

    fn stage_row(
        &self,
        batch: &mut RowBatch,
        row: ReceiverAbsenceSummaryEntry,
    ) -> Result<(), String> {
        batch.root = upsert_row(
            &self.history.rows,
            &mut batch.staged,
            batch.root,
            self.workspace_id,
            row,
            &self.history.counters,
        )?;
        Ok(())
    }

    /// Publish every staged index object in ONE audited archive publication and
    /// adopt the resulting immutable root.
    ///
    /// The map writer itself never writes: it stages into memory, and this is
    /// the single durable publication for the whole group. The root is adopted
    /// only after that publication returns, and the roots object that persists
    /// it is installed afterwards, so a crash leaves unreferenced objects rather
    /// than a root naming bytes that were never written.
    fn commit_batch(&self, batch: RowBatch) -> Result<(), String> {
        if self.history.is_damaged() {
            // Same rule as `install`: a retired index takes no more writes from
            // the activation that retired it.
            return Err(
                "receiver absence rows are retired for repair: this activation \
proved the derived index damaged and must not extend it"
                    .to_owned(),
            );
        }
        if batch.staged.is_empty() {
            return Ok(());
        }
        let artifacts = batch
            .staged
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice(), MAX_ROW_OBJECT_BYTES))
            .collect::<Vec<_>>();
        self.store
            .publish_coalesced_private_derived(
                &self.history.rows,
                &artifacts,
                "receiver absence index object",
            )
            .map_err(|error| error.to_string())?;
        self.history.adopt_root(batch.root);
        Ok(())
    }

    fn open_cache(
        store: &ObjectStore,
        cursor: Option<std::sync::Arc<ProjectionActionCursor>>,
        stats: &mut ReceiverAbsenceSummaryOpenStats,
    ) -> Result<Option<Self>, String> {
        let root = store
            .private_derived_root_capability()
            .map_err(|error| error.to_string())?;
        ensure_reconstructible_directory_nofollow(&root, ABSENCE_NAMESPACE)
            .map_err(|error| error.to_string())?;
        let absence =
            open_dir_nofollow(&root, ABSENCE_NAMESPACE).map_err(|error| error.to_string())?;
        ensure_reconstructible_directory_nofollow(&absence, SUMMARY_NAMESPACE)
            .map_err(|error| error.to_string())?;
        let directory =
            open_dir_nofollow(&absence, SUMMARY_NAMESPACE).map_err(|error| error.to_string())?;
        let rows = open_row_directory(&absence)?;
        let names = enumerate_names(&directory)?;
        let Some((&generation, name)) = names.last_key_value() else {
            return Ok(None);
        };
        let bytes = read_summary(&directory, name, stats)?;
        let object = decode_bound(&bytes, store.workspace_id(), generation)?;
        match (
            object.previous_digest,
            names.range(..generation).next_back(),
        ) {
            (None, None) => {}
            (Some(expected), Some((_, previous_name))) => {
                let previous = read_summary(&directory, previous_name, stats)?;
                if ContentDigest::of(&previous) != expected {
                    return Err("receiver absence summary chain digest mismatch".into());
                }
            }
            _ => return Err("receiver absence summary chain is torn".into()),
        }
        let history = std::sync::Arc::new(ReceiverAbsenceHistory {
            rows,
            chain: directory.try_clone().map_err(|error| error.to_string())?,
            workspace_id: store.workspace_id(),
            root: std::sync::RwLock::new(object.history_root.decode()?),
            counters: HistoryCounters::default(),
            damaged: std::sync::atomic::AtomicBool::new(false),
        });
        // One O(1) integrity probe of the named root node. Deeper damage is
        // still detected at its own point read and retires these roots, but a
        // wholly missing index heals at open rather than at decision time.
        probe_history_root(&history, stats)?;
        let mut incomplete_intents = BTreeMap::new();
        for intent in object.incomplete_intents {
            if intent.workspace_id() != store.workspace_id() {
                return Err("receiver absence summary incomplete intent workspace mismatch".into());
            }
            let intent_id = intent.id().map_err(|error| error.to_string())?;
            if incomplete_intents.insert(intent_id, intent).is_some() {
                return Err("duplicate receiver absence summary incomplete intent".into());
            }
        }
        Ok(Some(Self {
            store: store
                .duplicate_retained_capability()
                .map_err(|error| error.to_string())?,
            directory,
            workspace_id: store.workspace_id(),
            generation,
            tail_digest: Some(ContentDigest::of(&bytes)),
            names,
            coverage: object.coverage,
            cursor_coverage: object.cursor_coverage,
            history,
            incomplete_intents,
            cursor,
        }))
    }

    fn fresh_cache(
        store: &ObjectStore,
        cursor: Option<std::sync::Arc<ProjectionActionCursor>>,
    ) -> Result<Self, String> {
        let root = store
            .private_derived_root_capability()
            .map_err(|error| error.to_string())?;
        ensure_reconstructible_directory_nofollow(&root, ABSENCE_NAMESPACE)
            .map_err(|error| error.to_string())?;
        let absence =
            open_dir_nofollow(&root, ABSENCE_NAMESPACE).map_err(|error| error.to_string())?;
        ensure_reconstructible_directory_nofollow(&absence, SUMMARY_NAMESPACE)
            .map_err(|error| error.to_string())?;
        let directory =
            open_dir_nofollow(&absence, SUMMARY_NAMESPACE).map_err(|error| error.to_string())?;
        let rows = open_row_directory(&absence)?;
        clear_chain(&directory)?;
        // The rebuild republishes every row from retained receipts, so the old
        // derived objects — including nodes superseded by ordinary upserts —
        // are reclaimed here rather than accumulating for the workspace's life.
        clear_rows(&rows)?;
        let history = std::sync::Arc::new(ReceiverAbsenceHistory {
            rows,
            chain: directory.try_clone().map_err(|error| error.to_string())?,
            workspace_id: store.workspace_id(),
            root: std::sync::RwLock::new(AuthenticatedMapRootV1::empty()),
            counters: HistoryCounters::default(),
            damaged: std::sync::atomic::AtomicBool::new(false),
        });
        Ok(Self {
            store: store
                .duplicate_retained_capability()
                .map_err(|error| error.to_string())?,
            directory,
            workspace_id: store.workspace_id(),
            generation: 0,
            tail_digest: None,
            names: BTreeMap::new(),
            coverage: CoverageAttribution::default(),
            cursor_coverage: None,
            history,
            incomplete_intents: BTreeMap::new(),
            cursor,
        })
    }

    fn install(&mut self) -> Result<(), String> {
        if self.history.is_damaged() {
            // Publishing here would hand the next open an apparently healthy
            // chain over a known-damaged index and cancel its rebuild. Refuse
            // by name instead: the caller keeps the receipt's answer resident
            // for this activation (the receipts themselves are still the
            // retained truth), and the retired chain makes the next open run
            // the counted repair.
            return Err(
                "receiver absence roots are retired for repair: this activation \
proved the derived index damaged and must not republish it"
                    .to_owned(),
            );
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| "receiver absence summary generation overflow".to_owned())?;
        let object = ReceiverAbsenceSummaryObject {
            schema_version: SUMMARY_SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            generation,
            previous_digest: self.tail_digest,
            coverage: self.coverage,
            cursor_coverage: self.cursor_coverage,
            history_root: PersistedHistoryRoot::encode(self.history.root())?,
            incomplete_intents: self.incomplete_intents.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec(&object).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_SUMMARY_OBJECT_BYTES {
            return Err("receiver absence summary exceeds its bounded object limit".into());
        }
        let digest = ContentDigest::of(&bytes);
        let name = object_name(generation, digest);
        self.store
            .publish_coalesced_private_derived(
                &self.directory,
                &[(name.as_str(), bytes.as_slice(), MAX_SUMMARY_OBJECT_BYTES)],
                "receiver absence summary object",
            )
            .map_err(|error| error.to_string())?;
        self.generation = generation;
        self.tail_digest = Some(ContentDigest::of(&bytes));
        self.names.insert(generation, SummaryName { name, digest });
        self.prune_old_chain()?;
        Ok(())
    }

    fn prune_old_chain(&mut self) -> Result<(), String> {
        let obsolete = self
            .names
            .iter()
            .rev()
            .skip(2)
            .map(|(generation, name)| (*generation, name.name.clone()))
            .collect::<Vec<_>>();
        for (generation, name) in &obsolete {
            match self.directory.remove_file(name) {
                Ok(()) => {
                    self.names.remove(generation);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.names.remove(generation);
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        if !obsolete.is_empty() {
            sync_dir_required(&self.directory).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

/// Rebuild the exact (page, path) receiver rows from one validated catalog
/// pass.
///
/// This is the named repair's only history-wide work, and it uses exactly the
/// shared `merge_anchor` / `restored_generation_relation` pair the incremental
/// write-through uses, so a rebuilt row and an incrementally-grown row are the
/// same value.
fn rebuild_rows(
    catalog: Vec<ProjectionCatalogEntry>,
) -> Result<
    (
        BTreeMap<(super::PageId, ManagedPath), ReceiverAbsenceSummaryEntry>,
        Vec<ProjectionIntent>,
    ),
    ProjectionStoreError,
> {
    let mut rows = BTreeMap::new();
    let mut incomplete = Vec::new();
    for entry in catalog {
        if entry.completion.is_some() {
            let anchor = AbsenceCompletionAnchor::from_intent(&entry.intent)?;
            let row = rows
                .entry(anchor.key())
                .or_insert_with(|| ReceiverAbsenceSummaryEntry {
                    page_id: anchor.page_id,
                    path: anchor.path.clone(),
                    anchors: Vec::new(),
                    restored_generation_requires_deferral: false,
                });
            let relation = restored_generation_relation(&row.anchors, &anchor);
            merge_anchor(&mut row.anchors, anchor);
            row.restored_generation_requires_deferral |= relation;
        } else {
            incomplete.push(entry.intent);
        }
    }
    Ok((rows, incomplete))
}

fn open_row_directory(absence: &Dir) -> Result<Dir, String> {
    ensure_reconstructible_directory_nofollow(absence, ROWS_NAMESPACE)
        .map_err(|error| error.to_string())?;
    open_dir_nofollow(absence, ROWS_NAMESPACE).map_err(|error| error.to_string())
}

fn probe_history_root(
    history: &ReceiverAbsenceHistory,
    stats: &mut ReceiverAbsenceSummaryOpenStats,
) -> Result<(), String> {
    let root = history.root();
    let Some(link) = root.root else {
        return Ok(());
    };
    stats.history_point_reads = stats.history_point_reads.saturating_add(1);
    let store = SealedRowObjects {
        directory: &history.rows,
        staged: None,
        published: None,
        counters: &history.counters,
    };
    MapReader::new(&store)
        .read_map_node(link)
        .map(|_| ())
        .map_err(|error| format!("receiver absence history root node: {error}"))
}

fn enumerate_names(directory: &Dir) -> Result<BTreeMap<u64, SummaryName>, String> {
    let mut names = BTreeMap::new();
    for entry in directory.entries().map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 receiver absence summary entry".to_owned())?;
        if !name.starts_with(SUMMARY_PREFIX) {
            continue;
        }
        require_regular_entry(
            &entry.file_type().map_err(|error| error.to_string())?,
            &name,
        )
        .map_err(|error| error.to_string())?;
        let parsed = parse_object_name(&name)?;
        if names.insert(parsed.0, parsed.1).is_some() {
            return Err("receiver absence summary generation twin".into());
        }
    }
    Ok(names)
}

fn parse_object_name(name: &str) -> Result<(u64, SummaryName), String> {
    let body = name
        .strip_prefix(SUMMARY_PREFIX)
        .and_then(|value| value.strip_suffix(SUMMARY_SUFFIX))
        .ok_or_else(|| "invalid receiver absence summary name".to_owned())?;
    let (digits, digest_hex) = body
        .split_once('-')
        .ok_or_else(|| "receiver absence summary name lacks a digest".to_owned())?;
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("non-canonical receiver absence summary generation".into());
    }
    let digest = decode_digest(digest_hex)?;
    let generation = digits.parse::<u64>().map_err(|error| error.to_string())?;
    if generation == 0 || object_name(generation, digest) != name {
        return Err("non-canonical receiver absence summary name".into());
    }
    Ok((
        generation,
        SummaryName {
            name: name.to_owned(),
            digest,
        },
    ))
}

fn object_name(generation: u64, digest: ContentDigest) -> String {
    format!(
        "{SUMMARY_PREFIX}{generation:020}-{}{SUMMARY_SUFFIX}",
        hex(digest.as_bytes())
    )
}

fn read_summary(
    directory: &Dir,
    name: &SummaryName,
    stats: &mut ReceiverAbsenceSummaryOpenStats,
) -> Result<Vec<u8>, String> {
    let bytes = read_optional_regular(directory, &name.name, MAX_SUMMARY_OBJECT_BYTES, None)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "receiver absence summary disappeared during open".to_owned())?;
    if ContentDigest::of(&bytes) != name.digest {
        return Err("receiver absence summary filename digest mismatch".into());
    }
    stats.summary_content_reads = stats.summary_content_reads.saturating_add(1);
    Ok(bytes)
}

fn decode_bound(
    bytes: &[u8],
    workspace_id: WorkspaceId,
    generation: u64,
) -> Result<ReceiverAbsenceSummaryObject, String> {
    let object: ReceiverAbsenceSummaryObject =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if object.schema_version != SUMMARY_SCHEMA_VERSION
        || object.workspace_id != workspace_id
        || object.generation != generation
        || serde_json::to_vec(&object).map_err(|error| error.to_string())? != bytes
    {
        return Err("receiver absence summary object binding mismatch".into());
    }
    Ok(object)
}

#[cfg(test)]
fn completion_filename(intent: &ProjectionIntent) -> Result<String, super::ReceiptError> {
    Ok(format!(
        "{}{SUFFIX_COMPLETION}",
        hex(intent.id()?.as_bytes())
    ))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn decode_digest(value: &str) -> Result<ContentDigest, String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("non-canonical receiver absence summary digest".into());
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err("non-canonical receiver absence summary digest".to_owned()),
        };
        bytes[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(ContentDigest::from_bytes(bytes))
}

fn target_rank(kind: super::ProjectionTargetKind) -> u8 {
    match kind {
        super::ProjectionTargetKind::Present => 0,
        super::ProjectionTargetKind::Absent => 1,
    }
}

fn clear_chain(directory: &Dir) -> Result<(), String> {
    let names = enumerate_names(directory)?;
    for name in names.values() {
        match directory.remove_file(&name.name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    if !names.is_empty() {
        sync_dir_required(directory).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The durable point-addressable receiver history.
///
/// **D-14 search.** Existing machinery was searched before anything was built:
/// `tine_storage::sealed_accepted_index` already publishes an *immutable
/// authenticated map root* with a point `map_value(root, key)` API over a
/// caller-owned content-addressed object store, and `checkpoint_generation.rs`
/// already demonstrates the object-store adapters for it. That is the machinery
/// used here — the same treap, the same node encoding, the same root type — so
/// this module adds no tree, no map codec and no key derivation of its own.
/// `portable_path_index.rs` was rejected as a *substrate* because its key is
/// the portable (case-folded) path, which deliberately cannot distinguish two
/// exact managed paths; its `exact_path_digest` is reused as the key
/// derivation. `local_completion_index.rs` was rejected because it is the
/// own-endpoint half and is already pruned to live work.
///
/// **Key composition (no truncation, no tuple hashing).** The map key width is
/// 128 bits, and the lookup key is (PageId, exact ManagedPath). Three composed
/// maps carry it exactly:
///
/// * level A — key = the page's own UUID bytes, value = that page's path-index
///   record;
/// * level B — key = the HIGH 128 bits of `exact_path_digest(path)`, value = a
///   bucket record;
/// * level C — key = the LOW 128 bits of the same digest, value = the row for
///   one exact path.
///
/// Both halves of the full 256-bit digest are used, PageId keeps its own outer
/// key, and the row record carries and validates the full exact path. One
/// lookup or update therefore touches three small fixed-shape records plus
/// O(log) map nodes — never a vector of a page's historical paths.
pub(crate) const ROWS_NAMESPACE: &str = "receiver-absence-rows-v1";
const PAGE_PREFIX: &str = "receiver-absence-page-";
const BUCKET_PREFIX: &str = "receiver-absence-bucket-";
pub(crate) const ROW_PREFIX: &str = "receiver-absence-row-";
const NODE_PREFIX: &str = "receiver-absence-node-";
const OBJECT_SUFFIX: &str = ".obj";
const MAX_ROW_OBJECT_BYTES: u64 = 64 * 1024 * 1024;
const ROW_RECORD_SCHEMA_VERSION: u32 = 1;
/// A repair working-set budget, not an occupancy limit: the rebuild publishes
/// its rebuilt rows in groups of this many rows so one catalog pass does not
/// stage the whole history as temporary files at once. Crossing it publishes
/// and continues; nothing is refused and no history is skipped (D-5).
const ROW_REBUILD_PUBLISH_ROWS: usize = 256;

/// Level A: which exact paths this page has receiver rows for. Fixed shape —
/// one root, never a list of paths.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverAbsencePageIndex {
    schema_version: u32,
    workspace_id: WorkspaceId,
    page_id: super::PageId,
    paths: PersistedHistoryRoot,
}

/// Level B: the low half of the exact-path key space under one high half.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverAbsencePathBucket {
    schema_version: u32,
    workspace_id: WorkspaceId,
    page_id: super::PageId,
    key_high: [u8; 16],
    rows: PersistedHistoryRoot,
}

/// Level C: one exact (page, path) decision row, carrying the full exact path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiverAbsenceRowRecord {
    schema_version: u32,
    workspace_id: WorkspaceId,
    row: ReceiverAbsenceSummaryEntry,
}

/// Serde mirror of `AuthenticatedMapRootV1`, which is a `tine-storage` value
/// with no serde derive. Stores exactly its three fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedHistoryRoot {
    count: u64,
    key: Option<[u8; 16]>,
    digest: Option<[u8; 32]>,
}

impl PersistedHistoryRoot {
    /// Receiver absence keys are exactly 128 bits wide by construction. Reject
    /// rather than truncate if a root ever carries a wider shared key.
    fn encode(value: AuthenticatedMapRootV1) -> Result<Self, String> {
        Ok(Self {
            count: value.count,
            key: value
                .root
                .map(|link| {
                    <[u8; 16]>::try_from(link.key.as_slice()).map_err(|_| {
                        "receiver absence history root key is not a 128-bit key".to_string()
                    })
                })
                .transpose()?,
            digest: value.root.map(|link| *link.digest.as_bytes()),
        })
    }
}

impl PersistedHistoryRoot {
    fn decode(self) -> Result<AuthenticatedMapRootV1, String> {
        let root = match (self.key, self.digest) {
            (Some(key), Some(digest)) => Some(AuthenticatedMapLinkV1 {
                key: AuthenticatedMapKey::from(key),
                digest: ContentDigest::from_bytes(digest),
            }),
            (None, None) => None,
            _ => return Err("receiver absence history root is torn".into()),
        };
        if (self.count == 0) != root.is_none() {
            return Err("receiver absence history root count disagrees with its link".into());
        }
        Ok(AuthenticatedMapRootV1 {
            count: self.count,
            root,
        })
    }
}

/// The two 128-bit halves of the full-width exact-path digest. Both are used,
/// so the composed key is the complete 256-bit value.
fn exact_path_key(path: &ManagedPath) -> ([u8; 16], [u8; 16]) {
    let digest = *super::portable_path_index::exact_path_digest(path).as_bytes();
    let mut high = [0_u8; 16];
    let mut low = [0_u8; 16];
    high.copy_from_slice(&digest[..16]);
    low.copy_from_slice(&digest[16..]);
    (high, low)
}

/// The node object name. The kind code stays the one this namespace has always
/// written, so relocating the treap codec changed no durable name.
fn sealed_object_name(address: ContentDigest) -> String {
    format!(
        "{NODE_PREFIX}{MAP_NODE_KIND_CODE}-{}{OBJECT_SUFFIX}",
        hex(address.as_bytes())
    )
}

fn record_object_name(prefix: &str, address: ContentDigest) -> String {
    format!("{prefix}{}{OBJECT_SUFFIX}", hex(address.as_bytes()))
}

/// Read/write counters for the durable history, so growth qualification can
/// assert bytes and objects — not merely call counts — per decision and per
/// update.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HistoryAccess {
    pub(crate) point_lookups: usize,
    pub(crate) objects_read: usize,
    pub(crate) bytes_read: usize,
    pub(crate) objects_written: usize,
    pub(crate) bytes_written: usize,
}

#[derive(Debug, Default)]
struct HistoryCounters {
    point_lookups: std::sync::atomic::AtomicUsize,
    objects_read: std::sync::atomic::AtomicUsize,
    bytes_read: std::sync::atomic::AtomicUsize,
    objects_written: std::sync::atomic::AtomicUsize,
    bytes_written: std::sync::atomic::AtomicUsize,
}

impl HistoryCounters {
    fn snapshot(&self) -> HistoryAccess {
        use std::sync::atomic::Ordering::Relaxed;
        HistoryAccess {
            point_lookups: self.point_lookups.load(Relaxed),
            objects_read: self.objects_read.load(Relaxed),
            bytes_read: self.bytes_read.load(Relaxed),
            objects_written: self.objects_written.load(Relaxed),
            bytes_written: self.bytes_written.load(Relaxed),
        }
    }

    fn note_read(&self, bytes: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        self.objects_read.fetch_add(1, Relaxed);
        self.bytes_read.fetch_add(bytes, Relaxed);
    }

    fn note_write(&self, bytes: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        self.objects_written.fetch_add(1, Relaxed);
        self.bytes_written.fetch_add(bytes, Relaxed);
    }
}

/// The one `MapNodeObjects` adapter for this index.
///
/// Reads take the not-yet-published staging group first and the durable
/// directory second. Publication only stages: nothing becomes durable until the
/// caller commits the whole group through one audited archive publication, so a
/// crash can never leave a root naming a node that was never written.
struct SealedRowObjects<'a> {
    directory: &'a Dir,
    staged: Option<&'a BTreeMap<String, Vec<u8>>>,
    published: Option<&'a mut BTreeMap<String, Vec<u8>>>,
    counters: &'a HistoryCounters,
}

impl MapNodeObjects for SealedRowObjects<'_> {
    fn read_map_node_object(
        &self,
        address: ContentDigest,
    ) -> Result<Option<Vec<u8>>, SealedAcceptedIndexError> {
        let name = sealed_object_name(address);
        // The map writer path-copies, so within one call it reads back nodes it
        // has just published. Reads therefore see this batch's own group first.
        if let Some(bytes) = self
            .published
            .as_ref()
            .and_then(|published| published.get(&name))
        {
            self.counters.note_read(bytes.len());
            return Ok(Some(bytes.clone()));
        }
        read_index_object(self.directory, self.staged, &name, self.counters)
            .map_err(SealedAcceptedIndexError::Corrupt)
    }

    fn publish_map_node_object(
        &mut self,
        address: ContentDigest,
        bytes: &[u8],
    ) -> Result<(), SealedAcceptedIndexError> {
        let name = sealed_object_name(address);
        let counters = self.counters;
        let published = self.published.as_mut().ok_or_else(|| {
            SealedAcceptedIndexError::Corrupt(
                "receiver absence history is open for reading only".into(),
            )
        })?;
        counters.note_write(bytes.len());
        published.insert(name, bytes.to_vec());
        Ok(())
    }
}

fn read_index_object(
    directory: &Dir,
    staged: Option<&BTreeMap<String, Vec<u8>>>,
    name: &str,
    counters: &HistoryCounters,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(bytes) = staged.and_then(|staged| staged.get(name)) {
        counters.note_read(bytes.len());
        return Ok(Some(bytes.clone()));
    }
    let bytes = read_optional_regular(directory, name, MAX_ROW_OBJECT_BYTES, None)
        .map_err(|error| error.to_string())?;
    if let Some(bytes) = bytes.as_ref() {
        counters.note_read(bytes.len());
    }
    Ok(bytes)
}

fn required_index_object(
    directory: &Dir,
    staged: Option<&BTreeMap<String, Vec<u8>>>,
    name: &str,
    counters: &HistoryCounters,
    what: &str,
) -> Result<Vec<u8>, String> {
    read_index_object(directory, staged, name, counters)?
        .ok_or_else(|| format!("receiver absence history names a missing row object: {what}"))
}

fn encode_index_record<T: Serialize>(record: &T) -> Result<(Vec<u8>, ContentDigest), String> {
    let bytes = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_ROW_OBJECT_BYTES {
        return Err("receiver absence index object exceeds its bounded limit".into());
    }
    let address = ContentDigest::of(&bytes);
    Ok((bytes, address))
}

fn decode_index_record<T: Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    address: ContentDigest,
) -> Result<T, String> {
    if ContentDigest::of(bytes) != address {
        return Err("receiver absence index object address mismatch".into());
    }
    let record: T = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if serde_json::to_vec(&record).map_err(|error| error.to_string())? != bytes {
        return Err("receiver absence index object is non-canonical".into());
    }
    Ok(record)
}

/// One exact (page, path) point lookup through the three composed maps.
///
/// Every level validates its record's own bound identity, so an address that
/// resolved to the wrong bytes is named damage, never a wrong answer, and never
/// silently "this key has no history".
fn lookup_row(
    directory: &Dir,
    staged: Option<&BTreeMap<String, Vec<u8>>>,
    root: AuthenticatedMapRootV1,
    workspace_id: WorkspaceId,
    page_id: super::PageId,
    path: &ManagedPath,
    counters: &HistoryCounters,
) -> Result<Option<ReceiverAbsenceSummaryEntry>, String> {
    counters
        .point_lookups
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let store = SealedRowObjects {
        directory,
        staged,
        published: None,
        counters,
    };
    let reader = MapReader::new(&store);
    let Some(page_address) = reader
        .map_value(root, page_id.as_uuid().into_bytes())
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let bytes = required_index_object(
        directory,
        staged,
        &record_object_name(PAGE_PREFIX, page_address),
        counters,
        "page index",
    )?;
    let page: ReceiverAbsencePageIndex = decode_index_record(&bytes, page_address)?;
    if page.schema_version != ROW_RECORD_SCHEMA_VERSION
        || page.workspace_id != workspace_id
        || page.page_id != page_id
    {
        return Err("receiver absence page index binding mismatch".into());
    }

    let (high, low) = exact_path_key(path);
    let Some(bucket_address) = reader
        .map_value(page.paths.decode()?, high)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let bytes = required_index_object(
        directory,
        staged,
        &record_object_name(BUCKET_PREFIX, bucket_address),
        counters,
        "path bucket",
    )?;
    let bucket: ReceiverAbsencePathBucket = decode_index_record(&bytes, bucket_address)?;
    if bucket.schema_version != ROW_RECORD_SCHEMA_VERSION
        || bucket.workspace_id != workspace_id
        || bucket.page_id != page_id
        || bucket.key_high != high
    {
        return Err("receiver absence path bucket binding mismatch".into());
    }

    let Some(row_address) = reader
        .map_value(bucket.rows.decode()?, low)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let bytes = required_index_object(
        directory,
        staged,
        &record_object_name(ROW_PREFIX, row_address),
        counters,
        "decision row",
    )?;
    let record: ReceiverAbsenceRowRecord = decode_index_record(&bytes, row_address)?;
    if record.schema_version != ROW_RECORD_SCHEMA_VERSION
        || record.workspace_id != workspace_id
        || record.row.page_id != page_id
        || &record.row.path != path
        || record.row.anchors.is_empty()
        || record
            .row
            .anchors
            .iter()
            .any(|anchor| anchor.page_id != page_id || &anchor.path != path)
    {
        return Err("receiver absence row binding mismatch".into());
    }
    Ok(Some(record.row))
}

/// Install one exact (page, path) row, rewriting only the three fixed-shape
/// records on its own key path plus the O(log) map nodes above them.
fn upsert_row(
    directory: &Dir,
    published: &mut BTreeMap<String, Vec<u8>>,
    root: AuthenticatedMapRootV1,
    workspace_id: WorkspaceId,
    mut row: ReceiverAbsenceSummaryEntry,
    counters: &HistoryCounters,
) -> Result<AuthenticatedMapRootV1, String> {
    let page_id = row.page_id;
    let (high, low) = exact_path_key(&row.path);
    // Canonical order, so a row grown incrementally and the same row rebuilt
    // from retained receipts are the same bytes at the same address.
    row.anchors.sort_by(|left, right| {
        (left.intent_id, target_rank(left.target_kind))
            .cmp(&(right.intent_id, target_rank(right.target_kind)))
    });
    let (row_bytes, row_address) = encode_index_record(&ReceiverAbsenceRowRecord {
        schema_version: ROW_RECORD_SCHEMA_VERSION,
        workspace_id,
        row,
    })?;
    counters.note_write(row_bytes.len());
    published.insert(record_object_name(ROW_PREFIX, row_address), row_bytes);

    // The existing page index and bucket, read through this batch's own group
    // so a batch that touches one page twice composes instead of colliding.
    let (page_paths, bucket_rows) =
        read_page_levels(directory, Some(&*published), root, page_id, high, counters)?;

    let rows = upsert_map_level(
        directory,
        published,
        bucket_rows,
        low,
        row_address,
        counters,
    )?;
    let (bucket_bytes, bucket_address) = encode_index_record(&ReceiverAbsencePathBucket {
        schema_version: ROW_RECORD_SCHEMA_VERSION,
        workspace_id,
        page_id,
        key_high: high,
        rows: PersistedHistoryRoot::encode(rows)?,
    })?;
    counters.note_write(bucket_bytes.len());
    published.insert(
        record_object_name(BUCKET_PREFIX, bucket_address),
        bucket_bytes,
    );

    let paths = upsert_map_level(
        directory,
        published,
        page_paths,
        high,
        bucket_address,
        counters,
    )?;
    let (page_bytes, page_address) = encode_index_record(&ReceiverAbsencePageIndex {
        schema_version: ROW_RECORD_SCHEMA_VERSION,
        workspace_id,
        page_id,
        paths: PersistedHistoryRoot::encode(paths)?,
    })?;
    counters.note_write(page_bytes.len());
    published.insert(record_object_name(PAGE_PREFIX, page_address), page_bytes);

    upsert_map_level(
        directory,
        published,
        root,
        page_id.as_uuid().into_bytes(),
        page_address,
        counters,
    )
}

/// The current level-A `paths` root and level-B `rows` root for one key path.
fn read_page_levels(
    directory: &Dir,
    staged: Option<&BTreeMap<String, Vec<u8>>>,
    root: AuthenticatedMapRootV1,
    page_id: super::PageId,
    high: [u8; 16],
    counters: &HistoryCounters,
) -> Result<(AuthenticatedMapRootV1, AuthenticatedMapRootV1), String> {
    let store = SealedRowObjects {
        directory,
        staged,
        published: None,
        counters,
    };
    let reader = MapReader::new(&store);
    let Some(page_address) = reader
        .map_value(root, page_id.as_uuid().into_bytes())
        .map_err(|error| error.to_string())?
    else {
        return Ok((
            AuthenticatedMapRootV1::empty(),
            AuthenticatedMapRootV1::empty(),
        ));
    };
    let bytes = required_index_object(
        directory,
        staged,
        &record_object_name(PAGE_PREFIX, page_address),
        counters,
        "page index",
    )?;
    let page: ReceiverAbsencePageIndex = decode_index_record(&bytes, page_address)?;
    let paths = page.paths.decode()?;
    let Some(bucket_address) = reader
        .map_value(paths, high)
        .map_err(|error| error.to_string())?
    else {
        return Ok((paths, AuthenticatedMapRootV1::empty()));
    };
    let bytes = required_index_object(
        directory,
        staged,
        &record_object_name(BUCKET_PREFIX, bucket_address),
        counters,
        "path bucket",
    )?;
    let bucket: ReceiverAbsencePathBucket = decode_index_record(&bytes, bucket_address)?;
    Ok((paths, bucket.rows.decode()?))
}

fn upsert_map_level(
    directory: &Dir,
    published: &mut BTreeMap<String, Vec<u8>>,
    root: AuthenticatedMapRootV1,
    key: [u8; 16],
    value: ContentDigest,
    counters: &HistoryCounters,
) -> Result<AuthenticatedMapRootV1, String> {
    let mut store = SealedRowObjects {
        directory,
        staged: None,
        published: Some(published),
        counters,
    };
    MapWriter::new(&mut store)
        .upsert_map(root, key, value)
        .map_err(|error| error.to_string())
}

/// Shared read handle for the durable receiver history.
///
/// The absence-decision map holds this and nothing else for completed receiver
/// history: one exact (page, path) row is read on demand and dropped again, so
/// no point load accumulates in a second cache. The root is replaced only after
/// the objects it names are durable.
pub(crate) struct ReceiverAbsenceHistory {
    rows: Dir,
    chain: Dir,
    workspace_id: WorkspaceId,
    root: std::sync::RwLock<AuthenticatedMapRootV1>,
    counters: HistoryCounters,
    /// Set once a point read proved this activation's derived index damaged.
    ///
    /// The summary cache and the decision map share this one handle, so the
    /// producer learns about damage the consumer found. Retiring the chain is
    /// not enough on its own: later receipts in the same activation would
    /// republish two fresh, self-consistent chain objects over the retired
    /// ones, and the next open would then find an apparently healthy chain and
    /// cancel the rebuild the damage requires.
    damaged: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for ReceiverAbsenceHistory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiverAbsenceHistory")
            .field("pages", &self.root().count)
            .finish_non_exhaustive()
    }
}

impl ReceiverAbsenceHistory {
    fn root(&self) -> AuthenticatedMapRootV1 {
        *self
            .root
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn adopt_root(&self, root: AuthenticatedMapRootV1) {
        *self
            .root
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root;
    }

    /// Objects and bytes this handle has read and written. Diagnostic; the
    /// growth qualification asserts they stay flat as history grows.
    pub(crate) fn access(&self) -> HistoryAccess {
        self.counters.snapshot()
    }

    /// Has this activation already proved the derived index damaged?
    pub(crate) fn is_damaged(&self) -> bool {
        self.damaged.load(std::sync::atomic::Ordering::Acquire)
    }

    fn row(
        &self,
        page_id: super::PageId,
        path: &ManagedPath,
    ) -> Result<Option<ReceiverAbsenceSummaryEntry>, String> {
        lookup_row(
            &self.rows,
            None,
            self.root(),
            self.workspace_id,
            page_id,
            path,
            &self.counters,
        )
    }
}

impl ReceiverHistoryRead for ReceiverAbsenceHistory {
    fn receiver_row(
        &self,
        page_id: super::PageId,
        path: &ManagedPath,
    ) -> Result<Option<ReceiverAbsenceSummaryEntry>, ReceiverHistoryUnavailable> {
        self.row(page_id, path).map_err(ReceiverHistoryUnavailable)
    }

    /// Damaged derived rows retire the roots chain so the very next open takes
    /// the named, counted rebuild from retained receipts. The current caller
    /// still gets its named refusal (I-8); reopening heals (D-3, I-10).
    fn retire_damaged(&self) {
        self.damaged
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = clear_chain(&self.chain);
    }
}

/// One staged group of index updates. Committed as a single audited archive
/// publication, before the roots object that names its root.
struct RowBatch {
    staged: BTreeMap<String, Vec<u8>>,
    root: AuthenticatedMapRootV1,
}

/// Remove every derived index object. Used only by the instrumented rebuild,
/// which republishes the whole index from retained receipts.
fn clear_rows(directory: &Dir) -> Result<(), String> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 receiver absence row entry".to_owned())?;
        if [PAGE_PREFIX, BUCKET_PREFIX, ROW_PREFIX, NODE_PREFIX]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            names.push(name);
        }
    }
    for name in &names {
        match directory.remove_file(name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    if !names.is_empty() {
        sync_dir_required(directory).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        absence_decision::AbsenceDecision, BlobDescription, CrdtPeerCounter, CrdtPeerId,
        DocumentDependencies, DocumentId, FrontierV2, ManagedPath, PageId, ProjectionPrecondition,
        ProjectionTargetKind,
    };
    use crate::Graph;

    struct Fixture {
        root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        archive: ObjectStore,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let fixture = Self::build(label);
            fixture.attach_cursor();
            fixture
        }

        fn build(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tine-receiver-absence-summary-{label}-{}",
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(root.join("graph")).unwrap();
            let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(0xc6_1000));
            Self {
                graph: Graph::open(&root.join("graph")),
                receipts: ProjectionReceiptStore::open(&root.join("receipts"), workspace_id)
                    .unwrap(),
                archive: ObjectStore::open(&root.join("operations"), workspace_id).unwrap(),
                root,
            }
        }

        /// The same fixture with no cursor installed, used to attribute the
        /// cursor's own share of a receipt publication's barrier cost.
        fn without_cursor(label: &str) -> Self {
            Self::build(label)
        }

        /// Mirror the production managed open, which installs the durable
        /// current-action cursor before the actor can author a receipt.
        fn attach_cursor(&self) {
            self.receipts.attach_action_cursor(std::sync::Arc::new(
                ProjectionActionCursor::open(&self.archive).unwrap(),
            ));
        }

        fn copied_graph(label: &str, source: &PathBuf) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tine-receiver-absence-summary-{label}-{}",
                Uuid::new_v4()
            ));
            copy_tree_without_symlinks(source, &root.join("graph"));
            let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(0xc6_1000));
            let fixture = Self {
                graph: Graph::open(&root.join("graph")),
                receipts: ProjectionReceiptStore::open(&root.join("receipts"), workspace_id)
                    .unwrap(),
                archive: ObjectStore::open(&root.join("operations"), workspace_id).unwrap(),
                root,
            };
            fixture.attach_cursor();
            fixture
        }

        fn intent(
            &self,
            page: u128,
            path: &str,
            counter: u64,
            base: Option<&[u8]>,
            target: &[u8],
        ) -> ProjectionIntent {
            let frontier = FrontierV2::new(vec![DocumentDependencies::new(
                DocumentId::from_uuid(Uuid::from_u128(0xc6_1001)),
                vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(7), counter)],
                Vec::new(),
            )
            .unwrap()])
            .unwrap();
            ProjectionIntent::new(
                self.receipts.workspace_id(),
                PageId::from_uuid(Uuid::from_u128(page)),
                ManagedPath::parse(path).unwrap(),
                frontier,
                Vec::new(),
                base.map_or(ProjectionPrecondition::Absent, |bytes| {
                    ProjectionPrecondition::Base(BlobDescription::of(bytes))
                }),
                ProjectionTargetKind::Present,
                BlobDescription::of(target),
                Vec::new(),
            )
            .unwrap()
        }

        fn complete(&self, intent: &ProjectionIntent, base: Option<&[u8]>, target: &[u8]) {
            self.receipts.publish_intent(intent, base).unwrap();
            let reservation = self.receipts.reserve_attempt(intent).unwrap();
            let mut authority = self
                .receipts
                .begin_mutation(intent, Some(&reservation))
                .unwrap();
            let proof = self
                .graph
                .write_page_projection(intent.path().as_str(), base, target, &mut authority)
                .unwrap();
            self.receipts
                .publish_completion(authority, intent, &proof)
                .unwrap();
        }

        fn workspace_id(&self) -> WorkspaceId {
            self.receipts.workspace_id()
        }

        /// One intent with a chosen target kind and CRDT document, so a test
        /// can build comparable and incomparable frontiers on one key.
        fn kind_intent(
            &self,
            page: u128,
            path: &str,
            counter: u64,
            target_kind: ProjectionTargetKind,
            document: u128,
        ) -> ProjectionIntent {
            let frontier = FrontierV2::new(vec![DocumentDependencies::new(
                DocumentId::from_uuid(Uuid::from_u128(document)),
                vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(7), counter)],
                Vec::new(),
            )
            .unwrap()])
            .unwrap();
            ProjectionIntent::new(
                self.receipts.workspace_id(),
                PageId::from_uuid(Uuid::from_u128(page)),
                ManagedPath::parse(path).unwrap(),
                frontier,
                Vec::new(),
                ProjectionPrecondition::Absent,
                target_kind,
                match target_kind {
                    ProjectionTargetKind::Present => {
                        BlobDescription::of(format!("- {path} {counter}\n").as_bytes())
                    }
                    ProjectionTargetKind::Absent => BlobDescription::of(&[]),
                },
                Vec::new(),
            )
            .unwrap()
        }

        fn rows_dir(&self) -> PathBuf {
            self.archive
                .root_path()
                .join(ABSENCE_NAMESPACE)
                .join(ROWS_NAMESPACE)
        }

        fn summary_dir(&self) -> PathBuf {
            self.archive
                .root_path()
                .join(ABSENCE_NAMESPACE)
                .join(SUMMARY_NAMESPACE)
        }

        fn cursor_dir(&self) -> PathBuf {
            self.archive
                .root_path()
                .join(ABSENCE_NAMESPACE)
                .join("current-action-cursor-v1")
        }

        /// Every durable cursor mark filename, ascending by sequence.
        fn cursor_mark_files(&self) -> Vec<PathBuf> {
            let mut marks = std::fs::read_dir(self.cursor_dir())
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.ends_with(".intent") || name.ends_with(".completion")
                        })
                })
                .collect::<Vec<_>>();
            marks.sort();
            marks
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            crate::test_support::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn crash_before_summary_update_is_recovered_from_the_bounded_cursor() {
        let fixture = Fixture::new("crash-before-summary");
        let first_bytes = b"- first projected bytes\n";
        let first = fixture.intent(0xc6_1010, "pages/first.md", 1, None, first_bytes);
        fixture.complete(&first, None, first_bytes);
        let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(opened.stats.full_catalog_passes, 1);
        let stale_cache = opened.cache.expect("rebuild installs a summary");

        // The receipt lands durably; the process dies before the summary is
        // told. Exactly the write-ahead cursor mark survives.
        let second_bytes = b"- second projected bytes\n";
        let second = fixture.intent(0xc6_1020, "pages/second.md", 1, None, second_bytes);
        fixture.complete(&second, None, second_bytes);

        assert_eq!(
            stale_cache
                .materialized_map()
                .unwrap()
                .decision(second.page_id(), second.path())
                .unwrap(),
            AbsenceDecision::Create,
            "necessity: a structurally valid summary trusted without its cursor recreates"
        );
        drop(stale_cache);

        let healed = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            healed.stats.full_catalog_passes, 0,
            "one uncovered receipt is a point read, never a lifetime rebuild"
        );
        assert_eq!(
            healed.stats.evidence_names_observed, 0,
            "no receipt namespace is enumerated on a healthy open"
        );
        assert_eq!(
            healed.stats.cursor_marks_observed, 2,
            "both halves of the receipt reserved a sequence"
        );
        assert_eq!(healed.stats.delta_completions, 1);
        assert_eq!(healed.stats.receipt_content_reads, 2);
        assert_eq!(
            healed
                .map
                .decision(second.page_id(), second.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        drop(healed);

        // The mark is released, so the next open resolves nothing at all.
        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.cursor_marks_observed, 0);
        assert_eq!(steady.stats.receipt_content_reads, 0);
        assert_eq!(
            steady
                .map
                .decision(second.page_id(), second.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    #[test]
    fn a_durable_intent_without_completion_stays_actionable_without_a_rebuild() {
        let fixture = Fixture::new("actionable-intent");
        let first_bytes = b"- first projected bytes\n";
        let first = fixture.intent(0xc6_1040, "pages/first.md", 1, None, first_bytes);
        fixture.complete(&first, None, first_bytes);
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(built.stats.full_catalog_passes, 1);
        drop(built);

        // A receiver intent lands durably, then the process dies before any
        // summary update. Losing that pending obligation would silently drop
        // unfinished work.
        let pending_bytes = b"- pending projected bytes\n";
        let pending = fixture.intent(0xc6_1050, "pages/pending.md", 1, None, pending_bytes);
        fixture.receipts.publish_intent(&pending, None).unwrap();

        let healed = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            healed.stats.full_catalog_passes, 0,
            "an intent-only delta must not force a rebuild"
        );
        assert_eq!(healed.stats.evidence_names_observed, 0);
        assert_eq!(healed.stats.cursor_marks_observed, 1);
        assert_eq!(
            healed.stats.actionable_intents, 1,
            "the pending intent is the current-action root's receipt half"
        );
        assert_eq!(
            healed
                .map
                .incomplete_receiver_intents(pending.page_id(), pending.path()),
            vec![pending.clone()],
            "necessity: a producer blind to unsummarized intents silently drops pending work"
        );
        assert!(healed
            .map
            .receiver_history_key_present(&(pending.page_id(), pending.path().clone()))
            .unwrap());
        // The receipt half of the bounded retention closure: an unfinished
        // obligation pins its own intent, its page and its frontier's
        // documents, so a generation capture cannot relocate them out of reach.
        let closure = healed
            .cache
            .as_ref()
            .expect("the resumed open retains its roots")
            .current_action_roots()
            .retention_closure();
        assert!(closure.intents.contains(&pending.id().unwrap()));
        assert!(closure.pages.contains(&pending.page_id()));
        assert!(closure
            .documents
            .contains(&pending.frontier().documents()[0].document_id()));
        drop(healed);

        // Still owed after a clean reopen with no marks left: the obligation
        // is a persisted root, not a rediscovered filename.
        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.cursor_marks_observed, 0);
        assert_eq!(steady.stats.actionable_intents, 1);
        assert_eq!(
            steady
                .map
                .incomplete_receiver_intents(pending.page_id(), pending.path()),
            vec![pending]
        );
    }

    #[test]
    fn torn_summary_rebuilds_once_then_returns_to_the_steady_path() {
        let fixture = Fixture::new("torn-rebuild");
        let bytes = b"- durable projected bytes\n";
        let intent = fixture.intent(0xc6_1030, "pages/torn.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(built.stats.full_catalog_passes, 1);
        drop(built);

        let latest = std::fs::read_dir(fixture.summary_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(SUMMARY_PREFIX))
            })
            .max()
            .expect("summary generation exists");
        std::fs::write(&latest, b"{\"torn\":").unwrap();

        let rebuilt = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(rebuilt.stats.full_catalog_passes, 1);
        assert!(rebuilt.stats.rebuilt);
        assert_eq!(
            rebuilt
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        drop(rebuilt);

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.full_catalog_passes, 0);
        assert_eq!(steady.stats.delta_completions, 0);
        assert_eq!(steady.stats.evidence_names_observed, 0);
        assert_eq!(
            steady
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    /// A summary that outlives its retained completion keeps deferring.
    ///
    /// The lifetime evidence-filename horizon used to notice this for free,
    /// because it re-read every receipt name on every open. That is the term
    /// this packet removes, so the check is now bounded rather than free, and
    /// the guarantee is stated exactly:
    ///
    /// * With no usable cursor (generic/offline stores, or a damaged cursor),
    ///   the roots are rebuilt from retained truth and never decide from a
    ///   summary that is ahead of it.
    /// * With a healthy cursor the summary keeps its recorded completion and
    ///   answers `DeferredAbsence`. That is the conservative direction: it
    ///   declines to recreate a page, which is the resurrection this map
    ///   exists to prevent. The dangerous direction — a *missing* completion
    ///   turning into `Create` — is exactly what the write-ahead cursor makes
    ///   impossible, and `crash_before_summary_update_is_recovered_from_the_bounded_cursor`
    ///   pins it.
    ///
    /// Manager seam: verifying an anchor against retained truth belongs at
    /// decision time in `hot_engine::receiver_absence_decision`, where it is
    /// one point read for the one page being decided. `hot_engine.rs` is a
    /// forbidden owner region for this packet.
    #[test]
    fn summary_ahead_of_completion_truth_never_recreates_and_rebuilds_without_a_cursor() {
        let fixture = Fixture::new("ahead-rebuild");
        let bytes = b"- completion later lost\n";
        let intent = fixture.intent(0xc6_1035, "pages/ahead.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            built.map.decision(intent.page_id(), intent.path()).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        drop(built);

        std::fs::remove_file(
            fixture
                .receipts
                .root_path()
                .join("completions")
                .join(completion_filename(&intent).unwrap()),
        )
        .unwrap();

        let with_cursor =
            ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(with_cursor.stats.full_catalog_passes, 0);
        assert_eq!(
            with_cursor
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence,
            "losing a completion receipt must never resurrect the deleted page"
        );
        drop(with_cursor);

        // Without the cursor there is no bounded coverage proof, so the open
        // rebuilds from retained truth rather than trusting derived state.
        let store =
            ProjectionReceiptStore::open(&fixture.root.join("receipts"), fixture.workspace_id())
                .unwrap();
        let rebuilt = ReceiverAbsenceSummary::open(&fixture.archive, &store).unwrap();
        assert_eq!(rebuilt.stats.full_catalog_passes, 1);
        assert!(rebuilt.stats.cursor_unavailable);
        assert_eq!(
            rebuilt
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::Create,
            "a summary ahead of retained truth must never supply the decision"
        );
    }

    #[test]
    fn repeated_intent_and_completion_updates_install_no_duplicate_generation() {
        let fixture = Fixture::new("idempotent-update");
        let bytes = b"- idempotent completion\n";
        let intent = fixture.intent(0xc6_1038, "pages/idempotent.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let mut cache = opened.cache.expect("rebuild installs the cache");
        let before = enumerate_names(&cache.directory).unwrap().len();
        cache.record_intent(&intent).unwrap();
        cache.record_completion(&intent).unwrap();
        let after = enumerate_names(&cache.directory).unwrap().len();
        assert_eq!(after, before);
    }

    /// Fixed actionable work with increasing completed history.
    ///
    /// The measured quantities are the ones the retention census named: how
    /// many receipt filenames the open observes, how many receipt bodies it
    /// decodes, and how many records it keeps resident. All three stay flat
    /// while the completed chain grows.
    #[test]
    fn steady_open_cost_is_flat_while_completed_history_grows() {
        let fixture = Fixture::new("measured-cost");
        let mut prior: Option<Vec<u8>> = None;
        let mut short_steady = None;
        for generation in 1..=32_u64 {
            let target = format!("- projected generation {generation}\n").into_bytes();
            let intent = fixture.intent(
                0xc6_1040,
                "pages/history.md",
                generation,
                prior.as_deref(),
                &target,
            );
            fixture.complete(&intent, prior.as_deref(), &target);
            prior = Some(target);
            if generation == 4 {
                let built =
                    ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
                drop(built);
                let steady =
                    ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
                short_steady = Some((
                    steady.stats.evidence_names_observed,
                    steady.stats.receipt_content_reads,
                    steady.map.resident_row_count(),
                ));
            }
        }

        let rebuild = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(rebuild.stats.full_catalog_passes, 0);
        assert_eq!(rebuild.map.resident_row_count(), 0);
        drop(rebuild);

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let long = (
            steady.stats.evidence_names_observed,
            steady.stats.receipt_content_reads,
            steady.map.resident_row_count(),
        );
        assert_eq!(
            short_steady.expect("the short-history sample ran"),
            long,
            "ordinary open cost must not grow with completed receipt history"
        );
        assert_eq!(long.0, 0, "no receipt filename is enumerated");
        assert_eq!(long.1, 0, "no receipt body is decoded");
        assert_eq!(steady.stats.cursor_marks_observed, 0);
        assert!(steady.stats.summary_content_reads <= 2);
    }

    /// The named repair still exists, is counted, and never refuses.
    #[test]
    fn a_missing_roots_object_repairs_once_and_returns_to_the_bounded_path() {
        let fixture = Fixture::new("missing-roots-repair");
        let bytes = b"- repaired projection\n";
        let intent = fixture.intent(0xc6_1060, "pages/repair.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        drop(built);

        crate::test_support::remove_dir_all(&fixture.summary_dir());

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(repaired.stats.rebuilt);
        assert_eq!(repaired.stats.full_catalog_passes, 1);
        assert_eq!(repaired.stats.evidence_names_observed, 2);
        assert_eq!(
            repaired
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        drop(repaired);

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(!steady.stats.rebuilt);
        assert_eq!(steady.stats.full_catalog_passes, 0);
        assert_eq!(steady.stats.evidence_names_observed, 0);
        assert_eq!(
            steady
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    /// Manager negative control, integrated verbatim in behaviour.
    ///
    /// A summary covers one pending receipt; a second receipt is published and
    /// the cursor directory is then lost. Recreating the directory must not
    /// look like "everything is covered": the recreated cursor mints a new
    /// incarnation that no roots object claims, so the open repairs from
    /// retained receipts and both preserved receipts stay actionable.
    #[test]
    fn manager_missing_cursor_directory_preserves_pending_receipt_work() {
        use std::sync::Arc;
        let fixture = Fixture::new("manager-lost-cursor");
        let baseline = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(baseline.cache.is_some());
        assert_eq!(baseline.stats.actionable_intents, 0);
        drop(baseline);

        // Positive control: an intact cursor recovers a receipt published
        // after the previous summary, with no completion yet.
        let first = fixture.intent(0xaa01, "pages/first.md", 1, None, b"- first\n");
        fixture.receipts.publish_intent(&first, None).unwrap();
        let normal = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(normal.stats.actionable_intents, 1);
        assert_eq!(normal.stats.full_catalog_passes, 0);
        assert!(normal.cache.is_some());
        drop(normal);

        let second = fixture.intent(0xaa02, "pages/second.md", 2, None, b"- second\n");
        fixture.receipts.publish_intent(&second, None).unwrap();
        assert!(fixture
            .receipts
            .load_intent(second.id().unwrap())
            .unwrap()
            .is_some());
        crate::test_support::remove_dir_all(&fixture.cursor_dir());
        fixture.receipts.attach_action_cursor(Arc::new(
            ProjectionActionCursor::open(&fixture.archive).unwrap(),
        ));

        let recovered = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            recovered.stats.actionable_intents, 2,
            "missing derived cursor storage cannot silently hide the second preserved receipt"
        );
        assert!(
            recovered.stats.rebuilt && recovered.stats.full_catalog_passes == 1,
            "recovery is the named, counted repair: {:?}",
            recovered.stats
        );
        drop(recovered);

        // And the repair is once, not forever.
        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.full_catalog_passes, 0);
        assert_eq!(steady.stats.actionable_intents, 2);
    }

    /// Manager negative control, integrated: occupancy is not damage.
    ///
    /// A legitimately large uncovered window is ordinary paged work. It must
    /// not be classified as damage, must not trigger a full-history catalog
    /// pass, and must not lose a single pending receipt (D-5).
    #[test]
    fn manager_legitimate_cursor_backlog_is_not_damaged_state() {
        let fixture = Fixture::new("manager-large-cursor");
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        drop(built);

        let cursor = fixture
            .receipts
            .action_cursor()
            .expect("the fixture attaches a cursor");
        let backlog = 4_097_usize;
        let mut pending = Vec::with_capacity(backlog);
        for counter in 1..=backlog as u64 {
            let intent = fixture.intent(
                0xbb01,
                "pages/pending.md",
                counter,
                None,
                format!("- pending {counter}\n").as_bytes(),
            );
            fixture.receipts.publish_intent(&intent, None).unwrap();
            pending.push(intent);
        }
        let coverage = ReceiverAbsenceSummary::open_cache(
            &fixture.archive,
            fixture.receipts.action_cursor(),
            &mut ReceiverAbsenceSummaryOpenStats::default(),
        )
        .unwrap()
        .expect("a roots object exists")
        .cursor_coverage;
        let outstanding = cursor.uncovered_for(coverage);
        assert!(
            outstanding.is_ok(),
            "a legitimate uncovered receipt window must not be classified as damage merely \
             because it is large: {:?}",
            outstanding.err()
        );
        assert_eq!(outstanding.unwrap().len(), backlog);

        let resumed = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            resumed.stats.full_catalog_passes, 0,
            "a large backlog is streamed, never reconstructed from history"
        );
        assert_eq!(resumed.stats.evidence_names_observed, 0);
        assert_eq!(resumed.stats.cursor_marks_observed, backlog);
        assert_eq!(resumed.stats.actionable_intents, backlog);
        assert_eq!(
            resumed.stats.cursor_resume_installs,
            backlog.div_ceil(CURSOR_RESUME_CHUNK_MARKS),
            "catch-up installs durable progress per chunk"
        );
        let sample = pending.first().expect("the backlog is non-empty");
        assert_eq!(
            resumed
                .map
                .incomplete_receiver_intents(sample.page_id(), sample.path())
                .len(),
            backlog,
            "every pending receipt survives the backlog catch-up"
        );
    }

    /// A crash in the middle of a large catch-up resumes; it does not restart
    /// and it does not lose the receipts the finished chunks already folded.
    #[test]
    fn a_partial_backlog_catch_up_resumes_after_a_crash() {
        let fixture = Fixture::new("partial-catch-up");
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        drop(built);

        let backlog = CURSOR_RESUME_CHUNK_MARKS * 2 + 5;
        for counter in 1..=backlog as u64 {
            let intent = fixture.intent(
                0xbb02,
                "pages/partial.md",
                counter,
                None,
                format!("- partial {counter}\n").as_bytes(),
            );
            fixture.receipts.publish_intent(&intent, None).unwrap();
        }

        // The process dies after one durable chunk.
        let installs = ReceiverAbsenceSummary::resume_partially_for_test(
            &fixture.archive,
            &fixture.receipts,
            1,
        )
        .unwrap();
        assert_eq!(installs, 1);

        // A fresh cursor handle models the restart: the durable head survives,
        // so the incarnation is unchanged and only the unfolded tail is owed.
        fixture.attach_cursor();
        let resumed = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            resumed.stats.full_catalog_passes, 0,
            "a resumable catch-up never falls back to history: {:?}",
            resumed.stats
        );
        assert_eq!(
            resumed.stats.cursor_marks_observed,
            backlog - CURSOR_RESUME_CHUNK_MARKS,
            "the finished chunk stays covered across the crash"
        );
        assert_eq!(
            resumed.stats.actionable_intents, backlog,
            "no pending receipt is lost by resuming instead of restarting"
        );
    }

    /// One lost mark file is a detected gap, not silence.
    #[test]
    fn a_lost_individual_cursor_mark_repairs_instead_of_hiding_its_receipt() {
        let fixture = Fixture::new("lost-single-mark");
        let bytes = b"- covered\n";
        let covered = fixture.intent(0xcc01, "pages/covered.md", 1, None, bytes);
        fixture.complete(&covered, None, bytes);
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(built.stats.full_catalog_passes, 1);
        drop(built);

        let pending = fixture.intent(0xcc02, "pages/pending.md", 1, None, b"- pending\n");
        fixture.receipts.publish_intent(&pending, None).unwrap();
        let marks = fixture.cursor_mark_files();
        assert_eq!(marks.len(), 1, "exactly the uncovered reservation remains");
        std::fs::remove_file(&marks[0]).unwrap();

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(
            repaired.stats.rebuilt && repaired.stats.full_catalog_passes == 1,
            "a hole between the roots watermark and the reservation head is damage: {:?}",
            repaired.stats
        );
        assert_eq!(
            repaired.stats.actionable_intents, 1,
            "losing one discovery mark must not lose its preserved receipt"
        );
        assert_eq!(
            repaired
                .map
                .incomplete_receiver_intents(pending.page_id(), pending.path()),
            vec![pending]
        );
    }

    /// A torn mark fails its name/content binding and repairs.
    #[test]
    fn a_torn_cursor_mark_repairs_instead_of_hiding_its_receipt() {
        let fixture = Fixture::new("torn-single-mark");
        let bytes = b"- covered\n";
        let covered = fixture.intent(0xcd01, "pages/covered.md", 1, None, bytes);
        fixture.complete(&covered, None, bytes);
        drop(ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap());

        let pending = fixture.intent(0xcd02, "pages/pending.md", 1, None, b"- pending\n");
        fixture.receipts.publish_intent(&pending, None).unwrap();
        let marks = fixture.cursor_mark_files();
        assert_eq!(marks.len(), 1);
        std::fs::write(&marks[0], [0x00_u8; 32]).unwrap();

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(
            repaired.stats.rebuilt && repaired.stats.cursor_unavailable,
            "a mark whose content does not bind its name proves nothing: {:?}",
            repaired.stats
        );
        assert_eq!(repaired.stats.actionable_intents, 1);
    }

    /// The manager's exact second-crash window: the cursor directory is
    /// recreated, and the process dies again BEFORE the repair finishes.
    ///
    /// An in-memory `was_created` flag would be gone by then. The durable
    /// incarnation is not, so the next open still repairs and still recovers
    /// both preserved receipts.
    #[test]
    fn a_second_crash_between_cursor_recreation_and_repair_still_recovers() {
        use std::sync::Arc;
        let fixture = Fixture::new("second-crash");
        let first = fixture.intent(0xce01, "pages/first.md", 1, None, b"- first\n");
        fixture.receipts.publish_intent(&first, None).unwrap();
        let built = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(built.stats.actionable_intents, 1);
        drop(built);

        let second = fixture.intent(0xce02, "pages/second.md", 2, None, b"- second\n");
        fixture.receipts.publish_intent(&second, None).unwrap();
        crate::test_support::remove_dir_all(&fixture.cursor_dir());

        // Crash #1 recreated the cursor directory and died before repairing.
        let recreated = Arc::new(ProjectionActionCursor::open(&fixture.archive).unwrap());
        let first_incarnation = recreated.incarnation();
        drop(recreated);

        // Crash #2: another process, another handle, no in-memory state at all.
        let reopened = Arc::new(ProjectionActionCursor::open(&fixture.archive).unwrap());
        assert_eq!(
            reopened.incarnation(),
            first_incarnation,
            "a durable head is not re-minted on every open"
        );
        fixture.receipts.attach_action_cursor(reopened);

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            repaired.stats.actionable_intents, 2,
            "durable incarnation binding survives a crash inside the repair window"
        );
        assert!(repaired.stats.rebuilt);
    }

    /// Fixed active work, growing DISTINCT completed page identities.
    ///
    /// Repeating completions on one path only proves per-path overwrite, so
    /// every completion here retires a distinct page and path while the
    /// actionable set stays empty. All measured quantities are asserted flat:
    /// receipt filenames enumerated, receipt bodies decoded, roots objects
    /// read, cursor marks resolved, actionable obligations, history point
    /// reads — and, the term this pass exists to remove, the number of
    /// resident current-state rows the engine's decision map holds.
    ///
    /// Fail-before: with the completed decisions carried in the roots object
    /// and materialized into `AbsenceDecisionMap` (the pre-correction shape),
    /// the last column read 8 -> 64. It is now 0 -> 0, and each of those 64
    /// identities is still answered exactly, at one point read per question,
    /// after a reopen.
    #[test]
    fn steady_open_cost_is_flat_across_distinct_completed_page_identities() {
        let fixture = Fixture::new("distinct-identity-cost");
        let mut sample = Vec::new();
        let mut completed = Vec::new();
        for generation in 1..=64_u64 {
            let target = format!("- retired page {generation}\n").into_bytes();
            let intent = fixture.intent(
                0xc6_2000 + u128::from(generation),
                &format!("pages/retired-{generation}.md"),
                generation,
                None,
                &target,
            );
            fixture.complete(&intent, None, &target);
            completed.push(intent);
            if generation == 8 || generation == 64 {
                let built =
                    ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
                drop(built);
                let steady =
                    ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
                sample.push((
                    steady.stats.evidence_names_observed,
                    steady.stats.receipt_content_reads,
                    steady.stats.cursor_marks_observed,
                    steady.stats.actionable_intents,
                    steady.stats.full_catalog_passes,
                    steady.stats.summary_content_reads,
                    steady.stats.history_point_reads,
                    steady.map.resident_row_count(),
                ));
            }
        }
        let (short, long) = (sample[0], sample[1]);
        assert_eq!(
            (short.0, short.1, short.2, short.3, short.4),
            (0, 0, 0, 0, 0),
            "a steady open at short history reads no receipt name or body"
        );
        assert_eq!(
            (short.0, short.1, short.2, short.3, short.4, short.6, short.7),
            (long.0, long.1, long.2, long.3, long.4, long.6, long.7),
            "ordinary open work AND resident current-state rows must not grow with distinct \
             completed page identities: short={short:?} long={long:?}"
        );
        // Roots reads are 1 or 2 depending only on whether a previous chain
        // generation exists to cross-check; it is a chain property, not a
        // history term.
        assert!(short.5 <= 2 && long.5 <= 2, "roots reads stay bounded");
        assert!(short.6 <= 1 && long.6 <= 1, "one O(1) root probe per open");
        assert_eq!(
            long.7, 0,
            "completed receiver history must not be resident at all"
        );

        // Every one of the 64 distinct identities is still answered exactly,
        // after a reopen, from the point-addressable index — including the two
        // that never had receiver history at all.
        let reopened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let history = reopened
            .cache
            .as_ref()
            .expect("the steady open retains its roots")
            .history()
            .clone();
        let before = history.access();
        for intent in &completed {
            assert_eq!(
                reopened
                    .map
                    .decision(intent.page_id(), intent.path())
                    .unwrap(),
                AbsenceDecision::DeferredAbsence,
                "an old completed identity must still decide from its durable point row"
            );
        }
        let unknown_page = PageId::from_uuid(Uuid::from_u128(0xc6_2fff));
        let unknown_path = ManagedPath::parse("pages/never-projected.md").unwrap();
        assert_eq!(
            reopened.map.decision(unknown_page, &unknown_path).unwrap(),
            AbsenceDecision::Create,
            "a page the authenticated map proves absent creates"
        );
        // A page identity that shares a path with a completed one, and a path
        // that shares a page with a completed one, must stay distinguishable.
        let shared_path = completed[0].path().clone();
        assert_eq!(
            reopened.map.decision(unknown_page, &shared_path).unwrap(),
            AbsenceDecision::Create,
            "the row key is the full page identity, not the path alone"
        );
        assert_eq!(
            reopened
                .map
                .decision(completed[0].page_id(), &unknown_path)
                .unwrap(),
            AbsenceDecision::Create,
            "the row key is the full exact path, not the page alone"
        );
        let after = history.access();
        assert_eq!(
            after.point_lookups - before.point_lookups,
            completed.len() + 3,
            "each answer is exactly one exact-key point lookup; nothing scans"
        );
        assert!(
            after.objects_read - before.objects_read <= 12 * (completed.len() + 3),
            "each lookup reads a bounded number of small objects: {:?} -> {:?}",
            before,
            after
        );
        assert_eq!(
            reopened.map.resident_row_count(),
            0,
            "answering every historical identity must not accumulate resident rows"
        );
    }

    /// Fixed live page and fixed actionable work, growing DISTINCT completed
    /// *paths* on that one page.
    ///
    /// Manager correction of 2026-09-08: a page repeatedly renamed / moved away
    /// / moved back is fixed G and O with growing H, and a per-page vector of
    /// historical path rows would hide the whole H term inside one value — read
    /// in full on every point query and rewritten in full on every update.
    ///
    /// So this measures BYTES and OBJECTS, not calls: what one current decision
    /// reads and what one current update writes, at 8 distinct completed paths
    /// and at 64. Both stay flat to within the composed maps' O(log) depth, an
    /// older path is still answered exactly after reopen, and the resident
    /// current-state row count stays zero.
    #[test]
    fn one_page_with_many_completed_paths_keeps_decisions_and_updates_bounded() {
        fn measure(paths: u64) -> (HistoryAccess, HistoryAccess, usize, usize) {
            let fixture = Fixture::new(&format!("one-page-{paths}-paths"));
            let page = 0xc6_3000_u128;
            let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
            let mut cache = opened.cache.expect("the open installs roots");
            for index in 1..=paths {
                // Move away from the previous path, then take the next one:
                // exactly the rename/move-away/move-back shape.
                if index > 1 {
                    cache
                        .record_completion(&fixture.kind_intent(
                            page,
                            &format!("pages/renamed-{}.md", index - 1),
                            index * 2,
                            ProjectionTargetKind::Absent,
                            0xc6_1001,
                        ))
                        .unwrap();
                }
                cache
                    .record_completion(&fixture.kind_intent(
                        page,
                        &format!("pages/renamed-{index}.md"),
                        index * 2 + 1,
                        ProjectionTargetKind::Present,
                        0xc6_1001,
                    ))
                    .unwrap();
            }
            drop(cache);

            let reopened =
                ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
            assert_eq!(reopened.stats.full_catalog_passes, 0);
            let history = reopened
                .cache
                .as_ref()
                .expect("the steady open retains its roots")
                .history()
                .clone();

            // ONE current decision on the OLDEST completed path of this page.
            let oldest = ManagedPath::parse("pages/renamed-1.md").unwrap();
            let before = history.access();
            assert_eq!(
                reopened
                    .map
                    .decision(PageId::from_uuid(Uuid::from_u128(page)), &oldest)
                    .unwrap(),
                AbsenceDecision::Create,
                "the oldest path was moved away from, so recreation there is allowed"
            );
            let after_read = history.access();

            // ONE current update: a fresh completion on a brand-new path.
            let mut cache = reopened.cache.expect("roots are retained");
            let update_before = history.access();
            cache
                .record_completion(&fixture.kind_intent(
                    page,
                    "pages/renamed-next.md",
                    4_000,
                    ProjectionTargetKind::Present,
                    0xc6_1001,
                ))
                .unwrap();
            let after_write = history.access();

            (
                HistoryAccess {
                    point_lookups: after_read.point_lookups - before.point_lookups,
                    objects_read: after_read.objects_read - before.objects_read,
                    bytes_read: after_read.bytes_read - before.bytes_read,
                    objects_written: 0,
                    bytes_written: 0,
                },
                HistoryAccess {
                    point_lookups: after_write.point_lookups - update_before.point_lookups,
                    objects_read: after_write.objects_read - update_before.objects_read,
                    bytes_read: after_write.bytes_read - update_before.bytes_read,
                    objects_written: after_write.objects_written - update_before.objects_written,
                    bytes_written: after_write.bytes_written - update_before.bytes_written,
                },
                reopened.map.resident_row_count(),
                reopened.stats.history_point_reads,
            )
        }

        let (short_read, short_write, short_resident, short_probe) = measure(8);
        let (long_read, long_write, long_resident, long_probe) = measure(64);
        eprintln!("one-page decision at 8 paths: {short_read:?}");
        eprintln!("one-page decision at 64 paths: {long_read:?}");
        eprintln!("one-page update at 8 paths: {short_write:?}");
        eprintln!("one-page update at 64 paths: {long_write:?}");

        assert_eq!(short_read.point_lookups, 1);
        assert_eq!(long_read.point_lookups, 1);
        assert_eq!(
            (short_resident, long_resident),
            (0, 0),
            "no historical path row is resident at either history length"
        );
        assert_eq!(
            (short_probe, long_probe),
            (1, 1),
            "one O(1) root probe per open regardless of how many paths exist"
        );
        assert!(
            long_read.bytes_read <= 2 * short_read.bytes_read,
            "one current decision must not read bytes proportional to the page's completed \
             path history: 8 paths read {} bytes, 64 paths read {} bytes",
            short_read.bytes_read,
            long_read.bytes_read
        );
        assert!(
            long_write.bytes_written <= 2 * short_write.bytes_written,
            "one current update must not rewrite bytes proportional to the page's completed \
             path history: 8 paths wrote {} bytes, 64 paths wrote {} bytes",
            short_write.bytes_written,
            long_write.bytes_written
        );
        assert!(
            long_read.objects_read <= 2 * short_read.objects_read
                && long_write.objects_written <= 2 * short_write.objects_written,
            "object counts grow only with the composed maps' depth: read {} -> {}, \
             written {} -> {}",
            short_read.objects_read,
            long_read.objects_read,
            short_write.objects_written,
            long_write.objects_written
        );
    }

    /// Exact older-path identity survives a reopen, with the frontier semantics
    /// each path earned in its own right.
    #[test]
    fn an_older_path_of_a_renamed_page_is_answered_exactly_after_reopen() {
        let fixture = Fixture::new("older-path-identity");
        let page = 0xc6_3100_u128;
        let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let mut cache = opened.cache.expect("the open installs roots");

        // first.md: present, then moved away.
        cache
            .record_completion(&fixture.kind_intent(
                page,
                "pages/first.md",
                1,
                ProjectionTargetKind::Present,
                0xc6_1001,
            ))
            .unwrap();
        cache
            .record_completion(&fixture.kind_intent(
                page,
                "pages/first.md",
                2,
                ProjectionTargetKind::Absent,
                0xc6_1001,
            ))
            .unwrap();
        // second.md: absent, then moved back in — a restored generation.
        cache
            .record_completion(&fixture.kind_intent(
                page,
                "pages/second.md",
                3,
                ProjectionTargetKind::Absent,
                0xc6_1001,
            ))
            .unwrap();
        cache
            .record_completion(&fixture.kind_intent(
                page,
                "pages/second.md",
                4,
                ProjectionTargetKind::Present,
                0xc6_1001,
            ))
            .unwrap();
        // third.md: current, present only.
        cache
            .record_completion(&fixture.kind_intent(
                page,
                "pages/third.md",
                5,
                ProjectionTargetKind::Present,
                0xc6_1001,
            ))
            .unwrap();
        // A case-variant of an existing path must stay a DIFFERENT identity.
        let variant = ManagedPath::parse("pages/First.md").unwrap();
        drop(cache);

        let reopened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let page_id = PageId::from_uuid(Uuid::from_u128(page));
        let first = ManagedPath::parse("pages/first.md").unwrap();
        let second = ManagedPath::parse("pages/second.md").unwrap();
        let third = ManagedPath::parse("pages/third.md").unwrap();

        assert_eq!(
            reopened.map.decision(page_id, &first).unwrap(),
            AbsenceDecision::Create,
            "an older path this page moved away from stays recreatable"
        );
        assert!(!reopened
            .map
            .restored_generation_requires_deferral(page_id, &first)
            .unwrap());
        assert_eq!(
            reopened.map.decision(page_id, &second).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        assert!(
            reopened
                .map
                .restored_generation_requires_deferral(page_id, &second)
                .unwrap(),
            "the older path's restored generation keeps its own sticky bit"
        );
        assert_eq!(
            reopened.map.decision(page_id, &third).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        assert_eq!(
            reopened.map.decision(page_id, &variant).unwrap(),
            AbsenceDecision::Create,
            "the key is the exact path: a case variant is a different identity"
        );
        assert_eq!(reopened.map.resident_row_count(), 0);
    }

    /// Manager-supplied regression: a later unrelated completion must not
    /// republish known-damaged roots as healthy.
    ///
    /// Retiring the chain on damage is not sufficient by itself. Two further
    /// receipts in the same activation restore the ordinary two-object chain,
    /// and the next open then reads a self-consistent roots object, reports
    /// `full_catalog_passes = 0`, and cancels the rebuild the damage requires —
    /// leaving the missing row permanently unreadable. The shared damaged flag
    /// on the one history handle closes that hole: once damage is proven, this
    /// activation publishes nothing more over it.
    #[test]
    fn manager_damaged_row_stays_repairable_after_unrelated_completion() {
        let fixture = Fixture::new("manager-damage-then-other-completion");
        let bytes_a = b"- retained original a\n";
        let a = fixture.intent(0xc6_30a0, "pages/old-a.md", 1, None, bytes_a);
        fixture.complete(&a, None, bytes_a);
        drop(ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap());
        let mut opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let rows = std::fs::read_dir(fixture.rows_dir())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(ROW_PREFIX)
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        std::fs::remove_file(&rows[0]).unwrap();
        assert!(opened.map.decision(a.page_id(), a.path()).is_err());
        opened.map.retire_damaged_history();
        // The graph remains open after refusing one damaged point read. A
        // different receiver completion must not cancel the required repair.
        let bytes_b = b"- retained original b\n";
        let b = fixture.intent(0xc6_30b0, "pages/new-b.md", 1, None, bytes_b);
        fixture.complete(&b, None, bytes_b);
        let _update = opened.cache.as_mut().unwrap().record_completion(&b);
        // Two later publications restore the usual two-element chain even
        // if the first one was recognizably torn after invalidation.
        let bytes_c = b"- retained original c\n";
        let c = fixture.intent(0xc6_30c0, "pages/new-c.md", 1, None, bytes_c);
        fixture.complete(&c, None, bytes_c);
        let _update = opened.cache.as_mut().unwrap().record_completion(&c);
        drop(opened);
        let reopened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(
            reopened.stats.full_catalog_passes, 1,
            "an unrelated completion must not republish known-damaged roots as healthy: {:?}",
            reopened.stats
        );
        assert_eq!(
            reopened.map.decision(a.page_id(), a.path()).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        assert_eq!(
            reopened.map.decision(b.page_id(), b.path()).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    /// A damaged derived row is a named repair, never a resurrection.
    ///
    /// The row object the authenticated map names is deleted. The decision must
    /// refuse by name rather than answering `Create`, the roots must be retired
    /// so the very next open runs the counted rebuild, and after that rebuild
    /// the page must still decide `DeferredAbsence` — the deleted page is not
    /// recreated at any point in the sequence.
    #[test]
    fn a_damaged_history_row_repairs_and_never_resurrects_its_page() {
        let fixture = Fixture::new("damaged-history-row");
        let bytes = b"- durable receiver page\n";
        let intent = fixture.intent(0xc6_20a0, "pages/damaged.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        drop(ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap());

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.full_catalog_passes, 0);
        assert_eq!(
            steady
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );

        let rows = std::fs::read_dir(fixture.rows_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(ROW_PREFIX))
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1, "one completed page, one durable row object");
        std::fs::remove_file(&rows[0]).unwrap();

        let error = steady
            .map
            .decision(intent.page_id(), intent.path())
            .unwrap_err();
        assert!(
            error.to_string().contains("missing row object"),
            "a row the map claims but cannot read must name its damage: {error}"
        );
        steady.map.retire_damaged_history();
        drop(steady);

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(
            repaired.stats.rebuilt && repaired.stats.full_catalog_passes == 1,
            "damaged derived rows take the named, counted rebuild: {:?}",
            repaired.stats
        );
        assert_eq!(
            repaired
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence,
            "the rebuilt index must not resurrect the receiver-deleted page"
        );
        drop(repaired);

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(steady.stats.full_catalog_passes, 0, "the repair is once");
        assert_eq!(
            steady
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    /// A damaged authenticated-map node is caught by the O(1) root probe at
    /// open, so a wholly unreadable index heals without ever being consulted.
    #[test]
    fn a_damaged_history_root_node_repairs_at_open() {
        let fixture = Fixture::new("damaged-history-node");
        let bytes = b"- node damage\n";
        let intent = fixture.intent(0xc6_20b0, "pages/node.md", 1, None, bytes);
        fixture.complete(&intent, None, bytes);
        drop(ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap());
        drop(ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap());

        for entry in std::fs::read_dir(fixture.rows_dir()).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(NODE_PREFIX))
            {
                std::fs::remove_file(&path).unwrap();
            }
        }

        let repaired = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert!(repaired.stats.rebuilt);
        assert_eq!(repaired.stats.full_catalog_passes, 1);
        assert!(repaired
            .stats
            .repair_cause
            .as_deref()
            .is_some_and(|cause| cause.contains("root node")));
        assert_eq!(
            repaired
                .map
                .decision(intent.page_id(), intent.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
    }

    /// Present / Absent / concurrent-frontier antichain / restored-generation
    /// deferral — every one answered from the durable point rows after a
    /// reopen, with nothing resident.
    #[test]
    fn point_rows_answer_present_absent_concurrent_and_restore_deferral() {
        let fixture = Fixture::new("point-row-semantics");
        let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        let mut cache = opened.cache.expect("the open installs roots");

        // Present, then a strictly later Absent: the page was deleted after it
        // existed, so recreation is allowed.
        let deleted_present = fixture.kind_intent(
            0xc6_20c0,
            "pages/deleted.md",
            1,
            ProjectionTargetKind::Present,
            0xc6_1001,
        );
        let deleted_absent = fixture.kind_intent(
            0xc6_20c0,
            "pages/deleted.md",
            5,
            ProjectionTargetKind::Absent,
            0xc6_1001,
        );

        // Absent, then a strictly later Present: a restored generation.
        let restored_absent = fixture.kind_intent(
            0xc6_20d0,
            "pages/restored.md",
            1,
            ProjectionTargetKind::Absent,
            0xc6_1001,
        );
        let restored_present = fixture.kind_intent(
            0xc6_20d0,
            "pages/restored.md",
            5,
            ProjectionTargetKind::Present,
            0xc6_1001,
        );

        // Present only.
        let kept = fixture.kind_intent(
            0xc6_20e0,
            "pages/kept.md",
            1,
            ProjectionTargetKind::Present,
            0xc6_1001,
        );

        // Concurrent, incomparable Present/Absent on one key.
        let concurrent_present = fixture.kind_intent(
            0xc6_20f0,
            "pages/concurrent.md",
            3,
            ProjectionTargetKind::Present,
            0xc6_1001,
        );
        let concurrent_absent = fixture.kind_intent(
            0xc6_20f0,
            "pages/concurrent.md",
            3,
            ProjectionTargetKind::Absent,
            0xc6_1002,
        );

        for intent in [
            &deleted_present,
            &deleted_absent,
            &restored_absent,
            &restored_present,
            &kept,
            &concurrent_present,
            &concurrent_absent,
        ] {
            cache.record_completion(intent).unwrap();
        }
        drop(cache);

        let reopened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        assert_eq!(reopened.stats.full_catalog_passes, 0);
        assert_eq!(
            reopened
                .map
                .decision(deleted_present.page_id(), deleted_present.path())
                .unwrap(),
            AbsenceDecision::Create,
            "a maximal Absent completion allows recreation"
        );
        assert_eq!(
            reopened
                .map
                .decision(restored_present.page_id(), restored_present.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        assert!(
            reopened
                .map
                .restored_generation_requires_deferral(
                    restored_present.page_id(),
                    restored_present.path()
                )
                .unwrap(),
            "the sticky restored-generation bit survives on the durable row"
        );
        assert!(!reopened
            .map
            .restored_generation_requires_deferral(kept.page_id(), kept.path())
            .unwrap());
        assert_eq!(
            reopened.map.decision(kept.page_id(), kept.path()).unwrap(),
            AbsenceDecision::DeferredAbsence
        );
        assert_eq!(
            reopened
                .map
                .decision(concurrent_present.page_id(), concurrent_present.path())
                .unwrap(),
            AbsenceDecision::DeferredAbsence,
            "an incomparable mixed antichain takes the reversible direction"
        );
        assert_eq!(
            reopened.map.resident_row_count(),
            0,
            "none of these answers made history resident"
        );
    }

    /// The exact foreground durability cost the current-action cursor adds to
    /// one receiver receipt publication.
    ///
    /// A barrier is a device round trip, so the *count* is the thing that
    /// scales with the user's hardware — see `durability_counters`. This pins
    /// the production sequence `projection.rs` runs for one inbound page:
    /// publish intent, fold it, publish completion, fold it. Every barrier is
    /// attributed, and the cursor's own share is measured separately by
    /// running the identical sequence on a store with no cursor attached.
    ///
    /// The cursor coalesces: its mark and its advanced reservation head are
    /// staged into ONE audited batch publication, and reclamation of covered
    /// marks takes no barrier at all, so a reservation costs exactly what the
    /// mark alone would.
    ///
    /// Own-endpoint saves publish no receipt artifacts at all
    /// (`own_endpoint_save_and_move_author_no_receipt_artifacts`), so this
    /// cost lands on the sync-inbound path only and the pinned managed-save
    /// and managed-move budgets are unchanged.
    #[test]
    fn one_receiver_receipt_publication_has_a_pinned_barrier_cost() {
        fn measure(with_cursor: bool, label: &str) -> u64 {
            let fixture = if with_cursor {
                Fixture::new(label)
            } else {
                Fixture::without_cursor(label)
            };
            let bytes = b"- measured receiver page\n";
            let intent = fixture.intent(0xc6_1080, "pages/measured.md", 1, None, bytes);
            let opened = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
            let mut cache = opened.cache.expect("the open installs roots");

            let session = crate::durability_counters::BarrierSession::begin();
            fixture.receipts.publish_intent(&intent, None).unwrap();
            cache.record_intent(&intent).unwrap();
            let reservation = fixture.receipts.reserve_attempt(&intent).unwrap();
            let mut authority = fixture
                .receipts
                .begin_mutation(&intent, Some(&reservation))
                .unwrap();
            let proof = fixture
                .graph
                .write_page_projection(intent.path().as_str(), None, bytes, &mut authority)
                .unwrap();
            fixture
                .receipts
                .publish_completion(authority, &intent, &proof)
                .unwrap();
            cache.record_completion(&intent).unwrap();
            let measured = session.counts();
            crate::durability_counters::BarrierSession::detach_current_thread();
            eprintln!("receiver receipt barriers ({label}): {}", measured.report());
            measured.total()
        }

        let without = measure(false, "barrier-no-cursor");
        let with = measure(true, "barrier-with-cursor");
        assert_eq!(
            with - without,
            4,
            "the current-action cursor adds exactly two coalesced publications \
             (one per receipt half) to a receiver receipt: with={with} without={without}"
        );
        assert_eq!(
            without, 27,
            "the receipt path without the cursor: 25 before this pass plus the one \
             coalesced page-row publication the point-addressable index adds"
        );
        assert_eq!(
            with, RECEIVER_RECEIPT_BARRIER_BUDGET,
            "one receiver receipt publication performed {with} core-initiated durability \
             barriers against the pinned budget {RECEIVER_RECEIPT_BARRIER_BUDGET}; any drift \
             must be attributed before this ledger changes"
        );
    }

    #[test]
    #[ignore = "manual measured gate: anonymized-corpus copy plus long receiver history"]
    fn anonymized_corpus_copy_receiver_summary_cost_probe() {
        let source = PathBuf::from(
            std::env::var_os("TINE_MS_AUDIT_GRAPH_COPY")
                .expect("TINE_MS_AUDIT_GRAPH_COPY must name the read-only anonymized corpus"),
        );
        let fixture = Fixture::copied_graph("corpus-cost", &source);
        let mut prior: Option<Vec<u8>> = None;
        for generation in 1..=512_u64 {
            let target = format!("- derived summary cost generation {generation}\n").into_bytes();
            let intent = fixture.intent(
                0xc6_1050,
                "pages/derived-summary-cost-probe.md",
                generation,
                prior.as_deref(),
                &target,
            );
            fixture.complete(&intent, prior.as_deref(), &target);
            prior = Some(target);
        }

        let rebuild = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        eprintln!("receiver-summary rebuild: {:?}", rebuild.stats);
        assert_eq!(rebuild.stats.full_catalog_passes, 1);
        assert_eq!(rebuild.stats.evidence_names_observed, 1024);
        assert_eq!(rebuild.map.resident_row_count(), 0);
        drop(rebuild);

        let steady = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        eprintln!("receiver-summary steady: {:?}", steady.stats);
        assert_eq!(steady.stats.full_catalog_passes, 0);
        assert_eq!(steady.stats.receipt_content_reads, 0);
        assert!(steady.stats.summary_content_reads <= 2);
        assert_eq!(steady.map.resident_row_count(), 0);
        drop(steady);

        crate::test_support::remove_dir_all(&fixture.summary_dir());
        let missing = ReceiverAbsenceSummary::open(&fixture.archive, &fixture.receipts).unwrap();
        eprintln!(
            "receiver-summary missing-cache rebuild: {:?}",
            missing.stats
        );
        assert_eq!(missing.stats.full_catalog_passes, 1);
        assert_eq!(missing.map.resident_row_count(), 0);
    }

    fn copy_tree_without_symlinks(source: &PathBuf, destination: &PathBuf) {
        let metadata = std::fs::symlink_metadata(source).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "corpus copy refuses symlinks"
        );
        if metadata.is_dir() {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                copy_tree_without_symlinks(&entry.path(), &destination.join(entry.file_name()));
            }
        } else {
            std::fs::copy(source, destination).unwrap();
        }
    }
}
