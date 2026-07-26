//! Inactive stable-scan to disposable-baseline persistence adapter.
//!
//! This module stores diagnostics only. Its output is never import, oplog,
//! Markdown-write, scan-suppression, or hash-suppression authority. The only
//! clean-head transition admitted here is a still-current, stable scan with no
//! candidates and no blocking diagnostics.

use sha2::{Digest, Sha256};
use std::{fmt, mem};

use super::{
    reconciliation_baseline::{
        BaselineBlockedReason, BaselineDirectoryPath, BaselineEpochId, BaselineEpochOutcome,
        BaselineHead, BaselineObservedState, BaselineScanDirectory, BaselineScanInstrumentation,
        BaselineScanPath, BaselineScanRowsIdentity, BaselineScanRowsIdentityBuilder,
        BaselineTimestamp, BeginBaselineEpoch, FinishBaselineEpoch, ReconciliationBaseline,
        ReconciliationBaselineError, MAX_BASELINE_WRITE_ROWS,
    },
    reconciliation_scan::{
        AuthenticatedExpectedPathSource, ExpectedPathSourceFailure, GraphTextCandidateKind,
        GraphTextScanDiagnostic, GraphTextScanFileFingerprint, GraphTextScanPathClass,
        StableGraphTextBaselineIdentity, StableGraphTextScan,
    },
    ContentDigest, ManagedPath,
};

#[derive(Debug)]
pub(crate) enum BaselineUnavailableCause {
    Store(ReconciliationBaselineError),
    ExpectedAuthority(ExpectedPathSourceFailure),
    StableScanBindingMismatch,
    StableScanBindingChanged,
    InvalidStableScanEvidence(String),
}

/// Typed fail-closed result for every binding, schema, corruption, race, and
/// limit failure observed by this adapter.
#[derive(Debug)]
pub(crate) struct BaselineUnavailable {
    pub(crate) cause: BaselineUnavailableCause,
}

impl fmt::Display for BaselineUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("disposable reconciliation baseline unavailable: ")?;
        match &self.cause {
            BaselineUnavailableCause::Store(error) => write!(formatter, "{error}"),
            BaselineUnavailableCause::ExpectedAuthority(error) => write!(formatter, "{error}"),
            BaselineUnavailableCause::StableScanBindingMismatch => {
                formatter.write_str("stable scan does not match the trusted baseline binding")
            }
            BaselineUnavailableCause::StableScanBindingChanged => {
                formatter.write_str("stable scan binding changed before baseline finish")
            }
            BaselineUnavailableCause::InvalidStableScanEvidence(detail) => {
                write!(formatter, "invalid stable-scan evidence: {detail}")
            }
        }
    }
}

impl std::error::Error for BaselineUnavailable {}

impl From<ReconciliationBaselineError> for BaselineUnavailable {
    fn from(error: ReconciliationBaselineError) -> Self {
        Self {
            cause: BaselineUnavailableCause::Store(error),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineTerminalOutcome<'a> {
    Noop,
    Complete,
    Blocked(BaselineBlockedRegistration<'a>),
    FailedClosed,
    Retry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BaselineBlockedRegistration<'a> {
    pub(crate) observation_digest: ContentDigest,
    pub(crate) reason: BaselineBlockedReason,
    pub(crate) detail: &'a str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BaselineAdapterInstrumentation {
    pub(crate) path_rows: u64,
    pub(crate) directory_rows: u64,
    pub(crate) write_batches: u64,
    pub(crate) peak_added_retained_rows: u64,
    pub(crate) peak_added_retained_bytes: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingStableScanBaseline {
    epoch: BaselineEpochId,
    scan_identity: StableGraphTextBaselineIdentity,
    rows_identity: BaselineScanRowsIdentity,
    scan_instrumentation: BaselineScanInstrumentation,
    adapter_instrumentation: BaselineAdapterInstrumentation,
    commitment: ContentDigest,
}

impl PendingStableScanBaseline {
    pub(crate) const fn epoch(&self) -> BaselineEpochId {
        self.epoch
    }

    pub(crate) const fn instrumentation(&self) -> BaselineAdapterInstrumentation {
        self.adapter_instrumentation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineAdapterStatus {
    Clean {
        head: BaselineHead,
        instrumentation: BaselineAdapterInstrumentation,
    },
    NeedPostDrainFullScan {
        instrumentation: BaselineAdapterInstrumentation,
    },
    DiagnosticOnly {
        instrumentation: BaselineAdapterInstrumentation,
    },
}

/// Begin a bounded epoch and stream the scan-owned stable pass into SQLite.
///
/// Dropping the returned handle models a crash after append: the building
/// epoch remains diagnostic garbage and the prior clean head is unchanged.
pub(crate) fn append_stable_scan_to_baseline<S: AuthenticatedExpectedPathSource>(
    baseline: &mut ReconciliationBaseline,
    scan: &StableGraphTextScan,
    source: &S,
    started_at: BaselineTimestamp,
) -> Result<PendingStableScanBaseline, BaselineUnavailable> {
    let scan_identity = scan
        .validated_baseline_identity()
        .map_err(invalid_evidence)?;
    require_exact_baseline_binding(baseline, &scan_identity)?;
    require_current_scan_binding(&scan_identity, source, false)?;

    let evidence = scan.baseline_evidence();
    let epoch = baseline.begin_epoch(BeginBaselineEpoch {
        started_at,
        accepted_frontier: scan.binding.expected_binding.accepted_frontier,
        projection_generation: scan.binding.expected_binding.projection_generation,
    })?;
    let mut instrumentation = BaselineAdapterInstrumentation::default();
    let mut rows_identity = BaselineScanRowsIdentityBuilder::new();

    let mut directory_page = Vec::with_capacity(MAX_BASELINE_WRITE_ROWS);
    let mut directory_page_path_bytes = 0_usize;
    for (path, resource) in evidence.directories {
        let path = BaselineDirectoryPath::parse(path.clone())?;
        let resource = if path.as_str().is_empty() {
            ContentDigest::from_bytes(*scan_identity.graph_resource.as_bytes())
        } else {
            *resource
        };
        observe_page_row(
            &mut instrumentation,
            &directory_page,
            directory_page_path_bytes,
            path.as_str().len(),
            mem::size_of::<BaselineScanDirectory>(),
        );
        directory_page_path_bytes = directory_page_path_bytes.saturating_add(path.as_str().len());
        let row = BaselineScanDirectory { path, resource };
        rows_identity.observe_directory(&row);
        directory_page.push(row);
        if directory_page.len() == MAX_BASELINE_WRITE_ROWS {
            flush_directories(baseline, epoch, &mut directory_page, &mut instrumentation)?;
            directory_page_path_bytes = 0;
        }
    }
    flush_directories(baseline, epoch, &mut directory_page, &mut instrumentation)?;
    drop(directory_page);

    let mut path_page = Vec::with_capacity(MAX_BASELINE_WRITE_ROWS);
    let mut path_page_path_bytes = 0_usize;
    for file in evidence.files {
        let Some(row) = scan_file_row(file)? else {
            continue;
        };
        rows_identity.observe_path(&row);
        observe_page_row(
            &mut instrumentation,
            &path_page,
            path_page_path_bytes,
            row.path.as_str().len(),
            mem::size_of::<BaselineScanPath>(),
        );
        path_page_path_bytes = path_page_path_bytes.saturating_add(row.path.as_str().len());
        path_page.push(row);
        if path_page.len() == MAX_BASELINE_WRITE_ROWS {
            flush_paths(baseline, epoch, &mut path_page, &mut instrumentation)?;
            path_page_path_bytes = 0;
        }
    }
    for candidate in &scan.candidates {
        if candidate.change != GraphTextCandidateKind::Absence {
            continue;
        }
        let row = BaselineScanPath {
            path: candidate.path.clone(),
            managed_kind: candidate.managed_kind,
            state: BaselineObservedState::Absent,
        };
        rows_identity.observe_path(&row);
        observe_page_row(
            &mut instrumentation,
            &path_page,
            path_page_path_bytes,
            row.path.as_str().len(),
            mem::size_of::<BaselineScanPath>(),
        );
        path_page_path_bytes = path_page_path_bytes.saturating_add(row.path.as_str().len());
        path_page.push(row);
        if path_page.len() == MAX_BASELINE_WRITE_ROWS {
            flush_paths(baseline, epoch, &mut path_page, &mut instrumentation)?;
            path_page_path_bytes = 0;
        }
    }
    flush_paths(baseline, epoch, &mut path_page, &mut instrumentation)?;

    for diagnostic in &scan.diagnostics {
        baseline.record_blocked(
            scan_diagnostic_digest(diagnostic),
            BaselineBlockedReason::UnsafeFilesystem,
            &diagnostic.path,
            started_at,
        )?;
    }

    let mut pending = PendingStableScanBaseline {
        epoch,
        scan_identity,
        rows_identity: rows_identity.finish(),
        scan_instrumentation: baseline_scan_instrumentation(scan),
        adapter_instrumentation: instrumentation,
        commitment: ContentDigest::of(b"unsealed pending stable scan baseline"),
    };
    pending.commitment = pending_stable_scan_baseline_commitment(&pending);
    Ok(pending)
}

/// Finish one appended epoch. Candidate-bearing completion is deliberately
/// retained as incomplete diagnostics and requests a fresh post-drain scan.
pub(crate) fn finish_stable_scan_baseline<S: AuthenticatedExpectedPathSource>(
    baseline: &mut ReconciliationBaseline,
    source: &S,
    pending: PendingStableScanBaseline,
    terminal: BaselineTerminalOutcome<'_>,
    completed_at: BaselineTimestamp,
) -> Result<BaselineAdapterStatus, BaselineUnavailable> {
    if !pending.scan_identity.is_sealed()
        || pending.commitment != pending_stable_scan_baseline_commitment(&pending)
    {
        return Err(invalid_evidence(
            "pending stable-scan baseline identity seal does not match",
        ));
    }
    require_exact_baseline_binding(baseline, &pending.scan_identity)?;

    let candidate_count = pending.scan_identity.candidate_count;
    let diagnostic_count = pending.scan_identity.diagnostic_count;
    let can_promote =
        candidate_count == 0 && diagnostic_count == 0 && terminal == BaselineTerminalOutcome::Noop;
    // Diagnostic settlement is bound to the sealed scan and appended-row
    // identities, even when a successful import has intentionally advanced
    // expected authority. Clean promotion alone requires the scanned source
    // identity to remain current through finish.
    if can_promote {
        require_current_scan_binding(&pending.scan_identity, source, true)?;
    }
    let (outcome, complete_with_candidates) = match terminal {
        BaselineTerminalOutcome::Noop if can_promote => (BaselineEpochOutcome::Noop, false),
        BaselineTerminalOutcome::Noop => (BaselineEpochOutcome::Incomplete, false),
        BaselineTerminalOutcome::Complete => {
            (BaselineEpochOutcome::Incomplete, candidate_count != 0)
        }
        BaselineTerminalOutcome::Blocked(blocked) => {
            baseline.record_blocked(
                blocked.observation_digest,
                blocked.reason,
                blocked.detail,
                completed_at,
            )?;
            (BaselineEpochOutcome::Blocked, false)
        }
        BaselineTerminalOutcome::FailedClosed => (BaselineEpochOutcome::Blocked, false),
        BaselineTerminalOutcome::Retry => (BaselineEpochOutcome::Incomplete, false),
    };
    let finish = FinishBaselineEpoch {
        completed_at,
        pass_a_digest: pending.scan_identity.pass_a_digest,
        pass_b_digest: pending.scan_identity.pass_b_digest,
        candidate_digest: pending.scan_identity.candidate_digest,
        candidate_count,
        outcome,
        instrumentation: pending.scan_instrumentation,
    };
    if can_promote {
        let head = baseline
            .finish_sealed_scan_epoch(pending.epoch, finish, pending.rows_identity)?
            .ok_or_else(|| invalid_evidence("clean Noop did not install a baseline head"))?;
        return Ok(BaselineAdapterStatus::Clean {
            head,
            instrumentation: pending.adapter_instrumentation,
        });
    }

    baseline.finish_sealed_diagnostic_epoch(pending.epoch, finish, pending.rows_identity)?;
    if complete_with_candidates {
        Ok(BaselineAdapterStatus::NeedPostDrainFullScan {
            instrumentation: pending.adapter_instrumentation,
        })
    } else {
        Ok(BaselineAdapterStatus::DiagnosticOnly {
            instrumentation: pending.adapter_instrumentation,
        })
    }
}

fn require_exact_baseline_binding(
    baseline: &ReconciliationBaseline,
    identity: &StableGraphTextBaselineIdentity,
) -> Result<(), BaselineUnavailable> {
    if baseline.binding().graph_resource() != identity.graph_resource
        || baseline.binding().scope_binding() != identity.scope_binding
    {
        return Err(BaselineUnavailable {
            cause: BaselineUnavailableCause::StableScanBindingMismatch,
        });
    }
    Ok(())
}

fn require_current_scan_binding<S: AuthenticatedExpectedPathSource>(
    identity: &StableGraphTextBaselineIdentity,
    source: &S,
    finishing: bool,
) -> Result<(), BaselineUnavailable> {
    let (current, source_commitment) = source
        .current_scan_identity(StableGraphTextScan::baseline_revalidation_retained_bytes())
        .map_err(|error| BaselineUnavailable {
            cause: BaselineUnavailableCause::ExpectedAuthority(error),
        })?;
    if current != identity.expected_binding
        || source_commitment != identity.expected_source_commitment
    {
        return Err(BaselineUnavailable {
            cause: if finishing {
                BaselineUnavailableCause::StableScanBindingChanged
            } else {
                BaselineUnavailableCause::StableScanBindingMismatch
            },
        });
    }
    Ok(())
}

fn scan_file_row(
    file: &GraphTextScanFileFingerprint,
) -> Result<Option<BaselineScanPath>, BaselineUnavailable> {
    let Some(description) = file.description else {
        // Non-text and conflict-copy bytes are not hashed by the authenticated
        // scanner. Their exact metadata remains committed by both pass digests;
        // fabricating a content baseline here would create false authority.
        return Ok(None);
    };
    let managed_kind = match file.class {
        GraphTextScanPathClass::EligibleManaged(kind) => Some(kind),
        GraphTextScanPathClass::EligibleUnmanaged
        | GraphTextScanPathClass::ProviderConflictCopy
        | GraphTextScanPathClass::Configuration
        | GraphTextScanPathClass::RetainedNonText => None,
    };
    let path = ManagedPath::parse(file.exact_relative.clone())
        .map_err(|error| invalid_evidence(format!("invalid retained path: {error}")))?;
    Ok(Some(BaselineScanPath {
        path,
        managed_kind,
        state: BaselineObservedState::Present {
            description,
            file_resource: file.file_resource_id,
            link_count: file.link_count,
        },
    }))
}

fn flush_directories(
    baseline: &mut ReconciliationBaseline,
    epoch: BaselineEpochId,
    page: &mut Vec<BaselineScanDirectory>,
    instrumentation: &mut BaselineAdapterInstrumentation,
) -> Result<(), BaselineUnavailable> {
    if page.is_empty() {
        return Ok(());
    }
    baseline.append_scan_directories(epoch, page)?;
    instrumentation.directory_rows = instrumentation
        .directory_rows
        .saturating_add(page.len() as u64);
    instrumentation.write_batches = instrumentation.write_batches.saturating_add(1);
    page.clear();
    Ok(())
}

fn flush_paths(
    baseline: &mut ReconciliationBaseline,
    epoch: BaselineEpochId,
    page: &mut Vec<BaselineScanPath>,
    instrumentation: &mut BaselineAdapterInstrumentation,
) -> Result<(), BaselineUnavailable> {
    if page.is_empty() {
        return Ok(());
    }
    baseline.append_scan_paths(epoch, page)?;
    instrumentation.path_rows = instrumentation.path_rows.saturating_add(page.len() as u64);
    instrumentation.write_batches = instrumentation.write_batches.saturating_add(1);
    page.clear();
    Ok(())
}

fn observe_page_row<T>(
    instrumentation: &mut BaselineAdapterInstrumentation,
    page: &Vec<T>,
    retained_path_bytes: usize,
    added_path_bytes: usize,
    row_bytes: usize,
) {
    let rows = page.len().saturating_add(1) as u64;
    let retained = (page.capacity() as u64)
        .saturating_mul(row_bytes as u64)
        .saturating_add(retained_path_bytes as u64)
        .saturating_add(added_path_bytes as u64);
    instrumentation.peak_added_retained_rows = instrumentation.peak_added_retained_rows.max(rows);
    instrumentation.peak_added_retained_bytes =
        instrumentation.peak_added_retained_bytes.max(retained);
}

fn pending_stable_scan_baseline_commitment(pending: &PendingStableScanBaseline) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/pending-stable-scan-baseline/v1\0");
    hasher.update(pending.epoch.as_i64().to_be_bytes());
    hasher.update(pending.scan_identity.commitment().as_bytes());
    hasher.update(pending.rows_identity.commitment().as_bytes());
    for metric in [
        pending.scan_instrumentation.passes,
        pending.scan_instrumentation.directory_entries,
        pending.scan_instrumentation.directories,
        pending.scan_instrumentation.regular_files,
        pending.scan_instrumentation.eligible_files,
        pending.scan_instrumentation.bytes_read,
        pending.scan_instrumentation.bytes_hashed,
        pending.scan_instrumentation.peak_retained_rows,
        pending.scan_instrumentation.peak_retained_bytes,
        pending.scan_instrumentation.candidates,
        pending.scan_instrumentation.diagnostics,
        pending.scan_instrumentation.wall_time_millis,
        pending.adapter_instrumentation.path_rows,
        pending.adapter_instrumentation.directory_rows,
        pending.adapter_instrumentation.write_batches,
        pending.adapter_instrumentation.peak_added_retained_rows,
        pending.adapter_instrumentation.peak_added_retained_bytes,
    ] {
        hasher.update(metric.to_be_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn baseline_scan_instrumentation(scan: &StableGraphTextScan) -> BaselineScanInstrumentation {
    BaselineScanInstrumentation {
        passes: scan.instrumentation.passes,
        directory_entries: scan.instrumentation.directory_entries,
        directories: scan.instrumentation.directories,
        regular_files: scan.instrumentation.regular_files,
        eligible_files: scan.instrumentation.eligible_files,
        bytes_read: scan.instrumentation.bytes_read,
        bytes_hashed: scan.instrumentation.bytes_hashed,
        peak_retained_rows: scan.instrumentation.peak_retained_rows,
        peak_retained_bytes: scan.instrumentation.peak_retained_bytes,
        candidates: scan.candidates.len() as u64,
        diagnostics: scan.diagnostics.len() as u64,
        wall_time_millis: u64::try_from(scan.wall_time.as_millis()).unwrap_or(u64::MAX),
    }
}

fn scan_diagnostic_digest(diagnostic: &GraphTextScanDiagnostic) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/reconciliation/baseline-blocked-scan-diagnostic/v1\0");
    hasher.update([match diagnostic.kind {
        super::reconciliation_scan::GraphTextScanDiagnosticKind::ProviderConflictCopy => 1,
    }]);
    hasher.update((diagnostic.path.len() as u64).to_be_bytes());
    hasher.update(diagnostic.path.as_bytes());
    hasher.update(diagnostic.file_resource_id.as_bytes());
    hasher.update(diagnostic.link_count.to_be_bytes());
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn invalid_evidence(detail: impl Into<String>) -> BaselineUnavailable {
    BaselineUnavailable {
        cause: BaselineUnavailableCause::InvalidStableScanEvidence(detail.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_text_scope::GraphTextScope;
    use crate::oplog::{
        reconciliation_baseline::{
            ReconciliationBaselineBinding, TrustedPrivateApplicationRuntimeRoot,
        },
        reconciliation_scan::{
            graph_text_scan_pass_digest, scan_epoch_digest_from_commitments,
            AuthenticatedExpectedPath, AuthenticatedExpectedPathPage,
            AuthenticatedExpectedPathStreamHeader, ExpectedPathBinding, ExpectedPathPageRequest,
            ExpectedPathPointRequest, ExpectedPathStreamLimits, GraphTextCandidateBinding,
            GraphTextScanCandidate, GraphTextScanDiagnosticKind, GraphTextScanInstrumentation,
            GraphTextScanPass, GraphTextScanPassInstrumentation,
        },
        ApplicationRuntimeRoot, BlobDescription, CanonicalGraphResourceId, ManagedTextKind,
        ProjectionEndpointId, WorkspaceId,
    };
    use rusqlite::Connection;
    use std::{
        cell::Cell,
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tine-reconciliation-baseline-adapter-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        _directory: TestDir,
        binding: ReconciliationBaselineBinding,
        baseline: ReconciliationBaseline,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let directory = TestDir::new(label);
            let graph_resource = CanonicalGraphResourceId::from_capability_identity(
                b"adapter-test",
                label.as_bytes(),
            );
            let scope_binding = GraphTextScope::new(&[], false).bind_graph_resource(graph_resource);
            let binding = ReconciliationBaselineBinding::new(
                WorkspaceId::new(),
                ProjectionEndpointId::new(),
                graph_resource,
                scope_binding,
            )
            .unwrap();
            let runtime = ApplicationRuntimeRoot::open_for_test(directory.path()).unwrap();
            let trusted =
                TrustedPrivateApplicationRuntimeRoot::from_application_runtime_root(&runtime);
            let baseline = ReconciliationBaseline::create_fresh(&trusted, binding.clone()).unwrap();
            Self {
                _directory: directory,
                binding,
                baseline,
            }
        }
    }

    struct CurrentBinding {
        binding: Cell<ExpectedPathBinding>,
        source_commitment: Cell<ContentDigest>,
    }

    impl CurrentBinding {
        fn new(binding: &GraphTextCandidateBinding) -> Self {
            Self {
                binding: Cell::new(binding.expected_binding),
                source_commitment: Cell::new(binding.expected_source_commitment),
            }
        }

        fn set(&self, binding: ExpectedPathBinding) {
            self.binding.set(binding);
        }

        fn set_source_commitment(&self, source_commitment: ContentDigest) {
            self.source_commitment.set(source_commitment);
        }
    }

    impl AuthenticatedExpectedPathSource for CurrentBinding {
        type Cursor = ();

        fn open_expected_paths(
            &self,
            _limits: ExpectedPathStreamLimits,
        ) -> Result<(AuthenticatedExpectedPathStreamHeader, Self::Cursor), ExpectedPathSourceFailure>
        {
            Err(ExpectedPathSourceFailure::Unavailable)
        }

        fn read_expected_path_page(
            &self,
            _cursor: &mut Self::Cursor,
            _request: ExpectedPathPageRequest,
        ) -> Result<AuthenticatedExpectedPathPage, ExpectedPathSourceFailure> {
            Err(ExpectedPathSourceFailure::Unavailable)
        }

        fn expected_path_at(
            &self,
            _path: &ManagedPath,
            _request: ExpectedPathPointRequest,
        ) -> Result<Option<AuthenticatedExpectedPath>, ExpectedPathSourceFailure> {
            Err(ExpectedPathSourceFailure::Unavailable)
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
            Ok((self.binding.get(), self.source_commitment.get()))
        }
    }

    fn expected_binding(generation: u64) -> ExpectedPathBinding {
        ExpectedPathBinding {
            accepted_frontier: ContentDigest::of(format!("frontier-{generation}").as_bytes()),
            projection_generation: generation,
        }
    }

    fn scan(
        binding: &ReconciliationBaselineBinding,
        generation: u64,
        paths: impl IntoIterator<Item = (String, GraphTextScanPathClass)>,
        directories: impl IntoIterator<Item = String>,
        candidate: bool,
        diagnostic: bool,
    ) -> StableGraphTextScan {
        let expected_binding = expected_binding(generation);
        let mut directories_by_exact_relative = BTreeMap::new();
        directories_by_exact_relative.insert(
            String::new(),
            ContentDigest::from_bytes(*binding.graph_resource().as_bytes()),
        );
        for path in directories {
            directories_by_exact_relative.insert(
                path.clone(),
                ContentDigest::of(format!("directory:{path}").as_bytes()),
            );
        }
        let files = paths
            .into_iter()
            .map(|(exact_relative, class)| {
                let description = (!matches!(
                    class,
                    GraphTextScanPathClass::ProviderConflictCopy
                        | GraphTextScanPathClass::RetainedNonText
                ))
                .then(|| BlobDescription::of(format!("contents:{exact_relative}").as_bytes()));
                GraphTextScanFileFingerprint {
                    description,
                    file_resource_id: ContentDigest::of(
                        format!("resource:{exact_relative}").as_bytes(),
                    ),
                    exact_relative,
                    class,
                    portable_key: None,
                    link_count: 1,
                }
            })
            .collect::<Vec<_>>();
        let baseline_pass = GraphTextScanPass {
            graph_resource: binding.graph_resource(),
            scope_binding: binding.scope_binding(),
            instrumentation: GraphTextScanPassInstrumentation {
                directories: directories_by_exact_relative.len() as u64,
                regular_files: files.len() as u64,
                eligible_files: files.len() as u64,
                retained_rows: (directories_by_exact_relative.len() + files.len()) as u64,
                ..GraphTextScanPassInstrumentation::default()
            },
            directories_by_exact_relative,
            files,
        };
        let pass_digest = graph_text_scan_pass_digest(&baseline_pass);
        let expected_source_commitment = ContentDigest::of(b"source");
        let expected_rows_commitment = ContentDigest::of(b"expected-rows");
        let candidate_binding = GraphTextCandidateBinding {
            graph_resource: binding.graph_resource(),
            scope_binding: binding.scope_binding(),
            expected_binding,
            expected_source_commitment,
            expected_rows_commitment,
            scan_epoch_digest: scan_epoch_digest_from_commitments(
                &baseline_pass,
                expected_binding,
                expected_source_commitment,
                expected_rows_commitment,
            ),
        };
        let candidates = candidate
            .then(|| {
                let file = &baseline_pass.files[0];
                GraphTextScanCandidate {
                    path: ManagedPath::parse(file.exact_relative.clone()).unwrap(),
                    managed_kind: Some(ManagedTextKind::Page),
                    change: GraphTextCandidateKind::Edit,
                    expected_description: Some(BlobDescription::of(b"old")),
                    expected_owner_binding: Some(ContentDigest::of(b"owner")),
                    observed_description: file.description,
                    observed_file_resource_id: Some(file.file_resource_id),
                    observed_link_count: Some(file.link_count),
                    binding: candidate_binding.clone(),
                }
            })
            .into_iter()
            .collect::<Vec<_>>();
        let diagnostics = diagnostic
            .then(|| {
                let file = &baseline_pass.files[0];
                GraphTextScanDiagnostic {
                    path: file.exact_relative.clone(),
                    kind: GraphTextScanDiagnosticKind::ProviderConflictCopy,
                    file_resource_id: file.file_resource_id,
                    link_count: file.link_count,
                }
            })
            .into_iter()
            .collect::<Vec<_>>();
        StableGraphTextScan {
            instrumentation: GraphTextScanInstrumentation {
                passes: 2,
                directories: (baseline_pass.directories_by_exact_relative.len() * 2) as u64,
                regular_files: (baseline_pass.files.len() * 2) as u64,
                eligible_files: (baseline_pass.files.len() * 2) as u64,
                candidates: candidates.len() as u64,
                diagnostics: diagnostics.len() as u64,
                ..GraphTextScanInstrumentation::default()
            },
            candidates,
            diagnostics,
            binding: candidate_binding,
            wall_time: Duration::from_millis(2),
            baseline_pass,
            pass_a_digest: pass_digest,
            pass_b_digest: pass_digest,
        }
    }

    fn one_path_scan(fixture: &Fixture, generation: u64, candidate: bool) -> StableGraphTextScan {
        scan(
            &fixture.binding,
            generation,
            [(
                "custom/nested/page.MarkDown".to_owned(),
                GraphTextScanPathClass::EligibleManaged(ManagedTextKind::Page),
            )],
            ["custom".to_owned(), "custom/nested".to_owned()],
            candidate,
            false,
        )
    }

    fn append(
        fixture: &mut Fixture,
        scan: &StableGraphTextScan,
        timestamp: u64,
    ) -> (CurrentBinding, PendingStableScanBaseline) {
        let source = CurrentBinding::new(&scan.binding);
        let pending = append_stable_scan_to_baseline(
            &mut fixture.baseline,
            scan,
            &source,
            BaselineTimestamp::from_millis(timestamp).unwrap(),
        )
        .unwrap();
        (source, pending)
    }

    #[test]
    fn pending_baseline_is_a_constant_sized_identity_not_a_retained_scan() {
        assert!(!std::mem::needs_drop::<PendingStableScanBaseline>());
        assert!(std::mem::size_of::<PendingStableScanBaseline>() <= 4096);
        assert!(std::mem::needs_drop::<StableGraphTextScan>());
    }

    fn install_clean(fixture: &mut Fixture, generation: u64) -> BaselineHead {
        let scan = one_path_scan(fixture, generation, false);
        let (source, pending) = append(fixture, &scan, generation * 10);
        match finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(generation * 10 + 1).unwrap(),
        )
        .unwrap()
        {
            BaselineAdapterStatus::Clean { head, .. } => head,
            status => panic!("expected clean head, got {status:?}"),
        }
    }

    #[test]
    fn zero_candidate_stable_noop_atomically_promotes_clean_head() {
        let mut fixture = Fixture::new("zero-clean");
        let scan = one_path_scan(&fixture, 1, false);
        let (source, pending) = append(&mut fixture, &scan, 10);
        let status = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(11).unwrap(),
        )
        .unwrap();
        let BaselineAdapterStatus::Clean {
            head,
            instrumentation,
        } = status
        else {
            panic!("zero-candidate Noop did not become clean");
        };
        assert_eq!(fixture.baseline.head().unwrap(), head);
        assert_eq!(instrumentation.path_rows, 1);
        assert_eq!(instrumentation.directory_rows, 3);
    }

    #[test]
    fn candidate_complete_requires_post_drain_full_scan_and_has_no_head() {
        let mut fixture = Fixture::new("candidate-complete");
        let scan = one_path_scan(&fixture, 1, true);
        let (source, pending) = append(&mut fixture, &scan, 10);
        source.set(expected_binding(2));
        source.set_source_commitment(ContentDigest::of(b"post-import-source"));
        assert!(matches!(
            finish_stable_scan_baseline(
                &mut fixture.baseline,
                &source,
                pending,
                BaselineTerminalOutcome::Complete,
                BaselineTimestamp::from_millis(11).unwrap(),
            )
            .unwrap(),
            BaselineAdapterStatus::NeedPostDrainFullScan { .. }
        ));
        assert!(fixture.baseline.head().is_err());
    }

    #[test]
    fn blocked_failed_closed_and_retry_never_replace_clean_head() {
        let mut fixture = Fixture::new("negative-outcomes");
        let clean = install_clean(&mut fixture, 1);
        for (generation, terminal) in [
            (
                2,
                BaselineTerminalOutcome::Blocked(BaselineBlockedRegistration {
                    observation_digest: ContentDigest::of(b"blocked-negative-outcome"),
                    reason: BaselineBlockedReason::ReconciliationFailed,
                    detail: "blocked negative outcome",
                }),
            ),
            (3, BaselineTerminalOutcome::FailedClosed),
            (4, BaselineTerminalOutcome::Retry),
        ] {
            let scan = one_path_scan(&fixture, generation, true);
            let (source, pending) = append(&mut fixture, &scan, generation * 10);
            assert!(matches!(
                finish_stable_scan_baseline(
                    &mut fixture.baseline,
                    &source,
                    pending,
                    terminal,
                    BaselineTimestamp::from_millis(generation * 10 + 1).unwrap(),
                )
                .unwrap(),
                BaselineAdapterStatus::DiagnosticOnly { .. }
            ));
            assert_eq!(fixture.baseline.head().unwrap(), clean);
        }
    }

    #[test]
    fn crash_after_append_before_finish_leaves_old_head() {
        let mut fixture = Fixture::new("crash-before-finish");
        let clean = install_clean(&mut fixture, 1);
        let scan = one_path_scan(&fixture, 2, false);
        let (_source, pending) = append(&mut fixture, &scan, 20);
        assert!(pending.epoch().as_i64() > clean.epoch.as_i64());
        let _ = pending;
        assert_eq!(fixture.baseline.head().unwrap(), clean);
    }

    #[test]
    fn pending_handle_finishes_exact_appended_scan_without_replaceable_scan_parameter() {
        let mut fixture = Fixture::new("scan-swap");
        let clean = install_clean(&mut fixture, 1);
        let scan_a = scan(
            &fixture.binding,
            2,
            [(
                "pages/scan-a.md".to_owned(),
                GraphTextScanPathClass::EligibleManaged(ManagedTextKind::Page),
            )],
            ["pages".to_owned()],
            false,
            false,
        );
        let (source, pending) = append(&mut fixture, &scan_a, 20);
        let status = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(21).unwrap(),
        )
        .unwrap();
        let BaselineAdapterStatus::Clean { head, .. } = status else {
            panic!("sealed pending scan did not promote");
        };
        assert_ne!(head, clean);
        let page = fixture.baseline.read_head_paths_page(None, 8).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].path.as_str(), "pages/scan-a.md");
    }

    #[test]
    fn noop_terminal_outcome_has_no_blocked_registration() {
        let mut fixture = Fixture::new("noop-blocked");
        install_clean(&mut fixture, 1);
        let scan = one_path_scan(&fixture, 2, false);
        let (source, pending) = append(&mut fixture, &scan, 20);
        let impossible_blocked_digest = ContentDigest::of(b"contradictory-blocked-observation");
        assert!(matches!(
            finish_stable_scan_baseline(
                &mut fixture.baseline,
                &source,
                pending,
                BaselineTerminalOutcome::Noop,
                BaselineTimestamp::from_millis(21).unwrap(),
            )
            .unwrap(),
            BaselineAdapterStatus::Clean { .. }
        ));
        assert!(fixture
            .baseline
            .blocked_signature(impossible_blocked_digest)
            .unwrap()
            .is_none());
    }

    #[test]
    fn tampered_pending_row_commitment_fails_closed_and_preserves_old_head() {
        let mut fixture = Fixture::new("tampered-pending-rows");
        let clean = install_clean(&mut fixture, 1);
        let scan = one_path_scan(&fixture, 2, false);
        let (source, mut pending) = append(&mut fixture, &scan, 20);
        pending.rows_identity = BaselineScanRowsIdentityBuilder::new().finish();
        pending.commitment = pending_stable_scan_baseline_commitment(&pending);
        let error = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(21).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error.cause,
            BaselineUnavailableCause::Store(
                ReconciliationBaselineError::BaselineUnavailable { .. }
            )
        ));
        assert_eq!(fixture.baseline.head().unwrap(), clean);
    }

    #[test]
    fn candidate_and_diagnostic_commitment_swap_fails_closed_and_preserves_old_head() {
        let mut fixture = Fixture::new("candidate-diagnostic-swap");
        let clean = install_clean(&mut fixture, 1);
        let scan = scan(
            &fixture.binding,
            2,
            [(
                "pages/provider-conflict.md".to_owned(),
                GraphTextScanPathClass::ProviderConflictCopy,
            )],
            ["pages".to_owned()],
            true,
            true,
        );
        let (source, mut pending) = append(&mut fixture, &scan, 20);
        let candidate_digest = pending.scan_identity.candidate_digest;
        pending.scan_identity.candidate_digest = pending.scan_identity.diagnostic_digest;
        pending.scan_identity.diagnostic_digest = candidate_digest;
        let error = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(21).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error.cause,
            BaselineUnavailableCause::InvalidStableScanEvidence(_)
        ));
        assert_eq!(fixture.baseline.head().unwrap(), clean);
    }

    #[test]
    fn tampered_pass_commitment_is_rejected_before_append() {
        let mut fixture = Fixture::new("tampered-pass-commitment");
        let mut scan = one_path_scan(&fixture, 1, false);
        scan.pass_b_digest = ContentDigest::of(b"tampered-pass-b");
        let source = CurrentBinding::new(&scan.binding);
        append_stable_scan_to_baseline(
            &mut fixture.baseline,
            &scan,
            &source,
            BaselineTimestamp::from_millis(10).unwrap(),
        )
        .unwrap_err();
        assert!(fixture.baseline.head().is_err());
    }

    #[test]
    fn tampered_expected_rows_commitment_is_rejected_before_append() {
        let mut fixture = Fixture::new("tampered-expected-rows");
        let mut scan = one_path_scan(&fixture, 1, false);
        scan.binding.expected_rows_commitment = ContentDigest::of(b"tampered-expected-rows");
        let source = CurrentBinding::new(&scan.binding);
        append_stable_scan_to_baseline(
            &mut fixture.baseline,
            &scan,
            &source,
            BaselineTimestamp::from_millis(10).unwrap(),
        )
        .unwrap_err();
        assert!(fixture.baseline.head().is_err());
    }

    #[test]
    fn retained_evidence_row_digest_mismatch_is_rejected_before_append() {
        let mut fixture = Fixture::new("tampered-pass-row");
        let mut scan = one_path_scan(&fixture, 1, false);
        scan.baseline_pass.files[0].file_resource_id = ContentDigest::of(b"tampered-file-resource");
        let source = CurrentBinding::new(&scan.binding);
        append_stable_scan_to_baseline(
            &mut fixture.baseline,
            &scan,
            &source,
            BaselineTimestamp::from_millis(10).unwrap(),
        )
        .unwrap_err();
        assert!(fixture.baseline.head().is_err());
    }

    #[test]
    fn corruption_between_append_and_finish_is_typed_unavailable_and_preserved() {
        let mut fixture = Fixture::new("finish-corruption");
        let scan = one_path_scan(&fixture, 1, false);
        let (source, pending) = append(&mut fixture, &scan, 10);
        let database_path = fixture.baseline.path().to_path_buf();
        let original_length = fs::metadata(&database_path).unwrap().len();
        let external = Connection::open(&database_path).unwrap();
        external
            .execute("UPDATE binding SET endpoint = zeroblob(16)", [])
            .unwrap();
        drop(external);
        let error = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(11).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error.cause,
            BaselineUnavailableCause::Store(ReconciliationBaselineError::RebuildRequired { .. })
        ));
        assert!(database_path.exists());
        assert!(fs::metadata(database_path).unwrap().len() >= original_length);
    }

    #[test]
    fn nested_unicode_and_nonstandard_paths_round_trip_exactly() {
        let mut fixture = Fixture::new("unicode-nonstandard");
        let exact = "资料/任意/页面-é.MARKDOWN";
        let scan = scan(
            &fixture.binding,
            1,
            [(exact.to_owned(), GraphTextScanPathClass::EligibleUnmanaged)],
            ["资料".to_owned(), "资料/任意".to_owned()],
            false,
            false,
        );
        let (source, pending) = append(&mut fixture, &scan, 10);
        finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(11).unwrap(),
        )
        .unwrap();
        let page = fixture.baseline.read_head_paths_page(None, 8).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].path.as_str(), exact);
        assert_eq!(page.rows[0].managed_kind, None);
    }

    #[test]
    fn stale_expected_binding_before_finish_leaves_epoch_non_authoritative() {
        let mut fixture = Fixture::new("stale-finish");
        let clean = install_clean(&mut fixture, 1);
        let scan = one_path_scan(&fixture, 2, false);
        let (source, pending) = append(&mut fixture, &scan, 20);
        source.set(expected_binding(3));
        let error = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(21).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error.cause,
            BaselineUnavailableCause::StableScanBindingChanged
        ));
        assert_eq!(fixture.baseline.head().unwrap(), clean);
    }

    #[test]
    fn stale_expected_source_commitment_before_finish_preserves_old_head() {
        let mut fixture = Fixture::new("stale-source-commitment");
        let clean = install_clean(&mut fixture, 1);
        let scan = one_path_scan(&fixture, 2, false);
        let (source, pending) = append(&mut fixture, &scan, 20);
        source.set_source_commitment(ContentDigest::of(b"moved-source-commitment"));
        let error = finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(21).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error.cause,
            BaselineUnavailableCause::StableScanBindingChanged
        ));
        assert_eq!(fixture.baseline.head().unwrap(), clean);
    }

    #[test]
    fn blocked_signatures_deduplicate_independently_of_clean_head() {
        let mut fixture = Fixture::new("blocked-independent");
        let clean = install_clean(&mut fixture, 1);
        let digest = ContentDigest::of(b"same-blocked-observation");
        for (generation, completed_at) in [(2, 21), (3, 31)] {
            let scan = one_path_scan(&fixture, generation, true);
            let (source, pending) = append(&mut fixture, &scan, generation * 10);
            finish_stable_scan_baseline(
                &mut fixture.baseline,
                &source,
                pending,
                BaselineTerminalOutcome::Blocked(BaselineBlockedRegistration {
                    observation_digest: digest,
                    reason: BaselineBlockedReason::ReconciliationFailed,
                    detail: "same failed reconciliation",
                }),
                BaselineTimestamp::from_millis(completed_at).unwrap(),
            )
            .unwrap();
            assert_eq!(fixture.baseline.head().unwrap(), clean);
        }
        let signature = fixture.baseline.blocked_signature(digest).unwrap().unwrap();
        assert_eq!(signature.first_seen.as_millis(), 21);
        assert_eq!(signature.last_seen.as_millis(), 31);
    }

    #[test]
    fn blocking_scan_diagnostic_is_recorded_without_promoting_noop() {
        let mut fixture = Fixture::new("scan-diagnostic");
        let clean = install_clean(&mut fixture, 1);
        let scan = scan(
            &fixture.binding,
            2,
            [(
                "custom/nested/provider-conflict.MarkDown".to_owned(),
                GraphTextScanPathClass::ProviderConflictCopy,
            )],
            ["custom".to_owned(), "custom/nested".to_owned()],
            false,
            true,
        );
        let digest = scan_diagnostic_digest(&scan.diagnostics[0]);
        let (source, pending) = append(&mut fixture, &scan, 20);
        assert!(matches!(
            finish_stable_scan_baseline(
                &mut fixture.baseline,
                &source,
                pending,
                BaselineTerminalOutcome::Noop,
                BaselineTimestamp::from_millis(21).unwrap(),
            )
            .unwrap(),
            BaselineAdapterStatus::DiagnosticOnly { .. }
        ));
        assert_eq!(fixture.baseline.head().unwrap(), clean);
        assert!(fixture
            .baseline
            .blocked_signature(digest)
            .unwrap()
            .is_some());
    }

    #[test]
    fn ten_thousand_rows_are_paged_with_bounded_added_heap_and_sqlite_receipt() {
        let mut fixture = Fixture::new("ten-thousand");
        let paths = (0..10_000)
            .map(|index| {
                (
                    format!("arbitrary/nested/{index:05}.markdown"),
                    GraphTextScanPathClass::EligibleUnmanaged,
                )
            })
            .collect::<Vec<_>>();
        let scan = scan(
            &fixture.binding,
            1,
            paths,
            ["arbitrary".to_owned(), "arbitrary/nested".to_owned()],
            false,
            false,
        );
        let source = CurrentBinding::new(&scan.binding);
        let started = Instant::now();
        let pending = append_stable_scan_to_baseline(
            &mut fixture.baseline,
            &scan,
            &source,
            BaselineTimestamp::from_millis(10).unwrap(),
        )
        .unwrap();
        let instrumentation = pending.instrumentation();
        finish_stable_scan_baseline(
            &mut fixture.baseline,
            &source,
            pending,
            BaselineTerminalOutcome::Noop,
            BaselineTimestamp::from_millis(11).unwrap(),
        )
        .unwrap();
        let elapsed = started.elapsed();
        let database_path = fixture.baseline.path();
        let sqlite_bytes = [
            database_path.to_path_buf(),
            database_path.with_file_name("scan.sqlite-journal"),
            database_path.with_file_name("scan.sqlite-wal"),
            database_path.with_file_name("scan.sqlite-shm"),
        ]
        .into_iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();
        assert_eq!(instrumentation.path_rows, 10_000);
        assert!(instrumentation.write_batches >= 20);
        assert!(instrumentation.peak_added_retained_rows <= MAX_BASELINE_WRITE_ROWS as u64);
        assert!(instrumentation.peak_added_retained_bytes < 1024 * 1024);
        assert!(sqlite_bytes > 100_000);
        assert!(sqlite_bytes < 16 * 1024 * 1024);

        let mut after = None;
        let mut read = 0_usize;
        loop {
            let page = fixture
                .baseline
                .read_head_paths_page(after.as_ref(), 257)
                .unwrap();
            read += page.rows.len();
            let Some(next) = page.next_after else {
                break;
            };
            after = Some(next);
        }
        assert_eq!(read, 10_000);
        eprintln!(
            "ADAPTER_10K_RECEIPT rows=10000 sqlite_bytes={sqlite_bytes} elapsed_ms={} \
             peak_added_rows={} peak_added_bytes={} write_batches={}",
            elapsed.as_millis(),
            instrumentation.peak_added_retained_rows,
            instrumentation.peak_added_retained_bytes,
            instrumentation.write_batches,
        );
    }
}
