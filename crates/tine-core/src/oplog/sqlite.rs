//! Disposable SQLite frontier projection for the sparse operation log.
//!
//! This module deliberately accepts only already-accepted operation events. It
//! has no mutation-authoring API and is never part of keystroke durability.
//! Callers place the database and its workspace lease in device-local app data;
//! neither path is derived from, or requires access to, the shared graph.
//!
//! The lease uses the platform's advisory file-lock primitive through `fs2`.
//! Dropping the applier or terminating its process releases the lock on Linux,
//! macOS, Windows, and Android. The small lock file remains as diagnostic
//! metadata, but never decides ownership by its contents.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use cap_std::{ambient_authority, fs::Dir as CapDir};
use fs2::FileExt as _;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior,
};
use uuid::Uuid;

use super::{
    AcceptedFrontierRoot, BatchId, BatchInspection, ContentDigest, DocumentDependencies,
    DocumentId, FrontierV2, LineageDigest, ObjectKind, ObjectStore, SemanticEffect,
    SemanticEffectDigest, ShardedHotEngine, ValidatedBatch, WorkspaceId, WorkspaceStatus,
    MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION,
    OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};

pub const SQLITE_APPLICATION_ID: u32 = 0x5449_4e45;
pub const SQLITE_SCHEMA_VERSION: u32 = 3;
pub const TAIL_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const TAIL_MAX_BATCHES: usize = 10_000;

const EXPECTED_TABLES: [&str; 4] = ["applied_batches", "frontier", "frontier_documents", "meta"];
const EXPECTED_INDEXES: [&str; 2] = [
    "applied_batches_acceptance_sequence_uq",
    "applied_batches_batch_id_uq",
];
const META_DDL: &str = "CREATE TABLE meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    workspace_id BLOB NOT NULL CHECK (length(workspace_id) = 16),
    lineage_digest BLOB NOT NULL CHECK (length(lineage_digest) = 32),
    oplog_protocol_version INTEGER NOT NULL,
    operation_schema_version INTEGER NOT NULL,
    object_envelope_schema_version INTEGER NOT NULL,
    manifest_encoding_version INTEGER NOT NULL,
    managed_entity_set_version INTEGER NOT NULL
) STRICT";
const FRONTIER_DDL: &str = "CREATE TABLE frontier (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    frontier_root BLOB NOT NULL,
    frontier_root_digest BLOB NOT NULL CHECK (length(frontier_root_digest) = 32),
    applied_batch_count INTEGER NOT NULL CHECK (applied_batch_count >= 0)
) STRICT";
const FRONTIER_DOCUMENTS_DDL: &str = "CREATE TABLE frontier_documents (
    document_id BLOB PRIMARY KEY CHECK (length(document_id) = 16),
    dependencies BLOB NOT NULL,
    dependencies_digest BLOB NOT NULL CHECK (length(dependencies_digest) = 32)
) STRICT";
const APPLIED_BATCHES_DDL: &str = "CREATE TABLE applied_batches (
    sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
    batch_id BLOB NOT NULL CHECK (length(batch_id) = 16),
    manifest_digest BLOB NOT NULL CHECK (length(manifest_digest) = 32),
    semantic_effect BLOB NOT NULL,
    semantic_effect_digest BLOB NOT NULL CHECK (length(semantic_effect_digest) = 32),
    dependency_frontier BLOB NOT NULL,
    dependency_frontier_digest BLOB NOT NULL
        CHECK (length(dependency_frontier_digest) = 32),
    prior_frontier_root BLOB NOT NULL,
    prior_frontier_root_digest BLOB NOT NULL
        CHECK (length(prior_frontier_root_digest) = 32),
    post_frontier_root BLOB NOT NULL,
    post_frontier_root_digest BLOB NOT NULL
        CHECK (length(post_frontier_root_digest) = 32),
    affected_documents BLOB NOT NULL,
    affected_documents_digest BLOB NOT NULL
        CHECK (length(affected_documents_digest) = 32),
    causal_dependency_heads BLOB NOT NULL,
    acceptance_sequence INTEGER NOT NULL CHECK (acceptance_sequence > 0),
    retained_bytes INTEGER NOT NULL CHECK (retained_bytes >= 0)
) STRICT";
const BATCH_ID_INDEX_DDL: &str =
    "CREATE UNIQUE INDEX applied_batches_batch_id_uq ON applied_batches(batch_id)";
const ACCEPTANCE_SEQUENCE_INDEX_DDL: &str = "CREATE UNIQUE INDEX \
    applied_batches_acceptance_sequence_uq ON applied_batches(acceptance_sequence)";
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
    "frontier_root",
    "frontier_root_digest",
    "applied_batch_count",
];
const FRONTIER_DOCUMENT_COLUMNS: [&str; 3] = ["document_id", "dependencies", "dependencies_digest"];
const APPLIED_BATCH_COLUMNS: [&str; 16] = [
    "sequence",
    "batch_id",
    "manifest_digest",
    "semantic_effect",
    "semantic_effect_digest",
    "dependency_frontier",
    "dependency_frontier_digest",
    "prior_frontier_root",
    "prior_frontier_root_digest",
    "post_frontier_root",
    "post_frontier_root_digest",
    "affected_documents",
    "affected_documents_digest",
    "causal_dependency_heads",
    "acceptance_sequence",
    "retained_bytes",
];
const FORENSIC_SUFFIXES: [&str; 3] = ["", "-wal", "-shm"];
const FORENSIC_NAMES: [&str; 3] = ["database", "wal", "shm"];

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
    event_binding_digest: ContentDigest,
    semantic_effect: Vec<u8>,
    semantic_effect_digest: SemanticEffectDigest,
    dependency_frontier: FrontierV2,
    prior_frontier_root: AcceptedFrontierRoot,
    post_frontier_root: AcceptedFrontierRoot,
    affected_documents: Vec<DocumentDependencies>,
    acceptance_sequence: u64,
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
        let evidence = engine
            .accepted_batch_evidence(batch_id)
            .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
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
        let manifest_digest =
            ContentDigest::of(&validated.manifest().encode().map_err(|error| {
                ProjectionError::InvalidAcceptedEvent(format!(
                    "cannot encode accepted manifest {batch_id}: {error}"
                ))
            })?);
        if manifest_digest != evidence.manifest_fingerprint() {
            return Err(ProjectionError::ManifestMismatch {
                batch_id,
                expected: evidence.manifest_fingerprint(),
                found: manifest_digest,
            });
        }
        if evidence.post_frontier_root().has_persistent_point_index() {
            for document in evidence.affected_documents() {
                let authenticated = engine
                    .accepted_frontier_document(
                        evidence.post_frontier_root(),
                        document.document_id(),
                    )
                    .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
                if authenticated.as_ref() != Some(document) {
                    return Err(ProjectionError::InvalidAcceptedEvent(format!(
                        "accepted batch {batch_id} affected document {} is not bound to its frontier root",
                        document.document_id()
                    )));
                }
            }
        }
        Self::from_validated(&validated, &evidence)
    }

    fn from_validated(
        batch: &ValidatedBatch,
        evidence: &super::AcceptedBatchEvidence,
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
        let event_binding_digest = super::AcceptedBatchEvidence::binding_digest_for(
            manifest.batch_id(),
            ContentDigest::of(&manifest_bytes),
            semantic_effect_digest,
            manifest.dependency_frontier(),
            manifest.causal_dependency_heads(),
        )
        .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
        if event_binding_digest != evidence.event_binding_digest() {
            return Err(ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} event binding differs from its frontier evidence",
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
        let updated_documents = batch
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::CrdtUpdate)
            .map(|object| object.document_id())
            .collect::<BTreeSet<_>>();
        let evidenced_documents = evidence
            .affected_documents()
            .iter()
            .map(DocumentDependencies::document_id)
            .collect::<BTreeSet<_>>();
        if updated_documents != evidenced_documents {
            return Err(ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} affected-document evidence differs from its CRDT updates",
                manifest.batch_id()
            )));
        }
        canonical_frontier_root_bytes(evidence.prior_frontier_root())?;
        canonical_frontier_root_bytes(evidence.post_frontier_root())?;
        canonical_affected_documents_bytes(evidence.affected_documents())?;
        Ok(Self {
            workspace_id: manifest.workspace_id(),
            lineage_digest: manifest.lineage_digest(),
            batch_id: manifest.batch_id(),
            manifest_digest: ContentDigest::of(&manifest_bytes),
            event_binding_digest,
            semantic_effect,
            semantic_effect_digest,
            dependency_frontier: manifest.dependency_frontier().clone(),
            prior_frontier_root: evidence.prior_frontier_root().clone(),
            post_frontier_root: evidence.post_frontier_root().clone(),
            affected_documents: evidence.affected_documents().to_vec(),
            acceptance_sequence: evidence.acceptance_sequence(),
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

    pub const fn prior_frontier_root(&self) -> &AcceptedFrontierRoot {
        &self.prior_frontier_root
    }

    pub const fn post_frontier_root(&self) -> &AcceptedFrontierRoot {
        &self.post_frontier_root
    }

    pub fn affected_documents(&self) -> &[DocumentDependencies] {
        &self.affected_documents
    }

    #[cfg(test)]
    fn exact_frontier(&self) -> FrontierV2 {
        FrontierV2::new(self.affected_documents.clone())
            .expect("test event affected documents are canonical")
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    pub fn causal_dependency_heads(&self) -> &[BatchId] {
        &self.causal_dependency_heads
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRuntimeLeaseRoot {
    path: PathBuf,
    workspace_id: WorkspaceId,
}

impl WorkspaceRuntimeLeaseRoot {
    pub fn open(path: &Path, workspace_id: WorkspaceId) -> Result<Self, ProjectionError> {
        let path = prepare_runtime_lease_root(path, workspace_id)?;
        Ok(Self { path, workspace_id })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
}

pub struct RebuildSource<'a> {
    engine: &'a ShardedHotEngine,
    store: &'a ObjectStore,
    exact_frontier_root: AcceptedFrontierRoot,
    accepted_batch_count: u64,
}

impl<'a> RebuildSource<'a> {
    pub fn new(
        engine: &'a ShardedHotEngine,
        store: &'a ObjectStore,
    ) -> Result<Self, ProjectionError> {
        let exact_frontier_root = engine
            .accepted_frontier_root()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        let accepted_batch_count = engine
            .accepted_batch_count()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        Ok(Self {
            engine,
            store,
            exact_frontier_root,
            accepted_batch_count,
        })
    }

    fn accepted_event_at(
        &self,
        acceptance_sequence: u64,
    ) -> Result<AcceptedBatchEvent, ProjectionError> {
        let batch_id = self
            .engine
            .accepted_batch_id_at(acceptance_sequence)
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?
            .ok_or_else(|| {
                ProjectionError::Rebuild(format!(
                    "accepted history is missing sequence {acceptance_sequence}"
                ))
            })?;
        let event = AcceptedBatchEvent::from_accepted(self.engine, self.store, batch_id)?;
        if event.acceptance_sequence != acceptance_sequence {
            return Err(ProjectionError::Rebuild(format!(
                "accepted batch {batch_id} is indexed at sequence {acceptance_sequence} but carries {}",
                event.acceptance_sequence
            )));
        }
        Ok(event)
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
    pub rebuild: RebuildInstrumentation,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RebuildInstrumentation {
    pub accepted_events_validated: usize,
    pub accepted_events_applied: usize,
    pub max_live_events: usize,
    pub max_live_evidence_records: usize,
    pub ancestry_full_scans: usize,
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
    UnknownReservation,
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
            Self::UnknownReservation => write!(f, "tail mutation reservation is not active"),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TailReservation {
    id: u64,
    retained_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TailDescriptor {
    batch_id: BatchId,
    manifest_digest: ContentDigest,
    retained_bytes: usize,
}

#[derive(Default)]
pub struct TailOverlay {
    hot_descriptors: BTreeMap<u64, TailDescriptor>,
    retained_bytes: usize,
    authoritative_through: u64,
    applied_through: u64,
    descriptor_overflow: bool,
    reservations: BTreeMap<u64, usize>,
    reserved_bytes: usize,
    next_reservation_id: u64,
}

impl TailOverlay {
    #[cfg(test)]
    fn hot_descriptor_count(&self) -> usize {
        self.hot_descriptors.len()
    }

    pub fn status(&self) -> TailOverlayStatus {
        let authoritative_pending = self
            .authoritative_through
            .saturating_sub(self.applied_through);
        TailOverlayStatus {
            unapplied_batches: usize::try_from(authoritative_pending)
                .unwrap_or(usize::MAX)
                .saturating_add(self.reservations.len()),
            retained_bytes: self.retained_bytes.saturating_add(self.reserved_bytes),
            backpressured: self.descriptor_overflow
                || usize::try_from(authoritative_pending)
                    .unwrap_or(usize::MAX)
                    .saturating_add(self.reservations.len())
                    >= TAIL_MAX_BATCHES
                || self.retained_bytes.saturating_add(self.reserved_bytes) >= TAIL_MAX_BYTES,
        }
    }

    /// Reserve bounded projection capacity before exposing a local mutation.
    ///
    /// `retained_bytes` must be an upper bound for the accepted event's encoded
    /// manifest and objects. A single event larger than the byte cap therefore
    /// cannot become locally authoritative through this admission path.
    pub fn reserve_mutation(
        &mut self,
        retained_bytes: usize,
    ) -> Result<TailReservation, TailOverlayError> {
        let next_batches = self
            .authoritative_through
            .saturating_sub(self.applied_through)
            .try_into()
            .unwrap_or(usize::MAX)
            .saturating_add(self.reservations.len())
            .saturating_add(1);
        let next_bytes = self
            .retained_bytes
            .saturating_add(self.reserved_bytes)
            .saturating_add(retained_bytes);
        if next_batches > TAIL_MAX_BATCHES || next_bytes > TAIL_MAX_BYTES {
            return Err(TailOverlayError::Backpressure(TailOverlayStatus {
                unapplied_batches: next_batches,
                retained_bytes: next_bytes,
                backpressured: true,
            }));
        }
        self.next_reservation_id = self.next_reservation_id.wrapping_add(1);
        if self.next_reservation_id == 0 {
            self.next_reservation_id = 1;
        }
        while self.reservations.contains_key(&self.next_reservation_id) {
            self.next_reservation_id = self.next_reservation_id.wrapping_add(1);
            if self.next_reservation_id == 0 {
                self.next_reservation_id = 1;
            }
        }
        let reservation = TailReservation {
            id: self.next_reservation_id,
            retained_bytes,
        };
        self.reservations.insert(reservation.id, retained_bytes);
        self.reserved_bytes = self.reserved_bytes.saturating_add(retained_bytes);
        Ok(reservation)
    }

    pub fn cancel_reservation(
        &mut self,
        reservation: TailReservation,
    ) -> Result<(), TailOverlayError> {
        let Some(retained_bytes) = self.reservations.remove(&reservation.id) else {
            return Err(TailOverlayError::UnknownReservation);
        };
        debug_assert_eq!(retained_bytes, reservation.retained_bytes);
        self.reserved_bytes = self.reserved_bytes.saturating_sub(retained_bytes);
        Ok(())
    }

    /// Convert a pre-acceptance reservation into an authoritative tail event.
    /// The event remains retained even if its actual encoding exceeded the
    /// caller's upper bound, because acceptance is already authoritative.
    pub fn enqueue_reserved(
        &mut self,
        reservation: TailReservation,
        database: &mut SqliteFrontier,
        event: AcceptedBatchEvent,
    ) -> Result<bool, TailOverlayError> {
        let Some(retained_bytes) = self.reservations.remove(&reservation.id) else {
            return Err(TailOverlayError::UnknownReservation);
        };
        debug_assert_eq!(retained_bytes, reservation.retained_bytes);
        self.reserved_bytes = self.reserved_bytes.saturating_sub(retained_bytes);
        self.observe_authoritative(database, &event)
    }

    /// Observe an already-authoritative local or provider event. The durable
    /// accepted-history sequence remains the backlog; RAM retains only bounded
    /// descriptors and can therefore discard stale duplicates immediately.
    pub fn try_enqueue(
        &mut self,
        database: &mut SqliteFrontier,
        event: &AcceptedBatchEvent,
    ) -> Result<bool, TailOverlayError> {
        self.observe_authoritative(database, event)
    }

    fn observe_authoritative(
        &mut self,
        database: &mut SqliteFrontier,
        event: &AcceptedBatchEvent,
    ) -> Result<bool, TailOverlayError> {
        let applied = u64::try_from(database.applied_batch_count()?)
            .map_err(|_| ProjectionError::Corrupt("applied count exceeds u64".into()))?;
        self.applied_through = self.applied_through.max(applied);
        if event.acceptance_sequence <= applied {
            return match database.apply_accepted(event)? {
                ApplyDisposition::Duplicate => Ok(false),
                ApplyDisposition::Applied => Err(ProjectionError::Corrupt(
                    "stale provider event unexpectedly advanced SQLite".into(),
                )
                .into()),
            };
        }
        let descriptor = TailDescriptor {
            batch_id: event.batch_id,
            manifest_digest: event.manifest_digest,
            retained_bytes: event.retained_bytes,
        };
        if let Some(existing) = self.hot_descriptors.get(&event.acceptance_sequence) {
            return if existing == &descriptor {
                Ok(false)
            } else {
                Err(TailOverlayError::BatchCollision(event.batch_id))
            };
        }
        self.authoritative_through = self.authoritative_through.max(event.acceptance_sequence);
        if self.hot_descriptors.len() < TAIL_MAX_BATCHES
            && self
                .retained_bytes
                .checked_add(event.retained_bytes)
                .is_some_and(|bytes| bytes <= TAIL_MAX_BYTES)
        {
            self.retained_bytes = self.retained_bytes.saturating_add(event.retained_bytes);
            self.hot_descriptors
                .insert(event.acceptance_sequence, descriptor);
        } else {
            self.descriptor_overflow = true;
            self.retained_bytes = self
                .retained_bytes
                .saturating_add(event.retained_bytes)
                .min(TAIL_MAX_BYTES.saturating_add(1));
        }
        Ok(true)
    }

    /// Drain by authoritative acceptance sequence. Provider arrival order is
    /// only a hint; every missing next event is rediscovered from durable
    /// accepted history and validated exactly once before application.
    pub fn drain_ready(
        &mut self,
        database: &mut SqliteFrontier,
        source: &RebuildSource<'_>,
        max_batches: usize,
    ) -> Result<usize, TailOverlayError> {
        let mut applied = 0;
        self.authoritative_through = self.authoritative_through.max(source.accepted_batch_count);
        while applied < max_batches {
            let expected_sequence =
                database
                    .applied_batch_count()?
                    .checked_add(1)
                    .ok_or_else(|| {
                        TailOverlayError::Projection(ProjectionError::Corrupt(
                            "applied batch sequence overflowed".into(),
                        ))
                    })? as u64;
            if expected_sequence > source.accepted_batch_count {
                break;
            }
            let event = source.accepted_event_at(expected_sequence)?;
            if let Some(descriptor) = self.hot_descriptors.get(&expected_sequence) {
                if descriptor.batch_id != event.batch_id
                    || descriptor.manifest_digest != event.manifest_digest
                {
                    return Err(TailOverlayError::BatchCollision(event.batch_id));
                }
            }
            let retained_bytes = event.retained_bytes;
            database.apply_accepted(&event)?;
            if let Some(descriptor) = self.hot_descriptors.remove(&expected_sequence) {
                self.retained_bytes = self
                    .retained_bytes
                    .saturating_sub(descriptor.retained_bytes);
            } else if self.descriptor_overflow {
                self.retained_bytes = self.retained_bytes.saturating_sub(retained_bytes);
            }
            self.applied_through = expected_sequence;
            applied += 1;
        }
        if self.applied_through >= self.authoritative_through {
            self.descriptor_overflow = false;
            self.retained_bytes = self
                .hot_descriptors
                .values()
                .fold(0_usize, |total, descriptor| {
                    total.saturating_add(descriptor.retained_bytes)
                });
        }
        Ok(applied)
    }
}

/// One leased device-local projection handle.
///
/// The projection's canonical device-root/workspace lease lives exactly as
/// long as this value, independent of the projection database's file name.
/// A clean drop or process termination releases the OS lock; a later process
/// validates the database before reuse and rebuilds from engine/store evidence
/// when deletion, stale state, corruption, or an interrupted WAL is observed.
pub struct SqliteFrontier {
    path: PathBuf,
    claim: ProjectionClaim,
    connection: Connection,
    _lease: ProcessLease,
}

#[derive(Clone, Copy)]
enum ApplyFault {
    None,
    #[cfg(test)]
    ReturnAfterInsert,
    #[cfg(test)]
    AbortAfterInsert,
    #[cfg(test)]
    AbortAfterCommit,
}

impl SqliteFrontier {
    pub fn open_or_rebuild(
        path: &Path,
        runtime_lease_root: &WorkspaceRuntimeLeaseRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
    ) -> Result<OpenProjection, ProjectionError> {
        validate_source(claim, &source)?;
        let path = prepare_database_path(path)?;
        if runtime_lease_root.workspace_id != claim.workspace_id {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: claim.workspace_id,
                found: runtime_lease_root.workspace_id,
            });
        }
        let lease = ProcessLease::acquire(runtime_lease_root, &path, claim.workspace_id)?;
        let mut pending_forensics = resume_pending_forensics(&path)?;
        let existed = projection_files_exist(&path);

        if existed {
            match validate_existing(&path, claim, &source) {
                Ok(()) => {
                    if !pending_forensics.directories.is_empty() {
                        mark_rebuild_complete(&pending_forensics)?;
                        let connection = open_writable(&path)?;
                        return Ok(OpenProjection {
                            database: Self {
                                path,
                                claim,
                                connection,
                                _lease: lease,
                            },
                            recovery: ProjectionRecovery::RebuiltPreservingEvidence {
                                reason: "recovered a committed rebuild after process termination"
                                    .into(),
                                evidence: pending_forensics.evidence,
                                applied_batches: usize::try_from(source.accepted_batch_count)
                                    .unwrap_or(usize::MAX),
                            },
                            rebuild: RebuildInstrumentation::default(),
                        });
                    }
                    let connection = open_writable(&path)?;
                    return Ok(OpenProjection {
                        database: Self {
                            path,
                            claim,
                            connection,
                            _lease: lease,
                        },
                        recovery: ProjectionRecovery::OpenedExisting,
                        rebuild: RebuildInstrumentation::default(),
                    });
                }
                Err(reason) => {
                    pending_forensics.extend(preserve_forensics(&path)?);
                    maybe_abort_forensic_test("before-rebuild", 0);
                    let (database, rebuild) =
                        Self::build_candidate_and_publish(&path, claim, lease, &source)?;
                    mark_rebuild_complete(&pending_forensics)?;
                    return Ok(OpenProjection {
                        database,
                        recovery: ProjectionRecovery::RebuiltPreservingEvidence {
                            reason,
                            evidence: pending_forensics.evidence,
                            applied_batches: rebuild.accepted_events_applied,
                        },
                        rebuild,
                    });
                }
            }
        }

        let (database, rebuild) = Self::build_candidate_and_publish(&path, claim, lease, &source)?;
        if !pending_forensics.directories.is_empty() {
            mark_rebuild_complete(&pending_forensics)?;
            return Ok(OpenProjection {
                database,
                recovery: ProjectionRecovery::RebuiltPreservingEvidence {
                    reason: "resumed interrupted forensic preservation and rebuild".into(),
                    evidence: pending_forensics.evidence,
                    applied_batches: rebuild.accepted_events_applied,
                },
                rebuild,
            });
        }
        Ok(OpenProjection {
            database,
            recovery: ProjectionRecovery::RebuiltMissing {
                applied_batches: rebuild.accepted_events_applied,
            },
            rebuild,
        })
    }

    fn build_candidate_and_publish(
        path: &Path,
        claim: ProjectionClaim,
        lease: ProcessLease,
        source: &RebuildSource<'_>,
    ) -> Result<(Self, RebuildInstrumentation), ProjectionError> {
        let candidate_path = candidate_database_path(path)?;
        remove_projection_files(&candidate_path)?;
        let mut candidate = Self::create_new(&candidate_path, claim, lease)?;
        let rebuild = match candidate.rebuild_stream(source) {
            Ok(rebuild) => rebuild,
            Err(error) => {
                drop(candidate);
                remove_projection_files(&candidate_path)?;
                return Err(error);
            }
        };
        candidate
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
        let Self {
            path: _,
            claim,
            connection,
            _lease: lease,
        } = candidate;
        connection
            .close()
            .map_err(|(_, error)| ProjectionError::Sqlite(error.to_string()))?;
        if sidecar_path(&candidate_path, "-wal").exists()
            || sidecar_path(&candidate_path, "-shm").exists()
        {
            remove_projection_files(&candidate_path)?;
            return Err(ProjectionError::Corrupt(
                "checkpointed SQLite candidate retained sidecars".into(),
            ));
        }
        fs::rename(&candidate_path, path)?;
        sync_directory(
            path.parent()
                .ok_or_else(|| ProjectionError::UnsafePath("database has no parent".into()))?,
        )?;
        let connection = open_writable(path)?;
        Ok((
            Self {
                path: path.to_path_buf(),
                claim,
                connection,
                _lease: lease,
            },
            rebuild,
        ))
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

    pub fn frontier_root(&self) -> Result<AcceptedFrontierRoot, ProjectionError> {
        read_frontier_root(&self.connection)
    }

    /// Explicit whole-frontier materialization for diagnostics and recovery.
    /// Normal apply, startup, and point authorization use `frontier_root` and
    /// `contains_frontier` instead.
    pub fn frontier(&self) -> Result<FrontierV2, ProjectionError> {
        read_frontier_documents(&self.connection)
    }

    pub fn contains_frontier(&self, required: &FrontierV2) -> Result<bool, ProjectionError> {
        canonical_frontier_bytes(required)?;
        for needed in required.documents() {
            let Some(have) = load_frontier_document(&self.connection, needed.document_id())? else {
                return Ok(false);
            };
            if !document_frontier_contains(&self.connection, &have, needed)? {
                return Ok(false);
            }
        }
        Ok(true)
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
        self.apply_internal(event, ApplyFault::None)
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
        let root = canonical_frontier_root_bytes(&read_frontier_root(&self.connection)?)?;
        bytes.extend_from_slice(&(root.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&root);
        Ok(ContentDigest::of(&bytes))
    }

    fn rebuild_stream(
        &mut self,
        source: &RebuildSource<'_>,
    ) -> Result<RebuildInstrumentation, ProjectionError> {
        let mut instrumentation = RebuildInstrumentation::default();
        for sequence in 1..=source.accepted_batch_count {
            let event = source.accepted_event_at(sequence)?;
            instrumentation.accepted_events_validated += 1;
            instrumentation.max_live_events = instrumentation.max_live_events.max(1);
            instrumentation.max_live_evidence_records =
                instrumentation.max_live_evidence_records.max(1);
            self.apply_internal(&event, ApplyFault::None)?;
            instrumentation.accepted_events_applied += 1;
            maybe_abort_rebuild_test(instrumentation.accepted_events_applied);
        }
        if read_frontier_root(&self.connection)? != source.exact_frontier_root {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's authenticated frontier root".into(),
            ));
        }
        if u64::try_from(self.applied_batch_count()?)
            .map_err(|_| ProjectionError::Corrupt("applied count exceeds u64".into()))?
            != source.accepted_batch_count
        {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's accepted event count".into(),
            ));
        }
        Ok(instrumentation)
    }

    fn apply_internal(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
    ) -> Result<ApplyDisposition, ProjectionError> {
        #[cfg(not(test))]
        let _ = fault;
        self.validate_event_claim(event)?;
        if let Some(existing) = load_batch(&self.connection, event.batch_id)? {
            if existing.matches(event)? {
                let current = read_frontier_root(&self.connection)?;
                if current.acceptance_sequence() >= event.acceptance_sequence
                    && current.state_digest() != event.prior_frontier_root.state_digest()
                {
                    return Ok(ApplyDisposition::Duplicate);
                }
                return Err(ProjectionError::FrontierRegression);
            }
            return Err(ProjectionError::BatchCollision(event.batch_id));
        }

        for dependency in &event.causal_dependency_heads {
            if !self.contains_batch(*dependency)? {
                return Err(ProjectionError::MissingDependency(*dependency));
            }
        }
        let sequence = self
            .applied_batch_count()?
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("applied batch sequence overflowed".into()))?;
        let expected_acceptance_sequence = u64::try_from(sequence)
            .map_err(|_| ProjectionError::Corrupt("applied batch sequence exceeds u64".into()))?;
        if event.acceptance_sequence != expected_acceptance_sequence {
            return Err(ProjectionError::AcceptanceOrder {
                expected: expected_acceptance_sequence,
                found: event.acceptance_sequence,
            });
        }
        let current_root = read_frontier_root(&self.connection)?;
        if current_root != event.prior_frontier_root
            || event.post_frontier_root.acceptance_sequence() != event.acceptance_sequence
        {
            return Err(ProjectionError::FrontierRegression);
        }
        let binding = super::AcceptedBatchEvidence::binding_digest_for(
            event.batch_id,
            event.manifest_digest,
            event.semantic_effect_digest,
            &event.dependency_frontier,
            &event.causal_dependency_heads,
        )
        .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
        if binding != event.event_binding_digest
            || !current_root
                .validates_transition(
                    binding,
                    event.acceptance_sequence,
                    event.post_frontier_root.document_count(),
                    &event.affected_documents,
                    &event.post_frontier_root,
                )
                .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?
        {
            return Err(ProjectionError::InvalidAcceptedEvent(
                "accepted event is not bound to its authenticated frontier transition".into(),
            ));
        }
        let mut new_documents = 0_u64;
        for document in &event.affected_documents {
            if load_frontier_document(&self.connection, document.document_id())?.is_none() {
                new_documents = new_documents.saturating_add(1);
            }
            if !document.direct_dependency_heads().contains(&event.batch_id) {
                return Err(ProjectionError::InvalidAcceptedEvent(format!(
                    "affected document {} does not name accepted batch {} as a direct head",
                    document.document_id(),
                    event.batch_id
                )));
            }
        }
        if event.post_frontier_root.document_count()
            != current_root.document_count().saturating_add(new_documents)
        {
            return Err(ProjectionError::FrontierRegression);
        }

        let dependency_frontier = canonical_frontier_bytes(&event.dependency_frontier)?;
        let prior_frontier_root = canonical_frontier_root_bytes(&event.prior_frontier_root)?;
        let post_frontier_root = canonical_frontier_root_bytes(&event.post_frontier_root)?;
        let affected_documents = canonical_affected_documents_bytes(&event.affected_documents)?;
        let causal_dependencies = encode_batch_ids(&event.causal_dependency_heads)?;
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
            &prior_frontier_root,
            &post_frontier_root,
            &affected_documents,
            &causal_dependencies,
            retained_bytes,
        )?;
        #[cfg(test)]
        if matches!(fault, ApplyFault::ReturnAfterInsert) {
            return Err(ProjectionError::InjectedFailure);
        }
        #[cfg(test)]
        if matches!(fault, ApplyFault::AbortAfterInsert) {
            std::process::abort();
        }
        for document in &event.affected_documents {
            upsert_frontier_document(&transaction, document)?;
        }
        transaction.execute(
            "UPDATE frontier
             SET frontier_root = ?1,
                 frontier_root_digest = ?2,
                 applied_batch_count = ?3
             WHERE singleton = 1",
            params![
                post_frontier_root,
                ContentDigest::of(&post_frontier_root).as_bytes().as_slice(),
                i64::try_from(sequence).map_err(|_| {
                    ProjectionError::Corrupt("applied batch sequence exceeds SQLite".into())
                })?
            ],
        )?;
        transaction.commit()?;
        #[cfg(test)]
        if matches!(fault, ApplyFault::AbortAfterCommit) {
            std::process::abort();
        }
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
    prior_frontier_root: &[u8],
    post_frontier_root: &[u8],
    affected_documents: &[u8],
    causal_dependencies: &[u8],
    retained_bytes: i64,
) -> Result<(), ProjectionError> {
    transaction.execute(
        "INSERT INTO applied_batches (
             sequence, batch_id, manifest_digest, semantic_effect,
             semantic_effect_digest, dependency_frontier,
             dependency_frontier_digest, prior_frontier_root,
             prior_frontier_root_digest, post_frontier_root,
             post_frontier_root_digest, affected_documents,
             affected_documents_digest, causal_dependency_heads,
             acceptance_sequence, retained_bytes
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
             ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
         )",
        params![
            i64::try_from(sequence)
                .map_err(|_| ProjectionError::Corrupt("batch sequence exceeds SQLite".into()))?,
            uuid_blob(&event.batch_id.as_uuid()),
            event.manifest_digest.as_bytes().as_slice(),
            &event.semantic_effect,
            event.semantic_effect_digest.as_bytes().as_slice(),
            dependency_frontier,
            ContentDigest::of(dependency_frontier).as_bytes().as_slice(),
            prior_frontier_root,
            ContentDigest::of(prior_frontier_root).as_bytes().as_slice(),
            post_frontier_root,
            ContentDigest::of(post_frontier_root).as_bytes().as_slice(),
            affected_documents,
            ContentDigest::of(affected_documents).as_bytes().as_slice(),
            causal_dependencies,
            i64::try_from(event.acceptance_sequence).map_err(|_| {
                ProjectionError::InvalidAcceptedEvent("acceptance sequence exceeds SQLite".into())
            })?,
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
    canonical_frontier_root_bytes(&source.exact_frontier_root)?;
    if source.exact_frontier_root.acceptance_sequence() != source.accepted_batch_count {
        return Err(ProjectionError::Rebuild(
            "accepted count differs from authenticated frontier version".into(),
        ));
    }
    Ok(())
}

fn validate_existing(
    path: &Path,
    claim: ProjectionClaim,
    source: &RebuildSource<'_>,
) -> Result<(), String> {
    validate_sidecar_shape(path).map_err(|error| error.to_string())?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open SQLite projection read-only: {error}"))?;
    validate_integrity(&connection).map_err(|error| error.to_string())?;
    validate_schema_and_claim(&connection, claim).map_err(|error| error.to_string())?;
    let found_frontier = read_frontier_root(&connection).map_err(|error| error.to_string())?;
    if found_frontier != source.exact_frontier_root {
        return Err("SQLite frontier is stale".into());
    }
    let count: i64 = connection
        .query_row(
            "SELECT applied_batch_count FROM frontier WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let expected_count = i64::try_from(source.accepted_batch_count)
        .map_err(|_| "accepted batch count exceeds SQLite".to_string())?;
    if count != expected_count {
        return Err("SQLite frontier batch count is stale".into());
    }
    let row_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM applied_batches", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if row_count != expected_count {
        return Err("SQLite accepted batch rows are incomplete".into());
    }
    let (validated_root, validated_count) =
        validate_stored_history(&connection).map_err(|error| error.to_string())?;
    if validated_count != source.accepted_batch_count || validated_root != found_frontier {
        return Err("SQLite accepted history authentication chain is stale".into());
    }
    let document_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM frontier_documents", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if u64::try_from(document_count).ok() != Some(found_frontier.document_count()) {
        return Err("SQLite frontier document count is stale".into());
    }
    if expected_count > 0 {
        let final_record = load_batch_at_sequence(&connection, expected_count)
            .map_err(|error| error.to_string())?;
        let final_record =
            final_record.ok_or_else(|| "SQLite final accepted row is missing".to_string())?;
        let final_root = decode_frontier_root(&final_record.post_frontier_root)
            .map_err(|error| error.to_string())?;
        if final_record.sequence != expected_count
            || final_record.acceptance_sequence != expected_count
            || final_root != found_frontier
        {
            return Err("SQLite final accepted row is not bound to the frontier root".into());
        }
    }
    Ok(())
}

fn validate_sidecar_shape(path: &Path) -> Result<(), ProjectionError> {
    let wal_path = sidecar_path(path, "-wal");
    let shm_path = sidecar_path(path, "-shm");
    let wal = match fs::read(&wal_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let shm = match fs::read(&shm_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if shm.is_some() && wal.is_none() {
        return Err(ProjectionError::Corrupt(
            "SQLite SHM exists without its WAL".into(),
        ));
    }
    if let Some(wal) = wal {
        if wal.len() < 32 {
            return Err(ProjectionError::Corrupt(
                "SQLite WAL header is truncated".into(),
            ));
        }
        let magic = u32::from_be_bytes(wal[0..4].try_into().expect("fixed WAL magic slice"));
        if !matches!(magic, 0x377f_0682 | 0x377f_0683) {
            return Err(ProjectionError::Corrupt(
                "SQLite WAL magic is invalid".into(),
            ));
        }
        let encoded_page_size =
            u32::from_be_bytes(wal[8..12].try_into().expect("fixed WAL page-size slice"));
        let page_size = if encoded_page_size == 1 {
            65_536
        } else {
            encoded_page_size as usize
        };
        if !(512..=65_536).contains(&page_size)
            || !page_size.is_power_of_two()
            || (wal.len() - 32) % (24 + page_size) != 0
        {
            return Err(ProjectionError::Corrupt(
                "SQLite WAL frame layout is invalid".into(),
            ));
        }
    }
    if let Some(shm) = shm {
        if shm.len() < 136 {
            return Err(ProjectionError::Corrupt(
                "SQLite SHM header is truncated".into(),
            ));
        }
        let version = u32::from_ne_bytes(shm[0..4].try_into().expect("fixed SHM version slice"));
        let second_version =
            u32::from_ne_bytes(shm[48..52].try_into().expect("fixed SHM version slice"));
        if version != 3_007_000 || second_version != 3_007_000 {
            return Err(ProjectionError::Corrupt(
                "SQLite SHM header version is invalid".into(),
            ));
        }
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
         {META_DDL};
         {FRONTIER_DDL};
         {FRONTIER_DOCUMENTS_DDL};
         {APPLIED_BATCHES_DDL};
         {BATCH_ID_INDEX_DDL};
         {ACCEPTANCE_SEQUENCE_INDEX_DDL};"
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
    let frontier = canonical_frontier_root_bytes(&AcceptedFrontierRoot::empty())?;
    connection.execute(
        "INSERT INTO frontier (
             singleton, frontier_root, frontier_root_digest, applied_batch_count
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
    let indexes: BTreeSet<String> = {
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'index' AND name NOT LIKE 'sqlite_%'",
        )?;
        let indexes = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        indexes
    };
    let expected_indexes: BTreeSet<String> =
        EXPECTED_INDEXES.iter().map(|name| (*name).into()).collect();
    if indexes != expected_indexes {
        return Err(ProjectionError::SchemaMismatch(format!(
            "unexpected P2.1 indexes: {indexes:?}"
        )));
    }
    validate_table_columns(connection, "meta", &META_COLUMNS)?;
    validate_table_columns(connection, "frontier", &FRONTIER_COLUMNS)?;
    validate_table_columns(connection, "frontier_documents", &FRONTIER_DOCUMENT_COLUMNS)?;
    validate_table_columns(connection, "applied_batches", &APPLIED_BATCH_COLUMNS)?;
    validate_schema_sql(connection, "table", "meta", META_DDL)?;
    validate_schema_sql(connection, "table", "frontier", FRONTIER_DDL)?;
    validate_schema_sql(
        connection,
        "table",
        "frontier_documents",
        FRONTIER_DOCUMENTS_DDL,
    )?;
    validate_schema_sql(connection, "table", "applied_batches", APPLIED_BATCHES_DDL)?;
    validate_schema_sql(
        connection,
        "index",
        "applied_batches_batch_id_uq",
        BATCH_ID_INDEX_DDL,
    )?;
    validate_schema_sql(
        connection,
        "index",
        "applied_batches_acceptance_sequence_uq",
        ACCEPTANCE_SEQUENCE_INDEX_DDL,
    )?;
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

fn validate_schema_sql(
    connection: &Connection,
    object_type: &str,
    name: &str,
    expected: &str,
) -> Result<(), ProjectionError> {
    let found: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
        params![object_type, name],
        |row| row.get(0),
    )?;
    if canonical_sql(&found) != canonical_sql(expected) {
        return Err(ProjectionError::SchemaMismatch(format!(
            "{object_type} {name} does not match canonical DDL"
        )));
    }
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
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
    prior_frontier_root: Vec<u8>,
    prior_frontier_root_digest: Vec<u8>,
    post_frontier_root: Vec<u8>,
    post_frontier_root_digest: Vec<u8>,
    affected_documents: Vec<u8>,
    affected_documents_digest: Vec<u8>,
    causal_dependency_heads: Vec<u8>,
    acceptance_sequence: i64,
    retained_bytes: i64,
}

impl StoredBatch {
    fn matches(&self, event: &AcceptedBatchEvent) -> Result<bool, ProjectionError> {
        Ok(self.matches_static(event)?
            && decode_frontier_root(&self.prior_frontier_root)? == event.prior_frontier_root
            && self.prior_frontier_root_digest
                == ContentDigest::of(&self.prior_frontier_root)
                    .as_bytes()
                    .as_slice()
            && decode_frontier_root(&self.post_frontier_root)? == event.post_frontier_root
            && self.post_frontier_root_digest
                == ContentDigest::of(&self.post_frontier_root)
                    .as_bytes()
                    .as_slice()
            && decode_affected_documents(&self.affected_documents)? == event.affected_documents
            && self.affected_documents_digest
                == ContentDigest::of(&self.affected_documents)
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
        Ok(self.batch_id == event.batch_id
            && self.manifest_digest == event.manifest_digest.as_bytes().as_slice()
            && self.semantic_effect == event.semantic_effect
            && self.semantic_effect_digest == event.semantic_effect_digest.as_bytes().as_slice()
            && dependency_frontier == event.dependency_frontier
            && self.dependency_frontier_digest
                == ContentDigest::of(&self.dependency_frontier)
                    .as_bytes()
                    .as_slice()
            && decode_batch_ids(&self.causal_dependency_heads)? == event.causal_dependency_heads
            && self.acceptance_sequence == event.acceptance_sequence as i64
            && self.retained_bytes == event.retained_bytes as i64)
    }

    fn validate_canonical_transition(
        &self,
        prior: &AcceptedFrontierRoot,
    ) -> Result<AcceptedFrontierRoot, ProjectionError> {
        if self.sequence <= 0
            || self.acceptance_sequence != self.sequence
            || self.retained_bytes < 0
        {
            return Err(ProjectionError::Corrupt(
                "stored accepted sequence or retained-byte count is invalid".into(),
            ));
        }
        let manifest_digest = decode_content_digest(&self.manifest_digest)?;
        let semantic_effect_digest = decode_semantic_effect_digest(&self.semantic_effect_digest)?;
        if SemanticEffectDigest::of(&self.semantic_effect) != semantic_effect_digest {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} semantic-effect digest mismatch",
                self.batch_id
            )));
        }
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
        if self.dependency_frontier_digest
            != ContentDigest::of(&self.dependency_frontier)
                .as_bytes()
                .as_slice()
        {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} dependency-frontier digest mismatch",
                self.batch_id
            )));
        }
        let dependency_frontier = decode_frontier(&self.dependency_frontier)?;
        let causal_dependency_heads = decode_batch_ids(&self.causal_dependency_heads)?;
        if self.prior_frontier_root_digest
            != ContentDigest::of(&self.prior_frontier_root)
                .as_bytes()
                .as_slice()
            || self.post_frontier_root_digest
                != ContentDigest::of(&self.post_frontier_root)
                    .as_bytes()
                    .as_slice()
            || self.affected_documents_digest
                != ContentDigest::of(&self.affected_documents)
                    .as_bytes()
                    .as_slice()
        {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} frontier evidence digest mismatch",
                self.batch_id
            )));
        }
        let stored_prior = decode_frontier_root(&self.prior_frontier_root)?;
        let post = decode_frontier_root(&self.post_frontier_root)?;
        let affected_documents = decode_affected_documents(&self.affected_documents)?;
        if stored_prior != *prior {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} does not continue the accepted frontier root",
                self.batch_id
            )));
        }
        let binding = super::AcceptedBatchEvidence::binding_digest_for(
            self.batch_id,
            manifest_digest,
            semantic_effect_digest,
            &dependency_frontier,
            &causal_dependency_heads,
        )
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
        if !prior
            .validates_transition(
                binding,
                self.acceptance_sequence as u64,
                post.document_count(),
                &affected_documents,
                &post,
            )
            .map_err(|error| ProjectionError::Corrupt(error.to_string()))?
        {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} frontier transition is not authenticated",
                self.batch_id
            )));
        }
        Ok(post)
    }
}

fn validate_stored_history(
    connection: &Connection,
) -> Result<(AcceptedFrontierRoot, u64), ProjectionError> {
    let mut statement = connection.prepare(
        "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier,
                dependency_frontier_digest, prior_frontier_root,
                prior_frontier_root_digest, post_frontier_root,
                post_frontier_root_digest, affected_documents,
                affected_documents_digest, causal_dependency_heads,
                acceptance_sequence, retained_bytes
         FROM applied_batches ORDER BY sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut prior = AcceptedFrontierRoot::empty();
    let mut count = 0_u64;
    while let Some(row) = rows.next()? {
        let record = stored_batch_from_row(row)?;
        count = count
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("stored history count overflowed".into()))?;
        if record.sequence != count as i64 {
            return Err(ProjectionError::Corrupt(
                "stored accepted history sequence is not contiguous".into(),
            ));
        }
        prior = record.validate_canonical_transition(&prior)?;
    }
    Ok((prior, count))
}

fn load_batch(
    connection: &Connection,
    batch_id: BatchId,
) -> Result<Option<StoredBatch>, ProjectionError> {
    connection
        .query_row(
            "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                    semantic_effect_digest, dependency_frontier,
                    dependency_frontier_digest, prior_frontier_root,
                    prior_frontier_root_digest, post_frontier_root,
                    post_frontier_root_digest, affected_documents,
                    affected_documents_digest, causal_dependency_heads,
                    acceptance_sequence, retained_bytes
             FROM applied_batches WHERE batch_id = ?1",
            [uuid_blob(&batch_id.as_uuid())],
            stored_batch_from_row,
        )
        .optional()
        .map_err(ProjectionError::from)
}

fn load_batch_at_sequence(
    connection: &Connection,
    sequence: i64,
) -> Result<Option<StoredBatch>, ProjectionError> {
    connection
        .query_row(
            "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier,
                dependency_frontier_digest, prior_frontier_root,
                prior_frontier_root_digest, post_frontier_root,
                post_frontier_root_digest, affected_documents,
                affected_documents_digest, causal_dependency_heads,
                acceptance_sequence, retained_bytes
         FROM applied_batches WHERE sequence = ?1",
            [sequence],
            stored_batch_from_row,
        )
        .optional()
        .map_err(ProjectionError::from)
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
        prior_frontier_root: row.get(7)?,
        prior_frontier_root_digest: row.get(8)?,
        post_frontier_root: row.get(9)?,
        post_frontier_root_digest: row.get(10)?,
        affected_documents: row.get(11)?,
        affected_documents_digest: row.get(12)?,
        causal_dependency_heads: row.get(13)?,
        acceptance_sequence: row.get(14)?,
        retained_bytes: row.get(15)?,
    })
}

fn read_frontier_root(connection: &Connection) -> Result<AcceptedFrontierRoot, ProjectionError> {
    let (bytes, digest): (Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT frontier_root, frontier_root_digest FROM frontier WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if digest != ContentDigest::of(&bytes).as_bytes().as_slice() {
        return Err(ProjectionError::Corrupt(
            "frontier-root digest does not match frontier-root bytes".into(),
        ));
    }
    decode_frontier_root(&bytes)
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

fn canonical_frontier_root_bytes(root: &AcceptedFrontierRoot) -> Result<Vec<u8>, ProjectionError> {
    let bytes = postcard::to_allocvec(root)
        .map_err(|error| ProjectionError::InvalidFrontier(error.to_string()))?;
    if decode_frontier_root(&bytes)? != *root {
        return Err(ProjectionError::InvalidFrontier(
            "frontier root did not survive canonical round trip".into(),
        ));
    }
    Ok(bytes)
}

fn decode_frontier_root(bytes: &[u8]) -> Result<AcceptedFrontierRoot, ProjectionError> {
    let root: AcceptedFrontierRoot = postcard::from_bytes(bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid frontier root: {error}")))?;
    let canonical = postcard::to_allocvec(&root)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    if canonical != bytes {
        return Err(ProjectionError::Corrupt(
            "stored frontier root is not canonical".into(),
        ));
    }
    if root.acceptance_sequence() == 0 && root != AcceptedFrontierRoot::empty() {
        return Err(ProjectionError::Corrupt(
            "stored empty frontier root is malformed".into(),
        ));
    }
    Ok(root)
}

fn canonical_affected_documents_bytes(
    documents: &[DocumentDependencies],
) -> Result<Vec<u8>, ProjectionError> {
    let canonical = FrontierV2::new(documents.to_vec())
        .map_err(|error| ProjectionError::InvalidFrontier(error.to_string()))?;
    if canonical.documents() != documents {
        return Err(ProjectionError::InvalidFrontier(
            "affected documents are not canonically ordered".into(),
        ));
    }
    postcard::to_allocvec(&documents)
        .map_err(|error| ProjectionError::InvalidFrontier(error.to_string()))
}

fn decode_affected_documents(bytes: &[u8]) -> Result<Vec<DocumentDependencies>, ProjectionError> {
    let documents: Vec<DocumentDependencies> = postcard::from_bytes(bytes).map_err(|error| {
        ProjectionError::Corrupt(format!("invalid affected documents: {error}"))
    })?;
    let canonical = canonical_affected_documents_bytes(&documents)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    if canonical != bytes {
        return Err(ProjectionError::Corrupt(
            "stored affected documents are not canonical".into(),
        ));
    }
    Ok(documents)
}

fn encode_frontier_document(document: &DocumentDependencies) -> Result<Vec<u8>, ProjectionError> {
    canonical_affected_documents_bytes(std::slice::from_ref(document))
}

fn decode_frontier_document(
    expected_document_id: DocumentId,
    bytes: &[u8],
) -> Result<DocumentDependencies, ProjectionError> {
    let mut documents = decode_affected_documents(bytes)?;
    if documents.len() != 1 || documents[0].document_id() != expected_document_id {
        return Err(ProjectionError::Corrupt(
            "frontier document row has mismatched identity".into(),
        ));
    }
    Ok(documents.remove(0))
}

fn upsert_frontier_document(
    transaction: &Transaction<'_>,
    document: &DocumentDependencies,
) -> Result<(), ProjectionError> {
    let bytes = encode_frontier_document(document)?;
    transaction.execute(
        "INSERT INTO frontier_documents (
             document_id, dependencies, dependencies_digest
         ) VALUES (?1, ?2, ?3)
         ON CONFLICT(document_id) DO UPDATE SET
             dependencies = excluded.dependencies,
             dependencies_digest = excluded.dependencies_digest",
        params![
            uuid_blob(&document.document_id().as_uuid()),
            &bytes,
            ContentDigest::of(&bytes).as_bytes().as_slice(),
        ],
    )?;
    Ok(())
}

fn load_frontier_document(
    connection: &Connection,
    document_id: DocumentId,
) -> Result<Option<DocumentDependencies>, ProjectionError> {
    let found: Option<(Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT dependencies, dependencies_digest
             FROM frontier_documents WHERE document_id = ?1",
            [uuid_blob(&document_id.as_uuid())],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((bytes, digest)) = found else {
        return Ok(None);
    };
    if digest != ContentDigest::of(&bytes).as_bytes().as_slice() {
        return Err(ProjectionError::Corrupt(format!(
            "frontier document {document_id} digest mismatch"
        )));
    }
    Ok(Some(decode_frontier_document(document_id, &bytes)?))
}

fn read_frontier_documents(connection: &Connection) -> Result<FrontierV2, ProjectionError> {
    let mut statement = connection.prepare(
        "SELECT document_id, dependencies, dependencies_digest
         FROM frontier_documents ORDER BY document_id",
    )?;
    let mut rows = statement.query([])?;
    let mut documents = Vec::new();
    while let Some(row) = rows.next()? {
        let id_bytes: Vec<u8> = row.get(0)?;
        let document_id = decode_document_id(&id_bytes)?;
        let bytes: Vec<u8> = row.get(1)?;
        let digest: Vec<u8> = row.get(2)?;
        if digest != ContentDigest::of(&bytes).as_bytes().as_slice() {
            return Err(ProjectionError::Corrupt(format!(
                "frontier document {document_id} digest mismatch"
            )));
        }
        documents.push(decode_frontier_document(document_id, &bytes)?);
    }
    FrontierV2::new(documents).map_err(|error| ProjectionError::Corrupt(error.to_string()))
}

fn document_frontier_contains(
    connection: &Connection,
    have: &DocumentDependencies,
    required: &DocumentDependencies,
) -> Result<bool, ProjectionError> {
    if have.document_id() != required.document_id() {
        return Ok(false);
    }
    let have_counters = have
        .peer_counters()
        .iter()
        .map(|counter| (counter.peer_id(), counter.max_counter()))
        .collect::<BTreeMap<_, _>>();
    if required.peer_counters().iter().any(|counter| {
        have_counters.get(&counter.peer_id()).copied().unwrap_or(0) < counter.max_counter()
    }) {
        return Ok(false);
    }
    for required_head in required.direct_dependency_heads() {
        let mut contained = false;
        for have_head in have.direct_dependency_heads() {
            if batch_descends_from_database(connection, *have_head, *required_head)? {
                contained = true;
                break;
            }
        }
        if !contained {
            return Ok(false);
        }
    }
    Ok(true)
}

fn batch_descends_from_database(
    connection: &Connection,
    descendant: BatchId,
    ancestor: BatchId,
) -> Result<bool, ProjectionError> {
    let mut pending = vec![descendant];
    let mut visited = BTreeSet::new();
    while let Some(batch_id) = pending.pop() {
        if batch_id == ancestor {
            return Ok(true);
        }
        if !visited.insert(batch_id) {
            continue;
        }
        let Some(record) = load_batch(connection, batch_id)? else {
            return Ok(false);
        };
        pending.extend(decode_batch_ids(&record.causal_dependency_heads)?);
    }
    Ok(false)
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

fn prepare_database_path(path: &Path) -> Result<PathBuf, ProjectionError> {
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
    let canonical_parent = fs::canonicalize(parent)?;
    let canonical_path = canonical_parent.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&canonical_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ProjectionError::UnsafePath(
                "database path is not a regular no-follow file".into(),
            ));
        }
    }
    Ok(canonical_path)
}

fn prepare_runtime_lease_root(
    path: &Path,
    workspace_id: WorkspaceId,
) -> Result<PathBuf, ProjectionError> {
    fs::create_dir_all(path)?;
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProjectionError::UnsafePath(
            "runtime lease root is not a real directory".into(),
        ));
    }
    let binding = canonical.join("workspace-id");
    let expected = format!("{}\n", workspace_id);
    match fs::symlink_metadata(&binding) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ProjectionError::UnsafePath(
                    "runtime lease workspace binding is not a regular file".into(),
                ));
            }
            let found = fs::read_to_string(&binding)?;
            if found != expected {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: found
                        .trim()
                        .parse::<Uuid>()
                        .map(WorkspaceId::from_uuid)
                        .map_err(|_| {
                            ProjectionError::Corrupt(
                                "runtime lease workspace binding is malformed".into(),
                            )
                        })?,
                });
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let mut file = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&binding)
            {
                Ok(file) => file,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return prepare_runtime_lease_root(&canonical, workspace_id)
                }
                Err(error) => return Err(error.into()),
            };
            file.write_all(expected.as_bytes())?;
            file.sync_all()?;
            sync_directory(&canonical)?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(canonical)
}

fn candidate_database_path(path: &Path) -> Result<PathBuf, ProjectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no parent".into()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
    Ok(parent.join(format!(".{name}.candidate-{}.sqlite", Uuid::new_v4())))
}

fn remove_projection_files(path: &Path) -> Result<(), ProjectionError> {
    for suffix in FORENSIC_SUFFIXES {
        let candidate = sidecar_path(path, suffix);
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn projection_files_exist(path: &Path) -> bool {
    FORENSIC_SUFFIXES
        .iter()
        .any(|suffix| sidecar_path(path, suffix).exists())
}

#[derive(Default)]
struct PendingForensics {
    directories: Vec<PathBuf>,
    evidence: Vec<ForensicEvidence>,
}

impl PendingForensics {
    fn extend(&mut self, other: Self) {
        self.directories.extend(other.directories);
        self.evidence.extend(other.evidence);
    }
}

fn preserve_forensics(path: &Path) -> Result<PendingForensics, ProjectionError> {
    let token = Uuid::new_v4().simple().to_string();
    let parent = path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no parent".into()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
    let directory = parent.join(format!("{file_name}.forensic-{token}"));
    fs::create_dir(&directory)?;
    sync_directory(parent)?;
    let mut pending = PendingForensics {
        directories: vec![directory.clone()],
        evidence: Vec::new(),
    };
    for (index, suffix) in FORENSIC_SUFFIXES.iter().enumerate() {
        let original = sidecar_path(path, suffix);
        match fs::symlink_metadata(&original) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(ProjectionError::UnsafePath(format!(
                        "projection evidence {} is not a regular file",
                        original.display()
                    )));
                }
                let preserved = directory.join(FORENSIC_NAMES[index]);
                fs::rename(&original, &preserved)?;
                sync_directory(&directory)?;
                sync_directory(parent)?;
                pending.evidence.push(ForensicEvidence {
                    original_path: original,
                    preserved_path: preserved,
                });
                maybe_abort_forensic_test("after-move", pending.evidence.len());
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    write_durable_marker(&directory, "EVIDENCE_COMPLETE")?;
    maybe_abort_forensic_test("after-evidence", pending.evidence.len());
    Ok(pending)
}

fn resume_pending_forensics(path: &Path) -> Result<PendingForensics, ProjectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no parent".into()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
    let prefix = format!("{file_name}.forensic-");
    let mut pending = PendingForensics::default();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let directory = entry.path();
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ProjectionError::UnsafePath(format!(
                "forensic evidence {} is not a regular directory",
                directory.display()
            )));
        }
        if directory.join("REBUILD_COMPLETE").exists() {
            continue;
        }
        let evidence_complete = directory.join("EVIDENCE_COMPLETE").exists();
        for (index, suffix) in FORENSIC_SUFFIXES.iter().enumerate() {
            let original = sidecar_path(path, suffix);
            let preserved = directory.join(FORENSIC_NAMES[index]);
            let original_exists = original.exists();
            let preserved_exists = preserved.exists();
            if !evidence_complete && original_exists && preserved_exists {
                return Err(ProjectionError::Corrupt(format!(
                    "forensic recovery found both {} and {}",
                    original.display(),
                    preserved.display()
                )));
            }
            if !evidence_complete && original_exists {
                let metadata = fs::symlink_metadata(&original)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(ProjectionError::UnsafePath(format!(
                        "projection evidence {} is not a regular file",
                        original.display()
                    )));
                }
                fs::rename(&original, &preserved)?;
                sync_directory(&directory)?;
                sync_directory(parent)?;
            }
            if preserved.exists() {
                pending.evidence.push(ForensicEvidence {
                    original_path: original,
                    preserved_path: preserved,
                });
            }
        }
        if !evidence_complete {
            write_durable_marker(&directory, "EVIDENCE_COMPLETE")?;
        }
        pending.directories.push(directory);
    }
    Ok(pending)
}

fn mark_rebuild_complete(pending: &PendingForensics) -> Result<(), ProjectionError> {
    for directory in &pending.directories {
        write_durable_marker(directory, "REBUILD_COMPLETE")?;
    }
    Ok(())
}

fn write_durable_marker(directory: &Path, name: &str) -> Result<(), ProjectionError> {
    let marker = directory.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)?;
    writeln!(file, "pid={}", std::process::id())?;
    file.sync_all()?;
    sync_directory(directory)?;
    let parent = directory
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("forensic directory has no parent".into()))?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), ProjectionError> {
    let directory = CapDir::open_ambient_dir(path, ambient_authority())
        .map_err(|error| ProjectionError::Io(error.to_string()))?;
    super::object_store::sync_dir_required(&directory)
        .map_err(|error| ProjectionError::Io(error.to_string()))
}

#[cfg(test)]
fn maybe_abort_forensic_test(stage: &str, moved: usize) {
    let configured = std::env::var("TINE_SQLITE_FORENSIC_ABORT").ok();
    if configured.as_deref() == Some(stage)
        || configured.as_deref() == Some(&format!("{stage}:{moved}"))
    {
        std::process::abort();
    }
}

#[cfg(not(test))]
fn maybe_abort_forensic_test(_stage: &str, _moved: usize) {}

#[cfg(test)]
fn maybe_abort_rebuild_test(applied: usize) {
    if std::env::var("TINE_SQLITE_REBUILD_ABORT_AFTER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        == Some(applied)
    {
        std::process::abort();
    }
}

#[cfg(not(test))]
fn maybe_abort_rebuild_test(_applied: usize) {}

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
    files: Vec<File>,
}

impl ProcessLease {
    fn acquire(
        runtime_root: &WorkspaceRuntimeLeaseRoot,
        database_path: &Path,
        workspace_id: WorkspaceId,
    ) -> Result<Self, ProjectionError> {
        if runtime_root.workspace_id != workspace_id {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: workspace_id,
                found: runtime_root.workspace_id,
            });
        }
        let workspace_lease_path = runtime_root.path.join("sqlite-applier.lock");
        let mut workspace_file = lock_lease_file(&workspace_lease_path)?;
        workspace_file.set_len(0)?;
        workspace_file.seek(SeekFrom::Start(0))?;
        writeln!(
            workspace_file,
            "workspace={}\npid={}\nplatform={}",
            workspace_id,
            std::process::id(),
            std::env::consts::OS
        )?;
        workspace_file.sync_all()?;
        let file_name = database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
        let database_lease_path =
            database_path.with_file_name(format!(".{file_name}.database-applier.lock"));
        let database_file = lock_lease_file(&database_lease_path)?;
        Ok(Self {
            files: vec![workspace_file, database_file],
        })
    }
}

fn lock_lease_file(lease_path: &Path) -> Result<File, ProjectionError> {
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
    let file = OpenOptions::new()
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
            return Err(ProjectionError::LeaseContended(lease_path.to_path_buf()));
        }
        return Err(error.into());
    }
    Ok(file)
}

impl Drop for ProcessLease {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = file.unlock();
        }
    }
}

fn uuid_blob(uuid: &Uuid) -> Vec<u8> {
    uuid.as_bytes().to_vec()
}

fn decode_workspace_id(bytes: &[u8]) -> Result<WorkspaceId, ProjectionError> {
    Ok(WorkspaceId::from_uuid(decode_uuid(bytes)?))
}

fn decode_document_id(bytes: &[u8]) -> Result<DocumentId, ProjectionError> {
    Ok(DocumentId::from_uuid(decode_uuid(bytes)?))
}

fn decode_content_digest(bytes: &[u8]) -> Result<ContentDigest, ProjectionError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProjectionError::Corrupt("content digest has invalid length".into()))?;
    Ok(ContentDigest::from_bytes(bytes))
}

fn decode_semantic_effect_digest(bytes: &[u8]) -> Result<SemanticEffectDigest, ProjectionError> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        ProjectionError::Corrupt("semantic-effect digest has invalid length".into())
    })?;
    Ok(SemanticEffectDigest::from_bytes(bytes))
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
    ManifestMismatch {
        batch_id: BatchId,
        expected: ContentDigest,
        found: ContentDigest,
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
    FrontierUnappliedBatch(BatchId),
    AcceptanceOrder {
        expected: u64,
        found: u64,
    },
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
            Self::ManifestMismatch {
                batch_id,
                expected,
                found,
            } => write!(
                f,
                "accepted batch {batch_id} manifest mismatch: expected {expected}, found {found}"
            ),
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
            Self::FrontierUnappliedBatch(batch_id) => {
                write!(f, "exact frontier implies unapplied batch {batch_id}")
            }
            Self::AcceptanceOrder { expected, found } => write!(
                f,
                "accepted event sequence {found} cannot apply before sequence {expected}"
            ),
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
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::oplog::{
        AuthorBatch, BatchCausalDot, BatchDisposition, BlockId, BlockLocation, CausalPeerId,
        CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies, DocumentId, ManagedPath,
        OperationBatch, OperationObject, OperationTransaction, PageId, PreparedBatch,
        SemanticOperation, SessionId,
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

    fn open_test_projection(
        path: &Path,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
    ) -> Result<OpenProjection, ProjectionError> {
        let parent = path.parent().expect("test database parent");
        let runtime = WorkspaceRuntimeLeaseRoot::open(
            &parent.join(format!(".runtime-{}", claim.workspace_id())),
            claim.workspace_id(),
        )?;
        SqliteFrontier::open_or_rebuild(path, &runtime, claim, source)
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

    fn root_transaction(ids: TestIds, path: &str, content: &str) -> OperationTransaction {
        OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                path: ManagedPath::parse(path).unwrap(),
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: ids.block,
                    home_document_id: ids.document,
                },
                page_id: ids.page,
                parent: None,
                order: "a".into(),
                content: content.into(),
            },
        ])
        .unwrap()
    }

    fn publish_and_stage(
        engine: &mut ShardedHotEngine,
        store: &ObjectStore,
        prepared: &PreparedBatch,
    ) {
        store.publish_prepared(prepared).unwrap();
        let outcome = engine
            .stage_from_store(store, prepared.manifest().batch_id())
            .unwrap();
        assert!(matches!(
            outcome.disposition(),
            BatchDisposition::Accepted { .. }
        ));
    }

    fn publish_and_stage_archive(
        engine: &mut ShardedHotEngine,
        store: &ObjectStore,
        prepared: &PreparedBatch,
    ) {
        store.publish_prepared(prepared).unwrap();
        let outcome = engine
            .stage_archive_batch(prepared.manifest().batch_id())
            .unwrap();
        assert!(matches!(
            outcome.disposition,
            BatchDisposition::Accepted { .. }
        ));
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn spawn_test_helper(
        mode: &str,
        root: &Path,
        seed: u128,
        extra_environment: &[(&str, &str)],
    ) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("oplog::sqlite::tests::sqlite_subprocess_helper")
            .arg("--nocapture")
            .env("TINE_SQLITE_HELPER_MODE", mode)
            .env("TINE_SQLITE_HELPER_ROOT", root)
            .env("TINE_SQLITE_HELPER_SEED", seed.to_string());
        for (name, value) in extra_environment {
            command.env(name, value);
        }
        command.spawn().unwrap()
    }

    fn prepare_crash_case(
        dir: &TestDir,
        seed: u128,
    ) -> (TestIds, ObjectStore, ShardedHotEngine, PathBuf) {
        let ids = TestIds::new(seed);
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let prepared = ids
            .engine()
            .prepare_transaction(
                author(seed + 100),
                &root_transaction(ids, "pages/crash.md", "crash"),
            )
            .unwrap();
        store.publish_prepared(&prepared).unwrap();
        let mut accepted_engine = ids.engine();
        assert!(matches!(
            accepted_engine
                .stage_from_store(&store, prepared.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let path = dir.path().join("frontier.sqlite");
        let empty_engine = ids.engine();
        drop(
            open_test_projection(
                &path,
                ids.claim(),
                RebuildSource::new(&empty_engine, &store).unwrap(),
            )
            .unwrap(),
        );
        (ids, store, accepted_engine, path)
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
        let opened = open_test_projection(
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
        let root_document = frontier(ids.document, 1, vec![root_id]).documents()[0].clone();
        let root_fingerprint = ContentDigest::of(&root.manifest().encode().unwrap());
        let root_binding = super::super::AcceptedBatchEvidence::binding_digest_for(
            root_id,
            root_fingerprint,
            root.manifest().semantic_effect_digest(),
            root.manifest().dependency_frontier(),
            root.manifest().causal_dependency_heads(),
        )
        .unwrap();
        let root_evidence = super::super::AcceptedBatchEvidence::for_test(
            root_id,
            root_fingerprint,
            root_binding,
            AcceptedFrontierRoot::empty(),
            vec![root_document],
            1,
        );
        let root_event = AcceptedBatchEvent::from_validated(&root, &root_evidence).unwrap();
        let child = fake_validated(
            store,
            ids,
            child_id,
            vec![root_id],
            frontier(ids.document, 1, vec![root_id]),
        );
        let child_document = frontier(ids.document, 2, vec![child_id]).documents()[0].clone();
        let child_fingerprint = ContentDigest::of(&child.manifest().encode().unwrap());
        let child_binding = super::super::AcceptedBatchEvidence::binding_digest_for(
            child_id,
            child_fingerprint,
            child.manifest().semantic_effect_digest(),
            child.manifest().dependency_frontier(),
            child.manifest().causal_dependency_heads(),
        )
        .unwrap();
        let child_evidence = super::super::AcceptedBatchEvidence::for_test(
            child_id,
            child_fingerprint,
            child_binding,
            root_event.post_frontier_root.clone(),
            vec![child_document],
            1,
        );
        let child_event = AcceptedBatchEvent::from_validated(&child, &child_evidence).unwrap();
        (root_event, child_event)
    }

    #[test]
    fn sqlite_subprocess_helper() {
        let Ok(mode) = std::env::var("TINE_SQLITE_HELPER_MODE") else {
            return;
        };
        let root = PathBuf::from(std::env::var_os("TINE_SQLITE_HELPER_ROOT").unwrap());
        let seed = std::env::var("TINE_SQLITE_HELPER_SEED")
            .unwrap()
            .parse::<u128>()
            .unwrap();
        let ids = TestIds::new(seed);
        let store = ObjectStore::open(&root.join("objects"), ids.workspace).unwrap();
        let ready = root.join("helper-ready");
        if mode == "lease" {
            let engine = ids.engine();
            let _opened = open_test_projection(
                &root.join("lease-a.sqlite"),
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            )
            .unwrap();
            fs::write(&ready, b"ready").unwrap();
            loop {
                thread::park_timeout(Duration::from_secs(60));
            }
        }

        if mode == "canonical-lease-contender" {
            let runtime =
                WorkspaceRuntimeLeaseRoot::open(&root.join("runtime"), ids.workspace).unwrap();
            let engine = ids.engine();
            let result = SqliteFrontier::open_or_rebuild(
                &root.join("db-b/frontier.sqlite"),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            );
            assert!(matches!(result, Err(ProjectionError::LeaseContended(_))));
            return;
        }

        if mode == "recover" {
            let mut accepted_engine = ids.engine();
            for manifest in store.committed_manifests().unwrap() {
                assert!(matches!(
                    accepted_engine
                        .stage_from_store(&store, manifest.batch_id())
                        .unwrap()
                        .disposition(),
                    BatchDisposition::Accepted { .. }
                ));
            }
            fs::write(&ready, b"ready").unwrap();
            let _ = open_test_projection(
                &root.join("frontier.sqlite"),
                ids.claim(),
                RebuildSource::new(&accepted_engine, &store).unwrap(),
            )
            .unwrap();
            return;
        }

        let batch_id = batch(seed + 100);
        let mut accepted_engine = ids.engine();
        assert!(matches!(
            accepted_engine
                .stage_from_store(&store, batch_id)
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let empty_engine = ids.engine();
        let mut database = open_test_projection(
            &root.join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&empty_engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        database
            .connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        let event = AcceptedBatchEvent::from_accepted(&accepted_engine, &store, batch_id).unwrap();
        fs::write(&ready, b"ready").unwrap();
        match mode.as_str() {
            "apply-before" => std::process::abort(),
            "apply-during" => {
                let _ = database.apply_internal(&event, ApplyFault::AbortAfterInsert);
            }
            "apply-after" => {
                let _ = database.apply_internal(&event, ApplyFault::AbortAfterCommit);
            }
            other => panic!("unknown SQLite subprocess helper mode {other}"),
        }
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
            database.apply_internal(&root, ApplyFault::ReturnAfterInsert),
            Err(ProjectionError::InjectedFailure)
        );
        assert_eq!(database.applied_batch_count().unwrap(), 0);
        assert_eq!(database.frontier().unwrap(), FrontierV2::default());
        let database_path = database.path().to_path_buf();
        drop(database);
        let reopened = open_test_projection(
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
        assert_eq!(database.frontier().unwrap(), root.exact_frontier());
    }

    #[test]
    fn canonical_schema_rejects_type_pk_check_strict_index_and_version_mutations() {
        for (case, seed) in [
            ("type", 1_100),
            ("primary-key", 1_200),
            ("check", 1_300),
            ("strict", 1_400),
            ("unique-index", 1_500),
            ("user-version", 1_600),
        ] {
            let ids = TestIds::new(seed);
            let dir = TestDir::new(&format!("schema-{case}"));
            let (database, engine, store) = open_empty(&dir, ids);
            let path = database.path().to_path_buf();
            drop(database);
            let connection = Connection::open(&path).unwrap();
            match case {
                "type" | "primary-key" | "check" | "strict" => {
                    connection
                        .execute_batch("DROP TABLE applied_batches")
                        .unwrap();
                    let altered = match case {
                        "type" => APPLIED_BATCHES_DDL.replacen("batch_id BLOB", "batch_id TEXT", 1),
                        "primary-key" => APPLIED_BATCHES_DDL.replacen(
                            "sequence INTEGER PRIMARY KEY",
                            "sequence INTEGER NOT NULL",
                            1,
                        ),
                        "check" => APPLIED_BATCHES_DDL.replacen(
                            "retained_bytes INTEGER NOT NULL CHECK (retained_bytes >= 0)",
                            "retained_bytes INTEGER NOT NULL",
                            1,
                        ),
                        "strict" => APPLIED_BATCHES_DDL.replacen(") STRICT", ")", 1),
                        _ => unreachable!(),
                    };
                    connection.execute_batch(&altered).unwrap();
                    connection.execute_batch(BATCH_ID_INDEX_DDL).unwrap();
                    connection
                        .execute_batch(ACCEPTANCE_SEQUENCE_INDEX_DDL)
                        .unwrap();
                }
                "unique-index" => {
                    connection
                        .execute_batch(
                            "DROP INDEX applied_batches_batch_id_uq;
                             CREATE INDEX applied_batches_batch_id_uq
                             ON applied_batches(batch_id)",
                        )
                        .unwrap();
                }
                "user-version" => {
                    connection
                        .pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION + 1)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(connection);
            let rebuilt = open_test_projection(
                &path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            )
            .unwrap();
            assert!(
                matches!(
                    rebuilt.recovery,
                    ProjectionRecovery::RebuiltPreservingEvidence { .. }
                ),
                "schema mutation {case} was not rebuilt: {:?}",
                rebuilt.recovery
            );
            validate_schema_and_claim(&rebuilt.database.connection, ids.claim()).unwrap();
        }
    }

    #[test]
    fn exact_frontier_point_queries_are_monotonic_and_ancestry_aware() {
        let ids = TestIds::new(2_000);
        let dir = TestDir::new("frontier-point-containment");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (root, child) = root_and_child_events(&store, ids);
        let required = root.exact_frontier();
        assert!(!database.contains_frontier(&required).unwrap());
        database.apply_accepted(&root).unwrap();
        assert!(database.contains_frontier(&required).unwrap());
        database.apply_accepted(&child).unwrap();
        assert!(database.contains_frontier(&required).unwrap());

        let unrelated = frontier(ids.document, 2, vec![batch(202)]);
        assert!(!database.contains_frontier(&unrelated).unwrap());
        let missing_peer = FrontierV2::new(vec![DocumentDependencies::new(
            ids.document,
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(999), 1)],
            vec![child.batch_id()],
        )
        .unwrap()])
        .unwrap();
        assert!(!database.contains_frontier(&missing_peer).unwrap());
    }

    #[test]
    fn accepted_events_keep_compact_historical_roots_and_structural_applied_closure() {
        let ids = TestIds::new(2_100);
        let dir = TestDir::new("historical-frontier");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine = ids.engine();
        let root = engine
            .prepare_transaction(
                author(2_200),
                &root_transaction(ids, "pages/root.md", "root"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &root);
        let early_root =
            AcceptedBatchEvent::from_accepted(&engine, &store, root.manifest().batch_id()).unwrap();

        let child = engine
            .prepare_transaction(
                author(2_201),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "child".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &child);
        let child_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, child.manifest().batch_id())
                .unwrap();
        let late_root =
            AcceptedBatchEvent::from_accepted(&engine, &store, root.manifest().batch_id()).unwrap();
        assert_eq!(late_root, early_root);
        assert_ne!(late_root.exact_frontier(), child_event.exact_frontier());

        let empty = ids.engine();
        let mut database = open_test_projection(
            &dir.path().join("live.sqlite"),
            ids.claim(),
            RebuildSource::new(&empty, &store).unwrap(),
        )
        .unwrap()
        .database;
        assert_eq!(
            database.apply_accepted(&late_root).unwrap(),
            ApplyDisposition::Applied
        );
        assert!(!database
            .contains_batch(child.manifest().batch_id())
            .unwrap());
        assert_eq!(database.frontier().unwrap(), late_root.exact_frontier());
        assert_eq!(
            database.apply_accepted(&late_root).unwrap(),
            ApplyDisposition::Duplicate
        );
        assert_eq!(
            database.apply_accepted(&child_event).unwrap(),
            ApplyDisposition::Applied
        );
        drop(database);

        let rebuild_path = dir.path().join("rebuild.sqlite");
        let rebuilt = open_test_projection(
            &rebuild_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(rebuilt.database.applied_batch_count().unwrap(), 2);
        drop(rebuilt);
        let connection = Connection::open(&rebuild_path).unwrap();
        let row_frontiers: Vec<(Vec<u8>, Vec<u8>)> = connection
            .prepare(
                "SELECT post_frontier_root, affected_documents
                 FROM applied_batches ORDER BY sequence",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(row_frontiers.len(), 2);
        assert_ne!(row_frontiers[0].0, row_frontiers[1].0);
        assert_eq!(
            decode_frontier_root(&row_frontiers[0].0).unwrap(),
            late_root.post_frontier_root
        );
        assert_eq!(
            decode_frontier_root(&row_frontiers[1].0).unwrap(),
            child_event.post_frontier_root
        );
        assert_eq!(
            decode_affected_documents(&row_frontiers[0].1).unwrap(),
            late_root.affected_documents
        );
        assert_eq!(
            decode_affected_documents(&row_frontiers[1].1).unwrap(),
            child_event.affected_documents
        );
    }

    #[test]
    fn store_backed_one_document_acceptance_keeps_compact_authenticated_evidence() {
        const PAGE_COUNT: usize = 128;
        let ids = TestIds::new(2_300);
        let dir = TestDir::new("compact-frontier-evidence");
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut operations = Vec::with_capacity(PAGE_COUNT * 2);
        let mut target = None;
        let mut untouched_document = None;
        for index in 0..PAGE_COUNT as u128 {
            let page_id = PageId::from_uuid(uuid(20_000 + index * 3));
            let document_id = DocumentId::from_uuid(uuid(20_001 + index * 3));
            let block_id = BlockId::from_uuid(uuid(20_002 + index * 3));
            operations.push(SemanticOperation::CreatePage {
                page_id,
                home_document_id: document_id,
                path: ManagedPath::parse(&format!("pages/wide-{index}.md")).unwrap(),
            });
            operations.push(SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: document_id,
                },
                page_id,
                parent: None,
                order: "a".into(),
                content: format!("wide {index}"),
            });
            if index == 0 {
                target = Some((block_id, document_id));
            } else if index == 1 {
                untouched_document = Some(document_id);
            }
        }
        let wide = engine
            .prepare_transaction(
                author(2_301),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &wide);
        let wide_evidence = engine
            .accepted_batch_evidence(wide.manifest().batch_id())
            .unwrap();
        assert_eq!(
            engine.exact_frontier().unwrap().documents().len(),
            PAGE_COUNT + 1
        );
        let (block_id, document_id) = target.unwrap();
        let edit = engine
            .prepare_transaction(
                author(2_302),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id,
                        home_document_id: document_id,
                    },
                    content: "bounded edit".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &edit);
        let evidence = engine
            .accepted_batch_evidence(edit.manifest().batch_id())
            .unwrap();
        assert!(evidence.post_frontier_root().has_persistent_point_index());
        assert_eq!(evidence.affected_documents().len(), 1);
        let evidence_bytes = postcard::to_allocvec(&evidence).unwrap();
        assert!(
            evidence_bytes.len() < 32 * 1024,
            "one-document evidence retained {} bytes for {PAGE_COUNT} pages",
            evidence_bytes.len()
        );
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, edit.manifest().batch_id()).unwrap();
        assert_eq!(event.affected_documents().len(), 1);
        assert!(
            canonical_frontier_root_bytes(event.post_frontier_root())
                .unwrap()
                .len()
                < 16 * 1024
        );
        let untouched_document = untouched_document.unwrap();
        assert_eq!(
            engine
                .accepted_frontier_document(wide_evidence.post_frontier_root(), untouched_document,)
                .unwrap(),
            engine
                .accepted_frontier_document(evidence.post_frontier_root(), untouched_document)
                .unwrap()
        );
        assert_ne!(
            engine
                .accepted_frontier_document(wide_evidence.post_frontier_root(), document_id,)
                .unwrap(),
            engine
                .accepted_frontier_document(evidence.post_frontier_root(), document_id)
                .unwrap()
        );
    }

    fn measured_streaming_rebuild(
        batch_count: usize,
        seed: u128,
    ) -> (RebuildInstrumentation, Duration, Duration) {
        let ids = TestIds::new(seed);
        let dir = TestDir::new(&format!("streaming-rebuild-{batch_count}"));
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let root = engine
            .prepare_transaction(
                author(seed + 1),
                &root_transaction(ids, "pages/linear.md", "0"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &root);
        for index in 1..batch_count {
            let edit = engine
                .prepare_transaction(
                    author(seed + 1 + index as u128),
                    &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                        block: BlockLocation {
                            block_id: ids.block,
                            home_document_id: ids.document,
                        },
                        content: index.to_string(),
                    }])
                    .unwrap(),
                )
                .unwrap();
            publish_and_stage_archive(&mut engine, &store, &edit);
        }
        let started = Instant::now();
        let path = dir.path().join("frontier.sqlite");
        let opened = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let rebuild_elapsed = started.elapsed();
        assert_eq!(opened.database.applied_batch_count().unwrap(), batch_count);
        let rebuild = opened.rebuild;
        drop(opened);
        let started = Instant::now();
        let reopened = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let startup_elapsed = started.elapsed();
        assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
        (rebuild, rebuild_elapsed, startup_elapsed)
    }

    #[test]
    fn rebuild_streams_linearly_with_one_live_event_and_evidence_record() {
        let (small, small_elapsed, small_startup) = measured_streaming_rebuild(24, 2_500);
        let (large, large_elapsed, large_startup) = measured_streaming_rebuild(48, 2_700);
        assert_eq!(small.accepted_events_validated, 24);
        assert_eq!(small.accepted_events_applied, 24);
        assert_eq!(large.accepted_events_validated, 48);
        assert_eq!(large.accepted_events_applied, 48);
        assert_eq!(small.max_live_events, 1);
        assert_eq!(large.max_live_events, 1);
        assert_eq!(small.max_live_evidence_records, 1);
        assert_eq!(large.max_live_evidence_records, 1);
        assert_eq!(small.ancestry_full_scans, 0);
        assert_eq!(large.ancestry_full_scans, 0);
        assert!(small_startup < Duration::from_secs(2));
        assert!(large_startup < Duration::from_secs(2));
        eprintln!(
            "sqlite_streaming_rebuild batches=24 rebuild_ms={} startup_ms={} validated={} max_live_events={} max_live_evidence={}; batches=48 rebuild_ms={} startup_ms={} validated={} max_live_events={} max_live_evidence={}",
            small_elapsed.as_millis(),
            small_startup.as_millis(),
            small.accepted_events_validated,
            small.max_live_events,
            small.max_live_evidence_records,
            large_elapsed.as_millis(),
            large_startup.as_millis(),
            large.accepted_events_validated,
            large.max_live_events,
            large.max_live_evidence_records,
        );
    }

    #[test]
    #[ignore = "explicit authenticated SQLite cold-rebuild performance gate"]
    fn sqlite_streaming_rebuild_cold_gate() {
        let (work, rebuild_elapsed, startup_elapsed) = measured_streaming_rebuild(1_000, 2_900);
        assert_eq!(work.accepted_events_validated, 1_000);
        assert_eq!(work.accepted_events_applied, 1_000);
        assert_eq!(work.max_live_events, 1);
        assert_eq!(work.max_live_evidence_records, 1);
        assert_eq!(work.ancestry_full_scans, 0);
        assert!(
            rebuild_elapsed <= Duration::from_secs(45),
            "authenticated SQLite rebuild took {rebuild_elapsed:?}"
        );
        assert!(
            startup_elapsed <= Duration::from_secs(2),
            "normal SQLite startup took {startup_elapsed:?}"
        );
        eprintln!(
            "sqlite_streaming_rebuild_gate batches=1000 rebuild_ms={} startup_ms={} validated={} max_live_events={} max_live_evidence={}",
            rebuild_elapsed.as_millis(),
            startup_elapsed.as_millis(),
            work.accepted_events_validated,
            work.max_live_events,
            work.max_live_evidence_records,
        );
    }

    #[test]
    fn concurrent_events_wait_for_their_authenticated_acceptance_prefix() {
        let base = TestIds::new(2_300);
        let right = TestIds {
            workspace: base.workspace,
            lineage: base.lineage,
            catalog: base.catalog,
            document: DocumentId::from_uuid(uuid(2_403)),
            page: PageId::from_uuid(uuid(2_404)),
            block: BlockId::from_uuid(uuid(2_405)),
        };
        let dir = TestDir::new("concurrent-order");
        let store = ObjectStore::open(&dir.path().join("objects"), base.workspace).unwrap();
        let left_batch = base
            .engine()
            .prepare_transaction(
                author(2_500),
                &root_transaction(base, "pages/left.md", "left"),
            )
            .unwrap();
        let right_batch = right
            .engine()
            .prepare_transaction(
                author(2_501),
                &root_transaction(right, "pages/right.md", "right"),
            )
            .unwrap();
        store.publish_prepared(&left_batch).unwrap();
        store.publish_prepared(&right_batch).unwrap();
        let mut receiver = base.engine();
        assert!(matches!(
            receiver
                .stage_from_store(&store, left_batch.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let left =
            AcceptedBatchEvent::from_accepted(&receiver, &store, left_batch.manifest().batch_id())
                .unwrap();
        assert!(matches!(
            receiver
                .stage_from_store(&store, right_batch.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let right =
            AcceptedBatchEvent::from_accepted(&receiver, &store, right_batch.manifest().batch_id())
                .unwrap();
        let (mut database, _, _) = open_empty(&dir, base);
        assert_eq!(
            database.apply_accepted(&right),
            Err(ProjectionError::AcceptanceOrder {
                expected: 1,
                found: 2
            })
        );
        assert_eq!(database.applied_batch_count().unwrap(), 0);
        assert_eq!(
            database.apply_accepted(&left).unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(
            database.apply_accepted(&right).unwrap(),
            ApplyDisposition::Applied
        );
    }

    #[test]
    fn accepted_manifest_fingerprint_rejects_same_id_cross_store_collision() {
        let ids = TestIds::new(2_600);
        let dir = TestDir::new("manifest-collision");
        let good_store = ObjectStore::open(&dir.path().join("good"), ids.workspace).unwrap();
        let evil_store = ObjectStore::open(&dir.path().join("evil"), ids.workspace).unwrap();
        let shared_author = author(2_700);
        let good = ids
            .engine()
            .prepare_transaction(
                shared_author,
                &root_transaction(ids, "pages/same.md", "GOOD"),
            )
            .unwrap();
        let evil = ids
            .engine()
            .prepare_transaction(
                shared_author,
                &root_transaction(ids, "pages/same.md", "EVIL"),
            )
            .unwrap();
        assert_eq!(good.manifest().batch_id(), evil.manifest().batch_id());
        assert_ne!(
            good.manifest().encode().unwrap(),
            evil.manifest().encode().unwrap()
        );
        good_store.publish_prepared(&good).unwrap();
        evil_store.publish_prepared(&evil).unwrap();
        let mut receiver = ids.engine();
        assert!(matches!(
            receiver
                .stage_from_store(&good_store, good.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            AcceptedBatchEvent::from_accepted(&receiver, &evil_store, good.manifest().batch_id()),
            Err(ProjectionError::ManifestMismatch { .. })
        ));
        assert!(matches!(
            open_test_projection(
                &dir.path().join("evil-rebuild.sqlite"),
                ids.claim(),
                RebuildSource::new(&receiver, &evil_store).unwrap(),
            ),
            Err(ProjectionError::ManifestMismatch { .. })
        ));
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
        assert!(database.contains_frontier(&root.exact_frontier()).unwrap());
        assert!(!database.contains_frontier(&child.exact_frontier()).unwrap());
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
        assert!(database.contains_frontier(&root.exact_frontier()).unwrap());
        assert!(database.contains_frontier(&child.exact_frontier()).unwrap());
        let sibling = fake_validated(
            &store,
            ids,
            batch(102),
            vec![root.batch_id()],
            root.exact_frontier(),
        );
        let sibling_document = frontier(ids.document, 3, vec![batch(102)]).documents()[0].clone();
        let sibling_evidence = super::super::AcceptedBatchEvidence::for_test(
            batch(102),
            ContentDigest::of(&sibling.manifest().encode().unwrap()),
            super::super::AcceptedBatchEvidence::binding_digest_for(
                batch(102),
                ContentDigest::of(&sibling.manifest().encode().unwrap()),
                sibling.manifest().semantic_effect_digest(),
                sibling.manifest().dependency_frontier(),
                sibling.manifest().causal_dependency_heads(),
            )
            .unwrap(),
            child.post_frontier_root.clone(),
            vec![sibling_document],
            1,
        );
        let mut regressing =
            AcceptedBatchEvent::from_validated(&sibling, &sibling_evidence).unwrap();
        regressing.prior_frontier_root = root.post_frontier_root.clone();
        assert_eq!(
            database.apply_accepted(&regressing),
            Err(ProjectionError::FrontierRegression)
        );
    }

    #[test]
    fn overlay_reorders_dependencies_and_enforces_both_limits() {
        let ids = TestIds::new(4_000);
        let dir = TestDir::new("overlay");
        let (mut database, _empty, store) = open_empty(&dir, ids);
        let mut engine = ids.engine();
        let root_prepared = engine
            .prepare_transaction(
                author(4_010),
                &root_transaction(ids, "pages/overlay.md", "root"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &root_prepared);
        let root =
            AcceptedBatchEvent::from_accepted(&engine, &store, root_prepared.manifest().batch_id())
                .unwrap();
        let child_prepared = engine
            .prepare_transaction(
                author(4_011),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "child".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &child_prepared);
        let child = AcceptedBatchEvent::from_accepted(
            &engine,
            &store,
            child_prepared.manifest().batch_id(),
        )
        .unwrap();
        let source = RebuildSource::new(&engine, &store).unwrap();
        let mut overlay = TailOverlay::default();
        assert!(overlay.try_enqueue(&mut database, &child).unwrap());
        assert!(overlay.try_enqueue(&mut database, &root).unwrap());
        assert_eq!(
            overlay
                .drain_ready(&mut database, &source, usize::MAX)
                .unwrap(),
            2
        );
        assert_eq!(
            database.frontier().unwrap(),
            engine.exact_frontier().unwrap()
        );
        assert_eq!(overlay.status().unapplied_batches, 0);
        assert!(!overlay.try_enqueue(&mut database, &root).unwrap());
        assert_eq!(overlay.status().unapplied_batches, 0);

        let mut count_limited = TailOverlay::default();
        let mut reservations = Vec::with_capacity(TAIL_MAX_BATCHES);
        for _ in 0..TAIL_MAX_BATCHES {
            reservations.push(count_limited.reserve_mutation(1).unwrap());
        }
        assert!(count_limited.status().backpressured);
        assert!(matches!(
            count_limited.reserve_mutation(1),
            Err(TailOverlayError::Backpressure(TailOverlayStatus {
                backpressured: true,
                ..
            }))
        ));
        for reservation in reservations {
            count_limited.cancel_reservation(reservation).unwrap();
        }

        let mut byte_limited = TailOverlay::default();
        let reservation = byte_limited.reserve_mutation(TAIL_MAX_BYTES).unwrap();
        assert!(byte_limited.status().backpressured);
        assert!(matches!(
            byte_limited.reserve_mutation(1),
            Err(TailOverlayError::Backpressure(TailOverlayStatus {
                backpressured: true,
                ..
            }))
        ));
        byte_limited.cancel_reservation(reservation).unwrap();
    }

    #[test]
    fn provider_tail_over_cap_retains_only_bounded_hot_descriptors() {
        let ids = TestIds::new(4_050);
        let dir = TestDir::new("provider-tail-cap");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (root, _) = root_and_child_events(&store, ids);
        let mut overlay = TailOverlay::default();
        let tail = TAIL_MAX_BATCHES + 257;
        for index in (0..tail).rev() {
            let mut event = root.clone();
            event.batch_id = batch(80_000 + index as u128);
            event.manifest_digest = ContentDigest::of(&index.to_be_bytes());
            event.acceptance_sequence = index as u64 + 1;
            event.retained_bytes = 1;
            assert!(overlay.try_enqueue(&mut database, &event).unwrap());
        }
        assert_eq!(overlay.status().unapplied_batches, tail);
        assert!(overlay.status().backpressured);
        assert_eq!(overlay.hot_descriptor_count(), TAIL_MAX_BATCHES);
        assert!(overlay.hot_descriptor_count() <= TAIL_MAX_BATCHES);

        let mut duplicate = root;
        duplicate.batch_id = batch(80_000 + (tail - 1) as u128);
        duplicate.manifest_digest = ContentDigest::of(&(tail - 1).to_be_bytes());
        duplicate.acceptance_sequence = tail as u64;
        duplicate.retained_bytes = 1;
        assert!(!overlay.try_enqueue(&mut database, &duplicate).unwrap());
        assert_eq!(overlay.hot_descriptor_count(), TAIL_MAX_BATCHES);
    }

    #[test]
    fn oversized_authoritative_event_is_retained_backpressured_and_drainable() {
        let ids = TestIds::new(4_100);
        let dir = TestDir::new("oversized-overlay");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut overlay = TailOverlay::default();
        assert!(matches!(
            overlay.reserve_mutation(TAIL_MAX_BYTES + 1),
            Err(TailOverlayError::Backpressure(_))
        ));
        assert_eq!(overlay.status().unapplied_batches, 0);

        let content = "x".repeat(4 * 1024 * 1024);
        let mut operations = vec![SemanticOperation::CreatePage {
            page_id: ids.page,
            home_document_id: ids.document,
            path: ManagedPath::parse("pages/oversized.md").unwrap(),
        }];
        for index in 0..5_u128 {
            operations.push(SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(4_200 + index)),
                    home_document_id: ids.document,
                },
                page_id: ids.page,
                parent: None,
                order: index.to_string(),
                content: content.clone(),
            });
        }
        let mut engine = ids.engine();
        let prepared = engine
            .prepare_transaction(
                author(4_300),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        assert!(event.retained_bytes() > TAIL_MAX_BYTES);
        let empty = ids.engine();
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&empty, &store).unwrap(),
        )
        .unwrap()
        .database;
        assert!(overlay.try_enqueue(&mut database, &event).unwrap());
        assert_eq!(overlay.status().unapplied_batches, 1);
        assert!(overlay.status().backpressured);
        assert!(overlay.status().retained_bytes > TAIL_MAX_BYTES);

        let source = RebuildSource::new(&engine, &store).unwrap();
        assert_eq!(overlay.drain_ready(&mut database, &source, 1).unwrap(), 1);
        assert_eq!(overlay.status().unapplied_batches, 0);
        assert!(database.contains_batch(event.batch_id()).unwrap());
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
        let first = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            open_test_projection(
                &database_path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        assert!(matches!(
            open_test_projection(
                &dir.path().join("alternate.sqlite"),
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        fs::create_dir(dir.path().join("alias")).unwrap();
        assert!(matches!(
            open_test_projection(
                &dir.path().join("alias").join("..").join("aliased.sqlite"),
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
            open_test_projection(
                &database_path,
                foreign_ids.claim(),
                RebuildSource::new(&foreign_engine, &foreign_store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(first);
        let recovered = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered.recovery, ProjectionRecovery::OpenedExisting);
    }

    #[test]
    fn separate_process_workspace_lease_contends_and_crash_releases() {
        let seed = 7_200;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("lease-subprocess");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut child = spawn_test_helper("lease", dir.path(), seed, &[]);
        wait_for_file(&dir.path().join("helper-ready"));
        let engine = ids.engine();
        assert!(matches!(
            open_test_projection(
                &dir.path().join("lease-b.sqlite"),
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let recovered = open_test_projection(
            &dir.path().join("lease-b.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            recovered.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        ));
    }

    #[test]
    fn canonical_runtime_lease_contends_across_database_parents_and_subprocess() {
        let seed = 7_400;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("canonical-runtime-lease");
        fs::create_dir_all(dir.path().join("db-a")).unwrap();
        fs::create_dir_all(dir.path().join("db-b")).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine = ids.engine();
        let runtime =
            WorkspaceRuntimeLeaseRoot::open(&dir.path().join("runtime"), ids.workspace).unwrap();
        let first = SqliteFrontier::open_or_rebuild(
            &dir.path().join("db-a/frontier.sqlite"),
            &runtime,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild(
                &dir.path().join("db-b/frontier.sqlite"),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        let mut child = spawn_test_helper("canonical-lease-contender", dir.path(), seed, &[]);
        assert!(child.wait().unwrap().success());

        let foreign = TestIds::new(seed + 100);
        assert!(matches!(
            WorkspaceRuntimeLeaseRoot::open(&dir.path().join("runtime"), foreign.workspace),
            Err(ProjectionError::WorkspaceMismatch { .. })
        ));

        drop(first);
        let recovered = SqliteFrontier::open_or_rebuild(
            &dir.path().join("db-b/frontier.sqlite"),
            &runtime,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            recovered.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        ));
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
        assert_eq!(accepted_event.exact_frontier(), exact_frontier);
        let expected_snapshot = engine.canonical_snapshot().unwrap();
        let database_path = dir.path().join("frontier.sqlite");

        let first = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(first.database.applied_batch_count().unwrap(), 1);
        let first_digest = first.database.semantic_projection_digest().unwrap();
        drop(first);
        remove_projection_files(&database_path);

        let rebuilt = open_test_projection(
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
            let rebuilt = open_test_projection(
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
    fn subprocess_death_before_during_and_after_commit_recovers_exactly() {
        for (index, mode) in ["apply-before", "apply-during", "apply-after"]
            .into_iter()
            .enumerate()
        {
            let seed = 9_200 + index as u128 * 100;
            let dir = TestDir::new(mode);
            let (ids, store, accepted_engine, path) = prepare_crash_case(&dir, seed);
            let mut child = spawn_test_helper(mode, dir.path(), seed, &[]);
            wait_for_file(&dir.path().join("helper-ready"));
            assert!(!child.wait().unwrap().success());
            if mode == "apply-after" {
                assert!(fs::metadata(sidecar_path(&path, "-wal")).unwrap().len() >= 32);
            }
            let reopened = open_test_projection(
                &path,
                ids.claim(),
                RebuildSource::new(&accepted_engine, &store).unwrap(),
            )
            .unwrap();
            if mode == "apply-after" {
                assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
            } else {
                assert!(matches!(
                    reopened.recovery,
                    ProjectionRecovery::RebuiltPreservingEvidence { .. }
                ));
            }
            assert_eq!(reopened.database.applied_batch_count().unwrap(), 1);
            assert_eq!(
                reopened.database.frontier().unwrap(),
                accepted_engine.exact_frontier().unwrap()
            );
        }
    }

    #[test]
    fn corrupt_or_truncated_wal_and_shm_are_preserved_before_rebuild() {
        for (index, mutation) in ["wal-truncate", "wal-corrupt", "shm-truncate", "shm-corrupt"]
            .into_iter()
            .enumerate()
        {
            let seed = 9_600 + index as u128 * 100;
            let dir = TestDir::new(mutation);
            let (ids, store, accepted_engine, path) = prepare_crash_case(&dir, seed);
            let mut child = spawn_test_helper("apply-after", dir.path(), seed, &[]);
            wait_for_file(&dir.path().join("helper-ready"));
            assert!(!child.wait().unwrap().success());
            let target = if mutation.starts_with("wal") {
                sidecar_path(&path, "-wal")
            } else {
                sidecar_path(&path, "-shm")
            };
            assert!(
                target.exists(),
                "missing crash sidecar {}",
                target.display()
            );
            if mutation.ends_with("truncate") {
                OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .unwrap()
                    .set_len(8)
                    .unwrap();
            } else {
                let mut file = OpenOptions::new().write(true).open(&target).unwrap();
                file.seek(SeekFrom::Start(0)).unwrap();
                file.write_all(&[0_u8; 8]).unwrap();
                file.sync_all().unwrap();
            }
            let reopened = open_test_projection(
                &path,
                ids.claim(),
                RebuildSource::new(&accepted_engine, &store).unwrap(),
            )
            .unwrap();
            let ProjectionRecovery::RebuiltPreservingEvidence { evidence, .. } = &reopened.recovery
            else {
                panic!("sidecar mutation {mutation} was not rebuilt");
            };
            assert!(evidence.iter().any(|item| item.original_path == target));
            assert_eq!(reopened.database.applied_batch_count().unwrap(), 1);
        }
    }

    #[test]
    fn forensic_preservation_and_rebuild_resume_after_subprocess_crashes() {
        for (index, hook) in ["after-move:1", "after-evidence"].into_iter().enumerate() {
            let seed = 10_000 + index as u128 * 100;
            let dir = TestDir::new(&format!("forensic-{hook}"));
            let (ids, store, accepted_engine, path) = prepare_crash_case(&dir, seed);
            fs::write(&path, b"corrupt SQLite evidence").unwrap();
            fs::write(sidecar_path(&path, "-wal"), b"partial wal").unwrap();
            fs::write(sidecar_path(&path, "-shm"), b"partial shm").unwrap();
            let mut child = spawn_test_helper(
                "recover",
                dir.path(),
                seed,
                &[("TINE_SQLITE_FORENSIC_ABORT", hook)],
            );
            wait_for_file(&dir.path().join("helper-ready"));
            assert!(!child.wait().unwrap().success());
            let reopened = open_test_projection(
                &path,
                ids.claim(),
                RebuildSource::new(&accepted_engine, &store).unwrap(),
            )
            .unwrap();
            let ProjectionRecovery::RebuiltPreservingEvidence { evidence, .. } = &reopened.recovery
            else {
                panic!("forensic crash {hook} was not resumed");
            };
            assert_eq!(evidence.len(), 3);
            assert!(evidence.iter().all(|item| item.preserved_path.exists()));
            assert_eq!(reopened.database.applied_batch_count().unwrap(), 1);
        }

        let seed = 10_200;
        let dir = TestDir::new("rebuild-crash");
        let (ids, store, mut accepted_engine, path) = prepare_crash_case(&dir, seed);
        let child = accepted_engine
            .prepare_transaction(
                author(seed + 101),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "second".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut accepted_engine, &store, &child);
        fs::write(&path, b"corrupt before rebuild").unwrap();
        let mut helper = spawn_test_helper(
            "recover",
            dir.path(),
            seed,
            &[("TINE_SQLITE_REBUILD_ABORT_AFTER", "1")],
        );
        wait_for_file(&dir.path().join("helper-ready"));
        assert!(!helper.wait().unwrap().success());
        let reopened = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&accepted_engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            reopened.recovery,
            ProjectionRecovery::RebuiltPreservingEvidence { .. }
        ));
        assert_eq!(reopened.database.applied_batch_count().unwrap(), 2);
        assert_eq!(
            reopened.database.frontier().unwrap(),
            accepted_engine.exact_frontier().unwrap()
        );
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
                connection
                    .execute(
                        "UPDATE frontier
                         SET frontier_root_digest = zeroblob(32)
                         WHERE singleton = 1",
                        [],
                    )
                    .unwrap();
            }
            drop(connection);
            let rebuilt = open_test_projection(
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
