//! Disposable SQLite frontier projection for the sparse operation log.
//!
//! This module deliberately accepts only already-accepted operation events. It
//! has no mutation-authoring API and is never part of keystroke durability.
//! Callers place the database and its sibling lease in device-local app data;
//! neither path is derived from, or requires access to, the shared graph.
//!
//! The lease uses the platform's advisory file-lock primitive through `fs2`.
//! Dropping the applier or terminating its process releases the lock on Linux,
//! macOS, Windows, and Android. The small lock file remains as diagnostic
//! metadata, but never decides ownership by its contents.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt as _;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior,
};
use uuid::Uuid;

use super::{
    BatchId, BatchInspection, ContentDigest, FrontierV2, LineageDigest, ObjectKind, ObjectStore,
    SemanticEffect, SemanticEffectDigest, ShardedHotEngine, ValidatedBatch, WorkspaceId,
    WorkspaceStatus, MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};

pub const SQLITE_APPLICATION_ID: u32 = 0x5449_4e45;
pub const SQLITE_SCHEMA_VERSION: u32 = 1;
pub const TAIL_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const TAIL_MAX_BATCHES: usize = 10_000;

const EXPECTED_TABLES: [&str; 3] = ["applied_batches", "frontier", "meta"];
const META_COLUMNS: [&str; 8] = [
    "singleton",
    "workspace_id",
    "lineage_digest",
    "oplog_protocol_version",
    "operation_schema_version",
    "object_envelope_schema_version",
    "manifest_encoding_version",
    "managed_entity_set_version",
];
const FRONTIER_COLUMNS: [&str; 4] = [
    "singleton",
    "exact_frontier",
    "frontier_digest",
    "applied_batch_count",
];
const APPLIED_BATCH_COLUMNS: [&str; 11] = [
    "sequence",
    "batch_id",
    "manifest_digest",
    "semantic_effect",
    "semantic_effect_digest",
    "dependency_frontier",
    "dependency_frontier_digest",
    "exact_frontier",
    "exact_frontier_digest",
    "causal_dependency_heads",
    "retained_bytes",
];
const FORENSIC_SUFFIXES: [&str; 3] = ["", "-wal", "-shm"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionClaim {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    oplog_protocol_version: u32,
    operation_schema_version: u32,
    object_envelope_schema_version: u32,
    manifest_encoding_version: u32,
    managed_entity_set_version: u32,
}

impl ProjectionClaim {
    pub const fn current(workspace_id: WorkspaceId, lineage_digest: LineageDigest) -> Self {
        Self {
            workspace_id,
            lineage_digest,
            oplog_protocol_version: OPLOG_PROTOCOL_VERSION,
            operation_schema_version: OPERATION_SCHEMA_VERSION,
            object_envelope_schema_version: OBJECT_ENVELOPE_SCHEMA_VERSION,
            manifest_encoding_version: MANIFEST_ENCODING_VERSION,
            managed_entity_set_version: MANAGED_ENTITY_SET_VERSION,
        }
    }

    pub const fn workspace_id(self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn lineage_digest(self) -> LineageDigest {
        self.lineage_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedBatchEvent {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: BatchId,
    manifest_digest: ContentDigest,
    semantic_effect: Vec<u8>,
    semantic_effect_digest: SemanticEffectDigest,
    dependency_frontier: FrontierV2,
    exact_frontier: FrontierV2,
    causal_dependency_heads: Vec<BatchId>,
    retained_bytes: usize,
}

impl AcceptedBatchEvent {
    pub fn from_accepted(
        engine: &ShardedHotEngine,
        store: &ObjectStore,
        batch_id: BatchId,
    ) -> Result<Self, ProjectionError> {
        if engine.workspace_id() != store.workspace_id() {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: engine.workspace_id(),
                found: store.workspace_id(),
            });
        }
        if !engine
            .status()
            .accepted_batch_ids()
            .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?
            .contains(&batch_id)
        {
            return Err(ProjectionError::InvalidAcceptedEvent(format!(
                "batch {batch_id} is not accepted by the hot engine"
            )));
        }
        let validated = match store.inspect_batch(batch_id)? {
            BatchInspection::Ready(validated) => validated,
            BatchInspection::Absent => {
                return Err(ProjectionError::InvalidAcceptedEvent(format!(
                    "accepted batch {batch_id} is absent from the object store"
                )));
            }
            BatchInspection::Staged { .. } => {
                return Err(ProjectionError::InvalidAcceptedEvent(format!(
                    "accepted batch {batch_id} is partial in the object store"
                )));
            }
        };
        if validated.manifest().lineage_digest() != engine.lineage_digest() {
            return Err(ProjectionError::LineageMismatch {
                expected: engine.lineage_digest(),
                found: validated.manifest().lineage_digest(),
            });
        }
        Self::from_validated(
            &validated,
            engine
                .exact_frontier()
                .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?,
        )
    }

    fn from_validated(
        batch: &ValidatedBatch,
        exact_frontier: FrontierV2,
    ) -> Result<Self, ProjectionError> {
        let manifest = batch.manifest();
        let manifest_bytes = manifest.encode().map_err(|error| {
            ProjectionError::InvalidAcceptedEvent(format!(
                "cannot encode accepted manifest {}: {error}",
                manifest.batch_id()
            ))
        })?;
        let semantic = batch
            .objects()
            .iter()
            .find(|object| object.kind() == ObjectKind::SemanticEffect)
            .ok_or_else(|| {
                ProjectionError::InvalidAcceptedEvent(format!(
                    "accepted batch {} has no semantic effect",
                    manifest.batch_id()
                ))
            })?;
        let semantic_effect = semantic.payload().to_vec();
        let decoded = SemanticEffect::decode(&semantic_effect).map_err(|error| {
            ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} has an invalid semantic effect: {error}",
                manifest.batch_id()
            ))
        })?;
        if decoded.encode().map_err(|error| {
            ProjectionError::InvalidAcceptedEvent(format!(
                "cannot re-encode semantic effect for {}: {error}",
                manifest.batch_id()
            ))
        })? != semantic_effect
        {
            return Err(ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} has a non-canonical semantic effect",
                manifest.batch_id()
            )));
        }
        let semantic_effect_digest = SemanticEffectDigest::of(&semantic_effect);
        if semantic_effect_digest != manifest.semantic_effect_digest() {
            return Err(ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} semantic effect digest differs from its manifest",
                manifest.batch_id()
            )));
        }
        let retained_bytes = batch.objects().iter().try_fold(
            manifest_bytes.len(),
            |total, object| -> Result<usize, ProjectionError> {
                let encoded = object.encode().map_err(|error| {
                    ProjectionError::InvalidAcceptedEvent(format!(
                        "cannot encode object for accepted batch {}: {error}",
                        manifest.batch_id()
                    ))
                })?;
                total.checked_add(encoded.len()).ok_or_else(|| {
                    ProjectionError::InvalidAcceptedEvent(
                        "accepted event retained-byte count overflowed".into(),
                    )
                })
            },
        )?;
        canonical_frontier_bytes(&exact_frontier)?;
        Ok(Self {
            workspace_id: manifest.workspace_id(),
            lineage_digest: manifest.lineage_digest(),
            batch_id: manifest.batch_id(),
            manifest_digest: ContentDigest::of(&manifest_bytes),
            semantic_effect,
            semantic_effect_digest,
            dependency_frontier: manifest.dependency_frontier().clone(),
            exact_frontier,
            causal_dependency_heads: manifest.causal_dependency_heads().to_vec(),
            retained_bytes,
        })
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub const fn manifest_digest(&self) -> ContentDigest {
        self.manifest_digest
    }

    pub fn semantic_effect(&self) -> &[u8] {
        &self.semantic_effect
    }

    pub const fn semantic_effect_digest(&self) -> SemanticEffectDigest {
        self.semantic_effect_digest
    }

    pub fn dependency_frontier(&self) -> &FrontierV2 {
        &self.dependency_frontier
    }

    pub fn exact_frontier(&self) -> &FrontierV2 {
        &self.exact_frontier
    }

    pub fn causal_dependency_heads(&self) -> &[BatchId] {
        &self.causal_dependency_heads
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

pub struct RebuildSource<'a> {
    engine: &'a ShardedHotEngine,
    store: &'a ObjectStore,
    exact_frontier: FrontierV2,
}

impl<'a> RebuildSource<'a> {
    pub fn new(
        engine: &'a ShardedHotEngine,
        store: &'a ObjectStore,
    ) -> Result<Self, ProjectionError> {
        let exact_frontier = engine
            .exact_frontier()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        Ok(Self {
            engine,
            store,
            exact_frontier,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForensicEvidence {
    pub original_path: PathBuf,
    pub preserved_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionRecovery {
    OpenedExisting,
    RebuiltMissing {
        applied_batches: usize,
    },
    RebuiltPreservingEvidence {
        reason: String,
        evidence: Vec<ForensicEvidence>,
        applied_batches: usize,
    },
}

pub struct OpenProjection {
    pub database: SqliteFrontier,
    pub recovery: ProjectionRecovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyDisposition {
    Applied,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TailOverlayStatus {
    pub unapplied_batches: usize,
    pub retained_bytes: usize,
    pub backpressured: bool,
}

impl TailOverlayStatus {
    pub const fn visible_reason(self) -> Option<&'static str> {
        if self.backpressured {
            Some("Operation indexing is catching up; mutations are temporarily paused.")
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TailOverlayError {
    Backpressure(TailOverlayStatus),
    BatchCollision(BatchId),
    Projection(ProjectionError),
}

impl fmt::Display for TailOverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backpressure(status) => write!(
                f,
                "SQLite tail backpressure at {} batches and {} bytes",
                status.unapplied_batches, status.retained_bytes
            ),
            Self::BatchCollision(batch_id) => {
                write!(f, "conflicting unapplied event for batch {batch_id}")
            }
            Self::Projection(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for TailOverlayError {}

impl From<ProjectionError> for TailOverlayError {
    fn from(value: ProjectionError) -> Self {
        Self::Projection(value)
    }
}

#[derive(Default)]
pub struct TailOverlay {
    queue: VecDeque<AcceptedBatchEvent>,
    retained_bytes: usize,
}

impl TailOverlay {
    pub fn status(&self) -> TailOverlayStatus {
        TailOverlayStatus {
            unapplied_batches: self.queue.len(),
            retained_bytes: self.retained_bytes,
            backpressured: self.queue.len() >= TAIL_MAX_BATCHES
                || self.retained_bytes >= TAIL_MAX_BYTES,
        }
    }

    /// Queue an already-authoritative accepted event. Mutation admission must
    /// call this before exposing another event; SQLite work itself stays async.
    pub fn try_enqueue(&mut self, event: AcceptedBatchEvent) -> Result<bool, TailOverlayError> {
        if let Some(existing) = self
            .queue
            .iter()
            .find(|existing| existing.batch_id == event.batch_id)
        {
            return if existing == &event {
                Ok(false)
            } else {
                Err(TailOverlayError::BatchCollision(event.batch_id))
            };
        }
        let next_bytes = self.retained_bytes.saturating_add(event.retained_bytes);
        if self.queue.len() >= TAIL_MAX_BATCHES || next_bytes > TAIL_MAX_BYTES {
            return Err(TailOverlayError::Backpressure(TailOverlayStatus {
                unapplied_batches: self.queue.len().saturating_add(1),
                retained_bytes: next_bytes,
                backpressured: true,
            }));
        }
        self.retained_bytes = next_bytes;
        self.queue.push_back(event);
        Ok(true)
    }

    /// Apply any dependency-ready events, scanning the bounded tail so provider
    /// reordering does not turn FIFO order into a false dependency failure.
    pub fn drain_ready(
        &mut self,
        database: &mut SqliteFrontier,
        max_batches: usize,
    ) -> Result<usize, TailOverlayError> {
        let mut applied = 0;
        while applied < max_batches {
            let mut ready = None;
            for (index, event) in self.queue.iter().enumerate() {
                let mut dependencies_applied = true;
                for dependency in &event.causal_dependency_heads {
                    if !database.contains_batch(*dependency)? {
                        dependencies_applied = false;
                        break;
                    }
                }
                if dependencies_applied {
                    ready = Some(index);
                    break;
                }
            }
            let Some(index) = ready else {
                break;
            };
            let event = self
                .queue
                .remove(index)
                .expect("selected overlay event exists");
            let retained_bytes = event.retained_bytes;
            match database.apply_accepted(&event) {
                Ok(_) => {
                    self.retained_bytes = self.retained_bytes.saturating_sub(retained_bytes);
                    applied += 1;
                }
                Err(error) => {
                    self.queue.insert(index, event);
                    return Err(error.into());
                }
            }
        }
        Ok(applied)
    }
}

/// One leased device-local projection handle.
///
/// The projection's sibling lease, annotated with its exact workspace claim,
/// lives exactly as long as this value.
/// A clean drop or process termination releases the OS lock; a later process
/// validates the database before reuse and rebuilds from engine/store evidence
/// when deletion, stale state, corruption, or an interrupted WAL is observed.
pub struct SqliteFrontier {
    path: PathBuf,
    claim: ProjectionClaim,
    connection: Connection,
    _lease: ProcessLease,
}

impl SqliteFrontier {
    pub fn open_or_rebuild(
        path: &Path,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
    ) -> Result<OpenProjection, ProjectionError> {
        validate_source(claim, &source)?;
        prepare_database_parent(path)?;
        let lease = ProcessLease::acquire(path, claim.workspace_id)?;
        let expected = collect_rebuild_events(claim, &source)?;
        let existed = projection_files_exist(path);

        if existed {
            match validate_existing(path, claim, &expected, &source.exact_frontier) {
                Ok(()) => {
                    let connection = open_writable(path)?;
                    return Ok(OpenProjection {
                        database: Self {
                            path: path.to_path_buf(),
                            claim,
                            connection,
                            _lease: lease,
                        },
                        recovery: ProjectionRecovery::OpenedExisting,
                    });
                }
                Err(reason) => {
                    let evidence = preserve_forensics(path)?;
                    let mut database = Self::create_new(path, claim, lease)?;
                    database.rebuild_events(&expected, &source.exact_frontier)?;
                    return Ok(OpenProjection {
                        database,
                        recovery: ProjectionRecovery::RebuiltPreservingEvidence {
                            reason,
                            evidence,
                            applied_batches: expected.len(),
                        },
                    });
                }
            }
        }

        let mut database = Self::create_new(path, claim, lease)?;
        database.rebuild_events(&expected, &source.exact_frontier)?;
        Ok(OpenProjection {
            database,
            recovery: ProjectionRecovery::RebuiltMissing {
                applied_batches: expected.len(),
            },
        })
    }

    fn create_new(
        path: &Path,
        claim: ProjectionClaim,
        lease: ProcessLease,
    ) -> Result<Self, ProjectionError> {
        let connection = open_writable(path)?;
        initialize_schema(&connection, claim)?;
        Ok(Self {
            path: path.to_path_buf(),
            claim,
            connection,
            _lease: lease,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn claim(&self) -> ProjectionClaim {
        self.claim
    }

    pub fn frontier(&self) -> Result<FrontierV2, ProjectionError> {
        read_frontier(&self.connection)
    }

    pub fn contains_frontier(&self, required: &FrontierV2) -> Result<bool, ProjectionError> {
        canonical_frontier_bytes(required)?;
        Ok(frontier_contains(
            &self.frontier()?,
            required,
            &load_ancestry(&self.connection)?,
        ))
    }

    pub fn applied_batch_count(&self) -> Result<usize, ProjectionError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM applied_batches", [], |row| row.get(0))?;
        usize::try_from(count)
            .map_err(|_| ProjectionError::Corrupt("negative applied batch count".into()))
    }

    pub fn contains_batch(&self, batch_id: BatchId) -> Result<bool, ProjectionError> {
        let found = self
            .connection
            .query_row(
                "SELECT 1 FROM applied_batches WHERE batch_id = ?1",
                [uuid_blob(&batch_id.as_uuid())],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(found)
    }

    pub fn apply_accepted(
        &mut self,
        event: &AcceptedBatchEvent,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_internal(event, false, None)
    }

    pub fn semantic_projection_digest(&self) -> Result<ContentDigest, ProjectionError> {
        let mut statement = self.connection.prepare(
            "SELECT batch_id, manifest_digest, semantic_effect, semantic_effect_digest,
                    dependency_frontier
             FROM applied_batches ORDER BY batch_id",
        )?;
        let mut rows = statement.query([])?;
        let mut bytes = b"tine/sqlite-frontier/semantic-projection/v1\0".to_vec();
        while let Some(row) = rows.next()? {
            for index in 0..5 {
                let value: Vec<u8> = row.get(index)?;
                bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
                bytes.extend_from_slice(&value);
            }
        }
        let frontier = canonical_frontier_bytes(&self.frontier()?)?;
        bytes.extend_from_slice(&(frontier.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&frontier);
        Ok(ContentDigest::of(&bytes))
    }

    fn rebuild_events(
        &mut self,
        events: &[AcceptedBatchEvent],
        exact_frontier: &FrontierV2,
    ) -> Result<(), ProjectionError> {
        let ordered = topological_events(events)?;
        let ancestry = event_ancestry(events);
        for event in ordered {
            let mut rebuilt = event.clone();
            rebuilt.exact_frontier = exact_frontier.clone();
            self.apply_internal(&rebuilt, false, Some(&ancestry))?;
        }
        if self.frontier()? != *exact_frontier {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's exact frontier".into(),
            ));
        }
        Ok(())
    }

    fn apply_internal(
        &mut self,
        event: &AcceptedBatchEvent,
        fail_after_insert: bool,
        external_ancestry: Option<&BTreeMap<BatchId, Vec<BatchId>>>,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.validate_event_claim(event)?;
        if let Some(existing) = load_batch(&self.connection, event.batch_id)? {
            if existing.matches(event)? {
                let current = self.frontier()?;
                let ancestry = load_ancestry(&self.connection)?;
                if frontier_contains(&current, &event.exact_frontier, &ancestry) {
                    return Ok(ApplyDisposition::Duplicate);
                }
                return Err(ProjectionError::FrontierRegression);
            }
            return Err(ProjectionError::BatchCollision(event.batch_id));
        }

        let mut ancestry = load_ancestry(&self.connection)?;
        if let Some(external) = external_ancestry {
            for (batch_id, dependencies) in external {
                ancestry
                    .entry(*batch_id)
                    .or_insert_with(|| dependencies.clone());
            }
        }
        ancestry.insert(event.batch_id, event.causal_dependency_heads.clone());
        for dependency in &event.causal_dependency_heads {
            if !self.contains_batch(*dependency)? {
                return Err(ProjectionError::MissingDependency(*dependency));
            }
        }
        let current = self.frontier()?;
        if !frontier_contains(&event.exact_frontier, &current, &ancestry)
            || !frontier_contains(&event.exact_frontier, &event.dependency_frontier, &ancestry)
        {
            return Err(ProjectionError::FrontierRegression);
        }

        let dependency_frontier = canonical_frontier_bytes(&event.dependency_frontier)?;
        let exact_frontier = canonical_frontier_bytes(&event.exact_frontier)?;
        let causal_dependencies = encode_batch_ids(&event.causal_dependency_heads)?;
        let sequence = self
            .applied_batch_count()?
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("applied batch sequence overflowed".into()))?;
        let retained_bytes = i64::try_from(event.retained_bytes).map_err(|_| {
            ProjectionError::InvalidAcceptedEvent("retained-byte count exceeds SQLite".into())
        })?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_event(
            &transaction,
            sequence,
            event,
            &dependency_frontier,
            &exact_frontier,
            &causal_dependencies,
            retained_bytes,
        )?;
        if fail_after_insert {
            return Err(ProjectionError::InjectedFailure);
        }
        transaction.execute(
            "UPDATE frontier
             SET exact_frontier = ?1,
                 frontier_digest = ?2,
                 applied_batch_count = ?3
             WHERE singleton = 1",
            params![
                exact_frontier,
                ContentDigest::of(&exact_frontier).as_bytes().as_slice(),
                i64::try_from(sequence).map_err(|_| {
                    ProjectionError::Corrupt("applied batch sequence exceeds SQLite".into())
                })?
            ],
        )?;
        transaction.commit()?;
        Ok(ApplyDisposition::Applied)
    }

    fn validate_event_claim(&self, event: &AcceptedBatchEvent) -> Result<(), ProjectionError> {
        if event.workspace_id != self.claim.workspace_id {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: self.claim.workspace_id,
                found: event.workspace_id,
            });
        }
        if event.lineage_digest != self.claim.lineage_digest {
            return Err(ProjectionError::LineageMismatch {
                expected: self.claim.lineage_digest,
                found: event.lineage_digest,
            });
        }
        Ok(())
    }
}

fn insert_event(
    transaction: &Transaction<'_>,
    sequence: usize,
    event: &AcceptedBatchEvent,
    dependency_frontier: &[u8],
    exact_frontier: &[u8],
    causal_dependencies: &[u8],
    retained_bytes: i64,
) -> Result<(), ProjectionError> {
    transaction.execute(
        "INSERT INTO applied_batches (
             sequence, batch_id, manifest_digest, semantic_effect,
             semantic_effect_digest, dependency_frontier,
             dependency_frontier_digest, exact_frontier,
             exact_frontier_digest, causal_dependency_heads, retained_bytes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            i64::try_from(sequence)
                .map_err(|_| ProjectionError::Corrupt("batch sequence exceeds SQLite".into()))?,
            uuid_blob(&event.batch_id.as_uuid()),
            event.manifest_digest.as_bytes().as_slice(),
            &event.semantic_effect,
            event.semantic_effect_digest.as_bytes().as_slice(),
            dependency_frontier,
            ContentDigest::of(dependency_frontier).as_bytes().as_slice(),
            exact_frontier,
            ContentDigest::of(exact_frontier).as_bytes().as_slice(),
            causal_dependencies,
            retained_bytes,
        ],
    )?;
    Ok(())
}

fn validate_source(
    claim: ProjectionClaim,
    source: &RebuildSource<'_>,
) -> Result<(), ProjectionError> {
    if source.engine.workspace_id() != claim.workspace_id {
        return Err(ProjectionError::WorkspaceMismatch {
            expected: claim.workspace_id,
            found: source.engine.workspace_id(),
        });
    }
    if source.store.workspace_id() != claim.workspace_id {
        return Err(ProjectionError::WorkspaceMismatch {
            expected: claim.workspace_id,
            found: source.store.workspace_id(),
        });
    }
    if source.engine.lineage_digest() != claim.lineage_digest {
        return Err(ProjectionError::LineageMismatch {
            expected: claim.lineage_digest,
            found: source.engine.lineage_digest(),
        });
    }
    if !matches!(
        source.engine.status().workspace(),
        WorkspaceStatus::Operational
    ) {
        return Err(ProjectionError::Rebuild(
            "blocked hot engine cannot authorize a SQLite rebuild".into(),
        ));
    }
    canonical_frontier_bytes(&source.exact_frontier)?;
    Ok(())
}

fn collect_rebuild_events(
    claim: ProjectionClaim,
    source: &RebuildSource<'_>,
) -> Result<Vec<AcceptedBatchEvent>, ProjectionError> {
    let accepted = source
        .engine
        .status()
        .accepted_batch_ids()
        .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
    let accepted_set: BTreeSet<_> = accepted.iter().copied().collect();
    let mut events = Vec::with_capacity(accepted.len());
    for batch_id in accepted {
        let validated = match source.store.inspect_batch(batch_id)? {
            BatchInspection::Ready(validated) => validated,
            BatchInspection::Absent => {
                return Err(ProjectionError::Rebuild(format!(
                    "accepted batch {batch_id} is absent from the object store"
                )));
            }
            BatchInspection::Staged { .. } => {
                return Err(ProjectionError::Rebuild(format!(
                    "accepted batch {batch_id} is partial in the object store"
                )));
            }
        };
        if validated.manifest().lineage_digest() != claim.lineage_digest {
            return Err(ProjectionError::LineageMismatch {
                expected: claim.lineage_digest,
                found: validated.manifest().lineage_digest(),
            });
        }
        for dependency in validated.manifest().causal_dependency_heads() {
            if !accepted_set.contains(dependency) {
                return Err(ProjectionError::Rebuild(format!(
                    "accepted batch {batch_id} depends on non-accepted batch {dependency}"
                )));
            }
        }
        events.push(AcceptedBatchEvent::from_validated(
            &validated,
            source.exact_frontier.clone(),
        )?);
    }
    let ancestry = event_ancestry(&events);
    for event in &events {
        if !frontier_contains(
            &source.exact_frontier,
            &event.dependency_frontier,
            &ancestry,
        ) {
            return Err(ProjectionError::Rebuild(format!(
                "engine frontier does not contain accepted batch {} dependency frontier",
                event.batch_id
            )));
        }
    }
    topological_events(&events)?;
    Ok(events)
}

fn validate_existing(
    path: &Path,
    claim: ProjectionClaim,
    expected: &[AcceptedBatchEvent],
    exact_frontier: &FrontierV2,
) -> Result<(), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open SQLite projection read-only: {error}"))?;
    validate_integrity(&connection).map_err(|error| error.to_string())?;
    validate_schema_and_claim(&connection, claim).map_err(|error| error.to_string())?;
    let found_frontier = read_frontier(&connection).map_err(|error| error.to_string())?;
    if found_frontier != *exact_frontier {
        return Err("SQLite frontier is stale".into());
    }
    let expected_by_id: BTreeMap<_, _> = expected
        .iter()
        .map(|event| (event.batch_id, event))
        .collect();
    let stored = load_all_batches(&connection).map_err(|error| error.to_string())?;
    if stored.len() != expected_by_id.len() {
        return Err(format!(
            "SQLite accepted batch count {} differs from engine count {}",
            stored.len(),
            expected_by_id.len()
        ));
    }
    let ancestry = load_ancestry(&connection).map_err(|error| error.to_string())?;
    let mut prior = FrontierV2::default();
    for (index, record) in stored.iter().enumerate() {
        if record.sequence != (index + 1) as i64 {
            return Err("SQLite applied batch sequence is not contiguous".into());
        }
        let Some(expected) = expected_by_id.get(&record.batch_id) else {
            return Err(format!(
                "SQLite records non-accepted batch {}",
                record.batch_id
            ));
        };
        if !record
            .matches_static(expected)
            .map_err(|error| error.to_string())?
        {
            return Err(format!(
                "SQLite batch {} differs from immutable oplog evidence",
                record.batch_id
            ));
        }
        let frontier_after =
            decode_frontier(&record.exact_frontier).map_err(|error| error.to_string())?;
        if record.exact_frontier_digest
            != ContentDigest::of(&record.exact_frontier)
                .as_bytes()
                .as_slice()
        {
            return Err(format!(
                "SQLite batch {} exact frontier digest is corrupt",
                record.batch_id
            ));
        }
        if !frontier_contains(&frontier_after, &prior, &ancestry)
            || !frontier_contains(&frontier_after, &expected.dependency_frontier, &ancestry)
            || !frontier_contains(exact_frontier, &frontier_after, &ancestry)
        {
            return Err(format!(
                "SQLite batch {} carries a regressing or impossible frontier",
                record.batch_id
            ));
        }
        prior = frontier_after;
    }
    let count: i64 = connection
        .query_row(
            "SELECT applied_batch_count FROM frontier WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count != stored.len() as i64 {
        return Err("SQLite frontier batch count is stale".into());
    }
    Ok(())
}

fn initialize_schema(
    connection: &Connection,
    claim: ProjectionClaim,
) -> Result<(), ProjectionError> {
    connection.execute_batch(&format!(
        "PRAGMA application_id = {SQLITE_APPLICATION_ID};
         PRAGMA user_version = {SQLITE_SCHEMA_VERSION};
         CREATE TABLE meta (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             workspace_id BLOB NOT NULL CHECK (length(workspace_id) = 16),
             lineage_digest BLOB NOT NULL CHECK (length(lineage_digest) = 32),
             oplog_protocol_version INTEGER NOT NULL,
             operation_schema_version INTEGER NOT NULL,
             object_envelope_schema_version INTEGER NOT NULL,
             manifest_encoding_version INTEGER NOT NULL,
             managed_entity_set_version INTEGER NOT NULL
         ) STRICT;
         CREATE TABLE frontier (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             exact_frontier BLOB NOT NULL,
             frontier_digest BLOB NOT NULL CHECK (length(frontier_digest) = 32),
             applied_batch_count INTEGER NOT NULL CHECK (applied_batch_count >= 0)
         ) STRICT;
         CREATE TABLE applied_batches (
             sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
             batch_id BLOB NOT NULL UNIQUE CHECK (length(batch_id) = 16),
             manifest_digest BLOB NOT NULL CHECK (length(manifest_digest) = 32),
             semantic_effect BLOB NOT NULL,
             semantic_effect_digest BLOB NOT NULL CHECK (length(semantic_effect_digest) = 32),
             dependency_frontier BLOB NOT NULL,
             dependency_frontier_digest BLOB NOT NULL
                 CHECK (length(dependency_frontier_digest) = 32),
             exact_frontier BLOB NOT NULL,
             exact_frontier_digest BLOB NOT NULL CHECK (length(exact_frontier_digest) = 32),
             causal_dependency_heads BLOB NOT NULL,
             retained_bytes INTEGER NOT NULL CHECK (retained_bytes >= 0)
         ) STRICT;"
    ))?;
    connection.execute(
        "INSERT INTO meta (
             singleton, workspace_id, lineage_digest, oplog_protocol_version,
             operation_schema_version, object_envelope_schema_version,
             manifest_encoding_version, managed_entity_set_version
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            uuid_blob(&claim.workspace_id.as_uuid()),
            claim.lineage_digest.as_bytes().as_slice(),
            i64::from(claim.oplog_protocol_version),
            i64::from(claim.operation_schema_version),
            i64::from(claim.object_envelope_schema_version),
            i64::from(claim.manifest_encoding_version),
            i64::from(claim.managed_entity_set_version),
        ],
    )?;
    let frontier = canonical_frontier_bytes(&FrontierV2::default())?;
    connection.execute(
        "INSERT INTO frontier (
             singleton, exact_frontier, frontier_digest, applied_batch_count
         ) VALUES (1, ?1, ?2, 0)",
        params![
            &frontier,
            ContentDigest::of(&frontier).as_bytes().as_slice()
        ],
    )?;
    validate_schema_and_claim(connection, claim)?;
    Ok(())
}

fn open_writable(path: &Path) -> Result<Connection, ProjectionError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA trusted_schema = OFF;",
    )?;
    Ok(connection)
}

fn validate_integrity(connection: &Connection) -> Result<(), ProjectionError> {
    let result: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(ProjectionError::Corrupt(format!(
            "SQLite quick_check failed: {result}"
        )));
    }
    Ok(())
}

fn validate_schema_and_claim(
    connection: &Connection,
    claim: ProjectionClaim,
) -> Result<(), ProjectionError> {
    let application_id: u32 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if application_id != SQLITE_APPLICATION_ID {
        return Err(ProjectionError::SchemaMismatch(format!(
            "application_id {application_id:#x} != {SQLITE_APPLICATION_ID:#x}"
        )));
    }
    let user_version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version != SQLITE_SCHEMA_VERSION {
        return Err(ProjectionError::SchemaMismatch(format!(
            "user_version {user_version} != {SQLITE_SCHEMA_VERSION}"
        )));
    }
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(ProjectionError::SchemaMismatch(format!(
            "journal_mode {journal_mode:?} is not WAL"
        )));
    }
    let tables: BTreeSet<String> = {
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let tables = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        tables
    };
    let expected_tables: BTreeSet<String> =
        EXPECTED_TABLES.iter().map(|name| (*name).into()).collect();
    if tables != expected_tables {
        return Err(ProjectionError::SchemaMismatch(format!(
            "unexpected P2.1 tables: {tables:?}"
        )));
    }
    validate_table_columns(connection, "meta", &META_COLUMNS)?;
    validate_table_columns(connection, "frontier", &FRONTIER_COLUMNS)?;
    validate_table_columns(connection, "applied_batches", &APPLIED_BATCH_COLUMNS)?;
    let stored: StoredClaim = connection.query_row(
        "SELECT workspace_id, lineage_digest, oplog_protocol_version,
                operation_schema_version, object_envelope_schema_version,
                manifest_encoding_version, managed_entity_set_version
         FROM meta WHERE singleton = 1",
        [],
        |row| {
            Ok(StoredClaim {
                workspace_id: row.get(0)?,
                lineage_digest: row.get(1)?,
                oplog_protocol_version: row.get(2)?,
                operation_schema_version: row.get(3)?,
                object_envelope_schema_version: row.get(4)?,
                manifest_encoding_version: row.get(5)?,
                managed_entity_set_version: row.get(6)?,
            })
        },
    )?;
    stored.matches(claim)?;
    let row_counts: (i64, i64) = connection.query_row(
        "SELECT
             (SELECT COUNT(*) FROM meta),
             (SELECT COUNT(*) FROM frontier)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if row_counts != (1, 1) {
        return Err(ProjectionError::Corrupt(
            "meta/frontier singleton cardinality is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_table_columns(
    connection: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), ProjectionError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()?;
    if columns != expected {
        return Err(ProjectionError::SchemaMismatch(format!(
            "{table} columns {columns:?} != {expected:?}"
        )));
    }
    Ok(())
}

struct StoredClaim {
    workspace_id: Vec<u8>,
    lineage_digest: Vec<u8>,
    oplog_protocol_version: i64,
    operation_schema_version: i64,
    object_envelope_schema_version: i64,
    manifest_encoding_version: i64,
    managed_entity_set_version: i64,
}

impl StoredClaim {
    fn matches(&self, claim: ProjectionClaim) -> Result<(), ProjectionError> {
        let workspace_id = decode_workspace_id(&self.workspace_id)?;
        if workspace_id != claim.workspace_id {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: claim.workspace_id,
                found: workspace_id,
            });
        }
        let lineage_digest = decode_lineage_digest(&self.lineage_digest)?;
        if lineage_digest != claim.lineage_digest {
            return Err(ProjectionError::LineageMismatch {
                expected: claim.lineage_digest,
                found: lineage_digest,
            });
        }
        let expected = [
            (
                "oplog_protocol_version",
                self.oplog_protocol_version,
                i64::from(claim.oplog_protocol_version),
            ),
            (
                "operation_schema_version",
                self.operation_schema_version,
                i64::from(claim.operation_schema_version),
            ),
            (
                "object_envelope_schema_version",
                self.object_envelope_schema_version,
                i64::from(claim.object_envelope_schema_version),
            ),
            (
                "manifest_encoding_version",
                self.manifest_encoding_version,
                i64::from(claim.manifest_encoding_version),
            ),
            (
                "managed_entity_set_version",
                self.managed_entity_set_version,
                i64::from(claim.managed_entity_set_version),
            ),
        ];
        for (field, found, expected) in expected {
            if found != expected {
                return Err(ProjectionError::ProtocolMismatch {
                    field,
                    expected,
                    found,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct StoredBatch {
    sequence: i64,
    batch_id: BatchId,
    manifest_digest: Vec<u8>,
    semantic_effect: Vec<u8>,
    semantic_effect_digest: Vec<u8>,
    dependency_frontier: Vec<u8>,
    dependency_frontier_digest: Vec<u8>,
    exact_frontier: Vec<u8>,
    exact_frontier_digest: Vec<u8>,
    causal_dependency_heads: Vec<u8>,
    retained_bytes: i64,
}

impl StoredBatch {
    fn matches(&self, event: &AcceptedBatchEvent) -> Result<bool, ProjectionError> {
        Ok(self.matches_static(event)?
            && decode_frontier(&self.exact_frontier)? == event.exact_frontier
            && self.exact_frontier_digest
                == ContentDigest::of(&self.exact_frontier)
                    .as_bytes()
                    .as_slice()
            && self.retained_bytes == event.retained_bytes as i64)
    }

    fn matches_static(&self, event: &AcceptedBatchEvent) -> Result<bool, ProjectionError> {
        let dependency_frontier = decode_frontier(&self.dependency_frontier)?;
        let semantic = SemanticEffect::decode(&self.semantic_effect)
            .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
        if semantic
            .encode()
            .map_err(|error| ProjectionError::Corrupt(error.to_string()))?
            != self.semantic_effect
        {
            return Err(ProjectionError::Corrupt(
                "stored semantic effect is not canonical".into(),
            ));
        }
        Ok(
            self.manifest_digest == event.manifest_digest.as_bytes().as_slice()
                && self.semantic_effect == event.semantic_effect
                && self.semantic_effect_digest
                    == event.semantic_effect_digest.as_bytes().as_slice()
                && dependency_frontier == event.dependency_frontier
                && self.dependency_frontier_digest
                    == ContentDigest::of(&self.dependency_frontier)
                        .as_bytes()
                        .as_slice()
                && decode_batch_ids(&self.causal_dependency_heads)?
                    == event.causal_dependency_heads
                && self.retained_bytes == event.retained_bytes as i64,
        )
    }
}

fn load_batch(
    connection: &Connection,
    batch_id: BatchId,
) -> Result<Option<StoredBatch>, ProjectionError> {
    connection
        .query_row(
            "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                    semantic_effect_digest, dependency_frontier,
                    dependency_frontier_digest, exact_frontier,
                    exact_frontier_digest, causal_dependency_heads, retained_bytes
             FROM applied_batches WHERE batch_id = ?1",
            [uuid_blob(&batch_id.as_uuid())],
            stored_batch_from_row,
        )
        .optional()
        .map_err(ProjectionError::from)
}

fn load_all_batches(connection: &Connection) -> Result<Vec<StoredBatch>, ProjectionError> {
    let mut statement = connection.prepare(
        "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier,
                dependency_frontier_digest, exact_frontier,
                exact_frontier_digest, causal_dependency_heads, retained_bytes
         FROM applied_batches ORDER BY sequence",
    )?;
    let batches = statement
        .query_map([], stored_batch_from_row)?
        .collect::<Result<_, _>>()
        .map_err(ProjectionError::from)?;
    Ok(batches)
}

fn stored_batch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredBatch> {
    let batch_id: Vec<u8> = row.get(1)?;
    let batch_id = decode_batch_id_sql(&batch_id)?;
    Ok(StoredBatch {
        sequence: row.get(0)?,
        batch_id,
        manifest_digest: row.get(2)?,
        semantic_effect: row.get(3)?,
        semantic_effect_digest: row.get(4)?,
        dependency_frontier: row.get(5)?,
        dependency_frontier_digest: row.get(6)?,
        exact_frontier: row.get(7)?,
        exact_frontier_digest: row.get(8)?,
        causal_dependency_heads: row.get(9)?,
        retained_bytes: row.get(10)?,
    })
}

fn read_frontier(connection: &Connection) -> Result<FrontierV2, ProjectionError> {
    let (bytes, digest): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT exact_frontier, frontier_digest FROM frontier WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if digest != ContentDigest::of(&bytes).as_bytes().as_slice() {
        return Err(ProjectionError::Corrupt(
            "frontier digest does not match frontier bytes".into(),
        ));
    }
    decode_frontier(&bytes)
}

fn canonical_frontier_bytes(frontier: &FrontierV2) -> Result<Vec<u8>, ProjectionError> {
    let bytes = serde_json::to_vec(frontier)
        .map_err(|error| ProjectionError::InvalidFrontier(error.to_string()))?;
    if decode_frontier(&bytes)? != *frontier {
        return Err(ProjectionError::InvalidFrontier(
            "frontier did not survive canonical round trip".into(),
        ));
    }
    Ok(bytes)
}

fn decode_frontier(bytes: &[u8]) -> Result<FrontierV2, ProjectionError> {
    let frontier: FrontierV2 = serde_json::from_slice(bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid frontier: {error}")))?;
    let canonical = serde_json::to_vec(&frontier)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    if canonical != bytes {
        return Err(ProjectionError::Corrupt(
            "stored frontier is not canonical".into(),
        ));
    }
    Ok(frontier)
}

fn frontier_contains(
    have: &FrontierV2,
    required: &FrontierV2,
    ancestry: &BTreeMap<BatchId, Vec<BatchId>>,
) -> bool {
    for needed_document in required.documents() {
        let Some(have_document) = have
            .documents()
            .iter()
            .find(|document| document.document_id() == needed_document.document_id())
        else {
            return false;
        };
        for needed_counter in needed_document.peer_counters() {
            let Some(have_counter) = have_document
                .peer_counters()
                .iter()
                .find(|counter| counter.peer_id() == needed_counter.peer_id())
            else {
                return false;
            };
            if have_counter.max_counter() < needed_counter.max_counter() {
                return false;
            }
        }
        for needed_head in needed_document.direct_dependency_heads() {
            if !have_document
                .direct_dependency_heads()
                .iter()
                .any(|have_head| {
                    have_head == needed_head
                        || batch_descends_from(*have_head, *needed_head, ancestry)
                })
            {
                return false;
            }
        }
    }
    true
}

fn batch_descends_from(
    descendant: BatchId,
    ancestor: BatchId,
    ancestry: &BTreeMap<BatchId, Vec<BatchId>>,
) -> bool {
    let mut pending = vec![descendant];
    let mut visited = BTreeSet::new();
    while let Some(batch_id) = pending.pop() {
        if !visited.insert(batch_id) {
            continue;
        }
        let Some(dependencies) = ancestry.get(&batch_id) else {
            continue;
        };
        if dependencies.contains(&ancestor) {
            return true;
        }
        pending.extend(dependencies.iter().copied());
    }
    false
}

fn load_ancestry(
    connection: &Connection,
) -> Result<BTreeMap<BatchId, Vec<BatchId>>, ProjectionError> {
    let mut statement = connection.prepare(
        "SELECT batch_id, causal_dependency_heads FROM applied_batches ORDER BY batch_id",
    )?;
    let rows = statement.query_map([], |row| {
        let batch_id: Vec<u8> = row.get(0)?;
        let dependencies: Vec<u8> = row.get(1)?;
        Ok((batch_id, dependencies))
    })?;
    let mut ancestry = BTreeMap::new();
    for row in rows {
        let (batch_id, dependencies) = row?;
        ancestry.insert(
            decode_batch_id(&batch_id)?,
            decode_batch_ids(&dependencies)?,
        );
    }
    Ok(ancestry)
}

fn event_ancestry(events: &[AcceptedBatchEvent]) -> BTreeMap<BatchId, Vec<BatchId>> {
    events
        .iter()
        .map(|event| (event.batch_id, event.causal_dependency_heads.clone()))
        .collect()
}

fn topological_events(
    events: &[AcceptedBatchEvent],
) -> Result<Vec<&AcceptedBatchEvent>, ProjectionError> {
    let by_id: BTreeMap<_, _> = events.iter().map(|event| (event.batch_id, event)).collect();
    if by_id.len() != events.len() {
        return Err(ProjectionError::Rebuild(
            "duplicate accepted batch IDs in rebuild input".into(),
        ));
    }
    let mut done = BTreeSet::new();
    let mut ordered = Vec::with_capacity(events.len());
    while ordered.len() < events.len() {
        let next = by_id.iter().find(|(batch_id, event)| {
            !done.contains(*batch_id)
                && event
                    .causal_dependency_heads
                    .iter()
                    .all(|dependency| done.contains(dependency))
        });
        let Some((batch_id, event)) = next else {
            return Err(ProjectionError::Rebuild(
                "accepted batch dependency graph is cyclic or incomplete".into(),
            ));
        };
        done.insert(*batch_id);
        ordered.push(*event);
    }
    Ok(ordered)
}

fn encode_batch_ids(batch_ids: &[BatchId]) -> Result<Vec<u8>, ProjectionError> {
    let bytes = serde_json::to_vec(batch_ids)
        .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
    if decode_batch_ids(&bytes)? != batch_ids {
        return Err(ProjectionError::InvalidAcceptedEvent(
            "causal dependency heads are not canonical".into(),
        ));
    }
    Ok(bytes)
}

fn decode_batch_ids(bytes: &[u8]) -> Result<Vec<BatchId>, ProjectionError> {
    let batch_ids: Vec<BatchId> = serde_json::from_slice(bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid batch IDs: {error}")))?;
    if batch_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(ProjectionError::Corrupt(
            "batch IDs are not canonical sorted unique values".into(),
        ));
    }
    let canonical = serde_json::to_vec(&batch_ids)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    if canonical != bytes {
        return Err(ProjectionError::Corrupt(
            "batch IDs are not canonically encoded".into(),
        ));
    }
    Ok(batch_ids)
}

fn prepare_database_parent(path: &Path) -> Result<(), ProjectionError> {
    let name = path
        .file_name()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no file name".into()))?;
    if name.is_empty() {
        return Err(ProjectionError::UnsafePath(
            "database path has an empty file name".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(ProjectionError::UnsafePath(
            "database parent cannot be a symlink".into(),
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ProjectionError::UnsafePath(
                "database path is not a regular no-follow file".into(),
            ));
        }
    }
    Ok(())
}

fn projection_files_exist(path: &Path) -> bool {
    FORENSIC_SUFFIXES
        .iter()
        .any(|suffix| sidecar_path(path, suffix).exists())
}

fn preserve_forensics(path: &Path) -> Result<Vec<ForensicEvidence>, ProjectionError> {
    let token = Uuid::new_v4().simple().to_string();
    let mut evidence = Vec::new();
    for suffix in FORENSIC_SUFFIXES {
        let original = sidecar_path(path, suffix);
        match fs::symlink_metadata(&original) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(ProjectionError::UnsafePath(format!(
                        "projection evidence {} is not a regular file",
                        original.display()
                    )));
                }
                let file_name = original
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| {
                        ProjectionError::UnsafePath("projection evidence path is not UTF-8".into())
                    })?;
                let preserved = original.with_file_name(format!("{file_name}.forensic-{token}"));
                fs::rename(&original, &preserved)?;
                evidence.push(ForensicEvidence {
                    original_path: original,
                    preserved_path: preserved,
                });
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(evidence)
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        path.to_path_buf()
    } else {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        PathBuf::from(value)
    }
}

struct ProcessLease {
    file: File,
}

impl ProcessLease {
    fn acquire(path: &Path, workspace_id: WorkspaceId) -> Result<Self, ProjectionError> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
        let lease_path = path.with_file_name(format!("{file_name}.applier.lock"));
        match fs::symlink_metadata(&lease_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(ProjectionError::UnsafePath(
                    "SQLite applier lease is not a regular no-follow file".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lease_path)?;
        if let Err(error) = file.try_lock_exclusive() {
            if matches!(
                error.kind(),
                ErrorKind::WouldBlock | ErrorKind::PermissionDenied
            ) {
                return Err(ProjectionError::LeaseContended(lease_path));
            }
            return Err(error.into());
        }
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(
            file,
            "workspace={}\npid={}\nplatform={}",
            workspace_id,
            std::process::id(),
            std::env::consts::OS
        )?;
        file.sync_all()?;
        Ok(Self { file })
    }
}

impl Drop for ProcessLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn uuid_blob(uuid: &Uuid) -> Vec<u8> {
    uuid.as_bytes().to_vec()
}

fn decode_workspace_id(bytes: &[u8]) -> Result<WorkspaceId, ProjectionError> {
    Ok(WorkspaceId::from_uuid(decode_uuid(bytes)?))
}

fn decode_batch_id(bytes: &[u8]) -> Result<BatchId, ProjectionError> {
    Ok(BatchId::from_uuid(decode_uuid(bytes)?))
}

fn decode_batch_id_sql(bytes: &[u8]) -> rusqlite::Result<BatchId> {
    let uuid = Uuid::from_slice(bytes).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            bytes.len(),
            rusqlite::types::Type::Blob,
            Box::new(error),
        )
    })?;
    Ok(BatchId::from_uuid(uuid))
}

fn decode_uuid(bytes: &[u8]) -> Result<Uuid, ProjectionError> {
    Uuid::from_slice(bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid UUID bytes: {error}")))
}

fn decode_lineage_digest(bytes: &[u8]) -> Result<LineageDigest, ProjectionError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProjectionError::Corrupt("invalid lineage digest length".into()))?;
    Ok(LineageDigest::from_bytes(bytes))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    Sqlite(String),
    Io(String),
    UnsafePath(String),
    LeaseContended(PathBuf),
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    LineageMismatch {
        expected: LineageDigest,
        found: LineageDigest,
    },
    ProtocolMismatch {
        field: &'static str,
        expected: i64,
        found: i64,
    },
    SchemaMismatch(String),
    Corrupt(String),
    InvalidFrontier(String),
    InvalidAcceptedEvent(String),
    MissingDependency(BatchId),
    FrontierRegression,
    BatchCollision(BatchId),
    Rebuild(String),
    InjectedFailure,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite projection error: {error}"),
            Self::Io(error) => write!(f, "SQLite projection I/O error: {error}"),
            Self::UnsafePath(error) => write!(f, "unsafe SQLite projection path: {error}"),
            Self::LeaseContended(path) => {
                write!(f, "SQLite applier lease is held: {}", path.display())
            }
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::LineageMismatch { expected, found } => {
                write!(f, "lineage mismatch: expected {expected}, found {found}")
            }
            Self::ProtocolMismatch {
                field,
                expected,
                found,
            } => write!(
                f,
                "SQLite claim {field} mismatch: expected {expected}, found {found}"
            ),
            Self::SchemaMismatch(error) => write!(f, "SQLite schema mismatch: {error}"),
            Self::Corrupt(error) => write!(f, "corrupt SQLite projection: {error}"),
            Self::InvalidFrontier(error) => write!(f, "invalid exact frontier: {error}"),
            Self::InvalidAcceptedEvent(error) => write!(f, "invalid accepted event: {error}"),
            Self::MissingDependency(batch_id) => {
                write!(f, "accepted batch dependency {batch_id} is not applied")
            }
            Self::FrontierRegression => {
                write!(
                    f,
                    "accepted event frontier does not contain current/dependency state"
                )
            }
            Self::BatchCollision(batch_id) => {
                write!(
                    f,
                    "accepted batch {batch_id} collides with its SQLite record"
                )
            }
            Self::Rebuild(error) => write!(f, "SQLite rebuild failed: {error}"),
            Self::InjectedFailure => write!(f, "injected SQLite transaction failure"),
        }
    }
}

impl std::error::Error for ProjectionError {}

impl From<rusqlite::Error> for ProjectionError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value.to_string())
    }
}

impl From<std::io::Error> for ProjectionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<super::StoreError> for ProjectionError {
    fn from(value: super::StoreError) -> Self {
        Self::Rebuild(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::params;

    use super::*;
    use crate::oplog::{
        AuthorBatch, BatchCausalDot, BlockId, BlockLocation, CausalPeerId, CrdtPeerCounter,
        CrdtPeerId, DeviceId, DocumentDependencies, DocumentId, ManagedPath, OperationBatch,
        OperationObject, OperationTransaction, PageId, PreparedBatch, SemanticOperation, SessionId,
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-sqlite-frontier-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
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

    #[derive(Clone, Copy)]
    struct TestIds {
        workspace: WorkspaceId,
        lineage: LineageDigest,
        catalog: DocumentId,
        document: DocumentId,
        page: PageId,
        block: BlockId,
    }

    impl TestIds {
        fn new(seed: u128) -> Self {
            Self {
                workspace: WorkspaceId::from_uuid(uuid(seed + 1)),
                lineage: LineageDigest::of(&seed.to_be_bytes()),
                catalog: DocumentId::from_uuid(uuid(seed + 2)),
                document: DocumentId::from_uuid(uuid(seed + 3)),
                page: PageId::from_uuid(uuid(seed + 4)),
                block: BlockId::from_uuid(uuid(seed + 5)),
            }
        }

        fn claim(self) -> ProjectionClaim {
            ProjectionClaim::current(self.workspace, self.lineage)
        }

        fn engine(self) -> ShardedHotEngine {
            ShardedHotEngine::new(self.workspace, self.lineage, self.catalog)
        }
    }

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn batch(value: u128) -> BatchId {
        BatchId::from_uuid(uuid(value))
    }

    fn author(value: u128) -> AuthorBatch {
        AuthorBatch {
            batch_id: batch(value),
            author_device_id: DeviceId::from_uuid(uuid(value + 10_000)),
            author_session_id: SessionId::from_uuid(uuid(value + 20_000)),
            crdt_peer_id: CrdtPeerId::from_u64(value as u64),
        }
    }

    fn frontier(document_id: DocumentId, counter: u64, heads: Vec<BatchId>) -> FrontierV2 {
        FrontierV2::new(vec![DocumentDependencies::new(
            document_id,
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(7), counter)],
            heads,
        )
        .unwrap()])
        .unwrap()
    }

    fn open_empty(dir: &TestDir, ids: TestIds) -> (SqliteFrontier, ShardedHotEngine, ObjectStore) {
        let engine = ids.engine();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let opened = SqliteFrontier::open_or_rebuild(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            opened.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        );
        (opened.database, engine, store)
    }

    fn fake_validated(
        store: &ObjectStore,
        ids: TestIds,
        batch_id: BatchId,
        causal_dependencies: Vec<BatchId>,
        dependency_frontier: FrontierV2,
    ) -> ValidatedBatch {
        let effect = SemanticEffect::new(Vec::new(), Vec::new(), Vec::new())
            .unwrap()
            .encode()
            .unwrap();
        let semantic = OperationObject::new(
            ids.workspace,
            ids.catalog,
            ObjectKind::SemanticEffect,
            effect.clone(),
        )
        .unwrap();
        let update = OperationObject::new(
            ids.workspace,
            ids.document,
            ObjectKind::CrdtUpdate,
            format!("test update {batch_id}").into_bytes(),
        )
        .unwrap();
        let objects = vec![semantic, update];
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let device = DeviceId::from_uuid(uuid(batch_id.as_uuid().as_u128() + 30_000));
        let manifest = OperationBatch::new_with_causality(
            ids.workspace,
            ids.lineage,
            batch_id,
            device,
            SessionId::from_uuid(uuid(batch_id.as_uuid().as_u128() + 40_000)),
            BatchCausalDot::new(CausalPeerId::from_device_id(device), 1).unwrap(),
            causal_dependencies,
            dependency_frontier,
            SemanticEffectDigest::of(&effect),
            descriptors,
        )
        .unwrap();
        let prepared = PreparedBatch::new(manifest, objects).unwrap();
        store.publish_prepared(&prepared).unwrap();
        match store.inspect_batch(batch_id).unwrap() {
            BatchInspection::Ready(validated) => validated,
            other => panic!("expected ready test batch, found {other:?}"),
        }
    }

    fn root_and_child_events(
        store: &ObjectStore,
        ids: TestIds,
    ) -> (AcceptedBatchEvent, AcceptedBatchEvent) {
        let root_id = batch(100);
        let child_id = batch(101);
        let root = fake_validated(store, ids, root_id, Vec::new(), FrontierV2::default());
        let root_frontier = frontier(ids.document, 1, vec![root_id]);
        let child = fake_validated(store, ids, child_id, vec![root_id], root_frontier.clone());
        (
            AcceptedBatchEvent::from_validated(&root, root_frontier).unwrap(),
            AcceptedBatchEvent::from_validated(&child, frontier(ids.document, 2, vec![child_id]))
                .unwrap(),
        )
    }

    #[test]
    fn schema_claim_wal_and_transaction_rollback_are_atomic() {
        let ids = TestIds::new(1_000);
        let dir = TestDir::new("transaction");
        let (mut database, engine, store) = open_empty(&dir, ids);
        let application_id: u32 = database
            .connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        let user_version: u32 = database
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let journal_mode: String = database
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(application_id, SQLITE_APPLICATION_ID);
        assert_eq!(user_version, SQLITE_SCHEMA_VERSION);
        assert_eq!(journal_mode, "wal");

        let (root, _) = root_and_child_events(&store, ids);
        assert_eq!(
            database.apply_internal(&root, true, None),
            Err(ProjectionError::InjectedFailure)
        );
        assert_eq!(database.applied_batch_count().unwrap(), 0);
        assert_eq!(database.frontier().unwrap(), FrontierV2::default());
        let database_path = database.path().to_path_buf();
        drop(database);
        let reopened = SqliteFrontier::open_or_rebuild(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
        let mut database = reopened.database;
        assert_eq!(
            database.apply_accepted(&root).unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(database.applied_batch_count().unwrap(), 1);
        assert_eq!(database.frontier().unwrap(), *root.exact_frontier());
    }

    #[test]
    fn exact_frontier_containment_is_monotonic_and_ancestry_aware() {
        let ids = TestIds::new(2_000);
        let root = batch(200);
        let child = batch(201);
        let required = frontier(ids.document, 1, vec![root]);
        let have = frontier(ids.document, 2, vec![child]);
        let ancestry = BTreeMap::from([(child, vec![root]), (root, Vec::new())]);
        assert!(frontier_contains(&have, &required, &ancestry));
        assert!(!frontier_contains(&required, &have, &ancestry));

        let unrelated = frontier(ids.document, 2, vec![batch(202)]);
        assert!(!frontier_contains(&unrelated, &required, &ancestry));
        let missing_peer = FrontierV2::new(vec![DocumentDependencies::new(
            ids.document,
            Vec::new(),
            vec![child],
        )
        .unwrap()])
        .unwrap();
        assert!(!frontier_contains(&missing_peer, &required, &ancestry));
    }

    #[test]
    fn duplicate_apply_is_idempotent_and_collisions_or_regressions_fail_closed() {
        let ids = TestIds::new(3_000);
        let dir = TestDir::new("idempotence");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (root, child) = root_and_child_events(&store, ids);
        assert_eq!(
            database.apply_accepted(&root).unwrap(),
            ApplyDisposition::Applied
        );
        assert!(database.contains_frontier(root.exact_frontier()).unwrap());
        assert!(!database.contains_frontier(child.exact_frontier()).unwrap());
        assert_eq!(
            database.apply_accepted(&root).unwrap(),
            ApplyDisposition::Duplicate
        );
        assert_eq!(database.applied_batch_count().unwrap(), 1);

        let mut collision = root.clone();
        collision.semantic_effect.push(0);
        assert_eq!(
            database.apply_accepted(&collision),
            Err(ProjectionError::BatchCollision(root.batch_id()))
        );

        assert_eq!(
            database.apply_accepted(&child).unwrap(),
            ApplyDisposition::Applied
        );
        assert!(database.contains_frontier(root.exact_frontier()).unwrap());
        assert!(database.contains_frontier(child.exact_frontier()).unwrap());
        let sibling = fake_validated(
            &store,
            ids,
            batch(102),
            vec![root.batch_id()],
            root.exact_frontier().clone(),
        );
        let regressing =
            AcceptedBatchEvent::from_validated(&sibling, root.exact_frontier().clone()).unwrap();
        assert_eq!(
            database.apply_accepted(&regressing),
            Err(ProjectionError::FrontierRegression)
        );
    }

    #[test]
    fn overlay_reorders_dependencies_and_enforces_both_limits() {
        let ids = TestIds::new(4_000);
        let dir = TestDir::new("overlay");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (root, child) = root_and_child_events(&store, ids);
        let mut overlay = TailOverlay::default();
        assert!(overlay.try_enqueue(child.clone()).unwrap());
        assert!(overlay.try_enqueue(root.clone()).unwrap());
        assert_eq!(overlay.drain_ready(&mut database, usize::MAX).unwrap(), 2);
        assert_eq!(database.frontier().unwrap(), *child.exact_frontier());
        assert_eq!(overlay.status().unapplied_batches, 0);

        let mut count_limited = TailOverlay::default();
        let mut tiny = root.clone();
        tiny.retained_bytes = 1;
        for value in 0..TAIL_MAX_BATCHES {
            tiny.batch_id = batch(50_000 + value as u128);
            assert!(count_limited.try_enqueue(tiny.clone()).unwrap());
        }
        assert!(count_limited.status().backpressured);
        tiny.batch_id = batch(70_000);
        assert!(matches!(
            count_limited.try_enqueue(tiny),
            Err(TailOverlayError::Backpressure(TailOverlayStatus {
                backpressured: true,
                ..
            }))
        ));

        let mut byte_limited = TailOverlay::default();
        let mut full = root;
        full.retained_bytes = TAIL_MAX_BYTES;
        assert!(byte_limited.try_enqueue(full.clone()).unwrap());
        assert!(byte_limited.status().backpressured);
        full.batch_id = batch(70_001);
        assert!(matches!(
            byte_limited.try_enqueue(full),
            Err(TailOverlayError::Backpressure(TailOverlayStatus {
                backpressured: true,
                ..
            }))
        ));
    }

    #[test]
    fn missing_dependency_and_workspace_or_lineage_mismatch_are_rejected() {
        let ids = TestIds::new(5_000);
        let dir = TestDir::new("fences");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (_, child) = root_and_child_events(&store, ids);
        assert_eq!(
            database.apply_accepted(&child),
            Err(ProjectionError::MissingDependency(batch(100)))
        );

        let mut foreign_workspace = child.clone();
        foreign_workspace.workspace_id = TestIds::new(6_000).workspace;
        assert!(matches!(
            database.apply_accepted(&foreign_workspace),
            Err(ProjectionError::WorkspaceMismatch { .. })
        ));
        let mut foreign_lineage = child;
        foreign_lineage.lineage_digest = LineageDigest::of(b"foreign");
        assert!(matches!(
            database.apply_accepted(&foreign_lineage),
            Err(ProjectionError::LineageMismatch { .. })
        ));
    }

    #[test]
    fn lease_contention_and_drop_recovery_are_process_scoped() {
        let ids = TestIds::new(7_000);
        let dir = TestDir::new("lease");
        let engine = ids.engine();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let database_path = dir.path().join("frontier.sqlite");
        let first = SqliteFrontier::open_or_rebuild(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild(
                &database_path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        let foreign_ids = TestIds::new(7_100);
        let foreign_engine = foreign_ids.engine();
        let foreign_store =
            ObjectStore::open(&dir.path().join("foreign-objects"), foreign_ids.workspace).unwrap();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild(
                &database_path,
                foreign_ids.claim(),
                RebuildSource::new(&foreign_engine, &foreign_store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(first);
        let recovered = SqliteFrontier::open_or_rebuild(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered.recovery, ProjectionRecovery::OpenedExisting);
    }

    #[test]
    fn delete_and_rebuild_from_production_engine_store_is_semantically_equivalent() {
        let ids = TestIds::new(8_000);
        let dir = TestDir::new("rebuild");
        let store_path = dir.path().join("objects");
        let store = ObjectStore::open(&store_path, ids.workspace).unwrap();
        let author_engine = ids.engine();
        let transaction = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                path: ManagedPath::parse("pages/SQLite.md").unwrap(),
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: ids.block,
                    home_document_id: ids.document,
                },
                page_id: ids.page,
                parent: None,
                order: "a".into(),
                content: "authoritative content".into(),
            },
        ])
        .unwrap();
        let prepared = author_engine
            .prepare_transaction(author(8_100), &transaction)
            .unwrap();
        store.publish_prepared(&prepared).unwrap();
        let reader = ObjectStore::open(&store_path, ids.workspace).unwrap();
        let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
        assert!(matches!(
            engine
                .stage_archive_batch(prepared.manifest().batch_id())
                .unwrap()
                .disposition,
            super::super::BatchDisposition::Accepted { .. }
        ));
        let accepted_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        assert_eq!(accepted_event.batch_id(), prepared.manifest().batch_id());
        let probe = engine
            .prepare_transaction(
                author(8_101),
                &OperationTransaction::new(vec![
                    SemanticOperation::EditPagePath {
                        page_id: ids.page,
                        path: ManagedPath::parse("pages/SQLite-renamed.md").unwrap(),
                    },
                    SemanticOperation::EditBlockContent {
                        block: BlockLocation {
                            block_id: ids.block,
                            home_document_id: ids.document,
                        },
                        content: "probe".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        let exact_frontier = probe.manifest().dependency_frontier().clone();
        assert_eq!(engine.exact_frontier().unwrap(), exact_frontier);
        assert_eq!(accepted_event.exact_frontier(), &exact_frontier);
        let expected_snapshot = engine.canonical_snapshot().unwrap();
        let database_path = dir.path().join("frontier.sqlite");

        let first = SqliteFrontier::open_or_rebuild(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(first.database.applied_batch_count().unwrap(), 1);
        let first_digest = first.database.semantic_projection_digest().unwrap();
        drop(first);
        remove_projection_files(&database_path);

        let rebuilt = SqliteFrontier::open_or_rebuild(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            rebuilt.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 1 }
        );
        assert_eq!(rebuilt.database.frontier().unwrap(), exact_frontier);
        assert_eq!(
            rebuilt.database.semantic_projection_digest().unwrap(),
            first_digest
        );

        let mut clean_replay = ids.engine();
        for manifest in store.committed_manifests().unwrap() {
            clean_replay
                .stage_from_store(&store, manifest.batch_id())
                .unwrap();
        }
        assert_eq!(
            clean_replay.canonical_snapshot().unwrap(),
            expected_snapshot
        );
    }

    #[test]
    fn corruption_and_truncation_are_preserved_before_rebuild() {
        for (label, bytes) in [
            ("corrupt", b"not a SQLite database".as_slice()),
            ("truncated", b"SQLite format 3\0short".as_slice()),
        ] {
            let ids = TestIds::new(if label == "corrupt" { 9_000 } else { 9_100 });
            let dir = TestDir::new(label);
            let (database, engine, store) = open_empty(&dir, ids);
            let path = database.path().to_path_buf();
            drop(database);
            fs::write(&path, bytes).unwrap();
            let rebuilt = SqliteFrontier::open_or_rebuild(
                &path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            )
            .unwrap();
            let ProjectionRecovery::RebuiltPreservingEvidence { evidence, .. } = &rebuilt.recovery
            else {
                panic!("expected forensic rebuild, found {:?}", rebuilt.recovery);
            };
            let database_evidence = evidence
                .iter()
                .find(|item| item.original_path == path)
                .unwrap();
            assert_eq!(fs::read(&database_evidence.preserved_path).unwrap(), bytes);
            assert_eq!(rebuilt.database.frontier().unwrap(), FrontierV2::default());
        }
    }

    #[test]
    fn stale_frontier_and_protocol_claim_are_preserved_and_rebuilt() {
        for protocol_stale in [false, true] {
            let ids = TestIds::new(if protocol_stale { 10_100 } else { 10_000 });
            let dir = TestDir::new(if protocol_stale {
                "stale-protocol"
            } else {
                "stale-frontier"
            });
            let (database, engine, store) = open_empty(&dir, ids);
            let path = database.path().to_path_buf();
            drop(database);
            let connection = Connection::open(&path).unwrap();
            if protocol_stale {
                connection
                    .execute(
                        "UPDATE meta SET oplog_protocol_version = ?1 WHERE singleton = 1",
                        [i64::from(OPLOG_PROTOCOL_VERSION + 1)],
                    )
                    .unwrap();
            } else {
                let stale =
                    canonical_frontier_bytes(&frontier(ids.document, 1, vec![batch(10_200)]))
                        .unwrap();
                connection
                    .execute(
                        "UPDATE frontier
                         SET exact_frontier = ?1, frontier_digest = ?2
                         WHERE singleton = 1",
                        params![&stale, ContentDigest::of(&stale).as_bytes().as_slice()],
                    )
                    .unwrap();
            }
            drop(connection);
            let rebuilt = SqliteFrontier::open_or_rebuild(
                &path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            )
            .unwrap();
            assert!(matches!(
                rebuilt.recovery,
                ProjectionRecovery::RebuiltPreservingEvidence { .. }
            ));
            assert_eq!(rebuilt.database.frontier().unwrap(), FrontierV2::default());
        }
    }

    fn remove_projection_files(path: &Path) {
        for suffix in FORENSIC_SUFFIXES {
            let candidate = sidecar_path(path, suffix);
            match fs::remove_file(candidate) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => panic!("cannot remove test projection: {error}"),
            }
        }
    }
}
