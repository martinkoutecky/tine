//! Inactive reconciliation scheduling and scan substrate.
//!
//! This module deliberately has no production caller and exposes no write,
//! import, projection, watcher, or continuing filesystem authority. A stable
//! epoch is only a bounded candidate set for a later point-revalidated packet.

use super::{
    hot_engine::{
        CurrentPathCatalogBinding, CurrentPathCatalogCursor, CurrentPathCatalogRow,
        ShardedHotEngine, MAX_CURRENT_PATH_CURSOR_PAGE_ROWS,
    },
    projection_work_index::{
        ProjectionExpectedPathHead, ProjectionExpectedPathReadBudget, ProjectionWorkError,
        ProjectionWorkIndex,
    },
    BlobDescription, CanonicalGraphResourceId, ContentDigest, ManagedPath, ManagedTextKind, PageId,
    PortablePathKey, ProjectionWorkTarget,
};
use crate::graph_text_scope::GraphTextScopeBinding;
use crate::model::Graph;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) const GRAPH_TEXT_SCAN_READ_BUFFER_BYTES: usize = 64 * 1024;
pub(crate) const GRAPH_TEXT_EXPECTED_PAGE_ROWS: usize = 256;
pub(crate) const GRAPH_TEXT_EXPECTED_PAGE_BYTES: u64 = 1024 * 1024;
const MAX_SCAN_RETAINED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCAN_EXPECTED_PATHS: usize = 1_000_000;
const MAX_SCAN_EXACT_PATH_BYTES: usize = 4096;

/// Bounded, single-flight scheduling limits for one graph endpoint.
///
/// These limits bound only retained scheduler state. Callers may receive a
/// larger watcher batch, but any path which cannot be retained turns that
/// batch into a safe full-scan request instead of being silently forgotten.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationSchedulerLimits {
    pub(crate) maximum_watcher_paths: usize,
    pub(crate) maximum_watcher_path_bytes: usize,
    pub(crate) maximum_precondition_paths: usize,
    pub(crate) maximum_precondition_path_bytes: usize,
    pub(crate) maximum_full_scan_reasons: usize,
}

impl Default for ReconciliationSchedulerLimits {
    fn default() -> Self {
        Self {
            maximum_watcher_paths: 1024,
            maximum_watcher_path_bytes: 128 * 1024,
            maximum_precondition_paths: 1024,
            maximum_precondition_path_bytes: 128 * 1024,
            maximum_full_scan_reasons: 8,
        }
    }
}

/// A freshness signal. It does not grant filesystem, graph, or write
/// authority; the future executor must recapture every selected path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationTrigger {
    WatcherPaths(BTreeSet<ManagedPath>),
    WatcherUncertain,
    Startup,
    Periodic,
    Explicit,
    ProjectionPreconditionMismatch(BTreeSet<ManagedPath>),
    BaselineUnavailable,
}

/// Bounded diagnostics retained on one coalesced full scan.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ReconciliationFullScanReason {
    Explicit,
    WatcherUncertain,
    Startup,
    BaselineUnavailable,
    Periodic,
    WatcherPathOverflow,
    ProjectionPreconditionPathOverflow,
    Retry,
    Uncertain,
    Cancelled,
}

/// The diagnostic reasons carried by a full scan job.
///
/// `omitted_reasons` means the configured diagnostic bound was reached. It
/// never means that work was dropped: the job remains a full scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationFullScanReasons {
    pub(crate) reasons: BTreeSet<ReconciliationFullScanReason>,
    pub(crate) omitted_reasons: bool,
}

/// The only work a scheduler may ask a later executor to perform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationWork {
    ProjectionPreconditionMismatch { paths: BTreeSet<ManagedPath> },
    FullScan(ReconciliationFullScanReasons),
    WatcherPaths { paths: BTreeSet<ManagedPath> },
}

/// Opaque identity for exactly one started reconciliation job.
///
/// The fields intentionally stay module-private so another endpoint cannot
/// forge a completion token. Cloning this value is harmless: it remains the
/// same exact lease, not a new lease.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ReconciliationLease {
    scheduler_id: u64,
    sequence: u64,
}

/// Immutable work and its exact completion lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationJob {
    lease: ReconciliationLease,
    work: ReconciliationWork,
}

impl ReconciliationJob {
    pub(crate) fn lease(&self) -> ReconciliationLease {
        self.lease
    }

    pub(crate) fn work(&self) -> &ReconciliationWork {
        &self.work
    }
}

/// Terminal result reported by a future scan/coordinator executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationCompletionOutcome {
    Noop,
    Complete,
    Blocked,
    Retry,
    Uncertain,
    Cancelled,
    Shutdown,
}

/// Rejected completions never affect scheduler state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationCompletionError {
    NoActiveJob,
    StaleOrForeignLease,
}

/// The last blocked job remains visible until a later successful completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationBlockedStatus {
    pub(crate) lease: ReconciliationLease,
}

/// A compact state snapshot; there is intentionally no inferred "clean"
/// state. A blocked result stays represented even if fresh work is queued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationSchedulerStatus {
    pub(crate) active: bool,
    pub(crate) pending: bool,
    pub(crate) blocked: Option<ReconciliationBlockedStatus>,
    pub(crate) last_completion: Option<ReconciliationCompletionOutcome>,
}

#[derive(Default)]
struct PendingReconciliation {
    precondition_paths: BTreeSet<ManagedPath>,
    precondition_path_bytes: usize,
    watcher_paths: BTreeSet<ManagedPath>,
    watcher_path_bytes: usize,
    full_scan_reasons: BTreeSet<ReconciliationFullScanReason>,
    omitted_full_scan_reasons: bool,
}

impl PendingReconciliation {
    fn retain_precondition_paths(
        &mut self,
        paths: BTreeSet<ManagedPath>,
        limits: ReconciliationSchedulerLimits,
    ) -> bool {
        retain_bounded_paths(
            &mut self.precondition_paths,
            &mut self.precondition_path_bytes,
            paths,
            limits.maximum_precondition_paths,
            limits.maximum_precondition_path_bytes,
        )
    }

    fn retain_watcher_paths(
        &mut self,
        paths: BTreeSet<ManagedPath>,
        limits: ReconciliationSchedulerLimits,
    ) -> bool {
        retain_bounded_paths(
            &mut self.watcher_paths,
            &mut self.watcher_path_bytes,
            paths,
            limits.maximum_watcher_paths,
            limits.maximum_watcher_path_bytes,
        )
    }

    fn request_full_scan(
        &mut self,
        reason: ReconciliationFullScanReason,
        limits: ReconciliationSchedulerLimits,
    ) {
        self.watcher_paths.clear();
        self.watcher_path_bytes = 0;
        if self.full_scan_reasons.contains(&reason) {
            return;
        }
        if self.full_scan_reasons.len() < limits.maximum_full_scan_reasons {
            self.full_scan_reasons.insert(reason);
        } else {
            self.omitted_full_scan_reasons = true;
        }
    }

    fn has_full_scan(&self) -> bool {
        !self.full_scan_reasons.is_empty() || self.omitted_full_scan_reasons
    }

    fn has_pending_work(&self) -> bool {
        !self.precondition_paths.is_empty()
            || self.has_full_scan()
            || !self.watcher_paths.is_empty()
    }

    fn take_next_work(&mut self) -> Option<ReconciliationWork> {
        if !self.precondition_paths.is_empty() {
            self.precondition_path_bytes = 0;
            return Some(ReconciliationWork::ProjectionPreconditionMismatch {
                paths: mem::take(&mut self.precondition_paths),
            });
        }
        if self.has_full_scan() {
            let reasons = ReconciliationFullScanReasons {
                reasons: mem::take(&mut self.full_scan_reasons),
                omitted_reasons: mem::replace(&mut self.omitted_full_scan_reasons, false),
            };
            return Some(ReconciliationWork::FullScan(reasons));
        }
        if !self.watcher_paths.is_empty() {
            self.watcher_path_bytes = 0;
            return Some(ReconciliationWork::WatcherPaths {
                paths: mem::take(&mut self.watcher_paths),
            });
        }
        None
    }
}

fn retain_bounded_paths(
    retained: &mut BTreeSet<ManagedPath>,
    retained_path_bytes: &mut usize,
    paths: BTreeSet<ManagedPath>,
    maximum_paths: usize,
    maximum_path_bytes: usize,
) -> bool {
    let mut overflowed = false;
    for path in paths {
        if retained.contains(&path) {
            continue;
        }
        let path_bytes = path.as_str().len();
        if retained.len() >= maximum_paths
            || path_bytes > maximum_path_bytes.saturating_sub(*retained_path_bytes)
        {
            overflowed = true;
            continue;
        }
        *retained_path_bytes = retained_path_bytes.saturating_add(path_bytes);
        retained.insert(path);
    }
    overflowed
}

static NEXT_RECONCILIATION_SCHEDULER_ID: AtomicU64 = AtomicU64::new(1);

/// A bounded, deterministic, in-memory single-flight scheduler for one
/// enrolled graph endpoint. It owns only discovery work hints, never Graph
/// truth or filesystem authority.
pub(crate) struct ReconciliationScheduler {
    limits: ReconciliationSchedulerLimits,
    scheduler_id: u64,
    next_lease_sequence: u64,
    pending: PendingReconciliation,
    active: Option<ReconciliationLease>,
    blocked: Option<ReconciliationBlockedStatus>,
    last_completion: Option<ReconciliationCompletionOutcome>,
}

impl ReconciliationScheduler {
    pub(crate) fn new(limits: ReconciliationSchedulerLimits) -> Self {
        Self {
            limits,
            scheduler_id: NEXT_RECONCILIATION_SCHEDULER_ID.fetch_add(1, Ordering::Relaxed),
            next_lease_sequence: 0,
            pending: PendingReconciliation::default(),
            active: None,
            blocked: None,
            last_completion: None,
        }
    }

    /// Retain a coalesced discovery hint. A path which cannot fit is converted
    /// into full work before this call returns.
    pub(crate) fn trigger(&mut self, trigger: ReconciliationTrigger) {
        match trigger {
            ReconciliationTrigger::WatcherPaths(paths) => {
                if self.pending.retain_watcher_paths(paths, self.limits) {
                    self.pending.request_full_scan(
                        ReconciliationFullScanReason::WatcherPathOverflow,
                        self.limits,
                    );
                }
            }
            ReconciliationTrigger::WatcherUncertain => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::WatcherUncertain, self.limits),
            ReconciliationTrigger::Startup => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Startup, self.limits),
            ReconciliationTrigger::Periodic => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Periodic, self.limits),
            ReconciliationTrigger::Explicit => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Explicit, self.limits),
            ReconciliationTrigger::ProjectionPreconditionMismatch(paths) => {
                if self.pending.retain_precondition_paths(paths, self.limits) {
                    self.pending.request_full_scan(
                        ReconciliationFullScanReason::ProjectionPreconditionPathOverflow,
                        self.limits,
                    );
                }
            }
            ReconciliationTrigger::BaselineUnavailable => self.pending.request_full_scan(
                ReconciliationFullScanReason::BaselineUnavailable,
                self.limits,
            ),
        }
    }

    /// Start the highest-priority retained work, if no lease is active.
    pub(crate) fn next(&mut self) -> Option<ReconciliationJob> {
        if self.active.is_some() {
            return None;
        }
        let work = self.pending.take_next_work()?;
        self.next_lease_sequence = self
            .next_lease_sequence
            .checked_add(1)
            .expect("reconciliation scheduler lease sequence exhausted");
        let lease = ReconciliationLease {
            scheduler_id: self.scheduler_id,
            sequence: self.next_lease_sequence,
        };
        self.active = Some(lease);
        Some(ReconciliationJob { lease, work })
    }

    /// Finish exactly the currently active lease. Stale, duplicate, and
    /// foreign leases fail without mutating pending work or status.
    pub(crate) fn complete(
        &mut self,
        lease: ReconciliationLease,
        outcome: ReconciliationCompletionOutcome,
    ) -> Result<(), ReconciliationCompletionError> {
        let Some(active) = self.active else {
            return Err(ReconciliationCompletionError::NoActiveJob);
        };
        if active != lease {
            return Err(ReconciliationCompletionError::StaleOrForeignLease);
        }
        self.active = None;
        self.last_completion = Some(outcome);
        match outcome {
            ReconciliationCompletionOutcome::Noop | ReconciliationCompletionOutcome::Complete => {
                self.blocked = None;
            }
            ReconciliationCompletionOutcome::Blocked => {
                self.blocked = Some(ReconciliationBlockedStatus { lease });
            }
            ReconciliationCompletionOutcome::Retry => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Retry, self.limits),
            ReconciliationCompletionOutcome::Uncertain => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Uncertain, self.limits),
            ReconciliationCompletionOutcome::Cancelled => self
                .pending
                .request_full_scan(ReconciliationFullScanReason::Cancelled, self.limits),
            // Shutdown hands control back to the lifecycle owner. It is
            // observable through `last_completion`, while a later scheduler
            // instance must receive its own Startup trigger.
            ReconciliationCompletionOutcome::Shutdown => {}
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> ReconciliationSchedulerStatus {
        ReconciliationSchedulerStatus {
            active: self.active.is_some(),
            pending: self.pending.has_pending_work(),
            blocked: self.blocked,
            last_completion: self.last_completion,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphTextScanLimits {
    pub(crate) eligible_files: usize,
    pub(crate) all_entries: usize,
    pub(crate) directories: usize,
    pub(crate) pending_directories: usize,
    pub(crate) directory_depth: usize,
    pub(crate) aggregate_path_bytes: u64,
    pub(crate) aggregate_hashed_bytes: u64,
    pub(crate) retained_rows: usize,
    pub(crate) retained_bytes: u64,
    pub(crate) expected_paths: usize,
    pub(crate) aggregate_expected_path_bytes: u64,
    pub(crate) expected_page_rows: usize,
    pub(crate) expected_page_bytes: u64,
    pub(crate) exact_path_bytes: usize,
    pub(crate) read_buffer_bytes: usize,
}

impl Default for GraphTextScanLimits {
    fn default() -> Self {
        Self {
            eligible_files: 1_000_000,
            all_entries: 2_000_000,
            directories: 1_000_000,
            pending_directories: 1_000_000,
            directory_depth: 256,
            aggregate_path_bytes: 512 * 1024 * 1024,
            aggregate_hashed_bytes: 512 * 1024 * 1024,
            retained_rows: 2_000_000,
            retained_bytes: MAX_SCAN_RETAINED_BYTES,
            expected_paths: MAX_SCAN_EXPECTED_PATHS,
            aggregate_expected_path_bytes: 512 * 1024 * 1024,
            expected_page_rows: GRAPH_TEXT_EXPECTED_PAGE_ROWS,
            expected_page_bytes: GRAPH_TEXT_EXPECTED_PAGE_BYTES,
            exact_path_bytes: MAX_SCAN_EXACT_PATH_BYTES,
            read_buffer_bytes: GRAPH_TEXT_SCAN_READ_BUFFER_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum GraphTextScanPathClass {
    EligibleManaged(ManagedTextKind),
    EligibleUnmanaged,
    ProviderConflictCopy,
    Configuration,
    RetainedNonText,
}

impl GraphTextScanPathClass {
    fn tag(self) -> u8 {
        match self {
            Self::EligibleManaged(ManagedTextKind::Page) => 0,
            Self::EligibleManaged(ManagedTextKind::Journal) => 1,
            Self::EligibleUnmanaged => 2,
            Self::ProviderConflictCopy => 3,
            Self::Configuration => 4,
            Self::RetainedNonText => 5,
        }
    }

    fn is_eligible(self) -> bool {
        matches!(self, Self::EligibleManaged(_) | Self::EligibleUnmanaged)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphTextScanFileFingerprint {
    pub(crate) exact_relative: String,
    pub(crate) class: GraphTextScanPathClass,
    pub(crate) portable_key: Option<PortablePathKey>,
    pub(crate) description: Option<BlobDescription>,
    pub(crate) file_resource_id: ContentDigest,
    pub(crate) link_count: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GraphTextScanPassInstrumentation {
    pub(crate) directory_entries: u64,
    pub(crate) directories: u64,
    pub(crate) regular_files: u64,
    pub(crate) eligible_files: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_hashed: u64,
    pub(crate) peak_retained_rows: u64,
    pub(crate) peak_retained_bytes: u64,
    pub(crate) peak_read_buffers: u64,
    pub(crate) peak_read_buffer_bytes: u64,
    pub(crate) retained_rows: u64,
    pub(crate) retained_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct GraphTextScanPass {
    pub(crate) graph_resource: CanonicalGraphResourceId,
    pub(crate) scope_binding: GraphTextScopeBinding,
    pub(crate) directories_by_exact_relative: BTreeMap<String, ContentDigest>,
    pub(crate) files: Vec<GraphTextScanFileFingerprint>,
    pub(crate) instrumentation: GraphTextScanPassInstrumentation,
}

impl GraphTextScanPass {
    fn evidence_eq(&self, other: &Self) -> bool {
        self.graph_resource == other.graph_resource
            && self.scope_binding == other.scope_binding
            && self.directories_by_exact_relative == other.directories_by_exact_relative
            && self.files == other.files
    }
}

pub(crate) fn graph_text_scan_pass_digest(pass: &GraphTextScanPass) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/stable-graph-text-pass/v1\0");
    hasher.update(pass.graph_resource.as_bytes());
    hasher.update(pass.scope_binding.canonical_bytes());
    hasher.update((pass.directories_by_exact_relative.len() as u64).to_be_bytes());
    for (path, resource) in &pass.directories_by_exact_relative {
        hash_len_bytes(&mut hasher, path.as_bytes());
        hasher.update(resource.as_bytes());
    }
    hasher.update((pass.files.len() as u64).to_be_bytes());
    for file in &pass.files {
        hash_len_bytes(&mut hasher, file.exact_relative.as_bytes());
        hasher.update([file.class.tag()]);
        match &file.portable_key {
            Some(key) => {
                hasher.update([1]);
                hash_len_bytes(&mut hasher, key.as_bytes());
            }
            None => hasher.update([0]),
        }
        match file.description {
            Some(description) => {
                hasher.update([1]);
                hash_description(&mut hasher, description);
            }
            None => hasher.update([0]),
        }
        hasher.update(file.file_resource_id.as_bytes());
        hasher.update(file.link_count.to_be_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

/// Binding supplied by the future authenticated engine cursor.
///
/// It intentionally contains only roots which must stay pinned across both
/// complete filesystem passes. It is not filesystem or import authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedPathBinding {
    pub(crate) accepted_frontier: ContentDigest,
    pub(crate) projection_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedExpectedPath {
    pub(crate) page_id: PageId,
    pub(crate) path: ManagedPath,
    pub(crate) kind: ManagedTextKind,
    pub(crate) description: BlobDescription,
    pub(crate) owner_binding: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedExpectedPathStreamHeader {
    pub(crate) binding: ExpectedPathBinding,
    pub(crate) total_rows: usize,
    /// Constant-size commitment to the authenticated source roots and all
    /// joined identities. The scan separately hashes the streamed rows and
    /// requires the same row commitment from both opens.
    pub(crate) source_commitment: ContentDigest,
    /// Live scan-owned cursor state. A joined engine/projection adapter reports
    /// only its cursor token and bounded page state here, never its engine's
    /// independently retained authenticated indexes.
    pub(crate) cursor_retained_rows: usize,
    pub(crate) cursor_retained_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpectedPathStreamLimits {
    pub(crate) maximum_rows: usize,
    pub(crate) maximum_path_bytes: usize,
    pub(crate) maximum_aggregate_path_bytes: u64,
    pub(crate) maximum_page_rows: usize,
    pub(crate) maximum_page_bytes: u64,
    pub(crate) maximum_retained_rows: usize,
    pub(crate) maximum_retained_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpectedPathPageRequest {
    pub(crate) maximum_rows: usize,
    pub(crate) maximum_path_bytes: usize,
    pub(crate) maximum_aggregate_path_bytes: u64,
    pub(crate) maximum_retained_rows: usize,
    pub(crate) maximum_retained_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpectedPathPointRequest {
    pub(crate) maximum_path_bytes: usize,
    pub(crate) maximum_retained_rows: usize,
    pub(crate) maximum_retained_bytes: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedExpectedPathPage {
    pub(crate) rows: Vec<AuthenticatedExpectedPath>,
    pub(crate) done: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpectedPathSourceFailure {
    Missing,
    Corrupt,
    Ambiguous,
    Unavailable,
    BoundExceeded,
}

impl fmt::Display for ExpectedPathSourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "authenticated expected-path authority is missing",
            Self::Corrupt => "authenticated expected-path authority is corrupt",
            Self::Ambiguous => "authenticated expected-path authority is ambiguous",
            Self::Unavailable => "authenticated expected-path authority is unavailable",
            Self::BoundExceeded => "authenticated expected-path page bound exceeded",
        })
    }
}

/// Joined authenticated engine/projection cursor boundary.
///
/// Each open returns a small cursor over one PageId-sorted live set. Pages
/// must be allocated causally within the supplied row/path/retained limits.
/// Implementations must fail with `Ambiguous` before opening when exact or
/// portable path ownership is not unique. They must not rematerialize the
/// complete catalog in a cursor, page, or exact-path point lookup. The scan
/// independently checks PageId ordering, row/path bounds, total count, both
/// streamed-row commitments, the authenticated source-root commitment, and the
/// binding before/after both filesystem passes and both stream traversals.
pub(crate) trait AuthenticatedExpectedPathSource {
    type Cursor;

    fn open_expected_paths(
        &self,
        limits: ExpectedPathStreamLimits,
    ) -> Result<(AuthenticatedExpectedPathStreamHeader, Self::Cursor), ExpectedPathSourceFailure>;

    fn read_expected_path_page(
        &self,
        cursor: &mut Self::Cursor,
        request: ExpectedPathPageRequest,
    ) -> Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure>;

    fn expected_path_at(
        &self,
        path: &ManagedPath,
        request: ExpectedPathPointRequest,
    ) -> Result<Option<AuthenticatedExpectedPath>, ExpectedPathSourceFailure>;

    fn current_binding(
        &self,
        maximum_retained_bytes: u64,
    ) -> Result<ExpectedPathBinding, ExpectedPathSourceFailure>;

    fn current_scan_identity(
        &self,
        maximum_retained_bytes: u64,
    ) -> Result<(ExpectedPathBinding, ContentDigest), ExpectedPathSourceFailure>;
}

/// Joined semantic/projection expected-state source.
pub(crate) struct JoinedAuthenticatedExpectedPathSource<'a> {
    engine: &'a ShardedHotEngine,
    projection: &'a ProjectionWorkIndex,
}

impl<'a> JoinedAuthenticatedExpectedPathSource<'a> {
    pub(crate) const fn new(
        engine: &'a ShardedHotEngine,
        projection: &'a ProjectionWorkIndex,
    ) -> Self {
        Self { engine, projection }
    }

    pub(crate) fn current_scan_identity(
        &self,
        maximum_retained_bytes: u64,
    ) -> Result<(ExpectedPathBinding, ContentDigest), ExpectedPathSourceFailure> {
        let mut budget = ProjectionExpectedPathReadBudget::new(maximum_retained_bytes);
        self.pin_join(&mut budget)
            .map(|(_, _, binding, source_commitment)| (binding, source_commitment))
    }

    fn pin_join(
        &self,
        budget: &mut ProjectionExpectedPathReadBudget,
    ) -> Result<
        (
            CurrentPathCatalogBinding,
            ProjectionExpectedPathHead,
            ExpectedPathBinding,
            ContentDigest,
        ),
        ExpectedPathSourceFailure,
    > {
        let engine = self
            .engine
            .current_path_catalog_binding()
            .map_err(map_engine_expected_failure)?;
        let projection = self
            .projection
            .pin_expected_path_head(budget)
            .map_err(map_projection_expected_failure)?;
        let endpoint = self
            .engine
            .projection_endpoint_binding()
            .ok_or(ExpectedPathSourceFailure::Missing)?;
        let receipt_store_id = self
            .engine
            .projection_receipt_store_id()
            .ok_or(ExpectedPathSourceFailure::Missing)?;
        if engine.workspace_id() != projection.workspace_id()
            || endpoint.endpoint_id() != projection.endpoint_id()
            || endpoint.graph_resource_id() != projection.graph_resource_id()
            || receipt_store_id != projection.receipt_store_id()
            || engine.history_generation() != projection.engine_history_generation()
            || engine.history_root() != projection.engine_history_root()
        {
            return Err(ExpectedPathSourceFailure::Corrupt);
        }
        let binding = ExpectedPathBinding {
            accepted_frontier: engine.accepted_frontier(),
            projection_generation: projection.generation(),
        };
        let source_commitment = joined_source_commitment(engine, projection, endpoint.device_id());
        Ok((engine, projection, binding, source_commitment))
    }

    fn require_join_current(
        &self,
        engine: CurrentPathCatalogBinding,
        projection: ProjectionExpectedPathHead,
        budget: &mut ProjectionExpectedPathReadBudget,
    ) -> Result<(), ExpectedPathSourceFailure> {
        let (current_engine, current_projection, _, _) = self.pin_join(budget)?;
        if current_engine != engine || current_projection != projection {
            return Err(ExpectedPathSourceFailure::Unavailable);
        }
        self.projection
            .require_expected_path_head_current(projection, budget)
            .map_err(map_projection_expected_failure)
    }

    fn join_row(
        &self,
        head: ProjectionExpectedPathHead,
        source_commitment: ContentDigest,
        row: CurrentPathCatalogRow,
        budget: &mut ProjectionExpectedPathReadBudget,
    ) -> Result<AuthenticatedExpectedPath, ExpectedPathSourceFailure> {
        let receipt = self
            .projection
            .completed_receipt_at_expected_path_head_bounded(head, row.path(), budget)
            .map_err(map_projection_expected_failure)?
            .ok_or(ExpectedPathSourceFailure::Missing)?;
        if receipt.page_id() != row.page_id() || receipt.path() != row.path() {
            return Err(ExpectedPathSourceFailure::Corrupt);
        }
        let ProjectionWorkTarget::Present(description) = receipt.target() else {
            return Err(ExpectedPathSourceFailure::Missing);
        };
        Ok(AuthenticatedExpectedPath {
            page_id: row.page_id(),
            path: row.path().clone(),
            kind: row.kind(),
            description,
            owner_binding: joined_owner_binding(source_commitment, row.page_id(), row.path()),
        })
    }

    fn read_expected_path_page_bounded(
        &self,
        cursor: &mut JoinedAuthenticatedExpectedPathCursor<'a>,
        request: ExpectedPathPageRequest,
        budget: &mut ProjectionExpectedPathReadBudget,
    ) -> Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure> {
        if cursor.binding.accepted_frontier != cursor.engine_binding.accepted_frontier()
            || cursor.binding.projection_generation != cursor.projection_head.generation()
        {
            return Err(ExpectedPathSourceFailure::Corrupt);
        }
        if request.maximum_rows > cursor.limits.maximum_page_rows
            || request.maximum_path_bytes > cursor.limits.maximum_path_bytes
            || request.maximum_aggregate_path_bytes > cursor.limits.maximum_aggregate_path_bytes
            || request.maximum_retained_rows > cursor.limits.maximum_retained_rows
            || request.maximum_retained_bytes > cursor.limits.maximum_retained_bytes
            || request.maximum_retained_bytes > cursor.limits.maximum_page_bytes
        {
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        self.require_join_current(cursor.engine_binding, cursor.projection_head, budget)?;
        let token = cursor
            .token
            .take()
            .ok_or(ExpectedPathSourceFailure::Corrupt)?;
        let worst_row_bytes = mem::size_of::<CurrentPathCatalogRow>()
            .saturating_add(mem::size_of::<AuthenticatedExpectedPath>())
            .saturating_add(request.maximum_path_bytes.saturating_mul(2));
        let page_budget = request.maximum_retained_bytes / 2;
        let rows_by_bytes = usize::try_from(
            page_budget
                / u64::try_from(worst_row_bytes.max(1))
                    .map_err(|_| ExpectedPathSourceFailure::BoundExceeded)?,
        )
        .map_err(|_| ExpectedPathSourceFailure::BoundExceeded)?;
        let limit = request
            .maximum_rows
            .min(request.maximum_retained_rows)
            .min(rows_by_bytes)
            .min(MAX_CURRENT_PATH_CURSOR_PAGE_ROWS);
        if limit == 0 {
            let _ = self.engine.cancel_current_path_cursor(token);
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        let page = self
            .engine
            .current_path_cursor_page(token, limit)
            .map_err(map_engine_expected_failure)?;
        let (semantic_rows, next) = page.into_parts();
        cursor.token = next;
        let semantic_retained_bytes = (semantic_rows.capacity() as u64)
            .saturating_mul(mem::size_of::<CurrentPathCatalogRow>() as u64)
            .saturating_add(
                semantic_rows
                    .iter()
                    .map(|row| row.path().as_str().len() as u64)
                    .sum::<u64>(),
            );
        if semantic_retained_bytes > page_budget {
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        budget
            .reset_with_retained(semantic_retained_bytes)
            .map_err(map_projection_expected_failure)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(semantic_rows.len())
            .map_err(|_| ExpectedPathSourceFailure::BoundExceeded)?;
        let output_capacity_bytes = (rows.capacity() as u64)
            .checked_mul(mem::size_of::<AuthenticatedExpectedPath>() as u64)
            .ok_or(ExpectedPathSourceFailure::BoundExceeded)?;
        let mut aggregate_path_bytes = 0_u64;
        for semantic in semantic_rows {
            let path_bytes = semantic.path().as_str().len();
            if path_bytes > request.maximum_path_bytes {
                return Err(ExpectedPathSourceFailure::BoundExceeded);
            }
            aggregate_path_bytes = aggregate_path_bytes
                .checked_add(path_bytes as u64)
                .ok_or(ExpectedPathSourceFailure::BoundExceeded)?;
            if aggregate_path_bytes > request.maximum_aggregate_path_bytes {
                return Err(ExpectedPathSourceFailure::BoundExceeded);
            }
            budget
                .reset_with_retained(
                    semantic_retained_bytes
                        .checked_add(output_capacity_bytes)
                        .and_then(|retained| retained.checked_add(aggregate_path_bytes))
                        .ok_or(ExpectedPathSourceFailure::BoundExceeded)?,
                )
                .map_err(map_projection_expected_failure)?;
            rows.push(self.join_row(
                cursor.projection_head,
                cursor.source_commitment,
                semantic,
                budget,
            )?);
        }
        self.require_join_current(cursor.engine_binding, cursor.projection_head, budget)?;
        Ok(AuthenticatedExpectedPathPage {
            rows,
            done: cursor.token.is_none(),
        })
    }

    fn expected_path_at_bounded(
        &self,
        path: &ManagedPath,
        request: ExpectedPathPointRequest,
        budget: &mut ProjectionExpectedPathReadBudget,
    ) -> Result<Option<AuthenticatedExpectedPath>, ExpectedPathSourceFailure> {
        let point_bytes = mem::size_of::<CurrentPathCatalogRow>()
            .saturating_add(mem::size_of::<AuthenticatedExpectedPath>())
            .saturating_add(path.as_str().len().saturating_mul(2));
        if request.maximum_retained_rows < 2
            || path.as_str().len() > request.maximum_path_bytes
            || point_bytes as u64 > request.maximum_retained_bytes
        {
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        let (engine, projection, _, source_commitment) = self.pin_join(budget)?;
        let row = self
            .engine
            .current_path_catalog_row_at_path(path)
            .map_err(map_engine_expected_failure)?;
        budget
            .reset_with_retained(if row.is_some() { point_bytes as u64 } else { 0 })
            .map_err(map_projection_expected_failure)?;
        let joined = row
            .map(|row| self.join_row(projection, source_commitment, row, budget))
            .transpose()?;
        self.require_join_current(engine, projection, budget)?;
        Ok(joined)
    }

    #[cfg(test)]
    pub(crate) fn read_expected_path_page_budget_trace_for_test(
        &self,
        cursor: &mut JoinedAuthenticatedExpectedPathCursor<'a>,
        request: ExpectedPathPageRequest,
    ) -> (
        Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure>,
        Vec<super::projection_work_index::ProjectionExpectedPathBudgetRead>,
    ) {
        let mut budget = ProjectionExpectedPathReadBudget::new(request.maximum_retained_bytes);
        let result = self.read_expected_path_page_bounded(cursor, request, &mut budget);
        (result, budget.read_trace())
    }
}

pub(crate) struct JoinedAuthenticatedExpectedPathCursor<'a> {
    engine: &'a ShardedHotEngine,
    token: Option<CurrentPathCatalogCursor>,
    engine_binding: CurrentPathCatalogBinding,
    projection_head: ProjectionExpectedPathHead,
    binding: ExpectedPathBinding,
    source_commitment: ContentDigest,
    limits: ExpectedPathStreamLimits,
}

#[cfg(test)]
pub(crate) struct DetachedJoinedExpectedPathCursor {
    token: CurrentPathCatalogCursor,
    engine_binding: CurrentPathCatalogBinding,
    projection_head: ProjectionExpectedPathHead,
    binding: ExpectedPathBinding,
    source_commitment: ContentDigest,
    limits: ExpectedPathStreamLimits,
}

#[cfg(test)]
impl<'a> JoinedAuthenticatedExpectedPathSource<'a> {
    pub(crate) fn open_detached_for_test(
        &self,
        limits: ExpectedPathStreamLimits,
    ) -> Result<
        (
            AuthenticatedExpectedPathStreamHeader,
            DetachedJoinedExpectedPathCursor,
        ),
        ExpectedPathSourceFailure,
    > {
        let mut budget = ProjectionExpectedPathReadBudget::new(limits.maximum_retained_bytes);
        let (engine_binding, projection_head, binding, source_commitment) =
            self.pin_join(&mut budget)?;
        let total_rows = usize::try_from(engine_binding.catalog_rows())
            .map_err(|_| ExpectedPathSourceFailure::BoundExceeded)?;
        if total_rows > limits.maximum_rows {
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        budget
            .reset_with_retained(mem::size_of::<DetachedJoinedExpectedPathCursor>() as u64)
            .map_err(map_projection_expected_failure)?;
        let token = self
            .engine
            .begin_current_path_cursor()
            .map_err(map_engine_expected_failure)?;
        Ok((
            AuthenticatedExpectedPathStreamHeader {
                binding,
                total_rows,
                source_commitment,
                cursor_retained_rows: 1,
                cursor_retained_bytes: mem::size_of::<JoinedAuthenticatedExpectedPathCursor<'_>>()
                    as u64,
            },
            DetachedJoinedExpectedPathCursor {
                token,
                engine_binding,
                projection_head,
                binding,
                source_commitment,
                limits,
            },
        ))
    }

    pub(crate) fn attach_detached_for_test(
        &self,
        detached: DetachedJoinedExpectedPathCursor,
    ) -> JoinedAuthenticatedExpectedPathCursor<'a> {
        JoinedAuthenticatedExpectedPathCursor {
            engine: self.engine,
            token: Some(detached.token),
            engine_binding: detached.engine_binding,
            projection_head: detached.projection_head,
            binding: detached.binding,
            source_commitment: detached.source_commitment,
            limits: detached.limits,
        }
    }
}

impl Drop for JoinedAuthenticatedExpectedPathCursor<'_> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = self.engine.cancel_current_path_cursor(token);
        }
    }
}

impl<'a> AuthenticatedExpectedPathSource for JoinedAuthenticatedExpectedPathSource<'a> {
    type Cursor = JoinedAuthenticatedExpectedPathCursor<'a>;

    fn open_expected_paths(
        &self,
        limits: ExpectedPathStreamLimits,
    ) -> Result<(AuthenticatedExpectedPathStreamHeader, Self::Cursor), ExpectedPathSourceFailure>
    {
        let mut budget = ProjectionExpectedPathReadBudget::new(limits.maximum_retained_bytes);
        let (engine_binding, projection_head, binding, source_commitment) =
            self.pin_join(&mut budget)?;
        let total_rows = usize::try_from(engine_binding.catalog_rows())
            .map_err(|_| ExpectedPathSourceFailure::BoundExceeded)?;
        if total_rows > limits.maximum_rows
            || limits.maximum_page_rows == 0
            || limits.maximum_path_bytes == 0
            || limits.maximum_retained_rows == 0
            || limits.maximum_retained_bytes
                < mem::size_of::<JoinedAuthenticatedExpectedPathCursor<'_>>() as u64
        {
            return Err(ExpectedPathSourceFailure::BoundExceeded);
        }
        let cursor_retained_bytes =
            mem::size_of::<JoinedAuthenticatedExpectedPathCursor<'_>>() as u64;
        budget
            .reset_with_retained(cursor_retained_bytes)
            .map_err(map_projection_expected_failure)?;
        let token = self
            .engine
            .begin_current_path_cursor()
            .map_err(map_engine_expected_failure)?;
        Ok((
            AuthenticatedExpectedPathStreamHeader {
                binding,
                total_rows,
                source_commitment,
                cursor_retained_rows: 1,
                cursor_retained_bytes,
            },
            JoinedAuthenticatedExpectedPathCursor {
                engine: self.engine,
                token: Some(token),
                engine_binding,
                projection_head,
                binding,
                source_commitment,
                limits,
            },
        ))
    }

    fn read_expected_path_page(
        &self,
        cursor: &mut Self::Cursor,
        request: ExpectedPathPageRequest,
    ) -> Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure> {
        let mut budget = ProjectionExpectedPathReadBudget::new(request.maximum_retained_bytes);
        self.read_expected_path_page_bounded(cursor, request, &mut budget)
    }

    fn expected_path_at(
        &self,
        path: &ManagedPath,
        request: ExpectedPathPointRequest,
    ) -> Result<Option<AuthenticatedExpectedPath>, ExpectedPathSourceFailure> {
        let mut budget = ProjectionExpectedPathReadBudget::new(request.maximum_retained_bytes);
        self.expected_path_at_bounded(path, request, &mut budget)
    }

    fn current_binding(
        &self,
        maximum_retained_bytes: u64,
    ) -> Result<ExpectedPathBinding, ExpectedPathSourceFailure> {
        let mut budget = ProjectionExpectedPathReadBudget::new(maximum_retained_bytes);
        self.pin_join(&mut budget).map(|(_, _, binding, _)| binding)
    }

    fn current_scan_identity(
        &self,
        maximum_retained_bytes: u64,
    ) -> Result<(ExpectedPathBinding, ContentDigest), ExpectedPathSourceFailure> {
        JoinedAuthenticatedExpectedPathSource::current_scan_identity(self, maximum_retained_bytes)
    }
}

fn joined_source_commitment(
    engine: CurrentPathCatalogBinding,
    projection: ProjectionExpectedPathHead,
    device_id: super::DeviceId,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/joined-expected-path-source/v1\0");
    hasher.update(engine.workspace_id().as_uuid().as_bytes());
    hasher.update(engine.lineage_digest().as_bytes());
    hasher.update(engine.accepted_frontier().as_bytes());
    hasher.update(engine.history_generation().to_be_bytes());
    hasher.update(engine.history_root().as_bytes());
    hasher.update(engine.catalog_root().as_bytes());
    hasher.update(engine.catalog_rows().to_be_bytes());
    hasher.update(projection.head_digest().as_bytes());
    hasher.update(projection.endpoint_id().as_uuid().as_bytes());
    hasher.update(device_id.as_uuid().as_bytes());
    hasher.update(projection.graph_resource_id().as_bytes());
    hasher.update(projection.receipt_store_id().as_bytes());
    hasher.update(projection.generation().to_be_bytes());
    hasher.update(projection.engine_history_generation().to_be_bytes());
    hasher.update(projection.engine_history_root().as_bytes());
    hasher.update(projection.completed_paths_root().as_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn joined_owner_binding(
    source_commitment: ContentDigest,
    page_id: PageId,
    path: &ManagedPath,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/joined-expected-path-owner/v1\0");
    hasher.update(source_commitment.as_bytes());
    hasher.update(page_id.as_uuid().as_bytes());
    hash_len_bytes(&mut hasher, path.as_str().as_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn map_engine_expected_failure(error: super::EngineError) -> ExpectedPathSourceFailure {
    let detail = error.to_string();
    if detail.contains("ambiguity") || detail.contains("collision") {
        ExpectedPathSourceFailure::Ambiguous
    } else if detail.contains("unavailable") {
        ExpectedPathSourceFailure::Unavailable
    } else if detail.contains("missing") {
        ExpectedPathSourceFailure::Missing
    } else if detail.contains("bound") || detail.contains("limit") {
        ExpectedPathSourceFailure::BoundExceeded
    } else {
        ExpectedPathSourceFailure::Corrupt
    }
}

fn map_projection_expected_failure(error: ProjectionWorkError) -> ExpectedPathSourceFailure {
    if error.to_string().contains("too large") {
        return ExpectedPathSourceFailure::BoundExceeded;
    }
    match error {
        ProjectionWorkError::MissingHead
        | ProjectionWorkError::MissingRoot(_)
        | ProjectionWorkError::MissingNode(_) => ExpectedPathSourceFailure::Missing,
        ProjectionWorkError::AmbiguousCompletedPath => ExpectedPathSourceFailure::Ambiguous,
        ProjectionWorkError::ConcurrentRootTransition => ExpectedPathSourceFailure::Unavailable,
        ProjectionWorkError::Store(super::object_store::StoreError::StoredFileTooLarge {
            ..
        })
        | ProjectionWorkError::TooLarge(_)
        | ProjectionWorkError::PreflightLimitExceeded
        | ProjectionWorkError::RetainedMemoryLimitExceeded => {
            ExpectedPathSourceFailure::BoundExceeded
        }
        _ => ExpectedPathSourceFailure::Corrupt,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum GraphTextCandidateKind {
    Absence,
    Edit,
    Creation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphTextCandidateBinding {
    pub(crate) graph_resource: CanonicalGraphResourceId,
    pub(crate) scope_binding: GraphTextScopeBinding,
    pub(crate) expected_binding: ExpectedPathBinding,
    pub(crate) expected_source_commitment: ContentDigest,
    pub(crate) expected_rows_commitment: ContentDigest,
    pub(crate) scan_epoch_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphTextScanCandidate {
    pub(crate) path: ManagedPath,
    /// Expected rows retain their authenticated semantic kind. Disk-only
    /// creations deliberately defer kind to point recapture and parsing.
    pub(crate) managed_kind: Option<ManagedTextKind>,
    pub(crate) change: GraphTextCandidateKind,
    pub(crate) expected_description: Option<BlobDescription>,
    pub(crate) expected_owner_binding: Option<ContentDigest>,
    pub(crate) observed_description: Option<BlobDescription>,
    pub(crate) observed_file_resource_id: Option<ContentDigest>,
    pub(crate) observed_link_count: Option<u64>,
    pub(crate) binding: GraphTextCandidateBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GraphTextScanDiagnosticKind {
    ProviderConflictCopy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphTextScanDiagnostic {
    pub(crate) path: String,
    pub(crate) kind: GraphTextScanDiagnosticKind,
    pub(crate) file_resource_id: ContentDigest,
    pub(crate) link_count: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GraphTextScanInstrumentation {
    pub(crate) passes: u64,
    pub(crate) directory_entries: u64,
    pub(crate) directories: u64,
    pub(crate) regular_files: u64,
    pub(crate) eligible_files: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_hashed: u64,
    pub(crate) peak_retained_rows: u64,
    pub(crate) peak_retained_bytes: u64,
    pub(crate) peak_read_buffers: u64,
    pub(crate) peak_read_buffer_bytes: u64,
    pub(crate) expected_rows: u64,
    pub(crate) expected_pages: u64,
    pub(crate) expected_path_bytes: u64,
    pub(crate) candidates: u64,
    pub(crate) diagnostics: u64,
    pub(crate) parser_invocations: u64,
}

impl GraphTextScanInstrumentation {
    fn add_pass(
        &mut self,
        pass: GraphTextScanPassInstrumentation,
        live_rows: u64,
        live_bytes: u64,
    ) {
        self.passes = self.passes.saturating_add(1);
        self.directory_entries = self
            .directory_entries
            .saturating_add(pass.directory_entries);
        self.directories = self.directories.saturating_add(pass.directories);
        self.regular_files = self.regular_files.saturating_add(pass.regular_files);
        self.eligible_files = self.eligible_files.saturating_add(pass.eligible_files);
        self.bytes_read = self.bytes_read.saturating_add(pass.bytes_read);
        self.bytes_hashed = self.bytes_hashed.saturating_add(pass.bytes_hashed);
        self.peak_retained_rows = self
            .peak_retained_rows
            .max(live_rows.saturating_add(pass.peak_retained_rows));
        self.peak_retained_bytes = self
            .peak_retained_bytes
            .max(live_bytes.saturating_add(pass.peak_retained_bytes));
        self.peak_read_buffers = self.peak_read_buffers.max(pass.peak_read_buffers);
        self.peak_read_buffer_bytes = self.peak_read_buffer_bytes.max(pass.peak_read_buffer_bytes);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StableGraphTextScan {
    pub(crate) candidates: Vec<GraphTextScanCandidate>,
    pub(crate) diagnostics: Vec<GraphTextScanDiagnostic>,
    pub(crate) binding: GraphTextCandidateBinding,
    pub(crate) instrumentation: GraphTextScanInstrumentation,
    pub(crate) wall_time: Duration,
    pub(crate) baseline_pass: GraphTextScanPass,
    pub(crate) pass_a_digest: ContentDigest,
    pub(crate) pass_b_digest: ContentDigest,
}

/// Borrowed, non-authoritative stable-scan evidence for the disposable
/// baseline adapter. The adapter iterates these collections in bounded pages;
/// this accessor never clones or rematerializes the graph.
pub(crate) struct StableGraphTextBaselineEvidence<'a> {
    pub(crate) directories: &'a BTreeMap<String, ContentDigest>,
    pub(crate) files: &'a [GraphTextScanFileFingerprint],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StableGraphTextBaselineIdentity {
    pub(crate) graph_resource: CanonicalGraphResourceId,
    pub(crate) scope_binding: GraphTextScopeBinding,
    pub(crate) expected_binding: ExpectedPathBinding,
    pub(crate) expected_source_commitment: ContentDigest,
    pub(crate) expected_rows_commitment: ContentDigest,
    pub(crate) scan_epoch_digest: ContentDigest,
    pub(crate) pass_a_digest: ContentDigest,
    pub(crate) pass_b_digest: ContentDigest,
    pub(crate) candidate_digest: ContentDigest,
    pub(crate) candidate_count: usize,
    pub(crate) diagnostic_digest: ContentDigest,
    pub(crate) diagnostic_count: usize,
    pub(crate) instrumentation: GraphTextScanInstrumentation,
    pub(crate) wall_time_millis: u64,
    commitment: ContentDigest,
}

impl StableGraphTextBaselineIdentity {
    pub(crate) fn is_sealed(&self) -> bool {
        self.commitment == stable_graph_text_baseline_identity_digest(self)
    }

    pub(crate) const fn commitment(&self) -> ContentDigest {
        self.commitment
    }
}

impl StableGraphTextScan {
    pub(crate) fn baseline_evidence(&self) -> StableGraphTextBaselineEvidence<'_> {
        StableGraphTextBaselineEvidence {
            directories: &self.baseline_pass.directories_by_exact_relative,
            files: &self.baseline_pass.files,
        }
    }

    pub(crate) const fn baseline_revalidation_retained_bytes() -> u64 {
        MAX_SCAN_RETAINED_BYTES
    }

    pub(crate) fn validated_baseline_identity(
        &self,
    ) -> Result<StableGraphTextBaselineIdentity, &'static str> {
        let recomputed_pass_digest = graph_text_scan_pass_digest(&self.baseline_pass);
        if self.pass_a_digest != self.pass_b_digest || self.pass_a_digest != recomputed_pass_digest
        {
            return Err("retained stable pass does not match both pass commitments");
        }
        if self.baseline_pass.graph_resource != self.binding.graph_resource
            || self.baseline_pass.scope_binding != self.binding.scope_binding
        {
            return Err("retained stable pass does not match the scan graph and scope binding");
        }
        let recomputed_epoch_digest = scan_epoch_digest_from_commitments(
            &self.baseline_pass,
            self.binding.expected_binding,
            self.binding.expected_source_commitment,
            self.binding.expected_rows_commitment,
        );
        if recomputed_epoch_digest != self.binding.scan_epoch_digest {
            return Err("stable scan epoch commitment does not match its retained evidence");
        }
        if self.instrumentation.passes != 2
            || self.instrumentation.candidates != self.candidates.len() as u64
            || self.instrumentation.diagnostics != self.diagnostics.len() as u64
        {
            return Err("stable scan instrumentation does not match its retained output");
        }
        let mut previous_candidate = None;
        for candidate in &self.candidates {
            if candidate.binding != self.binding {
                return Err("stable scan candidate carries a different scan binding");
            }
            if previous_candidate.is_some_and(|previous: &ManagedPath| previous >= &candidate.path)
            {
                return Err("stable scan candidates are not in strict exact-path order");
            }
            previous_candidate = Some(&candidate.path);
        }
        let mut diagnostics = self.diagnostics.iter();
        for file in self
            .baseline_pass
            .files
            .iter()
            .filter(|file| file.class == GraphTextScanPathClass::ProviderConflictCopy)
        {
            let Some(diagnostic) = diagnostics.next() else {
                return Err("retained provider-conflict evidence lacks its scan diagnostic");
            };
            if diagnostic.path != file.exact_relative
                || diagnostic.kind != GraphTextScanDiagnosticKind::ProviderConflictCopy
                || diagnostic.file_resource_id != file.file_resource_id
                || diagnostic.link_count != file.link_count
            {
                return Err("scan diagnostic does not match retained provider-conflict evidence");
            }
        }
        if diagnostics.next().is_some() {
            return Err("scan diagnostics contain evidence absent from the retained pass");
        }

        let mut identity = StableGraphTextBaselineIdentity {
            graph_resource: self.binding.graph_resource,
            scope_binding: self.binding.scope_binding,
            expected_binding: self.binding.expected_binding,
            expected_source_commitment: self.binding.expected_source_commitment,
            expected_rows_commitment: self.binding.expected_rows_commitment,
            scan_epoch_digest: self.binding.scan_epoch_digest,
            pass_a_digest: self.pass_a_digest,
            pass_b_digest: self.pass_b_digest,
            candidate_digest: stable_graph_text_candidate_digest(&self.candidates),
            candidate_count: self.candidates.len(),
            diagnostic_digest: stable_graph_text_diagnostic_digest(&self.diagnostics),
            diagnostic_count: self.diagnostics.len(),
            instrumentation: self.instrumentation,
            wall_time_millis: u64::try_from(self.wall_time.as_millis()).unwrap_or(u64::MAX),
            commitment: ContentDigest::of(b"unsealed stable scan identity"),
        };
        identity.commitment = stable_graph_text_baseline_identity_digest(&identity);
        Ok(identity)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GraphTextScanFailureClass {
    UnstableEpoch,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GraphTextScanFailureReason {
    FilesystemEvidenceChanged,
    ExpectedBindingChanged,
    ExpectedAuthorityMissing,
    ExpectedAuthorityCorrupt,
    ExpectedAuthorityAmbiguous,
    ExpectedAuthorityUnavailable,
    ConfigRefreshRequired,
    UnsafeFilesystem,
    BoundExceeded,
}

#[derive(Debug)]
pub(crate) struct GraphTextScanFailure {
    pub(crate) class: GraphTextScanFailureClass,
    pub(crate) reason: GraphTextScanFailureReason,
    pub(crate) detail: String,
    pub(crate) instrumentation: GraphTextScanInstrumentation,
    pub(crate) wall_time: Duration,
}

impl fmt::Display for GraphTextScanFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.reason, self.detail)
    }
}

impl std::error::Error for GraphTextScanFailure {}

pub(crate) fn scan_graph_text<S: AuthenticatedExpectedPathSource>(
    graph: &Graph,
    source: &S,
    limits: GraphTextScanLimits,
) -> Result<StableGraphTextScan, GraphTextScanFailure> {
    scan_graph_text_impl(graph, source, limits, || Ok(()))
}

#[cfg(test)]
pub(crate) fn scan_graph_text_with_hook<S, H>(
    graph: &Graph,
    source: &S,
    limits: GraphTextScanLimits,
    between_passes: H,
) -> Result<StableGraphTextScan, GraphTextScanFailure>
where
    S: AuthenticatedExpectedPathSource,
    H: FnOnce() -> io::Result<()>,
{
    scan_graph_text_impl(graph, source, limits, between_passes)
}

fn scan_graph_text_impl<S, H>(
    graph: &Graph,
    source: &S,
    limits: GraphTextScanLimits,
    between_passes: H,
) -> Result<StableGraphTextScan, GraphTextScanFailure>
where
    S: AuthenticatedExpectedPathSource,
    H: FnOnce() -> io::Result<()>,
{
    let started = Instant::now();
    let mut instrumentation = GraphTextScanInstrumentation::default();
    if limits.expected_page_rows == 0 || limits.expected_page_bytes == 0 {
        return Err(scan_io_failure(
            started,
            instrumentation,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "reconciliation scan expected page bound exceeded",
            ),
        ));
    }
    let expected_binding = source
        .current_binding(limits.retained_bytes)
        .map_err(|failure| {
            expected_source_failure(started, instrumentation, failure, failure.to_string())
        })?;

    let first = graph
        .capture_reconciliation_scan_pass(limits)
        .map_err(|error| scan_io_failure(started, instrumentation, error))?;
    instrumentation.add_pass(first.instrumentation, 0, 0);
    require_expected_binding(
        source,
        expected_binding,
        limits
            .retained_bytes
            .saturating_sub(first.instrumentation.peak_retained_bytes),
        started,
        instrumentation,
    )?;

    between_passes().map_err(|error| scan_io_failure(started, instrumentation, error))?;

    let mut second_limits = limits;
    second_limits.retained_rows = limits
        .retained_rows
        .checked_sub(first.instrumentation.peak_retained_rows as usize)
        .ok_or_else(|| {
            scan_io_failure(
                started,
                instrumentation,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "reconciliation scan simultaneous two-pass retained row bound exceeded",
                ),
            )
        })?;
    second_limits.retained_bytes = limits
        .retained_bytes
        .checked_sub(first.instrumentation.peak_retained_bytes)
        .ok_or_else(|| {
            scan_io_failure(
                started,
                instrumentation,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "reconciliation scan simultaneous two-pass retained byte bound exceeded",
                ),
            )
        })?;
    let second = graph
        .capture_reconciliation_scan_pass(second_limits)
        .map_err(|error| scan_io_failure(started, instrumentation, error))?;
    let pass_a_digest = graph_text_scan_pass_digest(&first);
    let pass_b_digest = graph_text_scan_pass_digest(&second);
    instrumentation.add_pass(
        second.instrumentation,
        first.instrumentation.peak_retained_rows,
        first.instrumentation.peak_retained_bytes,
    );
    require_expected_binding(
        source,
        expected_binding,
        limits
            .retained_bytes
            .saturating_sub(instrumentation.peak_retained_bytes),
        started,
        instrumentation,
    )?;

    if !first.evidence_eq(&second) {
        return Err(GraphTextScanFailure {
            class: GraphTextScanFailureClass::UnstableEpoch,
            reason: GraphTextScanFailureReason::FilesystemEvidenceChanged,
            detail: "the two complete graph-text fingerprint passes differed".to_owned(),
            instrumentation,
            wall_time: started.elapsed(),
        });
    }
    drop(first);

    let (expected_stream, plan) = plan_candidate_merge(
        source,
        &second,
        expected_binding,
        limits,
        started,
        &mut instrumentation,
    )?;
    let scan_epoch_digest = scan_epoch_digest(&second, expected_stream);
    let binding = GraphTextCandidateBinding {
        graph_resource: second.graph_resource,
        scope_binding: second.scope_binding.clone(),
        expected_binding,
        expected_source_commitment: expected_stream.header.source_commitment,
        expected_rows_commitment: expected_stream.rows_commitment,
        scan_epoch_digest,
    };
    let output_rows = plan
        .candidate_count
        .checked_add(plan.diagnostic_count)
        .ok_or_else(|| scan_bound_failure(started, instrumentation, "candidate row"))?;
    let output_bytes = plan
        .candidate_bytes()
        .and_then(|bytes| bytes.checked_add(plan.diagnostic_bytes()))
        .ok_or_else(|| scan_bound_failure(started, instrumentation, "candidate byte"))?;
    observe_live_memory(
        &mut instrumentation,
        second.instrumentation.peak_retained_rows,
        second.instrumentation.peak_retained_bytes,
        output_rows as u64,
        output_bytes,
        limits,
        started,
    )?;
    let (candidates, diagnostics) = derive_candidates(
        source,
        &second,
        expected_stream,
        &plan,
        &binding,
        limits,
        started,
        &mut instrumentation,
    )?;
    require_expected_binding(
        source,
        expected_binding,
        limits
            .retained_bytes
            .saturating_sub(instrumentation.peak_retained_bytes),
        started,
        instrumentation,
    )?;
    instrumentation.candidates = candidates.len() as u64;
    instrumentation.diagnostics = diagnostics.len() as u64;
    Ok(StableGraphTextScan {
        candidates,
        diagnostics,
        binding,
        instrumentation,
        wall_time: started.elapsed(),
        baseline_pass: second,
        pass_a_digest,
        pass_b_digest,
    })
}

fn require_expected_binding<S: AuthenticatedExpectedPathSource>(
    source: &S,
    expected: ExpectedPathBinding,
    maximum_retained_bytes: u64,
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
) -> Result<(), GraphTextScanFailure> {
    let current = source
        .current_binding(maximum_retained_bytes)
        .map_err(|failure| {
            expected_source_failure(started, instrumentation, failure, failure.to_string())
        })?;
    if current != expected {
        return Err(GraphTextScanFailure {
            class: GraphTextScanFailureClass::UnstableEpoch,
            reason: GraphTextScanFailureReason::ExpectedBindingChanged,
            detail: "accepted-frontier or projection-generation binding changed".to_owned(),
            instrumentation,
            wall_time: started.elapsed(),
        });
    }
    Ok(())
}

fn expected_source_failure(
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
    failure: ExpectedPathSourceFailure,
    detail: String,
) -> GraphTextScanFailure {
    let reason = match failure {
        ExpectedPathSourceFailure::Missing => GraphTextScanFailureReason::ExpectedAuthorityMissing,
        ExpectedPathSourceFailure::Corrupt => GraphTextScanFailureReason::ExpectedAuthorityCorrupt,
        ExpectedPathSourceFailure::Ambiguous => {
            GraphTextScanFailureReason::ExpectedAuthorityAmbiguous
        }
        ExpectedPathSourceFailure::Unavailable => {
            GraphTextScanFailureReason::ExpectedAuthorityUnavailable
        }
        ExpectedPathSourceFailure::BoundExceeded => GraphTextScanFailureReason::BoundExceeded,
    };
    GraphTextScanFailure {
        class: GraphTextScanFailureClass::Blocked,
        reason,
        detail,
        instrumentation,
        wall_time: started.elapsed(),
    }
}

fn scan_io_failure(
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
    error: io::Error,
) -> GraphTextScanFailure {
    let detail = error.to_string();
    let unstable = error.kind() == io::ErrorKind::Interrupted
        || (detail.contains("root")
            && (detail.contains("binding")
                || detail.contains("changed")
                || detail.contains("identity")
                || detail.contains("replaced")));
    let bounded = detail.contains("bound exceeded");
    let refresh_required = detail.contains("config") && detail.contains("refresh required");
    GraphTextScanFailure {
        class: if unstable {
            GraphTextScanFailureClass::UnstableEpoch
        } else {
            GraphTextScanFailureClass::Blocked
        },
        reason: if refresh_required {
            GraphTextScanFailureReason::ConfigRefreshRequired
        } else if unstable {
            GraphTextScanFailureReason::FilesystemEvidenceChanged
        } else if bounded {
            GraphTextScanFailureReason::BoundExceeded
        } else {
            GraphTextScanFailureReason::UnsafeFilesystem
        },
        detail,
        instrumentation,
        wall_time: started.elapsed(),
    }
}

fn expected_rows_commitment(
    binding: ExpectedPathBinding,
    rows: &[AuthenticatedExpectedPath],
) -> ContentDigest {
    let mut hasher = expected_rows_hasher(binding, rows.len());
    for row in rows {
        hash_expected_row(&mut hasher, row);
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn expected_rows_hasher(binding: ExpectedPathBinding, total_rows: usize) -> Sha256 {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/test-only/reconciliation-expected-paths/v1\0");
    hasher.update(binding.accepted_frontier.as_bytes());
    hasher.update(binding.projection_generation.to_be_bytes());
    hasher.update((total_rows as u64).to_be_bytes());
    hasher
}

fn hash_expected_row(hasher: &mut Sha256, row: &AuthenticatedExpectedPath) {
    hasher.update(row.page_id.as_uuid().as_bytes());
    hash_len_bytes(hasher, row.path.as_str().as_bytes());
    hasher.update(match row.kind {
        ManagedTextKind::Page => [0],
        ManagedTextKind::Journal => [1],
    });
    hash_description(hasher, row.description);
    hasher.update(row.owner_binding.as_bytes());
}

fn scan_epoch_digest(
    pass: &GraphTextScanPass,
    expected: WalkedExpectedPathStream,
) -> ContentDigest {
    scan_epoch_digest_from_commitments(
        pass,
        expected.header.binding,
        expected.header.source_commitment,
        expected.rows_commitment,
    )
}

pub(crate) fn scan_epoch_digest_from_commitments(
    pass: &GraphTextScanPass,
    expected_binding: ExpectedPathBinding,
    expected_source_commitment: ContentDigest,
    expected_rows_commitment: ContentDigest,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/test-only/reconciliation-scan-epoch/v1\0");
    hasher.update(pass.graph_resource.as_bytes());
    hasher.update(pass.scope_binding.canonical_bytes());
    hasher.update(expected_binding.accepted_frontier.as_bytes());
    hasher.update(expected_binding.projection_generation.to_be_bytes());
    hasher.update(expected_source_commitment.as_bytes());
    hasher.update(expected_rows_commitment.as_bytes());
    hasher.update((pass.directories_by_exact_relative.len() as u64).to_be_bytes());
    for (path, resource) in &pass.directories_by_exact_relative {
        hash_len_bytes(&mut hasher, path.as_bytes());
        hasher.update(resource.as_bytes());
    }
    hasher.update((pass.files.len() as u64).to_be_bytes());
    for file in &pass.files {
        hash_len_bytes(&mut hasher, file.exact_relative.as_bytes());
        hasher.update([file.class.tag()]);
        hasher.update(file.file_resource_id.as_bytes());
        hasher.update(file.link_count.to_be_bytes());
        if let Some(description) = file.description {
            hasher.update([1]);
            hash_description(&mut hasher, description);
        } else {
            hasher.update([0]);
        }
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn stable_graph_text_candidate_digest(candidates: &[GraphTextScanCandidate]) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/stable-graph-text-candidates/v1\0");
    hasher.update((candidates.len() as u64).to_be_bytes());
    for candidate in candidates {
        hash_len_bytes(&mut hasher, candidate.path.as_str().as_bytes());
        hasher.update([match candidate.managed_kind {
            None => 0,
            Some(ManagedTextKind::Page) => 1,
            Some(ManagedTextKind::Journal) => 2,
        }]);
        hasher.update([match candidate.change {
            GraphTextCandidateKind::Absence => 1,
            GraphTextCandidateKind::Edit => 2,
            GraphTextCandidateKind::Creation => 3,
        }]);
        hash_optional_description(&mut hasher, candidate.expected_description);
        hash_optional_digest(&mut hasher, candidate.expected_owner_binding);
        hash_optional_description(&mut hasher, candidate.observed_description);
        hash_optional_digest(&mut hasher, candidate.observed_file_resource_id);
        hash_optional_u64(&mut hasher, candidate.observed_link_count);
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn stable_graph_text_diagnostic_digest(diagnostics: &[GraphTextScanDiagnostic]) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/stable-graph-text-diagnostics/v1\0");
    hasher.update((diagnostics.len() as u64).to_be_bytes());
    for diagnostic in diagnostics {
        hash_len_bytes(&mut hasher, diagnostic.path.as_bytes());
        hasher.update([match diagnostic.kind {
            GraphTextScanDiagnosticKind::ProviderConflictCopy => 1,
        }]);
        hasher.update(diagnostic.file_resource_id.as_bytes());
        hasher.update(diagnostic.link_count.to_be_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn stable_graph_text_baseline_identity_digest(
    identity: &StableGraphTextBaselineIdentity,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/stable-graph-text-baseline-identity/v1\0");
    hasher.update(identity.graph_resource.as_bytes());
    hasher.update(identity.scope_binding.canonical_bytes());
    hasher.update(identity.expected_binding.accepted_frontier.as_bytes());
    hasher.update(
        identity
            .expected_binding
            .projection_generation
            .to_be_bytes(),
    );
    hasher.update(identity.expected_source_commitment.as_bytes());
    hasher.update(identity.expected_rows_commitment.as_bytes());
    hasher.update(identity.scan_epoch_digest.as_bytes());
    hasher.update(identity.pass_a_digest.as_bytes());
    hasher.update(identity.pass_b_digest.as_bytes());
    hasher.update(identity.candidate_digest.as_bytes());
    hasher.update((identity.candidate_count as u64).to_be_bytes());
    hasher.update(identity.diagnostic_digest.as_bytes());
    hasher.update((identity.diagnostic_count as u64).to_be_bytes());
    for metric in [
        identity.instrumentation.passes,
        identity.instrumentation.directory_entries,
        identity.instrumentation.directories,
        identity.instrumentation.regular_files,
        identity.instrumentation.eligible_files,
        identity.instrumentation.bytes_read,
        identity.instrumentation.bytes_hashed,
        identity.instrumentation.peak_retained_rows,
        identity.instrumentation.peak_retained_bytes,
        identity.instrumentation.peak_read_buffers,
        identity.instrumentation.peak_read_buffer_bytes,
        identity.instrumentation.expected_rows,
        identity.instrumentation.expected_pages,
        identity.instrumentation.expected_path_bytes,
        identity.instrumentation.candidates,
        identity.instrumentation.diagnostics,
        identity.instrumentation.parser_invocations,
        identity.wall_time_millis,
    ] {
        hasher.update(metric.to_be_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn hash_optional_description(hasher: &mut Sha256, value: Option<BlobDescription>) {
    if let Some(value) = value {
        hasher.update([1]);
        hash_description(hasher, value);
    } else {
        hasher.update([0]);
    }
}

fn hash_optional_digest(hasher: &mut Sha256, value: Option<ContentDigest>) {
    if let Some(value) = value {
        hasher.update([1]);
        hasher.update(value.as_bytes());
    } else {
        hasher.update([0]);
    }
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    if let Some(value) = value {
        hasher.update([1]);
        hasher.update(value.to_be_bytes());
    } else {
        hasher.update([0]);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct CandidateMergePlan {
    candidate_count: usize,
    candidate_path_bytes: u64,
    diagnostic_count: usize,
    diagnostic_path_bytes: u64,
}

impl CandidateMergePlan {
    fn add_candidate_path(&mut self, path: &str) -> io::Result<()> {
        self.candidate_count = self
            .candidate_count
            .checked_add(1)
            .ok_or_else(expected_allocation_overflow)?;
        self.candidate_path_bytes = self
            .candidate_path_bytes
            .checked_add(path.len() as u64)
            .ok_or_else(expected_allocation_overflow)?;
        Ok(())
    }

    fn candidate_bytes(self) -> Option<u64> {
        (self.candidate_count as u64)
            .checked_mul(mem::size_of::<GraphTextScanCandidate>() as u64)
            .and_then(|bytes| bytes.checked_add(self.candidate_path_bytes))
    }

    fn diagnostic_bytes(self) -> u64 {
        (self.diagnostic_count as u64)
            .saturating_mul(mem::size_of::<GraphTextScanDiagnostic>() as u64)
            .saturating_add(self.diagnostic_path_bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WalkedExpectedPathStream {
    header: AuthenticatedExpectedPathStreamHeader,
    rows_commitment: ContentDigest,
}

fn plan_candidate_merge<S: AuthenticatedExpectedPathSource>(
    source: &S,
    pass: &GraphTextScanPass,
    expected_binding: ExpectedPathBinding,
    limits: GraphTextScanLimits,
    started: Instant,
    instrumentation: &mut GraphTextScanInstrumentation,
) -> Result<(WalkedExpectedPathStream, CandidateMergePlan), GraphTextScanFailure> {
    let mut plan = CandidateMergePlan::default();
    for file in &pass.files {
        if file.class == GraphTextScanPathClass::ProviderConflictCopy {
            plan.diagnostic_count = plan
                .diagnostic_count
                .checked_add(1)
                .ok_or_else(|| scan_bound_failure(started, *instrumentation, "diagnostic row"))?;
            plan.diagnostic_path_bytes = plan
                .diagnostic_path_bytes
                .checked_add(file.exact_relative.len() as u64)
                .ok_or_else(|| scan_bound_failure(started, *instrumentation, "diagnostic byte"))?;
        }
    }
    let stream = walk_expected_stream(
        source,
        expected_binding,
        None,
        pass,
        limits,
        0,
        0,
        started,
        instrumentation,
        |row| match eligible_file_at_path(&pass.files, row.path.as_str()) {
            Some(file) if file.description == Some(row.description) => Ok(()),
            Some(_) | None => plan.add_candidate_path(row.path.as_str()),
        },
    )?;
    for file in pass.files.iter().filter(|file| file.class.is_eligible()) {
        let path = ManagedPath::parse(file.exact_relative.clone())
            .expect("eligible scan rows retain validated managed paths");
        let point_bytes = expected_point_retained_bytes(&path);
        observe_live_memory(
            instrumentation,
            pass.instrumentation.peak_retained_rows,
            pass.instrumentation.peak_retained_bytes,
            2,
            point_bytes,
            limits,
            started,
        )?;
        let expected = source
            .expected_path_at(
                &path,
                expected_point_request(
                    pass.instrumentation.peak_retained_rows,
                    pass.instrumentation.peak_retained_bytes,
                    limits,
                    started,
                    *instrumentation,
                )?,
            )
            .map_err(|failure| {
                expected_source_failure(started, *instrumentation, failure, failure.to_string())
            })?;
        if expected.is_none() {
            plan.add_candidate_path(&file.exact_relative)
                .map_err(|error| scan_io_failure(started, *instrumentation, error))?;
        }
    }
    Ok((stream, plan))
}

#[allow(clippy::too_many_arguments)]
fn derive_candidates<S: AuthenticatedExpectedPathSource>(
    source: &S,
    pass: &GraphTextScanPass,
    expected_stream: WalkedExpectedPathStream,
    plan: &CandidateMergePlan,
    binding: &GraphTextCandidateBinding,
    limits: GraphTextScanLimits,
    started: Instant,
    instrumentation: &mut GraphTextScanInstrumentation,
) -> Result<(Vec<GraphTextScanCandidate>, Vec<GraphTextScanDiagnostic>), GraphTextScanFailure> {
    let mut candidates = Vec::with_capacity(plan.candidate_count);
    let mut diagnostics = Vec::with_capacity(plan.diagnostic_count);
    for file in &pass.files {
        if file.class == GraphTextScanPathClass::ProviderConflictCopy {
            diagnostics.push(GraphTextScanDiagnostic {
                path: file.exact_relative.clone(),
                kind: GraphTextScanDiagnosticKind::ProviderConflictCopy,
                file_resource_id: file.file_resource_id,
                link_count: file.link_count,
            });
        }
    }
    let output_rows = plan.candidate_count + plan.diagnostic_count;
    let output_bytes = plan
        .candidate_bytes()
        .and_then(|bytes| bytes.checked_add(plan.diagnostic_bytes()))
        .ok_or_else(|| scan_bound_failure(started, *instrumentation, "candidate byte"))?;
    let walked = walk_expected_stream(
        source,
        expected_stream.header.binding,
        Some(expected_stream),
        pass,
        limits,
        output_rows,
        output_bytes,
        started,
        instrumentation,
        |row| {
            let observed = eligible_file_at_path(&pass.files, row.path.as_str());
            if observed.is_some_and(|file| file.description == Some(row.description)) {
                return Ok(());
            }
            candidates.push(expected_candidate(
                row,
                observed,
                if observed.is_some() {
                    GraphTextCandidateKind::Edit
                } else {
                    GraphTextCandidateKind::Absence
                },
                binding,
            ));
            Ok(())
        },
    )?;
    if walked != expected_stream {
        return Err(expected_source_failure(
            started,
            *instrumentation,
            ExpectedPathSourceFailure::Corrupt,
            "reopened expected stream changed its joined rows".to_owned(),
        ));
    }
    for file in pass.files.iter().filter(|file| file.class.is_eligible()) {
        let path = ManagedPath::parse(file.exact_relative.clone())
            .expect("eligible scan rows retain validated managed paths");
        let base_rows = pass
            .instrumentation
            .peak_retained_rows
            .saturating_add(output_rows as u64);
        let base_bytes = pass
            .instrumentation
            .peak_retained_bytes
            .saturating_add(output_bytes);
        let point_bytes = expected_point_retained_bytes(&path);
        observe_live_memory(
            instrumentation,
            base_rows,
            base_bytes,
            2,
            point_bytes,
            limits,
            started,
        )?;
        let expected = source
            .expected_path_at(
                &path,
                expected_point_request(base_rows, base_bytes, limits, started, *instrumentation)?,
            )
            .map_err(|failure| {
                expected_source_failure(started, *instrumentation, failure, failure.to_string())
            })?;
        if expected.is_none() {
            candidates.push(creation_candidate(file, binding));
        }
    }
    candidates.sort_unstable_by(|left, right| {
        (left.path.as_str(), left.change).cmp(&(right.path.as_str(), right.change))
    });
    debug_assert_eq!(candidates.len(), plan.candidate_count);
    debug_assert_eq!(diagnostics.len(), plan.diagnostic_count);
    Ok((candidates, diagnostics))
}

fn expected_point_retained_bytes(path: &ManagedPath) -> u64 {
    (mem::size_of::<CurrentPathCatalogRow>() as u64)
        .saturating_add(mem::size_of::<AuthenticatedExpectedPath>() as u64)
        .saturating_add((path.as_str().len() as u64).saturating_mul(2))
}

fn expected_point_request(
    base_rows: u64,
    base_bytes: u64,
    limits: GraphTextScanLimits,
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
) -> Result<ExpectedPathPointRequest, GraphTextScanFailure> {
    Ok(ExpectedPathPointRequest {
        maximum_path_bytes: limits.exact_path_bytes,
        maximum_retained_rows: limits
            .retained_rows
            .checked_sub(base_rows as usize)
            .ok_or_else(|| scan_bound_failure(started, instrumentation, "retained row"))?,
        maximum_retained_bytes: limits
            .retained_bytes
            .checked_sub(base_bytes)
            .ok_or_else(|| scan_bound_failure(started, instrumentation, "retained byte"))?,
    })
}

fn eligible_file_at_path<'a>(
    files: &'a [GraphTextScanFileFingerprint],
    path: &str,
) -> Option<&'a GraphTextScanFileFingerprint> {
    files
        .binary_search_by(|file| file.exact_relative.as_str().cmp(path))
        .ok()
        .and_then(|index| files.get(index))
        .filter(|file| file.class.is_eligible())
}

fn creation_candidate(
    file: &GraphTextScanFileFingerprint,
    binding: &GraphTextCandidateBinding,
) -> GraphTextScanCandidate {
    GraphTextScanCandidate {
        path: ManagedPath::parse(file.exact_relative.clone())
            .expect("eligible scan rows retain validated managed paths"),
        managed_kind: None,
        change: GraphTextCandidateKind::Creation,
        expected_description: None,
        expected_owner_binding: None,
        observed_description: file.description,
        observed_file_resource_id: Some(file.file_resource_id),
        observed_link_count: Some(file.link_count),
        binding: binding.clone(),
    }
}

fn expected_candidate(
    row: &AuthenticatedExpectedPath,
    observed: Option<&GraphTextScanFileFingerprint>,
    change: GraphTextCandidateKind,
    binding: &GraphTextCandidateBinding,
) -> GraphTextScanCandidate {
    GraphTextScanCandidate {
        path: row.path.clone(),
        managed_kind: Some(row.kind),
        change,
        expected_description: Some(row.description),
        expected_owner_binding: Some(row.owner_binding),
        observed_description: observed.and_then(|file| file.description),
        observed_file_resource_id: observed.map(|file| file.file_resource_id),
        observed_link_count: observed.map(|file| file.link_count),
        binding: binding.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_expected_stream<S, F>(
    source: &S,
    expected_binding: ExpectedPathBinding,
    required_stream: Option<WalkedExpectedPathStream>,
    pass: &GraphTextScanPass,
    limits: GraphTextScanLimits,
    output_rows: usize,
    output_bytes: u64,
    started: Instant,
    instrumentation: &mut GraphTextScanInstrumentation,
    mut visit: F,
) -> Result<WalkedExpectedPathStream, GraphTextScanFailure>
where
    S: AuthenticatedExpectedPathSource,
    F: FnMut(&AuthenticatedExpectedPath) -> io::Result<()>,
{
    let base_rows = pass
        .instrumentation
        .peak_retained_rows
        .checked_add(output_rows as u64)
        .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained row"))?;
    let base_bytes = pass
        .instrumentation
        .peak_retained_bytes
        .checked_add(output_bytes)
        .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained byte"))?;
    let stream_limits = ExpectedPathStreamLimits {
        maximum_rows: limits.expected_paths,
        maximum_path_bytes: limits.exact_path_bytes,
        maximum_aggregate_path_bytes: limits.aggregate_expected_path_bytes,
        maximum_page_rows: limits.expected_page_rows,
        maximum_page_bytes: limits.expected_page_bytes,
        maximum_retained_rows: limits
            .retained_rows
            .checked_sub(base_rows as usize)
            .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained row"))?,
        maximum_retained_bytes: limits
            .retained_bytes
            .checked_sub(base_bytes)
            .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained byte"))?,
    };
    let (header, mut cursor) = source
        .open_expected_paths(stream_limits)
        .map_err(|failure| {
            expected_source_failure(started, *instrumentation, failure, failure.to_string())
        })?;
    if header.binding != expected_binding {
        return Err(GraphTextScanFailure {
            class: GraphTextScanFailureClass::UnstableEpoch,
            reason: GraphTextScanFailureReason::ExpectedBindingChanged,
            detail: "expected stream opened at a different authority binding".to_owned(),
            instrumentation: *instrumentation,
            wall_time: started.elapsed(),
        });
    }
    if required_stream.is_some_and(|required| {
        required.header.binding != header.binding
            || required.header.total_rows != header.total_rows
            || required.header.source_commitment != header.source_commitment
    }) {
        return Err(expected_source_failure(
            started,
            *instrumentation,
            ExpectedPathSourceFailure::Corrupt,
            "reopened expected stream changed its authenticated header".to_owned(),
        ));
    }
    if header.total_rows > limits.expected_paths {
        return Err(scan_bound_failure(
            started,
            *instrumentation,
            "expected row",
        ));
    }
    if header.cursor_retained_rows > stream_limits.maximum_retained_rows
        || header.cursor_retained_bytes > stream_limits.maximum_retained_bytes
    {
        return Err(expected_source_failure(
            started,
            *instrumentation,
            ExpectedPathSourceFailure::Corrupt,
            "expected source cursor exceeded its causal retained-memory grant".to_owned(),
        ));
    }
    instrumentation.expected_rows = header.total_rows as u64;
    let cursor_rows = header.cursor_retained_rows as u64;
    let cursor_bytes = header.cursor_retained_bytes;
    observe_live_memory(
        instrumentation,
        base_rows,
        base_bytes,
        cursor_rows,
        cursor_bytes,
        limits,
        started,
    )?;

    let mut hasher = expected_rows_hasher(header.binding, header.total_rows);
    let mut seen_rows = 0_usize;
    let mut aggregate_path_bytes = 0_u64;
    let mut previous_page_id: Option<PageId> = None;
    loop {
        let validation_rows = u64::from(previous_page_id.is_some());
        let validation_bytes = validation_rows.saturating_mul(mem::size_of::<PageId>() as u64);
        let live_rows = base_rows
            .saturating_add(cursor_rows)
            .saturating_add(validation_rows);
        let live_bytes = base_bytes
            .saturating_add(cursor_bytes)
            .saturating_add(validation_bytes);
        let request = ExpectedPathPageRequest {
            maximum_rows: limits
                .expected_page_rows
                .min(limits.retained_rows.saturating_sub(live_rows as usize)),
            maximum_path_bytes: limits.exact_path_bytes,
            maximum_aggregate_path_bytes: limits
                .aggregate_expected_path_bytes
                .saturating_sub(aggregate_path_bytes),
            maximum_retained_rows: limits.retained_rows.saturating_sub(live_rows as usize),
            maximum_retained_bytes: limits
                .expected_page_bytes
                .min(limits.retained_bytes.saturating_sub(live_bytes)),
        };
        if request.maximum_rows == 0 || request.maximum_retained_bytes == 0 {
            return Err(scan_bound_failure(
                started,
                *instrumentation,
                "expected page retained memory",
            ));
        }
        let page = source
            .read_expected_path_page(&mut cursor, request)
            .map_err(|failure| {
                expected_source_failure(started, *instrumentation, failure, failure.to_string())
            })?;
        instrumentation.expected_pages = instrumentation.expected_pages.saturating_add(1);
        if page.rows.is_empty() && !page.done {
            return Err(expected_source_failure(
                started,
                *instrumentation,
                ExpectedPathSourceFailure::Corrupt,
                "expected source returned an empty non-terminal page".to_owned(),
            ));
        }
        let page_bytes = expected_page_retained_bytes(&page);
        if page.rows.len() > request.maximum_rows
            || page.rows.len() > request.maximum_retained_rows
            || page_bytes > request.maximum_retained_bytes
        {
            return Err(expected_source_failure(
                started,
                *instrumentation,
                ExpectedPathSourceFailure::Corrupt,
                "expected source page exceeded its causal retained-memory grant".to_owned(),
            ));
        }
        observe_live_memory(
            instrumentation,
            live_rows,
            live_bytes,
            page.rows.len() as u64,
            page_bytes,
            limits,
            started,
        )?;
        for row in &page.rows {
            let path = row.path.as_str();
            if path.len() > limits.exact_path_bytes {
                return Err(scan_bound_failure(
                    started,
                    *instrumentation,
                    "expected exact path byte",
                ));
            }
            aggregate_path_bytes = aggregate_path_bytes
                .checked_add(path.len() as u64)
                .ok_or_else(|| {
                    scan_bound_failure(started, *instrumentation, "expected aggregate path byte")
                })?;
            if aggregate_path_bytes > limits.aggregate_expected_path_bytes {
                return Err(scan_bound_failure(
                    started,
                    *instrumentation,
                    "expected aggregate path byte",
                ));
            }
            if let Some(previous) = previous_page_id {
                match previous.cmp(&row.page_id) {
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Equal => {
                        return Err(expected_source_failure(
                            started,
                            *instrumentation,
                            ExpectedPathSourceFailure::Ambiguous,
                            "expected source contains duplicate PageId owners".to_owned(),
                        ));
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(expected_source_failure(
                            started,
                            *instrumentation,
                            ExpectedPathSourceFailure::Corrupt,
                            "expected source is not strictly PageId sorted".to_owned(),
                        ));
                    }
                }
            }
            hash_expected_row(&mut hasher, row);
            visit(row).map_err(|error| scan_io_failure(started, *instrumentation, error))?;
            previous_page_id = Some(row.page_id);
            seen_rows = seen_rows
                .checked_add(1)
                .ok_or_else(|| scan_bound_failure(started, *instrumentation, "expected row"))?;
            if seen_rows > header.total_rows {
                return Err(expected_source_failure(
                    started,
                    *instrumentation,
                    ExpectedPathSourceFailure::Corrupt,
                    "expected source returned more rows than its authenticated header".to_owned(),
                ));
            }
        }
        if page.done {
            break;
        }
    }
    let rows_commitment = ContentDigest::from_bytes(hasher.finalize().into());
    if seen_rows != header.total_rows
        || required_stream.is_some_and(|required| required.rows_commitment != rows_commitment)
    {
        return Err(expected_source_failure(
            started,
            *instrumentation,
            ExpectedPathSourceFailure::Corrupt,
            "expected source row count or authenticated commitment mismatched".to_owned(),
        ));
    }
    instrumentation.expected_path_bytes = instrumentation
        .expected_path_bytes
        .saturating_add(aggregate_path_bytes);
    require_expected_binding(
        source,
        expected_binding,
        limits.retained_bytes.saturating_sub(base_bytes),
        started,
        *instrumentation,
    )?;
    Ok(WalkedExpectedPathStream {
        header,
        rows_commitment,
    })
}

fn expected_page_retained_bytes(page: &AuthenticatedExpectedPathPage) -> u64 {
    (page.rows.capacity() as u64)
        .saturating_mul(mem::size_of::<AuthenticatedExpectedPath>() as u64)
        .saturating_add(
            page.rows
                .iter()
                .map(|row| row.path.as_str().len() as u64)
                .sum::<u64>(),
        )
}

#[allow(clippy::too_many_arguments)]
fn observe_live_memory(
    instrumentation: &mut GraphTextScanInstrumentation,
    base_rows: u64,
    base_bytes: u64,
    additional_rows: u64,
    additional_bytes: u64,
    limits: GraphTextScanLimits,
    started: Instant,
) -> Result<(), GraphTextScanFailure> {
    let rows = base_rows
        .checked_add(additional_rows)
        .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained row"))?;
    let bytes = base_bytes
        .checked_add(additional_bytes)
        .ok_or_else(|| scan_bound_failure(started, *instrumentation, "retained byte"))?;
    if rows > limits.retained_rows as u64 || bytes > limits.retained_bytes {
        return Err(scan_bound_failure(
            started,
            *instrumentation,
            "retained memory",
        ));
    }
    instrumentation.peak_retained_rows = instrumentation.peak_retained_rows.max(rows);
    instrumentation.peak_retained_bytes = instrumentation.peak_retained_bytes.max(bytes);
    Ok(())
}

fn scan_bound_failure(
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
    resource: &'static str,
) -> GraphTextScanFailure {
    scan_io_failure(
        started,
        instrumentation,
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("reconciliation scan {resource} bound exceeded"),
        ),
    )
}

fn expected_allocation_overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "reconciliation scan expected allocation bound exceeded",
    )
}

fn hash_description(hasher: &mut Sha256, description: BlobDescription) {
    hasher.update(description.sha256());
    hasher.update(description.byte_length().to_be_bytes());
}

fn hash_len_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        graph_text_parser_invocations_for_scan_test, reset_graph_text_parser_counter_for_scan_test,
    };
    use pretty_assertions::assert_eq;
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    struct TempGraph {
        root: PathBuf,
    }

    impl TempGraph {
        fn new(config: Option<&str>) -> Self {
            let root = std::env::temp_dir().join(format!("tine-scan-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            if let Some(config) = config {
                fs::create_dir_all(root.join("logseq")).unwrap();
                fs::write(root.join("logseq/config.edn"), config).unwrap();
            }
            Self { root }
        }

        fn write(&self, relative: &str, bytes: &[u8]) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn graph(&self) -> Graph {
            Graph::open(&self.root)
        }
    }

    impl Drop for TempGraph {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[derive(Clone)]
    struct FixtureExpectedSource {
        rows: Vec<AuthenticatedExpectedPath>,
        failure: Option<ExpectedPathSourceFailure>,
        ambiguous: bool,
        rows_commitment: Cell<ContentDigest>,
        binding: Cell<ExpectedPathBinding>,
        open_calls: Cell<u64>,
        maximum_page_rows_seen: Cell<usize>,
        maximum_page_bytes_seen: Cell<u64>,
    }

    struct FixtureExpectedCursor {
        next: usize,
    }

    impl FixtureExpectedSource {
        fn empty() -> Self {
            Self::with_rows(Vec::new())
        }

        fn with_rows(mut rows: Vec<AuthenticatedExpectedPath>) -> Self {
            let binding = expected_binding(1);
            rows.sort_unstable_by_key(|row| row.page_id);
            let mut exact = std::collections::BTreeSet::new();
            let mut portable = std::collections::BTreeSet::new();
            let ambiguous = rows.iter().any(|row| {
                !exact.insert(row.path.clone()) || !portable.insert(row.path.portable_key())
            });
            Self {
                rows_commitment: Cell::new(expected_rows_commitment(binding, &rows)),
                rows,
                failure: None,
                ambiguous,
                binding: Cell::new(binding),
                open_calls: Cell::new(0),
                maximum_page_rows_seen: Cell::new(0),
                maximum_page_bytes_seen: Cell::new(0),
            }
        }

        fn failure(failure: ExpectedPathSourceFailure) -> Self {
            let binding = expected_binding(1);
            Self {
                rows: Vec::new(),
                failure: Some(failure),
                ambiguous: false,
                rows_commitment: Cell::new(expected_rows_commitment(binding, &[])),
                binding: Cell::new(binding),
                open_calls: Cell::new(0),
                maximum_page_rows_seen: Cell::new(0),
                maximum_page_bytes_seen: Cell::new(0),
            }
        }
    }

    impl AuthenticatedExpectedPathSource for FixtureExpectedSource {
        type Cursor = FixtureExpectedCursor;

        fn open_expected_paths(
            &self,
            limits: ExpectedPathStreamLimits,
        ) -> Result<(AuthenticatedExpectedPathStreamHeader, Self::Cursor), ExpectedPathSourceFailure>
        {
            if let Some(failure) = self.failure {
                return Err(failure);
            }
            if self.ambiguous {
                return Err(ExpectedPathSourceFailure::Ambiguous);
            }
            if self.rows_commitment.get()
                != expected_rows_commitment(self.binding.get(), &self.rows)
            {
                return Err(ExpectedPathSourceFailure::Corrupt);
            }
            self.open_calls.set(self.open_calls.get() + 1);
            if self.rows.len() > limits.maximum_rows
                || limits.maximum_page_rows == 0
                || limits.maximum_page_bytes == 0
                || limits.maximum_retained_rows == 0
                || limits.maximum_retained_bytes == 0
            {
                return Err(ExpectedPathSourceFailure::BoundExceeded);
            }
            let mut aggregate = 0_u64;
            for row in &self.rows {
                aggregate = aggregate
                    .checked_add(row.path.as_str().len() as u64)
                    .ok_or(ExpectedPathSourceFailure::Unavailable)?;
                if row.path.as_str().len() > limits.maximum_path_bytes
                    || aggregate > limits.maximum_aggregate_path_bytes
                {
                    return Err(ExpectedPathSourceFailure::BoundExceeded);
                }
            }
            Ok((
                AuthenticatedExpectedPathStreamHeader {
                    binding: self.binding.get(),
                    total_rows: self.rows.len(),
                    source_commitment: self.rows_commitment.get(),
                    cursor_retained_rows: 0,
                    cursor_retained_bytes: 0,
                },
                FixtureExpectedCursor { next: 0 },
            ))
        }

        fn read_expected_path_page(
            &self,
            cursor: &mut Self::Cursor,
            request: ExpectedPathPageRequest,
        ) -> Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure> {
            let mut count = 0_usize;
            let mut path_bytes = 0_u64;
            let mut retained_bytes = 0_u64;
            while let Some(row) = self.rows.get(cursor.next + count) {
                let next_path_bytes = path_bytes
                    .checked_add(row.path.as_str().len() as u64)
                    .ok_or(ExpectedPathSourceFailure::Unavailable)?;
                let next_retained = retained_bytes
                    .checked_add(mem::size_of::<AuthenticatedExpectedPath>() as u64)
                    .and_then(|bytes| bytes.checked_add(row.path.as_str().len() as u64))
                    .ok_or(ExpectedPathSourceFailure::Unavailable)?;
                if count == request.maximum_rows
                    || count == request.maximum_retained_rows
                    || next_path_bytes > request.maximum_aggregate_path_bytes
                    || next_retained > request.maximum_retained_bytes
                {
                    break;
                }
                if row.path.as_str().len() > request.maximum_path_bytes {
                    return Err(ExpectedPathSourceFailure::BoundExceeded);
                }
                count += 1;
                path_bytes = next_path_bytes;
                retained_bytes = next_retained;
            }
            if count == 0 && cursor.next < self.rows.len() {
                return Err(ExpectedPathSourceFailure::BoundExceeded);
            }
            let end = cursor.next + count;
            let rows = self.rows[cursor.next..end].to_vec();
            self.maximum_page_rows_seen
                .set(self.maximum_page_rows_seen.get().max(rows.len()));
            let page_bytes = (rows.capacity() as u64)
                .saturating_mul(mem::size_of::<AuthenticatedExpectedPath>() as u64)
                .saturating_add(
                    rows.iter()
                        .map(|row| row.path.as_str().len() as u64)
                        .sum::<u64>(),
                );
            self.maximum_page_bytes_seen
                .set(self.maximum_page_bytes_seen.get().max(page_bytes));
            cursor.next = end;
            Ok(AuthenticatedExpectedPathPage {
                rows,
                done: cursor.next == self.rows.len(),
            })
        }

        fn current_binding(
            &self,
            _maximum_retained_bytes: u64,
        ) -> Result<ExpectedPathBinding, ExpectedPathSourceFailure> {
            Ok(self.binding.get())
        }

        fn current_scan_identity(
            &self,
            _maximum_retained_bytes: u64,
        ) -> Result<(ExpectedPathBinding, ContentDigest), ExpectedPathSourceFailure> {
            Ok((self.binding.get(), self.rows_commitment.get()))
        }

        fn expected_path_at(
            &self,
            path: &ManagedPath,
            request: ExpectedPathPointRequest,
        ) -> Result<Option<AuthenticatedExpectedPath>, ExpectedPathSourceFailure> {
            if let Some(failure) = self.failure {
                return Err(failure);
            }
            if self.ambiguous {
                return Err(ExpectedPathSourceFailure::Ambiguous);
            }
            if path.as_str().len() > request.maximum_path_bytes
                || request.maximum_retained_rows == 0
                || request.maximum_retained_bytes
                    < mem::size_of::<AuthenticatedExpectedPath>() as u64
                        + path.as_str().len() as u64
            {
                return Err(ExpectedPathSourceFailure::BoundExceeded);
            }
            Ok(self.rows.iter().find(|row| row.path == *path).cloned())
        }
    }

    fn expected_binding(generation: u64) -> ExpectedPathBinding {
        ExpectedPathBinding {
            accepted_frontier: ContentDigest::of(format!("frontier-{generation}").as_bytes()),
            projection_generation: generation,
        }
    }

    fn expected_row(path: &str, kind: ManagedTextKind, bytes: &[u8]) -> AuthenticatedExpectedPath {
        let path_digest = ContentDigest::of(path.as_bytes());
        let page_bytes: [u8; 16] = path_digest.as_bytes()[..16].try_into().unwrap();
        AuthenticatedExpectedPath {
            page_id: PageId::from_uuid(Uuid::from_bytes(page_bytes)),
            path: ManagedPath::parse(path).unwrap(),
            kind,
            description: BlobDescription::of(bytes),
            owner_binding: ContentDigest::of(format!("owner:{path}").as_bytes()),
        }
    }

    fn candidate_signature(
        scan: &StableGraphTextScan,
    ) -> Vec<(String, GraphTextCandidateKind, Option<ManagedTextKind>)> {
        scan.candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.path.as_str().to_owned(),
                    candidate.change,
                    candidate.managed_kind,
                )
            })
            .collect()
    }

    fn assert_failure(
        result: Result<StableGraphTextScan, GraphTextScanFailure>,
        class: GraphTextScanFailureClass,
        reason: GraphTextScanFailureReason,
    ) {
        let failure = result.expect_err("scan should fail closed");
        assert_eq!(failure.class, class);
        assert_eq!(failure.reason, reason);
        assert_eq!(failure.instrumentation.candidates, 0);
    }

    #[test]
    fn reconciliation_scan_stable_create_edit_delete_rename_copy_union() {
        let temp = TempGraph::new(None);
        temp.write("pages/unchanged.md", b"same");
        temp.write("pages/edit.md", b"new");
        temp.write("pages/rename-new.md", b"renamed");
        temp.write("pages/copy-source.md", b"copied");
        temp.write("pages/copy.md", b"copied");
        temp.write("pages/create.md", b"created");
        let graph = temp.graph();
        let source = FixtureExpectedSource::with_rows(vec![
            expected_row("pages/unchanged.md", ManagedTextKind::Page, b"same"),
            expected_row("pages/edit.md", ManagedTextKind::Page, b"old"),
            expected_row("pages/delete.md", ManagedTextKind::Page, b"deleted"),
            expected_row("pages/rename-old.md", ManagedTextKind::Page, b"renamed"),
            expected_row("pages/copy-source.md", ManagedTextKind::Page, b"copied"),
        ]);

        let scan = scan_graph_text(&graph, &source, GraphTextScanLimits::default()).unwrap();
        assert_eq!(
            candidate_signature(&scan),
            vec![
                (
                    "pages/copy.md".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "pages/create.md".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "pages/delete.md".into(),
                    GraphTextCandidateKind::Absence,
                    Some(ManagedTextKind::Page)
                ),
                (
                    "pages/edit.md".into(),
                    GraphTextCandidateKind::Edit,
                    Some(ManagedTextKind::Page)
                ),
                (
                    "pages/rename-new.md".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "pages/rename-old.md".into(),
                    GraphTextCandidateKind::Absence,
                    Some(ManagedTextKind::Page)
                ),
            ]
        );
        assert!(scan
            .candidates
            .iter()
            .all(|candidate| candidate.binding == scan.binding));
    }

    #[test]
    fn reconciliation_scan_mutation_between_passes_is_unstable_and_candidate_free() {
        let temp = TempGraph::new(None);
        temp.write("pages/page.md", b"before");
        let graph = temp.graph();
        let source = FixtureExpectedSource::with_rows(vec![expected_row(
            "pages/page.md",
            ManagedTextKind::Page,
            b"before",
        )]);
        let path = temp.root.join("pages/page.md");
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::write(&path, b"after")
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::FilesystemEvidenceChanged,
        );
    }

    #[test]
    fn reconciliation_scan_expected_binding_change_is_unstable() {
        let temp = TempGraph::new(None);
        temp.write("pages/page.md", b"page");
        let graph = temp.graph();
        let source = FixtureExpectedSource::empty();
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                source.binding.set(expected_binding(2));
                Ok(())
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::ExpectedBindingChanged,
        );
    }

    #[test]
    fn reconciliation_scan_stable_prepass_config_change_requires_graph_refresh() {
        let temp = TempGraph::new(Some(
            "{:pages-directory \"pages\" :journals-directory \"journals\"}\n",
        ));
        temp.write("new-pages/page.md", b"page");
        let graph = temp.graph();
        fs::write(
            temp.root.join("logseq/config.edn"),
            "{:pages-directory \"new-pages\" :journals-directory \"new-journals\"}\n",
        )
        .unwrap();

        assert_failure(
            scan_graph_text(
                &graph,
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::ConfigRefreshRequired,
        );
    }

    #[test]
    fn reconciliation_scan_binds_case_insensitive_config_path_at_graph_open() {
        let temp = TempGraph::new(None);
        temp.write(
            "LoGsEq/CoNfIg.EdN",
            b"{:pages-directory \"content\" :journals-directory \"daily\" :hidden [\"private\"]}\n",
        );
        temp.write("content/Page.MARKDOWN", b"page");
        temp.write("private/hidden.org", b"hidden");
        let scan = scan_graph_text(
            &temp.graph(),
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap();

        assert_eq!(
            candidate_signature(&scan),
            vec![(
                "content/Page.MARKDOWN".to_owned(),
                GraphTextCandidateKind::Creation,
                None,
            )]
        );
    }

    #[test]
    fn reconciliation_scan_directory_and_config_replacement_are_unstable() {
        let temp = TempGraph::new(Some(
            "{:pages-directory \"pages\" :journals-directory \"journals\"}\n",
        ));
        temp.write("pages/page.md", b"page");
        let graph = temp.graph();
        let source = FixtureExpectedSource::empty();
        let directory = temp.root.join("pages");
        let moved = temp.root.join("pages-old");
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::rename(&directory, &moved)?;
                fs::create_dir(&directory)?;
                fs::write(directory.join("page.md"), b"page")
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::FilesystemEvidenceChanged,
        );
        fs::remove_dir_all(&moved).unwrap();

        let graph = temp.graph();
        let config = temp.root.join("logseq/config.edn");
        let replacement = temp.root.join("logseq/config.next");
        fs::write(
            &replacement,
            "{:pages-directory \"pages\" :journals-directory \"journals\"}\n",
        )
        .unwrap();
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::remove_file(&config)?;
                fs::rename(&replacement, &config)
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::FilesystemEvidenceChanged,
        );
    }

    #[cfg(unix)]
    #[test]
    fn reconciliation_scan_file_resource_and_link_count_change_fail_closed() {
        let temp = TempGraph::new(None);
        temp.write("pages/page.md", b"page");
        let graph = temp.graph();
        let source = FixtureExpectedSource::empty();
        let page = temp.root.join("pages/page.md");
        let replacement = temp.root.join("pages/replacement.tmp");
        fs::write(&replacement, b"page").unwrap();
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::remove_file(&page)?;
                fs::rename(&replacement, &page)
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::FilesystemEvidenceChanged,
        );

        let graph = temp.graph();
        let outside_link =
            std::env::temp_dir().join(format!("tine-scan-link-{}.md", Uuid::new_v4()));
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::hard_link(&page, &outside_link)
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );
        fs::remove_file(outside_link).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reconciliation_scan_root_replacement_is_unstable() {
        let temp = TempGraph::new(None);
        temp.write("pages/page.md", b"page");
        let graph = temp.graph();
        let source = FixtureExpectedSource::empty();
        let moved = temp
            .root
            .with_file_name(format!("tine-scan-moved-{}", Uuid::new_v4()));
        let root = temp.root.clone();
        let result =
            scan_graph_text_with_hook(&graph, &source, GraphTextScanLimits::default(), || {
                fs::rename(&root, &moved)?;
                fs::create_dir(&root)?;
                fs::create_dir(root.join("pages"))?;
                fs::write(root.join("pages/page.md"), b"page")
            });
        assert_failure(
            result,
            GraphTextScanFailureClass::UnstableEpoch,
            GraphTextScanFailureReason::FilesystemEvidenceChanged,
        );
        fs::remove_dir_all(&moved).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reconciliation_scan_rejects_symlink_and_hardlink_ambiguity() {
        use std::os::unix::fs::symlink;

        let temp = TempGraph::new(None);
        temp.write("pages/source.md", b"page");
        symlink("source.md", temp.root.join("pages/link.md")).unwrap();
        let graph = temp.graph();
        assert_failure(
            scan_graph_text(
                &graph,
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );

        fs::remove_file(temp.root.join("pages/link.md")).unwrap();
        fs::hard_link(
            temp.root.join("pages/source.md"),
            temp.root.join("pages/hard.md"),
        )
        .unwrap();
        let graph = temp.graph();
        assert_failure(
            scan_graph_text(
                &graph,
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );
    }

    #[test]
    fn reconciliation_scan_mixed_extensions_and_longest_nested_roots() {
        let temp = TempGraph::new(Some(
            "{:pages-directory \"managed/text\"\n\
              :journals-directory \"managed/text/daily\"}\n",
        ));
        for (path, bytes) in [
            ("Root.MD", b"root".as_slice()),
            ("archive/nested.Markdown", b"nested".as_slice()),
            ("managed/text/Page.mD", b"page".as_slice()),
            ("managed/text/Another.MARKDOWN", b"markdown".as_slice()),
            ("managed/text/daily/2026-07-26.ORG", b"journal".as_slice()),
        ] {
            temp.write(path, bytes);
        }
        let scan = scan_graph_text(
            &temp.graph(),
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap();
        assert_eq!(
            candidate_signature(&scan),
            vec![
                ("Root.MD".into(), GraphTextCandidateKind::Creation, None),
                (
                    "archive/nested.Markdown".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "managed/text/Another.MARKDOWN".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "managed/text/Page.mD".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
                (
                    "managed/text/daily/2026-07-26.ORG".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
            ]
        );
        assert!(scan.diagnostics.is_empty());
    }

    #[test]
    fn reconciliation_scan_rejects_equal_or_portably_aliased_creation_roots() {
        for config in [
            "{:pages-directory \"content\" :journals-directory \"content\"}\n",
            "{:pages-directory \"Pages\" :journals-directory \"pages\"}\n",
        ] {
            let temp = TempGraph::new(Some(config));
            assert_failure(
                scan_graph_text(
                    &temp.graph(),
                    &FixtureExpectedSource::empty(),
                    GraphTextScanLimits::default(),
                ),
                GraphTextScanFailureClass::Blocked,
                GraphTextScanFailureReason::UnsafeFilesystem,
            );
        }
    }

    #[test]
    fn reconciliation_scan_rejects_portable_disk_path_collisions() {
        let temp = TempGraph::new(None);
        temp.write("pages/Page.md", b"one");
        temp.write("pages/page.MD", b"two");
        assert_failure(
            scan_graph_text(
                &temp.graph(),
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );
    }

    #[test]
    fn reconciliation_scan_bounds_fail_before_candidates() {
        let temp = TempGraph::new(None);
        temp.write("pages/a.md", b"0123456789");
        temp.write("pages/deep/b.md", b"b");
        let graph = temp.graph();
        let cases = [
            GraphTextScanLimits {
                all_entries: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                eligible_files: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                directories: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                pending_directories: 0,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                directory_depth: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                aggregate_path_bytes: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                aggregate_hashed_bytes: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                retained_rows: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                retained_bytes: 1,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                exact_path_bytes: 2,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                read_buffer_bytes: GRAPH_TEXT_SCAN_READ_BUFFER_BYTES + 1,
                ..GraphTextScanLimits::default()
            },
        ];
        for limits in cases {
            assert_failure(
                scan_graph_text(&graph, &FixtureExpectedSource::empty(), limits),
                GraphTextScanFailureClass::Blocked,
                GraphTextScanFailureReason::BoundExceeded,
            );
        }

        let measured = scan_graph_text(
            &graph,
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap()
        .instrumentation;
        for limits in [
            GraphTextScanLimits {
                retained_rows: (measured.peak_retained_rows - 1) as usize,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                retained_bytes: measured.peak_retained_bytes - 1,
                ..GraphTextScanLimits::default()
            },
        ] {
            assert_failure(
                scan_graph_text(&graph, &FixtureExpectedSource::empty(), limits),
                GraphTextScanFailureClass::Blocked,
                GraphTextScanFailureReason::BoundExceeded,
            );
        }
    }

    #[test]
    fn reconciliation_scan_expected_cursor_is_paged_and_never_rematerialized() {
        let temp = TempGraph::new(None);
        let source = FixtureExpectedSource::with_rows(
            (0..600)
                .map(|index| {
                    expected_row(
                        &format!("archive/{index:04}.md"),
                        ManagedTextKind::Page,
                        b"expected",
                    )
                })
                .collect(),
        );
        let limits = GraphTextScanLimits {
            expected_page_rows: 17,
            expected_page_bytes: 4096,
            ..GraphTextScanLimits::default()
        };
        let scan = scan_graph_text(&temp.graph(), &source, limits).unwrap();

        assert_eq!(scan.candidates.len(), 600);
        assert_eq!(source.open_calls.get(), 2);
        assert!(source.maximum_page_rows_seen.get() <= 17);
        assert!(source.maximum_page_bytes_seen.get() <= 4096);
        assert!(scan.instrumentation.expected_pages > 2);
        assert_eq!(scan.instrumentation.expected_rows, 600);
        assert!(scan.instrumentation.expected_path_bytes > 600);
    }

    #[test]
    fn reconciliation_scan_expected_path_and_page_byte_bounds_are_causal() {
        let temp = TempGraph::new(None);
        let graph = temp.graph();
        let source = FixtureExpectedSource::with_rows(vec![expected_row(
            "archive/a-very-long-expected-path.md",
            ManagedTextKind::Page,
            b"expected",
        )]);
        for limits in [
            GraphTextScanLimits {
                exact_path_bytes: 8,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                aggregate_expected_path_bytes: 8,
                ..GraphTextScanLimits::default()
            },
            GraphTextScanLimits {
                expected_page_bytes: 1,
                ..GraphTextScanLimits::default()
            },
        ] {
            assert_failure(
                scan_graph_text(&graph, &source, limits),
                GraphTextScanFailureClass::Blocked,
                GraphTextScanFailureReason::BoundExceeded,
            );
        }
    }

    #[test]
    fn reconciliation_scan_pass_b_and_output_memory_never_cross_the_live_limit() {
        let temp = TempGraph::new(None);
        temp.write("Root.md", b"root");
        temp.write(
            "pages/page.sync-conflict-20260726-120000-ABCDEF.md",
            b"provider",
        );
        let graph = temp.graph();
        let source = FixtureExpectedSource::with_rows(
            (0..1_000)
                .map(|index| {
                    expected_row(
                        &format!("archive/missing-{index:04}.md"),
                        ManagedTextKind::Page,
                        b"missing",
                    )
                })
                .collect(),
        );
        let test_limits = GraphTextScanLimits {
            expected_page_rows: 1,
            ..GraphTextScanLimits::default()
        };
        let success = scan_graph_text(&graph, &source, test_limits).unwrap();
        assert_eq!(success.candidates.len(), 1_001);
        assert_eq!(success.diagnostics.len(), 1);

        let one_pass = graph
            .capture_reconciliation_scan_pass(GraphTextScanLimits::default())
            .unwrap()
            .instrumentation;
        assert!(
            success.instrumentation.peak_retained_bytes
                > one_pass.peak_retained_bytes.saturating_mul(2)
        );
        let causal_limit = one_pass
            .peak_retained_bytes
            .saturating_mul(2)
            .saturating_sub(1);
        let hook_ran = Cell::new(false);
        let failure = scan_graph_text_with_hook(
            &graph,
            &source,
            GraphTextScanLimits {
                retained_bytes: causal_limit,
                ..test_limits
            },
            || {
                hook_ran.set(true);
                Ok(())
            },
        )
        .expect_err("Pass B must receive only Pass A's remaining retained budget");
        assert!(hook_ran.get());
        assert_eq!(failure.reason, GraphTextScanFailureReason::BoundExceeded);
        assert!(failure.instrumentation.peak_retained_bytes <= causal_limit);

        let output_limit = success.instrumentation.peak_retained_bytes - 1;
        let output_failure = scan_graph_text(
            &graph,
            &source,
            GraphTextScanLimits {
                retained_bytes: output_limit,
                ..test_limits
            },
        )
        .expect_err("candidate, diagnostic, validation, and page memory must be charged");
        assert_eq!(
            output_failure.reason,
            GraphTextScanFailureReason::BoundExceeded
        );
        assert!(output_failure.instrumentation.peak_retained_bytes <= output_limit);
    }

    #[cfg(unix)]
    #[test]
    fn reconciliation_scan_rejects_non_utf8_and_nonportable_physical_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let temp = TempGraph::new(None);
        fs::create_dir_all(temp.root.join("pages")).unwrap();
        fs::write(temp.root.join("pages/bad:name.md"), b"bad").unwrap();
        assert_failure(
            scan_graph_text(
                &temp.graph(),
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );
        fs::remove_file(temp.root.join("pages/bad:name.md")).unwrap();
        let non_utf8 = OsString::from_vec(vec![b'n', b'o', b'n', 0xff, b'.', b'm', b'd']);
        fs::write(temp.root.join("pages").join(non_utf8), b"bad").unwrap();
        assert_failure(
            scan_graph_text(
                &temp.graph(),
                &FixtureExpectedSource::empty(),
                GraphTextScanLimits::default(),
            ),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::UnsafeFilesystem,
        );
    }

    #[test]
    fn reconciliation_scan_absence_and_creation_do_not_depend_on_a_baseline() {
        let temp = TempGraph::new(None);
        temp.write("pages/new.md", b"new");
        let graph = temp.graph();

        let empty = scan_graph_text(
            &graph,
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap();
        assert_eq!(
            candidate_signature(&empty),
            vec![(
                "pages/new.md".into(),
                GraphTextCandidateKind::Creation,
                None
            )]
        );

        let expected_missing = scan_graph_text(
            &graph,
            &FixtureExpectedSource::with_rows(vec![expected_row(
                "pages/missing.md",
                ManagedTextKind::Page,
                b"missing",
            )]),
            GraphTextScanLimits::default(),
        )
        .unwrap();
        assert_eq!(
            candidate_signature(&expected_missing),
            vec![
                (
                    "pages/missing.md".into(),
                    GraphTextCandidateKind::Absence,
                    Some(ManagedTextKind::Page)
                ),
                (
                    "pages/new.md".into(),
                    GraphTextCandidateKind::Creation,
                    None
                ),
            ]
        );
    }

    #[test]
    fn reconciliation_scan_missing_corrupt_or_ambiguous_expected_authority_blocks() {
        let temp = TempGraph::new(None);
        temp.write("pages/new.md", b"new");
        let graph = temp.graph();
        for (failure, reason) in [
            (
                ExpectedPathSourceFailure::Missing,
                GraphTextScanFailureReason::ExpectedAuthorityMissing,
            ),
            (
                ExpectedPathSourceFailure::Corrupt,
                GraphTextScanFailureReason::ExpectedAuthorityCorrupt,
            ),
            (
                ExpectedPathSourceFailure::Ambiguous,
                GraphTextScanFailureReason::ExpectedAuthorityAmbiguous,
            ),
        ] {
            assert_failure(
                scan_graph_text(
                    &graph,
                    &FixtureExpectedSource::failure(failure),
                    GraphTextScanLimits::default(),
                ),
                GraphTextScanFailureClass::Blocked,
                reason,
            );
        }

        let corrupt = FixtureExpectedSource::empty();
        corrupt.rows_commitment.set(ContentDigest::of(b"forged"));
        assert_failure(
            scan_graph_text(&graph, &corrupt, GraphTextScanLimits::default()),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::ExpectedAuthorityCorrupt,
        );

        let duplicate = expected_row("pages/a.md", ManagedTextKind::Page, b"a");
        let ambiguous = FixtureExpectedSource::with_rows(vec![duplicate.clone(), duplicate]);
        assert_failure(
            scan_graph_text(&graph, &ambiguous, GraphTextScanLimits::default()),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::ExpectedAuthorityAmbiguous,
        );

        let portable_ambiguous = FixtureExpectedSource::with_rows(vec![
            expected_row("pages/Page.md", ManagedTextKind::Page, b"a"),
            expected_row("pages/page.MD", ManagedTextKind::Page, b"b"),
        ]);
        assert_failure(
            scan_graph_text(&graph, &portable_ambiguous, GraphTextScanLimits::default()),
            GraphTextScanFailureClass::Blocked,
            GraphTextScanFailureReason::ExpectedAuthorityAmbiguous,
        );
    }

    #[test]
    fn reconciliation_scan_provider_conflict_copy_is_preserved_and_classified() {
        let temp = TempGraph::new(None);
        let conflict = "pages/page.sync-conflict-20260726-120000-ABCDEF.md";
        temp.write(conflict, b"provider copy");
        let scan = scan_graph_text(
            &temp.graph(),
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap();
        assert!(scan.candidates.is_empty());
        assert_eq!(scan.diagnostics.len(), 1);
        assert_eq!(scan.diagnostics[0].path, conflict);
        assert_eq!(
            scan.diagnostics[0].kind,
            GraphTextScanDiagnosticKind::ProviderConflictCopy
        );
        assert_eq!(
            fs::read(temp.root.join(conflict)).unwrap(),
            b"provider copy"
        );
    }

    #[test]
    fn reconciliation_scan_hashes_unchanged_files_without_parsing_and_is_deterministic() {
        let temp = TempGraph::new(None);
        temp.write("pages/page.md", b"- unchanged\n");
        let graph = temp.graph();
        let source = FixtureExpectedSource::with_rows(vec![expected_row(
            "pages/page.md",
            ManagedTextKind::Page,
            b"- unchanged\n",
        )]);
        reset_graph_text_parser_counter_for_scan_test();
        let first = scan_graph_text(&graph, &source, GraphTextScanLimits::default()).unwrap();
        let second = scan_graph_text(&graph, &source, GraphTextScanLimits::default()).unwrap();
        assert!(first.candidates.is_empty());
        assert!(second.candidates.is_empty());
        assert_eq!(first.binding, second.binding);
        assert_eq!(first.instrumentation, second.instrumentation);
        assert_eq!(first.instrumentation.bytes_hashed, 2 * 12);
        assert_eq!(first.instrumentation.parser_invocations, 0);
        assert_eq!(graph_text_parser_invocations_for_scan_test(), 0);
    }

    #[test]
    #[ignore = "measured 10k-page scan harness; run explicitly for a platform receipt"]
    fn reconciliation_scan_measured_10k_page_harness() {
        let temp = TempGraph::new(None);
        for index in 0..10_000 {
            temp.write(
                &format!("pages/{index:05}.md"),
                format!("- page {index:05}\n").as_bytes(),
            );
        }
        let scan = scan_graph_text(
            &temp.graph(),
            &FixtureExpectedSource::empty(),
            GraphTextScanLimits::default(),
        )
        .unwrap();
        println!(
            "scan_10k wall_ms={} peak_retained_rows={} peak_retained_bytes={} \
             peak_read_buffers={} peak_read_buffer_bytes={} bytes_read={} \
             bytes_hashed={} entries={} candidates={}",
            scan.wall_time.as_millis(),
            scan.instrumentation.peak_retained_rows,
            scan.instrumentation.peak_retained_bytes,
            scan.instrumentation.peak_read_buffers,
            scan.instrumentation.peak_read_buffer_bytes,
            scan.instrumentation.bytes_read,
            scan.instrumentation.bytes_hashed,
            scan.instrumentation.directory_entries,
            scan.instrumentation.candidates,
        );
        assert_eq!(scan.instrumentation.passes, 2);
        assert_eq!(scan.instrumentation.eligible_files, 20_000);
        assert_eq!(scan.candidates.len(), 10_000);
        assert_eq!(scan.instrumentation.parser_invocations, 0);
    }

    fn scheduler_paths(paths: &[&str]) -> BTreeSet<ManagedPath> {
        paths
            .iter()
            .map(|path| ManagedPath::parse(*path).unwrap())
            .collect()
    }

    fn full_scan_reasons(job: &ReconciliationJob) -> &ReconciliationFullScanReasons {
        match job.work() {
            ReconciliationWork::FullScan(reasons) => reasons,
            work => panic!("expected full scan work, got {work:?}"),
        }
    }

    #[test]
    fn reconciliation_scheduler_coalesces_full_reasons_and_supersedes_watcher_hints() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/a.md",
            "pages/b.md",
        ])));
        scheduler.trigger(ReconciliationTrigger::Periodic);
        scheduler.trigger(ReconciliationTrigger::Startup);
        scheduler.trigger(ReconciliationTrigger::BaselineUnavailable);
        scheduler.trigger(ReconciliationTrigger::WatcherUncertain);
        scheduler.trigger(ReconciliationTrigger::Explicit);

        let job = scheduler.next().unwrap();
        assert_eq!(
            full_scan_reasons(&job).reasons,
            BTreeSet::from([
                ReconciliationFullScanReason::Explicit,
                ReconciliationFullScanReason::WatcherUncertain,
                ReconciliationFullScanReason::Startup,
                ReconciliationFullScanReason::BaselineUnavailable,
                ReconciliationFullScanReason::Periodic,
            ])
        );
        assert!(!full_scan_reasons(&job).omitted_reasons);
        scheduler
            .complete(job.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn reconciliation_scheduler_prioritizes_exact_preconditions_before_full_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/watcher.md",
        ])));
        scheduler.trigger(ReconciliationTrigger::Periodic);
        scheduler.trigger(ReconciliationTrigger::Explicit);
        scheduler.trigger(ReconciliationTrigger::ProjectionPreconditionMismatch(
            scheduler_paths(&["pages/z.md", "pages/a.md"]),
        ));

        let targeted = scheduler.next().unwrap();
        assert_eq!(
            targeted.work(),
            &ReconciliationWork::ProjectionPreconditionMismatch {
                paths: scheduler_paths(&["pages/a.md", "pages/z.md"]),
            }
        );
        scheduler
            .complete(targeted.lease(), ReconciliationCompletionOutcome::Noop)
            .unwrap();

        let full = scheduler.next().unwrap();
        assert!(full_scan_reasons(&full)
            .reasons
            .contains(&ReconciliationFullScanReason::Explicit));
        assert!(full_scan_reasons(&full)
            .reasons
            .contains(&ReconciliationFullScanReason::Periodic));
        scheduler
            .complete(full.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn reconciliation_scheduler_is_single_flight_and_retains_arrivals_during_active_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/first.md",
        ])));
        let first = scheduler.next().unwrap();
        assert!(scheduler.status().active);
        assert!(scheduler.next().is_none());

        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/second.md",
        ])));
        scheduler.trigger(ReconciliationTrigger::ProjectionPreconditionMismatch(
            scheduler_paths(&["pages/urgent.md"]),
        ));
        assert!(scheduler.status().pending);
        assert!(scheduler.next().is_none());
        scheduler
            .complete(first.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();

        let urgent = scheduler.next().unwrap();
        assert_eq!(
            urgent.work(),
            &ReconciliationWork::ProjectionPreconditionMismatch {
                paths: scheduler_paths(&["pages/urgent.md"]),
            }
        );
        scheduler
            .complete(urgent.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        let second = scheduler.next().unwrap();
        assert_eq!(
            second.work(),
            &ReconciliationWork::WatcherPaths {
                paths: scheduler_paths(&["pages/second.md"]),
            }
        );
    }

    #[test]
    fn reconciliation_scheduler_rejects_foreign_stale_and_double_completion_tokens() {
        let mut first_scheduler =
            ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        let mut second_scheduler =
            ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        first_scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/one.md",
        ])));
        second_scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/two.md",
        ])));
        let first = first_scheduler.next().unwrap();
        let foreign = second_scheduler.next().unwrap();

        assert_eq!(
            first_scheduler.complete(foreign.lease(), ReconciliationCompletionOutcome::Complete),
            Err(ReconciliationCompletionError::StaleOrForeignLease)
        );
        assert!(first_scheduler.status().active);
        first_scheduler
            .complete(first.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();

        first_scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/three.md",
        ])));
        let second = first_scheduler.next().unwrap();
        assert_eq!(
            first_scheduler.complete(first.lease(), ReconciliationCompletionOutcome::Complete),
            Err(ReconciliationCompletionError::StaleOrForeignLease)
        );
        first_scheduler
            .complete(second.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        assert_eq!(
            first_scheduler.complete(second.lease(), ReconciliationCompletionOutcome::Complete),
            Err(ReconciliationCompletionError::NoActiveJob)
        );
    }

    #[test]
    fn reconciliation_scheduler_watcher_count_and_byte_overflow_become_full_work() {
        let limits = ReconciliationSchedulerLimits {
            maximum_watcher_paths: 1,
            maximum_watcher_path_bytes: 64,
            ..ReconciliationSchedulerLimits::default()
        };
        let mut scheduler = ReconciliationScheduler::new(limits);
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/a.md",
            "pages/b.md",
        ])));
        let count_overflow = scheduler.next().unwrap();
        assert!(full_scan_reasons(&count_overflow)
            .reasons
            .contains(&ReconciliationFullScanReason::WatcherPathOverflow));

        let mut byte_limited = ReconciliationScheduler::new(ReconciliationSchedulerLimits {
            maximum_watcher_path_bytes: 1,
            ..ReconciliationSchedulerLimits::default()
        });
        byte_limited.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/a.md",
        ])));
        let byte_overflow = byte_limited.next().unwrap();
        assert!(full_scan_reasons(&byte_overflow)
            .reasons
            .contains(&ReconciliationFullScanReason::WatcherPathOverflow));
    }

    #[test]
    fn reconciliation_scheduler_precondition_overflow_preserves_bounded_paths_then_full_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits {
            maximum_precondition_paths: 1,
            maximum_precondition_path_bytes: 64,
            ..ReconciliationSchedulerLimits::default()
        });
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/watcher.md",
        ])));
        scheduler.trigger(ReconciliationTrigger::ProjectionPreconditionMismatch(
            scheduler_paths(&["pages/a.md", "pages/b.md"]),
        ));

        let targeted = scheduler.next().unwrap();
        assert_eq!(
            targeted.work(),
            &ReconciliationWork::ProjectionPreconditionMismatch {
                paths: scheduler_paths(&["pages/a.md"]),
            }
        );
        scheduler
            .complete(targeted.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        let full = scheduler.next().unwrap();
        assert!(full_scan_reasons(&full)
            .reasons
            .contains(&ReconciliationFullScanReason::ProjectionPreconditionPathOverflow));
        scheduler
            .complete(full.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn reconciliation_scheduler_bounds_full_scan_diagnostics_without_dropping_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits {
            maximum_full_scan_reasons: 1,
            ..ReconciliationSchedulerLimits::default()
        });
        scheduler.trigger(ReconciliationTrigger::Startup);
        scheduler.trigger(ReconciliationTrigger::Explicit);

        let job = scheduler.next().unwrap();
        let reasons = full_scan_reasons(&job);
        assert_eq!(reasons.reasons.len(), 1);
        assert!(reasons.omitted_reasons);
    }

    #[test]
    fn reconciliation_scheduler_blocked_is_observable_and_retry_schedules_safe_full_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/blocked.md",
        ])));
        let blocked = scheduler.next().unwrap();
        scheduler
            .complete(blocked.lease(), ReconciliationCompletionOutcome::Blocked)
            .unwrap();
        assert_eq!(
            scheduler.status().last_completion,
            Some(ReconciliationCompletionOutcome::Blocked)
        );
        assert_eq!(scheduler.status().blocked.unwrap().lease, blocked.lease());

        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/retry.md",
        ])));
        let retrying = scheduler.next().unwrap();
        scheduler
            .complete(retrying.lease(), ReconciliationCompletionOutcome::Retry)
            .unwrap();
        assert!(scheduler.status().blocked.is_some());
        let retry = scheduler.next().unwrap();
        assert!(full_scan_reasons(&retry)
            .reasons
            .contains(&ReconciliationFullScanReason::Retry));
        scheduler
            .complete(retry.lease(), ReconciliationCompletionOutcome::Complete)
            .unwrap();
        assert!(scheduler.status().blocked.is_none());

        scheduler.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/uncertain.md",
        ])));
        let uncertain = scheduler.next().unwrap();
        scheduler
            .complete(
                uncertain.lease(),
                ReconciliationCompletionOutcome::Uncertain,
            )
            .unwrap();
        let uncertain_retry = scheduler.next().unwrap();
        assert!(full_scan_reasons(&uncertain_retry)
            .reasons
            .contains(&ReconciliationFullScanReason::Uncertain));
    }

    #[test]
    fn reconciliation_scheduler_periodic_trigger_always_requests_full_work() {
        let mut scheduler = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        scheduler.trigger(ReconciliationTrigger::Periodic);
        let periodic = scheduler.next().unwrap();
        assert!(full_scan_reasons(&periodic)
            .reasons
            .contains(&ReconciliationFullScanReason::Periodic));
        scheduler
            .complete(periodic.lease(), ReconciliationCompletionOutcome::Noop)
            .unwrap();
        assert!(scheduler.next().is_none());
    }

    #[test]
    fn reconciliation_schedulers_are_independent_per_endpoint() {
        let mut first = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        let mut second = ReconciliationScheduler::new(ReconciliationSchedulerLimits::default());
        first.trigger(ReconciliationTrigger::WatcherPaths(scheduler_paths(&[
            "pages/first.md",
        ])));
        second.trigger(ReconciliationTrigger::Periodic);

        let first_job = first.next().unwrap();
        let second_job = second.next().unwrap();
        assert_ne!(first_job.lease(), second_job.lease());
        assert!(matches!(
            first_job.work(),
            ReconciliationWork::WatcherPaths { .. }
        ));
        assert!(matches!(second_job.work(), ReconciliationWork::FullScan(_)));
        assert!(first.status().active);
        assert!(second.status().active);
    }

    #[allow(dead_code)]
    fn _assert_path_is_inside_temp_graph(path: &Path, temp: &TempGraph) {
        assert!(path.starts_with(&temp.root));
    }
}
