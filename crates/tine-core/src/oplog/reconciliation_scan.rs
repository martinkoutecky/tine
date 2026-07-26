//! Test-only scan discovery substrate.
//!
//! This module deliberately has no production caller and exposes no write,
//! import, projection, watcher, or continuing filesystem authority. A stable
//! epoch is only a bounded candidate set for a later point-revalidated packet.

use super::{
    BlobDescription, CanonicalGraphResourceId, ContentDigest, ManagedPath, ManagedTextKind,
    PortablePathKey,
};
use crate::graph_text_scope::GraphTextScopeBinding;
use crate::model::Graph;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::time::{Duration, Instant};

pub(crate) const GRAPH_TEXT_SCAN_READ_BUFFER_BYTES: usize = 64 * 1024;
const MAX_SCAN_RETAINED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCAN_EXPECTED_PATHS: usize = 1_000_000;
const MAX_SCAN_EXACT_PATH_BYTES: usize = 4096;

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

    fn managed_kind(self) -> Option<ManagedTextKind> {
        match self {
            Self::EligibleManaged(kind) => Some(kind),
            Self::EligibleUnmanaged
            | Self::ProviderConflictCopy
            | Self::Configuration
            | Self::RetainedNonText => None,
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
    pub(crate) path: ManagedPath,
    pub(crate) kind: ManagedTextKind,
    pub(crate) description: BlobDescription,
    pub(crate) owner_binding: ContentDigest,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedExpectedPathSnapshot {
    pub(crate) binding: ExpectedPathBinding,
    pub(crate) rows: Vec<AuthenticatedExpectedPath>,
    pub(crate) rows_commitment: ContentDigest,
}

impl AuthenticatedExpectedPathSnapshot {
    pub(crate) fn new(
        binding: ExpectedPathBinding,
        mut rows: Vec<AuthenticatedExpectedPath>,
    ) -> Self {
        rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        let rows_commitment = expected_rows_commitment(binding, &rows);
        Self {
            binding,
            rows,
            rows_commitment,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpectedPathSourceFailure {
    Missing,
    Corrupt,
    Ambiguous,
    Unavailable,
}

impl fmt::Display for ExpectedPathSourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "authenticated expected-path authority is missing",
            Self::Corrupt => "authenticated expected-path authority is corrupt",
            Self::Ambiguous => "authenticated expected-path authority is ambiguous",
            Self::Unavailable => "authenticated expected-path authority is unavailable",
        })
    }
}

/// Test fixture boundary for the later authenticated paged engine cursor.
///
/// Implementations must return a complete current live-path set rooted at the
/// supplied binding. The scan validates the compact row commitment, exact and
/// portable uniqueness, count bounds, and the binding before/after both passes.
pub(crate) trait AuthenticatedExpectedPathSource {
    fn capture_expected_paths(
        &self,
        maximum_rows: usize,
    ) -> Result<AuthenticatedExpectedPathSnapshot, ExpectedPathSourceFailure>;

    fn current_binding(&self) -> Result<ExpectedPathBinding, ExpectedPathSourceFailure>;
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
    pub(crate) expected_rows_commitment: ContentDigest,
    pub(crate) scan_epoch_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphTextScanCandidate {
    pub(crate) path: ManagedPath,
    pub(crate) managed_kind: ManagedTextKind,
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
    EligibleOutsideCreationRoots,
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
    pub(crate) candidates: u64,
    pub(crate) parser_invocations: u64,
}

impl GraphTextScanInstrumentation {
    fn add_pass(&mut self, pass: GraphTextScanPassInstrumentation) {
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
            .max(pass.peak_retained_rows.saturating_mul(2));
        self.peak_retained_bytes = self
            .peak_retained_bytes
            .max(pass.peak_retained_bytes.saturating_mul(2));
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
    scan_graph_text_with_hook(graph, source, limits, || Ok(()))
}

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
    let started = Instant::now();
    let mut instrumentation = GraphTextScanInstrumentation::default();
    let expected = source
        .capture_expected_paths(limits.expected_paths)
        .map_err(|failure| {
            expected_source_failure(started, instrumentation, failure, failure.to_string())
        })?;
    validate_expected_snapshot(&expected, limits.expected_paths).map_err(|failure| {
        expected_source_failure(started, instrumentation, failure, failure.to_string())
    })?;
    instrumentation.expected_rows = expected.rows.len() as u64;
    require_expected_binding(source, expected.binding, started, instrumentation)?;

    let first = graph
        .capture_reconciliation_scan_pass(limits)
        .map_err(|error| scan_io_failure(started, instrumentation, error))?;
    instrumentation.add_pass(first.instrumentation);
    require_expected_binding(source, expected.binding, started, instrumentation)?;

    between_passes().map_err(|error| scan_io_failure(started, instrumentation, error))?;

    let second = graph
        .capture_reconciliation_scan_pass(limits)
        .map_err(|error| scan_io_failure(started, instrumentation, error))?;
    instrumentation.add_pass(second.instrumentation);
    require_expected_binding(source, expected.binding, started, instrumentation)?;
    let combined_retained_rows = first
        .instrumentation
        .peak_retained_rows
        .checked_add(second.instrumentation.peak_retained_rows);
    let combined_retained_bytes = first
        .instrumentation
        .peak_retained_bytes
        .checked_add(second.instrumentation.peak_retained_bytes);
    if combined_retained_rows.is_none_or(|rows| rows > limits.retained_rows as u64)
        || combined_retained_bytes.is_none_or(|bytes| bytes > limits.retained_bytes)
    {
        return Err(scan_io_failure(
            started,
            instrumentation,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "reconciliation scan simultaneous two-pass scratch bound exceeded",
            ),
        ));
    }

    if !first.evidence_eq(&second) {
        return Err(GraphTextScanFailure {
            class: GraphTextScanFailureClass::UnstableEpoch,
            reason: GraphTextScanFailureReason::FilesystemEvidenceChanged,
            detail: "the two complete graph-text fingerprint passes differed".to_owned(),
            instrumentation,
            wall_time: started.elapsed(),
        });
    }

    let scan_epoch_digest = scan_epoch_digest(&second, &expected);
    let binding = GraphTextCandidateBinding {
        graph_resource: second.graph_resource,
        scope_binding: second.scope_binding.clone(),
        expected_binding: expected.binding,
        expected_rows_commitment: expected.rows_commitment,
        scan_epoch_digest,
    };
    let (candidates, diagnostics) = derive_candidates(&second, &expected, &binding);
    instrumentation.candidates = candidates.len() as u64;
    Ok(StableGraphTextScan {
        candidates,
        diagnostics,
        binding,
        instrumentation,
        wall_time: started.elapsed(),
    })
}

fn require_expected_binding<S: AuthenticatedExpectedPathSource>(
    source: &S,
    expected: ExpectedPathBinding,
    started: Instant,
    instrumentation: GraphTextScanInstrumentation,
) -> Result<(), GraphTextScanFailure> {
    let current = source.current_binding().map_err(|failure| {
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
    GraphTextScanFailure {
        class: if unstable {
            GraphTextScanFailureClass::UnstableEpoch
        } else {
            GraphTextScanFailureClass::Blocked
        },
        reason: if unstable {
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

fn validate_expected_snapshot(
    expected: &AuthenticatedExpectedPathSnapshot,
    maximum_rows: usize,
) -> Result<(), ExpectedPathSourceFailure> {
    if expected.rows.len() > maximum_rows {
        return Err(ExpectedPathSourceFailure::Unavailable);
    }
    if expected.rows_commitment != expected_rows_commitment(expected.binding, &expected.rows) {
        return Err(ExpectedPathSourceFailure::Corrupt);
    }
    let mut exact = BTreeSet::new();
    let mut portable = BTreeSet::new();
    for row in &expected.rows {
        if !exact.insert(row.path.as_str()) || !portable.insert(row.path.portable_key()) {
            return Err(ExpectedPathSourceFailure::Ambiguous);
        }
    }
    Ok(())
}

fn expected_rows_commitment(
    binding: ExpectedPathBinding,
    rows: &[AuthenticatedExpectedPath],
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/test-only/reconciliation-expected-paths/v1\0");
    hasher.update(binding.accepted_frontier.as_bytes());
    hasher.update(binding.projection_generation.to_be_bytes());
    hasher.update((rows.len() as u64).to_be_bytes());
    for row in rows {
        hash_len_bytes(&mut hasher, row.path.as_str().as_bytes());
        hasher.update(match row.kind {
            ManagedTextKind::Page => [0],
            ManagedTextKind::Journal => [1],
        });
        hash_description(&mut hasher, row.description);
        hasher.update(row.owner_binding.as_bytes());
    }
    ContentDigest::from_bytes(hasher.finalize().into())
}

fn scan_epoch_digest(
    pass: &GraphTextScanPass,
    expected: &AuthenticatedExpectedPathSnapshot,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/test-only/reconciliation-scan-epoch/v1\0");
    hasher.update(pass.graph_resource.as_bytes());
    hasher.update(pass.scope_binding.canonical_bytes());
    hasher.update(expected.binding.accepted_frontier.as_bytes());
    hasher.update(expected.binding.projection_generation.to_be_bytes());
    hasher.update(expected.rows_commitment.as_bytes());
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

fn derive_candidates(
    pass: &GraphTextScanPass,
    expected: &AuthenticatedExpectedPathSnapshot,
    binding: &GraphTextCandidateBinding,
) -> (Vec<GraphTextScanCandidate>, Vec<GraphTextScanDiagnostic>) {
    let disk = pass
        .files
        .iter()
        .filter_map(|file| {
            file.class
                .is_eligible()
                .then(|| (file.exact_relative.as_str(), file))
        })
        .collect::<BTreeMap<_, _>>();
    let expected_by_path = expected
        .rows
        .iter()
        .map(|row| (row.path.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = Vec::new();
    for row in &expected.rows {
        match disk.get(row.path.as_str()) {
            None => candidates.push(GraphTextScanCandidate {
                path: row.path.clone(),
                managed_kind: row.kind,
                change: GraphTextCandidateKind::Absence,
                expected_description: Some(row.description),
                expected_owner_binding: Some(row.owner_binding),
                observed_description: None,
                observed_file_resource_id: None,
                observed_link_count: None,
                binding: binding.clone(),
            }),
            Some(file) if file.description != Some(row.description) => {
                candidates.push(GraphTextScanCandidate {
                    path: row.path.clone(),
                    managed_kind: row.kind,
                    change: GraphTextCandidateKind::Edit,
                    expected_description: Some(row.description),
                    expected_owner_binding: Some(row.owner_binding),
                    observed_description: file.description,
                    observed_file_resource_id: Some(file.file_resource_id),
                    observed_link_count: Some(file.link_count),
                    binding: binding.clone(),
                });
            }
            Some(_) => {}
        }
    }
    for file in &pass.files {
        let Some(kind) = file.class.managed_kind() else {
            continue;
        };
        if expected_by_path.contains_key(file.exact_relative.as_str()) {
            continue;
        }
        let path = ManagedPath::parse(file.exact_relative.clone())
            .expect("eligible scan rows retain validated managed paths");
        candidates.push(GraphTextScanCandidate {
            path,
            managed_kind: kind,
            change: GraphTextCandidateKind::Creation,
            expected_description: None,
            expected_owner_binding: None,
            observed_description: file.description,
            observed_file_resource_id: Some(file.file_resource_id),
            observed_link_count: Some(file.link_count),
            binding: binding.clone(),
        });
    }
    candidates.sort_unstable_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.change.cmp(&right.change))
    });

    let diagnostics = pass
        .files
        .iter()
        .filter_map(|file| {
            let kind = match file.class {
                GraphTextScanPathClass::ProviderConflictCopy => {
                    GraphTextScanDiagnosticKind::ProviderConflictCopy
                }
                GraphTextScanPathClass::EligibleUnmanaged => {
                    GraphTextScanDiagnosticKind::EligibleOutsideCreationRoots
                }
                GraphTextScanPathClass::EligibleManaged(_)
                | GraphTextScanPathClass::Configuration
                | GraphTextScanPathClass::RetainedNonText => return None,
            };
            Some(GraphTextScanDiagnostic {
                path: file.exact_relative.clone(),
                kind,
                file_resource_id: file.file_resource_id,
                link_count: file.link_count,
            })
        })
        .collect();
    (candidates, diagnostics)
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
        snapshot: Result<AuthenticatedExpectedPathSnapshot, ExpectedPathSourceFailure>,
        binding: Cell<ExpectedPathBinding>,
    }

    impl FixtureExpectedSource {
        fn empty() -> Self {
            let binding = expected_binding(1);
            Self {
                snapshot: Ok(AuthenticatedExpectedPathSnapshot::new(binding, Vec::new())),
                binding: Cell::new(binding),
            }
        }

        fn with_rows(rows: Vec<AuthenticatedExpectedPath>) -> Self {
            let binding = expected_binding(1);
            Self {
                snapshot: Ok(AuthenticatedExpectedPathSnapshot::new(binding, rows)),
                binding: Cell::new(binding),
            }
        }

        fn failure(failure: ExpectedPathSourceFailure) -> Self {
            Self {
                snapshot: Err(failure),
                binding: Cell::new(expected_binding(1)),
            }
        }
    }

    impl AuthenticatedExpectedPathSource for FixtureExpectedSource {
        fn capture_expected_paths(
            &self,
            maximum_rows: usize,
        ) -> Result<AuthenticatedExpectedPathSnapshot, ExpectedPathSourceFailure> {
            let snapshot = self.snapshot.clone()?;
            if snapshot.rows.len() > maximum_rows {
                return Err(ExpectedPathSourceFailure::Unavailable);
            }
            Ok(snapshot)
        }

        fn current_binding(&self) -> Result<ExpectedPathBinding, ExpectedPathSourceFailure> {
            Ok(self.binding.get())
        }
    }

    fn expected_binding(generation: u64) -> ExpectedPathBinding {
        ExpectedPathBinding {
            accepted_frontier: ContentDigest::of(format!("frontier-{generation}").as_bytes()),
            projection_generation: generation,
        }
    }

    fn expected_row(path: &str, kind: ManagedTextKind, bytes: &[u8]) -> AuthenticatedExpectedPath {
        AuthenticatedExpectedPath {
            path: ManagedPath::parse(path).unwrap(),
            kind,
            description: BlobDescription::of(bytes),
            owner_binding: ContentDigest::of(format!("owner:{path}").as_bytes()),
        }
    }

    fn candidate_signature(
        scan: &StableGraphTextScan,
    ) -> Vec<(String, GraphTextCandidateKind, ManagedTextKind)> {
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
                    ManagedTextKind::Page
                ),
                (
                    "pages/create.md".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Page
                ),
                (
                    "pages/delete.md".into(),
                    GraphTextCandidateKind::Absence,
                    ManagedTextKind::Page
                ),
                (
                    "pages/edit.md".into(),
                    GraphTextCandidateKind::Edit,
                    ManagedTextKind::Page
                ),
                (
                    "pages/rename-new.md".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Page
                ),
                (
                    "pages/rename-old.md".into(),
                    GraphTextCandidateKind::Absence,
                    ManagedTextKind::Page
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
                (
                    "managed/text/Another.MARKDOWN".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Page
                ),
                (
                    "managed/text/Page.mD".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Page
                ),
                (
                    "managed/text/daily/2026-07-26.ORG".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Journal
                ),
            ]
        );
        assert_eq!(
            scan.diagnostics
                .iter()
                .map(|diagnostic| diagnostic.path.as_str())
                .collect::<Vec<_>>(),
            vec!["Root.MD", "archive/nested.Markdown"]
        );
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
                ManagedTextKind::Page
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
                    ManagedTextKind::Page
                ),
                (
                    "pages/new.md".into(),
                    GraphTextCandidateKind::Creation,
                    ManagedTextKind::Page
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

        let mut corrupt = FixtureExpectedSource::empty();
        corrupt.snapshot.as_mut().unwrap().rows_commitment = ContentDigest::of(b"forged");
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

    #[allow(dead_code)]
    fn _assert_path_is_inside_temp_graph(path: &Path, temp: &TempGraph) {
        assert!(path.starts_with(&temp.root));
    }
}
