//! Device-local, disposable reconciliation acceleration and diagnostics.
//!
//! Nothing in this module is semantic authority. In particular, a clean head,
//! a recorded absence, or a missing row cannot suppress full authenticated
//! hashing, authorize import or oplog publication, or authorize overwriting
//! graph Markdown. An unavailable or suspicious baseline requires a full
//! authenticated scan. Callers must explicitly place this database in Tine's
//! private, device-local application runtime root and may discard it whenever
//! it is unavailable.

use super::{
    object_store::{ensure_directory_nofollow, open_dir_nofollow, sync_dir_required},
    projection_work_index::DurablyPublishedProjectionCompletion,
    ApplicationRuntimeRoot, BlobDescription, CanonicalGraphResourceId, ContentDigest,
    GraphTextScopeBinding, LogicalCompletionId, ManagedPath, ManagedTextKind, ProjectionEndpointId,
    ProjectionWorkTarget, WorkspaceId, MANAGED_ENTITY_SET_VERSION,
};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions as CapOpenOptions},
};
use rusqlite::{
    params, Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension as _,
    Transaction, TransactionBehavior,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const RECONCILIATION_BASELINE_SCHEMA_VERSION: u32 = 2;
pub(crate) const RECONCILIATION_BASELINE_APPLICATION_ID: u32 = 0x5449_4e42;
pub(crate) const MAX_BASELINE_WRITE_ROWS: usize = 512;
pub(crate) const MAX_BASELINE_PAGE_ROWS: usize = 512;
pub(crate) const MAX_BASELINE_EPOCHS: usize = 64;
pub(crate) const MAX_BASELINE_PATHS_PER_EPOCH: usize = 1_000_000;
pub(crate) const MAX_BASELINE_DIRECTORIES_PER_EPOCH: usize = 1_000_000;
pub(crate) const MAX_BASELINE_SCAN_ENTRIES: u64 = 2_000_000;
pub(crate) const MAX_BASELINE_BLOCKED_SIGNATURES: usize = 100_000;
pub(crate) const MAX_BASELINE_EXACT_PATH_BYTES: usize = 4096;
pub(crate) const MAX_BASELINE_AGGREGATE_PATH_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_BASELINE_BLOCKED_REASON_BYTES: usize = 4096;
const MAX_SCHEMA_OBJECTS: i64 = 32;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(250);
const RECONCILIATION_DIRECTORY: &str = "reconciliation";
const DATABASE_FILE: &str = "scan.sqlite";
const DATABASE_SIDECAR_FILES: &[&str] =
    &["scan.sqlite-journal", "scan.sqlite-wal", "scan.sqlite-shm"];

const BINDING_DDL: &str = "
CREATE TABLE binding (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL,
    workspace BLOB NOT NULL CHECK (length(workspace) = 16),
    endpoint BLOB NOT NULL CHECK (length(endpoint) = 16),
    graph_resource BLOB NOT NULL CHECK (length(graph_resource) = 32),
    scope_binding BLOB NOT NULL,
    managed_entity_version INTEGER NOT NULL
) STRICT;";

const EPOCHS_DDL: &str = "
CREATE TABLE epochs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    state INTEGER NOT NULL CHECK (state BETWEEN 0 AND 3),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    accepted_frontier BLOB NOT NULL CHECK (length(accepted_frontier) = 32),
    projection_generation INTEGER NOT NULL CHECK (projection_generation >= 0),
    pass_a_digest BLOB CHECK (pass_a_digest IS NULL OR length(pass_a_digest) = 32),
    pass_b_digest BLOB CHECK (pass_b_digest IS NULL OR length(pass_b_digest) = 32),
    candidate_digest BLOB CHECK (candidate_digest IS NULL OR length(candidate_digest) = 32),
    candidate_count INTEGER NOT NULL DEFAULT 0 CHECK (candidate_count >= 0),
    path_count INTEGER NOT NULL DEFAULT 0 CHECK (path_count >= 0),
    directory_count INTEGER NOT NULL DEFAULT 0 CHECK (directory_count >= 0),
    aggregate_path_bytes INTEGER NOT NULL DEFAULT 0 CHECK (aggregate_path_bytes >= 0),
    scan_passes INTEGER CHECK (scan_passes IS NULL OR scan_passes BETWEEN 0 AND 2),
    scan_directory_entries INTEGER CHECK (
        scan_directory_entries IS NULL OR scan_directory_entries >= 0
    ),
    scan_directories INTEGER CHECK (scan_directories IS NULL OR scan_directories >= 0),
    scan_regular_files INTEGER CHECK (scan_regular_files IS NULL OR scan_regular_files >= 0),
    scan_eligible_files INTEGER CHECK (scan_eligible_files IS NULL OR scan_eligible_files >= 0),
    scan_bytes_read INTEGER CHECK (scan_bytes_read IS NULL OR scan_bytes_read >= 0),
    scan_bytes_hashed INTEGER CHECK (scan_bytes_hashed IS NULL OR scan_bytes_hashed >= 0),
    scan_peak_retained_rows INTEGER CHECK (
        scan_peak_retained_rows IS NULL OR scan_peak_retained_rows >= 0
    ),
    scan_peak_retained_bytes INTEGER CHECK (
        scan_peak_retained_bytes IS NULL OR scan_peak_retained_bytes >= 0
    ),
    scan_candidates INTEGER CHECK (scan_candidates IS NULL OR scan_candidates >= 0),
    scan_diagnostics INTEGER CHECK (scan_diagnostics IS NULL OR scan_diagnostics >= 0),
    scan_wall_time_ms INTEGER CHECK (scan_wall_time_ms IS NULL OR scan_wall_time_ms >= 0)
) STRICT;";

const PATHS_DDL: &str = "
CREATE TABLE paths (
    epoch_id INTEGER NOT NULL REFERENCES epochs(id) ON DELETE CASCADE,
    exact_path TEXT NOT NULL,
    managed_kind INTEGER CHECK (managed_kind IS NULL OR managed_kind IN (1, 2)),
    state INTEGER NOT NULL CHECK (state IN (1, 2)),
    content_digest BLOB CHECK (content_digest IS NULL OR length(content_digest) = 32),
    byte_len INTEGER CHECK (byte_len IS NULL OR byte_len >= 0),
    file_resource BLOB CHECK (file_resource IS NULL OR length(file_resource) = 32),
    link_count INTEGER CHECK (link_count IS NULL OR link_count >= 0),
    source INTEGER NOT NULL CHECK (source IN (1, 2)),
    completion_identity BLOB
        CHECK (completion_identity IS NULL OR length(completion_identity) = 32),
    completion_frontier BLOB
        CHECK (completion_frontier IS NULL OR length(completion_frontier) = 32),
    PRIMARY KEY (epoch_id, exact_path),
    CHECK (
        (state = 1 AND content_digest IS NOT NULL AND byte_len IS NOT NULL)
        OR
        (state = 2 AND content_digest IS NULL AND byte_len IS NULL
            AND file_resource IS NULL AND link_count IS NULL)
    ),
    CHECK (
        (source = 1 AND completion_identity IS NULL AND completion_frontier IS NULL)
        OR
        (source = 2 AND completion_identity IS NOT NULL
            AND completion_frontier IS NOT NULL AND managed_kind IS NOT NULL)
    )
) STRICT;";

const DIRECTORIES_DDL: &str = "
CREATE TABLE directories (
    epoch_id INTEGER NOT NULL REFERENCES epochs(id) ON DELETE CASCADE,
    exact_path TEXT NOT NULL,
    resource BLOB NOT NULL CHECK (length(resource) = 32),
    PRIMARY KEY (epoch_id, exact_path)
) STRICT;";

const BLOCKED_DDL: &str = "
CREATE TABLE blocked (
    exact_observation_digest BLOB PRIMARY KEY
        CHECK (length(exact_observation_digest) = 32),
    reason INTEGER NOT NULL CHECK (reason BETWEEN 1 AND 5),
    detail TEXT NOT NULL,
    first_seen INTEGER NOT NULL CHECK (first_seen >= 0),
    last_seen INTEGER NOT NULL CHECK (last_seen >= first_seen)
) STRICT;";

const HEAD_DDL: &str = "
CREATE TABLE head (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    completed_epoch INTEGER NOT NULL REFERENCES epochs(id),
    baseline_generation INTEGER NOT NULL CHECK (baseline_generation > 0)
) STRICT;";

const EPOCH_STATE_INDEX_DDL: &str = "CREATE INDEX epochs_state_id_idx ON epochs(state, id);";
const BLOCKED_LAST_SEEN_INDEX_DDL: &str =
    "CREATE INDEX blocked_last_seen_idx ON blocked(last_seen);";

const EXPECTED_TABLES: &[&str] = &[
    "binding",
    "blocked",
    "directories",
    "epochs",
    "head",
    "paths",
];
const EXPECTED_INDEXES: &[&str] = &["blocked_last_seen_idx", "epochs_state_id_idx"];

/// Placement authority for disposable reconciliation baselines.
///
/// Production callers may construct this only from Tine's platform-selected,
/// private, device-local application runtime root. A user graph, synced
/// directory, or caller-selected shared path does not satisfy this contract.
/// Construction stays crate-private so later activation can be owned by the
/// Tauri app-data bootstrap without exposing a general path-based API.
///
/// This marker is not a same-OS-user security boundary. Stock SQLite opens the
/// database and its sidecars by ambient path, so another process with the same
/// app-data authority is out of scope. No-follow namespace inspections and
/// database/binding validation are best-effort accidental-substitution
/// defenses, not protection from an adversarial namespace race.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedPrivateApplicationRuntimeRoot {
    path: PathBuf,
}

impl TrustedPrivateApplicationRuntimeRoot {
    /// Acknowledge the private-runtime placement contract for an application
    /// root selected by Tine. Non-test callers must not promote harness or
    /// caller-arbitrary roots through this conversion.
    pub(crate) fn from_application_runtime_root(runtime: &ApplicationRuntimeRoot) -> Self {
        Self {
            path: runtime.path().to_path_buf(),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconciliationBaselineBinding {
    workspace: WorkspaceId,
    endpoint: ProjectionEndpointId,
    graph_resource: CanonicalGraphResourceId,
    scope_binding: GraphTextScopeBinding,
}

impl ReconciliationBaselineBinding {
    pub(crate) fn new(
        workspace: WorkspaceId,
        endpoint: ProjectionEndpointId,
        graph_resource: CanonicalGraphResourceId,
        scope_binding: GraphTextScopeBinding,
    ) -> Result<Self, ReconciliationBaselineError> {
        if scope_binding.graph_resource_id() != graph_resource {
            return Err(unavailable(
                "graph-text scope binding does not name the retained graph resource",
            ));
        }
        Ok(Self {
            workspace,
            endpoint,
            graph_resource,
            scope_binding,
        })
    }

    pub(crate) const fn workspace(&self) -> WorkspaceId {
        self.workspace
    }

    pub(crate) const fn endpoint(&self) -> ProjectionEndpointId {
        self.endpoint
    }

    pub(crate) const fn graph_resource(&self) -> CanonicalGraphResourceId {
        self.graph_resource
    }

    pub(crate) const fn scope_binding(&self) -> GraphTextScopeBinding {
        self.scope_binding
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconciliationBaselineError {
    BaselineUnavailable { detail: String },
    RebuildRequired { path: PathBuf, detail: String },
}

impl ReconciliationBaselineError {
    pub(crate) const fn requires_rebuild(&self) -> bool {
        matches!(self, Self::RebuildRequired { .. })
    }
}

impl fmt::Display for ReconciliationBaselineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BaselineUnavailable { detail } => {
                write!(formatter, "reconciliation baseline unavailable: {detail}")
            }
            Self::RebuildRequired { path, detail } => write!(
                formatter,
                "reconciliation baseline {} requires an explicit rebuild: {detail}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ReconciliationBaselineError {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct BaselineEpochId(i64);

impl BaselineEpochId {
    pub(crate) const fn as_i64(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BaselineTimestamp(i64);

impl BaselineTimestamp {
    pub(crate) fn from_millis(value: u64) -> Result<Self, ReconciliationBaselineError> {
        i64::try_from(value)
            .map(Self)
            .map_err(|_| unavailable("baseline timestamp exceeds SQLite integer range"))
    }

    pub(crate) const fn as_millis(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineDirectoryPath(String);

impl BaselineDirectoryPath {
    pub(crate) fn parse(value: impl Into<String>) -> Result<Self, ReconciliationBaselineError> {
        let value = value.into();
        if value.len() > MAX_BASELINE_EXACT_PATH_BYTES {
            return Err(unavailable("baseline directory path byte bound exceeded"));
        }
        if value.is_empty() {
            return Ok(Self(value));
        }
        if value != value.trim()
            || value.starts_with('/')
            || value.contains('\\')
            || value
                .split('/')
                .any(|component| !super::managed_component_is_portable(component))
        {
            return Err(unavailable("baseline directory path is not portable"));
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BeginBaselineEpoch {
    pub(crate) started_at: BaselineTimestamp,
    pub(crate) accepted_frontier: ContentDigest,
    pub(crate) projection_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineObservedState {
    Present {
        description: BlobDescription,
        file_resource: ContentDigest,
        link_count: u64,
    },
    Absent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineScanPath {
    pub(crate) path: ManagedPath,
    pub(crate) managed_kind: Option<ManagedTextKind>,
    pub(crate) state: BaselineObservedState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineScanDirectory {
    pub(crate) path: BaselineDirectoryPath,
    pub(crate) resource: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineEpochOutcome {
    Noop,
    Complete,
    Blocked,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BaselineScanInstrumentation {
    pub(crate) passes: u64,
    pub(crate) directory_entries: u64,
    pub(crate) directories: u64,
    pub(crate) regular_files: u64,
    pub(crate) eligible_files: u64,
    pub(crate) bytes_read: u64,
    pub(crate) bytes_hashed: u64,
    pub(crate) peak_retained_rows: u64,
    pub(crate) peak_retained_bytes: u64,
    pub(crate) candidates: u64,
    pub(crate) diagnostics: u64,
    pub(crate) wall_time_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinishBaselineEpoch {
    pub(crate) completed_at: BaselineTimestamp,
    pub(crate) pass_a_digest: ContentDigest,
    pub(crate) pass_b_digest: ContentDigest,
    pub(crate) candidate_digest: ContentDigest,
    pub(crate) candidate_count: usize,
    pub(crate) outcome: BaselineEpochOutcome,
    pub(crate) instrumentation: BaselineScanInstrumentation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BaselineCompletionIdentity([u8; 32]);

impl BaselineCompletionIdentity {
    pub(crate) const fn from_logical(value: LogicalCompletionId) -> Self {
        Self(*value.as_bytes())
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TineCompletionState {
    Present(BlobDescription),
    Deleted,
}

struct AuthenticatedTineCompletion<'a> {
    workspace: WorkspaceId,
    endpoint: ProjectionEndpointId,
    graph_resource: CanonicalGraphResourceId,
    path: &'a ManagedPath,
    managed_kind: ManagedTextKind,
    state: TineCompletionState,
    completion_identity: BaselineCompletionIdentity,
    completion_frontier: ContentDigest,
    projection_generation: u64,
    completed_at: BaselineTimestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BaselineHead {
    pub(crate) epoch: BaselineEpochId,
    pub(crate) baseline_generation: u64,
    pub(crate) accepted_frontier: ContentDigest,
    pub(crate) projection_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselinePathSource {
    StableScan,
    TineCompletion(BaselineCompletionIdentity),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineRecordedState {
    Present(BlobDescription),
    ExpectedAbsent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselinePathRecord {
    pub(crate) path: ManagedPath,
    pub(crate) managed_kind: Option<ManagedTextKind>,
    pub(crate) state: BaselineRecordedState,
    pub(crate) file_resource: Option<ContentDigest>,
    pub(crate) link_count: Option<u64>,
    pub(crate) source: BaselinePathSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselinePathPage {
    pub(crate) head: BaselineHead,
    pub(crate) rows: Vec<BaselinePathRecord>,
    pub(crate) next_after: Option<ManagedPath>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BaselineBlockedReason {
    UnsafeFilesystem,
    UnstableEpoch,
    ReconciliationFailed,
    BoundExceeded,
    AuthorityUnavailable,
}

impl BaselineBlockedReason {
    const fn tag(self) -> i64 {
        match self {
            Self::UnsafeFilesystem => 1,
            Self::UnstableEpoch => 2,
            Self::ReconciliationFailed => 3,
            Self::BoundExceeded => 4,
            Self::AuthorityUnavailable => 5,
        }
    }

    fn from_tag(tag: i64) -> Option<Self> {
        match tag {
            1 => Some(Self::UnsafeFilesystem),
            2 => Some(Self::UnstableEpoch),
            3 => Some(Self::ReconciliationFailed),
            4 => Some(Self::BoundExceeded),
            5 => Some(Self::AuthorityUnavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineBlockedSignature {
    pub(crate) observation_digest: ContentDigest,
    pub(crate) reason: BaselineBlockedReason,
    pub(crate) detail: String,
    pub(crate) first_seen: BaselineTimestamp,
    pub(crate) last_seen: BaselineTimestamp,
}

pub(crate) struct ReconciliationBaseline {
    connection: Connection,
    path: PathBuf,
    binding: ReconciliationBaselineBinding,
    trusted_data_version: i64,
}

impl ReconciliationBaseline {
    /// Create a new baseline. Existing bytes are never overwritten or removed;
    /// preserving/moving an unavailable database and retrying is an explicit
    /// caller recovery action.
    pub(crate) fn create_fresh(
        trusted_runtime_root: &TrustedPrivateApplicationRuntimeRoot,
        binding: ReconciliationBaselineBinding,
    ) -> Result<Self, ReconciliationBaselineError> {
        let (parent, path) = prepare_database_parent(trusted_runtime_root, &binding, true)?;
        require_vacant_database_namespace(&parent, &path)?;
        create_database_file_nofollow(&parent, &path)?;
        let mut connection = open_ambient_sqlite_connection(&path, true)?;
        require_existing_regular(&parent, &path)?;
        require_safe_sqlite_sidecars(&parent, &path)?;
        initialize_schema(&connection, &path, &binding)?;
        let trusted_data_version = validate_database(&mut connection, &path, &binding)?;
        sync_dir_required(&parent)
            .map_err(|error| unavailable(format!("cannot sync baseline directory: {error}")))?;
        Ok(Self {
            connection,
            path,
            binding,
            trusted_data_version,
        })
    }

    /// Open and fully validate an existing baseline without creating or
    /// repairing anything.
    pub(crate) fn open_existing(
        trusted_runtime_root: &TrustedPrivateApplicationRuntimeRoot,
        binding: ReconciliationBaselineBinding,
    ) -> Result<Self, ReconciliationBaselineError> {
        let (parent, path) = prepare_database_parent(trusted_runtime_root, &binding, false)?;
        require_existing_regular(&parent, &path)?;
        require_safe_sqlite_sidecars(&parent, &path)?;
        let mut connection = open_ambient_sqlite_connection(&path, false)?;
        require_existing_regular(&parent, &path)?;
        require_safe_sqlite_sidecars(&parent, &path)?;
        let trusted_data_version = validate_database(&mut connection, &path, &binding)?;
        configure_trusted_connection(&connection, &path)?;
        Ok(Self {
            connection,
            path,
            binding,
            trusted_data_version,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn binding(&self) -> &ReconciliationBaselineBinding {
        &self.binding
    }

    pub(crate) fn begin_epoch(
        &mut self,
        begin: BeginBaselineEpoch,
    ) -> Result<BaselineEpochId, ReconciliationBaselineError> {
        let projection_generation =
            sqlite_u64(begin.projection_generation, "projection generation")?;
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let epoch_count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM epochs", [], |row| row.get(0))
            .map_err(|error| classify_sql_error(&path, error, "counting baseline epochs"))?;
        if epoch_count >= MAX_BASELINE_EPOCHS as i64 {
            let removed = transaction
                .execute(
                    "DELETE FROM epochs
                     WHERE id = (
                        SELECT e.id FROM epochs e
                        WHERE e.state != 0
                          AND e.id != COALESCE(
                              (SELECT completed_epoch FROM head WHERE singleton = 1),
                              -1
                          )
                        ORDER BY e.id
                        LIMIT 1
                     )",
                    [],
                )
                .map_err(|error| {
                    classify_sql_error(&path, error, "pruning oldest diagnostic epoch")
                })?;
            if removed != 1 {
                return Err(unavailable(
                    "baseline retained epoch bound exceeded by active/head epochs",
                ));
            }
        }
        transaction
            .execute(
                "INSERT INTO epochs (
                    state, started_at, accepted_frontier, projection_generation
                 ) VALUES (0, ?1, ?2, ?3)",
                params![
                    begin.started_at.as_millis(),
                    digest_blob(begin.accepted_frontier),
                    projection_generation
                ],
            )
            .map_err(|error| classify_sql_error(&path, error, "starting baseline epoch"))?;
        let id = transaction.last_insert_rowid();
        if id <= 0 {
            return Err(rebuild(
                &path,
                "SQLite returned an impossible epoch identity",
            ));
        }
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "committing baseline epoch"))?;
        Ok(BaselineEpochId(id))
    }

    pub(crate) fn append_scan_paths(
        &mut self,
        epoch: BaselineEpochId,
        rows: &[BaselineScanPath],
    ) -> Result<(), ReconciliationBaselineError> {
        validate_write_batch(rows.len(), "baseline path")?;
        let batch_path_bytes = rows.iter().try_fold(0_u64, |total, row| {
            validate_managed_path(&row.path)?;
            if let BaselineObservedState::Present { link_count, .. } = row.state {
                if link_count != 1 {
                    return Err(unavailable(
                        "stable scan baseline rows require an unambiguous link count of one",
                    ));
                }
            }
            total
                .checked_add(row.path.as_str().len() as u64)
                .ok_or_else(|| unavailable("baseline aggregate path bytes overflow"))
        })?;
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let (state, path_count, aggregate_bytes) = epoch_write_state(&transaction, &path, epoch)?;
        if state != EpochState::Building {
            return Err(unavailable(
                "cannot append paths to a finished baseline epoch",
            ));
        }
        let new_count = checked_total(
            path_count,
            rows.len(),
            MAX_BASELINE_PATHS_PER_EPOCH,
            "baseline epoch path",
        )?;
        let new_aggregate = checked_aggregate_path_bytes(aggregate_bytes, batch_path_bytes)?;
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO paths (
                        epoch_id, exact_path, managed_kind, state, content_digest,
                        byte_len, file_resource, link_count, source, completion_identity,
                        completion_frontier
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, NULL, NULL)",
                )
                .map_err(|error| {
                    classify_sql_error(&path, error, "preparing baseline path insert")
                })?;
            for row in rows {
                let kind = row.managed_kind.map(managed_kind_tag);
                let (state, digest, length, resource, links) = match row.state {
                    BaselineObservedState::Present {
                        description,
                        file_resource,
                        link_count,
                    } => (
                        1_i64,
                        Some(description.sha256().to_vec()),
                        Some(sqlite_u64(description.byte_length(), "blob byte length")?),
                        Some(file_resource.as_bytes().to_vec()),
                        Some(sqlite_u64(link_count, "file link count")?),
                    ),
                    BaselineObservedState::Absent => (2, None, None, None, None),
                };
                statement
                    .execute(params![
                        epoch.as_i64(),
                        row.path.as_str(),
                        kind,
                        state,
                        digest,
                        length,
                        resource,
                        links
                    ])
                    .map_err(|error| {
                        classify_sql_error(&path, error, "inserting baseline path row")
                    })?;
            }
        }
        transaction
            .execute(
                "UPDATE epochs
                 SET path_count = ?2, aggregate_path_bytes = ?3
                 WHERE id = ?1 AND state = 0",
                params![
                    epoch.as_i64(),
                    sqlite_usize(new_count, "path count")?,
                    sqlite_u64(new_aggregate, "aggregate path bytes")?
                ],
            )
            .map_err(|error| classify_sql_error(&path, error, "updating baseline path counters"))?;
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "committing baseline paths"))
    }

    pub(crate) fn append_scan_directories(
        &mut self,
        epoch: BaselineEpochId,
        rows: &[BaselineScanDirectory],
    ) -> Result<(), ReconciliationBaselineError> {
        validate_write_batch(rows.len(), "baseline directory")?;
        let batch_path_bytes = rows.iter().try_fold(0_u64, |total, row| {
            total
                .checked_add(row.path.as_str().len() as u64)
                .ok_or_else(|| unavailable("baseline aggregate path bytes overflow"))
        })?;
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let (state, directory_count, aggregate_bytes) =
            epoch_directory_write_state(&transaction, &path, epoch)?;
        if state != EpochState::Building {
            return Err(unavailable(
                "cannot append directories to a finished baseline epoch",
            ));
        }
        let new_count = checked_total(
            directory_count,
            rows.len(),
            MAX_BASELINE_DIRECTORIES_PER_EPOCH,
            "baseline epoch directory",
        )?;
        let new_aggregate = checked_aggregate_path_bytes(aggregate_bytes, batch_path_bytes)?;
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO directories (epoch_id, exact_path, resource)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|error| {
                    classify_sql_error(&path, error, "preparing baseline directory insert")
                })?;
            for row in rows {
                statement
                    .execute(params![
                        epoch.as_i64(),
                        row.path.as_str(),
                        digest_blob(row.resource)
                    ])
                    .map_err(|error| {
                        classify_sql_error(&path, error, "inserting baseline directory row")
                    })?;
            }
        }
        transaction
            .execute(
                "UPDATE epochs
                 SET directory_count = ?2, aggregate_path_bytes = ?3
                 WHERE id = ?1 AND state = 0",
                params![
                    epoch.as_i64(),
                    sqlite_usize(new_count, "directory count")?,
                    sqlite_u64(new_aggregate, "aggregate path bytes")?
                ],
            )
            .map_err(|error| {
                classify_sql_error(&path, error, "updating baseline directory counters")
            })?;
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "committing baseline directories"))
    }

    pub(crate) fn finish_epoch(
        &mut self,
        epoch: BaselineEpochId,
        finish: FinishBaselineEpoch,
    ) -> Result<Option<BaselineHead>, ReconciliationBaselineError> {
        if finish.candidate_count > MAX_BASELINE_PATHS_PER_EPOCH {
            return Err(unavailable("baseline candidate count bound exceeded"));
        }
        if finish.outcome == BaselineEpochOutcome::Noop && finish.candidate_count != 0 {
            return Err(unavailable(
                "a Noop reconciliation outcome cannot retain candidates",
            ));
        }
        let clean = matches!(
            finish.outcome,
            BaselineEpochOutcome::Noop | BaselineEpochOutcome::Complete
        );
        validate_scan_instrumentation(&finish.instrumentation, finish.candidate_count, clean)?;
        if clean && finish.pass_a_digest != finish.pass_b_digest {
            return Err(unavailable(
                "an unstable two-pass scan cannot become the clean baseline head",
            ));
        }
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let epoch_row = load_epoch_for_finish(&transaction, &path, epoch)?;
        if epoch_row.state != EpochState::Building {
            return Err(unavailable("baseline epoch was already finished"));
        }
        if finish.completed_at.as_millis() < epoch_row.started_at {
            return Err(unavailable(
                "baseline epoch completion precedes its start timestamp",
            ));
        }
        require_epoch_counters_match(&transaction, &path, epoch, &epoch_row)?;
        if clean {
            require_epoch_root_binding(&transaction, &path, epoch, &self.binding)?;
        }
        let state = match finish.outcome {
            BaselineEpochOutcome::Noop | BaselineEpochOutcome::Complete => EpochState::Clean,
            BaselineEpochOutcome::Blocked => EpochState::Blocked,
            BaselineEpochOutcome::Incomplete => EpochState::Incomplete,
        };
        transaction
            .execute(
                "UPDATE epochs
                 SET state = ?2, completed_at = ?3, pass_a_digest = ?4,
                     pass_b_digest = ?5, candidate_digest = ?6, candidate_count = ?7,
                     scan_passes = ?8, scan_directory_entries = ?9,
                     scan_directories = ?10, scan_regular_files = ?11,
                     scan_eligible_files = ?12, scan_bytes_read = ?13,
                     scan_bytes_hashed = ?14, scan_peak_retained_rows = ?15,
                     scan_peak_retained_bytes = ?16, scan_candidates = ?17,
                     scan_diagnostics = ?18, scan_wall_time_ms = ?19
                 WHERE id = ?1 AND state = 0",
                params![
                    epoch.as_i64(),
                    state.tag(),
                    finish.completed_at.as_millis(),
                    digest_blob(finish.pass_a_digest),
                    digest_blob(finish.pass_b_digest),
                    digest_blob(finish.candidate_digest),
                    sqlite_usize(finish.candidate_count, "candidate count")?,
                    sqlite_u64(finish.instrumentation.passes, "scan passes")?,
                    sqlite_u64(
                        finish.instrumentation.directory_entries,
                        "scan directory entries"
                    )?,
                    sqlite_u64(finish.instrumentation.directories, "scan directories")?,
                    sqlite_u64(finish.instrumentation.regular_files, "scan regular files")?,
                    sqlite_u64(finish.instrumentation.eligible_files, "scan eligible files")?,
                    sqlite_u64(finish.instrumentation.bytes_read, "scan bytes read")?,
                    sqlite_u64(finish.instrumentation.bytes_hashed, "scan bytes hashed")?,
                    sqlite_u64(
                        finish.instrumentation.peak_retained_rows,
                        "scan peak retained rows"
                    )?,
                    sqlite_u64(
                        finish.instrumentation.peak_retained_bytes,
                        "scan peak retained bytes"
                    )?,
                    sqlite_u64(finish.instrumentation.candidates, "scan candidates")?,
                    sqlite_u64(finish.instrumentation.diagnostics, "scan diagnostics")?,
                    sqlite_u64(finish.instrumentation.wall_time_millis, "scan wall time")?
                ],
            )
            .map_err(|error| classify_sql_error(&path, error, "finishing baseline epoch"))?;

        let head = if clean {
            let previous: Option<(i64, i64)> = transaction
                .query_row(
                    "SELECT h.baseline_generation, e.projection_generation
                     FROM head h JOIN epochs e ON e.id = h.completed_epoch
                     WHERE h.singleton = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| {
                    classify_sql_error(&path, error, "reading previous baseline head")
                })?;
            let baseline_generation = match previous {
                Some((generation, prior_projection_generation)) => {
                    if generation <= 0
                        || epoch_row.projection_generation < prior_projection_generation
                    {
                        return Err(rebuild(
                            &path,
                            "baseline head generation or projection generation regressed",
                        ));
                    }
                    generation.checked_add(1).ok_or_else(|| {
                        rebuild(&path, "baseline generation overflowed SQLite integer")
                    })?
                }
                None => 1,
            };
            transaction
                .execute(
                    "INSERT INTO head (singleton, completed_epoch, baseline_generation)
                     VALUES (1, ?1, ?2)
                     ON CONFLICT(singleton) DO UPDATE SET
                        completed_epoch = excluded.completed_epoch,
                        baseline_generation = excluded.baseline_generation",
                    params![epoch.as_i64(), baseline_generation],
                )
                .map_err(|error| {
                    classify_sql_error(&path, error, "switching clean baseline head")
                })?;
            Some(BaselineHead {
                epoch,
                baseline_generation: baseline_generation as u64,
                accepted_frontier: epoch_row.accepted_frontier,
                projection_generation: epoch_row.projection_generation as u64,
            })
        } else {
            None
        };
        transaction.commit().map_err(|error| {
            classify_sql_error(&path, error, "committing baseline epoch finish")
        })?;
        Ok(head)
    }

    pub(crate) fn apply_tine_completion(
        &mut self,
        completion: &DurablyPublishedProjectionCompletion,
        managed_kind: ManagedTextKind,
        completed_at: BaselineTimestamp,
    ) -> Result<BaselineHead, ReconciliationBaselineError> {
        let state = match completion.target() {
            ProjectionWorkTarget::Present(description) => TineCompletionState::Present(description),
            ProjectionWorkTarget::Absent => TineCompletionState::Deleted,
        };
        self.apply_authenticated_tine_completion(&AuthenticatedTineCompletion {
            workspace: completion.workspace_id(),
            endpoint: completion.endpoint_id(),
            graph_resource: completion.graph_resource_id(),
            path: completion.path(),
            managed_kind,
            state,
            completion_identity: BaselineCompletionIdentity::from_logical(
                completion.logical_completion_id(),
            ),
            completion_frontier: completion.frontier_digest(),
            projection_generation: completion.projection_generation(),
            completed_at,
        })
    }

    fn apply_authenticated_tine_completion(
        &mut self,
        update: &AuthenticatedTineCompletion<'_>,
    ) -> Result<BaselineHead, ReconciliationBaselineError> {
        validate_managed_path(update.path)?;
        if update.workspace != self.binding.workspace
            || update.endpoint != self.binding.endpoint
            || update.graph_resource != self.binding.graph_resource
        {
            return Err(unavailable(
                "durable completion update does not match the baseline binding",
            ));
        }
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let head = load_head(&transaction, &path)?;
        if update.projection_generation <= head.projection_generation {
            return Err(unavailable(
                "durable completion projection generation did not advance",
            ));
        }
        let started_at: i64 = transaction
            .query_row(
                "SELECT started_at FROM epochs WHERE id = ?1 AND state = 1",
                [head.epoch.as_i64()],
                |row| row.get(0),
            )
            .map_err(|error| {
                classify_sql_error(&path, error, "reading direct-update baseline epoch")
            })?;
        if update.completed_at.as_millis() < started_at {
            return Err(unavailable(
                "durable completion timestamp precedes the baseline epoch",
            ));
        }
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT length(exact_path) FROM paths
                 WHERE epoch_id = ?1 AND exact_path = ?2",
                params![head.epoch.as_i64(), update.path.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| classify_sql_error(&path, error, "reading direct-update path"))?;
        if existing.is_none() {
            let (state, count, aggregate) = epoch_write_state(&transaction, &path, head.epoch)?;
            if state != EpochState::Clean {
                return Err(rebuild(&path, "baseline head does not name a clean epoch"));
            }
            let new_count = checked_total(
                count,
                1,
                MAX_BASELINE_PATHS_PER_EPOCH,
                "baseline epoch path",
            )?;
            let new_aggregate =
                checked_aggregate_path_bytes(aggregate, update.path.as_str().len() as u64)?;
            transaction
                .execute(
                    "UPDATE epochs
                     SET path_count = ?2, aggregate_path_bytes = ?3
                     WHERE id = ?1 AND state = 1",
                    params![
                        head.epoch.as_i64(),
                        sqlite_usize(new_count, "path count")?,
                        sqlite_u64(new_aggregate, "aggregate path bytes")?
                    ],
                )
                .map_err(|error| {
                    classify_sql_error(&path, error, "updating direct-completion counters")
                })?;
        }
        let (state, digest, length) = match update.state {
            TineCompletionState::Present(description) => (
                1_i64,
                Some(description.sha256().to_vec()),
                Some(sqlite_u64(description.byte_length(), "blob byte length")?),
            ),
            TineCompletionState::Deleted => (2, None, None),
        };
        transaction
            .execute(
                "INSERT INTO paths (
                    epoch_id, exact_path, managed_kind, state, content_digest,
                    byte_len, file_resource, link_count, source, completion_identity,
                    completion_frontier
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, 2, ?7, ?8)
                 ON CONFLICT(epoch_id, exact_path) DO UPDATE SET
                    managed_kind = excluded.managed_kind,
                    state = excluded.state,
                    content_digest = excluded.content_digest,
                    byte_len = excluded.byte_len,
                    file_resource = NULL,
                    link_count = NULL,
                    source = 2,
                    completion_identity = excluded.completion_identity,
                    completion_frontier = excluded.completion_frontier",
                params![
                    head.epoch.as_i64(),
                    update.path.as_str(),
                    managed_kind_tag(update.managed_kind),
                    state,
                    digest,
                    length,
                    update.completion_identity.as_bytes().as_slice(),
                    update.completion_frontier.as_bytes().as_slice()
                ],
            )
            .map_err(|error| {
                classify_sql_error(&path, error, "recording durable completion path")
            })?;
        let projection_generation =
            sqlite_u64(update.projection_generation, "projection generation")?;
        transaction
            .execute(
                "UPDATE epochs
                 SET accepted_frontier = ?2, projection_generation = ?3
                 WHERE id = ?1 AND state = 1",
                params![
                    head.epoch.as_i64(),
                    digest_blob(head.accepted_frontier),
                    projection_generation
                ],
            )
            .map_err(|error| {
                classify_sql_error(&path, error, "advancing durable completion baseline")
            })?;
        let baseline_generation = head
            .baseline_generation
            .checked_add(1)
            .ok_or_else(|| unavailable("baseline generation overflow"))?;
        transaction
            .execute(
                "UPDATE head SET baseline_generation = ?1 WHERE singleton = 1",
                [sqlite_u64(baseline_generation, "baseline generation")?],
            )
            .map_err(|error| {
                classify_sql_error(&path, error, "advancing baseline head generation")
            })?;
        transaction.commit().map_err(|error| {
            classify_sql_error(&path, error, "committing durable completion baseline")
        })?;
        Ok(BaselineHead {
            epoch: head.epoch,
            baseline_generation,
            accepted_frontier: head.accepted_frontier,
            projection_generation: update.projection_generation,
        })
    }

    pub(crate) fn head(&mut self) -> Result<BaselineHead, ReconciliationBaselineError> {
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_read(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        require_binding(&transaction, &path, &self.binding)?;
        let head = load_head(&transaction, &path)?;
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "closing baseline head read"))?;
        Ok(head)
    }

    /// Read one bounded page from the diagnostic clean baseline. Returned rows
    /// remain non-authoritative and cannot suppress authenticated comparison.
    pub(crate) fn read_head_paths_page(
        &mut self,
        after: Option<&ManagedPath>,
        limit: usize,
    ) -> Result<BaselinePathPage, ReconciliationBaselineError> {
        if limit == 0 || limit > MAX_BASELINE_PAGE_ROWS {
            return Err(unavailable("baseline read page row bound exceeded"));
        }
        if let Some(after) = after {
            validate_managed_path(after)?;
        }
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_read(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        require_binding(&transaction, &path, &self.binding)?;
        let head = load_head(&transaction, &path)?;
        let after = after.map_or("", ManagedPath::as_str);
        let mut statement = transaction
            .prepare(
                "SELECT exact_path, managed_kind, state, content_digest, byte_len,
                        file_resource, link_count, source, completion_identity,
                        completion_frontier
                 FROM paths
                 WHERE epoch_id = ?1 AND exact_path > ?2
                 ORDER BY exact_path
                 LIMIT ?3",
            )
            .map_err(|error| classify_sql_error(&path, error, "preparing baseline path page"))?;
        let mut query = statement
            .query(params![
                head.epoch.as_i64(),
                after,
                sqlite_usize(limit, "page row count")?
            ])
            .map_err(|error| classify_sql_error(&path, error, "querying baseline path page"))?;
        let mut rows = Vec::with_capacity(limit);
        while let Some(row) = query
            .next()
            .map_err(|error| classify_sql_error(&path, error, "reading baseline path page"))?
        {
            rows.push(decode_path_row(row, &path)?);
        }
        drop(query);
        drop(statement);
        let next_after = (rows.len() == limit)
            .then(|| rows.last().expect("full page has a last row").path.clone());
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "closing baseline path page"))?;
        Ok(BaselinePathPage {
            head,
            rows,
            next_after,
        })
    }

    pub(crate) fn record_blocked(
        &mut self,
        observation_digest: ContentDigest,
        reason: BaselineBlockedReason,
        detail: &str,
        observed_at: BaselineTimestamp,
    ) -> Result<BaselineBlockedSignature, ReconciliationBaselineError> {
        if detail.len() > MAX_BASELINE_BLOCKED_REASON_BYTES {
            return Err(unavailable("blocked diagnostic detail byte bound exceeded"));
        }
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_immediate(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        let prior: Option<(i64, String, i64, i64)> = transaction
            .query_row(
                "SELECT reason, detail, first_seen, last_seen
                 FROM blocked WHERE exact_observation_digest = ?1",
                [digest_blob(observation_digest)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| classify_sql_error(&path, error, "reading blocked signature"))?;
        let first_seen = match prior {
            Some((prior_reason, prior_detail, first_seen, last_seen)) => {
                if prior_reason != reason.tag() || prior_detail != detail {
                    return Err(unavailable(
                        "blocked observation digest was reused for different diagnostics",
                    ));
                }
                if observed_at.as_millis() < last_seen {
                    return Err(unavailable("blocked observation timestamp regressed"));
                }
                BaselineTimestamp(first_seen)
            }
            None => {
                let count: i64 = transaction
                    .query_row("SELECT COUNT(*) FROM blocked", [], |row| row.get(0))
                    .map_err(|error| {
                        classify_sql_error(&path, error, "counting blocked signatures")
                    })?;
                if count >= MAX_BASELINE_BLOCKED_SIGNATURES as i64 {
                    return Err(unavailable("blocked signature row bound exceeded"));
                }
                observed_at
            }
        };
        transaction
            .execute(
                "INSERT INTO blocked (
                    exact_observation_digest, reason, detail, first_seen, last_seen
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(exact_observation_digest) DO UPDATE SET
                    last_seen = excluded.last_seen",
                params![
                    digest_blob(observation_digest),
                    reason.tag(),
                    detail,
                    first_seen.as_millis(),
                    observed_at.as_millis()
                ],
            )
            .map_err(|error| classify_sql_error(&path, error, "recording blocked signature"))?;
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "committing blocked signature"))?;
        Ok(BaselineBlockedSignature {
            observation_digest,
            reason,
            detail: detail.to_owned(),
            first_seen,
            last_seen: observed_at,
        })
    }

    pub(crate) fn blocked_signature(
        &mut self,
        observation_digest: ContentDigest,
    ) -> Result<Option<BaselineBlockedSignature>, ReconciliationBaselineError> {
        let path = self.path.clone();
        let trusted_data_version = self.trusted_data_version;
        let transaction = begin_read(&mut self.connection, &path)?;
        require_data_version(&transaction, &path, trusted_data_version)?;
        require_binding(&transaction, &path, &self.binding)?;
        let stored: Option<(i64, String, i64, i64)> = transaction
            .query_row(
                "SELECT reason, detail, first_seen, last_seen
                 FROM blocked WHERE exact_observation_digest = ?1",
                [digest_blob(observation_digest)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| classify_sql_error(&path, error, "reading blocked signature"))?;
        let result = stored
            .map(|(reason, detail, first_seen, last_seen)| {
                if detail.len() > MAX_BASELINE_BLOCKED_REASON_BYTES
                    || first_seen < 0
                    || last_seen < first_seen
                {
                    return Err(rebuild(&path, "malformed blocked signature row"));
                }
                let reason = BaselineBlockedReason::from_tag(reason)
                    .ok_or_else(|| rebuild(&path, "unknown blocked reason tag"))?;
                Ok(BaselineBlockedSignature {
                    observation_digest,
                    reason,
                    detail,
                    first_seen: BaselineTimestamp(first_seen),
                    last_seen: BaselineTimestamp(last_seen),
                })
            })
            .transpose()?;
        transaction
            .commit()
            .map_err(|error| classify_sql_error(&path, error, "closing blocked signature read"))?;
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EpochState {
    Building,
    Clean,
    Blocked,
    Incomplete,
}

impl EpochState {
    const fn tag(self) -> i64 {
        match self {
            Self::Building => 0,
            Self::Clean => 1,
            Self::Blocked => 2,
            Self::Incomplete => 3,
        }
    }

    fn from_tag(tag: i64) -> Option<Self> {
        match tag {
            0 => Some(Self::Building),
            1 => Some(Self::Clean),
            2 => Some(Self::Blocked),
            3 => Some(Self::Incomplete),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct EpochValidation {
    state: EpochState,
    started_at: i64,
    accepted_frontier: ContentDigest,
    projection_generation: i64,
    path_count: usize,
    directory_count: usize,
    aggregate_path_bytes: u64,
}

fn initialize_schema(
    connection: &Connection,
    path: &Path,
    binding: &ReconciliationBaselineBinding,
) -> Result<(), ReconciliationBaselineError> {
    connection
        .execute_batch(&format!(
            "PRAGMA application_id = {RECONCILIATION_BASELINE_APPLICATION_ID};
             PRAGMA user_version = {RECONCILIATION_BASELINE_SCHEMA_VERSION};
             {BINDING_DDL}
             {EPOCHS_DDL}
             {PATHS_DDL}
             {DIRECTORIES_DDL}
             {BLOCKED_DDL}
             {HEAD_DDL}
             {EPOCH_STATE_INDEX_DDL}
             {BLOCKED_LAST_SEEN_INDEX_DDL}"
        ))
        .map_err(|error| classify_sql_error(path, error, "initializing baseline schema"))?;
    connection
        .execute(
            "INSERT INTO binding (
                singleton, schema_version, workspace, endpoint, graph_resource,
                scope_binding, managed_entity_version
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                i64::from(RECONCILIATION_BASELINE_SCHEMA_VERSION),
                binding.workspace.as_uuid().as_bytes().as_slice(),
                binding.endpoint.as_uuid().as_bytes().as_slice(),
                binding.graph_resource.as_bytes().as_slice(),
                binding.scope_binding.canonical_bytes().as_slice(),
                i64::from(MANAGED_ENTITY_SET_VERSION)
            ],
        )
        .map_err(|error| classify_sql_error(path, error, "binding baseline schema"))?;
    Ok(())
}

/// Open stock SQLite by ambient filename.
///
/// The surrounding no-follow checks reduce accidental placement mistakes, but
/// this connection is not capability-anchored and the checks do not close
/// namespace races against a process with the same app-data authority.
fn open_ambient_sqlite_connection(
    path: &Path,
    create: bool,
) -> Result<Connection, ReconciliationBaselineError> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection = Connection::open_with_flags(path, flags)
        .map_err(|error| classify_sql_error(path, error, "opening baseline SQLite"))?;
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| classify_sql_error(path, error, "setting baseline busy timeout"))?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;",
        )
        .map_err(|error| classify_sql_error(path, error, "configuring baseline SQLite"))?;
    if create {
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA wal_autocheckpoint = 1000;",
            )
            .map_err(|error| classify_sql_error(path, error, "configuring new baseline SQLite"))?;
    }
    Ok(connection)
}

fn configure_trusted_connection(
    connection: &Connection,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    connection
        .execute_batch(
            "PRAGMA synchronous = FULL;
             PRAGMA wal_autocheckpoint = 1000;",
        )
        .map_err(|error| classify_sql_error(path, error, "configuring trusted baseline SQLite"))
}

fn validate_database(
    connection: &mut Connection,
    path: &Path,
    binding: &ReconciliationBaselineBinding,
) -> Result<i64, ReconciliationBaselineError> {
    validate_database_with_after_commit_hook(connection, path, binding, || {})
}

fn validate_database_with_after_commit_hook(
    connection: &mut Connection,
    path: &Path,
    binding: &ReconciliationBaselineBinding,
    after_commit: impl FnOnce(),
) -> Result<i64, ReconciliationBaselineError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| classify_sql_error(path, error, "starting baseline validation"))?;
    let quick_check: String = transaction
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "checking baseline integrity"))?;
    if quick_check != "ok" {
        return Err(rebuild(
            path,
            format!("SQLite quick_check failed: {quick_check}"),
        ));
    }
    validate_schema(&transaction, path)?;
    require_binding(&transaction, path, binding)?;
    validate_rows(&transaction, path, binding)?;
    let trusted_data_version = transaction
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(|error| {
            classify_sql_error(path, error, "binding baseline validation data version")
        })?;
    transaction
        .commit()
        .map_err(|error| classify_sql_error(path, error, "closing baseline validation"))?;
    after_commit();
    Ok(trusted_data_version)
}

fn validate_schema(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    let application_id: u32 = transaction
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "reading baseline application id"))?;
    if application_id != RECONCILIATION_BASELINE_APPLICATION_ID {
        return Err(rebuild(
            path,
            format!(
                "unknown application id {application_id:#x}; expected {RECONCILIATION_BASELINE_APPLICATION_ID:#x}"
            ),
        ));
    }
    let user_version: u32 = transaction
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "reading baseline schema version"))?;
    if user_version != RECONCILIATION_BASELINE_SCHEMA_VERSION {
        return Err(rebuild(
            path,
            format!(
                "unknown schema version {user_version}; expected {RECONCILIATION_BASELINE_SCHEMA_VERSION}"
            ),
        ));
    }
    let foreign_keys: i64 = transaction
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "reading foreign-key mode"))?;
    if foreign_keys != 1 {
        return Err(rebuild(path, "baseline foreign keys are disabled"));
    }
    let journal_mode: String = transaction
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "reading baseline journal mode"))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(rebuild(path, "baseline journal mode is not WAL"));
    }
    let object_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| classify_sql_error(path, error, "counting baseline schema objects"))?;
    if object_count > MAX_SCHEMA_OBJECTS {
        return Err(rebuild(path, "baseline schema object bound exceeded"));
    }
    let tables = schema_names(transaction, path, "table")?;
    let indexes = schema_names(transaction, path, "index")?;
    let expected_tables: BTreeSet<String> = EXPECTED_TABLES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let expected_indexes: BTreeSet<String> = EXPECTED_INDEXES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    if tables != expected_tables || indexes != expected_indexes {
        return Err(rebuild(path, "baseline SQLite schema objects do not match"));
    }
    for (kind, name, expected) in [
        ("table", "binding", BINDING_DDL),
        ("table", "epochs", EPOCHS_DDL),
        ("table", "paths", PATHS_DDL),
        ("table", "directories", DIRECTORIES_DDL),
        ("table", "blocked", BLOCKED_DDL),
        ("table", "head", HEAD_DDL),
        ("index", "epochs_state_id_idx", EPOCH_STATE_INDEX_DDL),
        (
            "index",
            "blocked_last_seen_idx",
            BLOCKED_LAST_SEEN_INDEX_DDL,
        ),
    ] {
        require_schema_sql(transaction, path, kind, name, expected)?;
    }
    let other_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%' AND type NOT IN ('table', 'index')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| classify_sql_error(path, error, "checking unknown schema objects"))?;
    if other_count != 0 {
        return Err(rebuild(path, "baseline contains unknown views or triggers"));
    }
    Ok(())
}

fn require_schema_sql(
    transaction: &Transaction<'_>,
    path: &Path,
    kind: &str,
    name: &str,
    expected: &str,
) -> Result<(), ReconciliationBaselineError> {
    let stored: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
            params![kind, name],
            |row| row.get(0),
        )
        .map_err(|error| classify_sql_error(path, error, "reading baseline schema definition"))?;
    if normalized_schema_sql(&stored) != normalized_schema_sql(expected) {
        return Err(rebuild(
            path,
            format!("baseline schema definition for {name} does not match"),
        ));
    }
    Ok(())
}

fn normalized_schema_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn schema_names(
    transaction: &Transaction<'_>,
    path: &Path,
    kind: &str,
) -> Result<BTreeSet<String>, ReconciliationBaselineError> {
    let mut statement = transaction
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = ?1 AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .map_err(|error| classify_sql_error(path, error, "preparing schema validation"))?;
    let mut rows = statement
        .query([kind])
        .map_err(|error| classify_sql_error(path, error, "querying schema validation"))?;
    let mut names = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| classify_sql_error(path, error, "reading schema validation"))?
    {
        if names.len() >= MAX_SCHEMA_OBJECTS as usize {
            return Err(rebuild(path, "baseline schema object bound exceeded"));
        }
        names.insert(
            row.get(0)
                .map_err(|error| classify_sql_error(path, error, "decoding schema name"))?,
        );
    }
    Ok(names)
}

fn require_binding(
    transaction: &Transaction<'_>,
    path: &Path,
    expected: &ReconciliationBaselineBinding,
) -> Result<(), ReconciliationBaselineError> {
    type StoredBinding = (i64, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, i64);
    let count: i64 = transaction
        .query_row("SELECT COUNT(*) FROM binding", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "counting baseline binding rows"))?;
    if count != 1 {
        return Err(rebuild(
            path,
            "baseline must contain exactly one binding row",
        ));
    }
    let stored: StoredBinding = transaction
        .query_row(
            "SELECT schema_version, workspace, endpoint, graph_resource,
                    scope_binding, managed_entity_version
             FROM binding WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|error| classify_sql_error(path, error, "reading baseline binding"))?;
    let scope_binding = GraphTextScopeBinding::from_canonical_bytes(&stored.4)
        .map_err(|error| rebuild(path, format!("malformed scope binding: {error}")))?;
    if stored.0 != i64::from(RECONCILIATION_BASELINE_SCHEMA_VERSION)
        || stored.1.as_slice() != expected.workspace.as_uuid().as_bytes()
        || stored.2.as_slice() != expected.endpoint.as_uuid().as_bytes()
        || stored.3.as_slice() != expected.graph_resource.as_bytes()
        || scope_binding != expected.scope_binding
        || scope_binding.graph_resource_id() != expected.graph_resource
        || stored.5 != i64::from(MANAGED_ENTITY_SET_VERSION)
    {
        return Err(rebuild(
            path,
            "baseline binding mismatch or graph-resource substitution",
        ));
    }
    Ok(())
}

fn validate_rows(
    transaction: &Transaction<'_>,
    path: &Path,
    binding: &ReconciliationBaselineBinding,
) -> Result<(), ReconciliationBaselineError> {
    require_table_bound(transaction, path, "epochs", MAX_BASELINE_EPOCHS)?;
    require_table_bound(
        transaction,
        path,
        "paths",
        MAX_BASELINE_EPOCHS.saturating_mul(MAX_BASELINE_PATHS_PER_EPOCH),
    )?;
    require_table_bound(
        transaction,
        path,
        "directories",
        MAX_BASELINE_EPOCHS.saturating_mul(MAX_BASELINE_DIRECTORIES_PER_EPOCH),
    )?;
    require_table_bound(
        transaction,
        path,
        "blocked",
        MAX_BASELINE_BLOCKED_SIGNATURES,
    )?;
    require_table_bound(transaction, path, "head", 1)?;

    let mut epochs = validate_epochs(transaction, path)?;
    validate_path_rows(transaction, path, &mut epochs)?;
    validate_directory_rows(transaction, path, binding, &mut epochs)?;
    validate_epoch_counts(path, &epochs)?;
    validate_blocked_rows(transaction, path)?;
    validate_head_row(transaction, path, &epochs)?;
    Ok(())
}

fn require_table_bound(
    transaction: &Transaction<'_>,
    path: &Path,
    table: &str,
    maximum: usize,
) -> Result<(), ReconciliationBaselineError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = transaction
        .query_row(&sql, [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "counting baseline rows"))?;
    if count < 0 || count as u128 > maximum as u128 {
        return Err(rebuild(
            path,
            format!("baseline {table} row bound exceeded"),
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct ValidatedEpoch {
    validation: EpochValidation,
    stored_path_count: usize,
    stored_directory_count: usize,
    stored_aggregate_bytes: u64,
    actual_path_count: usize,
    actual_directory_count: usize,
    actual_aggregate_bytes: u64,
}

fn validate_epochs(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<BTreeMap<i64, ValidatedEpoch>, ReconciliationBaselineError> {
    let mut statement = transaction
        .prepare(
            "SELECT id, state, started_at, completed_at, accepted_frontier,
                    projection_generation, pass_a_digest, pass_b_digest,
                    candidate_digest, candidate_count, path_count,
                    directory_count, aggregate_path_bytes,
                    scan_passes, scan_directory_entries, scan_directories,
                    scan_regular_files, scan_eligible_files, scan_bytes_read,
                    scan_bytes_hashed, scan_peak_retained_rows,
                    scan_peak_retained_bytes, scan_candidates,
                    scan_diagnostics, scan_wall_time_ms
             FROM epochs ORDER BY id",
        )
        .map_err(|error| classify_sql_error(path, error, "preparing epoch validation"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| classify_sql_error(path, error, "querying epochs"))?;
    let mut epochs = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| classify_sql_error(path, error, "reading epoch row"))?
    {
        let id: i64 = row
            .get(0)
            .map_err(|error| classify_sql_error(path, error, "decoding epoch id"))?;
        let state_tag: i64 = row
            .get(1)
            .map_err(|error| classify_sql_error(path, error, "decoding epoch state"))?;
        let started_at: i64 = row
            .get(2)
            .map_err(|error| classify_sql_error(path, error, "decoding epoch start"))?;
        let completed_at: Option<i64> = row
            .get(3)
            .map_err(|error| classify_sql_error(path, error, "decoding epoch completion"))?;
        let accepted_frontier = decode_digest_value(row.get(4), path, "accepted frontier")?;
        let projection_generation: i64 = row
            .get(5)
            .map_err(|error| classify_sql_error(path, error, "decoding projection generation"))?;
        let pass_a: Option<Vec<u8>> = row
            .get(6)
            .map_err(|error| classify_sql_error(path, error, "decoding pass A digest"))?;
        let pass_b: Option<Vec<u8>> = row
            .get(7)
            .map_err(|error| classify_sql_error(path, error, "decoding pass B digest"))?;
        let candidate: Option<Vec<u8>> = row
            .get(8)
            .map_err(|error| classify_sql_error(path, error, "decoding candidate digest"))?;
        let candidate_count = decode_bounded_usize(
            row.get(9),
            MAX_BASELINE_PATHS_PER_EPOCH,
            path,
            "candidate count",
        )?;
        let path_count = decode_bounded_usize(
            row.get(10),
            MAX_BASELINE_PATHS_PER_EPOCH,
            path,
            "path count",
        )?;
        let directory_count = decode_bounded_usize(
            row.get(11),
            MAX_BASELINE_DIRECTORIES_PER_EPOCH,
            path,
            "directory count",
        )?;
        let aggregate_path_bytes = decode_bounded_u64(
            row.get(12),
            MAX_BASELINE_AGGREGATE_PATH_BYTES,
            path,
            "path bytes",
        )?;
        let mut stored_instrumentation = Vec::with_capacity(12);
        for index in 13..25 {
            stored_instrumentation.push(row.get::<_, Option<i64>>(index).map_err(|error| {
                classify_sql_error(path, error, "decoding scan instrumentation")
            })?);
        }
        let state = EpochState::from_tag(state_tag)
            .ok_or_else(|| rebuild(path, "unknown baseline epoch state"))?;
        if id <= 0 || started_at < 0 || projection_generation < 0 {
            return Err(rebuild(
                path,
                "impossible baseline epoch identity or generation",
            ));
        }
        match state {
            EpochState::Building => {
                if completed_at.is_some()
                    || pass_a.is_some()
                    || pass_b.is_some()
                    || candidate.is_some()
                    || candidate_count != 0
                    || stored_instrumentation.iter().any(Option::is_some)
                {
                    return Err(rebuild(path, "building epoch contains completed evidence"));
                }
            }
            EpochState::Clean | EpochState::Blocked | EpochState::Incomplete => {
                let completed_at = completed_at
                    .ok_or_else(|| rebuild(path, "finished epoch lacks completion timestamp"))?;
                if completed_at < started_at {
                    return Err(rebuild(path, "epoch completion precedes epoch start"));
                }
                let pass_a = decode_optional_digest(pass_a, path, "pass A digest")?
                    .ok_or_else(|| rebuild(path, "finished epoch lacks pass A digest"))?;
                let pass_b = decode_optional_digest(pass_b, path, "pass B digest")?
                    .ok_or_else(|| rebuild(path, "finished epoch lacks pass B digest"))?;
                if decode_optional_digest(candidate, path, "candidate digest")?.is_none() {
                    return Err(rebuild(path, "finished epoch lacks candidate digest"));
                }
                if state == EpochState::Clean && pass_a != pass_b {
                    return Err(rebuild(path, "clean epoch contains unstable scan evidence"));
                }
                let instrumentation =
                    decode_stored_instrumentation(&stored_instrumentation, path, candidate_count)?;
                validate_scan_instrumentation(
                    &instrumentation,
                    candidate_count,
                    state == EpochState::Clean,
                )
                .map_err(|error| {
                    rebuild(
                        path,
                        format!("malformed stored scan instrumentation: {error}"),
                    )
                })?;
            }
        }
        let validation = EpochValidation {
            state,
            started_at,
            accepted_frontier,
            projection_generation,
            path_count,
            directory_count,
            aggregate_path_bytes,
        };
        if epochs
            .insert(
                id,
                ValidatedEpoch {
                    validation,
                    stored_path_count: path_count,
                    stored_directory_count: directory_count,
                    stored_aggregate_bytes: aggregate_path_bytes,
                    actual_path_count: 0,
                    actual_directory_count: 0,
                    actual_aggregate_bytes: 0,
                },
            )
            .is_some()
        {
            return Err(rebuild(path, "duplicate epoch identity"));
        }
    }
    Ok(epochs)
}

fn validate_path_rows(
    transaction: &Transaction<'_>,
    path: &Path,
    epochs: &mut BTreeMap<i64, ValidatedEpoch>,
) -> Result<(), ReconciliationBaselineError> {
    let mut statement = transaction
        .prepare(
            "SELECT epoch_id, exact_path, managed_kind, state, content_digest,
                    byte_len, file_resource, link_count, source, completion_identity,
                    completion_frontier
             FROM paths ORDER BY epoch_id, exact_path",
        )
        .map_err(|error| classify_sql_error(path, error, "preparing baseline path validation"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| classify_sql_error(path, error, "querying baseline paths"))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| classify_sql_error(path, error, "reading baseline path row"))?
    {
        let epoch_id: i64 = row
            .get(0)
            .map_err(|error| classify_sql_error(path, error, "decoding path epoch"))?;
        let record = decode_path_row_at(row, 1, path)?;
        let epoch = epochs
            .get_mut(&epoch_id)
            .ok_or_else(|| rebuild(path, "path row names a missing epoch"))?;
        epoch.actual_path_count = epoch
            .actual_path_count
            .checked_add(1)
            .ok_or_else(|| rebuild(path, "path row count overflow"))?;
        epoch.actual_aggregate_bytes = epoch
            .actual_aggregate_bytes
            .checked_add(record.path.as_str().len() as u64)
            .ok_or_else(|| rebuild(path, "aggregate path bytes overflow"))?;
    }
    Ok(())
}

fn validate_directory_rows(
    transaction: &Transaction<'_>,
    path: &Path,
    binding: &ReconciliationBaselineBinding,
    epochs: &mut BTreeMap<i64, ValidatedEpoch>,
) -> Result<(), ReconciliationBaselineError> {
    let mut statement = transaction
        .prepare(
            "SELECT epoch_id, exact_path, resource
             FROM directories ORDER BY epoch_id, exact_path",
        )
        .map_err(|error| {
            classify_sql_error(path, error, "preparing baseline directory validation")
        })?;
    let mut rows = statement
        .query([])
        .map_err(|error| classify_sql_error(path, error, "querying baseline directories"))?;
    let mut roots = BTreeSet::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| classify_sql_error(path, error, "reading baseline directory row"))?
    {
        let epoch_id: i64 = row
            .get(0)
            .map_err(|error| classify_sql_error(path, error, "decoding directory epoch"))?;
        let exact: String = row
            .get(1)
            .map_err(|error| classify_sql_error(path, error, "decoding directory path"))?;
        let directory = BaselineDirectoryPath::parse(exact)
            .map_err(|error| rebuild(path, format!("malformed directory row: {error}")))?;
        let resource = decode_digest_value(row.get(2), path, "directory resource")?;
        let epoch = epochs
            .get_mut(&epoch_id)
            .ok_or_else(|| rebuild(path, "directory row names a missing epoch"))?;
        epoch.actual_directory_count = epoch
            .actual_directory_count
            .checked_add(1)
            .ok_or_else(|| rebuild(path, "directory row count overflow"))?;
        epoch.actual_aggregate_bytes = epoch
            .actual_aggregate_bytes
            .checked_add(directory.as_str().len() as u64)
            .ok_or_else(|| rebuild(path, "aggregate path bytes overflow"))?;
        if directory.as_str().is_empty() {
            if resource.as_bytes() != binding.graph_resource.as_bytes() || !roots.insert(epoch_id) {
                return Err(rebuild(path, "graph-resource substitution in epoch root"));
            }
        }
    }
    for (id, epoch) in epochs {
        if epoch.validation.state == EpochState::Clean && !roots.contains(id) {
            return Err(rebuild(path, "clean epoch lacks its retained graph root"));
        }
    }
    Ok(())
}

fn validate_epoch_counts(
    path: &Path,
    epochs: &BTreeMap<i64, ValidatedEpoch>,
) -> Result<(), ReconciliationBaselineError> {
    for epoch in epochs.values() {
        if epoch.stored_path_count != epoch.actual_path_count
            || epoch.stored_directory_count != epoch.actual_directory_count
            || epoch.stored_aggregate_bytes != epoch.actual_aggregate_bytes
            || epoch.actual_aggregate_bytes > MAX_BASELINE_AGGREGATE_PATH_BYTES
        {
            return Err(rebuild(path, "baseline epoch counters do not match rows"));
        }
    }
    Ok(())
}

fn validate_blocked_rows(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    let mut statement = transaction
        .prepare(
            "SELECT exact_observation_digest, reason, detail, first_seen, last_seen
             FROM blocked ORDER BY exact_observation_digest",
        )
        .map_err(|error| classify_sql_error(path, error, "preparing blocked-row validation"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| classify_sql_error(path, error, "querying blocked rows"))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| classify_sql_error(path, error, "reading blocked row"))?
    {
        let _: ContentDigest = decode_digest_value(row.get(0), path, "blocked observation")?;
        let reason: i64 = row
            .get(1)
            .map_err(|error| classify_sql_error(path, error, "decoding blocked reason"))?;
        let detail: String = row
            .get(2)
            .map_err(|error| classify_sql_error(path, error, "decoding blocked detail"))?;
        let first_seen: i64 = row
            .get(3)
            .map_err(|error| classify_sql_error(path, error, "decoding blocked first seen"))?;
        let last_seen: i64 = row
            .get(4)
            .map_err(|error| classify_sql_error(path, error, "decoding blocked last seen"))?;
        if BaselineBlockedReason::from_tag(reason).is_none()
            || detail.len() > MAX_BASELINE_BLOCKED_REASON_BYTES
            || first_seen < 0
            || last_seen < first_seen
        {
            return Err(rebuild(path, "malformed blocked signature row"));
        }
    }
    Ok(())
}

fn validate_head_row(
    transaction: &Transaction<'_>,
    path: &Path,
    epochs: &BTreeMap<i64, ValidatedEpoch>,
) -> Result<(), ReconciliationBaselineError> {
    let head: Option<(i64, i64)> = transaction
        .query_row(
            "SELECT completed_epoch, baseline_generation
             FROM head WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| classify_sql_error(path, error, "validating baseline head"))?;
    if let Some((epoch, generation)) = head {
        let epoch = epochs
            .get(&epoch)
            .ok_or_else(|| rebuild(path, "baseline head names a missing epoch"))?;
        if generation <= 0 || epoch.validation.state != EpochState::Clean {
            return Err(rebuild(
                path,
                "baseline head is not a possible clean generation",
            ));
        }
    }
    Ok(())
}

fn load_epoch_for_finish(
    transaction: &Transaction<'_>,
    path: &Path,
    epoch: BaselineEpochId,
) -> Result<EpochValidation, ReconciliationBaselineError> {
    transaction
        .query_row(
            "SELECT state, started_at, accepted_frontier, projection_generation,
                    path_count, directory_count, aggregate_path_bytes
             FROM epochs WHERE id = ?1",
            [epoch.as_i64()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify_sql_error(path, error, "reading baseline epoch"))?
        .ok_or_else(|| unavailable("baseline epoch does not exist"))
        .and_then(
            |(state, started_at, frontier, generation, paths, directories, bytes)| {
                Ok(EpochValidation {
                    state: EpochState::from_tag(state)
                        .ok_or_else(|| rebuild(path, "unknown baseline epoch state"))?,
                    started_at,
                    accepted_frontier: decode_digest_bytes(&frontier, path, "accepted frontier")?,
                    projection_generation: require_nonnegative(generation, path, "generation")?,
                    path_count: decode_bounded_usize(
                        Ok(paths),
                        MAX_BASELINE_PATHS_PER_EPOCH,
                        path,
                        "path count",
                    )?,
                    directory_count: decode_bounded_usize(
                        Ok(directories),
                        MAX_BASELINE_DIRECTORIES_PER_EPOCH,
                        path,
                        "directory count",
                    )?,
                    aggregate_path_bytes: decode_bounded_u64(
                        Ok(bytes),
                        MAX_BASELINE_AGGREGATE_PATH_BYTES,
                        path,
                        "aggregate path bytes",
                    )?,
                })
            },
        )
}

fn epoch_write_state(
    transaction: &Transaction<'_>,
    path: &Path,
    epoch: BaselineEpochId,
) -> Result<(EpochState, usize, u64), ReconciliationBaselineError> {
    transaction
        .query_row(
            "SELECT state, path_count, aggregate_path_bytes
             FROM epochs WHERE id = ?1",
            [epoch.as_i64()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify_sql_error(path, error, "reading epoch path counters"))?
        .ok_or_else(|| unavailable("baseline epoch does not exist"))
        .and_then(|(state, count, bytes)| {
            Ok((
                EpochState::from_tag(state)
                    .ok_or_else(|| rebuild(path, "unknown baseline epoch state"))?,
                decode_bounded_usize(Ok(count), MAX_BASELINE_PATHS_PER_EPOCH, path, "path count")?,
                decode_bounded_u64(
                    Ok(bytes),
                    MAX_BASELINE_AGGREGATE_PATH_BYTES,
                    path,
                    "path bytes",
                )?,
            ))
        })
}

fn epoch_directory_write_state(
    transaction: &Transaction<'_>,
    path: &Path,
    epoch: BaselineEpochId,
) -> Result<(EpochState, usize, u64), ReconciliationBaselineError> {
    transaction
        .query_row(
            "SELECT state, directory_count, aggregate_path_bytes
             FROM epochs WHERE id = ?1",
            [epoch.as_i64()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify_sql_error(path, error, "reading epoch directory counters"))?
        .ok_or_else(|| unavailable("baseline epoch does not exist"))
        .and_then(|(state, count, bytes)| {
            Ok((
                EpochState::from_tag(state)
                    .ok_or_else(|| rebuild(path, "unknown baseline epoch state"))?,
                decode_bounded_usize(
                    Ok(count),
                    MAX_BASELINE_DIRECTORIES_PER_EPOCH,
                    path,
                    "directory count",
                )?,
                decode_bounded_u64(
                    Ok(bytes),
                    MAX_BASELINE_AGGREGATE_PATH_BYTES,
                    path,
                    "path bytes",
                )?,
            ))
        })
}

fn require_epoch_counters_match(
    transaction: &Transaction<'_>,
    path: &Path,
    epoch: BaselineEpochId,
    stored: &EpochValidation,
) -> Result<(), ReconciliationBaselineError> {
    let (path_count, directory_count, path_bytes, directory_bytes): (i64, i64, i64, i64) =
        transaction
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM paths WHERE epoch_id = ?1),
                    (SELECT COUNT(*) FROM directories WHERE epoch_id = ?1),
                    COALESCE((SELECT SUM(length(CAST(exact_path AS BLOB)))
                              FROM paths WHERE epoch_id = ?1), 0),
                    COALESCE((SELECT SUM(length(CAST(exact_path AS BLOB)))
                              FROM directories WHERE epoch_id = ?1), 0)",
                [epoch.as_i64()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| classify_sql_error(path, error, "checking epoch row counters"))?;
    let aggregate = path_bytes
        .checked_add(directory_bytes)
        .ok_or_else(|| rebuild(path, "epoch path byte sum overflow"))?;
    if path_count != stored.path_count as i64
        || directory_count != stored.directory_count as i64
        || aggregate < 0
        || aggregate as u64 != stored.aggregate_path_bytes
    {
        return Err(rebuild(path, "baseline epoch counters do not match rows"));
    }
    Ok(())
}

fn require_epoch_root_binding(
    transaction: &Transaction<'_>,
    path: &Path,
    epoch: BaselineEpochId,
    binding: &ReconciliationBaselineBinding,
) -> Result<(), ReconciliationBaselineError> {
    let roots: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM directories
             WHERE epoch_id = ?1 AND exact_path = '' AND resource = ?2",
            params![epoch.as_i64(), binding.graph_resource.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .map_err(|error| classify_sql_error(path, error, "checking epoch graph root"))?;
    if roots != 1 {
        return Err(unavailable(
            "a clean epoch must retain the exact bound graph resource",
        ));
    }
    Ok(())
}

fn load_head(
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<BaselineHead, ReconciliationBaselineError> {
    let stored: Option<(i64, i64, Vec<u8>, i64, i64)> = transaction
        .query_row(
            "SELECT h.completed_epoch, h.baseline_generation,
                    e.accepted_frontier, e.projection_generation, e.state
             FROM head h JOIN epochs e ON e.id = h.completed_epoch
             WHERE h.singleton = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify_sql_error(path, error, "reading clean baseline head"))?;
    let Some((epoch, baseline_generation, frontier, projection_generation, state)) = stored else {
        return Err(unavailable("no clean reconciliation baseline head exists"));
    };
    if epoch <= 0 || baseline_generation <= 0 || projection_generation < 0 || state != 1 {
        return Err(rebuild(path, "impossible clean baseline head"));
    }
    Ok(BaselineHead {
        epoch: BaselineEpochId(epoch),
        baseline_generation: baseline_generation as u64,
        accepted_frontier: decode_digest_bytes(&frontier, path, "accepted frontier")?,
        projection_generation: projection_generation as u64,
    })
}

fn decode_path_row(
    row: &rusqlite::Row<'_>,
    path: &Path,
) -> Result<BaselinePathRecord, ReconciliationBaselineError> {
    decode_path_row_at(row, 0, path)
}

fn decode_path_row_at(
    row: &rusqlite::Row<'_>,
    offset: usize,
    path: &Path,
) -> Result<BaselinePathRecord, ReconciliationBaselineError> {
    let exact: String = row
        .get(offset)
        .map_err(|error| classify_sql_error(path, error, "decoding baseline exact path"))?;
    let managed = ManagedPath::parse(exact)
        .map_err(|error| rebuild(path, format!("malformed managed path row: {error}")))?;
    validate_managed_path(&managed)
        .map_err(|error| rebuild(path, format!("malformed managed path row: {error}")))?;
    let kind_tag: Option<i64> = row
        .get(offset + 1)
        .map_err(|error| classify_sql_error(path, error, "decoding managed kind"))?;
    let managed_kind = kind_tag
        .map(|tag| {
            managed_kind_from_tag(tag)
                .ok_or_else(|| rebuild(path, "unknown managed text kind in baseline"))
        })
        .transpose()?;
    let state: i64 = row
        .get(offset + 2)
        .map_err(|error| classify_sql_error(path, error, "decoding baseline path state"))?;
    let content: Option<Vec<u8>> = row
        .get(offset + 3)
        .map_err(|error| classify_sql_error(path, error, "decoding content digest"))?;
    let byte_len: Option<i64> = row
        .get(offset + 4)
        .map_err(|error| classify_sql_error(path, error, "decoding content length"))?;
    let resource: Option<Vec<u8>> = row
        .get(offset + 5)
        .map_err(|error| classify_sql_error(path, error, "decoding file resource"))?;
    let link_count: Option<i64> = row
        .get(offset + 6)
        .map_err(|error| classify_sql_error(path, error, "decoding link count"))?;
    let source: i64 = row
        .get(offset + 7)
        .map_err(|error| classify_sql_error(path, error, "decoding baseline path source"))?;
    let completion: Option<Vec<u8>> = row
        .get(offset + 8)
        .map_err(|error| classify_sql_error(path, error, "decoding completion identity"))?;
    let completion_frontier: Option<Vec<u8>> = row
        .get(offset + 9)
        .map_err(|error| classify_sql_error(path, error, "decoding completion frontier"))?;
    let (recorded_state, file_resource, link_count) =
        match (state, content, byte_len, resource, link_count) {
            (1, Some(content), Some(length), resource, links) if length >= 0 => {
                let digest = decode_fixed_32(&content, path, "content digest")?;
                let resource = resource
                    .map(|bytes| {
                        decode_fixed_32(&bytes, path, "file resource")
                            .map(ContentDigest::from_bytes)
                    })
                    .transpose()?;
                let links = links
                    .map(|links| {
                        if links < 0 {
                            Err(rebuild(path, "negative file link count"))
                        } else {
                            Ok(links as u64)
                        }
                    })
                    .transpose()?;
                (
                    BaselineRecordedState::Present(BlobDescription::from_parts(
                        digest,
                        length as u64,
                    )),
                    resource,
                    links,
                )
            }
            (2, None, None, None, None) => (BaselineRecordedState::ExpectedAbsent, None, None),
            _ => return Err(rebuild(path, "malformed baseline path state")),
        };
    let source = match (source, completion, completion_frontier) {
        (1, None, None) => {
            if matches!(recorded_state, BaselineRecordedState::Present(_))
                && (file_resource.is_none() || link_count != Some(1))
            {
                return Err(rebuild(
                    path,
                    "stable-scan row lacks unambiguous file resource evidence",
                ));
            }
            BaselinePathSource::StableScan
        }
        (2, Some(completion), Some(frontier)) if managed_kind.is_some() => {
            decode_fixed_32(&frontier, path, "completion frontier")?;
            BaselinePathSource::TineCompletion(BaselineCompletionIdentity(decode_fixed_32(
                &completion,
                path,
                "completion identity",
            )?))
        }
        _ => return Err(rebuild(path, "malformed baseline path source")),
    };
    Ok(BaselinePathRecord {
        path: managed,
        managed_kind,
        state: recorded_state,
        file_resource,
        link_count,
        source,
    })
}

fn prepare_database_parent(
    trusted_runtime_root: &TrustedPrivateApplicationRuntimeRoot,
    binding: &ReconciliationBaselineBinding,
    create: bool,
) -> Result<(Dir, PathBuf), ReconciliationBaselineError> {
    let root = Dir::open_ambient_dir(trusted_runtime_root.path(), ambient_authority())
        .map_err(|error| unavailable(format!("cannot retain application runtime root: {error}")))?;
    let open_component = |parent: &Dir, name: &str| {
        if create {
            ensure_and_open(parent, name)
        } else {
            open_existing_directory(parent, name)
        }
    };
    let reconciliation = open_component(&root, RECONCILIATION_DIRECTORY)?;
    let workspace_name = binding.workspace.to_string();
    let workspace = open_component(&reconciliation, &workspace_name)?;
    let endpoint_name = binding.endpoint.to_string();
    let endpoint = open_component(&workspace, &endpoint_name)?;
    let path = trusted_runtime_root
        .path()
        .join(RECONCILIATION_DIRECTORY)
        .join(workspace_name)
        .join(endpoint_name)
        .join(DATABASE_FILE);
    Ok((endpoint, path))
}

fn ensure_and_open(parent: &Dir, name: &str) -> Result<Dir, ReconciliationBaselineError> {
    ensure_directory_nofollow(parent, name).map_err(|error| {
        unavailable(format!("cannot create baseline directory {name}: {error}"))
    })?;
    open_dir_nofollow(parent, name)
        .map_err(|error| unavailable(format!("baseline directory {name} is unsafe: {error}")))
}

fn open_existing_directory(parent: &Dir, name: &str) -> Result<Dir, ReconciliationBaselineError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(unavailable(format!(
                "baseline directory {name} is not a real no-follow directory"
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(unavailable("reconciliation baseline does not exist"));
        }
        Err(error) => {
            return Err(unavailable(format!(
                "cannot inspect baseline directory {name}: {error}"
            )));
        }
    }
    open_dir_nofollow(parent, name)
        .map_err(|error| unavailable(format!("baseline directory {name} is unsafe: {error}")))
}

fn create_database_file_nofollow(
    parent: &Dir,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    let mut options = CapOpenOptions::new();
    options.read(true).write(true).create_new(true);
    match parent.open_with(DATABASE_FILE, &options) {
        Ok(file) => {
            let metadata = file
                .metadata()
                .map_err(|error| unavailable(format!("cannot inspect new baseline: {error}")))?;
            if !metadata.is_file() {
                return Err(unavailable("new baseline path is not a regular file"));
            }
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(unavailable(format!(
            "{} already exists; preserve it and request an explicit fresh create",
            path.display()
        ))),
        Err(error) => Err(unavailable(format!(
            "cannot create baseline {} without following links: {error}",
            path.display()
        ))),
    }
}

fn require_vacant_database_namespace(
    parent: &Dir,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    require_namespace_entry_absent(parent, path, DATABASE_FILE)?;
    for sidecar in DATABASE_SIDECAR_FILES {
        require_namespace_entry_absent(parent, path, sidecar)?;
    }
    Ok(())
}

fn require_namespace_entry_absent(
    parent: &Dir,
    path: &Path,
    name: &str,
) -> Result<(), ReconciliationBaselineError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) => {
            let kind = if metadata_is_link_or_reparse(&metadata) {
                "link or reparse point"
            } else if metadata.is_file() {
                "file"
            } else {
                "unsupported file type"
            };
            Err(unavailable(format!(
                "{} already has a {kind} at {name}; preserve it and request an explicit fresh create",
                path.display()
            )))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(unavailable(format!(
            "cannot inspect fresh baseline namespace entry {name}: {error}"
        ))),
    }
}

fn require_existing_regular(parent: &Dir, path: &Path) -> Result<(), ReconciliationBaselineError> {
    match parent.symlink_metadata(DATABASE_FILE) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() => Err(
            rebuild(path, "baseline path is not a regular no-follow file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(unavailable("reconciliation baseline does not exist"))
        }
        Err(error) => Err(unavailable(format!(
            "cannot inspect reconciliation baseline: {error}"
        ))),
    }
}

fn require_safe_sqlite_sidecars(
    parent: &Dir,
    path: &Path,
) -> Result<(), ReconciliationBaselineError> {
    for sidecar in DATABASE_SIDECAR_FILES {
        match parent.symlink_metadata(sidecar) {
            Ok(metadata) if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() => {
                return Err(rebuild(
                    path,
                    format!("baseline SQLite sidecar {sidecar} has an unsupported file type"),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(unavailable(format!(
                    "cannot inspect baseline SQLite sidecar {sidecar}: {error}"
                )));
            }
        }
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &cap_std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt as _;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn begin_immediate<'a>(
    connection: &'a mut Connection,
    path: &Path,
) -> Result<Transaction<'a>, ReconciliationBaselineError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| classify_sql_error(path, error, "acquiring baseline writer"))
}

fn begin_read<'a>(
    connection: &'a mut Connection,
    path: &Path,
) -> Result<Transaction<'a>, ReconciliationBaselineError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(|error| classify_sql_error(path, error, "starting baseline read"))
}

fn require_data_version(
    transaction: &Transaction<'_>,
    path: &Path,
    trusted: i64,
) -> Result<(), ReconciliationBaselineError> {
    let current: i64 = transaction
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(|error| classify_sql_error(path, error, "checking baseline data version"))?;
    if current != trusted {
        return Err(rebuild(
            path,
            "baseline changed through another connection after validation",
        ));
    }
    Ok(())
}

fn validate_write_batch(rows: usize, resource: &str) -> Result<(), ReconciliationBaselineError> {
    if rows == 0 || rows > MAX_BASELINE_WRITE_ROWS {
        Err(unavailable(format!(
            "{resource} write batch bound exceeded"
        )))
    } else {
        Ok(())
    }
}

fn validate_scan_instrumentation(
    instrumentation: &BaselineScanInstrumentation,
    candidate_count: usize,
    clean: bool,
) -> Result<(), ReconciliationBaselineError> {
    if instrumentation.passes > 2
        || (clean && instrumentation.passes != 2)
        || instrumentation.directory_entries > MAX_BASELINE_SCAN_ENTRIES
        || instrumentation.directories > MAX_BASELINE_DIRECTORIES_PER_EPOCH as u64
        || instrumentation.regular_files > MAX_BASELINE_SCAN_ENTRIES
        || instrumentation.eligible_files > MAX_BASELINE_PATHS_PER_EPOCH as u64
        || instrumentation.bytes_read > MAX_BASELINE_AGGREGATE_PATH_BYTES
        || instrumentation.bytes_hashed > MAX_BASELINE_AGGREGATE_PATH_BYTES
        || instrumentation.peak_retained_rows > MAX_BASELINE_SCAN_ENTRIES
        || instrumentation.peak_retained_bytes > MAX_BASELINE_AGGREGATE_PATH_BYTES
        || instrumentation.candidates != candidate_count as u64
        || instrumentation.candidates > MAX_BASELINE_PATHS_PER_EPOCH as u64
        || instrumentation.diagnostics > MAX_BASELINE_PATHS_PER_EPOCH as u64
    {
        return Err(unavailable(
            "baseline scan instrumentation is inconsistent or exceeds its bounds",
        ));
    }
    sqlite_u64(instrumentation.wall_time_millis, "scan wall time")?;
    Ok(())
}

fn decode_stored_instrumentation(
    values: &[Option<i64>],
    path: &Path,
    candidate_count: usize,
) -> Result<BaselineScanInstrumentation, ReconciliationBaselineError> {
    if values.len() != 12 || values.iter().any(Option::is_none) {
        return Err(rebuild(
            path,
            "finished epoch lacks complete scan instrumentation",
        ));
    }
    let metric = |index: usize, name: &str| {
        let value = values[index].expect("instrumentation presence was checked");
        if value < 0 {
            Err(rebuild(path, format!("negative scan {name}")))
        } else {
            Ok(value as u64)
        }
    };
    let instrumentation = BaselineScanInstrumentation {
        passes: metric(0, "passes")?,
        directory_entries: metric(1, "directory entries")?,
        directories: metric(2, "directories")?,
        regular_files: metric(3, "regular files")?,
        eligible_files: metric(4, "eligible files")?,
        bytes_read: metric(5, "bytes read")?,
        bytes_hashed: metric(6, "bytes hashed")?,
        peak_retained_rows: metric(7, "peak retained rows")?,
        peak_retained_bytes: metric(8, "peak retained bytes")?,
        candidates: metric(9, "candidates")?,
        diagnostics: metric(10, "diagnostics")?,
        wall_time_millis: metric(11, "wall time")?,
    };
    if instrumentation.candidates != candidate_count as u64 {
        return Err(rebuild(
            path,
            "scan candidate instrumentation does not match epoch candidates",
        ));
    }
    Ok(instrumentation)
}

fn validate_managed_path(path: &ManagedPath) -> Result<(), ReconciliationBaselineError> {
    if path.as_str().len() > MAX_BASELINE_EXACT_PATH_BYTES {
        Err(unavailable("baseline managed path byte bound exceeded"))
    } else {
        ManagedPath::parse(path.as_str().to_owned())
            .map(|_| ())
            .map_err(|error| unavailable(format!("invalid baseline managed path: {error}")))
    }
}

fn checked_total(
    current: usize,
    added: usize,
    maximum: usize,
    resource: &str,
) -> Result<usize, ReconciliationBaselineError> {
    current
        .checked_add(added)
        .filter(|total| *total <= maximum)
        .ok_or_else(|| unavailable(format!("{resource} bound exceeded")))
}

fn checked_aggregate_path_bytes(
    current: u64,
    added: u64,
) -> Result<u64, ReconciliationBaselineError> {
    current
        .checked_add(added)
        .filter(|total| *total <= MAX_BASELINE_AGGREGATE_PATH_BYTES)
        .ok_or_else(|| unavailable("baseline aggregate path byte bound exceeded"))
}

fn sqlite_u64(value: u64, resource: &str) -> Result<i64, ReconciliationBaselineError> {
    i64::try_from(value)
        .map_err(|_| unavailable(format!("{resource} exceeds SQLite integer range")))
}

fn sqlite_usize(value: usize, resource: &str) -> Result<i64, ReconciliationBaselineError> {
    i64::try_from(value)
        .map_err(|_| unavailable(format!("{resource} exceeds SQLite integer range")))
}

fn require_nonnegative(
    value: i64,
    path: &Path,
    resource: &str,
) -> Result<i64, ReconciliationBaselineError> {
    if value < 0 {
        Err(rebuild(path, format!("negative baseline {resource}")))
    } else {
        Ok(value)
    }
}

fn decode_bounded_usize(
    value: Result<i64, SqlError>,
    maximum: usize,
    path: &Path,
    resource: &str,
) -> Result<usize, ReconciliationBaselineError> {
    let value =
        value.map_err(|error| classify_sql_error(path, error, "decoding baseline integer"))?;
    if value < 0 || value as u128 > maximum as u128 {
        return Err(rebuild(path, format!("baseline {resource} bound exceeded")));
    }
    Ok(value as usize)
}

fn decode_bounded_u64(
    value: Result<i64, SqlError>,
    maximum: u64,
    path: &Path,
    resource: &str,
) -> Result<u64, ReconciliationBaselineError> {
    let value =
        value.map_err(|error| classify_sql_error(path, error, "decoding baseline integer"))?;
    if value < 0 || value as u64 > maximum {
        return Err(rebuild(path, format!("baseline {resource} bound exceeded")));
    }
    Ok(value as u64)
}

fn digest_blob(digest: ContentDigest) -> Vec<u8> {
    digest.as_bytes().to_vec()
}

fn decode_digest_value(
    value: Result<Vec<u8>, SqlError>,
    path: &Path,
    resource: &str,
) -> Result<ContentDigest, ReconciliationBaselineError> {
    let bytes =
        value.map_err(|error| classify_sql_error(path, error, "decoding baseline digest"))?;
    decode_digest_bytes(&bytes, path, resource)
}

fn decode_digest_bytes(
    bytes: &[u8],
    path: &Path,
    resource: &str,
) -> Result<ContentDigest, ReconciliationBaselineError> {
    decode_fixed_32(bytes, path, resource).map(ContentDigest::from_bytes)
}

fn decode_optional_digest(
    bytes: Option<Vec<u8>>,
    path: &Path,
    resource: &str,
) -> Result<Option<ContentDigest>, ReconciliationBaselineError> {
    bytes
        .map(|bytes| decode_digest_bytes(&bytes, path, resource))
        .transpose()
}

fn decode_fixed_32(
    bytes: &[u8],
    path: &Path,
    resource: &str,
) -> Result<[u8; 32], ReconciliationBaselineError> {
    bytes
        .try_into()
        .map_err(|_| rebuild(path, format!("malformed {resource} length")))
}

const fn managed_kind_tag(kind: ManagedTextKind) -> i64 {
    match kind {
        ManagedTextKind::Page => 1,
        ManagedTextKind::Journal => 2,
    }
}

fn managed_kind_from_tag(tag: i64) -> Option<ManagedTextKind> {
    match tag {
        1 => Some(ManagedTextKind::Page),
        2 => Some(ManagedTextKind::Journal),
        _ => None,
    }
}

fn unavailable(detail: impl Into<String>) -> ReconciliationBaselineError {
    ReconciliationBaselineError::BaselineUnavailable {
        detail: detail.into(),
    }
}

fn rebuild(path: &Path, detail: impl Into<String>) -> ReconciliationBaselineError {
    ReconciliationBaselineError::RebuildRequired {
        path: path.to_path_buf(),
        detail: detail.into(),
    }
}

fn classify_sql_error(
    path: &Path,
    error: SqlError,
    operation: &str,
) -> ReconciliationBaselineError {
    let transient = error.sqlite_error_code().is_some_and(|code| {
        matches!(
            code,
            ErrorCode::DatabaseBusy
                | ErrorCode::DatabaseLocked
                | ErrorCode::SystemIoFailure
                | ErrorCode::CannotOpen
                | ErrorCode::FileLockingProtocolFailed
        )
    });
    if transient {
        unavailable(format!("{operation}: {error}"))
    } else {
        rebuild(path, format!("{operation}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_text_scope::GraphTextScope;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tine-reconciliation-baseline-{label}-{}-{sequence}",
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

    fn binding(label: &[u8]) -> ReconciliationBaselineBinding {
        let graph_resource = CanonicalGraphResourceId::from_capability_identity(b"test", label);
        let scope = GraphTextScope::new(&[], false).bind_graph_resource(graph_resource);
        ReconciliationBaselineBinding::new(
            WorkspaceId::new(),
            ProjectionEndpointId::new(),
            graph_resource,
            scope,
        )
        .unwrap()
    }

    fn trusted_runtime(path: &Path) -> TrustedPrivateApplicationRuntimeRoot {
        let runtime = ApplicationRuntimeRoot::open_for_test(path).unwrap();
        TrustedPrivateApplicationRuntimeRoot::from_application_runtime_root(&runtime)
    }

    fn open_fresh(
        label: &str,
    ) -> (
        TestDir,
        ReconciliationBaselineBinding,
        ReconciliationBaseline,
    ) {
        let directory = TestDir::new(label);
        let runtime = trusted_runtime(directory.path());
        let binding = binding(label.as_bytes());
        let baseline = ReconciliationBaseline::create_fresh(&runtime, binding.clone()).unwrap();
        (directory, binding, baseline)
    }

    fn root(binding: &ReconciliationBaselineBinding) -> BaselineScanDirectory {
        BaselineScanDirectory {
            path: BaselineDirectoryPath::parse("").unwrap(),
            resource: ContentDigest::from_bytes(*binding.graph_resource().as_bytes()),
        }
    }

    fn present(path: &str, bytes: &[u8]) -> BaselineScanPath {
        BaselineScanPath {
            path: ManagedPath::parse(path).unwrap(),
            managed_kind: Some(ManagedTextKind::Page),
            state: BaselineObservedState::Present {
                description: BlobDescription::of(bytes),
                file_resource: ContentDigest::of(format!("resource-{path}").as_bytes()),
                link_count: 1,
            },
        }
    }

    fn instrumentation(candidates: u64) -> BaselineScanInstrumentation {
        BaselineScanInstrumentation {
            passes: 2,
            candidates,
            wall_time_millis: 1,
            ..BaselineScanInstrumentation::default()
        }
    }

    fn clean_epoch(
        baseline: &mut ReconciliationBaseline,
        binding: &ReconciliationBaselineBinding,
        generation: u64,
        rows: &[BaselineScanPath],
    ) -> BaselineHead {
        let epoch = baseline
            .begin_epoch(BeginBaselineEpoch {
                started_at: BaselineTimestamp::from_millis(generation * 10).unwrap(),
                accepted_frontier: ContentDigest::of(format!("frontier-{generation}").as_bytes()),
                projection_generation: generation,
            })
            .unwrap();
        baseline
            .append_scan_directories(epoch, &[root(binding)])
            .unwrap();
        baseline.append_scan_paths(epoch, rows).unwrap();
        let pass = ContentDigest::of(format!("pass-{generation}").as_bytes());
        baseline
            .finish_epoch(
                epoch,
                FinishBaselineEpoch {
                    completed_at: BaselineTimestamp::from_millis(generation * 10 + 1).unwrap(),
                    pass_a_digest: pass,
                    pass_b_digest: pass,
                    candidate_digest: ContentDigest::of(b"no-candidates"),
                    candidate_count: 0,
                    outcome: BaselineEpochOutcome::Noop,
                    instrumentation: instrumentation(0),
                },
            )
            .unwrap()
            .unwrap()
    }

    #[test]
    fn clean_head_is_atomic_and_blocked_or_unstable_epochs_never_replace_it() {
        let (_directory, binding, mut baseline) = open_fresh("head");
        assert!(matches!(
            baseline.head(),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
        let first = clean_epoch(
            &mut baseline,
            &binding,
            1,
            &[present("custom/nested/Page.MARKDOWN", b"one")],
        );
        assert_eq!(first.baseline_generation, 1);

        let blocked = baseline
            .begin_epoch(BeginBaselineEpoch {
                started_at: BaselineTimestamp::from_millis(20).unwrap(),
                accepted_frontier: ContentDigest::of(b"frontier-2"),
                projection_generation: 2,
            })
            .unwrap();
        baseline
            .append_scan_directories(blocked, &[root(&binding)])
            .unwrap();
        baseline
            .append_scan_paths(
                blocked,
                &[present("custom/nested/Page.MARKDOWN", b"silent mutation")],
            )
            .unwrap();
        assert!(baseline
            .finish_epoch(
                blocked,
                FinishBaselineEpoch {
                    completed_at: BaselineTimestamp::from_millis(21).unwrap(),
                    pass_a_digest: ContentDigest::of(b"pass-a"),
                    pass_b_digest: ContentDigest::of(b"pass-b"),
                    candidate_digest: ContentDigest::of(b"candidate"),
                    candidate_count: 1,
                    outcome: BaselineEpochOutcome::Blocked,
                    instrumentation: instrumentation(1),
                },
            )
            .unwrap()
            .is_none());
        assert_eq!(baseline.head().unwrap(), first);

        let unstable = baseline
            .begin_epoch(BeginBaselineEpoch {
                started_at: BaselineTimestamp::from_millis(30).unwrap(),
                accepted_frontier: ContentDigest::of(b"frontier-3"),
                projection_generation: 3,
            })
            .unwrap();
        baseline
            .append_scan_directories(unstable, &[root(&binding)])
            .unwrap();
        baseline
            .append_scan_paths(unstable, &[present("nested/page.org", b"changed")])
            .unwrap();
        assert!(matches!(
            baseline.finish_epoch(
                unstable,
                FinishBaselineEpoch {
                    completed_at: BaselineTimestamp::from_millis(31).unwrap(),
                    pass_a_digest: ContentDigest::of(b"pass-a"),
                    pass_b_digest: ContentDigest::of(b"pass-b"),
                    candidate_digest: ContentDigest::of(b"candidate"),
                    candidate_count: 1,
                    outcome: BaselineEpochOutcome::Complete,
                    instrumentation: instrumentation(1),
                },
            ),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
        assert_eq!(baseline.head().unwrap(), first);
    }

    #[test]
    fn direct_completion_updates_present_and_deleted_paths_only_from_clean_head() {
        let (_directory, binding, mut baseline) = open_fresh("completion");
        let head = clean_epoch(
            &mut baseline,
            &binding,
            7,
            &[present("unusual/layout/page.md", b"old")],
        );
        let completion = BaselineCompletionIdentity([9; 32]);
        let page_path = ManagedPath::parse("unusual/layout/page.md").unwrap();
        let updated = baseline
            .apply_authenticated_tine_completion(&AuthenticatedTineCompletion {
                workspace: binding.workspace(),
                endpoint: binding.endpoint(),
                graph_resource: binding.graph_resource(),
                path: &page_path,
                managed_kind: ManagedTextKind::Page,
                state: TineCompletionState::Present(BlobDescription::of(b"new")),
                completion_identity: completion,
                completion_frontier: ContentDigest::of(b"frontier-8"),
                projection_generation: 8,
                completed_at: BaselineTimestamp::from_millis(80).unwrap(),
            })
            .unwrap();
        assert_eq!(updated.accepted_frontier, head.accepted_frontier);
        let stored_frontier: Vec<u8> = baseline
            .connection
            .query_row(
                "SELECT completion_frontier FROM paths
                 WHERE epoch_id = ?1 AND exact_path = ?2",
                params![head.epoch.as_i64(), page_path.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stored_frontier,
            ContentDigest::of(b"frontier-8").as_bytes().as_slice()
        );
        let page = baseline.read_head_paths_page(None, 8).unwrap();
        assert_eq!(page.head, updated);
        assert_eq!(
            page.rows[0].state,
            BaselineRecordedState::Present(BlobDescription::of(b"new"))
        );
        assert_eq!(
            page.rows[0].source,
            BaselinePathSource::TineCompletion(completion)
        );

        let deleted = baseline
            .apply_authenticated_tine_completion(&AuthenticatedTineCompletion {
                workspace: binding.workspace(),
                endpoint: binding.endpoint(),
                graph_resource: binding.graph_resource(),
                path: &page_path,
                managed_kind: ManagedTextKind::Page,
                state: TineCompletionState::Deleted,
                completion_identity: BaselineCompletionIdentity([10; 32]),
                completion_frontier: ContentDigest::of(b"frontier-9"),
                projection_generation: 9,
                completed_at: BaselineTimestamp::from_millis(90).unwrap(),
            })
            .unwrap();
        assert_eq!(deleted.baseline_generation, updated.baseline_generation + 1);
        let page = baseline.read_head_paths_page(None, 8).unwrap();
        assert_eq!(page.rows[0].state, BaselineRecordedState::ExpectedAbsent);

        assert!(matches!(
            baseline.apply_authenticated_tine_completion(&AuthenticatedTineCompletion {
                workspace: binding.workspace(),
                endpoint: binding.endpoint(),
                graph_resource: binding.graph_resource(),
                path: &page_path,
                managed_kind: ManagedTextKind::Page,
                state: TineCompletionState::Deleted,
                completion_identity: BaselineCompletionIdentity([11; 32]),
                completion_frontier: ContentDigest::of(b"stale"),
                projection_generation: 8,
                completed_at: BaselineTimestamp::from_millis(100).unwrap(),
            }),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
    }

    #[test]
    fn reopen_rejects_binding_schema_path_digest_and_head_corruption() {
        for corruption in ["binding", "schema", "path", "digest", "head"] {
            let (directory, binding, mut baseline) = open_fresh(corruption);
            clean_epoch(
                &mut baseline,
                &binding,
                1,
                &[present("nested/page.md", b"bytes")],
            );
            let database_path = baseline.path().to_path_buf();
            drop(baseline);
            let connection = Connection::open(&database_path).unwrap();
            match corruption {
                "binding" => {
                    connection
                        .execute("UPDATE binding SET endpoint = zeroblob(16)", [])
                        .unwrap();
                }
                "schema" => {
                    connection.pragma_update(None, "user_version", 99).unwrap();
                }
                "path" => {
                    connection
                        .execute("UPDATE paths SET exact_path = '../escape.md'", [])
                        .unwrap();
                }
                "digest" => {
                    connection
                        .execute("UPDATE paths SET content_digest = x'01'", [])
                        .unwrap_err();
                    connection
                        .execute_batch("PRAGMA ignore_check_constraints = ON;")
                        .unwrap();
                    connection
                        .execute("UPDATE paths SET content_digest = x'01'", [])
                        .unwrap();
                }
                "head" => {
                    connection
                        .execute("UPDATE epochs SET state = 2", [])
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(connection);
            let runtime = trusted_runtime(directory.path());
            let error = match ReconciliationBaseline::open_existing(&runtime, binding.clone()) {
                Err(error) => error,
                Ok(_) => panic!("{corruption} unexpectedly opened"),
            };
            assert!(error.requires_rebuild(), "{corruption}: {error}");
            assert!(
                database_path.exists(),
                "{corruption} database was not preserved"
            );
        }
    }

    #[test]
    fn graph_substitution_and_implicit_recreate_fail_without_replacing_bytes() {
        let (directory, binding, baseline) = open_fresh("substitution");
        let database_path = baseline.path().to_path_buf();
        drop(baseline);
        let runtime = trusted_runtime(directory.path());

        assert!(matches!(
            ReconciliationBaseline::create_fresh(&runtime, binding.clone()),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
        let original_length = fs::metadata(&database_path).unwrap().len();

        let foreign_resource =
            CanonicalGraphResourceId::from_capability_identity(b"test", b"substituted-root");
        let foreign_scope = GraphTextScope::new(&[], false).bind_graph_resource(foreign_resource);
        let foreign_binding = ReconciliationBaselineBinding::new(
            binding.workspace(),
            binding.endpoint(),
            foreign_resource,
            foreign_scope,
        )
        .unwrap();
        let error = match ReconciliationBaseline::open_existing(&runtime, foreign_binding) {
            Err(error) => error,
            Ok(_) => panic!("graph-resource substitution unexpectedly opened"),
        };
        assert!(error.requires_rebuild());
        assert_eq!(fs::metadata(database_path).unwrap().len(), original_length);
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_database_symlink_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let directory = TestDir::new("database-symlink");
        let runtime = trusted_runtime(directory.path());
        let binding = binding(b"database-symlink");
        let (_parent, database_path) = prepare_database_parent(&runtime, &binding, true).unwrap();
        let target = directory.path().join("database-symlink-target");
        let original = b"preserve non-SQLite target bytes";
        fs::write(&target, original).unwrap();
        symlink(&target, &database_path).unwrap();

        assert!(ReconciliationBaseline::create_fresh(&runtime, binding.clone()).is_err());
        assert!(ReconciliationBaseline::open_existing(&runtime, binding).is_err());
        assert_eq!(fs::read(target).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_sqlite_sidecar_symlinks_are_rejected_on_create_and_open() {
        use std::os::unix::fs::symlink;

        for sidecar in DATABASE_SIDECAR_FILES {
            let create_directory = TestDir::new(&format!("create-{sidecar}"));
            let create_runtime = trusted_runtime(create_directory.path());
            let create_binding = binding(format!("create-{sidecar}").as_bytes());
            let (_parent, database_path) =
                prepare_database_parent(&create_runtime, &create_binding, true).unwrap();
            let sidecar_path = database_path.with_file_name(sidecar);
            let target = create_directory.path().join("create-sidecar-target");
            let original = b"preserve create-sidecar target bytes";
            fs::write(&target, original).unwrap();
            symlink(&target, &sidecar_path).unwrap();

            assert!(ReconciliationBaseline::create_fresh(&create_runtime, create_binding).is_err());
            assert!(
                !database_path.exists(),
                "failed create through {sidecar} must not create a database or clean head"
            );
            assert_eq!(fs::read(target).unwrap(), original);

            let (open_directory, open_binding, baseline) = open_fresh(&format!("open-{sidecar}"));
            let database_path = baseline.path().to_path_buf();
            drop(baseline);
            let open_runtime = trusted_runtime(open_directory.path());
            let sidecar_path = database_path.with_file_name(sidecar);
            let target = open_directory.path().join("open-sidecar-target");
            let original = b"preserve open-sidecar target bytes";
            fs::write(&target, original).unwrap();
            symlink(&target, &sidecar_path).unwrap();

            assert!(
                ReconciliationBaseline::open_existing(&open_runtime, open_binding.clone()).is_err()
            );
            assert_eq!(fs::read(target).unwrap(), original);
            fs::remove_file(sidecar_path).unwrap();
            let mut reopened =
                ReconciliationBaseline::open_existing(&open_runtime, open_binding).unwrap();
            assert!(
                matches!(
                    reopened.head(),
                    Err(ReconciliationBaselineError::BaselineUnavailable { .. })
                ),
                "failed open through {sidecar} must not produce a clean head"
            );
        }
    }

    #[test]
    fn unsupported_sqlite_sidecar_file_type_fails_closed() {
        let (directory, binding, baseline) = open_fresh("sidecar-directory");
        let database_path = baseline.path().to_path_buf();
        drop(baseline);
        let runtime = trusted_runtime(directory.path());
        let sidecar_path = database_path.with_file_name(DATABASE_SIDECAR_FILES[0]);
        fs::create_dir(&sidecar_path).unwrap();

        assert!(ReconciliationBaseline::open_existing(&runtime, binding.clone()).is_err());
        fs::remove_dir(sidecar_path).unwrap();
        let mut reopened = ReconciliationBaseline::open_existing(&runtime, binding).unwrap();
        assert!(matches!(
            reopened.head(),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
    }

    #[test]
    fn contended_writer_fails_closed_without_changing_clean_head() {
        let (_directory, binding, mut baseline) = open_fresh("writer-contention");
        let head = clean_epoch(
            &mut baseline,
            &binding,
            1,
            &[present("nested/page.md", b"bytes")],
        );
        let blocker = Connection::open(baseline.path()).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let result = baseline.record_blocked(
            ContentDigest::of(b"contended"),
            BaselineBlockedReason::AuthorityUnavailable,
            "writer contended",
            BaselineTimestamp::from_millis(2).unwrap(),
        );
        assert!(matches!(
            result,
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
        blocker.execute_batch("ROLLBACK").unwrap();
        assert_eq!(baseline.head().unwrap(), head);
    }

    #[test]
    fn validation_binds_data_version_before_releasing_the_writer_exclusion() {
        let (directory, binding, mut baseline) = open_fresh("validation-watermark");
        clean_epoch(
            &mut baseline,
            &binding,
            1,
            &[present("nested/page.md", b"bytes")],
        );
        let database_path = baseline.path().to_path_buf();
        drop(baseline);

        let mut connection = open_ambient_sqlite_connection(&database_path, false).unwrap();
        let writer = Connection::open(&database_path).unwrap();
        let changed = ContentDigest::of(b"writer-after-validation-commit");
        let trusted_data_version = validate_database_with_after_commit_hook(
            &mut connection,
            &database_path,
            &binding,
            || {
                writer
                    .execute(
                        "INSERT INTO blocked (
                            exact_observation_digest, reason, detail, first_seen, last_seen
                         ) VALUES (?1, 5, 'validation race', 1, 1)",
                        [changed.as_bytes().as_slice()],
                    )
                    .unwrap();
            },
        )
        .unwrap();
        configure_trusted_connection(&connection, &database_path).unwrap();
        let mut reopened = ReconciliationBaseline {
            connection,
            path: database_path,
            binding,
            trusted_data_version,
        };
        assert!(matches!(
            reopened.head(),
            Err(ReconciliationBaselineError::RebuildRequired { .. })
        ));
        drop(reopened);
        drop(writer);
        drop(directory);
    }

    #[test]
    fn nested_unicode_paths_count_utf8_bytes_and_round_trip_on_reopen() {
        let (directory, binding, mut baseline) = open_fresh("unicode-paths");
        let epoch = baseline
            .begin_epoch(BeginBaselineEpoch {
                started_at: BaselineTimestamp::from_millis(1).unwrap(),
                accepted_frontier: ContentDigest::of(b"unicode-frontier"),
                projection_generation: 1,
            })
            .unwrap();
        baseline
            .append_scan_directories(
                epoch,
                &[
                    root(&binding),
                    BaselineScanDirectory {
                        path: BaselineDirectoryPath::parse("资料/层级").unwrap(),
                        resource: ContentDigest::of(b"unicode-directory"),
                    },
                ],
            )
            .unwrap();
        let exact_path = "资料/层级/页面-é.md";
        baseline
            .append_scan_paths(epoch, &[present(exact_path, b"unicode path bytes")])
            .unwrap();
        let pass = ContentDigest::of(b"unicode-pass");
        baseline
            .finish_epoch(
                epoch,
                FinishBaselineEpoch {
                    completed_at: BaselineTimestamp::from_millis(2).unwrap(),
                    pass_a_digest: pass,
                    pass_b_digest: pass,
                    candidate_digest: ContentDigest::of(b"unicode-none"),
                    candidate_count: 0,
                    outcome: BaselineEpochOutcome::Noop,
                    instrumentation: instrumentation(0),
                },
            )
            .unwrap()
            .unwrap();
        drop(baseline);

        let runtime = trusted_runtime(directory.path());
        let mut reopened =
            ReconciliationBaseline::open_existing(&runtime, binding.clone()).unwrap();
        let page = reopened.read_head_paths_page(None, 8).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].path.as_str(), exact_path);
        assert_eq!(
            page.rows[0].state,
            BaselineRecordedState::Present(BlobDescription::of(b"unicode path bytes"))
        );
    }

    #[test]
    fn blocked_signatures_are_deduplicated_without_changing_head() {
        let (_directory, binding, mut baseline) = open_fresh("blocked");
        let head = clean_epoch(&mut baseline, &binding, 1, &[present("pages/a.md", b"a")]);
        let digest = ContentDigest::of(b"same-blocked-observation");
        let first = baseline
            .record_blocked(
                digest,
                BaselineBlockedReason::UnsafeFilesystem,
                "hard link",
                BaselineTimestamp::from_millis(2).unwrap(),
            )
            .unwrap();
        let second = baseline
            .record_blocked(
                digest,
                BaselineBlockedReason::UnsafeFilesystem,
                "hard link",
                BaselineTimestamp::from_millis(3).unwrap(),
            )
            .unwrap();
        assert_eq!(first.first_seen, second.first_seen);
        assert_eq!(second.last_seen, BaselineTimestamp::from_millis(3).unwrap());
        assert_eq!(baseline.blocked_signature(digest).unwrap(), Some(second));
        assert_eq!(baseline.head().unwrap(), head);
    }

    #[test]
    fn operations_are_bounded_and_path_pages_do_not_fetch_all() {
        let (_directory, binding, mut baseline) = open_fresh("bounds");
        let epoch = baseline
            .begin_epoch(BeginBaselineEpoch {
                started_at: BaselineTimestamp::from_millis(1).unwrap(),
                accepted_frontier: ContentDigest::of(b"frontier"),
                projection_generation: 1,
            })
            .unwrap();
        let oversized = (0..=MAX_BASELINE_WRITE_ROWS)
            .map(|index| present(&format!("nested/{index}.md"), b"x"))
            .collect::<Vec<_>>();
        assert!(matches!(
            baseline.append_scan_paths(epoch, &oversized),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
        baseline
            .append_scan_directories(epoch, &[root(&binding)])
            .unwrap();
        let rows = (0..10)
            .map(|index| present(&format!("nested/{index:02}.md"), b"x"))
            .collect::<Vec<_>>();
        baseline.append_scan_paths(epoch, &rows).unwrap();
        let pass = ContentDigest::of(b"pass");
        baseline
            .finish_epoch(
                epoch,
                FinishBaselineEpoch {
                    completed_at: BaselineTimestamp::from_millis(2).unwrap(),
                    pass_a_digest: pass,
                    pass_b_digest: pass,
                    candidate_digest: ContentDigest::of(b"none"),
                    candidate_count: 0,
                    outcome: BaselineEpochOutcome::Noop,
                    instrumentation: instrumentation(0),
                },
            )
            .unwrap();
        let stored_instrumentation: (i64, i64, i64) = baseline
            .connection
            .query_row(
                "SELECT scan_passes, scan_candidates, scan_wall_time_ms
                 FROM epochs WHERE id = ?1",
                [epoch.as_i64()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored_instrumentation, (2, 0, 1));
        let first = baseline.read_head_paths_page(None, 3).unwrap();
        assert_eq!(first.rows.len(), 3);
        let second = baseline
            .read_head_paths_page(first.next_after.as_ref(), 3)
            .unwrap();
        assert_eq!(second.rows.len(), 3);
        assert!(first.rows.last().unwrap().path < second.rows[0].path);
        assert!(matches!(
            baseline.read_head_paths_page(None, MAX_BASELINE_PAGE_ROWS + 1),
            Err(ReconciliationBaselineError::BaselineUnavailable { .. })
        ));
    }
}
