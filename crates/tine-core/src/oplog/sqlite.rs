//! Disposable SQLite frontier projection for the sparse operation log.
//!
//! This module deliberately accepts only already-accepted operation events. It
//! has no mutation-authoring API and is never part of keystroke durability.
//! Callers place the disposable database in device-local app data. The
//! single-writer workspace lease is capability-relative to the exact
//! authoritative [`ObjectStore`] used for rebuild, so changing app-data
//! environment variables or the disposable database path cannot split it.
//! Accepted ancestry is a two-level authenticated index: the durable accepted
//! frontier commits `BatchId -> (manifest, binding, dot, clock root)` records,
//! and each clock root addresses a persistent peer-counter treap. Updates copy
//! only changed search paths; unchanged clock subtrees are shared by digest.
//!
//! The lease uses the platform's advisory file-lock primitive through `fs2`.
//! Dropping the applier or terminating its process releases the lock on Linux,
//! macOS, Windows, and Android. The small lock file remains as diagnostic
//! metadata, but never decides ownership by its contents.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
#[cfg(unix)]
use cap_std::fs::MetadataExt as CapMetadataExt;
#[cfg(windows)]
use cap_std::fs::OpenOptions as CapOpenOptions;
use cap_std::{ambient_authority, fs::Dir as CapDir};
use fs2::FileExt as _;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::hot_engine::AcceptedFrontierRoot;
use super::{
    BatchCausalDot, BatchId, BatchInspection, CausalPeerId, ContentDigest, DocumentDependencies,
    DocumentId, FrontierV2, LineageDigest, ObjectKind, ObjectStore, SemanticEffect,
    SemanticEffectDigest, ShardedHotEngine, ValidatedBatch, WorkspaceId, WorkspaceStatus,
    MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION, OBJECT_ENVELOPE_SCHEMA_VERSION,
    OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};

pub const SQLITE_APPLICATION_ID: u32 = 0x5449_4e45;
pub const SQLITE_SCHEMA_VERSION: u32 = 5;
pub const TAIL_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const TAIL_MAX_BATCHES: usize = 10_000;

const EXPECTED_TABLES: [&str; 6] = [
    "accepted_batch_nodes",
    "applied_batches",
    "causal_clock_nodes",
    "frontier",
    "frontier_documents",
    "meta",
];
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
    dependencies_digest BLOB NOT NULL CHECK (length(dependencies_digest) = 32),
    left_document_id BLOB,
    left_digest BLOB,
    right_document_id BLOB,
    right_digest BLOB,
    node_digest BLOB NOT NULL CHECK (length(node_digest) = 32),
    CHECK ((left_document_id IS NULL AND left_digest IS NULL)
        OR (length(left_document_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_document_id IS NULL AND right_digest IS NULL)
        OR (length(right_document_id) = 16 AND length(right_digest) = 32))
) STRICT";
const CAUSAL_CLOCK_NODES_DDL: &str = "CREATE TABLE causal_clock_nodes (
    node_digest BLOB PRIMARY KEY CHECK (length(node_digest) = 32),
    peer_id BLOB NOT NULL CHECK (length(peer_id) = 16),
    counter INTEGER NOT NULL CHECK (counter > 0),
    value_digest BLOB NOT NULL CHECK (length(value_digest) = 32),
    left_peer_id BLOB,
    left_digest BLOB,
    right_peer_id BLOB,
    right_digest BLOB,
    CHECK ((left_peer_id IS NULL AND left_digest IS NULL)
        OR (length(left_peer_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_peer_id IS NULL AND right_digest IS NULL)
        OR (length(right_peer_id) = 16 AND length(right_digest) = 32))
) STRICT";
const ACCEPTED_BATCH_NODES_DDL: &str = "CREATE TABLE accepted_batch_nodes (
    node_digest BLOB PRIMARY KEY CHECK (length(node_digest) = 32),
    batch_id BLOB NOT NULL CHECK (length(batch_id) = 16),
    value_digest BLOB NOT NULL CHECK (length(value_digest) = 32),
    left_batch_id BLOB,
    left_digest BLOB,
    right_batch_id BLOB,
    right_digest BLOB,
    CHECK ((left_batch_id IS NULL AND left_digest IS NULL)
        OR (length(left_batch_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_batch_id IS NULL AND right_digest IS NULL)
        OR (length(right_batch_id) = 16 AND length(right_digest) = 32))
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
    causal_peer_id BLOB NOT NULL CHECK (length(causal_peer_id) = 16),
    causal_counter INTEGER NOT NULL CHECK (causal_counter > 0),
    causal_clock_root_key BLOB NOT NULL CHECK (length(causal_clock_root_key) = 16),
    causal_clock_root_digest BLOB NOT NULL CHECK (length(causal_clock_root_digest) = 32),
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
const FRONTIER_DOCUMENT_COLUMNS: [&str; 8] = [
    "document_id",
    "dependencies",
    "dependencies_digest",
    "left_document_id",
    "left_digest",
    "right_document_id",
    "right_digest",
    "node_digest",
];
const CAUSAL_CLOCK_NODE_COLUMNS: [&str; 8] = [
    "node_digest",
    "peer_id",
    "counter",
    "value_digest",
    "left_peer_id",
    "left_digest",
    "right_peer_id",
    "right_digest",
];
const ACCEPTED_BATCH_NODE_COLUMNS: [&str; 7] = [
    "node_digest",
    "batch_id",
    "value_digest",
    "left_batch_id",
    "left_digest",
    "right_batch_id",
    "right_digest",
];
const APPLIED_BATCH_COLUMNS: [&str; 20] = [
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
    "causal_peer_id",
    "causal_counter",
    "causal_clock_root_key",
    "causal_clock_root_digest",
    "acceptance_sequence",
    "retained_bytes",
];
const FORENSIC_SUFFIXES: [&str; 4] = ["", "-wal", "-shm", "-auth"];
const FORENSIC_NAMES: [&str; 4] = ["database", "wal", "shm", "auth"];
const PROJECTION_CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const PROJECTION_FINGERPRINT_CHUNK_BYTES: usize = 64 * 1024;
const MAX_PROJECTION_CHECKPOINT_BYTES: u64 = 64 * 1024;
const MAX_AUTHENTICATED_MAP_DEPTH: usize = 256;
const OBJECT_STORE_LEASE_NAMESPACE: &str = ".tine-runtime";
const SQLITE_WORKSPACE_LEASE_NAMESPACE: &str = "sqlite-workspaces";
const SQLITE_APPLIER_LEASE_FILE: &str = "sqlite-applier.lock";

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
    causal_dot: BatchCausalDot,
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

    fn from_indexed(
        engine: &ShardedHotEngine,
        store: &ObjectStore,
        batch_id: BatchId,
        evidence: &super::AcceptedBatchEvidence,
    ) -> Result<Self, ProjectionError> {
        if engine.workspace_id() != store.workspace_id() {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: engine.workspace_id(),
                found: store.workspace_id(),
            });
        }
        evidence
            .validate()
            .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
        if evidence.batch_id() != batch_id {
            return Err(ProjectionError::InvalidAcceptedEvent(
                "accepted sequence evidence is bound to another batch".into(),
            ));
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
        Self::from_validated(&validated, evidence)
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
        evidence
            .validate()
            .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
        let manifest_digest = ContentDigest::of(&manifest_bytes);
        if manifest_digest != evidence.manifest_fingerprint() {
            return Err(ProjectionError::ManifestMismatch {
                batch_id: manifest.batch_id(),
                expected: evidence.manifest_fingerprint(),
                found: manifest_digest,
            });
        }
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
            manifest_digest,
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
            manifest_digest,
            event_binding_digest,
            semantic_effect,
            semantic_effect_digest,
            dependency_frontier: manifest.dependency_frontier().clone(),
            prior_frontier_root: evidence.prior_frontier_root().clone(),
            post_frontier_root: evidence.post_frontier_root().clone(),
            affected_documents: evidence.affected_documents().to_vec(),
            acceptance_sequence: evidence.acceptance_sequence(),
            causal_dependency_heads: manifest.causal_dependency_heads().to_vec(),
            causal_dot: manifest.causal_dot(),
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

    pub const fn causal_dot(&self) -> BatchCausalDot {
        self.causal_dot
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationRuntimeRoot {
    path: PathBuf,
}

impl ApplicationRuntimeRoot {
    /// Open Tine's platform-selected device-local application-data root.
    ///
    /// This root may guide disposable projection placement, but it is not a
    /// lease authority. The process lease is rooted in the exact
    /// [`ObjectStore`] capability supplied through [`RebuildSource`].
    pub fn open() -> Result<Self, ProjectionError> {
        let path = platform_application_runtime_root()?;
        let path = prepare_application_runtime_root(&path)?;
        Ok(Self { path })
    }

    #[cfg(test)]
    fn open_for_test(path: &Path) -> Result<Self, ProjectionError> {
        let path = prepare_application_runtime_root(path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn platform_application_runtime_root() -> Result<PathBuf, ProjectionError> {
    let base = dirs::data_local_dir().ok_or_else(|| {
        ProjectionError::UnsafePath(
            "platform did not provide a canonical per-user local-data directory".into(),
        )
    })?;
    let application_id = if cfg!(target_os = "android") {
        "page.tine.app"
    } else {
        "page.tine.Tine"
    };
    Ok(base.join(application_id).join("runtime"))
}

pub struct RebuildSource<'a> {
    engine: &'a ShardedHotEngine,
    store: &'a ObjectStore,
    exact_frontier_root: AcceptedFrontierRoot,
    accepted_batch_count: u64,
}

struct RebuildCursor<'a> {
    source: &'a RebuildSource<'a>,
    accepted: super::hot_engine::AcceptedBatchCursor<'a>,
}

impl RebuildCursor<'_> {
    fn next_event(&mut self) -> Result<Option<AcceptedBatchEvent>, ProjectionError> {
        let Some((sequence, batch_id, indexed_evidence)) = self
            .accepted
            .next_batch()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?
        else {
            return Ok(None);
        };
        let event = match indexed_evidence {
            Some(evidence) => AcceptedBatchEvent::from_indexed(
                self.source.engine,
                self.source.store,
                batch_id,
                &evidence,
            )?,
            None => {
                AcceptedBatchEvent::from_accepted(self.source.engine, self.source.store, batch_id)?
            }
        };
        if event.acceptance_sequence != sequence {
            return Err(ProjectionError::Rebuild(format!(
                "accepted batch {batch_id} is indexed at sequence {sequence} but carries {}",
                event.acceptance_sequence
            )));
        }
        Ok(Some(event))
    }

    fn page_stats(&self) -> (usize, usize, usize) {
        self.accepted.page_stats()
    }
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
        let (batch_id, indexed_evidence) = self
            .engine
            .accepted_batch_entry_at(acceptance_sequence)
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?
            .ok_or_else(|| {
                ProjectionError::Rebuild(format!(
                    "accepted history is missing sequence {acceptance_sequence}"
                ))
            })?;
        let event = match indexed_evidence {
            Some(evidence) => {
                AcceptedBatchEvent::from_indexed(self.engine, self.store, batch_id, &evidence)?
            }
            None => AcceptedBatchEvent::from_accepted(self.engine, self.store, batch_id)?,
        };
        if event.acceptance_sequence != acceptance_sequence {
            return Err(ProjectionError::Rebuild(format!(
                "accepted batch {batch_id} is indexed at sequence {acceptance_sequence} but carries {}",
                event.acceptance_sequence
            )));
        }
        Ok(event)
    }

    fn cursor(&'a self) -> Result<RebuildCursor<'a>, ProjectionError> {
        Ok(RebuildCursor {
            source: self,
            accepted: self
                .engine
                .accepted_batch_cursor()
                .map_err(|error| ProjectionError::Rebuild(error.to_string()))?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForensicEvidence {
    pub original_path: PathBuf,
    pub preserved_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundedFileCheckpoint {
    length: u64,
    first_chunk_digest: ContentDigest,
    last_chunk_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionCheckpoint {
    schema_version: u32,
    workspace_id: WorkspaceId,
    frontier_root_digest: ContentDigest,
    database: BoundedFileCheckpoint,
    wal: Option<BoundedFileCheckpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionCheckpointEnvelope {
    checkpoint: ProjectionCheckpoint,
    digest: ContentDigest,
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
    pub accepted_sequence_page_reads: usize,
    pub accepted_sequence_bytes_read: usize,
    pub max_accepted_sequence_page_bytes: usize,
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

pub struct TailOverlay {
    hot_descriptors: BTreeMap<u64, TailDescriptor>,
    retained_bytes: usize,
    authoritative_retained_bytes_total: u64,
    applied_retained_bytes_total: u64,
    authoritative_through: u64,
    applied_through: u64,
    descriptor_overflow: bool,
    reservations: BTreeMap<u64, usize>,
    reserved_bytes: usize,
    next_reservation_id: u64,
}

impl TailOverlay {
    #[cfg(test)]
    fn empty_for_test() -> Self {
        Self {
            hot_descriptors: BTreeMap::new(),
            retained_bytes: 0,
            authoritative_retained_bytes_total: 0,
            applied_retained_bytes_total: 0,
            authoritative_through: 0,
            applied_through: 0,
            descriptor_overflow: false,
            reservations: BTreeMap::new(),
            reserved_bytes: 0,
            next_reservation_id: 0,
        }
    }

    #[cfg(test)]
    fn hot_descriptor_count(&self) -> usize {
        self.hot_descriptors.len()
    }

    pub fn from_durable(
        database: &SqliteFrontier,
        source: &RebuildSource<'_>,
    ) -> Result<Self, TailOverlayError> {
        let applied = database.frontier_root()?;
        let accepted = &source.exact_frontier_root;
        if applied.acceptance_sequence() > accepted.acceptance_sequence()
            || applied.retained_bytes_total() > accepted.retained_bytes_total()
        {
            return Err(ProjectionError::FrontierRegression.into());
        }
        let retained_bytes = usize::try_from(
            accepted
                .retained_bytes_total()
                .saturating_sub(applied.retained_bytes_total()),
        )
        .map_err(|_| {
            ProjectionError::Corrupt("durable accepted backlog exceeds addressable memory".into())
        })?;
        let authoritative_pending = accepted
            .acceptance_sequence()
            .saturating_sub(applied.acceptance_sequence());
        Ok(Self {
            hot_descriptors: BTreeMap::new(),
            retained_bytes,
            authoritative_retained_bytes_total: accepted.retained_bytes_total(),
            applied_retained_bytes_total: applied.retained_bytes_total(),
            authoritative_through: accepted.acceptance_sequence(),
            applied_through: applied.acceptance_sequence(),
            descriptor_overflow: authoritative_pending > TAIL_MAX_BATCHES as u64
                || retained_bytes > TAIL_MAX_BYTES,
            reservations: BTreeMap::new(),
            reserved_bytes: 0,
            next_reservation_id: 0,
        })
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
        let applied = database.frontier_root()?;
        self.applied_through = self.applied_through.max(applied.acceptance_sequence());
        self.applied_retained_bytes_total = self
            .applied_retained_bytes_total
            .max(applied.retained_bytes_total());
        if event.acceptance_sequence <= applied.acceptance_sequence() {
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
        if event.acceptance_sequence > self.authoritative_through {
            let authoritative_retained_bytes_total =
                if event.post_frontier_root.acceptance_sequence() == event.acceptance_sequence {
                    event.post_frontier_root.retained_bytes_total()
                } else {
                    #[cfg(test)]
                    {
                        // Large-tail tests use private synthetic descriptors
                        // without manufacturing authenticated frontier roots.
                        self.authoritative_retained_bytes_total
                            .saturating_add(u64::try_from(event.retained_bytes).unwrap_or(u64::MAX))
                    }
                    #[cfg(not(test))]
                    {
                        return Err(ProjectionError::InvalidAcceptedEvent(
                            "accepted event sequence differs from its authenticated post-root"
                                .into(),
                        )
                        .into());
                    }
                };
            self.authoritative_retained_bytes_total =
                authoritative_retained_bytes_total.max(self.authoritative_retained_bytes_total);
            self.authoritative_through = event.acceptance_sequence;
            self.refresh_retained_bytes()?;
        }
        if self.hot_descriptors.len() < TAIL_MAX_BATCHES && self.retained_bytes <= TAIL_MAX_BYTES {
            self.hot_descriptors
                .insert(event.acceptance_sequence, descriptor);
        } else {
            self.descriptor_overflow = true;
        }
        Ok(true)
    }

    fn refresh_retained_bytes(&mut self) -> Result<(), TailOverlayError> {
        let retained_bytes = self
            .authoritative_retained_bytes_total
            .checked_sub(self.applied_retained_bytes_total)
            .ok_or(ProjectionError::FrontierRegression)?;
        self.retained_bytes = usize::try_from(retained_bytes).map_err(|_| {
            ProjectionError::Corrupt("durable accepted backlog exceeds addressable memory".into())
        })?;
        Ok(())
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
            database.apply_accepted(&event)?;
            self.hot_descriptors.remove(&expected_sequence);
            self.applied_through = expected_sequence;
            self.applied_retained_bytes_total = event.post_frontier_root.retained_bytes_total();
            self.refresh_retained_bytes()?;
            applied += 1;
        }
        if self.applied_through >= self.authoritative_through {
            self.descriptor_overflow = false;
        }
        Ok(applied)
    }
}

/// One leased device-local projection handle.
///
/// The projection's authoritative ObjectStore/workspace lease lives exactly
/// as long as this value, independent of the app-data root and projection
/// database's file name.
/// A clean drop or process termination releases the OS lock; a later process
/// validates the database before reuse and rebuilds from engine/store evidence
/// when deletion, stale state, corruption, or an interrupted WAL is observed.
pub struct SqliteFrontier {
    path: PathBuf,
    claim: ProjectionClaim,
    connection: Connection,
    checkpoint_each_apply: bool,
    _lease: Arc<ProcessLease>,
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
        _application_runtime_root: &ApplicationRuntimeRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
    ) -> Result<OpenProjection, ProjectionError> {
        validate_source(claim, &source)?;
        let path = prepare_database_path(path)?;
        let lease = Arc::new(ProcessLease::acquire(
            source.store,
            &path,
            claim.workspace_id,
        )?);
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
                                checkpoint_each_apply: true,
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
                            checkpoint_each_apply: true,
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
        lease: Arc<ProcessLease>,
        source: &RebuildSource<'_>,
    ) -> Result<(Self, RebuildInstrumentation), ProjectionError> {
        let candidate_path = candidate_database_path(path)?;
        remove_projection_files(&candidate_path)?;
        let mut candidate = Self::create_new(&candidate_path, claim, Arc::clone(&lease))?;
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
        drop(candidate);
        if sidecar_path(&candidate_path, "-wal").exists()
            || sidecar_path(&candidate_path, "-shm").exists()
        {
            remove_projection_files(&candidate_path)?;
            return Err(ProjectionError::Corrupt(
                "checkpointed SQLite candidate retained sidecars".into(),
            ));
        }
        match fs::remove_file(sidecar_path(&candidate_path, "-auth")) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(&candidate_path, path)?;
        sync_directory(
            path.parent()
                .ok_or_else(|| ProjectionError::UnsafePath("database has no parent".into()))?,
        )?;
        let connection = open_writable(path)?;
        let root = read_frontier_root(&connection)?;
        write_projection_checkpoint(path, claim, &root)?;
        Ok((
            Self {
                path: path.to_path_buf(),
                claim,
                connection,
                checkpoint_each_apply: true,
                _lease: lease,
            },
            rebuild,
        ))
    }

    fn create_new(
        path: &Path,
        claim: ProjectionClaim,
        lease: Arc<ProcessLease>,
    ) -> Result<Self, ProjectionError> {
        let connection = open_writable(path)?;
        initialize_schema(&connection, claim)?;
        let root = read_frontier_root(&connection)?;
        write_projection_checkpoint(path, claim, &root)?;
        Ok(Self {
            path: path.to_path_buf(),
            claim,
            connection,
            checkpoint_each_apply: false,
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
        let root = read_frontier_root(&self.connection)?;
        for needed in required.documents() {
            let Some(have) =
                authenticated_frontier_document(&self.connection, &root, needed.document_id())?
            else {
                return Ok(false);
            };
            if !document_frontier_contains(&self.connection, &have, needed)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn applied_batch_count(&self) -> Result<usize, ProjectionError> {
        usize::try_from(read_frontier_root(&self.connection)?.acceptance_sequence())
            .map_err(|_| ProjectionError::Corrupt("applied sequence exceeds usize".into()))
    }

    /// Explicit full diagnostic. Normal startup and apply never call this
    /// lifetime-history scan.
    pub fn diagnose_full_integrity(&self) -> Result<(), ProjectionError> {
        validate_integrity(&self.connection)?;
        let applied_rows: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM applied_batches", [], |row| row.get(0))?;
        let document_rows: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM frontier_documents", [], |row| {
                    row.get(0)
                })?;
        let root = read_frontier_root(&self.connection)?;
        if u64::try_from(applied_rows).ok() != Some(root.acceptance_sequence())
            || u64::try_from(document_rows).ok() != Some(root.document_count())
        {
            return Err(ProjectionError::Corrupt(
                "SQLite diagnostic row counts differ from the authenticated frontier".into(),
            ));
        }
        let (history_root, history_count) = validate_stored_history(&self.connection)?;
        if history_count != root.acceptance_sequence() || history_root != root {
            return Err(ProjectionError::Corrupt(
                "SQLite diagnostic history scan differs from the authenticated frontier".into(),
            ));
        }
        let _ = read_frontier_documents(&self.connection)?;
        Ok(())
    }

    pub fn contains_batch(&self, batch_id: BatchId) -> Result<bool, ProjectionError> {
        let root = read_frontier_root(&self.connection)?;
        authenticated_batch_record(&self.connection, &root, batch_id, &mut 0)
            .map(|record| record.is_some())
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

    /// Test-only recovery inspection of the exact semantic records rebuilt into
    /// this frontier. Production consumers retain only the authenticated
    /// frontier APIs above.
    #[cfg(test)]
    pub(crate) fn applied_semantic_effects_for_test(
        &self,
    ) -> Result<Vec<SemanticEffect>, ProjectionError> {
        let mut statement = self
            .connection
            .prepare("SELECT semantic_effect FROM applied_batches ORDER BY sequence")?;
        let mut rows = statement.query([])?;
        let mut effects = Vec::new();
        while let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            effects.push(
                SemanticEffect::decode(&bytes)
                    .map_err(|error| ProjectionError::Corrupt(error.to_string()))?,
            );
        }
        Ok(effects)
    }

    fn rebuild_stream(
        &mut self,
        source: &RebuildSource<'_>,
    ) -> Result<RebuildInstrumentation, ProjectionError> {
        let mut instrumentation = RebuildInstrumentation::default();
        let mut cursor = source.cursor()?;
        while let Some(event) = cursor.next_event()? {
            instrumentation.accepted_events_validated += 1;
            instrumentation.max_live_events = instrumentation.max_live_events.max(1);
            instrumentation.max_live_evidence_records =
                instrumentation.max_live_evidence_records.max(1);
            self.apply_internal(&event, ApplyFault::None)?;
            instrumentation.accepted_events_applied += 1;
            maybe_abort_rebuild_test(instrumentation.accepted_events_applied);
        }
        let (page_reads, page_bytes, max_page_bytes) = cursor.page_stats();
        instrumentation.accepted_sequence_page_reads = page_reads;
        instrumentation.accepted_sequence_bytes_read = page_bytes;
        instrumentation.max_accepted_sequence_page_bytes = max_page_bytes;
        if read_frontier_root(&self.connection)? != source.exact_frontier_root {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's authenticated frontier root".into(),
            ));
        }
        if read_frontier_root(&self.connection)?.acceptance_sequence()
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
        let current_root = read_frontier_root(&self.connection)?;
        if let Some(existing) = load_batch(&self.connection, event.batch_id)? {
            let mut rows_read = 0;
            if authenticated_batch_record(
                &self.connection,
                &current_root,
                event.batch_id,
                &mut rows_read,
            )?
            .is_none()
            {
                return Err(ProjectionError::Corrupt(format!(
                    "stored batch {} is absent from the authenticated accepted map",
                    event.batch_id
                )));
            }
            if existing.matches(event)? {
                if current_root.acceptance_sequence() >= event.acceptance_sequence
                    && current_root.state_digest() != event.prior_frontier_root.state_digest()
                {
                    return Ok(ApplyDisposition::Duplicate);
                }
                return Err(ProjectionError::FrontierRegression);
            }
            return Err(ProjectionError::BatchCollision(event.batch_id));
        }

        for dependency in &event.causal_dependency_heads {
            let mut rows_read = 0;
            if authenticated_batch_record(
                &self.connection,
                &current_root,
                *dependency,
                &mut rows_read,
            )?
            .is_none()
            {
                return Err(ProjectionError::MissingDependency(*dependency));
            }
        }
        let expected_acceptance_sequence = current_root
            .acceptance_sequence()
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("applied batch sequence overflowed".into()))?;
        if event.acceptance_sequence != expected_acceptance_sequence {
            return Err(ProjectionError::AcceptanceOrder {
                expected: expected_acceptance_sequence,
                found: event.acceptance_sequence,
            });
        }
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
                    u64::try_from(event.retained_bytes).map_err(|_| {
                        ProjectionError::InvalidAcceptedEvent(
                            "accepted retained bytes exceed u64".into(),
                        )
                    })?,
                    &event.affected_documents,
                    &event.post_frontier_root,
                )
                .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?
        {
            return Err(ProjectionError::InvalidAcceptedEvent(
                "accepted event is not bound to its authenticated frontier transition".into(),
            ));
        }
        for document in &event.affected_documents {
            let _ = authenticated_frontier_document(
                &self.connection,
                &current_root,
                document.document_id(),
            )?;
            if !document.direct_dependency_heads().contains(&event.batch_id) {
                return Err(ProjectionError::InvalidAcceptedEvent(format!(
                    "affected document {} does not name accepted batch {} as a direct head",
                    document.document_id(),
                    event.batch_id
                )));
            }
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
        let causal_clock_root = derive_causal_clock_root(&transaction, &current_root, event)?;
        let causal_record_digest = super::hot_engine::accepted_causal_record_digest(
            event.batch_id,
            event.manifest_digest,
            event.event_binding_digest,
            event.causal_dot,
            Some(causal_clock_root.key),
            causal_clock_root.digest,
        );
        let prior_batch_map_root = current_root.batch_map_root_key().map(|key| MapLink {
            key,
            digest: current_root.batch_map_root_digest(),
        });
        let post_batch_map_root = upsert_accepted_batch_map(
            &transaction,
            prior_batch_map_root,
            event.batch_id,
            causal_record_digest,
        )?;
        if event.post_frontier_root.batch_map_root_key() != Some(post_batch_map_root.key)
            || event.post_frontier_root.batch_map_root_digest() != post_batch_map_root.digest
        {
            return Err(ProjectionError::FrontierRegression);
        }
        insert_event(
            &transaction,
            usize::try_from(expected_acceptance_sequence)
                .map_err(|_| ProjectionError::Corrupt("applied sequence exceeds usize".into()))?,
            event,
            &dependency_frontier,
            &prior_frontier_root,
            &post_frontier_root,
            &affected_documents,
            &causal_dependencies,
            &causal_clock_root,
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
        let mut map_root_key = current_root.document_map_root_key();
        let mut map_root_digest = current_root.document_map_root_digest();
        let mut new_documents = 0_u64;
        for document in &event.affected_documents {
            let (root, inserted) =
                upsert_frontier_map(&transaction, map_root_key, map_root_digest, document)?;
            map_root_key = Some(root.document_id.as_uuid().into_bytes());
            map_root_digest = root.digest;
            new_documents = new_documents.saturating_add(u64::from(inserted));
        }
        if event.post_frontier_root.document_count()
            != current_root.document_count().saturating_add(new_documents)
            || event.post_frontier_root.document_map_root_key() != map_root_key
            || event.post_frontier_root.document_map_root_digest() != map_root_digest
        {
            return Err(ProjectionError::FrontierRegression);
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
                i64::try_from(expected_acceptance_sequence).map_err(|_| {
                    ProjectionError::Corrupt("applied batch sequence exceeds SQLite".into())
                })?
            ],
        )?;
        transaction.commit()?;
        if self.checkpoint_each_apply {
            write_projection_checkpoint(&self.path, self.claim, &event.post_frontier_root)?;
        }
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

impl Drop for SqliteFrontier {
    fn drop(&mut self) {
        let _ = self
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        if let Ok(root) = read_frontier_root(&self.connection) {
            let _ = write_projection_checkpoint(&self.path, self.claim, &root);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    transaction: &Transaction<'_>,
    sequence: usize,
    event: &AcceptedBatchEvent,
    dependency_frontier: &[u8],
    prior_frontier_root: &[u8],
    post_frontier_root: &[u8],
    affected_documents: &[u8],
    causal_dependencies: &[u8],
    causal_clock_root: &MapLink,
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
             causal_peer_id, causal_counter, causal_clock_root_key,
             causal_clock_root_digest,
             acceptance_sequence, retained_bytes
         ) VALUES (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
             ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
             ?17, ?18, ?19, ?20
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
            uuid_blob(&event.causal_dot.peer_id().as_device_id().as_uuid()),
            i64::try_from(event.causal_dot.counter()).map_err(|_| {
                ProjectionError::InvalidAcceptedEvent("causal counter exceeds SQLite".into())
            })?,
            causal_clock_root.key.as_slice(),
            causal_clock_root.digest.as_bytes().as_slice(),
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
    validate_projection_checkpoint(path, claim, &source.exact_frontier_root)
        .map_err(|error| error.to_string())?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open SQLite projection read-only: {error}"))?;
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
    if let Some(root_key) = found_frontier.document_map_root_key() {
        let root_id = DocumentId::from_uuid(Uuid::from_bytes(root_key));
        load_frontier_map_node(
            &connection,
            root_id,
            Some(found_frontier.document_map_root_digest()),
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "SQLite authenticated frontier root row is missing".to_string())?;
    } else if found_frontier.document_count() != 0 {
        return Err("SQLite authenticated frontier root key is missing".into());
    }
    if expected_count > 0 {
        let final_record = load_batch_at_sequence(&connection, expected_count)
            .map_err(|error| error.to_string())?;
        let final_record =
            final_record.ok_or_else(|| "SQLite final accepted row is missing".to_string())?;
        let prior_root = decode_frontier_root(&final_record.prior_frontier_root)
            .map_err(|error| error.to_string())?;
        let final_root = final_record
            .validate_canonical_transition(&prior_root)
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
    let wal = read_file_prefix(&wal_path, 32)?;
    let shm = read_file_prefix(&shm_path, 136)?;
    if shm.is_some() && wal.is_none() {
        return Err(ProjectionError::Corrupt(
            "SQLite SHM exists without its WAL".into(),
        ));
    }
    if let Some((wal_len, wal)) = wal {
        if wal_len < 32 {
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
            || (wal_len - 32) % (24 + page_size as u64) != 0
        {
            return Err(ProjectionError::Corrupt(
                "SQLite WAL frame layout is invalid".into(),
            ));
        }
    }
    if let Some((shm_len, shm)) = shm {
        if shm_len < 136 {
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

fn read_file_prefix(path: &Path, limit: usize) -> Result<Option<(u64, Vec<u8>)>, ProjectionError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ProjectionError::UnsafePath(format!(
            "SQLite sidecar {} is not a regular file",
            path.display()
        )));
    }
    let mut file = OpenOptions::new().read(true).open(path)?;
    let length = usize::try_from(metadata.len().min(limit as u64))
        .map_err(|_| ProjectionError::Corrupt("SQLite sidecar length exceeds usize".into()))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    Ok(Some((metadata.len(), bytes)))
}

fn validate_projection_checkpoint(
    path: &Path,
    claim: ProjectionClaim,
    expected_root: &AcceptedFrontierRoot,
) -> Result<(), ProjectionError> {
    let checkpoint_path = sidecar_path(path, "-auth");
    let metadata = fs::symlink_metadata(&checkpoint_path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_PROJECTION_CHECKPOINT_BYTES
    {
        return Err(ProjectionError::Corrupt(
            "SQLite projection checkpoint is not a bounded regular file".into(),
        ));
    }
    let bytes = fs::read(&checkpoint_path)?;
    let envelope: ProjectionCheckpointEnvelope = postcard::from_bytes(&bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid checkpoint: {error}")))?;
    let checkpoint_bytes = postcard::to_allocvec(&envelope.checkpoint)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    if postcard::to_allocvec(&envelope)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?
        != bytes
        || envelope.digest != ContentDigest::of(&checkpoint_bytes)
        || envelope.checkpoint.schema_version != PROJECTION_CHECKPOINT_SCHEMA_VERSION
        || envelope.checkpoint.workspace_id != claim.workspace_id
    {
        return Err(ProjectionError::Corrupt(
            "SQLite projection checkpoint authentication failed".into(),
        ));
    }
    let expected_root_bytes = canonical_frontier_root_bytes(expected_root)?;
    if envelope.checkpoint.frontier_root_digest != ContentDigest::of(&expected_root_bytes)
        || envelope.checkpoint.database != bounded_file_checkpoint(path)?
        || envelope.checkpoint.wal != optional_bounded_file_checkpoint(&sidecar_path(path, "-wal"))?
    {
        return Err(ProjectionError::Corrupt(
            "SQLite projection files differ from their authenticated checkpoint".into(),
        ));
    }
    Ok(())
}

fn write_projection_checkpoint(
    path: &Path,
    claim: ProjectionClaim,
    root: &AcceptedFrontierRoot,
) -> Result<(), ProjectionError> {
    let root_bytes = canonical_frontier_root_bytes(root)?;
    let checkpoint = ProjectionCheckpoint {
        schema_version: PROJECTION_CHECKPOINT_SCHEMA_VERSION,
        workspace_id: claim.workspace_id,
        frontier_root_digest: ContentDigest::of(&root_bytes),
        database: bounded_file_checkpoint(path)?,
        wal: optional_bounded_file_checkpoint(&sidecar_path(path, "-wal"))?,
    };
    let checkpoint_bytes = postcard::to_allocvec(&checkpoint)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    let envelope = ProjectionCheckpointEnvelope {
        digest: ContentDigest::of(&checkpoint_bytes),
        checkpoint,
    };
    let bytes = postcard::to_allocvec(&envelope)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    let checkpoint_path = sidecar_path(path, "-auth");
    let parent = checkpoint_path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("checkpoint has no parent".into()))?;
    let name = checkpoint_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectionError::UnsafePath("checkpoint name is not UTF-8".into()))?;
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, &checkpoint_path)?;
    sync_directory(parent)
}

fn optional_bounded_file_checkpoint(
    path: &Path,
) -> Result<Option<BoundedFileCheckpoint>, ProjectionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.len() == 0 => Ok(None),
        Ok(_) => bounded_file_checkpoint(path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn bounded_file_checkpoint(path: &Path) -> Result<BoundedFileCheckpoint, ProjectionError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ProjectionError::UnsafePath(format!(
            "SQLite projection file {} is not regular",
            path.display()
        )));
    }
    let length = metadata.len();
    let chunk_len = usize::try_from(bounded_file_checkpoint_sample_bytes(length) / 2)
        .map_err(|_| ProjectionError::Corrupt("projection file length exceeds usize".into()))?;
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut first = vec![0_u8; chunk_len];
    file.read_exact(&mut first)?;
    let mut last = vec![0_u8; chunk_len];
    if length > chunk_len as u64 {
        file.seek(SeekFrom::Start(length - chunk_len as u64))?;
        file.read_exact(&mut last)?;
    } else {
        last.copy_from_slice(&first);
    }
    let mut first_bound = b"tine/sqlite/checkpoint/v1/first\0".to_vec();
    first_bound.extend_from_slice(&length.to_be_bytes());
    first_bound.extend_from_slice(&first);
    let mut last_bound = b"tine/sqlite/checkpoint/v1/last\0".to_vec();
    last_bound.extend_from_slice(&length.to_be_bytes());
    last_bound.extend_from_slice(&last);
    Ok(BoundedFileCheckpoint {
        length,
        first_chunk_digest: ContentDigest::of(&first_bound),
        last_chunk_digest: ContentDigest::of(&last_bound),
    })
}

fn bounded_file_checkpoint_sample_bytes(length: u64) -> u64 {
    length
        .min(PROJECTION_FINGERPRINT_CHUNK_BYTES as u64)
        .saturating_mul(2)
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
         {CAUSAL_CLOCK_NODES_DDL};
         {ACCEPTED_BATCH_NODES_DDL};
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
    validate_table_columns(connection, "causal_clock_nodes", &CAUSAL_CLOCK_NODE_COLUMNS)?;
    validate_table_columns(
        connection,
        "accepted_batch_nodes",
        &ACCEPTED_BATCH_NODE_COLUMNS,
    )?;
    validate_table_columns(connection, "applied_batches", &APPLIED_BATCH_COLUMNS)?;
    validate_schema_sql(connection, "table", "meta", META_DDL)?;
    validate_schema_sql(connection, "table", "frontier", FRONTIER_DDL)?;
    validate_schema_sql(
        connection,
        "table",
        "frontier_documents",
        FRONTIER_DOCUMENTS_DDL,
    )?;
    validate_schema_sql(
        connection,
        "table",
        "causal_clock_nodes",
        CAUSAL_CLOCK_NODES_DDL,
    )?;
    validate_schema_sql(
        connection,
        "table",
        "accepted_batch_nodes",
        ACCEPTED_BATCH_NODES_DDL,
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
    causal_peer_id: Vec<u8>,
    causal_counter: i64,
    causal_clock_root_key: Vec<u8>,
    causal_clock_root_digest: Vec<u8>,
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
            && self.causal_dot()? == event.causal_dot
            && self.causal_clock_root_key.len() == 16
            && self.causal_clock_root_digest.len() == 32
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
            || self.causal_counter <= 0
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
        if self.causal_clock_root_key.len() != 16 || self.causal_clock_root_digest.len() != 32 {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} causal clock is invalid",
                self.batch_id
            )));
        }
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
                self.retained_bytes as u64,
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

    fn causal_dot(&self) -> Result<BatchCausalDot, ProjectionError> {
        let peer = CausalPeerId::from_device_id(super::DeviceId::from_uuid(decode_uuid(
            &self.causal_peer_id,
        )?));
        let counter = u64::try_from(self.causal_counter)
            .map_err(|_| ProjectionError::Corrupt("stored causal counter is invalid".into()))?;
        BatchCausalDot::new(peer, counter)
            .map_err(|error| ProjectionError::Corrupt(error.to_string()))
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
                causal_peer_id, causal_counter, causal_clock_root_key,
                causal_clock_root_digest,
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
        let post = record.validate_canonical_transition(&prior)?;
        let mut rows_read = 0;
        if authenticated_batch_record(connection, &post, record.batch_id, &mut rows_read)?.is_none()
        {
            return Err(ProjectionError::Corrupt(format!(
                "stored batch {} is absent from its authenticated accepted map",
                record.batch_id
            )));
        }
        prior = post;
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
                    causal_peer_id, causal_counter, causal_clock_root_key,
                    causal_clock_root_digest,
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
                causal_peer_id, causal_counter, causal_clock_root_key,
                causal_clock_root_digest,
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
        causal_peer_id: row.get(14)?,
        causal_counter: row.get(15)?,
        causal_clock_root_key: row.get(16)?,
        causal_clock_root_digest: row.get(17)?,
        acceptance_sequence: row.get(18)?,
        retained_bytes: row.get(19)?,
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
    postcard::to_allocvec(document)
        .map_err(|error| ProjectionError::InvalidFrontier(error.to_string()))
}

fn decode_frontier_document(
    expected_document_id: DocumentId,
    bytes: &[u8],
) -> Result<DocumentDependencies, ProjectionError> {
    let document: DocumentDependencies = postcard::from_bytes(bytes)
        .map_err(|error| ProjectionError::Corrupt(format!("invalid frontier document: {error}")))?;
    if document.document_id() != expected_document_id
        || encode_frontier_document(&document)
            .map_err(|error| ProjectionError::Corrupt(error.to_string()))?
            != bytes
    {
        return Err(ProjectionError::Corrupt(
            "frontier document row has mismatched identity".into(),
        ));
    }
    Ok(document)
}

#[derive(Clone)]
struct FrontierMapLink {
    document_id: DocumentId,
    digest: ContentDigest,
}

#[derive(Clone)]
struct FrontierMapNode {
    document: DocumentDependencies,
    encoded: Vec<u8>,
    value_digest: ContentDigest,
    left: Option<FrontierMapLink>,
    right: Option<FrontierMapLink>,
    node_digest: ContentDigest,
}

impl FrontierMapNode {
    fn key(&self) -> [u8; 16] {
        self.document.document_id().as_uuid().into_bytes()
    }

    fn recompute_digest(&self) -> ContentDigest {
        super::scratch_store::authenticated_map_node_digest(
            self.key(),
            self.value_digest,
            self.left
                .as_ref()
                .map(|child| (child.document_id.as_uuid().into_bytes(), child.digest)),
            self.right
                .as_ref()
                .map(|child| (child.document_id.as_uuid().into_bytes(), child.digest)),
        )
    }

    fn as_link(&self) -> FrontierMapLink {
        FrontierMapLink {
            document_id: self.document.document_id(),
            digest: self.node_digest,
        }
    }
}

fn authenticated_frontier_document(
    connection: &Connection,
    root: &AcceptedFrontierRoot,
    document_id: DocumentId,
) -> Result<Option<DocumentDependencies>, ProjectionError> {
    let mut current = match root.document_map_root_key() {
        Some(root_id) => Some(FrontierMapLink {
            document_id: DocumentId::from_uuid(Uuid::from_bytes(root_id)),
            digest: root.document_map_root_digest(),
        }),
        None => {
            if root.document_count() != 0
                || root.document_map_root_digest()
                    != super::scratch_store::authenticated_map_empty_digest()
            {
                return Err(ProjectionError::Corrupt(
                    "empty frontier map root is malformed".into(),
                ));
            }
            None
        }
    };
    let mut depth = 0_usize;
    while let Some(link) = current {
        if depth > 256 {
            return Err(ProjectionError::Corrupt(
                "frontier map exceeds its bounded depth".into(),
            ));
        }
        let node = load_frontier_map_node(connection, link.document_id, Some(link.digest))?
            .ok_or_else(|| {
                ProjectionError::Corrupt(format!(
                    "authenticated frontier node {} is missing",
                    link.document_id
                ))
            })?;
        match document_id.cmp(&node.document.document_id()) {
            std::cmp::Ordering::Equal => return Ok(Some(node.document)),
            std::cmp::Ordering::Less => current = node.left,
            std::cmp::Ordering::Greater => current = node.right,
        }
        depth += 1;
    }
    Ok(None)
}

fn upsert_frontier_map(
    transaction: &Transaction<'_>,
    root_key: Option<[u8; 16]>,
    root_digest: ContentDigest,
    document: &DocumentDependencies,
) -> Result<(FrontierMapLink, bool), ProjectionError> {
    let current = root_key.map(|key| FrontierMapLink {
        document_id: DocumentId::from_uuid(Uuid::from_bytes(key)),
        digest: root_digest,
    });
    upsert_frontier_map_link(transaction, current, document, 0)
}

fn upsert_frontier_map_link(
    transaction: &Transaction<'_>,
    current: Option<FrontierMapLink>,
    document: &DocumentDependencies,
    depth: usize,
) -> Result<(FrontierMapLink, bool), ProjectionError> {
    if depth > 256 {
        return Err(ProjectionError::Corrupt(
            "frontier map exceeds its bounded depth".into(),
        ));
    }
    let Some(current) = current else {
        let encoded = encode_frontier_document(document)?;
        let value_digest = ContentDigest::of(&encoded);
        let mut node = FrontierMapNode {
            document: document.clone(),
            encoded,
            value_digest,
            left: None,
            right: None,
            node_digest: super::scratch_store::authenticated_map_empty_digest(),
        };
        node.node_digest = node.recompute_digest();
        store_frontier_map_node(transaction, &node)?;
        return Ok((node.as_link(), true));
    };
    let mut node = load_frontier_map_node(transaction, current.document_id, Some(current.digest))?
        .ok_or_else(|| {
            ProjectionError::Corrupt(format!(
                "authenticated frontier node {} is missing",
                current.document_id
            ))
        })?;
    let inserted;
    match document.document_id().cmp(&node.document.document_id()) {
        std::cmp::Ordering::Equal => {
            node.document = document.clone();
            node.encoded = encode_frontier_document(document)?;
            node.value_digest = ContentDigest::of(&node.encoded);
            inserted = false;
        }
        std::cmp::Ordering::Less => {
            let (left, was_inserted) =
                upsert_frontier_map_link(transaction, node.left.take(), document, depth + 1)?;
            node.left = Some(left);
            inserted = was_inserted;
            if node.left.as_ref().is_some_and(|left| {
                super::scratch_store::authenticated_map_priority_order(
                    left.document_id.as_uuid().into_bytes(),
                    node.document.document_id().as_uuid().into_bytes(),
                )
                .is_lt()
            }) {
                return Ok((rotate_frontier_map_right(transaction, node)?, inserted));
            }
        }
        std::cmp::Ordering::Greater => {
            let (right, was_inserted) =
                upsert_frontier_map_link(transaction, node.right.take(), document, depth + 1)?;
            node.right = Some(right);
            inserted = was_inserted;
            if node.right.as_ref().is_some_and(|right| {
                super::scratch_store::authenticated_map_priority_order(
                    right.document_id.as_uuid().into_bytes(),
                    node.document.document_id().as_uuid().into_bytes(),
                )
                .is_lt()
            }) {
                return Ok((rotate_frontier_map_left(transaction, node)?, inserted));
            }
        }
    }
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    Ok((node.as_link(), inserted))
}

fn rotate_frontier_map_right(
    transaction: &Transaction<'_>,
    mut node: FrontierMapNode,
) -> Result<FrontierMapLink, ProjectionError> {
    let left = node.left.take().ok_or_else(|| {
        ProjectionError::Corrupt("frontier map right rotation has no left child".into())
    })?;
    let mut left_node = load_frontier_map_node(transaction, left.document_id, Some(left.digest))?
        .ok_or_else(|| {
        ProjectionError::Corrupt("frontier map rotation child is missing".into())
    })?;
    node.left = left_node.right.take();
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    left_node.right = Some(node.as_link());
    left_node.node_digest = left_node.recompute_digest();
    store_frontier_map_node(transaction, &left_node)?;
    Ok(left_node.as_link())
}

fn rotate_frontier_map_left(
    transaction: &Transaction<'_>,
    mut node: FrontierMapNode,
) -> Result<FrontierMapLink, ProjectionError> {
    let right = node.right.take().ok_or_else(|| {
        ProjectionError::Corrupt("frontier map left rotation has no right child".into())
    })?;
    let mut right_node =
        load_frontier_map_node(transaction, right.document_id, Some(right.digest))?.ok_or_else(
            || ProjectionError::Corrupt("frontier map rotation child is missing".into()),
        )?;
    node.right = right_node.left.take();
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    right_node.left = Some(node.as_link());
    right_node.node_digest = right_node.recompute_digest();
    store_frontier_map_node(transaction, &right_node)?;
    Ok(right_node.as_link())
}

fn store_frontier_map_node(
    transaction: &Transaction<'_>,
    node: &FrontierMapNode,
) -> Result<(), ProjectionError> {
    if node.node_digest != node.recompute_digest() {
        return Err(ProjectionError::Corrupt(
            "frontier map node digest is stale".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO frontier_documents (
             document_id, dependencies, dependencies_digest,
             left_document_id, left_digest, right_document_id, right_digest, node_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(document_id) DO UPDATE SET
             dependencies = excluded.dependencies,
             dependencies_digest = excluded.dependencies_digest,
             left_document_id = excluded.left_document_id,
             left_digest = excluded.left_digest,
             right_document_id = excluded.right_document_id,
             right_digest = excluded.right_digest,
             node_digest = excluded.node_digest",
        params![
            uuid_blob(&node.document.document_id().as_uuid()),
            &node.encoded,
            node.value_digest.as_bytes().as_slice(),
            node.left
                .as_ref()
                .map(|child| uuid_blob(&child.document_id.as_uuid())),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().to_vec()),
            node.right
                .as_ref()
                .map(|child| uuid_blob(&child.document_id.as_uuid())),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().to_vec()),
            node.node_digest.as_bytes().as_slice(),
        ],
    )?;
    Ok(())
}

fn load_frontier_map_node(
    connection: &Connection,
    document_id: DocumentId,
    expected_digest: Option<ContentDigest>,
) -> Result<Option<FrontierMapNode>, ProjectionError> {
    type StoredFrontierMapRow = (
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Vec<u8>,
    );
    let found: Option<StoredFrontierMapRow> = connection
        .query_row(
            "SELECT dependencies, dependencies_digest,
                    left_document_id, left_digest, right_document_id, right_digest, node_digest
             FROM frontier_documents WHERE document_id = ?1",
            [uuid_blob(&document_id.as_uuid())],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((encoded, value_digest, left_id, left_digest, right_id, right_digest, node_digest)) =
        found
    else {
        return Ok(None);
    };
    let value_digest = decode_content_digest(&value_digest)?;
    if value_digest != ContentDigest::of(&encoded) {
        return Err(ProjectionError::Corrupt(format!(
            "frontier document {document_id} digest mismatch"
        )));
    }
    let decode_link = |id: Option<Vec<u8>>,
                       digest: Option<Vec<u8>>|
     -> Result<Option<FrontierMapLink>, ProjectionError> {
        match (id, digest) {
            (None, None) => Ok(None),
            (Some(id), Some(digest)) => Ok(Some(FrontierMapLink {
                document_id: decode_document_id(&id)?,
                digest: decode_content_digest(&digest)?,
            })),
            _ => Err(ProjectionError::Corrupt(
                "frontier map child identity/digest pair is incomplete".into(),
            )),
        }
    };
    let left = decode_link(left_id, left_digest)?;
    let right = decode_link(right_id, right_digest)?;
    if left
        .as_ref()
        .is_some_and(|child| child.document_id >= document_id)
        || right
            .as_ref()
            .is_some_and(|child| child.document_id <= document_id)
    {
        return Err(ProjectionError::Corrupt(
            "frontier map child ordering is invalid".into(),
        ));
    }
    let mut node = FrontierMapNode {
        document: decode_frontier_document(document_id, &encoded)?,
        encoded,
        value_digest,
        left,
        right,
        node_digest: decode_content_digest(&node_digest)?,
    };
    let computed = node.recompute_digest();
    if node.node_digest != computed || expected_digest.is_some_and(|expected| expected != computed)
    {
        return Err(ProjectionError::Corrupt(format!(
            "frontier document {document_id} is not authenticated by its map root"
        )));
    }
    node.node_digest = computed;
    Ok(Some(node))
}

fn read_frontier_documents(connection: &Connection) -> Result<FrontierV2, ProjectionError> {
    let root = read_frontier_root(connection)?;
    let mut pending = root
        .document_map_root_key()
        .map(|key| FrontierMapLink {
            document_id: DocumentId::from_uuid(Uuid::from_bytes(key)),
            digest: root.document_map_root_digest(),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut documents = Vec::with_capacity(
        usize::try_from(root.document_count())
            .unwrap_or(1_000_000)
            .min(1_000_000),
    );
    while let Some(link) = pending.pop() {
        let node = load_frontier_map_node(connection, link.document_id, Some(link.digest))?
            .ok_or_else(|| {
                ProjectionError::Corrupt(format!(
                    "authenticated frontier node {} is missing",
                    link.document_id
                ))
            })?;
        if let Some(right) = node.right.clone() {
            pending.push(right);
        }
        documents.push(node.document);
        if let Some(left) = node.left {
            pending.push(left);
        }
    }
    documents.sort_unstable_by_key(DocumentDependencies::document_id);
    if documents.len() as u64 != root.document_count() {
        return Err(ProjectionError::Corrupt(
            "authenticated frontier document count is stale".into(),
        ));
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
    batch_descends_from_database_measured(connection, descendant, ancestor)
        .map(|(contained, _)| contained)
}

fn batch_descends_from_database_measured(
    connection: &Connection,
    descendant: BatchId,
    ancestor: BatchId,
) -> Result<(bool, usize), ProjectionError> {
    let root = read_frontier_root(connection)?;
    let mut rows_read = 0;
    let descendant_record =
        authenticated_batch_record(connection, &root, descendant, &mut rows_read)?.ok_or_else(
            || {
                ProjectionError::Corrupt(format!(
                    "descendant batch {descendant} is absent from the authenticated accepted map"
                ))
            },
        )?;
    let Some(ancestor_record) =
        authenticated_batch_record(connection, &root, ancestor, &mut rows_read)?
    else {
        return Ok((false, rows_read));
    };
    let ancestor_dot = ancestor_record.causal_dot()?;
    let root = descendant_record.clock_root()?;
    let counter = causal_clock_lookup(
        connection,
        Some(root),
        ancestor_dot.peer_id(),
        &mut rows_read,
    )?;
    Ok((
        counter.is_some_and(|counter| counter >= ancestor_dot.counter()),
        rows_read,
    ))
}

fn derive_causal_clock_root(
    transaction: &Transaction<'_>,
    accepted_root: &AcceptedFrontierRoot,
    event: &AcceptedBatchEvent,
) -> Result<MapLink, ProjectionError> {
    let mut root = None;
    let mut rows_read = 0;
    for parent in &event.causal_dependency_heads {
        let record =
            authenticated_batch_record(transaction, accepted_root, *parent, &mut rows_read)?
                .ok_or(ProjectionError::MissingDependency(*parent))?;
        root = merge_causal_clock_roots(transaction, root, Some(record.clock_root()?))?;
    }
    let expected = causal_clock_lookup(
        transaction,
        root.clone(),
        event.causal_dot.peer_id(),
        &mut rows_read,
    )?
    .unwrap_or(0)
    .checked_add(1)
    .ok_or_else(|| ProjectionError::Corrupt("causal counter overflowed".into()))?;
    if event.causal_dot.counter() != expected {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted batch {} causal counter {} does not follow {}",
            event.batch_id,
            event.causal_dot.counter(),
            expected.saturating_sub(1)
        )));
    }
    upsert_causal_clock(
        transaction,
        root,
        event.causal_dot.peer_id(),
        event.causal_dot.counter(),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MapLink {
    key: [u8; 16],
    digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClockNode {
    peer: CausalPeerId,
    counter: u64,
    left: Option<MapLink>,
    right: Option<MapLink>,
}

#[derive(Clone, Debug)]
struct BatchMapNode {
    batch_id: BatchId,
    value_digest: ContentDigest,
    left: Option<MapLink>,
    right: Option<MapLink>,
}

impl StoredBatch {
    fn clock_root(&self) -> Result<MapLink, ProjectionError> {
        Ok(MapLink {
            key: self
                .causal_clock_root_key
                .as_slice()
                .try_into()
                .map_err(|_| ProjectionError::Corrupt("causal clock root key is invalid".into()))?,
            digest: decode_content_digest(&self.causal_clock_root_digest)?,
        })
    }

    fn causal_record_digest(&self) -> Result<ContentDigest, ProjectionError> {
        let manifest_digest = decode_content_digest(&self.manifest_digest)?;
        let semantic_effect_digest = decode_semantic_effect_digest(&self.semantic_effect_digest)?;
        let dependency_frontier = decode_frontier(&self.dependency_frontier)?;
        let causal_dependency_heads = decode_batch_ids(&self.causal_dependency_heads)?;
        let binding = super::AcceptedBatchEvidence::binding_digest_for(
            self.batch_id,
            manifest_digest,
            semantic_effect_digest,
            &dependency_frontier,
            &causal_dependency_heads,
        )
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
        let clock_root = self.clock_root()?;
        Ok(super::hot_engine::accepted_causal_record_digest(
            self.batch_id,
            manifest_digest,
            binding,
            self.causal_dot()?,
            Some(clock_root.key),
            clock_root.digest,
        ))
    }
}

fn authenticated_batch_record(
    connection: &Connection,
    root: &AcceptedFrontierRoot,
    batch_id: BatchId,
    rows_read: &mut usize,
) -> Result<Option<StoredBatch>, ProjectionError> {
    let Some(root_key) = root.batch_map_root_key() else {
        if root.acceptance_sequence() == 0 {
            return Ok(None);
        }
        return Err(ProjectionError::Corrupt(
            "nonempty frontier has no authenticated batch-map root".into(),
        ));
    };
    let mut current = Some(MapLink {
        key: root_key,
        digest: root.batch_map_root_digest(),
    });
    let mut value_digest = None;
    while let Some(link) = current {
        let node = load_batch_map_node(connection, &link)?;
        *rows_read = rows_read.saturating_add(1);
        match batch_id.cmp(&node.batch_id) {
            std::cmp::Ordering::Equal => {
                value_digest = Some(node.value_digest);
                break;
            }
            std::cmp::Ordering::Less => current = node.left,
            std::cmp::Ordering::Greater => current = node.right,
        }
    }
    let Some(value_digest) = value_digest else {
        return Ok(None);
    };
    let record = load_batch(connection, batch_id)?.ok_or_else(|| {
        ProjectionError::Corrupt(format!(
            "authenticated accepted batch {batch_id} is missing its exact record"
        ))
    })?;
    *rows_read = rows_read.saturating_add(1);
    if record.causal_record_digest()? != value_digest {
        return Err(ProjectionError::Corrupt(format!(
            "accepted batch {batch_id} differs from its authenticated causal record"
        )));
    }
    let dot = record.causal_dot()?;
    let counter = causal_clock_lookup(
        connection,
        Some(record.clock_root()?),
        dot.peer_id(),
        rows_read,
    )?;
    if counter != Some(dot.counter()) {
        return Err(ProjectionError::Corrupt(format!(
            "accepted batch {batch_id} causal dot is absent from its authenticated clock"
        )));
    }
    Ok(Some(record))
}

fn causal_clock_lookup(
    connection: &Connection,
    mut current: Option<MapLink>,
    peer: CausalPeerId,
    rows_read: &mut usize,
) -> Result<Option<u64>, ProjectionError> {
    let mut depth = 0;
    while let Some(link) = current {
        ensure_authenticated_map_depth(depth, "causal clock lookup")?;
        let node = load_clock_node(connection, &link)?;
        *rows_read = rows_read.saturating_add(1);
        match peer.cmp(&node.peer) {
            std::cmp::Ordering::Equal => return Ok(Some(node.counter)),
            std::cmp::Ordering::Less => current = node.left,
            std::cmp::Ordering::Greater => current = node.right,
        }
        depth += 1;
    }
    Ok(None)
}

fn load_clock_node(
    connection: &Connection,
    expected: &MapLink,
) -> Result<ClockNode, ProjectionError> {
    let stored = connection
        .query_row(
            "SELECT peer_id, counter, value_digest, left_peer_id, left_digest,
                    right_peer_id, right_digest
             FROM causal_clock_nodes WHERE node_digest = ?1",
            [expected.digest.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            ProjectionError::Corrupt(format!(
                "authenticated causal clock node {} is missing",
                expected.digest
            ))
        })?;
    let peer = decode_causal_peer(&stored.0)?;
    let counter = u64::try_from(stored.1)
        .map_err(|_| ProjectionError::Corrupt("causal clock counter is invalid".into()))?;
    let value_digest = decode_content_digest(&stored.2)?;
    let left = decode_map_link(stored.3, stored.4)?;
    let right = decode_map_link(stored.5, stored.6)?;
    let node = ClockNode {
        peer,
        counter,
        left,
        right,
    };
    validate_clock_node(expected, value_digest, &node)?;
    Ok(node)
}

fn validate_clock_node(
    expected: &MapLink,
    value_digest: ContentDigest,
    node: &ClockNode,
) -> Result<(), ProjectionError> {
    let key = causal_peer_key(node.peer);
    if expected.key != key
        || node.counter == 0
        || value_digest != super::hot_engine::causal_clock_counter_digest(node.peer, node.counter)
        || !valid_map_children(key, node.left.as_ref(), node.right.as_ref())
        || super::scratch_store::authenticated_map_node_digest(
            key,
            value_digest,
            node.left.as_ref().map(|child| (child.key, child.digest)),
            node.right.as_ref().map(|child| (child.key, child.digest)),
        ) != expected.digest
    {
        return Err(ProjectionError::Corrupt(
            "authenticated causal clock node is misbound".into(),
        ));
    }
    Ok(())
}

fn load_batch_map_node(
    connection: &Connection,
    expected: &MapLink,
) -> Result<BatchMapNode, ProjectionError> {
    let stored = connection
        .query_row(
            "SELECT batch_id, value_digest, left_batch_id, left_digest,
                    right_batch_id, right_digest
             FROM accepted_batch_nodes WHERE node_digest = ?1",
            [expected.digest.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            ProjectionError::Corrupt(format!(
                "authenticated accepted-batch node {} is missing",
                expected.digest
            ))
        })?;
    let node = BatchMapNode {
        batch_id: BatchId::from_uuid(decode_uuid(&stored.0)?),
        value_digest: decode_content_digest(&stored.1)?,
        left: decode_map_link(stored.2, stored.3)?,
        right: decode_map_link(stored.4, stored.5)?,
    };
    let key = node.batch_id.as_uuid().into_bytes();
    if expected.key != key
        || !valid_map_children(key, node.left.as_ref(), node.right.as_ref())
        || super::scratch_store::authenticated_map_node_digest(
            key,
            node.value_digest,
            node.left.as_ref().map(|child| (child.key, child.digest)),
            node.right.as_ref().map(|child| (child.key, child.digest)),
        ) != expected.digest
    {
        return Err(ProjectionError::Corrupt(
            "authenticated accepted-batch node is misbound".into(),
        ));
    }
    Ok(node)
}

fn valid_map_children(key: [u8; 16], left: Option<&MapLink>, right: Option<&MapLink>) -> bool {
    left.is_none_or(|child| {
        child.key < key
            && super::scratch_store::authenticated_map_priority_order(key, child.key).is_lt()
    }) && right.is_none_or(|child| {
        child.key > key
            && super::scratch_store::authenticated_map_priority_order(key, child.key).is_lt()
    })
}

fn decode_map_link(
    key: Option<Vec<u8>>,
    digest: Option<Vec<u8>>,
) -> Result<Option<MapLink>, ProjectionError> {
    match (key, digest) {
        (None, None) => Ok(None),
        (Some(key), Some(digest)) => Ok(Some(MapLink {
            key: key
                .as_slice()
                .try_into()
                .map_err(|_| ProjectionError::Corrupt("authenticated map key is invalid".into()))?,
            digest: decode_content_digest(&digest)?,
        })),
        _ => Err(ProjectionError::Corrupt(
            "authenticated map child is incomplete".into(),
        )),
    }
}

fn causal_peer_key(peer: CausalPeerId) -> [u8; 16] {
    peer.as_device_id().as_uuid().into_bytes()
}

fn decode_causal_peer(bytes: &[u8]) -> Result<CausalPeerId, ProjectionError> {
    Ok(CausalPeerId::from_device_id(super::DeviceId::from_uuid(
        decode_uuid(bytes)?,
    )))
}

fn merge_causal_clock_roots(
    connection: &Connection,
    left: Option<MapLink>,
    right: Option<MapLink>,
) -> Result<Option<MapLink>, ProjectionError> {
    merge_causal_clock_roots_measured(connection, left, right, &mut ClockUnionStats::default())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ClockUnionStats {
    nodes_read: usize,
    nodes_written: usize,
    shared_subtrees: usize,
}

type ClockSplit = (Option<MapLink>, Option<u64>, Option<MapLink>);

fn merge_causal_clock_roots_measured(
    connection: &Connection,
    left: Option<MapLink>,
    right: Option<MapLink>,
    stats: &mut ClockUnionStats,
) -> Result<Option<MapLink>, ProjectionError> {
    union_causal_clock_roots(connection, left, right, stats, 0)
}

fn union_causal_clock_roots(
    connection: &Connection,
    left: Option<MapLink>,
    right: Option<MapLink>,
    stats: &mut ClockUnionStats,
    depth: usize,
) -> Result<Option<MapLink>, ProjectionError> {
    ensure_authenticated_map_depth(depth, "causal clock union")?;
    let (left_link, right_link) = match (left, right) {
        (None, right) => return Ok(right),
        (left, None) => return Ok(left),
        (Some(left), Some(right)) => (left, right),
    };
    if left_link == right_link {
        stats.shared_subtrees = stats.shared_subtrees.saturating_add(1);
        return Ok(Some(left_link));
    }

    if left_link.key == right_link.key {
        let left_node = load_clock_node_measured(connection, &left_link, stats)?;
        let right_node = load_clock_node_measured(connection, &right_link, stats)?;
        let merged = ClockNode {
            peer: left_node.peer,
            counter: left_node.counter.max(right_node.counter),
            left: union_causal_clock_roots(
                connection,
                left_node.left.clone(),
                right_node.left.clone(),
                stats,
                depth + 1,
            )?,
            right: union_causal_clock_roots(
                connection,
                left_node.right.clone(),
                right_node.right.clone(),
                stats,
                depth + 1,
            )?,
        };
        return Ok(Some(reuse_or_write_clock_node(
            connection,
            [(&left_link, &left_node), (&right_link, &right_node)],
            &merged,
            stats,
        )?));
    }

    if super::scratch_store::authenticated_map_priority_order(left_link.key, right_link.key).is_lt()
    {
        let left_node = load_clock_node_measured(connection, &left_link, stats)?;
        let (right_less, right_counter, right_greater) = split_causal_clock_root(
            connection,
            Some(right_link),
            left_link.key,
            stats,
            depth + 1,
        )?;
        let merged = ClockNode {
            peer: left_node.peer,
            counter: left_node.counter.max(right_counter.unwrap_or(0)),
            left: union_causal_clock_roots(
                connection,
                left_node.left.clone(),
                right_less,
                stats,
                depth + 1,
            )?,
            right: union_causal_clock_roots(
                connection,
                left_node.right.clone(),
                right_greater,
                stats,
                depth + 1,
            )?,
        };
        Ok(Some(reuse_or_write_clock_node(
            connection,
            [(&left_link, &left_node), (&left_link, &left_node)],
            &merged,
            stats,
        )?))
    } else {
        let right_node = load_clock_node_measured(connection, &right_link, stats)?;
        let (left_less, left_counter, left_greater) = split_causal_clock_root(
            connection,
            Some(left_link),
            right_link.key,
            stats,
            depth + 1,
        )?;
        let merged = ClockNode {
            peer: right_node.peer,
            counter: right_node.counter.max(left_counter.unwrap_or(0)),
            left: union_causal_clock_roots(
                connection,
                left_less,
                right_node.left.clone(),
                stats,
                depth + 1,
            )?,
            right: union_causal_clock_roots(
                connection,
                left_greater,
                right_node.right.clone(),
                stats,
                depth + 1,
            )?,
        };
        Ok(Some(reuse_or_write_clock_node(
            connection,
            [(&right_link, &right_node), (&right_link, &right_node)],
            &merged,
            stats,
        )?))
    }
}

fn split_causal_clock_root(
    connection: &Connection,
    root: Option<MapLink>,
    key: [u8; 16],
    stats: &mut ClockUnionStats,
    depth: usize,
) -> Result<ClockSplit, ProjectionError> {
    ensure_authenticated_map_depth(depth, "causal clock split")?;
    let Some(link) = root else {
        return Ok((None, None, None));
    };
    let node = load_clock_node_measured(connection, &link, stats)?;
    match key.cmp(&link.key) {
        std::cmp::Ordering::Equal => Ok((node.left, Some(node.counter), node.right)),
        std::cmp::Ordering::Less => {
            let (less, counter, greater_left) =
                split_causal_clock_root(connection, node.left.clone(), key, stats, depth + 1)?;
            let greater = ClockNode {
                left: greater_left,
                ..node.clone()
            };
            let greater = reuse_or_write_clock_node(
                connection,
                [(&link, &node), (&link, &node)],
                &greater,
                stats,
            )?;
            Ok((less, counter, Some(greater)))
        }
        std::cmp::Ordering::Greater => {
            let (less_right, counter, greater) =
                split_causal_clock_root(connection, node.right.clone(), key, stats, depth + 1)?;
            let less = ClockNode {
                right: less_right,
                ..node.clone()
            };
            let less = reuse_or_write_clock_node(
                connection,
                [(&link, &node), (&link, &node)],
                &less,
                stats,
            )?;
            Ok((Some(less), counter, greater))
        }
    }
}

fn load_clock_node_measured(
    connection: &Connection,
    link: &MapLink,
    stats: &mut ClockUnionStats,
) -> Result<ClockNode, ProjectionError> {
    let node = load_clock_node(connection, link)?;
    stats.nodes_read = stats.nodes_read.saturating_add(1);
    Ok(node)
}

fn reuse_or_write_clock_node<const N: usize>(
    connection: &Connection,
    candidates: [(&MapLink, &ClockNode); N],
    node: &ClockNode,
    stats: &mut ClockUnionStats,
) -> Result<MapLink, ProjectionError> {
    if let Some((link, _)) = candidates
        .into_iter()
        .find(|(_, candidate)| *candidate == node)
    {
        stats.shared_subtrees = stats.shared_subtrees.saturating_add(1);
        return Ok(link.clone());
    }
    stats.nodes_written = stats.nodes_written.saturating_add(1);
    write_clock_node(connection, node)
}

fn ensure_authenticated_map_depth(depth: usize, operation: &str) -> Result<(), ProjectionError> {
    if depth > MAX_AUTHENTICATED_MAP_DEPTH {
        return Err(ProjectionError::Corrupt(format!(
            "{operation} exceeds its bounded depth"
        )));
    }
    Ok(())
}

fn upsert_causal_clock(
    connection: &Connection,
    root: Option<MapLink>,
    peer: CausalPeerId,
    counter: u64,
) -> Result<MapLink, ProjectionError> {
    upsert_causal_clock_link(connection, root, peer, counter, 0)
}

fn upsert_causal_clock_link(
    connection: &Connection,
    root: Option<MapLink>,
    peer: CausalPeerId,
    counter: u64,
    depth: usize,
) -> Result<MapLink, ProjectionError> {
    ensure_authenticated_map_depth(depth, "causal clock update")?;
    let Some(root) = root else {
        return write_clock_node(
            connection,
            &ClockNode {
                peer,
                counter,
                left: None,
                right: None,
            },
        );
    };
    let mut node = load_clock_node(connection, &root)?;
    match peer.cmp(&node.peer) {
        std::cmp::Ordering::Equal => {
            node.counter = node.counter.max(counter);
            write_clock_node(connection, &node)
        }
        std::cmp::Ordering::Less => {
            node.left = Some(upsert_causal_clock_link(
                connection,
                node.left.take(),
                peer,
                counter,
                depth + 1,
            )?);
            if node.left.as_ref().is_some_and(|left| {
                super::scratch_store::authenticated_map_priority_order(
                    left.key,
                    causal_peer_key(node.peer),
                )
                .is_lt()
            }) {
                rotate_clock_right(connection, node)
            } else {
                write_clock_node(connection, &node)
            }
        }
        std::cmp::Ordering::Greater => {
            node.right = Some(upsert_causal_clock_link(
                connection,
                node.right.take(),
                peer,
                counter,
                depth + 1,
            )?);
            if node.right.as_ref().is_some_and(|right| {
                super::scratch_store::authenticated_map_priority_order(
                    right.key,
                    causal_peer_key(node.peer),
                )
                .is_lt()
            }) {
                rotate_clock_left(connection, node)
            } else {
                write_clock_node(connection, &node)
            }
        }
    }
}

fn rotate_clock_right(
    connection: &Connection,
    mut node: ClockNode,
) -> Result<MapLink, ProjectionError> {
    let left_link = node.left.take().ok_or_else(|| {
        ProjectionError::Corrupt("causal clock rotation has no left child".into())
    })?;
    let mut left = load_clock_node(connection, &left_link)?;
    node.left = left.right.take();
    left.right = Some(write_clock_node(connection, &node)?);
    write_clock_node(connection, &left)
}

fn rotate_clock_left(
    connection: &Connection,
    mut node: ClockNode,
) -> Result<MapLink, ProjectionError> {
    let right_link = node.right.take().ok_or_else(|| {
        ProjectionError::Corrupt("causal clock rotation has no right child".into())
    })?;
    let mut right = load_clock_node(connection, &right_link)?;
    node.right = right.left.take();
    right.left = Some(write_clock_node(connection, &node)?);
    write_clock_node(connection, &right)
}

fn write_clock_node(connection: &Connection, node: &ClockNode) -> Result<MapLink, ProjectionError> {
    let key = causal_peer_key(node.peer);
    let value_digest = super::hot_engine::causal_clock_counter_digest(node.peer, node.counter);
    let digest = super::scratch_store::authenticated_map_node_digest(
        key,
        value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    let link = MapLink { key, digest };
    validate_clock_node(&link, value_digest, node)?;
    connection.execute(
        "INSERT OR IGNORE INTO causal_clock_nodes (
             node_digest, peer_id, counter, value_digest, left_peer_id, left_digest,
             right_peer_id, right_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            digest.as_bytes().as_slice(),
            key.as_slice(),
            i64::try_from(node.counter)
                .map_err(|_| ProjectionError::Corrupt("causal counter exceeds SQLite".into()))?,
            value_digest.as_bytes().as_slice(),
            node.left.as_ref().map(|child| child.key.as_slice()),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.right.as_ref().map(|child| child.key.as_slice()),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
        ],
    )?;
    let _ = load_clock_node(connection, &link)?;
    Ok(link)
}

fn upsert_accepted_batch_map(
    connection: &Connection,
    root: Option<MapLink>,
    batch_id: BatchId,
    value_digest: ContentDigest,
) -> Result<MapLink, ProjectionError> {
    let Some(root) = root else {
        return write_batch_map_node(
            connection,
            &BatchMapNode {
                batch_id,
                value_digest,
                left: None,
                right: None,
            },
        );
    };
    let mut node = load_batch_map_node(connection, &root)?;
    match batch_id.cmp(&node.batch_id) {
        std::cmp::Ordering::Equal => {
            node.value_digest = value_digest;
            write_batch_map_node(connection, &node)
        }
        std::cmp::Ordering::Less => {
            node.left = Some(upsert_accepted_batch_map(
                connection,
                node.left.take(),
                batch_id,
                value_digest,
            )?);
            if node.left.as_ref().is_some_and(|left| {
                super::scratch_store::authenticated_map_priority_order(
                    left.key,
                    node.batch_id.as_uuid().into_bytes(),
                )
                .is_lt()
            }) {
                rotate_batch_map_right(connection, node)
            } else {
                write_batch_map_node(connection, &node)
            }
        }
        std::cmp::Ordering::Greater => {
            node.right = Some(upsert_accepted_batch_map(
                connection,
                node.right.take(),
                batch_id,
                value_digest,
            )?);
            if node.right.as_ref().is_some_and(|right| {
                super::scratch_store::authenticated_map_priority_order(
                    right.key,
                    node.batch_id.as_uuid().into_bytes(),
                )
                .is_lt()
            }) {
                rotate_batch_map_left(connection, node)
            } else {
                write_batch_map_node(connection, &node)
            }
        }
    }
}

fn rotate_batch_map_right(
    connection: &Connection,
    mut node: BatchMapNode,
) -> Result<MapLink, ProjectionError> {
    let left_link = node.left.take().ok_or_else(|| {
        ProjectionError::Corrupt("accepted batch-map rotation has no left child".into())
    })?;
    let mut left = load_batch_map_node(connection, &left_link)?;
    node.left = left.right.take();
    left.right = Some(write_batch_map_node(connection, &node)?);
    write_batch_map_node(connection, &left)
}

fn rotate_batch_map_left(
    connection: &Connection,
    mut node: BatchMapNode,
) -> Result<MapLink, ProjectionError> {
    let right_link = node.right.take().ok_or_else(|| {
        ProjectionError::Corrupt("accepted batch-map rotation has no right child".into())
    })?;
    let mut right = load_batch_map_node(connection, &right_link)?;
    node.right = right.left.take();
    right.left = Some(write_batch_map_node(connection, &node)?);
    write_batch_map_node(connection, &right)
}

fn write_batch_map_node(
    connection: &Connection,
    node: &BatchMapNode,
) -> Result<MapLink, ProjectionError> {
    let key = node.batch_id.as_uuid().into_bytes();
    let digest = super::scratch_store::authenticated_map_node_digest(
        key,
        node.value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    let link = MapLink { key, digest };
    connection.execute(
        "INSERT OR IGNORE INTO accepted_batch_nodes (
             node_digest, batch_id, value_digest, left_batch_id, left_digest,
             right_batch_id, right_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            digest.as_bytes().as_slice(),
            key.as_slice(),
            node.value_digest.as_bytes().as_slice(),
            node.left.as_ref().map(|child| child.key.as_slice()),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.right.as_ref().map(|child| child.key.as_slice()),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
        ],
    )?;
    let _ = load_batch_map_node(connection, &link)?;
    Ok(link)
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

fn prepare_application_runtime_root(path: &Path) -> Result<PathBuf, ProjectionError> {
    fs::create_dir_all(path)?;
    let direct_metadata = fs::symlink_metadata(path)?;
    if direct_metadata.file_type().is_symlink() || !direct_metadata.is_dir() {
        return Err(ProjectionError::UnsafePath(
            "application runtime root is not a no-follow directory".into(),
        ));
    }
    let canonical = fs::canonicalize(path)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProjectionError::UnsafePath(
            "application runtime root is not a real directory".into(),
        ));
    }
    #[cfg(unix)]
    // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(ProjectionError::UnsafePath(
            "application runtime root is not owned by the current user".into(),
        ));
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
        store: &ObjectStore,
        database_path: &Path,
        workspace_id: WorkspaceId,
    ) -> Result<Self, ProjectionError> {
        if store.workspace_id() != workspace_id {
            return Err(ProjectionError::WorkspaceMismatch {
                expected: workspace_id,
                found: store.workspace_id(),
            });
        }
        let store_root = store.sqlite_lease_capability().map_err(|error| {
            ProjectionError::UnsafePath(format!(
                "cannot retain ObjectStore lease authority: {error}"
            ))
        })?;
        let lease_namespace = open_or_create_lease_directory(
            &store_root,
            OBJECT_STORE_LEASE_NAMESPACE,
            "ObjectStore lease namespace",
        )?;
        let sqlite_namespace = open_or_create_lease_directory(
            &lease_namespace,
            SQLITE_WORKSPACE_LEASE_NAMESPACE,
            "SQLite workspace lease namespace",
        )?;
        let workspace_name = workspace_id.to_string();
        let workspace_root = open_or_create_lease_directory(
            &sqlite_namespace,
            &workspace_name,
            "SQLite workspace lease directory",
        )?;
        let workspace_lease_path = store
            .root_path()
            .join(OBJECT_STORE_LEASE_NAMESPACE)
            .join(SQLITE_WORKSPACE_LEASE_NAMESPACE)
            .join(&workspace_name)
            .join(SQLITE_APPLIER_LEASE_FILE);
        let mut workspace_file = lock_capability_lease_file(
            &workspace_root,
            SQLITE_APPLIER_LEASE_FILE,
            &workspace_lease_path,
        )?;
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
        super::object_store::sync_dir_required(&workspace_root)
            .map_err(|error| ProjectionError::Io(error.to_string()))?;
        let file_name = database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
        let database_lease_path =
            database_path.with_file_name(format!(".{file_name}.database-applier.lock"));
        let database_parent = database_lease_path.parent().ok_or_else(|| {
            ProjectionError::UnsafePath("database lease path has no parent".into())
        })?;
        let database_lease_name = database_lease_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                ProjectionError::UnsafePath("database lease file name is not UTF-8".into())
            })?;
        let database_parent = CapDir::open_ambient_dir(database_parent, ambient_authority())
            .map_err(|error| ProjectionError::Io(error.to_string()))?;
        let database_file = lock_capability_lease_file(
            &database_parent,
            database_lease_name,
            &database_lease_path,
        )?;
        Ok(Self {
            files: vec![workspace_file, database_file],
        })
    }
}

fn open_or_create_lease_directory(
    parent: &CapDir,
    name: &str,
    description: &str,
) -> Result<CapDir, ProjectionError> {
    let created = match parent.create_dir(name) {
        Ok(()) => true,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error.into()),
    };
    let directory = super::object_store::open_dir_nofollow(parent, name).map_err(|error| {
        ProjectionError::UnsafePath(format!(
            "{description} is not a no-follow directory: {error}"
        ))
    })?;
    #[cfg(unix)]
    if created {
        // SAFETY: `directory` is the retained descriptor returned by the
        // no-follow open above; `fchmod` changes that exact opened directory.
        if unsafe { libc::fchmod(directory.as_fd().as_raw_fd(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    validate_owned_lease_directory(&directory, description)?;
    if created {
        super::object_store::sync_dir_required(parent)
            .map_err(|error| ProjectionError::Io(error.to_string()))?;
    }
    Ok(directory)
}

fn validate_owned_lease_directory(
    directory: &CapDir,
    description: &str,
) -> Result<(), ProjectionError> {
    let metadata = directory.dir_metadata()?;
    if !metadata.is_dir() {
        return Err(ProjectionError::UnsafePath(format!(
            "{description} is not an opened directory"
        )));
    }
    #[cfg(unix)]
    // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
    if CapMetadataExt::uid(&metadata) != unsafe { libc::geteuid() }
        || CapMetadataExt::mode(&metadata) & 0o022 != 0
    {
        return Err(ProjectionError::UnsafePath(format!(
            "{description} is not exclusively writable by the current user"
        )));
    }
    Ok(())
}

fn lock_capability_lease_file(
    directory: &CapDir,
    name: &str,
    display_path: &Path,
) -> Result<File, ProjectionError> {
    let file = open_capability_lease_file(directory, name).map_err(|error| {
        ProjectionError::UnsafePath(format!(
            "cannot open SQLite applier lease {} without following links: {error}",
            display_path.display()
        ))
    })?;
    if let Err(error) = file.try_lock_exclusive() {
        if matches!(
            error.kind(),
            ErrorKind::WouldBlock | ErrorKind::PermissionDenied
        ) {
            return Err(ProjectionError::LeaseContended(display_path.to_path_buf()));
        }
        return Err(error.into());
    }
    validate_opened_lease_file(&file, display_path)?;
    Ok(file)
}

#[cfg(unix)]
fn open_capability_lease_file(directory: &CapDir, name: &str) -> std::io::Result<File> {
    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid lease file name"))?;
    // SAFETY: `name` is a live NUL-terminated relative name and `directory`
    // retains the authoritative ObjectStore or database-parent capability.
    // O_NOFOLLOW rejects a final-component symlink in the same open that
    // produces the handle subsequently locked and validated.
    let fd = unsafe {
        libc::openat(
            directory.as_fd().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
fn open_capability_lease_file(directory: &CapDir, name: &str) -> std::io::Result<File> {
    let mut options = CapOpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_capability_lease_file(_directory: &CapDir, _name: &str) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow lease files are unsupported on this target",
    ))
}

fn validate_opened_lease_file(file: &File, path: &Path) -> Result<(), ProjectionError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ProjectionError::UnsafePath(format!(
            "opened SQLite applier lease {} is not a regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
        unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(ProjectionError::UnsafePath(format!(
            "opened SQLite applier lease {} has unsafe ownership or links",
            path.display()
        )));
    }
    #[cfg(windows)]
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(ProjectionError::UnsafePath(format!(
            "opened SQLite applier lease {} is a reparse point",
            path.display()
        )));
    }
    Ok(())
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
        AuthorBatch, BatchCausalDot, BatchDisposition, BatchOrigin, BlockId, BlockLocation,
        CausalPeerId, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies, DocumentId,
        ManagedPath, ManagedTextKind, OperationBatch, OperationObject, OperationTransaction, PageId,
        PreparedBatch, SemanticOperation, SessionId,
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
        let runtime = ApplicationRuntimeRoot::open_for_test(&parent.join(".application-runtime"))?;
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

    fn constant_peer_author(seed: u128, index: usize) -> AuthorBatch {
        AuthorBatch {
            batch_id: batch(seed + 50_000 + index as u128),
            author_device_id: DeviceId::from_uuid(uuid(seed + 60_000)),
            author_session_id: SessionId::from_uuid(uuid(seed + 70_000)),
            crdt_peer_id: CrdtPeerId::from_u64((seed + 80_000) as u64),
        }
    }

    fn fresh_peer_author(seed: u128, index: usize) -> AuthorBatch {
        AuthorBatch {
            batch_id: batch(seed + 50_000 + index as u128),
            author_device_id: DeviceId::from_uuid(uuid(seed + 60_000 + index as u128)),
            author_session_id: SessionId::from_uuid(uuid(seed + 70_000 + index as u128)),
            crdt_peer_id: CrdtPeerId::from_u64(
                (seed + 80_000 + index as u128)
                    .try_into()
                    .expect("test peer fits u64"),
            ),
        }
    }

    fn root_transaction(ids: TestIds, path: &str, content: &str) -> OperationTransaction {
        OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                path: ManagedPath::parse(path).unwrap(),
                kind: ManagedTextKind::Page,
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
            .prepare_bootstrap_transaction(
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

    fn stored_semantic_effects(database: &SqliteFrontier) -> Vec<SemanticEffect> {
        let mut statement = database
            .connection
            .prepare("SELECT semantic_effect FROM applied_batches ORDER BY sequence")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|bytes| SemanticEffect::decode(&bytes.unwrap()).unwrap())
            .collect()
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
            BatchOrigin::BootstrapImport,
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
        let root_entry = test_causal_record_entry(
            &root,
            root_binding,
            vec![(root.manifest().causal_dot().peer_id(), 1)],
        );
        let root_evidence = super::super::AcceptedBatchEvidence::for_test(
            root_id,
            root_fingerprint,
            root_binding,
            AcceptedFrontierRoot::empty(),
            vec![root_document.clone()],
            vec![root_document],
            vec![root_entry],
            validated_retained_bytes(&root),
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
        let child_entry = test_causal_record_entry(
            &child,
            child_binding,
            vec![
                (root.manifest().causal_dot().peer_id(), 1),
                (child.manifest().causal_dot().peer_id(), 1),
            ],
        );
        let child_evidence = super::super::AcceptedBatchEvidence::for_test(
            child_id,
            child_fingerprint,
            child_binding,
            root_event.post_frontier_root.clone(),
            vec![child_document.clone()],
            vec![child_document],
            vec![root_entry, child_entry],
            validated_retained_bytes(&child),
        );
        let child_event = AcceptedBatchEvent::from_validated(&child, &child_evidence).unwrap();
        (root_event, child_event)
    }

    fn validated_retained_bytes(batch: &ValidatedBatch) -> u64 {
        let manifest = batch.manifest().encode().unwrap();
        batch
            .objects()
            .iter()
            .fold(manifest.len() as u64, |total, object| {
                total + object.encode().unwrap().len() as u64
            })
    }

    fn test_causal_record_entry(
        batch: &ValidatedBatch,
        binding: ContentDigest,
        mut clock: Vec<(CausalPeerId, u64)>,
    ) -> (BatchId, ContentDigest) {
        clock.sort_unstable_by_key(|(peer, _)| *peer);
        let (root_key, root_digest) =
            super::super::hot_engine::authenticated_causal_clock_root(&clock).unwrap();
        (
            batch.manifest().batch_id(),
            super::super::hot_engine::accepted_causal_record_digest(
                batch.manifest().batch_id(),
                ContentDigest::of(&batch.manifest().encode().unwrap()),
                binding,
                batch.manifest().causal_dot(),
                root_key,
                root_digest,
            ),
        )
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

        if mode == "production-lease-holder" || mode == "production-lease-contender" {
            let runtime = ApplicationRuntimeRoot::open().unwrap();
            let engine = ids.engine();
            let database_name = if mode == "production-lease-holder" {
                "db-a/frontier.sqlite"
            } else {
                "db-b/frontier.sqlite"
            };
            let result = SqliteFrontier::open_or_rebuild(
                &root.join(database_name),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            );
            if mode == "production-lease-contender" {
                assert!(matches!(result, Err(ProjectionError::LeaseContended(_))));
                return;
            }
            let _opened = result.unwrap();
            fs::write(&ready, b"ready").unwrap();
            loop {
                thread::park_timeout(Duration::from_secs(60));
            }
        }

        if mode == "injected-runtime-contender" {
            let would_be_runtime =
                PathBuf::from(std::env::var_os("TINE_SQLITE_HELPER_WOULD_BE_RUNTIME").unwrap());
            let runtime = ApplicationRuntimeRoot::open_for_test(&would_be_runtime).unwrap();
            let engine = ids.engine();
            let result = SqliteFrontier::open_or_rebuild(
                &root.join("db-b/fail-before.sqlite"),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            );
            assert!(matches!(result, Err(ProjectionError::LeaseContended(_))));
            return;
        }

        if mode == "production-lease-racer" {
            let label = std::env::var("TINE_SQLITE_RACER_LABEL").unwrap();
            let runtime = ApplicationRuntimeRoot::open().unwrap();
            fs::write(root.join(format!("race-ready-{label}")), b"ready").unwrap();
            wait_for_file(&root.join("race-go"));
            let engine = ids.engine();
            let result = SqliteFrontier::open_or_rebuild(
                &root.join(format!("db-{label}/frontier.sqlite")),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            );
            match result {
                Ok(_opened) => {
                    fs::write(root.join(format!("race-acquired-{label}")), b"acquired").unwrap();
                    wait_for_file(&root.join("race-stop"));
                }
                Err(ProjectionError::LeaseContended(_)) => {
                    fs::write(root.join(format!("race-contended-{label}")), b"contended").unwrap();
                }
                Err(error) => panic!("unexpected lease race error: {error}"),
            }
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
    fn ancestry_rejects_valid_local_clock_substitution_not_committed_by_accepted_root() {
        let ids = TestIds::new(2_025);
        let dir = TestDir::new("authenticated-clock-substitution");
        let (mut database, _engine, store) = open_empty(&dir, ids);
        let (root, child) = root_and_child_events(&store, ids);
        database.apply_accepted(&root).unwrap();
        database.apply_accepted(&child).unwrap();
        let root_record = load_batch(&database.connection, root.batch_id())
            .unwrap()
            .unwrap();
        database
            .connection
            .execute(
                "UPDATE applied_batches
                 SET causal_clock_root_key = ?1, causal_clock_root_digest = ?2
                 WHERE batch_id = ?3",
                params![
                    root_record.causal_clock_root_key,
                    root_record.causal_clock_root_digest,
                    uuid_blob(&child.batch_id().as_uuid()),
                ],
            )
            .unwrap();
        assert!(matches!(
            database.contains_frontier(&root.exact_frontier()),
            Err(ProjectionError::Corrupt(message))
                if message.contains("authenticated causal record")
        ));
    }

    #[test]
    fn ancestry_missing_authenticated_records_or_clock_nodes_require_rebuild() {
        for (case, seed) in [("batch-record", 2_050), ("clock-node", 2_075)] {
            let ids = TestIds::new(seed);
            let dir = TestDir::new(&format!("missing-authenticated-{case}"));
            let (mut database, _engine, store) = open_empty(&dir, ids);
            let (root, child) = root_and_child_events(&store, ids);
            database.apply_accepted(&root).unwrap();
            database.apply_accepted(&child).unwrap();
            let child_record = load_batch(&database.connection, child.batch_id())
                .unwrap()
                .unwrap();
            match case {
                "batch-record" => {
                    database
                        .connection
                        .execute(
                            "DELETE FROM applied_batches WHERE batch_id = ?1",
                            [uuid_blob(&child.batch_id().as_uuid())],
                        )
                        .unwrap();
                }
                "clock-node" => {
                    database
                        .connection
                        .execute(
                            "DELETE FROM causal_clock_nodes WHERE node_digest = ?1",
                            [child_record.causal_clock_root_digest],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    database.contains_frontier(&root.exact_frontier()),
                    Err(ProjectionError::Corrupt(_))
                ),
                "missing {case} did not fail closed"
            );
        }
    }

    #[test]
    fn accepted_events_keep_compact_historical_roots_and_structural_applied_closure() {
        let ids = TestIds::new(2_100);
        let dir = TestDir::new("historical-frontier");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine = ids.engine();
        let root = engine
            .prepare_bootstrap_transaction(
                author(2_200),
                &root_transaction(ids, "pages/root.md", "root"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &root);
        let early_root =
            AcceptedBatchEvent::from_accepted(&engine, &store, root.manifest().batch_id()).unwrap();

        let child = engine
            .prepare_bootstrap_transaction(
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
                path: ManagedPath::parse(format!("pages/wide-{index}.md")).unwrap(),
                kind: ManagedTextKind::Page,
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
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
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

    #[test]
    fn authenticated_frontier_map_rejects_rehashed_row_tampering_on_reopen() {
        const PAGE_COUNT: usize = 32;
        let ids = TestIds::new(2_350);
        let dir = TestDir::new("authenticated-frontier-row-tamper");
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut operations = Vec::with_capacity(PAGE_COUNT * 2);
        for index in 0..PAGE_COUNT as u128 {
            let page_id = PageId::from_uuid(uuid(30_000 + index * 3));
            let document_id = DocumentId::from_uuid(uuid(30_001 + index * 3));
            let block_id = BlockId::from_uuid(uuid(30_002 + index * 3));
            operations.push(SemanticOperation::CreatePage {
                page_id,
                home_document_id: document_id,
                path: ManagedPath::parse(format!("pages/auth-{index}.md")).unwrap(),
                kind: ManagedTextKind::Page,
            });
            operations.push(SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: document_id,
                },
                page_id,
                parent: None,
                order: "a".into(),
                content: format!("auth {index}"),
            });
        }
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(2_351),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let path = dir.path().join("frontier.sqlite");
        let opened = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let root_key = opened
            .database
            .frontier_root()
            .unwrap()
            .document_map_root_key()
            .unwrap();
        drop(opened);

        let connection = Connection::open(&path).unwrap();
        let (document_bytes, dependencies): (Vec<u8>, Vec<u8>) = connection
            .query_row(
                "SELECT document_id, dependencies
                 FROM frontier_documents
                 WHERE document_id != ?1
                 ORDER BY document_id LIMIT 1",
                [root_key.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let document_id = decode_document_id(&document_bytes).unwrap();
        let original = decode_frontier_document(document_id, &dependencies).unwrap();
        let tampered = DocumentDependencies::new(
            document_id,
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(99_999), 1)],
            original.direct_dependency_heads().to_vec(),
        )
        .unwrap();
        let tampered_bytes = encode_frontier_document(&tampered).unwrap();
        connection
            .execute(
                "UPDATE frontier_documents
                 SET dependencies = ?1, dependencies_digest = ?2
                 WHERE document_id = ?3",
                params![
                    &tampered_bytes,
                    ContentDigest::of(&tampered_bytes).as_bytes().as_slice(),
                    document_bytes,
                ],
            )
            .unwrap();
        drop(connection);

        let recovered = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let ProjectionRecovery::RebuiltPreservingEvidence { reason, .. } = &recovered.recovery
        else {
            panic!("rehashed frontier-row tampering was not quarantined");
        };
        assert!(reason.contains("checkpoint") || reason.contains("authenticated"));
        assert_eq!(
            recovered.database.frontier().unwrap(),
            engine.exact_frontier().unwrap()
        );
    }

    #[test]
    fn apply_sequence_comes_from_authenticated_root_not_lifetime_row_count() {
        let base = TestIds::new(2_380);
        let right = TestIds {
            workspace: base.workspace,
            lineage: base.lineage,
            catalog: base.catalog,
            document: DocumentId::from_uuid(uuid(2_483)),
            page: PageId::from_uuid(uuid(2_484)),
            block: BlockId::from_uuid(uuid(2_485)),
        };
        let dir = TestDir::new("root-derived-apply-sequence");
        let store = ObjectStore::open(&dir.path().join("objects"), base.workspace).unwrap();
        let left_batch = base
            .engine()
            .prepare_bootstrap_transaction(
                author(2_490),
                &root_transaction(base, "pages/root-sequence-left.md", "left"),
            )
            .unwrap();
        let right_batch = right
            .engine()
            .prepare_bootstrap_transaction(
                author(2_491),
                &root_transaction(right, "pages/root-sequence-right.md", "right"),
            )
            .unwrap();
        store.publish_prepared(&left_batch).unwrap();
        store.publish_prepared(&right_batch).unwrap();
        let mut engine = base.engine();
        assert!(matches!(
            engine
                .stage_from_store(&store, left_batch.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            engine
                .stage_from_store(&store, right_batch.manifest().batch_id())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let left =
            AcceptedBatchEvent::from_accepted(&engine, &store, left_batch.manifest().batch_id())
                .unwrap();
        let right =
            AcceptedBatchEvent::from_accepted(&engine, &store, right_batch.manifest().batch_id())
                .unwrap();
        let empty = base.engine();
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            base.claim(),
            RebuildSource::new(&empty, &store).unwrap(),
        )
        .unwrap()
        .database;
        database.apply_accepted(&left).unwrap();
        database
            .connection
            .execute("DELETE FROM applied_batches WHERE sequence = 1", [])
            .unwrap();
        assert_eq!(
            database.apply_accepted(&right).unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(database.frontier_root().unwrap().acceptance_sequence(), 2);
    }

    #[test]
    fn restart_tail_accounts_for_durable_unapplied_bytes_before_reservation() {
        let ids = TestIds::new(2_390);
        let dir = TestDir::new("restart-tail-backlog");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(2_391),
                &root_transaction(ids, "pages/restart-tail.md", "pending"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let source = RebuildSource::new(&engine, &store).unwrap();
        let mut overlay = TailOverlay::from_durable(&database, &source).unwrap();
        let status = overlay.status();
        assert_eq!(status.unapplied_batches, 1);
        assert!(status.retained_bytes > 0);
        assert!(matches!(
            overlay.reserve_mutation(TAIL_MAX_BYTES),
            Err(TailOverlayError::Backpressure(_))
        ));
        assert_eq!(overlay.drain_ready(&mut database, &source, 1).unwrap(), 1);
        assert_eq!(overlay.status().retained_bytes, 0);
    }

    #[test]
    fn projection_sidecar_fingerprint_reads_only_bounded_edge_chunks() {
        let dir = TestDir::new("bounded-sidecar-fingerprint");
        let path = dir.path().join("large-wal");
        let length = 64_u64 * 1024 * 1024;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(length).unwrap();
        file.seek(SeekFrom::Start(length - 1)).unwrap();
        file.write_all(&[1]).unwrap();
        drop(file);
        let checkpoint = bounded_file_checkpoint(&path).unwrap();
        assert_eq!(checkpoint.length, length);
        assert_eq!(
            bounded_file_checkpoint_sample_bytes(length),
            (PROJECTION_FINGERPRINT_CHUNK_BYTES * 2) as u64
        );
    }

    #[derive(Clone, Copy, Debug)]
    struct CausalStorageStats {
        clock_nodes: usize,
        clock_node_bytes: usize,
        batch_nodes: usize,
        batch_node_bytes: usize,
        database_bytes: usize,
        ancestry_rows_read: usize,
    }

    fn measured_streaming_rebuild(
        batch_count: usize,
        seed: u128,
        fresh_peers: bool,
    ) -> (
        RebuildInstrumentation,
        Duration,
        Duration,
        CausalStorageStats,
    ) {
        let ids = TestIds::new(seed);
        let dir = TestDir::new(&format!("streaming-rebuild-{batch_count}"));
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let root = engine
            .prepare_bootstrap_transaction(
                if fresh_peers {
                    fresh_peer_author(seed, 0)
                } else {
                    constant_peer_author(seed, 0)
                },
                &root_transaction(ids, "pages/linear.md", "0"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &root);
        for index in 1..batch_count {
            let edit = engine
                .prepare_bootstrap_transaction(
                    if fresh_peers {
                        fresh_peer_author(seed, index)
                    } else {
                        constant_peer_author(seed, index)
                    },
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
        let point_before = engine.instrumentation();
        assert!(engine.accepted_batch_id_at(1).unwrap().is_some());
        assert!(engine
            .accepted_batch_id_at(batch_count as u64)
            .unwrap()
            .is_some());
        let point_after = engine.instrumentation();
        let point_page_reads = point_after
            .scratch_page_reads
            .saturating_sub(point_before.scratch_page_reads);
        let point_page_bytes = point_after
            .scratch_page_bytes_read
            .saturating_sub(point_before.scratch_page_bytes_read);
        assert!(point_page_reads <= 8);
        assert!(point_page_bytes <= point_page_reads.saturating_mul(64 * 1024));
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
        let first = load_batch_at_sequence(&opened.database.connection, 1)
            .unwrap()
            .unwrap()
            .batch_id;
        let last = load_batch_at_sequence(&opened.database.connection, batch_count as i64)
            .unwrap()
            .unwrap()
            .batch_id;
        let (descends, batch_rows_read) =
            batch_descends_from_database_measured(&opened.database.connection, last, first)
                .unwrap();
        assert!(descends);
        assert!(
            batch_rows_read <= 96,
            "authenticated ancestry point lookup read {batch_rows_read} rows"
        );
        let (clock_nodes, clock_node_bytes): (i64, i64) = opened
            .database
            .connection
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(length(node_digest) + length(peer_id) + 8
                            + length(value_digest)
                            + COALESCE(length(left_peer_id), 0)
                            + COALESCE(length(left_digest), 0)
                            + COALESCE(length(right_peer_id), 0)
                            + COALESCE(length(right_digest), 0)), 0)
                 FROM causal_clock_nodes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (batch_nodes, batch_node_bytes): (i64, i64) = opened
            .database
            .connection
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(length(node_digest) + length(batch_id)
                            + length(value_digest)
                            + COALESCE(length(left_batch_id), 0)
                            + COALESCE(length(left_digest), 0)
                            + COALESCE(length(right_batch_id), 0)
                            + COALESCE(length(right_digest), 0)), 0)
                 FROM accepted_batch_nodes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let page_count: i64 = opened
            .database
            .connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let page_size: i64 = opened
            .database
            .connection
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        let causal_stats = CausalStorageStats {
            clock_nodes: usize::try_from(clock_nodes).unwrap(),
            clock_node_bytes: usize::try_from(clock_node_bytes).unwrap(),
            batch_nodes: usize::try_from(batch_nodes).unwrap(),
            batch_node_bytes: usize::try_from(batch_node_bytes).unwrap(),
            database_bytes: usize::try_from(page_count * page_size).unwrap(),
            ancestry_rows_read: batch_rows_read,
        };
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
        (rebuild, rebuild_elapsed, startup_elapsed, causal_stats)
    }

    #[test]
    fn rebuild_streams_linearly_with_one_live_event_and_evidence_record() {
        let (small, small_elapsed, small_startup, _) = measured_streaming_rebuild(24, 2_500, false);
        let (large, large_elapsed, large_startup, _) = measured_streaming_rebuild(48, 2_700, false);
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
        assert!(small.accepted_sequence_page_reads <= 24 + 4);
        assert!(large.accepted_sequence_page_reads <= 48 + 5);
        assert!(small.max_accepted_sequence_page_bytes < 64 * 1024);
        assert!(large.max_accepted_sequence_page_bytes < 64 * 1024);
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
    #[ignore = "explicit constant-peer SQLite rebuild scaling sweep"]
    fn sqlite_streaming_rebuild_constant_peer_scaling_sweep() {
        for (index, batch_count) in [100_usize, 200, 400, 800].into_iter().enumerate() {
            let (work, rebuild_elapsed, startup_elapsed, _) =
                measured_streaming_rebuild(batch_count, 2_800 + index as u128 * 10_000, false);
            let leaf_pages = batch_count;
            assert_eq!(work.accepted_events_validated, batch_count);
            assert_eq!(work.accepted_events_applied, batch_count);
            assert_eq!(work.max_live_events, 1);
            assert_eq!(work.max_live_evidence_records, 1);
            assert_eq!(work.ancestry_full_scans, 0);
            assert!(
                work.accepted_sequence_page_reads
                    <= leaf_pages
                        .saturating_add(batch_count.div_ceil(31))
                        .saturating_add(4),
                "{} events read {} accepted-sequence pages for {} leaves",
                batch_count,
                work.accepted_sequence_page_reads,
                leaf_pages
            );
            assert!(work.max_accepted_sequence_page_bytes < 64 * 1024);
            assert!(startup_elapsed < Duration::from_secs(2));
            eprintln!(
                "sqlite_constant_peer_sweep batches={} rebuild_ms={} startup_ms={} sequence_pages={} sequence_bytes={} max_sequence_page={} max_live_events={} max_live_evidence={}",
                batch_count,
                rebuild_elapsed.as_millis(),
                startup_elapsed.as_millis(),
                work.accepted_sequence_page_reads,
                work.accepted_sequence_bytes_read,
                work.max_accepted_sequence_page_bytes,
                work.max_live_events,
                work.max_live_evidence_records,
            );
        }
    }

    #[test]
    fn fresh_peer_clocks_use_structural_sharing_instead_of_full_vectors() {
        let (_, _, _, small) = measured_streaming_rebuild(24, 2_750, true);
        let (_, _, _, large) = measured_streaming_rebuild(48, 2_775, true);
        assert!(small.clock_nodes < 24 * 32);
        assert!(large.clock_nodes < 48 * 32);
        assert!(
            large.clock_nodes < small.clock_nodes.saturating_mul(3),
            "doubling fresh-peer history grew clock nodes from {} to {}",
            small.clock_nodes,
            large.clock_nodes
        );
        assert!(large.ancestry_rows_read <= 96);
    }

    fn repeated_fresh_peer_fork_merge_work(batch_count: usize, seed: u128) -> ClockUnionStats {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(CAUSAL_CLOCK_NODES_DDL).unwrap();
        let mut root = None;
        let mut stats = ClockUnionStats::default();
        for index in 0..batch_count {
            let left_peer =
                CausalPeerId::from_device_id(DeviceId::from_uuid(uuid(seed + index as u128 * 2)));
            let right_peer = CausalPeerId::from_device_id(DeviceId::from_uuid(uuid(
                seed + index as u128 * 2 + 1,
            )));
            let left = Some(upsert_causal_clock(&connection, root.clone(), left_peer, 1).unwrap());
            let right =
                Some(upsert_causal_clock(&connection, root.clone(), right_peer, 1).unwrap());
            root = merge_causal_clock_roots_measured(&connection, left, right, &mut stats).unwrap();
        }
        stats
    }

    #[test]
    fn repeated_fresh_peer_fork_merge_union_is_near_linear() {
        let work = [100_usize, 200, 400].map(|batch_count| {
            repeated_fresh_peer_fork_merge_work(batch_count, 90_000 + batch_count as u128 * 10_000)
        });
        eprintln!(
            "clock_union_fresh_peer_sweep batches=100 reads={} writes={} shared={}; batches=200 reads={} writes={} shared={}; batches=400 reads={} writes={} shared={}",
            work[0].nodes_read,
            work[0].nodes_written,
            work[0].shared_subtrees,
            work[1].nodes_read,
            work[1].nodes_written,
            work[1].shared_subtrees,
            work[2].nodes_read,
            work[2].nodes_written,
            work[2].shared_subtrees,
        );
        assert!(
            work[1].nodes_read <= work[0].nodes_read.saturating_mul(3),
            "doubling 100 -> 200 grew clock-union reads from {} to {}",
            work[0].nodes_read,
            work[1].nodes_read
        );
        assert!(
            work[2].nodes_read <= work[1].nodes_read.saturating_mul(3),
            "doubling 200 -> 400 grew clock-union reads from {} to {}",
            work[1].nodes_read,
            work[2].nodes_read
        );
        assert!(
            work[2].nodes_read <= 400 * 256,
            "400 repeated fork/merges read {} authenticated clock nodes",
            work[2].nodes_read
        );
        assert!(work.iter().all(|stats| stats.shared_subtrees > 0));
    }

    #[test]
    fn causal_clock_union_is_canonical_exact_and_order_independent() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(CAUSAL_CLOCK_NODES_DDL).unwrap();
        let peers = (0..4_u128)
            .map(|index| CausalPeerId::from_device_id(DeviceId::from_uuid(uuid(500_000 + index))))
            .collect::<Vec<_>>();
        let mut left = None;
        for (peer, counter) in [(peers[0], 2), (peers[1], 7), (peers[3], 1)] {
            left = Some(upsert_causal_clock(&connection, left, peer, counter).unwrap());
        }
        let mut right = None;
        for (peer, counter) in [(peers[0], 5), (peers[1], 3), (peers[2], 11)] {
            right = Some(upsert_causal_clock(&connection, right, peer, counter).unwrap());
        }

        let left_then_right =
            merge_causal_clock_roots(&connection, left.clone(), right.clone()).unwrap();
        let right_then_left =
            merge_causal_clock_roots(&connection, right.clone(), left.clone()).unwrap();
        assert_eq!(left_then_right, right_then_left);
        for (peer, expected) in [(peers[0], 5), (peers[1], 7), (peers[2], 11), (peers[3], 1)] {
            assert_eq!(
                causal_clock_lookup(&connection, left_then_right.clone(), peer, &mut 0).unwrap(),
                Some(expected)
            );
        }
        assert_eq!(
            merge_causal_clock_roots(&connection, None, left_then_right.clone()).unwrap(),
            left_then_right
        );
        let mut duplicate_stats = ClockUnionStats::default();
        assert_eq!(
            merge_causal_clock_roots_measured(
                &connection,
                left_then_right.clone(),
                left_then_right.clone(),
                &mut duplicate_stats,
            )
            .unwrap(),
            left_then_right
        );
        assert_eq!(duplicate_stats.nodes_read, 0);
        assert_eq!(duplicate_stats.nodes_written, 0);
        assert_eq!(duplicate_stats.shared_subtrees, 1);
    }

    #[test]
    #[ignore = "explicit fresh-peer authenticated SQLite scaling sweep"]
    fn sqlite_streaming_rebuild_fresh_peer_scaling_sweep() {
        for (index, batch_count) in [100_usize, 200, 400].into_iter().enumerate() {
            let (work, rebuild_elapsed, startup_elapsed, storage) =
                measured_streaming_rebuild(batch_count, 40_000 + index as u128 * 100_000, true);
            let logarithmic_path_bound =
                usize::try_from(usize::BITS - batch_count.leading_zeros()).unwrap() * 8 + 8;
            assert!(storage.clock_nodes <= batch_count * logarithmic_path_bound);
            assert!(storage.batch_nodes <= batch_count * logarithmic_path_bound);
            assert!(storage.clock_node_bytes <= storage.clock_nodes * 256);
            assert!(storage.batch_node_bytes <= storage.batch_nodes * 256);
            assert!(storage.database_bytes <= batch_count * 192 * 1024);
            assert!(storage.ancestry_rows_read <= 96);
            assert!(startup_elapsed < Duration::from_secs(2));
            eprintln!(
                "sqlite_fresh_peer_sweep batches={} rebuild_ms={} startup_ms={} clock_nodes={} clock_bytes={} batch_nodes={} batch_bytes={} database_bytes={} ancestry_rows={} sequence_pages={} sequence_bytes={}",
                batch_count,
                rebuild_elapsed.as_millis(),
                startup_elapsed.as_millis(),
                storage.clock_nodes,
                storage.clock_node_bytes,
                storage.batch_nodes,
                storage.batch_node_bytes,
                storage.database_bytes,
                storage.ancestry_rows_read,
                work.accepted_sequence_page_reads,
                work.accepted_sequence_bytes_read,
            );
        }
    }

    #[test]
    #[ignore = "explicit authenticated SQLite cold-rebuild performance gate"]
    fn sqlite_streaming_rebuild_cold_gate() {
        let (work, rebuild_elapsed, startup_elapsed, _) =
            measured_streaming_rebuild(1_000, 2_900, false);
        assert_eq!(work.accepted_events_validated, 1_000);
        assert_eq!(work.accepted_events_applied, 1_000);
        assert_eq!(work.max_live_events, 1);
        assert_eq!(work.max_live_evidence_records, 1);
        assert_eq!(work.ancestry_full_scans, 0);
        assert!(work.accepted_sequence_page_reads <= 1_040);
        assert!(work.max_accepted_sequence_page_bytes < 64 * 1024);
        assert!(
            rebuild_elapsed <= Duration::from_secs(45),
            "authenticated SQLite rebuild took {rebuild_elapsed:?}"
        );
        assert!(
            startup_elapsed <= Duration::from_secs(2),
            "normal SQLite startup took {startup_elapsed:?}"
        );
        eprintln!(
            "sqlite_streaming_rebuild_gate batches=1000 rebuild_ms={} startup_ms={} validated={} sequence_pages={} sequence_bytes={} max_sequence_page={} max_live_events={} max_live_evidence={}",
            rebuild_elapsed.as_millis(),
            startup_elapsed.as_millis(),
            work.accepted_events_validated,
            work.accepted_sequence_page_reads,
            work.accepted_sequence_bytes_read,
            work.max_accepted_sequence_page_bytes,
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
            .prepare_bootstrap_transaction(
                author(2_500),
                &root_transaction(base, "pages/left.md", "left"),
            )
            .unwrap();
        let right_batch = right
            .engine()
            .prepare_bootstrap_transaction(
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
            .prepare_bootstrap_transaction(
                shared_author,
                &root_transaction(ids, "pages/same.md", "GOOD"),
            )
            .unwrap();
        let evil = ids
            .engine()
            .prepare_bootstrap_transaction(
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
        database.diagnose_full_integrity().unwrap();
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
        let sibling_binding = super::super::AcceptedBatchEvidence::binding_digest_for(
            batch(102),
            ContentDigest::of(&sibling.manifest().encode().unwrap()),
            sibling.manifest().semantic_effect_digest(),
            sibling.manifest().dependency_frontier(),
            sibling.manifest().causal_dependency_heads(),
        )
        .unwrap();
        let sibling_entry = test_causal_record_entry(
            &sibling,
            sibling_binding,
            vec![
                (root.causal_dot().peer_id(), 1),
                (sibling.manifest().causal_dot().peer_id(), 1),
            ],
        );
        let mut batch_entries = vec![
            (
                root.batch_id(),
                load_batch(&database.connection, root.batch_id())
                    .unwrap()
                    .unwrap()
                    .causal_record_digest()
                    .unwrap(),
            ),
            (
                child.batch_id(),
                load_batch(&database.connection, child.batch_id())
                    .unwrap()
                    .unwrap()
                    .causal_record_digest()
                    .unwrap(),
            ),
            sibling_entry,
        ];
        batch_entries.sort_unstable_by_key(|(batch_id, _)| *batch_id);
        let sibling_evidence = super::super::AcceptedBatchEvidence::for_test(
            batch(102),
            ContentDigest::of(&sibling.manifest().encode().unwrap()),
            sibling_binding,
            child.post_frontier_root.clone(),
            vec![sibling_document.clone()],
            vec![sibling_document],
            batch_entries,
            validated_retained_bytes(&sibling),
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
            .prepare_bootstrap_transaction(
                author(4_010),
                &root_transaction(ids, "pages/overlay.md", "root"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &root_prepared);
        let root =
            AcceptedBatchEvent::from_accepted(&engine, &store, root_prepared.manifest().batch_id())
                .unwrap();
        let child_prepared = engine
            .prepare_bootstrap_transaction(
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
        let mut overlay = TailOverlay::empty_for_test();
        assert!(overlay.try_enqueue(&mut database, &child).unwrap());
        assert_eq!(
            overlay.status().retained_bytes,
            usize::try_from(child.post_frontier_root().retained_bytes_total()).unwrap()
        );
        assert!(overlay.try_enqueue(&mut database, &root).unwrap());
        assert_eq!(
            overlay.status().retained_bytes,
            usize::try_from(child.post_frontier_root().retained_bytes_total()).unwrap()
        );
        assert_eq!(overlay.drain_ready(&mut database, &source, 1).unwrap(), 1);
        assert_eq!(
            overlay.status().retained_bytes,
            usize::try_from(
                child
                    .post_frontier_root()
                    .retained_bytes_total()
                    .saturating_sub(root.post_frontier_root().retained_bytes_total())
            )
            .unwrap()
        );
        assert_eq!(
            overlay
                .drain_ready(&mut database, &source, usize::MAX)
                .unwrap(),
            1
        );
        assert_eq!(
            database.frontier().unwrap(),
            engine.exact_frontier().unwrap()
        );
        assert_eq!(overlay.status().unapplied_batches, 0);
        assert!(!overlay.try_enqueue(&mut database, &root).unwrap());
        assert_eq!(overlay.status().unapplied_batches, 0);

        let mut count_limited = TailOverlay::empty_for_test();
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

        let mut byte_limited = TailOverlay::empty_for_test();
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
        let mut overlay = TailOverlay::empty_for_test();
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
        let mut overlay = TailOverlay::empty_for_test();
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
            kind: ManagedTextKind::Page,
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
            .prepare_bootstrap_transaction(
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
    fn injected_runtime_roots_cannot_split_the_object_store_lease() {
        let seed = 7_400;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("canonical-runtime-lease");
        fs::create_dir_all(dir.path().join("db-a")).unwrap();
        fs::create_dir_all(dir.path().join("db-b")).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine = ids.engine();
        let would_be_a = dir.path().join("db-a/runtime");
        let would_be_b = dir.path().join("db-b/runtime");
        assert_ne!(would_be_a, would_be_b);

        let injected_a = ApplicationRuntimeRoot::open_for_test(&would_be_a).unwrap();
        let first = SqliteFrontier::open_or_rebuild(
            &dir.path().join("db-a/fail-before.sqlite"),
            &injected_a,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let mut contender = spawn_test_helper(
            "injected-runtime-contender",
            dir.path(),
            seed,
            &[(
                "TINE_SQLITE_HELPER_WOULD_BE_RUNTIME",
                would_be_b.to_str().unwrap(),
            )],
        );
        assert!(contender.wait().unwrap().success());
        drop(first);
        let runtime = ApplicationRuntimeRoot::open_for_test(&would_be_b).unwrap();
        let recovered = SqliteFrontier::open_or_rebuild(
            &dir.path().join("db-b/fail-before.sqlite"),
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
    fn production_lease_is_shared_across_distinct_xdg_and_home_roots() {
        let seed = 7_600;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("production-resource-lease");
        fs::create_dir_all(dir.path().join("db-a")).unwrap();
        fs::create_dir_all(dir.path().join("db-b")).unwrap();
        let _store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let xdg_a = dir.path().join("profile-a/xdg");
        let home_a = dir.path().join("profile-a/home");
        let xdg_b = dir.path().join("profile-b/xdg");
        let home_b = dir.path().join("profile-b/home");
        for path in [&xdg_a, &home_a, &xdg_b, &home_b] {
            fs::create_dir_all(path).unwrap();
        }
        let mut holder = spawn_test_helper(
            "production-lease-holder",
            dir.path(),
            seed,
            &[
                ("XDG_DATA_HOME", xdg_a.to_str().unwrap()),
                ("HOME", home_a.to_str().unwrap()),
            ],
        );
        wait_for_file(&dir.path().join("helper-ready"));
        let mut contender = spawn_test_helper(
            "production-lease-contender",
            dir.path(),
            seed,
            &[
                ("XDG_DATA_HOME", xdg_b.to_str().unwrap()),
                ("HOME", home_b.to_str().unwrap()),
            ],
        );
        let contender_succeeded = contender.wait().unwrap().success();
        holder.kill().unwrap();
        assert!(!holder.wait().unwrap().success());
        assert!(contender_succeeded);
    }

    #[cfg(unix)]
    #[test]
    fn object_store_lease_rejects_symlinked_namespaces_workspace_and_file() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        fn rejected(case: &str, prepare: impl FnOnce(&Path, WorkspaceId)) {
            let ids = TestIds::new(7_700 + case.len() as u128 * 100);
            let dir = TestDir::new(case);
            let store_path = dir.path().join("objects");
            let store = ObjectStore::open(&store_path, ids.workspace).unwrap();
            prepare(&store_path, ids.workspace);
            let runtime =
                ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
            let engine = ids.engine();
            assert!(matches!(
                SqliteFrontier::open_or_rebuild(
                    &dir.path().join("frontier.sqlite"),
                    &runtime,
                    ids.claim(),
                    RebuildSource::new(&engine, &store).unwrap(),
                ),
                Err(ProjectionError::UnsafePath(_))
            ));
        }

        rejected("lease-object-store-namespace-symlink", |store, _| {
            fs::create_dir(store.join("redirect")).unwrap();
            symlink(
                store.join("redirect"),
                store.join(OBJECT_STORE_LEASE_NAMESPACE),
            )
            .unwrap();
        });
        rejected("lease-sqlite-namespace-symlink", |store, _| {
            fs::create_dir(store.join(OBJECT_STORE_LEASE_NAMESPACE)).unwrap();
            fs::create_dir(store.join("redirect")).unwrap();
            symlink(
                store.join("redirect"),
                store
                    .join(OBJECT_STORE_LEASE_NAMESPACE)
                    .join(SQLITE_WORKSPACE_LEASE_NAMESPACE),
            )
            .unwrap();
        });
        rejected("lease-workspace-symlink", |store, workspace| {
            let namespace = store
                .join(OBJECT_STORE_LEASE_NAMESPACE)
                .join(SQLITE_WORKSPACE_LEASE_NAMESPACE);
            fs::create_dir_all(&namespace).unwrap();
            fs::create_dir(store.join("redirect")).unwrap();
            symlink(
                store.join("redirect"),
                namespace.join(workspace.to_string()),
            )
            .unwrap();
        });
        rejected("lease-file-symlink", |store, workspace| {
            let workspace = store
                .join(OBJECT_STORE_LEASE_NAMESPACE)
                .join(SQLITE_WORKSPACE_LEASE_NAMESPACE)
                .join(workspace.to_string());
            fs::create_dir_all(&workspace).unwrap();
            fs::write(store.join("redirect"), b"not a lease").unwrap();
            symlink(
                store.join("redirect"),
                workspace.join(SQLITE_APPLIER_LEASE_FILE),
            )
            .unwrap();
        });
        rejected("lease-group-writable-namespace", |store, _| {
            let namespace = store.join(OBJECT_STORE_LEASE_NAMESPACE);
            fs::create_dir(&namespace).unwrap();
            fs::set_permissions(&namespace, fs::Permissions::from_mode(0o770)).unwrap();
        });
    }

    #[test]
    fn object_store_lease_creation_race_has_one_process_winner() {
        let seed = 7_900;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("object-store-lease-race");
        fs::create_dir_all(dir.path().join("db-a")).unwrap();
        fs::create_dir_all(dir.path().join("db-b")).unwrap();
        let _store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let xdg_a = dir.path().join("profile-a");
        let xdg_b = dir.path().join("profile-b");
        fs::create_dir_all(&xdg_a).unwrap();
        fs::create_dir_all(&xdg_b).unwrap();
        let mut a = spawn_test_helper(
            "production-lease-racer",
            dir.path(),
            seed,
            &[
                ("TINE_SQLITE_RACER_LABEL", "a"),
                ("XDG_DATA_HOME", xdg_a.to_str().unwrap()),
            ],
        );
        let mut b = spawn_test_helper(
            "production-lease-racer",
            dir.path(),
            seed,
            &[
                ("TINE_SQLITE_RACER_LABEL", "b"),
                ("XDG_DATA_HOME", xdg_b.to_str().unwrap()),
            ],
        );
        wait_for_file(&dir.path().join("race-ready-a"));
        wait_for_file(&dir.path().join("race-ready-b"));
        fs::write(dir.path().join("race-go"), b"go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let results = ["a", "b"]
                .into_iter()
                .filter(|label| {
                    dir.path().join(format!("race-acquired-{label}")).exists()
                        || dir.path().join(format!("race-contended-{label}")).exists()
                })
                .count();
            if results == 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for lease racers"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let winners = ["a", "b"]
            .into_iter()
            .filter(|label| dir.path().join(format!("race-acquired-{label}")).exists())
            .count();
        let contenders = ["a", "b"]
            .into_iter()
            .filter(|label| dir.path().join(format!("race-contended-{label}")).exists())
            .count();
        assert_eq!((winners, contenders), (1, 1));
        fs::write(dir.path().join("race-stop"), b"stop").unwrap();
        assert!(a.wait().unwrap().success());
        assert!(b.wait().unwrap().success());
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
                kind: ManagedTextKind::Page,
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
            .prepare_bootstrap_transaction(author(8_100), &transaction)
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
            .prepare_bootstrap_transaction(
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
    fn kind_only_effect_survives_sqlite_reopen_and_rebuild() {
        let ids = TestIds::new(8_500);
        let dir = TestDir::new("kind-only-rebuild");
        let store_path = dir.path().join("objects");
        let store = ObjectStore::open(&store_path, ids.workspace).unwrap();
        let create = ids
            .engine()
            .prepare_bootstrap_transaction(
                author(8_600),
                &OperationTransaction::new(vec![SemanticOperation::CreatePage {
                    page_id: ids.page,
                    home_document_id: ids.document,
                    path: ManagedPath::parse("shared/SQLite.md").unwrap(),
                    kind: ManagedTextKind::Page,
                }])
                .unwrap(),
            )
            .unwrap();
        store.publish_prepared(&create).unwrap();
        let reader = ObjectStore::open(&store_path, ids.workspace).unwrap();
        let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
        assert!(matches!(
            engine
                .stage_archive_batch(create.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));

        let change = engine
            .prepare_bootstrap_transaction(
                author(8_601),
                &OperationTransaction::new(vec![SemanticOperation::SetPageKind {
                    page_id: ids.page,
                    kind: ManagedTextKind::Journal,
                }])
                .unwrap(),
            )
            .unwrap();
        store.publish_prepared(&change).unwrap();
        assert!(matches!(
            engine
                .stage_archive_batch(change.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));
        let change_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, change.manifest().batch_id())
                .unwrap();
        let change_effect = SemanticEffect::decode(change_event.semantic_effect()).unwrap();
        assert_eq!(change_effect.pages().len(), 1);
        assert_eq!(
            change_effect.pages()[0].before.as_ref().unwrap().kind(),
            ManagedTextKind::Page
        );
        assert_eq!(
            change_effect.pages()[0].after.as_ref().unwrap().kind(),
            ManagedTextKind::Journal
        );
        assert_eq!(
            change_effect.pages()[0].before.as_ref().unwrap().path(),
            change_effect.pages()[0].after.as_ref().unwrap().path()
        );

        let database_path = dir.path().join("frontier.sqlite");
        let first = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            first.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 2 }
        );
        assert_eq!(first.database.applied_batch_count().unwrap(), 2);
        assert_eq!(
            stored_semantic_effects(&first.database)[1].pages()[0]
                .after
                .as_ref()
                .unwrap()
                .kind(),
            ManagedTextKind::Journal
        );
        let expected_digest = first.database.semantic_projection_digest().unwrap();
        drop(first);

        let reopened = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
        assert_eq!(
            stored_semantic_effects(&reopened.database)[1].pages()[0]
                .after
                .as_ref()
                .unwrap()
                .kind(),
            ManagedTextKind::Journal
        );
        assert_eq!(
            reopened.database.semantic_projection_digest().unwrap(),
            expected_digest
        );
        drop(reopened);

        remove_projection_files(&database_path);
        let rebuilt = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            rebuilt.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 2 }
        );
        assert_eq!(
            stored_semantic_effects(&rebuilt.database)[1].pages()[0]
                .after
                .as_ref()
                .unwrap()
                .kind(),
            ManagedTextKind::Journal
        );
        assert_eq!(
            rebuilt.database.semantic_projection_digest().unwrap(),
            expected_digest
        );

        let mut replay = ids.engine();
        for manifest in store.committed_manifests().unwrap() {
            assert!(matches!(
                replay
                    .stage_from_store(&store, manifest.batch_id())
                    .unwrap()
                    .disposition(),
                BatchDisposition::Accepted { .. }
            ));
        }
        assert_eq!(
            replay.canonical_snapshot().unwrap().pages[0].1.kind(),
            ManagedTextKind::Journal
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
            assert_eq!(evidence.len(), 4);
            assert!(evidence.iter().all(|item| item.preserved_path.exists()));
            assert_eq!(reopened.database.applied_batch_count().unwrap(), 1);
        }

        let seed = 10_200;
        let dir = TestDir::new("rebuild-crash");
        let (ids, store, mut accepted_engine, path) = prepare_crash_case(&dir, seed);
        let child = accepted_engine
            .prepare_bootstrap_transaction(
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
