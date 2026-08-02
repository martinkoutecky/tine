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
//! macOS, Windows, and Android. The lock file is created empty and is never
//! written, so it never decides ownership by its contents. Ownership is decided
//! by the OS lock on one exact *stable file identity*, which the lease captures
//! at acquisition and revalidates at every authority-bearing boundary: a lock
//! file replaced out of band inside the replicated archive would otherwise let
//! the old holder and a new opener both believe they own the workspace.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
#[cfg(test)]
use std::io::{Seek as _, SeekFrom};
#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OsMetadataExt as CapOsMetadataExt};
#[cfg(unix)]
use cap_std::fs::MetadataExt as CapMetadataExt;
#[cfg(windows)]
use cap_std::fs::OpenOptions as CapOpenOptions;
use cap_std::{ambient_authority, fs::Dir as CapDir};
use fs2::FileExt as _;
#[cfg(test)]
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tine_storage::sqlite::{
    self as storage_frontier, PhysicalFileCheckpoint, PhysicalSqliteDatabase, SqliteFileSet,
    SqliteFileSetError,
};
use uuid::Uuid;

use super::hot_engine::{AcceptedFrontierRoot, EngineAuthority};
use super::import::{
    InactiveBootstrapAcceptedAuthority, InactiveBootstrapAcceptedAuthorityBinding,
    TerminalBootstrapConstructionMaterial,
};
use super::object_store::ValidatedBootstrapPublicationV1;
use super::{
    BatchCausalDot, BatchId, BatchInspection, BlockId, CausalPeerId, ContentDigest,
    DocumentDependencies, DocumentId, FrontierV2, LineageDigest, LogseqUuid, LogseqUuidResolution,
    ObjectKind, ObjectStore, PageId, PreparedBatch, ReferenceFactV1, ReferenceSourceLocatorV1,
    SemanticEffect, SemanticEffectDigest, ShardedHotEngine, ValidatedBatch, WorkspaceId,
    WorkspaceStatus, MANAGED_ENTITY_SET_VERSION, MANIFEST_ENCODING_VERSION,
    OBJECT_ENVELOPE_SCHEMA_VERSION, OPERATION_SCHEMA_VERSION, OPLOG_PROTOCOL_VERSION,
};

pub const SQLITE_APPLICATION_ID: u32 = storage_frontier::SQLITE_APPLICATION_ID;
pub const SQLITE_SCHEMA_VERSION: u32 = storage_frontier::SQLITE_SCHEMA_VERSION;
pub const TAIL_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const TAIL_MAX_BATCHES: usize = 10_000;

const PROJECTION_CHECKPOINT_SCHEMA_VERSION: u32 = 2;
/// Bounded current-path catalog page size for terminal construction. The rows
/// are drained into materialization chunks, so this only caps how many
/// authenticated catalog rows the builder owns at once.
pub(crate) const TERMINAL_CATALOG_CURSOR_PAGE_ROWS: usize = 128;
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
    effective_semantic_effect: Vec<u8>,
    effective_transitions: Vec<super::hot_engine::AuthenticatedPageLocalEffectiveTransition>,
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
        Self::from_validated(&validated, &evidence)?.with_effective_view(engine)
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
        Self::from_validated(&validated, evidence)?.with_effective_view(engine)
    }

    /// Retain the accepted event of one bootstrap part directly from the
    /// prepared bytes and accepted evidence a detached authoring pass just
    /// produced.
    ///
    /// This is deliberately the exact constructor the archive replay path takes
    /// after loading the same part back out of the immutable publication, so a
    /// retained value and a replayed value are the same typed event. It grants
    /// no acceptance by itself: `authenticate_event_for_engine` still has to
    /// bind it to the engine's authenticated history before it may be applied.
    pub(crate) fn from_authored_bootstrap_part(
        batch: &ValidatedBatch,
        evidence: &super::AcceptedBatchEvidence,
    ) -> Result<Self, ProjectionError> {
        Self::from_validated(batch, evidence)
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
            effective_semantic_effect: decoded.encode().map_err(|error| {
                ProjectionError::InvalidAcceptedEvent(format!(
                    "cannot encode initial effective semantic view for {}: {error}",
                    manifest.batch_id()
                ))
            })?,
            effective_transitions: Vec::new(),
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

    fn with_effective_view(mut self, engine: &ShardedHotEngine) -> Result<Self, ProjectionError> {
        let authored = SemanticEffect::decode(&self.semantic_effect).map_err(|error| {
            ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} authored effect cannot be decoded: {error}",
                self.batch_id
            ))
        })?;
        let view = engine
            .accepted_effective_semantic_view(self.batch_id, &authored)
            .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
        self.effective_semantic_effect = view.effect().encode().map_err(|error| {
            ProjectionError::InvalidAcceptedEvent(format!(
                "accepted batch {} effective effect cannot be encoded: {error}",
                self.batch_id
            ))
        })?;
        self.effective_transitions = view.transitions().to_vec();
        Ok(self)
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub const fn manifest_digest(&self) -> ContentDigest {
        self.manifest_digest
    }

    pub const fn event_binding_digest(&self) -> ContentDigest {
        self.event_binding_digest
    }

    pub fn semantic_effect(&self) -> &[u8] {
        &self.effective_semantic_effect
    }

    pub(crate) fn authored_semantic_effect(&self) -> &[u8] {
        &self.semantic_effect
    }

    pub(crate) fn effective_transitions(
        &self,
    ) -> &[super::hot_engine::AuthenticatedPageLocalEffectiveTransition] {
        &self.effective_transitions
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

fn lower_physical_claim(claim: ProjectionClaim) -> storage_frontier::PhysicalClaim {
    storage_frontier::PhysicalClaim {
        workspace_id: claim.workspace_id.as_uuid().into_bytes(),
        lineage_digest: ContentDigest::from_bytes(*claim.lineage_digest.as_bytes()),
        oplog_protocol_version: claim.oplog_protocol_version,
        operation_schema_version: claim.operation_schema_version,
        object_envelope_schema_version: claim.object_envelope_schema_version,
        manifest_encoding_version: claim.manifest_encoding_version,
        managed_entity_set_version: claim.managed_entity_set_version,
    }
}

fn lower_physical_frontier_root(
    root: &AcceptedFrontierRoot,
) -> Result<storage_frontier::PhysicalFrontierRoot, ProjectionError> {
    Ok(storage_frontier::PhysicalFrontierRoot {
        canonical_bytes: canonical_frontier_root_bytes(root)?,
        acceptance_sequence: root.acceptance_sequence(),
        document_count: root.document_count(),
        document_map_root_key: root.document_map_root_key(),
        document_map_root_digest: root.document_map_root_digest(),
        batch_map_root_key: root.batch_map_root_key(),
        batch_map_root_digest: root.batch_map_root_digest(),
        state_digest: root.state_digest(),
    })
}

fn lower_physical_accepted_batch(
    event: &AcceptedBatchEvent,
) -> Result<storage_frontier::PhysicalAcceptedBatch, ProjectionError> {
    let prior_frontier_root = lower_physical_frontier_root(&event.prior_frontier_root)?;
    let post_frontier_root = lower_physical_frontier_root(&event.post_frontier_root)?;
    let affected_documents_bytes = canonical_affected_documents_bytes(&event.affected_documents)?;
    let causal_dependency_heads_bytes = encode_batch_ids(&event.causal_dependency_heads)?;
    let causal_peer_id = event
        .causal_dot
        .peer_id()
        .as_device_id()
        .as_uuid()
        .into_bytes();
    Ok(storage_frontier::PhysicalAcceptedBatch {
        batch_id: event.batch_id.as_uuid().into_bytes(),
        manifest_digest: event.manifest_digest,
        event_binding_digest: event.event_binding_digest,
        semantic_effect: event.semantic_effect.clone(),
        semantic_effect_digest: ContentDigest::from_bytes(*event.semantic_effect_digest.as_bytes()),
        dependency_frontier: canonical_frontier_bytes(&event.dependency_frontier)?,
        prior_frontier_root,
        post_frontier_root,
        affected_documents: event
            .affected_documents
            .iter()
            .map(|document| {
                Ok(storage_frontier::PhysicalFrontierDocument {
                    document_id: document.document_id().as_uuid().into_bytes(),
                    canonical_bytes: encode_frontier_document(document)?,
                })
            })
            .collect::<Result<_, ProjectionError>>()?,
        affected_documents_bytes,
        causal_dependency_heads: event
            .causal_dependency_heads
            .iter()
            .map(|batch_id| batch_id.as_uuid().into_bytes())
            .collect(),
        causal_dependency_heads_bytes,
        causal_peer_id,
        causal_counter: event.causal_dot.counter(),
        acceptance_sequence: event.acceptance_sequence,
        retained_bytes: u64::try_from(event.retained_bytes).map_err(|_| {
            ProjectionError::InvalidAcceptedEvent("accepted retained bytes exceed u64".into())
        })?,
    })
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
    pub(crate) fn open_for_test(path: &Path) -> Result<Self, ProjectionError> {
        let path = prepare_application_runtime_root(path)?;
        Ok(Self { path })
    }

    /// Isolated deterministic harnesses need a caller-owned runtime root so
    /// they never consult or mutate normal application startup state.
    pub(crate) fn open_for_harness(path: &Path) -> Result<Self, ProjectionError> {
        let path = prepare_application_runtime_root(path)?;
        Ok(Self { path })
    }

    /// Open an explicitly supplied private application-runtime root for the
    /// opt-in activation API.  Its caller has already established that the
    /// path is outside the graph; this constructor preserves the runtime
    /// root's own no-follow and ownership checks.
    pub(crate) fn open_explicit_private(path: &Path) -> Result<Self, ProjectionError> {
        let path = prepare_application_runtime_root(path)?;
        Ok(Self { path })
    }

    /// Retain an already-existing private runtime root without creating it.
    pub(crate) fn open_existing_for_runtime_host(path: &Path) -> Result<Self, ProjectionError> {
        let direct_metadata = fs::symlink_metadata(path)?;
        if direct_metadata.file_type().is_symlink() || !direct_metadata.is_dir() {
            return Err(ProjectionError::UnsafePath(
                "application runtime root is not a no-follow directory".into(),
            ));
        }
        let path = fs::canonicalize(path)?;
        let metadata = fs::symlink_metadata(&path)?;
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
    loader: RebuildLoader<'a>,
    runtime_authority: EngineAuthority,
    exact_frontier_root: AcceptedFrontierRoot,
    accepted_batch_count: u64,
}

enum RebuildLoader<'a> {
    Ordinary,
    InactiveBootstrap {
        publication: &'a ValidatedBootstrapPublicationV1,
    },
    /// A promoted bootstrap-anchored lineage: the leading acceptance sequences
    /// are the retained immutable bootstrap parts and everything after them is
    /// an ordinary archived batch.
    PromotedBootstrapAnchored {
        publication: &'a ValidatedBootstrapPublicationV1,
    },
}

struct RebuildCursor<'a> {
    source: &'a RebuildSource<'a>,
    accepted: super::hot_engine::AcceptedBatchCursor<'a>,
    bootstrap: BootstrapSqliteRebuildInstrumentation,
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
        let (event, bootstrap_objects) =
            self.source
                .load_event(sequence, batch_id, indexed_evidence.as_ref())?;
        if let Some(object_count) = bootstrap_objects {
            self.bootstrap.bootstrap_part_reads += 1;
            self.bootstrap.bootstrap_object_reads = self
                .bootstrap
                .bootstrap_object_reads
                .saturating_add(object_count);
            self.bootstrap.max_live_bootstrap_parts =
                self.bootstrap.max_live_bootstrap_parts.max(1);
        }
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

    fn bootstrap_instrumentation(&self) -> BootstrapSqliteRebuildInstrumentation {
        self.bootstrap
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
            loader: RebuildLoader::Ordinary,
            runtime_authority: engine.runtime_authority().clone(),
            exact_frontier_root,
            accepted_batch_count,
        })
    }

    /// Rebuild source for a promoted runtime, whose accepted history begins
    /// with the retained immutable bootstrap parts and continues with ordinary
    /// archived batches.
    pub(crate) fn from_promoted_runtime(
        engine: &'a ShardedHotEngine,
        store: &'a ObjectStore,
        publication: &'a ValidatedBootstrapPublicationV1,
    ) -> Result<Self, ProjectionError> {
        let aggregate = publication.aggregate();
        if aggregate.workspace_id() != engine.workspace_id()
            || aggregate.lineage_digest() != engine.lineage_digest()
            || store.workspace_id() != engine.workspace_id()
        {
            return Err(ProjectionError::Rebuild(
                "promoted bootstrap publication is not this runtime's lineage".into(),
            ));
        }
        let exact_frontier_root = engine
            .accepted_frontier_root()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        let accepted_batch_count = engine
            .accepted_batch_count()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        if accepted_batch_count < aggregate.parts().len() as u64 {
            return Err(ProjectionError::Rebuild(
                "promoted accepted history is behind its own bootstrap".into(),
            ));
        }
        Ok(Self {
            engine,
            store,
            loader: RebuildLoader::PromotedBootstrapAnchored { publication },
            runtime_authority: engine.runtime_authority().clone(),
            exact_frontier_root,
            accepted_batch_count,
        })
    }

    pub(crate) fn from_inactive_bootstrap(
        authority: &'a InactiveBootstrapAcceptedAuthority,
    ) -> Result<Self, ProjectionError> {
        let engine = authority.accepted_engine();
        let store = authority.store();
        let binding = authority.binding();
        let exact_frontier_root = engine
            .accepted_frontier_root()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        let accepted_batch_count = engine
            .accepted_batch_count()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        if engine.workspace_id() != binding.workspace_id()
            || engine.lineage_digest() != binding.lineage_digest()
            || store.workspace_id() != binding.workspace_id()
            || &exact_frontier_root != binding.accepted_frontier()
            || accepted_batch_count != u64::from(binding.part_count())
        {
            return Err(ProjectionError::Rebuild(
                "inactive bootstrap authority changed before SQLite rebuild".into(),
            ));
        }
        Ok(Self {
            engine,
            store,
            loader: RebuildLoader::InactiveBootstrap {
                publication: authority.publication(),
            },
            runtime_authority: engine.runtime_authority().clone(),
            exact_frontier_root,
            accepted_batch_count,
        })
    }

    fn load_event(
        &self,
        acceptance_sequence: u64,
        batch_id: BatchId,
        indexed_evidence: Option<&super::AcceptedBatchEvidence>,
    ) -> Result<(AcceptedBatchEvent, Option<usize>), ProjectionError> {
        match &self.loader {
            RebuildLoader::Ordinary => {
                let event = match indexed_evidence {
                    Some(evidence) => AcceptedBatchEvent::from_indexed(
                        self.engine,
                        self.store,
                        batch_id,
                        evidence,
                    )?,
                    None => AcceptedBatchEvent::from_accepted(self.engine, self.store, batch_id)?,
                };
                Ok((event, None))
            }
            RebuildLoader::PromotedBootstrapAnchored { publication }
                if acceptance_sequence > publication.aggregate().parts().len() as u64 =>
            {
                let event = match indexed_evidence {
                    Some(evidence) => AcceptedBatchEvent::from_indexed(
                        self.engine,
                        self.store,
                        batch_id,
                        evidence,
                    )?,
                    None => AcceptedBatchEvent::from_accepted(self.engine, self.store, batch_id)?,
                };
                Ok((event, None))
            }
            RebuildLoader::InactiveBootstrap { publication }
            | RebuildLoader::PromotedBootstrapAnchored { publication } => {
                let evidence = indexed_evidence.ok_or_else(|| {
                    ProjectionError::Rebuild(format!(
                        "bootstrap batch {batch_id} lacks indexed accepted evidence"
                    ))
                })?;
                let ordinal = acceptance_sequence
                    .checked_sub(1)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        ProjectionError::Rebuild(
                            "bootstrap acceptance sequence cannot address a part".into(),
                        )
                    })?;
                let descriptor = publication
                    .aggregate()
                    .parts()
                    .get(ordinal)
                    .copied()
                    .ok_or_else(|| {
                        ProjectionError::Rebuild(
                            "bootstrap accepted sequence exceeds publication parts".into(),
                        )
                    })?;
                if descriptor.acceptance_sequence() as u64 != acceptance_sequence
                    || descriptor.evidence().ordinal() as usize != ordinal
                    || descriptor.batch_id() != batch_id
                    || evidence.batch_id() != batch_id
                    || evidence.acceptance_sequence() != acceptance_sequence
                {
                    return Err(ProjectionError::InvalidAcceptedEvent(format!(
                        "bootstrap part {ordinal} differs from indexed accepted evidence"
                    )));
                }
                let loaded = self.store.load_bootstrap_part(publication, ordinal)?;
                let object_count = loaded.objects().len();
                let prepared =
                    PreparedBatch::new(loaded.manifest().clone(), loaded.objects().to_vec())
                        .map_err(|error| {
                            ProjectionError::InvalidAcceptedEvent(error.to_string())
                        })?;
                let validated = ValidatedBatch::new(prepared);
                if validated.manifest().lineage_digest() != self.engine.lineage_digest() {
                    return Err(ProjectionError::LineageMismatch {
                        expected: self.engine.lineage_digest(),
                        found: validated.manifest().lineage_digest(),
                    });
                }
                let event = AcceptedBatchEvent::from_validated(&validated, evidence)?;
                authenticate_event_for_engine(self.engine, &event)?;
                Ok((event, Some(object_count)))
            }
        }
    }

    pub(crate) fn accepted_event_at(
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
        let (event, _) =
            self.load_event(acceptance_sequence, batch_id, indexed_evidence.as_ref())?;
        if event.acceptance_sequence != acceptance_sequence {
            return Err(ProjectionError::Rebuild(format!(
                "accepted batch {batch_id} is indexed at sequence {acceptance_sequence} but carries {}",
                event.acceptance_sequence
            )));
        }
        Ok(event)
    }

    fn authenticate_exact_frontier(&self) -> Result<(), ProjectionError> {
        if !self
            .runtime_authority
            .matches(self.engine.runtime_authority())
        {
            return Err(ProjectionError::AuthorityMismatch);
        }
        if self.accepted_batch_count == 0 {
            if self.exact_frontier_root != AcceptedFrontierRoot::empty() {
                return Err(ProjectionError::Rebuild(
                    "empty accepted history has a non-empty frontier".into(),
                ));
            }
            return Ok(());
        }
        if matches!(self.loader, RebuildLoader::InactiveBootstrap { .. }) {
            let (batch_id, evidence) = self
                .engine
                .accepted_batch_entry_at(self.accepted_batch_count)
                .map_err(|error| ProjectionError::Rebuild(error.to_string()))?
                .ok_or_else(|| {
                    ProjectionError::Rebuild(
                        "bootstrap accepted history has no terminal indexed event".into(),
                    )
                })?;
            let evidence = evidence.ok_or_else(|| {
                ProjectionError::Rebuild(
                    "bootstrap accepted history terminal lacks indexed evidence".into(),
                )
            })?;
            if evidence.batch_id() != batch_id
                || evidence.acceptance_sequence() != self.accepted_batch_count
                || evidence.post_frontier_root() != &self.exact_frontier_root
            {
                return Err(ProjectionError::Rebuild(
                    "bootstrap accepted history tail is not bound to exact frontier".into(),
                ));
            }
            return Ok(());
        }
        let event = self.accepted_event_at(self.accepted_batch_count)?;
        authenticate_event_for_engine(self.engine, &event)?;
        if event.post_frontier_root() != &self.exact_frontier_root {
            return Err(ProjectionError::Rebuild(
                "accepted history tail is not bound to the exact rebuild frontier".into(),
            ));
        }
        Ok(())
    }

    fn cursor(&'a self) -> Result<RebuildCursor<'a>, ProjectionError> {
        Ok(RebuildCursor {
            source: self,
            accepted: self
                .engine
                .accepted_batch_cursor()
                .map_err(|error| ProjectionError::Rebuild(error.to_string()))?,
            bootstrap: BootstrapSqliteRebuildInstrumentation::default(),
        })
    }
}

fn authenticate_event_for_engine(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
) -> Result<(), ProjectionError> {
    if event.workspace_id != engine.workspace_id() {
        return Err(ProjectionError::WorkspaceMismatch {
            expected: engine.workspace_id(),
            found: event.workspace_id,
        });
    }
    if event.lineage_digest != engine.lineage_digest() {
        return Err(ProjectionError::LineageMismatch {
            expected: engine.lineage_digest(),
            found: event.lineage_digest,
        });
    }
    let evidence = engine
        .accepted_batch_evidence(event.batch_id())
        .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
    if evidence.manifest_fingerprint() != event.manifest_digest()
        || evidence.event_binding_digest() != event.event_binding_digest()
        || evidence.acceptance_sequence() != event.acceptance_sequence()
        || evidence.prior_frontier_root() != event.prior_frontier_root()
        || evidence.post_frontier_root() != event.post_frontier_root()
        || evidence.affected_documents() != event.affected_documents()
    {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} is not bound to the engine's authenticated history",
            event.batch_id()
        )));
    }
    let authored = SemanticEffect::decode(event.authored_semantic_effect()).map_err(|error| {
        ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} authored effect is invalid: {error}",
            event.batch_id()
        ))
    })?;
    if SemanticEffectDigest::of(event.authored_semantic_effect()) != event.semantic_effect_digest()
    {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} authored manifest binding changed",
            event.batch_id()
        )));
    }
    let expected = engine
        .accepted_effective_semantic_view(event.batch_id(), &authored)
        .map_err(|error| ProjectionError::InvalidAcceptedEvent(error.to_string()))?;
    let expected_bytes = expected.effect().encode().map_err(|error| {
        ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} effective effect is invalid: {error}",
            event.batch_id()
        ))
    })?;
    if expected_bytes != event.semantic_effect()
        || expected.transitions() != event.effective_transitions()
    {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} effective transition proof changed",
            event.batch_id()
        )));
    }
    Ok(())
}

fn document_facets(
    content: &str,
    is_org: bool,
) -> (
    String,
    Option<u8>,
    bool,
    Vec<super::MaterializedProperty>,
    Vec<String>,
    Option<super::MaterializedTask>,
) {
    // `DocBlock` removes only parser-recognized properties from visible text.
    // Every other facet needs a property/header/tag/priority marker or an
    // uppercase task/planning token. If none can occur, preserve the same
    // searchable text without constructing a potentially very large AST.
    let may_have_facets = content
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b':' | b'#' | b'[' | b'*') || byte.is_ascii_uppercase());
    if !may_have_facets {
        return (
            content.split_whitespace().collect::<Vec<_>>().join(" "),
            None,
            false,
            Vec::new(),
            Vec::new(),
            None,
        );
    }
    let mut block = crate::doc::DocBlock::new(content);
    block.is_org = is_org;
    let searchable_text = block
        .visible_text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let heading_level = block.heading_level();
    let collapsed = block.collapsed();
    let properties = block
        .properties()
        .into_iter()
        .map(|(name, value)| super::MaterializedProperty { name, value })
        .collect();
    let tags = block.tags();
    let task = block.marker().map(|marker| super::MaterializedTask {
        marker: marker.to_owned(),
        priority: block.priority().map(str::to_owned),
        scheduled: block.scheduled().map(str::to_owned),
        deadline: block.deadline().map(str::to_owned),
    });
    (
        searchable_text,
        heading_level,
        collapsed,
        properties,
        tags,
        task,
    )
}

fn materialized_page_input(page: super::MaterializedPage) -> super::MaterializedPageInput {
    let is_org = super::reference_catalog::reference_source_is_org(&page.path);
    let (preamble_search, _, _, properties, tags, _) = page
        .preamble
        .as_deref()
        .map(|preamble| document_facets(preamble, is_org))
        .unwrap_or_default();
    let mut page_search = Vec::with_capacity(2);
    page_search.push(page.name.as_str().to_owned());
    if !preamble_search.is_empty() {
        page_search.push(preamble_search);
    }
    let blocks = page
        .blocks
        .into_iter()
        .map(|block| {
            let (searchable_text, heading_level, collapsed, properties, tags, task) =
                document_facets(&block.content, is_org);
            super::MaterializedBlockInput {
                block_id: block.block_id,
                home_document_id: block.home_document_id,
                parent: block.parent,
                order: block.order,
                content: block.content,
                searchable_text,
                heading_level,
                collapsed,
                logseq_uuid: block.logseq_uuid,
                logseq_identity_origin: block.logseq_identity_origin,
                references: Vec::new(),
                properties,
                tags,
                task,
            }
        })
        .collect::<Vec<_>>();
    super::MaterializedPageInput {
        page_id: page.page_id,
        home_document_id: page.home_document_id,
        name: page.name.as_str().to_owned(),
        name_key: page.name.canonical_key(),
        path: page.path,
        kind: page.kind,
        preamble: page.preamble,
        searchable_text: page_search.join(" "),
        references: Vec::new(),
        properties,
        tags,
        blocks,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EventMaterializationInstrumentation {
    accepted_root_authentications: usize,
    exact_document_loads: usize,
    exact_catalog_loads: usize,
    bulk_materialization_chunks: usize,
    bulk_pages_materialized: usize,
    peak_bulk_pages: usize,
    accepted_frontier_session_hits: usize,
    accepted_frontier_session_misses: usize,
    accepted_frontier_session_evictions: usize,
    accepted_frontier_session_oversize: usize,
    accepted_frontier_session_peak_resident_bytes: usize,
    external_exact_session_hits: usize,
    external_exact_session_misses: usize,
    external_exact_session_evictions: usize,
    external_exact_session_oversize: usize,
    external_exact_session_peak_resident_bytes: usize,
}

pub(crate) fn materialize_accepted_event(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
) -> Result<super::MaterializationChange, ProjectionError> {
    materialize_accepted_event_with_stats(engine, event).map(|(change, _)| change)
}

fn materialize_accepted_event_with_stats(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
) -> Result<
    (
        super::MaterializationChange,
        EventMaterializationInstrumentation,
    ),
    ProjectionError,
> {
    authenticate_event_for_engine(engine, event)?;
    let effect = SemanticEffect::decode(event.semantic_effect())
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    let canonical_effect = effect
        .encode()
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    if canonical_effect != event.semantic_effect() {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} has a non-canonical semantic effect",
            event.batch_id()
        )));
    }
    let affected_pages = super::reference_catalog::affected_reference_sources(&effect);
    let effective_transitions = effective_transition_index(event);
    let mut replacements = Vec::new();
    let mut deletions = Vec::new();
    let mut instrumentation = EventMaterializationInstrumentation::default();
    let mut materializer = (!affected_pages.is_empty())
        .then(|| engine.accepted_root_materializer(event.post_frontier_root()))
        .transpose()
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    if materializer.is_some() {
        instrumentation.accepted_root_authentications = 1;
    }
    for page_id in affected_pages {
        match materializer
            .as_mut()
            .expect("nonempty affected pages construct a materializer")
            .materialize_page(page_id)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
        {
            Some(mut page) => {
                if let Some(transition) = effective_transitions.get(&page_id) {
                    transition
                        .apply_to_materialized(&mut page)
                        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
                }
                replacements.push(materialized_page_input(page));
            }
            None => deletions.push(page_id),
        }
    }
    if let Some(materializer) = materializer {
        instrumentation.exact_document_loads = materializer.exact_document_loads();
        instrumentation.exact_catalog_loads = materializer.exact_catalog_loads();
    }
    let change = super::MaterializationChange::new(event.batch_id(), replacements, deletions)?;
    Ok((change, instrumentation))
}

/// Materialize one inactive-bootstrap event at its authenticated accepted root.
/// The private capability retains the event's validated catalog while batching
/// every membership and home checkpoint needed by the affected page chunk.
fn materialize_inactive_bootstrap_event_bulk(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
) -> Result<
    (
        super::MaterializationChange,
        EventMaterializationInstrumentation,
    ),
    ProjectionError,
> {
    materialize_inactive_bootstrap_event_bulk_with_budget(
        engine,
        event,
        super::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT,
    )
}

fn materialize_inactive_bootstrap_event_bulk_with_budget(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
    session_budget_bytes_per_root: usize,
) -> Result<
    (
        super::MaterializationChange,
        EventMaterializationInstrumentation,
    ),
    ProjectionError,
> {
    authenticate_event_for_engine(engine, event)?;
    let effect = SemanticEffect::decode(event.semantic_effect())
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    let canonical_effect = effect
        .encode()
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    if canonical_effect != event.semantic_effect() {
        return Err(ProjectionError::InvalidAcceptedEvent(format!(
            "accepted event {} has a non-canonical semantic effect",
            event.batch_id()
        )));
    }
    let affected_pages = super::reference_catalog::affected_reference_sources(&effect)
        .into_iter()
        .collect::<Vec<_>>();
    let effective_transitions = effective_transition_index(event);
    let materializer = (!affected_pages.is_empty())
        .then(|| {
            engine
                .bootstrap_bulk_materializer_with_session_budget(
                    event.post_frontier_root(),
                    session_budget_bytes_per_root,
                )
                .map_err(|error| ProjectionError::Materialization(error.to_string()))
        })
        .transpose()?;
    let mut replacements = Vec::new();
    let mut deletions = Vec::new();
    for page_ids in affected_pages.chunks(super::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES)
    {
        let pages = materializer
            .as_ref()
            .expect("nonempty affected pages construct a bulk materializer")
            .materialize_pages(page_ids)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        for (page_id, page) in page_ids.iter().copied().zip(pages) {
            match page {
                Some(mut page) => {
                    if let Some(transition) = effective_transitions.get(&page_id) {
                        transition
                            .apply_to_materialized(&mut page)
                            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
                    }
                    replacements.push(materialized_page_input(page));
                }
                None => deletions.push(page_id),
            }
        }
    }
    let change = super::MaterializationChange::new(event.batch_id(), replacements, deletions)?;
    let (accepted_frontier_stats, external_exact_stats) = materializer
        .as_ref()
        .map_or_else(Default::default, |materializer| {
            materializer.lookup_session_stats()
        });
    Ok((
        change,
        EventMaterializationInstrumentation {
            accepted_root_authentications: usize::from(materializer.is_some()),
            exact_document_loads: materializer
                .as_ref()
                .map_or(0, |materializer| materializer.exact_document_loads()),
            exact_catalog_loads: usize::from(materializer.is_some()),
            bulk_materialization_chunks: affected_pages
                .len()
                .div_ceil(super::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES),
            bulk_pages_materialized: affected_pages.len(),
            peak_bulk_pages: affected_pages
                .len()
                .min(super::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES),
            accepted_frontier_session_hits: accepted_frontier_stats.hits,
            accepted_frontier_session_misses: accepted_frontier_stats.misses,
            accepted_frontier_session_evictions: accepted_frontier_stats.evictions,
            accepted_frontier_session_oversize: accepted_frontier_stats.oversize,
            accepted_frontier_session_peak_resident_bytes: accepted_frontier_stats
                .peak_resident_bytes,
            external_exact_session_hits: external_exact_stats.hits,
            external_exact_session_misses: external_exact_stats.misses,
            external_exact_session_evictions: external_exact_stats.evictions,
            external_exact_session_oversize: external_exact_stats.oversize,
            external_exact_session_peak_resident_bytes: external_exact_stats.peak_resident_bytes,
        },
    ))
}

#[cfg(test)]
fn materialize_accepted_event_pointwise(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
) -> Result<super::MaterializationChange, ProjectionError> {
    authenticate_event_for_engine(engine, event)?;
    let effect = SemanticEffect::decode(event.semantic_effect())
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    let effective_transitions = effective_transition_index(event);
    let mut replacements = Vec::new();
    let mut deletions = Vec::new();
    for page_id in super::reference_catalog::affected_reference_sources(&effect) {
        match engine
            .materialize_page_at_accepted_root(event.post_frontier_root(), page_id)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
        {
            Some(mut page) => {
                if let Some(transition) = effective_transitions.get(&page_id) {
                    transition
                        .apply_to_materialized(&mut page)
                        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
                }
                replacements.push(materialized_page_input(page));
            }
            None => deletions.push(page_id),
        }
    }
    super::MaterializationChange::new(event.batch_id(), replacements, deletions).map_err(Into::into)
}

/// Page IDs are canonical map keys. `find` used the first matching transition,
/// so retaining the first duplicate deliberately keeps malformed-event behavior
/// unchanged while avoiding a transition scan for every affected page.
fn effective_transition_index(
    event: &AcceptedBatchEvent,
) -> BTreeMap<PageId, &super::hot_engine::AuthenticatedPageLocalEffectiveTransition> {
    let mut transitions = BTreeMap::new();
    for transition in event.effective_transitions() {
        transitions
            .entry(transition.page_id())
            .or_insert(transition);
    }
    transitions
}

fn attach_authenticated_reference_catalog(
    engine: &ShardedHotEngine,
    event: &AcceptedBatchEvent,
    change: super::MaterializationChange,
) -> Result<super::MaterializationChange, ProjectionError> {
    attach_authenticated_reference_catalog_at(
        engine,
        event.semantic_effect(),
        event.prior_frontier_root(),
        event.post_frontier_root(),
        change,
    )
}

fn attach_authenticated_reference_catalog_at(
    engine: &ShardedHotEngine,
    semantic_effect: &[u8],
    prior_frontier_root: &AcceptedFrontierRoot,
    post_frontier_root: &AcceptedFrontierRoot,
    change: super::MaterializationChange,
) -> Result<super::MaterializationChange, ProjectionError> {
    let effect = SemanticEffect::decode(semantic_effect)
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
    let post_root = post_frontier_root.reference_catalog_root().clone();
    let prior_root = prior_frontier_root.reference_catalog_root().clone();
    let stamp = super::sqlite_materialization::ReferenceExtractorDependencyStamp::new(
        post_root.extractor_digest(),
        post_root.policy_digest(),
    )?;
    let mut rows = ReferenceCatalogSourceRows::default();
    let mut removed_sources = Vec::new();
    for page_id in super::reference_catalog::affected_reference_sources(&effect) {
        if !collect_reference_source_rows(engine, &post_root, stamp, page_id, &mut rows)? {
            removed_sources.push(page_id);
        }
    }
    let ReferenceCatalogSourceRows {
        coverage,
        postings,
        aliases,
    } = rows;
    let reference_catalog =
        super::sqlite_materialization::ReferenceCatalogMaterializationInput::new(
            prior_root,
            post_root,
            postings,
            aliases,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            coverage,
            removed_sources,
        )?;
    change
        .with_authenticated_reference_catalog(reference_catalog)
        .map_err(Into::into)
}

/// Opt-in construction phase trace, matching the activation trace the bootstrap
/// preparation phases already emit.
fn trace_terminal_phase(label: &str, started: std::time::Instant) {
    if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
        eprintln!(
            "sqlite terminal {label}: {} ms",
            started.elapsed().as_millis()
        );
    }
}

fn record_candidate_write_instrumentation(
    instrumentation: &mut RebuildInstrumentation,
    before: storage_frontier::PhysicalWriteInstrumentation,
    after: storage_frontier::PhysicalWriteInstrumentation,
) {
    instrumentation.physical_candidate_transactions = after
        .candidate_transactions
        .saturating_sub(before.candidate_transactions);
    instrumentation.physical_candidate_durability_barriers = after
        .candidate_durability_barriers
        .saturating_sub(before.candidate_durability_barriers);
    instrumentation.physical_ordinary_transactions = after
        .ordinary_transactions
        .saturating_sub(before.ordinary_transactions);
    instrumentation.physical_ordinary_durability_barriers = after
        .ordinary_durability_barriers
        .saturating_sub(before.ordinary_durability_barriers);
}

/// Bind the retained process-local construction material to the exact archive
/// authority this build is for.
///
/// The material carries no acceptance of its own: this refuses unless its
/// events are the aggregate's parts, in order, with the authenticated prior/post
/// root chain that ends at the engine's exact terminal frontier. A refusal is
/// not corruption of the durable state — it only means this activation must
/// take the existing archive replay path.
fn validate_terminal_construction_material(
    source: &RebuildSource<'_>,
    publication: &ValidatedBootstrapPublicationV1,
    material: &TerminalBootstrapConstructionMaterial,
) -> Result<(), ProjectionError> {
    let aggregate = publication.aggregate();
    let engine = source.engine;
    if material.workspace_id() != engine.workspace_id()
        || material.lineage_digest() != engine.lineage_digest()
        || material.import_id() != aggregate.import_id()
    {
        return Err(ProjectionError::Rebuild(
            "retained terminal material belongs to another workspace, lineage, or import".into(),
        ));
    }
    let events = material.accepted_events();
    if events.len() != aggregate.parts().len() || events.len() as u64 != source.accepted_batch_count
    {
        return Err(ProjectionError::Rebuild(
            "retained terminal material does not cover the exact accepted prefix".into(),
        ));
    }
    let mut prior = AcceptedFrontierRoot::empty();
    for (ordinal, (event, descriptor)) in events.iter().zip(aggregate.parts().iter()).enumerate() {
        let sequence = u64::try_from(ordinal)
            .ok()
            .and_then(|ordinal| ordinal.checked_add(1))
            .ok_or_else(|| {
                ProjectionError::Rebuild("terminal accepted sequence overflowed".into())
            })?;
        let (indexed_batch_id, indexed_evidence) = engine
            .accepted_batch_entry_at(sequence)
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?
            .ok_or_else(|| {
                ProjectionError::Rebuild(format!("accepted history is missing sequence {sequence}"))
            })?;
        let evidence = indexed_evidence.ok_or_else(|| {
            ProjectionError::Rebuild(
                "bootstrap accepted sequence lacks indexed accepted evidence".into(),
            )
        })?;
        if event.batch_id() != indexed_batch_id
            || event.batch_id() != descriptor.batch_id()
            || event.acceptance_sequence() != sequence
            || u64::from(descriptor.acceptance_sequence()) != sequence
            || descriptor.evidence().ordinal() as usize != ordinal
            || evidence.batch_id() != indexed_batch_id
            || evidence.acceptance_sequence() != sequence
            || event.manifest_digest() != evidence.manifest_fingerprint()
            || event.event_binding_digest() != evidence.event_binding_digest()
            || *event.prior_frontier_root() != prior
        {
            return Err(ProjectionError::Rebuild(format!(
                "retained terminal event {ordinal} is not the aggregate's authenticated part"
            )));
        }
        prior = event.post_frontier_root().clone();
    }
    if prior != source.exact_frontier_root {
        return Err(ProjectionError::Rebuild(
            "retained terminal events do not chain to the engine's accepted frontier root".into(),
        ));
    }
    Ok(())
}

/// The authenticated catalog rows one reference source contributes at one
/// catalog root. Per-event lowering and terminal construction share this exact
/// derivation so a replayed database and a terminal one cannot disagree.
#[derive(Debug, Default)]
struct ReferenceCatalogSourceRows {
    coverage: Vec<super::sqlite_materialization::SourceCoverageFacet>,
    postings: Vec<super::sqlite_materialization::MaterializedReferencePosting>,
    aliases: Vec<super::sqlite_materialization::MaterializedAliasDeclaration>,
}

/// Append one source's authenticated rows. Returns `false` when the catalog has
/// no posting for that page at this root, which the per-event caller records as
/// a removed source.
fn collect_reference_source_rows(
    engine: &ShardedHotEngine,
    post_root: &super::ReferenceCatalogRootV2,
    stamp: super::sqlite_materialization::ReferenceExtractorDependencyStamp,
    page_id: PageId,
    rows: &mut ReferenceCatalogSourceRows,
) -> Result<bool, ProjectionError> {
    let Some(posting) = engine
        .reference_source_posting_at(post_root, page_id)
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?
    else {
        return Ok(false);
    };
    rows.coverage
        .push(super::sqlite_materialization::SourceCoverageFacet {
            source_page_id: page_id,
            source_digest: posting
                .digest()
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?,
            extractor_dependency_stamp: stamp,
        });
    for (ordinal, fact) in posting.facts().iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| {
            ProjectionError::Materialization(
                "reference posting ordinal exceeds the SQLite adapter bound".into(),
            )
        })?;
        let (source_entity, source_locator) = match fact {
            ReferenceFactV1::PageName(fact) => (fact.source, fact.source),
            ReferenceFactV1::Block(fact) => (fact.source, fact.source),
        };
        let source_entity = match source_entity {
            ReferenceSourceLocatorV1::Preamble => super::MaterializedEntityId::Page(page_id),
            ReferenceSourceLocatorV1::Block { block_id, .. } => {
                super::MaterializedEntityId::Block(block_id)
            }
        };
        let (kind, target) = match fact {
            ReferenceFactV1::PageName(fact) => (
                super::sqlite_materialization::ReferenceCatalogReferenceKind::from_page_kind(
                    fact.kind,
                ),
                super::sqlite_materialization::MaterializedReferenceTarget::PageName {
                    raw_name: fact.raw_target.clone(),
                    normalized_name: fact.normalized_target.clone(),
                    resolved_page_id: None,
                },
            ),
            ReferenceFactV1::Block(fact) => (
                super::sqlite_materialization::ReferenceCatalogReferenceKind::from_block_kind(
                    fact.kind,
                ),
                super::sqlite_materialization::MaterializedReferenceTarget::ExternalUuid {
                    raw_claim: fact.logseq_uuid,
                    resolved_block_id: None,
                },
            ),
        };
        rows.postings.push(
            super::sqlite_materialization::MaterializedReferencePosting {
                source_page_id: page_id,
                source_entity,
                source_locator,
                ordinal,
                kind,
                target,
            },
        );
        if let ReferenceFactV1::PageName(fact) = fact {
            if matches!(fact.kind, super::PageReferenceKindV1::AliasDeclaration) {
                rows.aliases.push(
                    super::sqlite_materialization::MaterializedAliasDeclaration {
                        source_page_id: page_id,
                        source_entity,
                        source_locator,
                        ordinal,
                        raw_alias: fact.raw_target.clone(),
                        normalized_alias: fact.normalized_target.clone(),
                    },
                );
            }
        }
    }
    Ok(true)
}

fn authenticated_reference_materialization(
    event: &AcceptedBatchEvent,
) -> Result<super::sqlite_materialization::AuthenticatedReferenceMaterialization, ProjectionError> {
    Ok(
        super::sqlite_materialization::AuthenticatedReferenceMaterialization {
            event_binding_digest: event.event_binding_digest(),
            prior_frontier_root_digest: ContentDigest::of(&canonical_frontier_root_bytes(
                event.prior_frontier_root(),
            )?),
            post_frontier_root_digest: ContentDigest::of(&canonical_frontier_root_bytes(
                event.post_frontier_root(),
            )?),
            prior_catalog_root: event.prior_frontier_root().reference_catalog_root().clone(),
            post_catalog_root: event.post_frontier_root().reference_catalog_root().clone(),
        },
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForensicEvidence {
    pub original_path: PathBuf,
    pub preserved_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionCheckpoint {
    schema_version: u32,
    workspace_id: WorkspaceId,
    frontier_root_digest: ContentDigest,
    database: PhysicalFileCheckpoint,
    wal: Option<PhysicalFileCheckpoint>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedBootstrapSqliteProjection {
    claim: ProjectionClaim,
    frontier_root: AcceptedFrontierRoot,
    accepted_batch_count: u64,
    semantic_projection_digest: ContentDigest,
    materialized_row_digest: ContentDigest,
    authority_binding: InactiveBootstrapAcceptedAuthorityBinding,
    bootstrap_rebuild: BootstrapSqliteRebuildInstrumentation,
}

impl VerifiedBootstrapSqliteProjection {
    pub(crate) const fn claim(&self) -> ProjectionClaim {
        self.claim
    }

    pub(crate) const fn frontier_root(&self) -> &AcceptedFrontierRoot {
        &self.frontier_root
    }

    pub(crate) const fn accepted_batch_count(&self) -> u64 {
        self.accepted_batch_count
    }

    pub(crate) const fn semantic_projection_digest(&self) -> ContentDigest {
        self.semantic_projection_digest
    }

    pub(crate) const fn materialized_row_digest(&self) -> ContentDigest {
        self.materialized_row_digest
    }

    pub(crate) const fn bootstrap_rebuild(&self) -> BootstrapSqliteRebuildInstrumentation {
        self.bootstrap_rebuild
    }

    pub(crate) const fn authority_binding(&self) -> &InactiveBootstrapAcceptedAuthorityBinding {
        &self.authority_binding
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BootstrapSqliteRebuildInstrumentation {
    pub(crate) bootstrap_part_reads: usize,
    pub(crate) bootstrap_object_reads: usize,
    pub(crate) max_live_bootstrap_parts: usize,
    /// One when this database was seeded from the retained terminal accepted
    /// state, zero when it replayed the archive parts.
    pub(crate) terminal_constructions: usize,
    /// Per-part intermediate page/reference materializations run through
    /// ordinary event DML. Terminal construction must leave this at zero.
    pub(crate) intermediate_page_materializations: usize,
    pub(crate) terminal_materializations: usize,
    pub(crate) terminal_pages_materialized: usize,
    pub(crate) terminal_materialization_chunks: usize,
    pub(crate) peak_terminal_bulk_pages: usize,
    pub(crate) terminal_materialization_micros: u128,
    pub(crate) terminal_reference_micros: u128,
    pub(crate) terminal_lowering_micros: u128,
    pub(crate) terminal_insert_micros: u128,
    pub(crate) terminal_catalog_cursor_micros: u128,
    pub(crate) terminal_finish_micros: u128,
    /// Bulk materializers, and therefore decoded-segment lookup sessions,
    /// opened by the terminal row seed. A graph-lifetime session leaves this at
    /// one no matter how many pages the terminal root carries.
    pub(crate) terminal_lookup_sessions: usize,
    /// Catalog rows the terminal row seed authenticated through the paged
    /// current-path cursor.
    pub(crate) terminal_catalog_rows_authenticated: usize,
    /// Catalog-document shape proofs derived while seeding the terminal rows.
    ///
    /// Each one costs a read linear in the catalog's page entries, so this must
    /// count bounded read windows (cursor pages plus materialization chunks)
    /// and never catalog rows: one proof per row is quadratic in graph pages.
    pub(crate) terminal_catalog_document_validations: usize,
    /// Peak pages one lookup session was asked to cover before it was dropped.
    /// This is the structural window bound: it must not grow with graph pages.
    pub(crate) peak_terminal_session_pages: usize,
    pub(crate) terminal_accepted_frontier_session_hits: usize,
    pub(crate) terminal_accepted_frontier_session_misses: usize,
    pub(crate) terminal_accepted_frontier_session_evictions: usize,
    pub(crate) terminal_accepted_frontier_session_oversize: usize,
    pub(crate) terminal_accepted_frontier_session_peak_resident_bytes: usize,
    pub(crate) terminal_external_exact_session_hits: usize,
    pub(crate) terminal_external_exact_session_misses: usize,
    pub(crate) terminal_external_exact_session_evictions: usize,
    pub(crate) terminal_external_exact_session_oversize: usize,
    pub(crate) terminal_external_exact_session_peak_resident_bytes: usize,
    /// One when retained terminal material was present but refused, so this
    /// activation discarded the private candidate and replayed the archive.
    pub(crate) terminal_construction_refusals: usize,
}

impl BootstrapSqliteRebuildInstrumentation {
    /// Fold one finished terminal lookup session's decoded-segment counters in.
    ///
    /// Residency is a peak rather than a sum: it is the bound on how much
    /// decoded state one window may hold at once.
    fn record_terminal_lookup_session(
        &mut self,
        session_pages: usize,
        accepted_frontier: super::scratch_store::ScratchLookupSessionStats,
        external_exact: super::scratch_store::ScratchLookupSessionStats,
    ) {
        self.terminal_lookup_sessions = self.terminal_lookup_sessions.saturating_add(1);
        self.peak_terminal_session_pages = self.peak_terminal_session_pages.max(session_pages);
        self.terminal_accepted_frontier_session_hits = self
            .terminal_accepted_frontier_session_hits
            .saturating_add(accepted_frontier.hits);
        self.terminal_accepted_frontier_session_misses = self
            .terminal_accepted_frontier_session_misses
            .saturating_add(accepted_frontier.misses);
        self.terminal_accepted_frontier_session_evictions = self
            .terminal_accepted_frontier_session_evictions
            .saturating_add(accepted_frontier.evictions);
        self.terminal_accepted_frontier_session_oversize = self
            .terminal_accepted_frontier_session_oversize
            .saturating_add(accepted_frontier.oversize);
        self.terminal_accepted_frontier_session_peak_resident_bytes = self
            .terminal_accepted_frontier_session_peak_resident_bytes
            .max(accepted_frontier.peak_resident_bytes);
        self.terminal_external_exact_session_hits = self
            .terminal_external_exact_session_hits
            .saturating_add(external_exact.hits);
        self.terminal_external_exact_session_misses = self
            .terminal_external_exact_session_misses
            .saturating_add(external_exact.misses);
        self.terminal_external_exact_session_evictions = self
            .terminal_external_exact_session_evictions
            .saturating_add(external_exact.evictions);
        self.terminal_external_exact_session_oversize = self
            .terminal_external_exact_session_oversize
            .saturating_add(external_exact.oversize);
        self.terminal_external_exact_session_peak_resident_bytes = self
            .terminal_external_exact_session_peak_resident_bytes
            .max(external_exact.peak_resident_bytes);
    }

    /// The terminal builder's catalog authority must cost one document shape
    /// proof per bounded read window and never one per catalog row.
    ///
    /// Each proof reads the catalog document, which is linear in its page
    /// entries, so a per-row proof makes the graph-sized traversal quadratic in
    /// pages. This is asserted as an exact identity rather than a bound so that
    /// a regression to per-row derivation cannot hide inside slack.
    #[cfg(test)]
    pub(crate) fn assert_catalog_authority_is_window_bounded(&self) {
        assert_eq!(
            self.terminal_catalog_rows_authenticated, self.terminal_pages_materialized,
            "every terminal page comes from one authenticated catalog row"
        );
        assert_eq!(
            self.terminal_catalog_document_validations,
            self.terminal_catalog_rows_authenticated
                .div_ceil(TERMINAL_CATALOG_CURSOR_PAGE_ROWS)
                + self.terminal_materialization_chunks,
            "catalog shape proofs must count cursor pages plus materialization \
             chunks, never catalog rows: {self:?}"
        );
        // The one graph-lifetime decoded-segment session is measured, not
        // assumed: it must not thrash while it covers the whole terminal root.
        assert_eq!(self.terminal_accepted_frontier_session_evictions, 0);
        assert_eq!(self.terminal_accepted_frontier_session_oversize, 0);
        assert_eq!(self.terminal_external_exact_session_evictions, 0);
        assert_eq!(self.terminal_external_exact_session_oversize, 0);
        for peak in [
            self.terminal_accepted_frontier_session_peak_resident_bytes,
            self.terminal_external_exact_session_peak_resident_bytes,
        ] {
            assert!(
                peak <= super::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT,
                "lookup session peak residency {peak} exceeds its per-root budget"
            );
        }
    }
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
    pub accepted_root_authentications: usize,
    pub exact_document_loads: usize,
    pub exact_catalog_loads: usize,
    pub bulk_materialization_chunks: usize,
    pub bulk_pages_materialized: usize,
    pub peak_bulk_pages: usize,
    pub accepted_frontier_session_hits: usize,
    pub accepted_frontier_session_misses: usize,
    pub accepted_frontier_session_evictions: usize,
    pub accepted_frontier_session_oversize: usize,
    pub accepted_frontier_session_peak_resident_bytes: usize,
    pub external_exact_session_hits: usize,
    pub external_exact_session_misses: usize,
    pub external_exact_session_evictions: usize,
    pub external_exact_session_oversize: usize,
    pub external_exact_session_peak_resident_bytes: usize,
    pub cleanup_page_attempts: usize,
    pub cleanup_existing_pages: usize,
    pub cleanup_owned_rows: usize,
    pub cleanup_fts_rowids: usize,
    pub reference_coverage_inductive_checks: usize,
    pub reference_coverage_full_scans: usize,
    pub final_semantic_equivalence_proofs: usize,
    pub final_row_digest_equivalence_proofs: usize,
    #[cfg(test)]
    pub final_row_digest_proof_micros: u128,
    /// Accepted-event apply transactions only; schema setup and terminal file
    /// checkpoint/publication remain separately durable lifecycle boundaries.
    pub physical_candidate_transactions: u64,
    pub physical_candidate_durability_barriers: u64,
    pub physical_ordinary_transactions: u64,
    pub physical_ordinary_durability_barriers: u64,
}

impl RebuildInstrumentation {
    fn record_materialization(&mut self, stats: EventMaterializationInstrumentation) {
        self.accepted_root_authentications += stats.accepted_root_authentications;
        self.exact_document_loads += stats.exact_document_loads;
        self.exact_catalog_loads += stats.exact_catalog_loads;
        self.bulk_materialization_chunks += stats.bulk_materialization_chunks;
        self.bulk_pages_materialized += stats.bulk_pages_materialized;
        self.peak_bulk_pages = self.peak_bulk_pages.max(stats.peak_bulk_pages);
        self.accepted_frontier_session_hits += stats.accepted_frontier_session_hits;
        self.accepted_frontier_session_misses += stats.accepted_frontier_session_misses;
        self.accepted_frontier_session_evictions += stats.accepted_frontier_session_evictions;
        self.accepted_frontier_session_oversize += stats.accepted_frontier_session_oversize;
        self.accepted_frontier_session_peak_resident_bytes = self
            .accepted_frontier_session_peak_resident_bytes
            .max(stats.accepted_frontier_session_peak_resident_bytes);
        self.external_exact_session_hits += stats.external_exact_session_hits;
        self.external_exact_session_misses += stats.external_exact_session_misses;
        self.external_exact_session_evictions += stats.external_exact_session_evictions;
        self.external_exact_session_oversize += stats.external_exact_session_oversize;
        self.external_exact_session_peak_resident_bytes = self
            .external_exact_session_peak_resident_bytes
            .max(stats.external_exact_session_peak_resident_bytes);
    }
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
    runtime_authority: EngineAuthority,
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
    authenticated_source_frontier: Option<AcceptedFrontierRoot>,
}

struct RequiredFrontierTransition {
    replacement: Option<AcceptedFrontierRoot>,
}

struct TailAdmissionPlan {
    required_frontier: RequiredFrontierTransition,
    applied_through: u64,
    applied_retained_bytes_total: u64,
    authoritative_through: u64,
    authoritative_retained_bytes_total: u64,
    retained_bytes: usize,
    descriptor_overflow: bool,
    descriptor: Option<(u64, TailDescriptor)>,
    observed: bool,
}

/// Causal accounting for a frontier-gated reference query.  The counters make
/// the bounded work visible to tests without adding a graph-sized hot cache.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReferenceQueryInstrumentation {
    pub sqlite_candidate_sources: usize,
    pub tail_source_postings: usize,
    pub revalidated_sources: usize,
}

/// One exact parser-owned reference occurrence.  `fact` deliberately retains
/// the user-authored spelling and byte range; callers must not reconstruct it
/// from a normalized page key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontierReferenceHit {
    pub source_page_id: PageId,
    pub fact: ReferenceFactV1,
    pub resolved_page_id: Option<PageId>,
    pub resolved_block_id: Option<BlockId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontierReferenceResults {
    pub hits: Vec<FrontierReferenceHit>,
    pub instrumentation: ReferenceQueryInstrumentation,
}

fn retain_reference_hit_bounded(
    retained_bytes: &mut usize,
    hit: &FrontierReferenceHit,
) -> Result<(), ProjectionError> {
    let raw_bytes = match &hit.fact {
        ReferenceFactV1::PageName(fact) => fact.raw_target.len(),
        ReferenceFactV1::Block(fact) => fact.raw_claim.len(),
    };
    let bytes = 128_usize.saturating_add(raw_bytes);
    *retained_bytes = retained_bytes.saturating_add(bytes);
    if *retained_bytes > super::MAX_MATERIALIZATION_READ_BYTES {
        return Err(ProjectionError::Materialization(
            "reference query output exceeds the materialization read bound".into(),
        ));
    }
    Ok(())
}

/// Bounded rename candidate plan.  The caller still supplies projection
/// captures to `ShardedHotEngine::finalize_author_transaction`; this plan only
/// carries an already revalidated semantic transaction.
#[derive(Clone, Debug)]
pub struct FrontierRenamePlan {
    target_page_id: PageId,
    transaction: super::OperationTransaction,
    touched_sources: Vec<PageId>,
    instrumentation: ReferenceQueryInstrumentation,
}

impl FrontierRenamePlan {
    pub const fn target_page_id(&self) -> PageId {
        self.target_page_id
    }

    pub const fn transaction(&self) -> &super::OperationTransaction {
        &self.transaction
    }

    pub fn touched_sources(&self) -> &[PageId] {
        &self.touched_sources
    }

    pub const fn instrumentation(&self) -> ReferenceQueryInstrumentation {
        self.instrumentation
    }
}

/// Read-only reference view at the engine's current authenticated frontier.
/// SQLite contributes only reverse-index candidates from its stamped prefix;
/// every returned candidate is checked against the catalog's current exact
/// posting, and the bounded tail is read from that same catalog.
pub struct FrontierReferenceQuery<'a> {
    database: &'a SqliteFrontier,
    engine: &'a ShardedHotEngine,
    base_catalog_root: super::ReferenceCatalogRootV2,
    tail_sources: BTreeMap<PageId, Option<super::ReferenceSourcePostingV2>>,
    instrumentation: ReferenceQueryInstrumentation,
}

impl TailOverlay {
    #[cfg(test)]
    fn empty_for_test(engine: &ShardedHotEngine) -> Self {
        Self {
            runtime_authority: engine.runtime_authority().clone(),
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
            authenticated_source_frontier: None,
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
        if !database
            .runtime_authority
            .matches(&source.runtime_authority)
        {
            return Err(ProjectionError::AuthorityMismatch.into());
        }
        source.authenticate_exact_frontier()?;
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
            runtime_authority: source.runtime_authority.clone(),
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
            authenticated_source_frontier: Some(source.exact_frontier_root.clone()),
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

    pub(crate) fn reserve_bound_mutation(
        &mut self,
        database: &SqliteFrontier,
        engine: &ShardedHotEngine,
        retained_bytes: usize,
    ) -> Result<TailReservation, TailOverlayError> {
        self.authenticate_event_authority(database, engine)?;
        self.reserve_mutation(retained_bytes)
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
        engine: &ShardedHotEngine,
        event: AcceptedBatchEvent,
    ) -> Result<bool, TailOverlayError> {
        self.authenticate_event_authority(database, engine)?;
        let Some(&retained_bytes) = self.reservations.get(&reservation.id) else {
            return Err(TailOverlayError::UnknownReservation);
        };
        debug_assert_eq!(retained_bytes, reservation.retained_bytes);
        authenticate_event_for_engine(engine, &event)?;
        let plan = self.plan_authenticated_admission(database, &event)?;
        let observed = plan.observed;
        self.commit_admission(database, plan);
        self.reservations.remove(&reservation.id);
        self.reserved_bytes = self.reserved_bytes.saturating_sub(retained_bytes);
        Ok(observed)
    }

    /// Observe an already-authoritative local or provider event. The durable
    /// accepted-history sequence remains the backlog; RAM retains only bounded
    /// descriptors and can therefore discard stale duplicates immediately.
    pub fn try_enqueue(
        &mut self,
        database: &mut SqliteFrontier,
        engine: &ShardedHotEngine,
        event: &AcceptedBatchEvent,
    ) -> Result<bool, TailOverlayError> {
        self.authenticate_event_authority(database, engine)?;
        authenticate_event_for_engine(engine, event)?;
        let plan = self.plan_authenticated_admission(database, event)?;
        let observed = plan.observed;
        self.commit_admission(database, plan);
        Ok(observed)
    }

    fn authenticate_event_authority(
        &self,
        database: &SqliteFrontier,
        engine: &ShardedHotEngine,
    ) -> Result<(), TailOverlayError> {
        if !self.runtime_authority.matches(&database.runtime_authority)
            || !self.runtime_authority.matches(engine.runtime_authority())
        {
            return Err(ProjectionError::AuthorityMismatch.into());
        }
        Ok(())
    }

    fn plan_authenticated_admission(
        &self,
        database: &SqliteFrontier,
        event: &AcceptedBatchEvent,
    ) -> Result<TailAdmissionPlan, TailOverlayError> {
        let required_frontier = database.plan_required_frontier(event.post_frontier_root())?;
        let applied = database.frontier_root()?;
        let applied_through = self.applied_through.max(applied.acceptance_sequence());
        let applied_retained_bytes_total = self
            .applied_retained_bytes_total
            .max(applied.retained_bytes_total());
        if event.acceptance_sequence <= applied.acceptance_sequence() {
            database.validate_applied_tail_duplicate(event, &applied)?;
            return Ok(TailAdmissionPlan {
                required_frontier,
                applied_through,
                applied_retained_bytes_total,
                authoritative_through: self.authoritative_through,
                authoritative_retained_bytes_total: self.authoritative_retained_bytes_total,
                retained_bytes: self.retained_bytes,
                descriptor_overflow: self.descriptor_overflow,
                descriptor: None,
                observed: false,
            });
        }
        let descriptor = TailDescriptor {
            batch_id: event.batch_id,
            manifest_digest: event.manifest_digest,
            retained_bytes: event.retained_bytes,
        };
        if event.post_frontier_root.acceptance_sequence() != event.acceptance_sequence {
            return Err(ProjectionError::InvalidAcceptedEvent(
                "accepted event sequence differs from its authenticated post-root".into(),
            )
            .into());
        }
        if let Some(existing) = self.hot_descriptors.get(&event.acceptance_sequence) {
            if existing != &descriptor {
                return Err(TailOverlayError::BatchCollision(descriptor.batch_id));
            }
            return Ok(TailAdmissionPlan {
                required_frontier,
                applied_through,
                applied_retained_bytes_total,
                authoritative_through: self.authoritative_through,
                authoritative_retained_bytes_total: self.authoritative_retained_bytes_total,
                retained_bytes: self.retained_bytes,
                descriptor_overflow: self.descriptor_overflow,
                descriptor: None,
                observed: false,
            });
        }

        let mut authoritative_through = self.authoritative_through;
        let mut authoritative_retained_bytes_total = self.authoritative_retained_bytes_total;
        let mut retained_bytes = self.retained_bytes;
        if event.acceptance_sequence > authoritative_through {
            authoritative_through = event.acceptance_sequence;
            authoritative_retained_bytes_total = event
                .post_frontier_root
                .retained_bytes_total()
                .max(authoritative_retained_bytes_total);
            let retained = authoritative_retained_bytes_total
                .checked_sub(applied_retained_bytes_total)
                .ok_or(ProjectionError::FrontierRegression)?;
            retained_bytes = usize::try_from(retained).map_err(|_| {
                ProjectionError::Corrupt(
                    "durable accepted backlog exceeds addressable memory".into(),
                )
            })?;
        }
        let mut descriptor_overflow = self.descriptor_overflow;
        let descriptor =
            if self.hot_descriptors.len() < TAIL_MAX_BATCHES && retained_bytes <= TAIL_MAX_BYTES {
                Some((event.acceptance_sequence, descriptor))
            } else {
                descriptor_overflow = true;
                None
            };
        Ok(TailAdmissionPlan {
            required_frontier,
            applied_through,
            applied_retained_bytes_total,
            authoritative_through,
            authoritative_retained_bytes_total,
            retained_bytes,
            descriptor_overflow,
            descriptor,
            observed: true,
        })
    }

    fn commit_admission(&mut self, database: &mut SqliteFrontier, plan: TailAdmissionPlan) {
        database.commit_required_frontier(plan.required_frontier);
        self.applied_through = plan.applied_through;
        self.applied_retained_bytes_total = plan.applied_retained_bytes_total;
        self.authoritative_through = plan.authoritative_through;
        self.authoritative_retained_bytes_total = plan.authoritative_retained_bytes_total;
        self.retained_bytes = plan.retained_bytes;
        self.descriptor_overflow = plan.descriptor_overflow;
        if let Some((sequence, descriptor)) = plan.descriptor {
            self.hot_descriptors.insert(sequence, descriptor);
        }
    }

    /// Direct bounded-accounting harness for the provider-cap regression.
    #[cfg(test)]
    fn record_authoritative_descriptor(
        &mut self,
        acceptance_sequence: u64,
        authoritative_retained_bytes_total: u64,
        descriptor: TailDescriptor,
    ) -> Result<bool, TailOverlayError> {
        if let Some(existing) = self.hot_descriptors.get(&acceptance_sequence) {
            return if existing == &descriptor {
                Ok(false)
            } else {
                Err(TailOverlayError::BatchCollision(descriptor.batch_id))
            };
        }
        let mut next_authoritative_through = self.authoritative_through;
        let mut next_authoritative_retained_bytes_total = self.authoritative_retained_bytes_total;
        let mut next_retained_bytes = self.retained_bytes;
        if acceptance_sequence > next_authoritative_through {
            next_authoritative_through = acceptance_sequence;
            next_authoritative_retained_bytes_total =
                authoritative_retained_bytes_total.max(next_authoritative_retained_bytes_total);
            let retained_bytes = next_authoritative_retained_bytes_total
                .checked_sub(self.applied_retained_bytes_total)
                .ok_or(ProjectionError::FrontierRegression)?;
            next_retained_bytes = usize::try_from(retained_bytes).map_err(|_| {
                ProjectionError::Corrupt(
                    "durable accepted backlog exceeds addressable memory".into(),
                )
            })?;
        }
        let insert =
            self.hot_descriptors.len() < TAIL_MAX_BATCHES && next_retained_bytes <= TAIL_MAX_BYTES;
        self.authoritative_through = next_authoritative_through;
        self.authoritative_retained_bytes_total = next_authoritative_retained_bytes_total;
        self.retained_bytes = next_retained_bytes;
        if insert {
            self.hot_descriptors.insert(acceptance_sequence, descriptor);
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
        if !self.runtime_authority.matches(&database.runtime_authority)
            || !self.runtime_authority.matches(&source.runtime_authority)
            || !source
                .runtime_authority
                .matches(source.engine.runtime_authority())
        {
            return Err(ProjectionError::AuthorityMismatch.into());
        }
        let authenticate_source =
            self.authenticated_source_frontier.as_ref() != Some(&source.exact_frontier_root);
        if authenticate_source {
            source.authenticate_exact_frontier()?;
        }
        let required_frontier = database.plan_required_frontier(&source.exact_frontier_root)?;

        // Once the bound exact source is authenticated, keep reads gated to
        // that root while its accepted prefix drains.
        database.commit_required_frontier(required_frontier);
        if authenticate_source {
            self.authenticated_source_frontier = Some(source.exact_frontier_root.clone());
        }
        // `accepted_batch_count` and `retained_bytes_total` are two halves of
        // the same authenticated source frontier and must advance together.
        // Raising only the sequence leaves the retained-bytes gauge behind the
        // events this drain is about to apply, and the first application then
        // underflows `applied - authoritative` as a false frontier regression.
        // This is reachable whenever an accepted event was never separately
        // enqueued — for example a locally authored batch staged straight into
        // the engine.
        self.authoritative_through = self.authoritative_through.max(source.accepted_batch_count);
        self.authoritative_retained_bytes_total = self
            .authoritative_retained_bytes_total
            .max(source.exact_frontier_root.retained_bytes_total());
        self.refresh_retained_bytes()?;
        let mut applied = 0;
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
            database.apply_engine_owned_accepted(&event, source.engine)?;
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
/// The projection's database-adjacent applier lock lives exactly as long as
/// this value, independent of the app-data root and projection database's file
/// name. On the compatibility entry points the value additionally retains its
/// own archive-rooted [`WorkspaceRuntimeLease`]; on the session-owned entry
/// points the caller's retained lease provides that authority instead and
/// outlives this value by construction (see [`LeasedSqliteFrontier`]).
/// A clean drop or process termination releases the OS locks; a later process
/// validates the database before reuse and rebuilds from engine/store evidence
/// when deletion, stale state, corruption, or an interrupted WAL is observed.
pub struct SqliteFrontier {
    path: PathBuf,
    claim: ProjectionClaim,
    physical: PhysicalSqliteDatabase,
    runtime_authority: EngineAuthority,
    required_frontier_root: AcceptedFrontierRoot,
    checkpoint_each_apply: bool,
    fresh_reference_coverage_count: Option<u64>,
    _lease: Arc<HeldApplierLocks>,
}

/// A [`SqliteFrontier`] opened under a caller-retained
/// [`WorkspaceRuntimeLease`], bound to the applier slot that authorized it.
///
/// The slot is *moved into* this value, so the database handle can never be
/// separated from the workspace authority that justified opening it, and the
/// borrow checker refuses to let either escape the lease. Closing the database
/// hands the same slot back, which is what makes a bootstrap -> promoted
/// database handoff possible without ever releasing the workspace lock.
pub(crate) struct LeasedSqliteFrontier<'lease> {
    database: SqliteFrontier,
    slot: SqliteApplierSlot<'lease>,
}

// The owning `LeasedWorkspaceProjection` is what activation uses; these two
// accessors serve the borrowed shape, which today only the applier-slot handoff
// regression exercises directly.
#[allow(dead_code)]
impl<'lease> LeasedSqliteFrontier<'lease> {
    pub(crate) const fn database(&self) -> &SqliteFrontier {
        &self.database
    }

    /// Close the session-owned database and return its applier slot to the same
    /// retained lease.
    ///
    /// The workspace lock is a distinct OS handle owned by the caller's lease
    /// and is never touched here, so there is no instant between closing this
    /// database and opening the next one in which another process could acquire
    /// the workspace lease.
    pub(crate) fn close_returning_applier_slot(self) -> SqliteApplierSlot<'lease> {
        let Self { database, slot } = self;
        drop(database);
        slot
    }
}

/// [`OpenProjection`] for the session-owned entry points.
pub(crate) struct LeasedOpenProjection<'lease> {
    pub(crate) database: LeasedSqliteFrontier<'lease>,
    pub(crate) recovery: ProjectionRecovery,
    pub(crate) rebuild: RebuildInstrumentation,
}

impl<'lease> LeasedOpenProjection<'lease> {
    /// Bind an opened projection to the exact slot that authorized its
    /// database-adjacent lock. Only the session-owned entry points call this,
    /// immediately after acquiring the lock through that same slot.
    fn bind(opened: OpenProjection, slot: SqliteApplierSlot<'lease>) -> Self {
        Self {
            database: LeasedSqliteFrontier {
                database: opened.database,
                slot,
            },
            recovery: opened.recovery,
            rebuild: opened.rebuild,
        }
    }

    /// Split the leased projection into the plain projection and the applier
    /// slot that authorized it.
    ///
    /// Only [`LeasedWorkspaceProjection::open_under`] calls this, and only to
    /// move the projection next to the very lease the slot borrows, so the two
    /// authorities stay inseparable across the split.
    fn into_parts(self) -> (OpenProjection, SqliteApplierSlot<'lease>) {
        let Self {
            database,
            recovery,
            rebuild,
        } = self;
        let LeasedSqliteFrontier { database, slot } = database;
        (
            OpenProjection {
                database,
                recovery,
                rebuild,
            },
            slot,
        )
    }
}

#[derive(Clone, Copy)]
enum ApplyFault {
    None,
    #[cfg(test)]
    ReturnAfterInsert,
    #[cfg(test)]
    ReturnAfterMaterialization,
    #[cfg(test)]
    AbortAfterInsert,
    #[cfg(test)]
    AbortAfterCommit,
}

thread_local! {
    // Crate-private deterministic simulator hook. It fires inside the same
    // transaction as row materialization, before the authoritative frontier
    // row moves, so returning an error proves rollback/reopen behavior at the
    // actual atomic SQLite boundary.
    static HARNESS_FAIL_DURING_APPLY: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

pub(crate) fn fail_next_apply_during_materialization_for_harness() {
    HARNESS_FAIL_DURING_APPLY.with(|fail| fail.set(true));
}

/// The three interruption boundaries of one terminal bootstrap construction.
///
/// They are the exact atomic edges the build crosses: the candidate
/// transaction's commit, the candidate file set's atomic publication, and the
/// projection checkpoint that finally proves the published file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalConstructionCut {
    BeforeCandidateCommit,
    AfterCandidateCommitBeforePublication,
    AfterPublicationBeforeCheckpointProof,
}

thread_local! {
    // Crate-private deterministic simulator hook, in the same shape as
    // `HARNESS_FAIL_DURING_APPLY` above: it fires at a real atomic boundary so
    // a harness observes the actual recovery behavior rather than a simulated
    // one.
    static HARNESS_TERMINAL_CONSTRUCTION_CUT: std::cell::Cell<Option<TerminalConstructionCut>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn fail_next_terminal_construction_at(cut: TerminalConstructionCut) {
    HARNESS_TERMINAL_CONSTRUCTION_CUT.with(|slot| slot.set(Some(cut)));
}

fn terminal_construction_cut(cut: TerminalConstructionCut) -> Result<(), ProjectionError> {
    HARNESS_TERMINAL_CONSTRUCTION_CUT.with(|slot| {
        if slot.get() == Some(cut) {
            slot.set(None);
            return Err(ProjectionError::InjectedFailure);
        }
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn refresh_projection_checkpoint_for_harness(
    path: &Path,
    claim: ProjectionClaim,
) -> Result<(), ProjectionError> {
    let physical = PhysicalSqliteDatabase::open_read_only(path)?;
    let root = read_frontier_root(&physical)?;
    drop(physical);
    write_projection_checkpoint(path, claim, &root)?;
    let files = SqliteFileSet::new(path);
    let wal_path = files.wal_path();
    if fs::symlink_metadata(&wal_path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == 0)
    {
        fs::remove_file(wal_path)?;
        let shm_path = files.shm_path();
        if fs::symlink_metadata(&shm_path).is_ok_and(|metadata| metadata.is_file()) {
            fs::remove_file(shm_path)?;
        }
    }
    Ok(())
}

fn fail_during_apply_for_harness() -> Result<(), ProjectionError> {
    HARNESS_FAIL_DURING_APPLY.with(|fail| {
        if fail.replace(false) {
            Err(ProjectionError::InjectedFailure)
        } else {
            Ok(())
        }
    })
}

impl SqliteFrontier {
    /// Re-authenticate the already-verified inactive-bootstrap projection for
    /// the uninterrupted promoted runtime without rebuilding it.
    ///
    /// The process token separately binds `proof.authority_binding()` to the
    /// exact promotion state and retained candidate. This check proves the
    /// reopened database still carries the same semantic rows, frontier, and
    /// reference-catalog authority before writable runtime authority exists.
    pub(crate) fn authenticate_same_process_bootstrap_reuse(
        &self,
        proof: &VerifiedBootstrapSqliteProjection,
    ) -> Result<(), ProjectionError> {
        let frontier = self.frontier_root()?;
        let accepted_batch_count = u64::try_from(self.applied_batch_count()?)
            .map_err(|_| ProjectionError::Rebuild("SQLite accepted count overflowed".into()))?;
        if self.claim != proof.claim()
            || frontier != *proof.frontier_root()
            || accepted_batch_count != proof.accepted_batch_count()
            || self.required_frontier_root != frontier
            || self.semantic_projection_digest()? != proof.semantic_projection_digest()
            || self.materialized_row_digest_for_harness()? != proof.materialized_row_digest()
            || (accepted_batch_count != 0
                && self.authenticated_reference_catalog_root()?
                    != *frontier.reference_catalog_root())
        {
            return Err(ProjectionError::Rebuild(
                "promoted SQLite reopen differs from retained bootstrap proof".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn freshly_verify_inactive_bootstrap(
        &self,
        authority: &InactiveBootstrapAcceptedAuthority,
        proof: &VerifiedBootstrapSqliteProjection,
    ) -> Result<(), ProjectionError> {
        let binding = authority.binding();
        let frontier_root = self.frontier_root()?;
        let accepted_batch_count = u64::try_from(self.applied_batch_count()?)
            .map_err(|_| ProjectionError::Rebuild("SQLite accepted count overflowed".into()))?;
        let materialized = self.materialized_read()?;
        validate_projection_checkpoint(&self.path, self.claim, &frontier_root)?;
        if proof.authority_binding() != binding
            || self.claim() != proof.claim()
            || proof.claim()
                != ProjectionClaim::current(binding.workspace_id(), binding.lineage_digest())
            || frontier_root != *proof.frontier_root()
            || frontier_root != *binding.accepted_frontier()
            || accepted_batch_count != proof.accepted_batch_count()
            || accepted_batch_count != u64::from(binding.part_count())
            || self.required_frontier_root != frontier_root
            || !self
                .runtime_authority
                .matches(authority.accepted_engine().runtime_authority())
            || materialized.acceptance_sequence() != accepted_batch_count
            || self.semantic_projection_digest()? != proof.semantic_projection_digest()
            || self.materialized_row_digest_for_harness()? != proof.materialized_row_digest()
        {
            return Err(ProjectionError::Rebuild(
                "fresh SQLite evidence differs from inactive bootstrap proof".into(),
            ));
        }
        if accepted_batch_count != 0
            && self.authenticated_reference_catalog_root()?
                != *frontier_root.reference_catalog_root()
        {
            return Err(ProjectionError::Rebuild(
                "fresh SQLite reference catalog differs from bootstrap authority".into(),
            ));
        }
        Ok(())
    }

    /// Test-only entry point: acquires a temporary [`WorkspaceRuntimeLease`]
    /// internally and keeps it inside the returned projection.
    ///
    /// It is `#[cfg(test)]` deliberately, and the gate is load bearing rather
    /// than tidy-up. This shape releases the workspace lock when the projection
    /// drops, so an activation that used it would have to release and reacquire
    /// the archive across the bootstrap -> promoted database handoff — exactly
    /// the window `local_active::InactiveBootstrapRuntimeSession` exists to make
    /// inexpressible. Its non-test call census is zero (every caller is inside
    /// `import`'s and `shadow_projection`'s `#[cfg(test)] mod tests`, and none
    /// promotes), so the gate turns "do not call this from activation code" from
    /// a convention into a compile error. Production activation wiring must use
    /// [`Self::open_or_rebuild_inactive_bootstrap_with_applier_slot`] through
    /// `InactiveBootstrapRuntimeSession`.
    #[cfg(test)]
    pub(crate) fn open_or_rebuild_inactive_bootstrap(
        path: &Path,
        application_runtime_root: &ApplicationRuntimeRoot,
        authority: &InactiveBootstrapAcceptedAuthority,
    ) -> Result<(OpenProjection, VerifiedBootstrapSqliteProjection), ProjectionError> {
        Self::open_or_rebuild_inactive_bootstrap_authorized(
            path,
            application_runtime_root,
            authority,
            &ApplierAuthorization::OwnWorkspaceLease,
            None,
        )
    }

    /// Test-only twin that additionally consumes retained terminal
    /// construction material, so SQLite-level tests can exercise both build
    /// paths over one authority.
    #[cfg(test)]
    pub(crate) fn open_or_rebuild_inactive_bootstrap_terminally(
        path: &Path,
        application_runtime_root: &ApplicationRuntimeRoot,
        authority: &InactiveBootstrapAcceptedAuthority,
        terminal: &TerminalBootstrapConstructionMaterial,
    ) -> Result<(OpenProjection, VerifiedBootstrapSqliteProjection), ProjectionError> {
        Self::open_or_rebuild_inactive_bootstrap_authorized(
            path,
            application_runtime_root,
            authority,
            &ApplierAuthorization::OwnWorkspaceLease,
            Some(terminal),
        )
    }

    /// Session-owned entry point: consumes the caller's applier slot and binds
    /// it to the opened database, so the retained workspace lease is never
    /// released between this database and the next one opened from the same
    /// slot.
    // This is the entry point `local_active::InactiveBootstrapRuntimeSession`
    // uses, and therefore the one every activation takes. The compatibility twin
    // above survives only for SQLite-level tests that open a bootstrap database
    // and never promote it.
    pub(crate) fn open_or_rebuild_inactive_bootstrap_with_applier_slot<'lease>(
        path: &Path,
        application_runtime_root: &ApplicationRuntimeRoot,
        authority: &InactiveBootstrapAcceptedAuthority,
        slot: SqliteApplierSlot<'lease>,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> Result<
        (
            LeasedOpenProjection<'lease>,
            VerifiedBootstrapSqliteProjection,
        ),
        ProjectionError,
    > {
        let (opened, proof) = Self::open_or_rebuild_inactive_bootstrap_authorized(
            path,
            application_runtime_root,
            authority,
            &ApplierAuthorization::Slot(&slot),
            terminal,
        )?;
        Ok((LeasedOpenProjection::bind(opened, slot), proof))
    }

    fn open_or_rebuild_inactive_bootstrap_authorized(
        path: &Path,
        _application_runtime_root: &ApplicationRuntimeRoot,
        authority: &InactiveBootstrapAcceptedAuthority,
        authorization: &ApplierAuthorization<'_, '_>,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> Result<(OpenProjection, VerifiedBootstrapSqliteProjection), ProjectionError> {
        let binding = authority.binding();
        let claim = ProjectionClaim::current(binding.workspace_id(), binding.lineage_digest());
        let source = RebuildSource::from_inactive_bootstrap(authority)?;
        let (opened, bootstrap_rebuild) =
            Self::rebuild_fresh_inactive_bootstrap(path, claim, source, authorization, terminal)?;
        let frontier_root = opened.database.frontier_root()?;
        let accepted_batch_count = u64::try_from(opened.database.applied_batch_count()?)
            .map_err(|_| ProjectionError::Rebuild("SQLite accepted count overflowed".into()))?;
        if opened.database.claim() != claim
            || frontier_root != *binding.accepted_frontier()
            || accepted_batch_count != u64::from(binding.part_count())
            || opened.database.required_frontier_root != frontier_root
            || !opened
                .database
                .runtime_authority
                .matches(authority.accepted_engine().runtime_authority())
        {
            return Err(ProjectionError::Rebuild(
                "SQLite projection does not agree with inactive bootstrap authority".into(),
            ));
        }
        let materialized = opened.database.materialized_read()?;
        if materialized.acceptance_sequence() != accepted_batch_count {
            return Err(ProjectionError::Rebuild(
                "SQLite materialization does not agree with inactive bootstrap authority".into(),
            ));
        }
        if accepted_batch_count != 0
            && opened.database.authenticated_reference_catalog_root()?
                != *frontier_root.reference_catalog_root()
        {
            return Err(ProjectionError::Rebuild(
                "SQLite reference catalog does not agree with inactive bootstrap authority".into(),
            ));
        }
        let semantic_projection_digest = opened.database.semantic_projection_digest()?;
        let materialized_row_digest = opened.database.materialized_row_digest_for_harness()?;
        let proof = VerifiedBootstrapSqliteProjection {
            claim,
            frontier_root,
            accepted_batch_count,
            semantic_projection_digest,
            materialized_row_digest,
            authority_binding: binding.clone(),
            bootstrap_rebuild,
        };
        Ok((opened, proof))
    }

    fn rebuild_fresh_inactive_bootstrap(
        path: &Path,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
        authorization: &ApplierAuthorization<'_, '_>,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> Result<(OpenProjection, BootstrapSqliteRebuildInstrumentation), ProjectionError> {
        validate_source(claim, &source)?;
        source.authenticate_exact_frontier()?;
        let path = prepare_database_path(path)?;
        let lease = authorization.acquire(source.store, &path, claim.workspace_id)?;
        let mut pending_forensics = resume_pending_forensics(&path)?;
        let existed = projection_files_exist(&path);
        if existed {
            pending_forensics.extend(preserve_forensics(&path)?);
            maybe_abort_forensic_test("before-rebuild", 0);
        }
        let (database, rebuild, bootstrap_rebuild) =
            Self::build_candidate_and_publish(&path, claim, lease, &source, terminal)?;
        if !pending_forensics.directories.is_empty() {
            mark_rebuild_complete(&pending_forensics)?;
            return Ok((
                OpenProjection {
                    database,
                    recovery: ProjectionRecovery::RebuiltPreservingEvidence {
                        reason: if existed {
                            "bootstrap verification requires a fresh authority-derived rebuild"
                                .into()
                        } else {
                            "resumed interrupted forensic preservation and bootstrap rebuild".into()
                        },
                        evidence: pending_forensics.evidence,
                        applied_batches: rebuild.accepted_events_applied,
                    },
                    rebuild,
                },
                bootstrap_rebuild,
            ));
        }
        Ok((
            OpenProjection {
                database,
                recovery: ProjectionRecovery::RebuiltMissing {
                    applied_batches: rebuild.accepted_events_applied,
                },
                rebuild,
            },
            bootstrap_rebuild,
        ))
    }

    /// Compatibility entry point: acquires a temporary
    /// [`WorkspaceRuntimeLease`] internally and keeps it inside the returned
    /// projection, so current callers and behavior are unchanged.
    pub fn open_or_rebuild(
        path: &Path,
        application_runtime_root: &ApplicationRuntimeRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
    ) -> Result<OpenProjection, ProjectionError> {
        Self::open_or_rebuild_authorized(
            path,
            application_runtime_root,
            claim,
            source,
            &ApplierAuthorization::OwnWorkspaceLease,
        )
    }

    /// Session-owned entry point: consumes the caller's applier slot and binds
    /// it to the opened database, so the retained workspace lease is never
    /// released between this database and the next one opened from the same
    /// slot.
    pub(crate) fn open_or_rebuild_with_applier_slot<'lease>(
        path: &Path,
        application_runtime_root: &ApplicationRuntimeRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
        slot: SqliteApplierSlot<'lease>,
    ) -> Result<LeasedOpenProjection<'lease>, ProjectionError> {
        let opened = Self::open_or_rebuild_authorized(
            path,
            application_runtime_root,
            claim,
            source,
            &ApplierAuthorization::Slot(&slot),
        )?;
        Ok(LeasedOpenProjection::bind(opened, slot))
    }

    /// Runtime-host reopen of one exact existing projection.
    ///
    /// Missing, invalid, divergent, or forensic-interrupted state is refused.
    /// This boundary never preserves, deletes, publishes, or rebuilds files.
    pub(crate) fn open_existing_with_applier_slot<'lease>(
        path: &Path,
        _application_runtime_root: &ApplicationRuntimeRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
        slot: SqliteApplierSlot<'lease>,
    ) -> Result<LeasedOpenProjection<'lease>, ProjectionError> {
        validate_source(claim, &source)?;
        source.authenticate_exact_frontier()?;
        let path = require_existing_database_path(path)?;
        let lease =
            ApplierAuthorization::Slot(&slot).acquire(source.store, &path, claim.workspace_id)?;
        if incomplete_forensic_recovery_exists(&path)? {
            return Err(ProjectionError::Corrupt(
                "existing projection has interrupted forensic recovery".into(),
            ));
        }
        validate_existing(&path, claim, &source).map_err(ProjectionError::Corrupt)?;
        let physical = PhysicalSqliteDatabase::open_writable(&path)?;
        Ok(LeasedOpenProjection::bind(
            OpenProjection {
                database: Self {
                    path,
                    claim,
                    physical,
                    runtime_authority: source.runtime_authority.clone(),
                    required_frontier_root: source.exact_frontier_root.clone(),
                    checkpoint_each_apply: true,
                    fresh_reference_coverage_count: None,
                    _lease: lease,
                },
                recovery: ProjectionRecovery::OpenedExisting,
                rebuild: RebuildInstrumentation::default(),
            },
            slot,
        ))
    }

    fn open_or_rebuild_authorized(
        path: &Path,
        _application_runtime_root: &ApplicationRuntimeRoot,
        claim: ProjectionClaim,
        source: RebuildSource<'_>,
        authorization: &ApplierAuthorization<'_, '_>,
    ) -> Result<OpenProjection, ProjectionError> {
        validate_source(claim, &source)?;
        source.authenticate_exact_frontier()?;
        let path = prepare_database_path(path)?;
        let lease = authorization.acquire(source.store, &path, claim.workspace_id)?;
        let mut pending_forensics = resume_pending_forensics(&path)?;
        let existed = projection_files_exist(&path);

        if existed {
            match validate_existing(&path, claim, &source) {
                Ok(()) => {
                    if !pending_forensics.directories.is_empty() {
                        mark_rebuild_complete(&pending_forensics)?;
                        let physical = PhysicalSqliteDatabase::open_writable(&path)?;
                        return Ok(OpenProjection {
                            database: Self {
                                path,
                                claim,
                                physical,
                                runtime_authority: source.runtime_authority.clone(),
                                required_frontier_root: source.exact_frontier_root.clone(),
                                checkpoint_each_apply: true,
                                fresh_reference_coverage_count: None,
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
                    let physical = PhysicalSqliteDatabase::open_writable(&path)?;
                    return Ok(OpenProjection {
                        database: Self {
                            path,
                            claim,
                            physical,
                            runtime_authority: source.runtime_authority.clone(),
                            required_frontier_root: source.exact_frontier_root.clone(),
                            checkpoint_each_apply: true,
                            fresh_reference_coverage_count: None,
                            _lease: lease,
                        },
                        recovery: ProjectionRecovery::OpenedExisting,
                        rebuild: RebuildInstrumentation::default(),
                    });
                }
                Err(reason) => {
                    pending_forensics.extend(preserve_forensics(&path)?);
                    maybe_abort_forensic_test("before-rebuild", 0);
                    let (database, rebuild, _) =
                        Self::build_candidate_and_publish(&path, claim, lease, &source, None)?;
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

        let (database, rebuild, _) =
            Self::build_candidate_and_publish(&path, claim, lease, &source, None)?;
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
        lease: Arc<HeldApplierLocks>,
        source: &RebuildSource<'_>,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> Result<
        (
            Self,
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
        ProjectionError,
    > {
        // The retained terminal material is an optimization capability, never
        // an authority. If it is absent, refuses to bind, or fails part way
        // through, the private candidate is discarded and this build falls back
        // to the unchanged archive replay path over the same durable evidence.
        let mut refused = 0_usize;
        if terminal.is_some() {
            match Self::build_candidate(path, claim, Arc::clone(&lease), source, terminal) {
                Ok(built) => return Self::publish_candidate(path, claim, lease, source, built),
                Err(_discarded) => refused = 1,
            }
        }
        let mut built = Self::build_candidate(path, claim, Arc::clone(&lease), source, None)?;
        built.2.terminal_construction_refusals = refused;
        Self::publish_candidate(path, claim, lease, source, built)
    }

    fn build_candidate(
        path: &Path,
        claim: ProjectionClaim,
        lease: Arc<HeldApplierLocks>,
        source: &RebuildSource<'_>,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> Result<
        (
            SqliteFileSet,
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
        ProjectionError,
    > {
        let candidate_files = SqliteFileSet::prepare_candidate(path)?;
        let candidate_path = candidate_files.database_path().to_path_buf();
        let mut candidate = Self::create_new(
            &candidate_path,
            claim,
            lease,
            source.runtime_authority.clone(),
        )?;
        candidate.require_frontier(&source.exact_frontier_root)?;
        let streamed = match terminal {
            Some(material) => candidate.terminal_stream(source, material),
            None => candidate.rebuild_stream(source),
        };
        let (rebuild, bootstrap_rebuild) = match streamed {
            Ok(rebuild) => rebuild,
            Err(error) => {
                drop(candidate);
                candidate_files.remove()?;
                return Err(error);
            }
        };
        let checkpointed = candidate
            .physical
            .checkpoint_truncate_and_disable_wal()
            .map_err(ProjectionError::from)
            .and_then(|()| {
                terminal_construction_cut(
                    TerminalConstructionCut::AfterCandidateCommitBeforePublication,
                )
            });
        if let Err(error) = checkpointed {
            drop(candidate);
            candidate_files.remove()?;
            return Err(error);
        }
        drop(candidate);
        Ok((candidate_files, rebuild, bootstrap_rebuild))
    }

    fn publish_candidate(
        path: &Path,
        claim: ProjectionClaim,
        lease: Arc<HeldApplierLocks>,
        source: &RebuildSource<'_>,
        built: (
            SqliteFileSet,
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
    ) -> Result<
        (
            Self,
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
        ProjectionError,
    > {
        let (candidate_files, rebuild, bootstrap_rebuild) = built;
        candidate_files.publish_candidate(path)?;
        terminal_construction_cut(TerminalConstructionCut::AfterPublicationBeforeCheckpointProof)?;
        let physical = PhysicalSqliteDatabase::open_writable(path)?;
        let root = read_frontier_root(&physical)?;
        write_projection_checkpoint(path, claim, &root)?;
        Ok((
            Self {
                path: path.to_path_buf(),
                claim,
                physical,
                runtime_authority: source.runtime_authority.clone(),
                required_frontier_root: source.exact_frontier_root.clone(),
                checkpoint_each_apply: true,
                fresh_reference_coverage_count: None,
                _lease: lease,
            },
            rebuild,
            bootstrap_rebuild,
        ))
    }

    fn create_new(
        path: &Path,
        claim: ProjectionClaim,
        lease: Arc<HeldApplierLocks>,
        runtime_authority: EngineAuthority,
    ) -> Result<Self, ProjectionError> {
        let physical = PhysicalSqliteDatabase::open_writable(path)?;
        initialize_schema(&physical, claim)?;
        let root = read_frontier_root(&physical)?;
        write_projection_checkpoint(path, claim, &root)?;
        Ok(Self {
            path: path.to_path_buf(),
            claim,
            physical,
            runtime_authority,
            required_frontier_root: AcceptedFrontierRoot::empty(),
            checkpoint_each_apply: false,
            fresh_reference_coverage_count: Some(0),
            _lease: lease,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn claim(&self) -> ProjectionClaim {
        self.claim
    }

    fn plan_required_frontier(
        &self,
        root: &AcceptedFrontierRoot,
    ) -> Result<RequiredFrontierTransition, ProjectionError> {
        canonical_frontier_root_bytes(root)?;
        let replacement = match root
            .acceptance_sequence()
            .cmp(&self.required_frontier_root.acceptance_sequence())
        {
            std::cmp::Ordering::Greater => Some(root.clone()),
            std::cmp::Ordering::Equal if root == &self.required_frontier_root => None,
            std::cmp::Ordering::Equal => {
                return Err(ProjectionError::Corrupt(
                    "two accepted frontier roots claim the same required sequence".into(),
                ));
            }
            std::cmp::Ordering::Less => None,
        };
        Ok(RequiredFrontierTransition { replacement })
    }

    fn commit_required_frontier(&mut self, transition: RequiredFrontierTransition) {
        if let Some(root) = transition.replacement {
            self.required_frontier_root = root;
        }
    }

    fn require_frontier(&mut self, root: &AcceptedFrontierRoot) -> Result<(), ProjectionError> {
        let transition = self.plan_required_frontier(root)?;
        self.commit_required_frontier(transition);
        Ok(())
    }

    fn validate_applied_tail_duplicate(
        &self,
        event: &AcceptedBatchEvent,
        current_root: &AcceptedFrontierRoot,
    ) -> Result<(), ProjectionError> {
        self.validate_event_claim(event)?;
        if let Some(existing) = load_batch(&self.physical, event.batch_id)? {
            if !self.physical.authenticate_batch(
                &lower_physical_frontier_root(current_root)?,
                event.batch_id.as_uuid().into_bytes(),
                existing.causal_record_digest()?,
            )? {
                return Err(ProjectionError::Corrupt(format!(
                    "stored batch {} is absent from the authenticated accepted map",
                    event.batch_id
                )));
            }
            if !existing.matches(event)? {
                return Err(ProjectionError::BatchCollision(event.batch_id));
            }
            if current_root.acceptance_sequence() >= event.acceptance_sequence
                && current_root.state_digest() != event.prior_frontier_root.state_digest()
            {
                return Ok(());
            }
            return Err(ProjectionError::FrontierRegression);
        }
        let expected = current_root
            .acceptance_sequence()
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("applied batch sequence overflowed".into()))?;
        Err(ProjectionError::AcceptanceOrder {
            expected,
            found: event.acceptance_sequence,
        })
    }

    pub fn frontier_root(&self) -> Result<AcceptedFrontierRoot, ProjectionError> {
        read_frontier_root(&self.physical)
    }

    /// Explicit whole-frontier materialization for diagnostics and recovery.
    /// Normal apply, startup, and point authorization use `frontier_root` and
    /// `contains_frontier` instead.
    pub fn frontier(&self) -> Result<FrontierV2, ProjectionError> {
        read_frontier_documents(&self.physical)
    }

    pub fn contains_frontier(&self, required: &FrontierV2) -> Result<bool, ProjectionError> {
        canonical_frontier_bytes(required)?;
        let root = read_frontier_root(&self.physical)?;
        for needed in required.documents() {
            let Some(have) =
                authenticated_frontier_document(&self.physical, &root, needed.document_id())?
            else {
                return Ok(false);
            };
            if !document_frontier_contains(&self.physical, &have, needed)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn applied_batch_count(&self) -> Result<usize, ProjectionError> {
        usize::try_from(read_frontier_root(&self.physical)?.acceptance_sequence())
            .map_err(|_| ProjectionError::Corrupt("applied sequence exceeds usize".into()))
    }

    /// Explicit full diagnostic. Normal startup and apply never call this
    /// lifetime-history scan.
    pub fn diagnose_full_integrity(&self) -> Result<(), ProjectionError> {
        self.physical.quick_check()?;
        let (applied_rows, document_rows) = self.physical.diagnostic_row_counts()?;
        let root = read_frontier_root(&self.physical)?;
        if applied_rows != root.acceptance_sequence() || document_rows != root.document_count() {
            return Err(ProjectionError::Corrupt(
                "SQLite diagnostic row counts differ from the authenticated frontier".into(),
            ));
        }
        let (history_root, history_count) = validate_stored_history(&self.physical)?;
        if history_count != root.acceptance_sequence() || history_root != root {
            return Err(ProjectionError::Corrupt(
                "SQLite diagnostic history scan differs from the authenticated frontier".into(),
            ));
        }
        let _ = read_frontier_documents(&self.physical)?;
        Ok(())
    }

    pub fn contains_batch(&self, batch_id: BatchId) -> Result<bool, ProjectionError> {
        let root = read_frontier_root(&self.physical)?;
        self.physical
            .contains_batch(
                &lower_physical_frontier_root(&root)?,
                batch_id.as_uuid().into_bytes(),
            )
            .map_err(Into::into)
    }

    #[cfg(test)]
    fn apply_accepted(
        &mut self,
        event: &AcceptedBatchEvent,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_internal(event, ApplyFault::None)
    }

    #[cfg(test)]
    fn apply_materialized_accepted(
        &mut self,
        event: &AcceptedBatchEvent,
        materialization: &super::MaterializationChange,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_internal_with_materialization(event, ApplyFault::None, Some(materialization))
    }

    /// Apply one accepted event using only projection data derived by the
    /// bound engine at the event's authenticated accepted root.
    fn apply_engine_owned_accepted(
        &mut self,
        event: &AcceptedBatchEvent,
        engine: &ShardedHotEngine,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_engine_owned_accepted_with_stats(event, engine)
            .map(|(disposition, _, _)| disposition)
    }

    fn apply_engine_owned_accepted_with_stats(
        &mut self,
        event: &AcceptedBatchEvent,
        engine: &ShardedHotEngine,
    ) -> Result<
        (
            ApplyDisposition,
            EventMaterializationInstrumentation,
            super::sqlite_materialization::ApplyChangeInstrumentation,
        ),
        ProjectionError,
    > {
        if !self.runtime_authority.matches(engine.runtime_authority()) {
            return Err(ProjectionError::AuthorityMismatch);
        }
        let (materialization, materialization_stats) =
            materialize_accepted_event_with_stats(engine, event)?;
        let materialization =
            attach_authenticated_reference_catalog(engine, event, materialization)?;
        let (disposition, apply_stats) = self.apply_internal_with_materialization_and_stats(
            event,
            ApplyFault::None,
            Some(&materialization),
        )?;
        Ok((disposition, materialization_stats, apply_stats))
    }

    #[cfg(test)]
    fn apply_authenticated_reference_catalog_materialized_accepted(
        &mut self,
        event: &AcceptedBatchEvent,
        materialization: super::MaterializationChange,
        engine: &ShardedHotEngine,
    ) -> Result<ApplyDisposition, ProjectionError> {
        if !self.runtime_authority.matches(engine.runtime_authority()) {
            return Err(ProjectionError::AuthorityMismatch);
        }
        authenticate_event_for_engine(engine, event)?;
        let materialization =
            attach_authenticated_reference_catalog(engine, event, materialization)?;
        self.apply_internal_with_materialization(event, ApplyFault::None, Some(&materialization))
    }

    pub fn materialized_read(&self) -> Result<super::SqliteMaterializedRead<'_>, ProjectionError> {
        let root = read_frontier_root(&self.physical)?;
        if root != self.required_frontier_root {
            return Err(ProjectionError::Materialization(format!(
                "SQLite materialization frontier {} is behind required accepted frontier {}",
                root.acceptance_sequence(),
                self.required_frontier_root.acceptance_sequence()
            )));
        }
        let root_bytes = canonical_frontier_root_bytes(&root)?;
        self.physical
            .materialized_read(root.acceptance_sequence(), ContentDigest::of(&root_bytes))
            .map(super::SqliteMaterializedRead::from_storage)
            .map_err(Into::into)
    }

    pub(crate) fn authenticated_reference_catalog_root(
        &self,
    ) -> Result<super::ReferenceCatalogRootV2, ProjectionError> {
        let frontier = read_frontier_root(&self.physical)?;
        let frontier_bytes = canonical_frontier_root_bytes(&frontier)?;
        let stamp = self.physical.reference_catalog_stamp()?;
        if u64::try_from(stamp.acceptance_sequence).ok() != Some(frontier.acceptance_sequence())
            || stamp.frontier_root_digest.as_slice()
                != ContentDigest::of(&frontier_bytes).as_bytes()
        {
            return Err(ProjectionError::Materialization(
                "reference materialization stamp is stale against SQLite frontier".into(),
            ));
        }
        let (Some(root_bytes), Some(root_digest), Some(coverage_digest), Some(stamp_digest)) = (
            stamp.catalog_root,
            stamp.catalog_root_digest,
            stamp.coverage_digest,
            stamp.extractor_dependency_stamp_digest,
        ) else {
            return Err(ProjectionError::Materialization(
                "SQLite frontier has no authenticated reference catalog materialization".into(),
            ));
        };
        let root = super::ReferenceCatalogRootV2::decode(&root_bytes)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        let expected = frontier.reference_catalog_root();
        if &root != expected
            || root_digest.as_slice()
                != root
                    .external_digest()
                    .map_err(|error| ProjectionError::Materialization(error.to_string()))?
                    .as_bytes()
            || coverage_digest.as_slice() != root.source_coverage_root().as_bytes()
            || stamp_digest.as_slice()
                != super::sqlite_materialization::ReferenceExtractorDependencyStamp::new(
                    root.extractor_digest(),
                    root.policy_digest(),
                )?
                .digest()?
                .as_bytes()
        {
            return Err(ProjectionError::Materialization(
                "SQLite reference catalog stamp is not bound to its authenticated frontier".into(),
            ));
        }
        let coverage_count = self.physical.reference_source_coverage_count()?;
        if u64::try_from(coverage_count).ok() != Some(root.source_count()) {
            return Err(ProjectionError::Materialization(
                "SQLite reference source coverage is incomplete for its catalog root".into(),
            ));
        }
        Ok(root)
    }

    /// Rebuild only the disposable graph-wide rows from a streaming sequence
    /// of authoritative, oplog-derived materialization inputs.
    ///
    /// The accepted frontier/history is not changed. Each input is checked
    /// against the stored accepted semantic effect at the same sequence and is
    /// committed with its materialization stamp. Until the final sequence is
    /// present, [`Self::materialized_read`] fails closed as stale.
    #[cfg(test)]
    fn rebuild_materialization<I>(&mut self, changes: I) -> Result<usize, ProjectionError>
    where
        I: IntoIterator<Item = super::MaterializationChange>,
    {
        self.rebuild_materialization_inner(changes, None)
    }

    /// Rebuild a disposable materialization while deriving every reference
    /// row afresh from the catalog roots bound to accepted history.  Generic
    /// page/search facets still come from the supplied projection inputs;
    /// reference facts never do.
    #[cfg(test)]
    fn rebuild_authenticated_reference_catalog_materialization<I>(
        &mut self,
        changes: I,
        engine: &ShardedHotEngine,
    ) -> Result<usize, ProjectionError>
    where
        I: IntoIterator<Item = super::MaterializationChange>,
    {
        if !self.runtime_authority.matches(engine.runtime_authority()) {
            return Err(ProjectionError::AuthorityMismatch);
        }
        self.rebuild_materialization_inner(changes, Some(engine))
    }

    #[cfg(test)]
    fn rebuild_materialization_inner<I>(
        &mut self,
        changes: I,
        reference_engine: Option<&ShardedHotEngine>,
    ) -> Result<usize, ProjectionError>
    where
        I: IntoIterator<Item = super::MaterializationChange>,
    {
        let empty_root = canonical_frontier_root_bytes(&AcceptedFrontierRoot::empty())?;
        self.physical
            .reset_materialization_for_test(ContentDigest::of(&empty_root))?;

        let expected_count = read_frontier_root(&self.physical)?.acceptance_sequence();
        let mut changes = changes.into_iter();
        let mut applied = 0_u64;
        for sequence in 1..=expected_count {
            let Some(mut change) = changes.next() else {
                return Err(ProjectionError::Materialization(format!(
                    "materialization rebuild ended before accepted sequence {sequence}"
                )));
            };
            let sequence_i64 = i64::try_from(sequence)
                .map_err(|_| ProjectionError::Corrupt("accepted sequence exceeds SQLite".into()))?;
            let stored =
                load_batch_at_sequence(&self.physical, sequence_i64)?.ok_or_else(|| {
                    ProjectionError::Corrupt(format!(
                        "accepted history is missing materialization sequence {sequence}"
                    ))
                })?;
            if let Some(engine) = reference_engine {
                let prior = decode_frontier_root(&stored.prior_frontier_root)?;
                let post = decode_frontier_root(&stored.post_frontier_root)?;
                change = attach_authenticated_reference_catalog_at(
                    engine,
                    &stored.semantic_effect,
                    &prior,
                    &post,
                    change.without_reference_catalog()?,
                )?;
            } else if change.reference_catalog().is_some() {
                return Err(ProjectionError::Materialization(
                    "reference catalog rebuild input requires authenticated engine authority"
                        .into(),
                ));
            }
            let input_digest =
                change.validate_against_stored(stored.batch_id, &stored.semantic_effect)?;
            let prior_digest = decode_content_digest(&stored.prior_frontier_root_digest)?;
            let post_digest = decode_content_digest(&stored.post_frontier_root_digest)?;
            let authenticated_reference = change
                .reference_catalog()
                .is_some()
                .then(|| stored.authenticated_reference_materialization())
                .transpose()?;
            self.physical
                .ensure_materialization_stamp(sequence - 1, prior_digest)?;
            let (physical, authenticated_reference) =
                super::sqlite_materialization::lower_validated_change(
                    &change,
                    &stored.semantic_effect,
                    authenticated_reference.as_ref(),
                )?;
            self.physical.apply_materialization_for_test(
                &physical,
                sequence,
                input_digest,
                post_digest,
                authenticated_reference.as_ref(),
            )?;
            applied = sequence;
        }
        if let Some(extra) = changes.next() {
            return Err(ProjectionError::Materialization(format!(
                "materialization rebuild supplied extra batch {}",
                extra.batch_id()
            )));
        }
        let root = read_frontier_root(&self.physical)?;
        let root_bytes = canonical_frontier_root_bytes(&root)?;
        self.physical
            .ensure_materialization_stamp(expected_count, ContentDigest::of(&root_bytes))?;
        usize::try_from(applied)
            .map_err(|_| ProjectionError::Corrupt("materialized sequence exceeds usize".into()))
    }

    pub fn semantic_projection_digest(&self) -> Result<ContentDigest, ProjectionError> {
        self.physical
            .semantic_projection_digest()
            .map_err(Into::into)
    }

    /// Exact materialized-row observation for the deterministic simulator.
    /// Requiring `materialized_read` first keeps this diagnostic behind the
    /// same stale-frontier read gate as production consumers.
    pub(crate) fn materialized_row_digest_for_harness(
        &self,
    ) -> Result<ContentDigest, ProjectionError> {
        let _gate = self.materialized_read()?;
        self.physical.materialized_row_digest().map_err(Into::into)
    }

    /// Test-only complete per-table row observation, used to compare two
    /// independently built projections table by table.
    #[cfg(test)]
    pub(crate) fn materialized_row_digests_by_table_for_test(
        &self,
    ) -> Result<Vec<(&'static str, ContentDigest)>, ProjectionError> {
        let _gate = self.materialized_read()?;
        self.physical
            .materialized_row_digests_by_table()
            .map_err(Into::into)
    }

    /// Test-only recovery inspection of the exact semantic records rebuilt into
    /// this frontier. Production consumers retain only the authenticated
    /// frontier APIs above.
    #[cfg(test)]
    pub(crate) fn applied_semantic_effects_for_test(
        &self,
    ) -> Result<Vec<SemanticEffect>, ProjectionError> {
        self.physical
            .stored_semantic_effects()?
            .into_iter()
            .map(|bytes| {
                SemanticEffect::decode(&bytes)
                    .map_err(|error| ProjectionError::Corrupt(error.to_string()))
            })
            .collect()
    }

    /// Seed the fresh inactive-bootstrap candidate from the retained terminal
    /// accepted state.
    ///
    /// Each authored part still contributes its complete authenticated
    /// accepted-prefix rows, in order, under the same event authentication the
    /// replay path performs — but no intermediate page or reference
    /// replacement is applied. Exactly one terminal page/block/facet/search and
    /// reference-catalog materialization then seeds the same unchanged schema
    /// inside the same candidate transaction.
    fn terminal_stream(
        &mut self,
        source: &RebuildSource<'_>,
        material: &TerminalBootstrapConstructionMaterial,
    ) -> Result<
        (
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
        ProjectionError,
    > {
        let RebuildLoader::InactiveBootstrap { publication } = source.loader else {
            return Err(ProjectionError::Rebuild(
                "terminal construction is only defined for a fresh inactive bootstrap".into(),
            ));
        };
        validate_terminal_construction_material(source, publication, material)?;
        let engine = source.engine;
        let mut instrumentation = RebuildInstrumentation::default();
        let mut bootstrap = BootstrapSqliteRebuildInstrumentation {
            terminal_constructions: 1,
            ..BootstrapSqliteRebuildInstrumentation::default()
        };
        let writes_before = self.physical.write_instrumentation();
        self.physical.begin_candidate_build()?;
        self.physical.begin_terminal_bootstrap_construction()?;
        let prefix_started = std::time::Instant::now();
        let mut provenance = Vec::with_capacity(material.accepted_events().len());
        for event in material.accepted_events() {
            instrumentation.accepted_events_validated += 1;
            instrumentation.max_live_events = instrumentation.max_live_events.max(1);
            instrumentation.max_live_evidence_records =
                instrumentation.max_live_evidence_records.max(1);
            authenticate_event_for_engine(engine, event)?;
            let (_, apply_stats) =
                self.apply_candidate_with_materialization_and_stats(event, ApplyFault::None, None)?;
            instrumentation.cleanup_page_attempts += apply_stats.cleanup_page_attempts;
            instrumentation.cleanup_existing_pages += apply_stats.cleanup_existing_pages;
            instrumentation.cleanup_owned_rows += apply_stats.cleanup_owned_rows;
            instrumentation.cleanup_fts_rowids += apply_stats.cleanup_fts_rowids;
            instrumentation.reference_coverage_inductive_checks +=
                apply_stats.reference_coverage_inductive_checks;
            instrumentation.reference_coverage_full_scans +=
                apply_stats.reference_coverage_full_scans;
            // Construction provenance for one accepted sequence whose page and
            // reference rows this build did not replay: the digest is of the
            // empty change actually applied here, never a copied per-event one.
            provenance.push(storage_frontier::PhysicalTerminalConstructionBatch {
                acceptance_sequence: event.acceptance_sequence(),
                batch_id: event.batch_id().as_uuid().into_bytes(),
                input_digest: super::MaterializationChange::new(
                    event.batch_id(),
                    Vec::new(),
                    Vec::new(),
                )
                .and_then(|change| change.digest())?,
            });
            instrumentation.accepted_events_applied += 1;
            maybe_abort_rebuild_test(instrumentation.accepted_events_applied);
        }
        let reached = read_frontier_root(&self.physical)?;
        if reached != source.exact_frontier_root
            || reached.acceptance_sequence() != source.accepted_batch_count
        {
            return Err(ProjectionError::Rebuild(
                "terminal accepted-prefix seed did not reach the authenticated frontier root"
                    .into(),
            ));
        }
        trace_terminal_phase("accepted prefix seed", prefix_started);
        let _ = super::hot_engine::take_current_path_cursor_probe();
        let rows_started = std::time::Instant::now();
        let coverage_count = self.seed_terminal_rows(
            engine,
            &source.exact_frontier_root,
            &provenance,
            &mut instrumentation,
            &mut bootstrap,
        )?;
        trace_terminal_phase("terminal row seed", rows_started);
        let cursor_probe = super::hot_engine::take_current_path_cursor_probe();
        bootstrap.terminal_catalog_rows_authenticated = cursor_probe.rows;
        bootstrap.terminal_catalog_document_validations = cursor_probe.catalog_document_validations;
        if std::env::var_os("TINE_ACTIVATION_TRACE").is_some() {
            eprintln!("sqlite terminal current-path cursor probe: {cursor_probe:?}");
        }
        self.fresh_reference_coverage_count = Some(coverage_count);
        if instrumentation.cleanup_page_attempts != 0
            || instrumentation.cleanup_owned_rows != 0
            || instrumentation.cleanup_fts_rowids != 0
            || instrumentation.reference_coverage_inductive_checks != 0
            || instrumentation.reference_coverage_full_scans != 0
            || bootstrap.intermediate_page_materializations != 0
            || bootstrap.bootstrap_part_reads != 0
            || bootstrap.terminal_materializations != 1
        {
            return Err(ProjectionError::Rebuild(
                "terminal candidate structural accounting invariant failed".into(),
            ));
        }
        // Every catalog-document shape proof costs a read linear in the
        // catalog's page entries, so the terminal seed may derive at most one
        // per bounded read window - one per cursor page and one per
        // materialization chunk. Deriving one per catalog row is quadratic in
        // graph pages, which is what this construction exists to avoid.
        let window_bound = bootstrap
            .terminal_catalog_rows_authenticated
            .div_ceil(TERMINAL_CATALOG_CURSOR_PAGE_ROWS)
            .saturating_add(bootstrap.terminal_materialization_chunks)
            .saturating_add(1);
        if bootstrap.terminal_catalog_rows_authenticated != bootstrap.terminal_pages_materialized
            || bootstrap.terminal_catalog_document_validations > window_bound
        {
            return Err(ProjectionError::Rebuild(format!(
                "terminal candidate catalog authority is not bounded by its read window: \
                 rows {} pages {} validations {} bound {window_bound}",
                bootstrap.terminal_catalog_rows_authenticated,
                bootstrap.terminal_pages_materialized,
                bootstrap.terminal_catalog_document_validations,
            )));
        }
        let proof_started = std::time::Instant::now();
        self.finish_fresh_candidate(source, coverage_count, &mut instrumentation)?;
        trace_terminal_phase("candidate proof scans", proof_started);
        terminal_construction_cut(TerminalConstructionCut::BeforeCandidateCommit)?;
        self.physical.finish_candidate_build()?;
        record_candidate_write_instrumentation(
            &mut instrumentation,
            writes_before,
            self.physical.write_instrumentation(),
        );
        Ok((instrumentation, bootstrap))
    }

    /// Stream the complete terminal page and reference rows in bounded chunks.
    ///
    /// The page set is the engine's authenticated current-path catalog at the
    /// exact terminal accepted frontier, and every row is materialized once at
    /// that same root. The final coverage count is separately proved against
    /// `AcceptedFrontierRoot::reference_catalog_root`.
    fn seed_terminal_rows(
        &mut self,
        engine: &ShardedHotEngine,
        terminal_root: &AcceptedFrontierRoot,
        provenance: &[storage_frontier::PhysicalTerminalConstructionBatch],
        instrumentation: &mut RebuildInstrumentation,
        bootstrap: &mut BootstrapSqliteRebuildInstrumentation,
    ) -> Result<u64, ProjectionError> {
        let binding = engine
            .current_path_catalog_binding()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        if binding.workspace_id() != engine.workspace_id()
            || binding.lineage_digest() != engine.lineage_digest()
            || binding.accepted_frontier() != terminal_root.state_digest()
        {
            return Err(ProjectionError::Rebuild(
                "current-path catalog is not bound to the terminal accepted frontier".into(),
            ));
        }
        let catalog_root = terminal_root.reference_catalog_root().clone();
        let extractor_stamp =
            super::sqlite_materialization::ReferenceExtractorDependencyStamp::new(
                catalog_root.extractor_digest(),
                catalog_root.policy_digest(),
            )?;
        let materializer = (binding.catalog_rows() != 0)
            .then(|| {
                engine
                    .bootstrap_bulk_materializer_with_session_budget(
                        terminal_root,
                        super::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT,
                    )
                    .map_err(|error| ProjectionError::Materialization(error.to_string()))
            })
            .transpose()?;
        bootstrap.terminal_materializations = 1;
        instrumentation.accepted_root_authentications += usize::from(materializer.is_some());
        instrumentation.exact_catalog_loads += usize::from(materializer.is_some());
        let mut cursor = Some(
            engine
                .begin_current_path_cursor()
                .map_err(|error| ProjectionError::Rebuild(error.to_string()))?,
        );
        let mut pending: Vec<super::hot_engine::CurrentPathCatalogRow> = Vec::new();
        let mut observed_rows = 0_u64;
        let mut seen_pages = BTreeSet::new();
        while let Some(token) = cursor.take() {
            let cursor_started = std::time::Instant::now();
            let page = engine
                .current_path_cursor_page(
                    token,
                    TERMINAL_CATALOG_CURSOR_PAGE_ROWS
                        .min(super::hot_engine::MAX_CURRENT_PATH_CURSOR_PAGE_ROWS),
                )
                .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
            bootstrap.terminal_catalog_cursor_micros = bootstrap
                .terminal_catalog_cursor_micros
                .saturating_add(cursor_started.elapsed().as_micros());
            let (rows, next) = page.into_parts();
            cursor = next;
            pending.extend(rows);
            while pending.len() >= super::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES
                || (cursor.is_none() && !pending.is_empty())
            {
                let take = pending
                    .len()
                    .min(super::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES);
                let chunk_rows = pending.drain(..take).collect::<Vec<_>>();
                observed_rows = observed_rows.saturating_add(chunk_rows.len() as u64);
                for row in &chunk_rows {
                    if !seen_pages.insert(row.page_id()) {
                        return Err(ProjectionError::Rebuild(
                            "current-path catalog repeats a terminal page identity".into(),
                        ));
                    }
                }
                self.seed_terminal_chunk(
                    materializer.as_ref().ok_or_else(|| {
                        ProjectionError::Rebuild(
                            "terminal catalog rows require a bulk materializer".into(),
                        )
                    })?,
                    engine,
                    &catalog_root,
                    extractor_stamp,
                    &chunk_rows,
                    instrumentation,
                    bootstrap,
                )?;
            }
        }
        if let Some(materializer) = materializer.as_ref() {
            let (accepted_frontier, external_exact) = materializer.lookup_session_stats();
            bootstrap.record_terminal_lookup_session(
                seen_pages.len(),
                accepted_frontier,
                external_exact,
            );
        }
        let current = engine
            .current_path_catalog_binding()
            .map_err(|error| ProjectionError::Rebuild(error.to_string()))?;
        if current != binding
            || observed_rows != binding.catalog_rows()
            || seen_pages.len() as u64 != binding.catalog_rows()
        {
            return Err(ProjectionError::Rebuild(
                "terminal current-path catalog changed or is incompletely covered".into(),
            ));
        }
        if provenance.is_empty() {
            if binding.catalog_rows() != 0 || catalog_root.source_count() != 0 {
                return Err(ProjectionError::Rebuild(
                    "an empty accepted prefix cannot carry terminal catalog rows".into(),
                ));
            }
            return Ok(0);
        }
        let stamp = storage_frontier::PhysicalTerminalCatalogStamp {
            acceptance_sequence: terminal_root.acceptance_sequence(),
            frontier_root_digest: ContentDigest::of(&canonical_frontier_root_bytes(terminal_root)?),
            catalog_root: catalog_root
                .encode()
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?,
            catalog_root_digest: catalog_root
                .external_digest()
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?,
            coverage_digest: catalog_root.source_coverage_root(),
            extractor_dependency_stamp_digest: extractor_stamp.digest()?,
            source_count: catalog_root.source_count(),
        };
        let finish_started = std::time::Instant::now();
        let finished = self
            .physical
            .finish_terminal_bootstrap_construction(provenance, &stamp)
            .map_err(Into::into);
        bootstrap.terminal_finish_micros = bootstrap
            .terminal_finish_micros
            .saturating_add(finish_started.elapsed().as_micros());
        finished
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_terminal_chunk(
        &mut self,
        materializer: &super::hot_engine::BootstrapBulkMaterializer<'_>,
        engine: &ShardedHotEngine,
        catalog_root: &super::ReferenceCatalogRootV2,
        extractor_stamp: super::sqlite_materialization::ReferenceExtractorDependencyStamp,
        rows: &[super::hot_engine::CurrentPathCatalogRow],
        instrumentation: &mut RebuildInstrumentation,
        bootstrap: &mut BootstrapSqliteRebuildInstrumentation,
    ) -> Result<(), ProjectionError> {
        let page_ids = rows.iter().map(|row| row.page_id()).collect::<Vec<_>>();
        let materialize_started = std::time::Instant::now();
        let materialized = materializer
            .materialize_pages(&page_ids)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        bootstrap.terminal_materialization_micros = bootstrap
            .terminal_materialization_micros
            .saturating_add(materialize_started.elapsed().as_micros());
        let mut chunk = super::sqlite_materialization::TerminalMaterializationChunk::default();
        let mut reference_rows = ReferenceCatalogSourceRows::default();
        let lower_started = std::time::Instant::now();
        let mut reference_micros = 0_u128;
        for (row, page) in rows.iter().zip(materialized) {
            let page = page.ok_or_else(|| {
                ProjectionError::Rebuild(
                    "authenticated current-path catalog row has no terminal page".into(),
                )
            })?;
            if page.page_id != row.page_id() || page.path != *row.path() || page.kind != row.kind()
            {
                return Err(ProjectionError::Rebuild(
                    "terminal page identity differs from its authenticated catalog row".into(),
                ));
            }
            chunk.pages.push(materialized_page_input(page));
            let reference_started = std::time::Instant::now();
            let posted = collect_reference_source_rows(
                engine,
                catalog_root,
                extractor_stamp,
                row.page_id(),
                &mut reference_rows,
            )?;
            reference_micros =
                reference_micros.saturating_add(reference_started.elapsed().as_micros());
            if !posted {
                return Err(ProjectionError::Rebuild(
                    "terminal catalog page has no authenticated reference posting".into(),
                ));
            }
        }
        chunk.coverage = reference_rows.coverage;
        chunk.postings = reference_rows.postings;
        chunk.aliases = reference_rows.aliases;
        bootstrap.terminal_materialization_chunks += 1;
        bootstrap.terminal_pages_materialized += page_ids.len();
        bootstrap.peak_terminal_bulk_pages = bootstrap.peak_terminal_bulk_pages.max(page_ids.len());
        instrumentation.bulk_materialization_chunks += 1;
        instrumentation.bulk_pages_materialized += page_ids.len();
        instrumentation.peak_bulk_pages = instrumentation.peak_bulk_pages.max(page_ids.len());
        instrumentation.exact_document_loads = materializer.exact_document_loads();
        let physical = super::sqlite_materialization::lower_terminal_chunk(chunk)?;
        bootstrap.terminal_reference_micros = bootstrap
            .terminal_reference_micros
            .saturating_add(reference_micros);
        bootstrap.terminal_lowering_micros = bootstrap.terminal_lowering_micros.saturating_add(
            lower_started
                .elapsed()
                .as_micros()
                .saturating_sub(reference_micros),
        );
        let insert_started = std::time::Instant::now();
        let seeded = self
            .physical
            .seed_terminal_bootstrap_chunk(&physical)
            .map_err(Into::into);
        bootstrap.terminal_insert_micros = bootstrap
            .terminal_insert_micros
            .saturating_add(insert_started.elapsed().as_micros());
        seeded
    }

    /// The two complete unpublished-candidate scans that close a fresh build's
    /// semantic and materialized-row proof, shared by archive replay and
    /// terminal construction.
    fn finish_fresh_candidate(
        &mut self,
        source: &RebuildSource<'_>,
        inductive_coverage_count: u64,
        instrumentation: &mut RebuildInstrumentation,
    ) -> Result<(), ProjectionError> {
        self.physical.finalize_fresh_bootstrap(
            source
                .exact_frontier_root
                .reference_catalog_root()
                .source_count(),
            inductive_coverage_count,
        )?;
        instrumentation.reference_coverage_full_scans += 1;
        let _semantic_digest = self.semantic_projection_digest()?;
        instrumentation.final_semantic_equivalence_proofs += 1;
        #[cfg(test)]
        let row_digest_started = std::time::Instant::now();
        let _row_digest = self.materialized_row_digest_for_harness()?;
        instrumentation.final_row_digest_equivalence_proofs += 1;
        #[cfg(test)]
        {
            instrumentation.final_row_digest_proof_micros =
                row_digest_started.elapsed().as_micros();
        }
        Ok(())
    }

    fn rebuild_stream(
        &mut self,
        source: &RebuildSource<'_>,
    ) -> Result<
        (
            RebuildInstrumentation,
            BootstrapSqliteRebuildInstrumentation,
        ),
        ProjectionError,
    > {
        let mut instrumentation = RebuildInstrumentation::default();
        let mut intermediate_page_materializations = 0_usize;
        let inactive_bulk = matches!(source.loader, RebuildLoader::InactiveBootstrap { .. });
        let writes_before = self.physical.write_instrumentation();
        if inactive_bulk {
            self.physical.begin_candidate_build()?;
        }
        let mut cursor = source.cursor()?;
        while let Some(event) = cursor.next_event()? {
            instrumentation.accepted_events_validated += 1;
            instrumentation.max_live_events = instrumentation.max_live_events.max(1);
            instrumentation.max_live_evidence_records =
                instrumentation.max_live_evidence_records.max(1);
            let apply_stats = if inactive_bulk {
                let (materialization, materialization_stats) =
                    materialize_inactive_bootstrap_event_bulk(source.engine, &event)?;
                let materialization =
                    attach_authenticated_reference_catalog(source.engine, &event, materialization)?;
                instrumentation.record_materialization(materialization_stats);
                intermediate_page_materializations += 1;
                self.apply_candidate_with_materialization_and_stats(
                    &event,
                    ApplyFault::None,
                    Some(&materialization),
                )?
                .1
            } else {
                let (_, materialization_stats, apply_stats) =
                    self.apply_engine_owned_accepted_with_stats(&event, source.engine)?;
                instrumentation.record_materialization(materialization_stats);
                apply_stats
            };
            instrumentation.cleanup_page_attempts += apply_stats.cleanup_page_attempts;
            instrumentation.cleanup_existing_pages += apply_stats.cleanup_existing_pages;
            instrumentation.cleanup_owned_rows += apply_stats.cleanup_owned_rows;
            instrumentation.cleanup_fts_rowids += apply_stats.cleanup_fts_rowids;
            instrumentation.reference_coverage_inductive_checks +=
                apply_stats.reference_coverage_inductive_checks;
            instrumentation.reference_coverage_full_scans +=
                apply_stats.reference_coverage_full_scans;
            instrumentation.accepted_events_applied += 1;
            maybe_abort_rebuild_test(instrumentation.accepted_events_applied);
        }
        let (page_reads, page_bytes, max_page_bytes) = cursor.page_stats();
        instrumentation.accepted_sequence_page_reads = page_reads;
        instrumentation.accepted_sequence_bytes_read = page_bytes;
        instrumentation.max_accepted_sequence_page_bytes = max_page_bytes;
        if read_frontier_root(&self.physical)? != source.exact_frontier_root {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's authenticated frontier root".into(),
            ));
        }
        if read_frontier_root(&self.physical)?.acceptance_sequence() != source.accepted_batch_count
        {
            return Err(ProjectionError::Rebuild(
                "rebuild did not reach the engine's accepted event count".into(),
            ));
        }
        if instrumentation.exact_catalog_loads > instrumentation.accepted_root_authentications
            || instrumentation.accepted_root_authentications
                > instrumentation.accepted_events_applied
            || instrumentation.cleanup_fts_rowids > instrumentation.cleanup_owned_rows
            || instrumentation.reference_coverage_inductive_checks
                != instrumentation.accepted_events_applied
            || instrumentation.reference_coverage_full_scans != 0
        {
            return Err(ProjectionError::Rebuild(
                "fresh candidate structural accounting invariant failed".into(),
            ));
        }
        let inductive_coverage_count = self.fresh_reference_coverage_count.ok_or_else(|| {
            ProjectionError::Rebuild(
                "fresh candidate lost its inductive reference coverage state".into(),
            )
        })?;
        // Every preceding row transition was committed atomically with one
        // authenticated archive event, and the exact terminal frontier above
        // equals the source authority. The two complete scans below close that
        // inductive semantic/materialized-row proof while the file is still an
        // unpublished candidate; publication happens only after this returns.
        self.finish_fresh_candidate(source, inductive_coverage_count, &mut instrumentation)?;
        if inactive_bulk {
            self.physical.finish_candidate_build()?;
        }
        record_candidate_write_instrumentation(
            &mut instrumentation,
            writes_before,
            self.physical.write_instrumentation(),
        );
        let mut bootstrap = cursor.bootstrap_instrumentation();
        bootstrap.intermediate_page_materializations = intermediate_page_materializations;
        Ok((instrumentation, bootstrap))
    }

    #[cfg(test)]
    fn apply_internal(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_internal_with_materialization(event, fault, None)
    }

    fn apply_internal_with_materialization(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
        materialization: Option<&super::MaterializationChange>,
    ) -> Result<ApplyDisposition, ProjectionError> {
        self.apply_internal_with_materialization_and_stats(event, fault, materialization)
            .map(|(disposition, _)| disposition)
    }

    fn apply_internal_with_materialization_and_stats(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
        materialization: Option<&super::MaterializationChange>,
    ) -> Result<
        (
            ApplyDisposition,
            super::sqlite_materialization::ApplyChangeInstrumentation,
        ),
        ProjectionError,
    > {
        self.apply_with_materialization_transaction_policy(event, fault, materialization, false)
    }

    fn apply_candidate_with_materialization_and_stats(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
        materialization: Option<&super::MaterializationChange>,
    ) -> Result<
        (
            ApplyDisposition,
            super::sqlite_materialization::ApplyChangeInstrumentation,
        ),
        ProjectionError,
    > {
        self.apply_with_materialization_transaction_policy(event, fault, materialization, true)
    }

    fn apply_with_materialization_transaction_policy(
        &mut self,
        event: &AcceptedBatchEvent,
        fault: ApplyFault,
        materialization: Option<&super::MaterializationChange>,
        candidate_build: bool,
    ) -> Result<
        (
            ApplyDisposition,
            super::sqlite_materialization::ApplyChangeInstrumentation,
        ),
        ProjectionError,
    > {
        #[cfg(not(test))]
        let _ = fault;
        self.validate_event_claim(event)?;
        self.require_frontier(event.post_frontier_root())?;
        let materialization_digest = materialization
            .map(|change| change.validate_for_event(event))
            .transpose()?;
        let current_root = read_frontier_root(&self.physical)?;
        let current_physical = lower_physical_frontier_root(&current_root)?;
        let batch = lower_physical_accepted_batch(event)?;
        let (physical_materialization, authenticated_reference) = match materialization {
            Some(change) => {
                let authenticated = change
                    .reference_catalog()
                    .is_some()
                    .then(|| authenticated_reference_materialization(event))
                    .transpose()?;
                let (physical, authenticated) =
                    super::sqlite_materialization::lower_validated_change(
                        change,
                        event.semantic_effect(),
                        authenticated.as_ref(),
                    )?;
                (Some(physical), authenticated)
            }
            None => (None, None),
        };
        let mut request = storage_frontier::PhysicalApplyRequest {
            batch,
            materialization: physical_materialization,
            materialization_input_digest: materialization_digest,
            authenticated_reference,
            prior_reference_coverage_count: self.fresh_reference_coverage_count,
            fault: storage_frontier::ApplyFault::None,
        };
        let preflight = match self.physical.preflight(&current_physical, &request) {
            Ok(disposition) => disposition,
            Err(storage_frontier::FrontierError::BatchCollision(_)) => {
                let existing = load_batch(&self.physical, event.batch_id)?.ok_or_else(|| {
                    ProjectionError::Corrupt(format!(
                        "colliding batch {} disappeared during physical preflight",
                        event.batch_id
                    ))
                })?;
                let _ = existing.matches(event)?;
                return Err(ProjectionError::BatchCollision(event.batch_id));
            }
            Err(error) => return Err(error.into()),
        };
        if matches!(preflight, storage_frontier::PreflightDisposition::Duplicate) {
            let existing = load_batch(&self.physical, event.batch_id)?.ok_or_else(|| {
                ProjectionError::Corrupt(format!(
                    "duplicate batch {} disappeared during physical preflight",
                    event.batch_id
                ))
            })?;
            if !existing.matches(event)? {
                return Err(ProjectionError::BatchCollision(event.batch_id));
            }
        }
        if matches!(preflight, storage_frontier::PreflightDisposition::New) {
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
                let _ = self.physical.frontier_document(
                    &current_physical,
                    document.document_id().as_uuid().into_bytes(),
                )?;
                if !document.direct_dependency_heads().contains(&event.batch_id) {
                    return Err(ProjectionError::InvalidAcceptedEvent(format!(
                        "affected document {} does not name accepted batch {} as a direct head",
                        document.document_id(),
                        event.batch_id
                    )));
                }
            }
            #[cfg(test)]
            {
                request.fault = match fault {
                    ApplyFault::ReturnAfterInsert => {
                        storage_frontier::ApplyFault::ReturnAfterInsert
                    }
                    ApplyFault::ReturnAfterMaterialization => {
                        storage_frontier::ApplyFault::ReturnAfterMaterialization
                    }
                    ApplyFault::AbortAfterInsert => storage_frontier::ApplyFault::AbortAfterInsert,
                    _ if fail_during_apply_for_harness().is_err() => {
                        storage_frontier::ApplyFault::ReturnAfterMaterialization
                    }
                    _ => storage_frontier::ApplyFault::None,
                };
            }
            #[cfg(not(test))]
            if fail_during_apply_for_harness().is_err() {
                request.fault = storage_frontier::ApplyFault::ReturnAfterMaterialization;
            }
        }
        let result = if candidate_build {
            self.physical.apply_candidate(&current_physical, &request)?
        } else {
            self.physical.apply(&current_physical, &request)?
        };
        if self.fresh_reference_coverage_count.is_some() {
            if let Some(next_count) = result.materialization.reference_coverage_count {
                self.fresh_reference_coverage_count = Some(next_count);
            }
        }
        let disposition = match result.disposition {
            storage_frontier::ApplyDisposition::Applied => ApplyDisposition::Applied,
            storage_frontier::ApplyDisposition::Duplicate => ApplyDisposition::Duplicate,
        };
        if matches!(disposition, ApplyDisposition::Applied) && self.checkpoint_each_apply {
            write_projection_checkpoint(&self.path, self.claim, &event.post_frontier_root)?;
        }
        #[cfg(test)]
        if matches!(fault, ApplyFault::AbortAfterCommit)
            && matches!(disposition, ApplyDisposition::Applied)
        {
            std::process::abort();
        }
        return Ok((disposition, result.materialization));
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
        let _ = self.physical.checkpoint_truncate();
        if let Ok(root) = read_frontier_root(&self.physical) {
            let _ = write_projection_checkpoint(&self.path, self.claim, &root);
        }
    }
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
    let physical = PhysicalSqliteDatabase::open_read_only(path)
        .map_err(|error| format!("cannot open SQLite projection read-only: {error}"))?;
    physical
        .validate_schema_and_claim(lower_physical_claim(claim))
        .map_err(|error| ProjectionError::from(error).to_string())?;
    let found_frontier = read_frontier_root(&physical).map_err(|error| error.to_string())?;
    if found_frontier != source.exact_frontier_root {
        return Err("SQLite frontier is stale".into());
    }
    let count = physical
        .read_frontier()
        .map_err(|error| error.to_string())?
        .applied_batch_count;
    let expected_count = i64::try_from(source.accepted_batch_count)
        .map_err(|_| "accepted batch count exceeds SQLite".to_string())?;
    if i64::try_from(count).ok() != Some(expected_count) {
        return Err("SQLite frontier batch count is stale".into());
    }
    if let Some(root_key) = found_frontier.document_map_root_key() {
        physical
            .frontier_document(
                &lower_physical_frontier_root(&found_frontier)
                    .map_err(|error| error.to_string())?,
                root_key,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "SQLite authenticated frontier root row is missing".to_string())?;
    } else if found_frontier.document_count() != 0 {
        return Err("SQLite authenticated frontier root key is missing".into());
    }
    if expected_count > 0 {
        let final_record =
            load_batch_at_sequence(&physical, expected_count).map_err(|error| error.to_string())?;
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
        if !physical
            .authenticate_batch(
                &lower_physical_frontier_root(&found_frontier)
                    .map_err(|error| error.to_string())?,
                final_record.batch_id.as_uuid().into_bytes(),
                final_record
                    .causal_record_digest()
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?
        {
            return Err("SQLite final accepted row is absent from its authenticated map".into());
        }
    }
    let root_bytes =
        canonical_frontier_root_bytes(&found_frontier).map_err(|error| error.to_string())?;
    physical
        .ensure_materialization_stamp(
            found_frontier.acceptance_sequence(),
            ContentDigest::of(&root_bytes),
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_sidecar_shape(path: &Path) -> Result<(), ProjectionError> {
    let files = SqliteFileSet::new(path);
    let wal = read_file_prefix(files.wal_path(), 32)?;
    let shm = read_file_prefix(files.shm_path(), 136)?;
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
    let files = SqliteFileSet::new(path);
    let checkpoint_path = files.checkpoint_path();
    let metadata = fs::symlink_metadata(&checkpoint_path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > storage_frontier::MAX_SQLITE_CHECKPOINT_BYTES as u64
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
    let physical_checkpoint = files.physical_checkpoint()?;
    if envelope.checkpoint.frontier_root_digest != ContentDigest::of(&expected_root_bytes)
        || envelope.checkpoint.database != physical_checkpoint.database
        || envelope.checkpoint.wal != physical_checkpoint.wal
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
    let files = SqliteFileSet::new(path);
    let physical_checkpoint = files.physical_checkpoint()?;
    let checkpoint = ProjectionCheckpoint {
        schema_version: PROJECTION_CHECKPOINT_SCHEMA_VERSION,
        workspace_id: claim.workspace_id,
        frontier_root_digest: ContentDigest::of(&root_bytes),
        database: physical_checkpoint.database,
        wal: physical_checkpoint.wal,
    };
    let checkpoint_bytes = postcard::to_allocvec(&checkpoint)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    let envelope = ProjectionCheckpointEnvelope {
        digest: ContentDigest::of(&checkpoint_bytes),
        checkpoint,
    };
    let bytes = postcard::to_allocvec(&envelope)
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
    files.publish_checkpoint(&bytes).map_err(Into::into)
}

fn initialize_schema(
    physical: &PhysicalSqliteDatabase,
    claim: ProjectionClaim,
) -> Result<(), ProjectionError> {
    let frontier = canonical_frontier_root_bytes(&AcceptedFrontierRoot::empty())?;
    physical.initialize_schema(lower_physical_claim(claim), &frontier)?;
    return Ok(());
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
        Ok(super::hot_engine::accepted_causal_record_digest(
            self.batch_id,
            manifest_digest,
            binding,
            self.causal_dot()?,
            Some(decode_uuid(&self.causal_clock_root_key)?.into_bytes()),
            decode_content_digest(&self.causal_clock_root_digest)?,
        ))
    }

    #[cfg(test)]
    fn authenticated_reference_materialization(
        &self,
    ) -> Result<super::sqlite_materialization::AuthenticatedReferenceMaterialization, ProjectionError>
    {
        let manifest_digest = decode_content_digest(&self.manifest_digest)?;
        let semantic_effect_digest = decode_semantic_effect_digest(&self.semantic_effect_digest)?;
        let dependency_frontier = decode_frontier(&self.dependency_frontier)?;
        let causal_dependency_heads = decode_batch_ids(&self.causal_dependency_heads)?;
        let event_binding_digest = super::AcceptedBatchEvidence::binding_digest_for(
            self.batch_id,
            manifest_digest,
            semantic_effect_digest,
            &dependency_frontier,
            &causal_dependency_heads,
        )
        .map_err(|error| ProjectionError::Corrupt(error.to_string()))?;
        let prior = decode_frontier_root(&self.prior_frontier_root)?;
        let post = decode_frontier_root(&self.post_frontier_root)?;
        Ok(
            super::sqlite_materialization::AuthenticatedReferenceMaterialization {
                event_binding_digest,
                prior_frontier_root_digest: decode_content_digest(
                    &self.prior_frontier_root_digest,
                )?,
                post_frontier_root_digest: decode_content_digest(&self.post_frontier_root_digest)?,
                prior_catalog_root: prior.reference_catalog_root().clone(),
                post_catalog_root: post.reference_catalog_root().clone(),
            },
        )
    }
}

fn validate_stored_history(
    physical: &PhysicalSqliteDatabase,
) -> Result<(AcceptedFrontierRoot, u64), ProjectionError> {
    let mut prior = AcceptedFrontierRoot::empty();
    let mut count = 0_u64;
    for physical_batch in physical.load_all_batches()? {
        let record = stored_batch_from_storage(physical_batch)?;
        count = count
            .checked_add(1)
            .ok_or_else(|| ProjectionError::Corrupt("stored history count overflowed".into()))?;
        if record.sequence != count as i64 {
            return Err(ProjectionError::Corrupt(
                "stored accepted history sequence is not contiguous".into(),
            ));
        }
        let post = record.validate_canonical_transition(&prior)?;
        if !physical.authenticate_batch(
            &lower_physical_frontier_root(&post)?,
            record.batch_id.as_uuid().into_bytes(),
            record.causal_record_digest()?,
        )? {
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
    physical: &PhysicalSqliteDatabase,
    batch_id: BatchId,
) -> Result<Option<StoredBatch>, ProjectionError> {
    physical
        .load_batch(batch_id.as_uuid().into_bytes())?
        .map(stored_batch_from_storage)
        .transpose()
}

fn load_batch_at_sequence(
    physical: &PhysicalSqliteDatabase,
    sequence: i64,
) -> Result<Option<StoredBatch>, ProjectionError> {
    physical
        .load_batch_at_sequence(sequence)?
        .map(stored_batch_from_storage)
        .transpose()
}

fn stored_batch_from_storage(
    row: storage_frontier::StoredBatch,
) -> Result<StoredBatch, ProjectionError> {
    Ok(StoredBatch {
        sequence: row.sequence,
        batch_id: BatchId::from_uuid(Uuid::from_bytes(row.batch_id)),
        manifest_digest: row.manifest_digest,
        semantic_effect: row.semantic_effect,
        semantic_effect_digest: row.semantic_effect_digest,
        dependency_frontier: row.dependency_frontier,
        dependency_frontier_digest: row.dependency_frontier_digest,
        prior_frontier_root: row.prior_frontier_root,
        prior_frontier_root_digest: row.prior_frontier_root_digest,
        post_frontier_root: row.post_frontier_root,
        post_frontier_root_digest: row.post_frontier_root_digest,
        affected_documents: row.affected_documents,
        affected_documents_digest: row.affected_documents_digest,
        causal_dependency_heads: row.causal_dependency_heads,
        causal_peer_id: row.causal_peer_id,
        causal_counter: row.causal_counter,
        causal_clock_root_key: row.causal_clock_root_key,
        causal_clock_root_digest: row.causal_clock_root_digest,
        acceptance_sequence: row.acceptance_sequence,
        retained_bytes: row.retained_bytes,
    })
}

fn read_frontier_root(
    physical: &PhysicalSqliteDatabase,
) -> Result<AcceptedFrontierRoot, ProjectionError> {
    let stored = physical.read_frontier()?;
    let root = decode_frontier_root(&stored.canonical_bytes)?;
    if stored.applied_batch_count != root.acceptance_sequence() {
        return Err(ProjectionError::Corrupt(
            "frontier applied-batch count differs from its root".into(),
        ));
    }
    Ok(root)
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

fn authenticated_frontier_document(
    physical: &PhysicalSqliteDatabase,
    root: &AcceptedFrontierRoot,
    document_id: DocumentId,
) -> Result<Option<DocumentDependencies>, ProjectionError> {
    let physical_root = lower_physical_frontier_root(root)?;
    return physical
        .frontier_document(&physical_root, document_id.as_uuid().into_bytes())?
        .map(|bytes| decode_frontier_document(document_id, &bytes))
        .transpose();
}

fn read_frontier_documents(
    physical: &PhysicalSqliteDatabase,
) -> Result<FrontierV2, ProjectionError> {
    let root = read_frontier_root(physical)?;
    let physical_root = lower_physical_frontier_root(&root)?;
    let documents = physical
        .read_frontier_documents(&physical_root)?
        .into_iter()
        .map(|document| {
            decode_frontier_document(
                DocumentId::from_uuid(Uuid::from_bytes(document.document_id)),
                &document.canonical_bytes,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    return FrontierV2::new(documents).map_err(|error| ProjectionError::Corrupt(error.to_string()));
}

fn document_frontier_contains(
    physical: &PhysicalSqliteDatabase,
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
            if batch_descends_from_database(physical, *have_head, *required_head)? {
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
    physical: &PhysicalSqliteDatabase,
    descendant: BatchId,
    ancestor: BatchId,
) -> Result<bool, ProjectionError> {
    batch_descends_from_database_measured(physical, descendant, ancestor)
        .map(|(contained, _)| contained)
}

fn batch_descends_from_database_measured(
    physical_database: &PhysicalSqliteDatabase,
    descendant: BatchId,
    ancestor: BatchId,
) -> Result<(bool, usize), ProjectionError> {
    let root = read_frontier_root(physical_database)?;
    let physical = lower_physical_frontier_root(&root)?;
    let descendant_record = load_batch(physical_database, descendant)?.ok_or_else(|| {
        ProjectionError::Corrupt(format!(
            "descendant batch {descendant} is absent from the authenticated accepted map"
        ))
    })?;
    if !physical_database.authenticate_batch(
        &physical,
        descendant.as_uuid().into_bytes(),
        descendant_record.causal_record_digest()?,
    )? {
        return Err(ProjectionError::Corrupt(format!(
            "descendant batch {descendant} is absent from the authenticated accepted map"
        )));
    }
    let Some(ancestor_record) = load_batch(physical_database, ancestor)? else {
        return Ok((false, 0));
    };
    if !physical_database.authenticate_batch(
        &physical,
        ancestor.as_uuid().into_bytes(),
        ancestor_record.causal_record_digest()?,
    )? {
        return Ok((false, 0));
    }
    Ok((
        physical_database.batch_descends_from(
            &physical,
            descendant.as_uuid().into_bytes(),
            ancestor.as_uuid().into_bytes(),
        )?,
        0,
    ))
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

fn require_existing_database_path(path: &Path) -> Result<PathBuf, ProjectionError> {
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
    let canonical_parent = fs::canonicalize(parent)?;
    let canonical_path = canonical_parent.join(name);
    let metadata = fs::symlink_metadata(&canonical_path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ProjectionError::UnsafePath(
            "database path is not an existing regular no-follow file".into(),
        ));
    }
    Ok(canonical_path)
}

fn incomplete_forensic_recovery_exists(path: &Path) -> Result<bool, ProjectionError> {
    let parent = path
        .parent()
        .ok_or_else(|| ProjectionError::UnsafePath("database path has no parent".into()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectionError::UnsafePath("database file name is not UTF-8".into()))?;
    let prefix = format!("{file_name}.forensic-");
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
        match fs::symlink_metadata(directory.join("REBUILD_COMPLETE")) {
            Ok(marker) if marker.file_type().is_symlink() || !marker.is_file() => {
                return Err(ProjectionError::UnsafePath(format!(
                    "forensic completion marker in {} is not a regular file",
                    directory.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
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

fn remove_projection_files(path: &Path) -> Result<(), ProjectionError> {
    SqliteFileSet::new(path).remove().map_err(Into::into)
}

fn projection_files_exist(path: &Path) -> bool {
    SqliteFileSet::new(path).any_exists()
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
    let files = SqliteFileSet::new(path);
    let mappings = files.preserve_forensic_files(&directory, |moved| {
        maybe_abort_forensic_test("after-move", moved);
    })?;
    pending
        .evidence
        .extend(mappings.into_iter().map(|mapping| ForensicEvidence {
            original_path: mapping.original_path,
            preserved_path: mapping.preserved_path,
        }));
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
        let files = SqliteFileSet::new(path);
        let mappings = files.resume_forensic_files(&directory, evidence_complete)?;
        pending
            .evidence
            .extend(mappings.into_iter().map(|mapping| ForensicEvidence {
                original_path: mapping.original_path,
                preserved_path: mapping.preserved_path,
            }));
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

/// Sealed construction site for the archive-rooted workspace runtime lease and
/// its affine SQLite applier slot.
///
/// Every value able to create an [`SqliteApplierSlot`] lives inside this
/// module, so no other code in this file - let alone elsewhere in the crate -
/// can forge one with a struct literal. The single way to obtain a slot is
/// [`WorkspaceRuntimeLease::applier_slot`] on a lease that holds the
/// archive-rooted workspace lock right now.
mod applier_lease {
    // `unlock` resolves to the inherent `std::fs::File` method; the fs2 trait is
    // only still needed for the test-only contention probe below.
    #[cfg(test)]
    use fs2::FileExt as _;

    use super::super::object_store::sync_dir_required;
    use super::*;

    /// The archive-rooted workspace runtime lease.
    ///
    /// This is the exact lock the SQLite applier has always taken first: the same
    /// `<archive>/.tine-runtime/sqlite-workspaces/<workspace>/sqlite-applier.lock`
    /// file, created and opened through the retained no-follow [`ObjectStore`]
    /// capability, in the same order, with the same ownership, mode, regular-file,
    /// and link-count validators. Hoisting it out of the combined SQLite applier
    /// lease introduces no second lock protocol and no durable-format change; it
    /// only lets one runtime session retain the workspace lock across a
    /// bootstrap -> promoted database handoff.
    ///
    /// The lease is exclusive and non-cloneable. Because its capability is the
    /// archive directory rather than device-local app data, two processes with
    /// different XDG, HOME, or Flatpak roots still contend on the same physical
    /// lock file. A clean drop or process termination releases the OS lock; the
    /// lock file itself never decides ownership by its contents.
    ///
    /// # Why the lock file is empty and never rewritten
    ///
    /// The lease file lives *inside the archive*, which in the supported
    /// multi-device configuration is a Syncthing/Dropbox-replicated directory.
    /// The OS lock is an inode-scoped `flock`: if a sync provider replaces the
    /// file (write-temp-then-rename, conflict resolution, or a restore), a
    /// later local process opening the same *name* gets a different inode and
    /// its `flock` succeeds while this process still holds the old one — two
    /// local runtimes would each believe they own the workspace. Providers
    /// replace a file only when its bytes change, so this lease writes none:
    /// the file is created empty through `O_CREAT` without `O_TRUNC` and is
    /// never truncated, written, or `fsync`ed afterwards. Every device's copy
    /// is therefore byte-identical and content-stable for the archive's whole
    /// life, so there is nothing to replicate, no conflict copy to create from
    /// a content divergence, and no reason for the local inode to be swapped.
    /// Diagnostics that used to live in these bytes (pid, platform) are exactly
    /// the device-varying content that made the file replicable, so they are
    /// deliberately absent. A provider that creates *sibling* conflict files is
    /// tolerated: this code opens one exact name.
    ///
    /// # Why the lock file's identity is checked, not just its name
    ///
    /// Stable empty bytes remove every reason Tine could *give* a provider to
    /// replace this file, but they cannot remove the replacements Tine does not
    /// cause: a Syncthing receive-only "Revert local changes", a folder
    /// reset/re-add, a delete-then-restore, a `.stversions` restore, or the user
    /// deleting `.tine-runtime` by hand. Any of those unlinks the locked inode
    /// and puts a new file at the same name, and an inode-scoped `flock` follows
    /// the inode: the old holder still owns a lock on a file nobody can reach by
    /// name, while a newcomer opening the name locks the new file and succeeds.
    /// Two runtimes would then each hold "the" workspace lease.
    ///
    /// That is closed by binding the lease to one [`LeaseFileIdentity`] —
    /// `(st_dev, st_ino)` on Unix, `FILE_ID_INFO` on Windows — taken from the
    /// held handle *and* from a fresh no-follow lookup of the exact lease
    /// pathname, and by requiring the two to stay equal:
    ///
    /// * at acquisition ([`Self::acquire`]), which closes the open/lock/check
    ///   race with a small bounded number of explicit retries and then fails
    ///   closed as [`ProjectionError::LeaseContended`]. There is deliberately no
    ///   blocking and no unbounded retry here — see the lock-order note in
    ///   `local_active`'s module documentation;
    /// * at every authority-bearing use of the lease while it is held
    ///   ([`Self::revalidate_identity`]).
    ///
    /// A replacement therefore makes exactly one of the two candidates authority:
    /// the newcomer, whose handle and name still agree. The old holder's
    /// authority becomes visibly unavailable before it can open a database,
    /// advance SQLite, swap a crash-takeover handoff record, or publish a `Safe`
    /// handoff.
    pub(crate) struct WorkspaceRuntimeLease {
        file: File,
        workspace_id: WorkspaceId,
        archive_root: PathBuf,
        lease_path: PathBuf,
        /// The exact file this lease locked, as the OS identifies it.
        identity: LeaseFileIdentity,
        /// The retained no-follow archive-root capability, kept so the exact
        /// lease pathname can be resolved again the way a newcomer would
        /// resolve it: component by component, no-follow at each step. The
        /// archive root itself is the trust anchor — an `ObjectStore` is a
        /// retained directory capability whose directory cannot be swapped
        /// underneath it, and a substituted archive root is refused by
        /// `authenticate_archive_identity` at every runtime boundary.
        archive_capability: CapDir,
        applier_slot_vended: AtomicBool,
    }

    /// How many times an acquisition may lose the open/lock/check race to an
    /// out-of-band replacement before it fails closed.
    ///
    /// Small, explicit, and finite on purpose. A replacement that keeps winning
    /// is a filesystem that is actively being rewritten underneath this process,
    /// which is a fail-closed condition, not something to wait out: this lease
    /// is acquired while the device-local enrollment lease may already be held,
    /// so blocking or retrying without a bound here would turn a deliberate
    /// non-blocking lock-order inversion into a real one.
    pub(super) const WORKSPACE_LEASE_IDENTITY_ATTEMPTS: usize = 3;

    impl WorkspaceRuntimeLease {
        pub(crate) fn acquire(
            store: &ObjectStore,
            workspace_id: WorkspaceId,
        ) -> Result<Self, ProjectionError> {
            if store.workspace_id() != workspace_id {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: store.workspace_id(),
                });
            }
            let store_root = store
                .workspace_runtime_lease_capability()
                .map_err(|error| {
                    ProjectionError::UnsafePath(format!(
                        "cannot retain ObjectStore lease authority: {error}"
                    ))
                })?;
            let workspace_name = workspace_id.to_string();
            let lease_path = store
                .root_path()
                .join(OBJECT_STORE_LEASE_NAMESPACE)
                .join(SQLITE_WORKSPACE_LEASE_NAMESPACE)
                .join(&workspace_name)
                .join(SQLITE_APPLIER_LEASE_FILE);

            // The whole open/lock/identity sequence retries, not just the
            // identity check: a replacement that reached the lease file may
            // equally have replaced a directory above it, so the next attempt
            // re-resolves every component from the retained archive capability.
            for _ in 0..WORKSPACE_LEASE_IDENTITY_ATTEMPTS {
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
                let workspace_root = open_or_create_lease_directory(
                    &sqlite_namespace,
                    &workspace_name,
                    "SQLite workspace lease directory",
                )?;
                let file = lock_workspace_lease_file(
                    &workspace_root,
                    SQLITE_APPLIER_LEASE_FILE,
                    &lease_path,
                )?;
                // The locked handle's own identity, and the identity the exact
                // pathname resolves to right now. If a replacement raced the
                // open, the lock this attempt holds is on an unreachable file
                // and grants nothing.
                let held = held_file_identity(&file, &lease_path)?;
                let named = resolve_lease_file_identity(&store_root, &workspace_name, &lease_path);
                if named.is_ok_and(|named| named == held) {
                    // Ownership, regular-file, and link validators run on the
                    // file just proved to be the one this pathname names.
                    validate_opened_lease_file(&file, &lease_path)?;
                    #[cfg(test)]
                    WORKSPACE_RUNTIME_LEASE_ACQUISITIONS
                        .with(|count| count.set(count.get().saturating_add(1)));
                    // Deliberately no truncate, no write, no `sync_all`: see the
                    // type's "Why the lock file is empty and never rewritten"
                    // section. Only the directory entry needs to be durable.
                    sync_dir_required(&workspace_root)
                        .map_err(|error| ProjectionError::Io(error.to_string()))?;
                    return Ok(Self {
                        file,
                        workspace_id,
                        archive_root: store.root_path().to_path_buf(),
                        lease_path,
                        identity: held,
                        archive_capability: store_root,
                        applier_slot_vended: AtomicBool::new(false),
                    });
                }
                // Release the lock on the file that is no longer this pathname
                // before trying again, so a bounded retry cannot self-contend.
                drop(file);
            }
            Err(ProjectionError::LeaseContended(lease_path))
        }

        /// Fail closed unless the file this lease locked is still the file the
        /// exact lease pathname resolves to.
        ///
        /// This is the while-held half of the identity contract. It is called
        /// from the authority-bearing boundaries enumerated in `local_active`'s
        /// "Workspace lease identity" documentation, never from the per-mutation
        /// admission fast path: an admission carries this session fact exactly as
        /// it carries the archive control-directory identity, and re-derives it
        /// the moment the enrollment binding generation moves.
        pub(crate) fn revalidate_identity(&self) -> Result<(), ProjectionError> {
            #[cfg(test)]
            WORKSPACE_LEASE_IDENTITY_REVALIDATIONS
                .with(|count| count.set(count.get().saturating_add(1)));
            #[cfg(test)]
            if FAIL_NEXT_WORKSPACE_LEASE_IDENTITY_CHECK.with(|fail| fail.replace(false)) {
                return Err(ProjectionError::LeaseIdentityUnavailable(
                    "injected transient workspace-lease identity-check failure".into(),
                ));
            }
            let held = held_file_identity(&self.file, &self.lease_path)
                .map_err(|error| ProjectionError::LeaseIdentityUnavailable(error.to_string()))?;
            if held != self.identity {
                return Err(ProjectionError::LeaseIdentityReplaced(format!(
                    "the held SQLite applier lease handle {} is no longer the file it locked",
                    self.lease_path.display()
                )));
            }
            let named = resolve_lease_file_identity(
                &self.archive_capability,
                &self.workspace_id.to_string(),
                &self.lease_path,
            )
            .map_err(LeasePathResolutionError::into_projection_error)?;
            if named != self.identity {
                return Err(ProjectionError::LeaseIdentityReplaced(format!(
                    "the SQLite applier lease {} was replaced while this runtime held it, so this \
                     lock no longer proves workspace ownership",
                    self.lease_path.display()
                )));
            }
            Ok(())
        }

        /// Vend this lease's single affine SQLite applier slot.
        ///
        /// At most one slot is live per lease at a time; dropping the slot returns
        /// availability to this exact lease. The slot borrows the lease, so it can
        /// neither outlive it nor be detached from it.
        pub(crate) fn applier_slot(&self) -> Result<SqliteApplierSlot<'_>, ProjectionError> {
            if self
                .applier_slot_vended
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_err()
            {
                return Err(ProjectionError::LeaseContended(self.lease_path.clone()));
            }
            Ok(SqliteApplierSlot { lease: self })
        }

        /// Borrowed proof that this process holds this exact archive-rooted
        /// workspace lease right now.
        ///
        /// The proof carries no applier slot, so handing one to a durable-state
        /// mutation proves workspace ownership without granting the right to
        /// open a database.
        pub(crate) const fn proof(&self) -> WorkspaceRuntimeProof<'_> {
            WorkspaceRuntimeProof { lease: self }
        }
    }

    impl Drop for WorkspaceRuntimeLease {
        fn drop(&mut self) {
            let _ = self.file.unlock();
            #[cfg(test)]
            record_applier_lock_release(|| ApplierLockRelease::Workspace);
        }
    }

    /// One observed applier-lock release, in the order it actually happened.
    ///
    /// This exists so the declared field order of [`LeasedWorkspaceProjection`]
    /// has a receipt instead of a comment: an inverted order is observable
    /// here, and the boolean records the security-relevant fact directly.
    #[cfg(test)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum ApplierLockRelease {
        /// The database-adjacent applier lock was released.
        ///
        /// `workspace_still_contended` is measured at that exact instant, from
        /// an independent open of the archive lock file: while a database
        /// applier is being torn down, no other process — under any app-data
        /// root — may be able to take the archive.
        Database { workspace_still_contended: bool },
        /// The archive-rooted workspace lock was released.
        Workspace,
    }

    #[cfg(test)]
    thread_local! {
        static APPLIER_LOCK_RELEASES: std::cell::RefCell<Option<Vec<ApplierLockRelease>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Run `body` while recording applier-lock releases on this thread.
    ///
    /// Recording is per-thread and off by default, so the probe below costs
    /// nothing outside the tests that ask for it.
    #[cfg(test)]
    pub(crate) fn recorded_applier_lock_releases<T>(
        body: impl FnOnce() -> T,
    ) -> (T, Vec<ApplierLockRelease>) {
        APPLIER_LOCK_RELEASES.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
        let value = body();
        let releases = APPLIER_LOCK_RELEASES
            .with(|slot| slot.borrow_mut().take())
            .unwrap_or_default();
        (value, releases)
    }

    #[cfg(test)]
    fn record_applier_lock_release(event: impl FnOnce() -> ApplierLockRelease) {
        let recording = APPLIER_LOCK_RELEASES.with(|slot| slot.borrow().is_some());
        if !recording {
            return;
        }
        let event = event();
        APPLIER_LOCK_RELEASES.with(|slot| {
            if let Some(releases) = slot.borrow_mut().as_mut() {
                releases.push(event);
            }
        });
    }

    #[cfg(test)]
    thread_local! {
        /// Runs inside [`WorkspaceRuntimeLease::acquire`], after the lease file
        /// has been opened and before it is locked and identity-checked. This is
        /// the one window a regression cannot otherwise reach.
        static INTERPOSE_WORKSPACE_LEASE_OPEN: std::cell::RefCell<Option<Box<dyn FnMut(&Path)>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Install the open-before-lock interposition used by the replacement
    /// regressions. Per-thread and off by default.
    #[cfg(test)]
    pub(crate) fn set_workspace_lease_open_hook_for_test(hook: Box<dyn FnMut(&Path)>) {
        INTERPOSE_WORKSPACE_LEASE_OPEN.with(|slot| *slot.borrow_mut() = Some(hook));
    }

    #[cfg(test)]
    pub(crate) fn clear_workspace_lease_open_hook_for_test() {
        INTERPOSE_WORKSPACE_LEASE_OPEN.with(|slot| *slot.borrow_mut() = None);
    }

    #[cfg(test)]
    pub(super) fn interpose_workspace_lease_open_for_test(path: &Path) {
        INTERPOSE_WORKSPACE_LEASE_OPEN.with(|slot| {
            if let Some(hook) = slot.borrow_mut().as_mut() {
                hook(path);
            }
        });
    }

    /// Is the archive workspace lock held by *someone* right now?
    ///
    /// `flock` is scoped to an open file description, so a second open in this
    /// same process contends with a live lease exactly as another process
    /// would. A missing file counts as uncontended.
    #[cfg(test)]
    pub(crate) fn workspace_lock_is_contended(path: &Path) -> bool {
        let Ok(file) = File::options().read(true).write(true).open(path) else {
            return false;
        };
        match file.try_lock_exclusive() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(_) => true,
        }
    }

    /// The affine right to run this process's single SQLite applier under one
    /// [`WorkspaceRuntimeLease`].
    ///
    /// The slot has no public constructor, no `Clone`, and no owned state: it is
    /// only ever the value returned by [`WorkspaceRuntimeLease::applier_slot`], and
    /// it borrows that lease for its whole life. Holding one is therefore a
    /// compile-time proof that this process holds the archive-rooted workspace lock
    /// right now, and a caller cannot forge, copy, or outlive that proof.
    pub(crate) struct SqliteApplierSlot<'lease> {
        lease: &'lease WorkspaceRuntimeLease,
    }

    impl SqliteApplierSlot<'_> {
        /// Fail closed unless this slot's lease is the archive-rooted workspace
        /// authority for the exact workspace and exact archive being opened.
        fn authorize(
            &self,
            store: &ObjectStore,
            workspace_id: WorkspaceId,
        ) -> Result<(), ProjectionError> {
            if self.lease.workspace_id != workspace_id {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: self.lease.workspace_id,
                });
            }
            if store.workspace_id() != workspace_id {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: store.workspace_id(),
                });
            }
            if self.lease.archive_root != store.root_path() {
                return Err(ProjectionError::UnsafePath(format!(
                    "applier slot is leased from archive {} but the rebuild source archive is {}",
                    self.lease.archive_root.display(),
                    store.root_path().display()
                )));
            }
            // Authority boundary: opening a database under this slot is the act
            // the workspace lock exists to make exclusive, so the lock must
            // still be on the file this archive's lease pathname names. The
            // cheap identity comparisons run first, so a lease that is not this
            // archive's is refused without any filesystem work.
            self.lease.revalidate_identity()
        }
    }

    impl Drop for SqliteApplierSlot<'_> {
        fn drop(&mut self) {
            self.lease
                .applier_slot_vended
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    /// The database-adjacent applier lock.
    ///
    /// Exactly the second lock the previous combined `ProcessLease` took, with the
    /// same `.<database>.database-applier.lock` name, the same no-follow open, and
    /// the same validators, so the one-applier-per-database guarantee is unchanged.
    /// Acquiring it now requires an [`SqliteApplierSlot`], which is why a database
    /// applier can exist only under a live archive-rooted workspace lease.
    struct DatabaseApplierLease {
        file: File,
        /// The archive lock this database applier was authorized by, so its
        /// release can be observed against a live workspace lease.
        #[cfg(test)]
        workspace_lease_path: PathBuf,
    }

    impl DatabaseApplierLease {
        fn acquire(
            slot: &SqliteApplierSlot<'_>,
            store: &ObjectStore,
            database_path: &Path,
            workspace_id: WorkspaceId,
        ) -> Result<Self, ProjectionError> {
            slot.authorize(store, workspace_id)?;
            let file_name = database_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    ProjectionError::UnsafePath("database file name is not UTF-8".into())
                })?;
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
            let file = lock_capability_lease_file(
                &database_parent,
                database_lease_name,
                &database_lease_path,
            )?;
            Ok(Self {
                file,
                #[cfg(test)]
                workspace_lease_path: slot.lease.lease_path.clone(),
            })
        }
    }

    impl Drop for DatabaseApplierLease {
        fn drop(&mut self) {
            let _ = self.file.unlock();
            #[cfg(test)]
            record_applier_lock_release(|| ApplierLockRelease::Database {
                workspace_still_contended: workspace_lock_is_contended(&self.workspace_lease_path),
            });
        }
    }

    /// The applier locks retained by one live [`SqliteFrontier`].
    ///
    /// `owned_workspace` is populated only by the compatibility entry points, whose
    /// callers do not yet retain a session [`WorkspaceRuntimeLease`]; those
    /// frontiers privately own the workspace lock for their whole life, exactly as
    /// the previous combined `ProcessLease` did. Session-owned frontiers leave it
    /// `None`, because the caller's retained lease already outlives them: the
    /// applier slot is moved into [`LeasedSqliteFrontier`], which the borrow
    /// checker will not let escape that lease.
    pub(super) struct HeldApplierLocks {
        _database: DatabaseApplierLease,
        _owned_workspace: Option<WorkspaceRuntimeLease>,
    }

    impl HeldApplierLocks {
        /// Take a private workspace runtime lease, use its one applier slot to take
        /// the database-adjacent lock in the historical order, then retain the
        /// workspace lease for the frontier's whole life.
        ///
        /// The slot is released back into that private lease, which no other code
        /// can reach, so the released availability is unobservable and the workspace
        /// lock stays held by the same OS handle until the frontier drops.
        fn acquire_owning_workspace(
            store: &ObjectStore,
            database_path: &Path,
            workspace_id: WorkspaceId,
        ) -> Result<Self, ProjectionError> {
            let workspace = WorkspaceRuntimeLease::acquire(store, workspace_id)?;
            let database = {
                let slot = workspace.applier_slot()?;
                DatabaseApplierLease::acquire(&slot, store, database_path, workspace_id)?
            };
            Ok(Self {
                _database: database,
                _owned_workspace: Some(workspace),
            })
        }

        fn acquire_from_slot(
            slot: &SqliteApplierSlot<'_>,
            store: &ObjectStore,
            database_path: &Path,
            workspace_id: WorkspaceId,
        ) -> Result<Self, ProjectionError> {
            Ok(Self {
                _database: DatabaseApplierLease::acquire(slot, store, database_path, workspace_id)?,
                _owned_workspace: None,
            })
        }
    }

    /// How one open/rebuild call proves it may run this workspace's applier.
    pub(super) enum ApplierAuthorization<'slot, 'lease> {
        /// Compatibility path: acquire a temporary workspace runtime lease inside
        /// the call and keep it inside the returned frontier.
        OwnWorkspaceLease,
        /// Session-owned path: the caller retains the workspace runtime lease and
        /// lends its single applier slot.
        Slot(&'slot SqliteApplierSlot<'lease>),
    }

    impl ApplierAuthorization<'_, '_> {
        pub(super) fn acquire(
            &self,
            store: &ObjectStore,
            database_path: &Path,
            workspace_id: WorkspaceId,
        ) -> Result<Arc<HeldApplierLocks>, ProjectionError> {
            Ok(Arc::new(match self {
                Self::OwnWorkspaceLease => {
                    HeldApplierLocks::acquire_owning_workspace(store, database_path, workspace_id)?
                }
                Self::Slot(slot) => {
                    HeldApplierLocks::acquire_from_slot(slot, store, database_path, workspace_id)?
                }
            }))
        }
    }

    /// One runtime session's inseparable pair: the archive-rooted workspace
    /// runtime lease, and the single device-local SQLite projection opened under
    /// that exact lease's applier slot.
    ///
    /// This is the owning answer to "a database handle and the lease that
    /// authorized it must not be able to drift apart". [`LeasedSqliteFrontier`]
    /// proves it at compile time but borrows the lease, so it cannot be stored
    /// beside the lease it borrows. This value stores the lease itself and keeps
    /// the projection next to it:
    ///
    /// * every projection here was opened through [`WorkspaceRuntimeLease::applier_slot`]
    ///   on *this* lease, because [`Self::open_under`] is the only constructor and
    ///   it vends the slot itself;
    /// * the lease field is private to this sealed module and no accessor hands
    ///   out `&WorkspaceRuntimeLease`, so no second applier slot, and therefore no
    ///   second database, can ever be vended while a projection is held;
    /// * the declared field order is load bearing: the projection (and its
    ///   database-adjacent lock) drops before the workspace lease, so a drop or an
    ///   unwind releases the two locks in the reverse of the acquisition order and
    ///   the archive stays contended for as long as this process still has a
    ///   database applier alive. That order is not left to a comment:
    ///   `a_leased_workspace_projection_releases_the_database_lock_before_the_workspace_lease`
    ///   observes both releases and asserts the archive was still contended at
    ///   the instant the database lock went away;
    /// * [`Self::close_retaining_lease`] is the only way to reach the lease
    ///   again, and it requires the projection to be closed first, which is
    ///   exactly the bootstrap -> promoted database handoff without an instant
    ///   of released workspace lock.
    pub(crate) struct LeasedWorkspaceProjection {
        projection: OpenProjection,
        lease: WorkspaceRuntimeLease,
    }

    impl LeasedWorkspaceProjection {
        /// Open one database under `lease`'s single applier slot and retain both.
        ///
        /// `open` receives the slot and must consume it into the
        /// [`LeasedOpenProjection`] it returns, which is the compile-time proof
        /// that the database it produced is the one that slot authorized. On any
        /// failure the slot is released back into the lease and the lease itself
        /// is handed back to the caller, which is what makes a failed open
        /// retryable without leaking the workspace lock.
        pub(crate) fn open_under<T, E>(
            lease: WorkspaceRuntimeLease,
            open: impl for<'lease> FnOnce(
                SqliteApplierSlot<'lease>,
            ) -> Result<(LeasedOpenProjection<'lease>, T), E>,
        ) -> Result<(Self, T), (WorkspaceRuntimeLease, E)>
        where
            E: From<ProjectionError>,
        {
            // The slot borrows `lease`, so every value derived from it must be
            // dead before the lease can be moved into `Self` or handed back.
            let opened: Result<(OpenProjection, T), E> = (|| {
                let slot = lease.applier_slot().map_err(E::from)?;
                let (opened, value) = open(slot)?;
                let (projection, slot) = opened.into_parts();
                // The slot goes back to this lease, which nothing outside this
                // value can reach for as long as the projection is held.
                drop(slot);
                Ok((projection, value))
            })();
            match opened {
                Ok((projection, value)) => Ok((Self { projection, lease }, value)),
                Err(error) => Err((lease, error)),
            }
        }

        /// Close the database and keep the workspace lease.
        ///
        /// The workspace lock is a distinct OS handle from the database-adjacent
        /// lock and is never touched here, so there is no instant between this
        /// database and the next one opened from the returned lease in which
        /// another process — under any app-data or XDG root — could acquire this
        /// archive's workspace lease. `local_active::InactiveBootstrapRuntimeSession::promote`
        /// is the caller: it is the bootstrap -> promoted database handoff.
        pub(crate) fn close_retaining_lease(self) -> WorkspaceRuntimeLease {
            let Self { projection, lease } = self;
            drop(projection);
            lease
        }

        /// Non-forgeable evidence that this process holds the archive-rooted
        /// workspace runtime lease for one exact workspace and archive right now.
        pub(crate) const fn workspace_proof(&self) -> WorkspaceRuntimeProof<'_> {
            WorkspaceRuntimeProof { lease: &self.lease }
        }

        /// Fail closed unless the retained workspace lock is still on the file
        /// this archive's lease pathname names.
        ///
        /// This is the hook the promoted runtime's own authority boundaries use
        /// (see `local_active`'s "Workspace lease identity" documentation); it
        /// asks nothing about the archive, because those boundaries have already
        /// proved the archive by other means.
        pub(crate) fn revalidate_workspace_lease_identity(&self) -> Result<(), ProjectionError> {
            self.lease.revalidate_identity()
        }

        pub(crate) const fn projection(&self) -> &OpenProjection {
            &self.projection
        }

        pub(crate) const fn database(&self) -> &SqliteFrontier {
            &self.projection.database
        }

        /// Split this value into the mutable database handle and the borrowed
        /// while-held identity check for the lease that authorized it.
        ///
        /// The two borrows are disjoint fields, which is what lets a promoted
        /// runtime hand a caller `&mut SqliteFrontier` *and* still re-prove
        /// workspace ownership at its own drain boundary. The identity check
        /// vends no lease and no applier slot, so this does not reopen the
        /// second-database hole the private `lease` field exists to close.
        pub(crate) const fn database_and_lease_identity(
            &mut self,
        ) -> (&mut SqliteFrontier, WorkspaceLeaseIdentity<'_>) {
            (
                &mut self.projection.database,
                WorkspaceLeaseIdentity { lease: &self.lease },
            )
        }
    }

    /// The while-held workspace-lease identity check, borrowed on its own.
    ///
    /// It exposes exactly one operation and no identity, no archive, no applier
    /// slot, and no lease. It exists so a runtime that has already split its
    /// leased projection into a mutable database handle can still fail closed
    /// when the lock it holds stops being the lock on its own lease pathname.
    pub(crate) struct WorkspaceLeaseIdentity<'lease> {
        lease: &'lease WorkspaceRuntimeLease,
    }

    impl WorkspaceLeaseIdentity<'_> {
        pub(crate) fn revalidate(&self) -> Result<(), ProjectionError> {
            self.lease.revalidate_identity()
        }
    }

    /// Borrowed proof of live archive-rooted workspace ownership.
    ///
    /// It exposes the lease's identity and its fail-closed authorization check
    /// and nothing else — in particular no applier slot — so a durable state
    /// mutation can *require* workspace ownership without gaining the ability to
    /// open a second database behind the runtime's back. It cannot outlive the
    /// lease, cannot be cloned into a longer life, and has no constructor
    /// outside this sealed module.
    pub(crate) struct WorkspaceRuntimeProof<'lease> {
        lease: &'lease WorkspaceRuntimeLease,
    }

    impl WorkspaceRuntimeProof<'_> {
        pub(crate) const fn workspace_id(&self) -> WorkspaceId {
            self.lease.workspace_id
        }

        /// Fail closed unless this proof is the workspace lease for the exact
        /// workspace and the exact archive named by `store`.
        pub(crate) fn authorize_archive(
            &self,
            store: &ObjectStore,
            workspace_id: WorkspaceId,
        ) -> Result<(), ProjectionError> {
            if self.lease.workspace_id != workspace_id {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: self.lease.workspace_id,
                });
            }
            if store.workspace_id() != workspace_id {
                return Err(ProjectionError::WorkspaceMismatch {
                    expected: workspace_id,
                    found: store.workspace_id(),
                });
            }
            if self.lease.archive_root != store.root_path() {
                return Err(ProjectionError::UnsafePath(format!(
                    "workspace runtime lease is rooted at archive {} but the runtime archive is {}",
                    self.lease.archive_root.display(),
                    store.root_path().display()
                )));
            }
            // Authority boundary: this proof is what the bootstrap -> promoted
            // lease handover and the crash-takeover compare-and-swap consume, so
            // a lock that no longer refers to the file this pathname names must
            // not authorize either. The cheap identity comparisons run first, so
            // a foreign lease is still refused without any filesystem work.
            self.lease.revalidate_identity()
        }
    }
}

#[cfg(test)]
pub(crate) use applier_lease::{
    clear_workspace_lease_open_hook_for_test, recorded_applier_lock_releases,
    set_workspace_lease_open_hook_for_test, workspace_lock_is_contended, ApplierLockRelease,
};
use applier_lease::{ApplierAuthorization, HeldApplierLocks};
pub(crate) use applier_lease::{
    LeasedWorkspaceProjection, SqliteApplierSlot, WorkspaceLeaseIdentity, WorkspaceRuntimeLease,
    WorkspaceRuntimeProof,
};

#[cfg(test)]
thread_local! {
    /// Archive-rooted workspace runtime lease acquisitions performed by this
    /// thread. A retained runtime must acquire exactly one for its whole life,
    /// so this counter is what proves the lease is not silently reacquired per
    /// mutation.
    ///
    /// Incremented once per *successful* acquisition — after the lock and after
    /// the stable-identity check agree — so neither a contended attempt nor a
    /// bounded identity retry can inflate it.
    static WORKSPACE_RUNTIME_LEASE_ACQUISITIONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    /// While-held workspace-lease identity revalidations performed by this
    /// thread: one `lstat`-equivalent of the held handle plus one no-follow
    /// resolution of the exact lease pathname each.
    ///
    /// This is the instrument that proves the correction added no
    /// keystroke-proportional filesystem work: the bounded-admission table
    /// asserts it stays at zero across 1, 1,000, and 10,000 admissions.
    static WORKSPACE_LEASE_IDENTITY_REVALIDATIONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    static FAIL_NEXT_WORKSPACE_LEASE_IDENTITY_CHECK: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn workspace_runtime_lease_acquisitions() -> usize {
    WORKSPACE_RUNTIME_LEASE_ACQUISITIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn workspace_lease_identity_revalidations() -> usize {
    WORKSPACE_LEASE_IDENTITY_REVALIDATIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn fail_next_workspace_lease_identity_check() {
    FAIL_NEXT_WORKSPACE_LEASE_IDENTITY_CHECK.with(|fail| fail.set(true));
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
    let file = lock_opened_lease_file(file, display_path)?;
    validate_opened_lease_file(&file, display_path)?;
    Ok(file)
}

/// The archive-rooted workspace lease's own open-then-lock.
///
/// It differs from [`lock_capability_lease_file`] in two ways, both because
/// this lease's file lives inside a replicated archive and can be replaced
/// underneath it:
///
/// * it stops after the lock and leaves [`validate_opened_lease_file`] to the
///   caller, so the ownership/link validators run on the file the caller has
///   already *proved* is the one the lease pathname names. A replaced file's
///   old handle has `nlink == 0`, which would otherwise surface as an
///   unrecoverable unsafe-path error instead of a retryable lost race;
/// * it carries one test-only interposition point between the open and the
///   lock. That gap is exactly the window an out-of-band replacement could win,
///   so the regression proving the window is closed has to be able to act
///   inside it.
fn lock_workspace_lease_file(
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
    #[cfg(test)]
    applier_lease::interpose_workspace_lease_open_for_test(display_path);
    lock_opened_lease_file(file, display_path)
}

fn lock_opened_lease_file(file: File, display_path: &Path) -> Result<File, ProjectionError> {
    if let Err(error) = file.try_lock_exclusive() {
        if matches!(
            error.kind(),
            ErrorKind::WouldBlock | ErrorKind::PermissionDenied
        ) {
            return Err(ProjectionError::LeaseContended(display_path.to_path_buf()));
        }
        return Err(error.into());
    }
    Ok(file)
}

/// One file's stable platform identity.
///
/// This is the value that decides whether an OS lock still refers to the file a
/// pathname resolves to. It is deliberately platform native: a path, a length,
/// or a modification time would all be identical across the replacement this
/// exists to detect, because the replacement's whole point is that the lease
/// file's *bytes* never change.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeaseFileIdentity {
    /// `st_dev`.
    device: u64,
    /// `st_ino`.
    inode: u64,
}

/// One file's stable platform identity.
///
/// `FILE_ID_INFO` is the Windows equivalent of `(st_dev, st_ino)`: a volume
/// serial number plus a 128-bit file id that is stable on NTFS and ReFS. It is
/// read from an open handle, so it identifies the file the handle refers to,
/// not the name it was opened by.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeaseFileIdentity {
    /// `FILE_ID_INFO::VolumeSerialNumber`.
    volume: u64,
    /// `FILE_ID_INFO::FileId`.
    file_id: [u8; 16],
}

/// Targets without an atomic no-follow lease open never reach this type: the
/// lease open itself already fails as unsupported.
#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeaseFileIdentity(());

/// The identity of the file this *held handle* refers to.
#[cfg(unix)]
fn held_file_identity(file: &File, path: &Path) -> Result<LeaseFileIdentity, ProjectionError> {
    let metadata = file.metadata().map_err(|error| {
        ProjectionError::UnsafePath(format!(
            "cannot read the held SQLite applier lease {}: {error}",
            path.display()
        ))
    })?;
    Ok(LeaseFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn held_file_identity(file: &File, path: &Path) -> Result<LeaseFileIdentity, ProjectionError> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information = FILE_ID_INFO::default();
    // SAFETY: `file` owns a live handle for the whole call, and `information`
    // is a live, correctly sized out-parameter of the requested class.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(ProjectionError::UnsafePath(format!(
            "cannot read the held SQLite applier lease {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(LeaseFileIdentity {
        volume: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
fn held_file_identity(_file: &File, path: &Path) -> Result<LeaseFileIdentity, ProjectionError> {
    Err(ProjectionError::UnsafePath(format!(
        "stable lease file identity is unsupported on this target: {}",
        path.display()
    )))
}

/// Resolve the exact workspace-lease pathname again — the way another process
/// opening that pathname would — and report the identity of the file it names
/// right now.
///
/// The walk starts at the retained no-follow archive-root capability and opens
/// every component with no-follow, so it observes a replaced *directory* as
/// well as a replaced lease file, while remaining immune to a substituted
/// archive root (which the runtime's archive-identity proof owns).
fn resolve_lease_file_identity(
    archive_capability: &CapDir,
    workspace_name: &str,
    display_path: &Path,
) -> Result<LeaseFileIdentity, LeasePathResolutionError> {
    let resolve_directory = |parent: &CapDir, name: &str| {
        super::object_store::open_dir_nofollow(parent, name)
            .map_err(|error| classify_lease_directory_resolution(parent, name, display_path, error))
    };
    let lease_namespace = resolve_directory(archive_capability, OBJECT_STORE_LEASE_NAMESPACE)?;
    let sqlite_namespace = resolve_directory(&lease_namespace, SQLITE_WORKSPACE_LEASE_NAMESPACE)?;
    let workspace_root = resolve_directory(&sqlite_namespace, workspace_name)?;
    entry_file_identity(&workspace_root, SQLITE_APPLIER_LEASE_FILE, display_path)
}

/// Structured result of resolving the live pathname of a held workspace lease.
///
/// `Replaced` is positive evidence that the exact name can no longer denote the
/// held regular file: a component or final entry is absent, a component is not
/// a real directory, or the final entry is not a real regular file. `Unavailable`
/// is reserved for I/O that does not establish what the name denotes. Keeping
/// this distinction below the display layer prevents runtime revocation from
/// depending on platform error strings.
#[derive(Debug)]
enum LeasePathResolutionError {
    Replaced(String),
    Unavailable(String),
}

impl LeasePathResolutionError {
    fn into_projection_error(self) -> ProjectionError {
        match self {
            Self::Replaced(error) => ProjectionError::LeaseIdentityReplaced(error),
            Self::Unavailable(error) => ProjectionError::LeaseIdentityUnavailable(error),
        }
    }
}

fn missing_or_wrong_kind(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::IsADirectory
    )
}

fn classify_lease_directory_resolution(
    parent: &CapDir,
    name: &str,
    display_path: &Path,
    error: super::object_store::StoreError,
) -> LeasePathResolutionError {
    let detail = format!(
        "cannot resolve the SQLite applier lease {} without following component {name}: {error}",
        display_path.display()
    );
    match &error {
        super::object_store::StoreError::Io(error) if missing_or_wrong_kind(error) => {
            return LeasePathResolutionError::Replaced(detail);
        }
        super::object_store::StoreError::UnsafeEntry(_) => {
            return LeasePathResolutionError::Replaced(detail);
        }
        _ => {}
    }
    match parent.symlink_metadata(name) {
        Ok(metadata) if !metadata.is_dir() => LeasePathResolutionError::Replaced(detail),
        Err(error) if missing_or_wrong_kind(&error) => LeasePathResolutionError::Replaced(detail),
        _ => LeasePathResolutionError::Unavailable(detail),
    }
}

/// The identity of whatever file `name` resolves to inside `directory` right
/// now, resolved without following a final-component link.
///
/// On Unix this is an `lstat` relative to the retained directory capability. On
/// Windows it opens an exact-path handle with the same default sharing mode the
/// live lease handle was opened with (`FILE_SHARE_READ | FILE_SHARE_WRITE |
/// FILE_SHARE_DELETE`), so the probe is compatible with the live byte-range
/// lock rather than contending with it, and reads the identity from that
/// handle.
#[cfg(unix)]
fn entry_file_identity(
    directory: &CapDir,
    name: &str,
    display_path: &Path,
) -> Result<LeaseFileIdentity, LeasePathResolutionError> {
    let metadata = directory.symlink_metadata(name).map_err(|error| {
        let detail = format!(
            "cannot resolve the SQLite applier lease {} without following its final entry: {error}",
            display_path.display()
        );
        if missing_or_wrong_kind(&error) {
            LeasePathResolutionError::Replaced(detail)
        } else {
            LeasePathResolutionError::Unavailable(detail)
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(LeasePathResolutionError::Replaced(format!(
            "the SQLite applier lease {} no longer names a regular non-link file",
            display_path.display()
        )));
    }
    Ok(LeaseFileIdentity {
        device: CapMetadataExt::dev(&metadata),
        inode: CapMetadataExt::ino(&metadata),
    })
}

#[cfg(windows)]
fn entry_file_identity(
    directory: &CapDir,
    name: &str,
    display_path: &Path,
) -> Result<LeaseFileIdentity, LeasePathResolutionError> {
    let mut options = CapOpenOptions::new();
    options
        .read(true)
        .create(false)
        .truncate(false)
        .follow(FollowSymlinks::No);
    let file = directory
        .open_with(name, &options)
        .map_err(|error| {
            let detail = format!(
                "cannot resolve the SQLite applier lease {} without following its final entry: \
                 {error}",
                display_path.display()
            );
            if missing_or_wrong_kind(&error) {
                LeasePathResolutionError::Replaced(detail)
            } else {
                match directory.symlink_metadata(name) {
                    Ok(metadata)
                        if !metadata.is_file()
                            || CapOsMetadataExt::file_attributes(&metadata)
                                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                                != 0 =>
                    {
                        LeasePathResolutionError::Replaced(detail)
                    }
                    Err(metadata_error) if missing_or_wrong_kind(&metadata_error) => {
                        LeasePathResolutionError::Replaced(detail)
                    }
                    _ => LeasePathResolutionError::Unavailable(detail),
                }
            }
        })?
        .into_std();
    let metadata = file.metadata().map_err(|error| {
        LeasePathResolutionError::Unavailable(format!(
            "cannot inspect the SQLite applier lease {} after resolving it: {error}",
            display_path.display()
        ))
    })?;
    if !metadata.is_file()
        || metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    {
        return Err(LeasePathResolutionError::Replaced(format!(
            "the SQLite applier lease {} no longer names a regular non-reparse file",
            display_path.display()
        )));
    }
    held_file_identity(&file, display_path).map_err(|error| {
        LeasePathResolutionError::Unavailable(format!(
            "cannot identify the resolved SQLite applier lease {}: {error}",
            display_path.display()
        ))
    })
}

#[cfg(not(any(unix, windows)))]
fn entry_file_identity(
    _directory: &CapDir,
    _name: &str,
    display_path: &Path,
) -> Result<LeaseFileIdentity, LeasePathResolutionError> {
    Err(LeasePathResolutionError::Unavailable(format!(
        "stable lease file identity is unsupported on this target: {}",
        display_path.display()
    )))
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

#[cfg(test)]
fn uuid_blob(uuid: &Uuid) -> Vec<u8> {
    uuid.as_bytes().to_vec()
}

fn decode_workspace_id(bytes: &[u8]) -> Result<WorkspaceId, ProjectionError> {
    Ok(WorkspaceId::from_uuid(decode_uuid(bytes)?))
}

#[cfg(test)]
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
    /// The retained lease pathname or held handle was proved to name a
    /// different file. Terminal for the runtime; never self-healed in place.
    LeaseIdentityReplaced(String),
    /// The identity check could not be performed. The current operation fails
    /// closed, but a later proof may retry on the same runtime.
    LeaseIdentityUnavailable(String),
    LeaseContended(PathBuf),
    AuthorityMismatch,
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
    Materialization(String),
    Rebuild(String),
    InjectedFailure,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite projection error: {error}"),
            Self::Io(error) => write!(f, "SQLite projection I/O error: {error}"),
            Self::UnsafePath(error) => write!(f, "unsafe SQLite projection path: {error}"),
            Self::LeaseIdentityReplaced(error) => {
                write!(f, "SQLite workspace lease identity was replaced: {error}")
            }
            Self::LeaseIdentityUnavailable(error) => {
                write!(
                    f,
                    "SQLite workspace lease identity check is unavailable: {error}"
                )
            }
            Self::LeaseContended(path) => {
                write!(f, "SQLite applier lease is held: {}", path.display())
            }
            Self::AuthorityMismatch => {
                write!(
                    f,
                    "SQLite projection is bound to another live engine authority"
                )
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
            Self::Materialization(error) => write!(f, "SQLite materialization failed: {error}"),
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

impl From<SqliteFileSetError> for ProjectionError {
    fn from(value: SqliteFileSetError) -> Self {
        match value {
            SqliteFileSetError::Io(error) => Self::from(error),
            SqliteFileSetError::UnsafePath(error) => Self::UnsafePath(error),
            SqliteFileSetError::Corrupt(error) => Self::Corrupt(error),
            SqliteFileSetError::CheckpointTooLarge { length, limit } => Self::Corrupt(format!(
                "SQLite projection checkpoint is too large: {length} bytes exceeds {limit}"
            )),
            SqliteFileSetError::CandidateRetainedSidecars => {
                Self::Corrupt("checkpointed SQLite candidate retained sidecars".into())
            }
        }
    }
}

impl From<super::StoreError> for ProjectionError {
    fn from(value: super::StoreError) -> Self {
        Self::Rebuild(value.to_string())
    }
}

impl From<super::sqlite_materialization::MaterializationError> for ProjectionError {
    fn from(value: super::sqlite_materialization::MaterializationError) -> Self {
        Self::Materialization(value.to_string())
    }
}

impl From<storage_frontier::FrontierError> for ProjectionError {
    fn from(value: storage_frontier::FrontierError) -> Self {
        match value {
            storage_frontier::FrontierError::Sqlite(error) => Self::Sqlite(error),
            storage_frontier::FrontierError::Materialization(error) => Self::from(
                super::sqlite_materialization::MaterializationError::from(error),
            ),
            storage_frontier::FrontierError::Schema(error) => Self::SchemaMismatch(error),
            storage_frontier::FrontierError::ClaimBytes {
                field: "workspace_id",
                expected,
                found,
            } => match (decode_workspace_id(&expected), decode_workspace_id(&found)) {
                (Ok(expected), Ok(found)) => Self::WorkspaceMismatch { expected, found },
                _ => Self::Corrupt("stored workspace claim has an invalid length".into()),
            },
            storage_frontier::FrontierError::ClaimBytes {
                field: "lineage_digest",
                expected,
                found,
            } => match (
                decode_lineage_digest(&expected),
                decode_lineage_digest(&found),
            ) {
                (Ok(expected), Ok(found)) => Self::LineageMismatch { expected, found },
                _ => Self::Corrupt("stored lineage claim has an invalid length".into()),
            },
            storage_frontier::FrontierError::ClaimBytes { field, .. } => {
                Self::Corrupt(format!("stored {field} claim is invalid"))
            }
            storage_frontier::FrontierError::ClaimVersion {
                field,
                expected,
                found,
            } => Self::ProtocolMismatch {
                field,
                expected,
                found,
            },
            storage_frontier::FrontierError::Corrupt(error) => Self::Corrupt(error),
            storage_frontier::FrontierError::InvalidInput(error) => {
                Self::InvalidAcceptedEvent(error)
            }
            storage_frontier::FrontierError::MissingDependency(batch_id) => {
                Self::MissingDependency(BatchId::from_uuid(Uuid::from_bytes(batch_id)))
            }
            storage_frontier::FrontierError::AcceptanceOrder { expected, found } => {
                Self::AcceptanceOrder { expected, found }
            }
            storage_frontier::FrontierError::FrontierRegression => Self::FrontierRegression,
            storage_frontier::FrontierError::BatchCollision(batch_id) => {
                Self::BatchCollision(BatchId::from_uuid(Uuid::from_bytes(batch_id)))
            }
            storage_frontier::FrontierError::MaterializationCollision(batch_id) => {
                Self::Materialization(
                    super::MaterializationError::DuplicateCollision(BatchId::from_uuid(
                        Uuid::from_bytes(batch_id),
                    ))
                    .to_string(),
                )
            }
            storage_frontier::FrontierError::InjectedFailure => Self::InjectedFailure,
        }
    }
}

impl SqliteFrontier {
    /// Open a read-only reference view at `engine`'s exact accepted frontier.
    /// A SQLite prefix is usable only when its catalog stamp matches its
    /// authenticated frontier; any newer portion is bounded by the existing
    /// tail limits and read from immutable catalog postings.
    pub fn frontier_reference_query<'a>(
        &'a self,
        engine: &'a ShardedHotEngine,
        store: &'a ObjectStore,
    ) -> Result<FrontierReferenceQuery<'a>, ProjectionError> {
        if !self.runtime_authority.matches(engine.runtime_authority()) {
            return Err(ProjectionError::AuthorityMismatch);
        }
        let source = RebuildSource::new(engine, store)?;
        source.authenticate_exact_frontier()?;
        let mut expected = self.frontier_root()?;
        let base_catalog_root = self.authenticated_reference_catalog_root()?;
        let accepted = engine
            .accepted_frontier_root()
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        if expected.acceptance_sequence() > accepted.acceptance_sequence()
            || expected.retained_bytes_total() > accepted.retained_bytes_total()
        {
            return Err(ProjectionError::FrontierRegression);
        }
        let mut tail_sources = BTreeMap::new();
        let mut tail_bytes = 0_usize;
        let mut tail_batches = 0_usize;
        for sequence in
            expected.acceptance_sequence().saturating_add(1)..=accepted.acceptance_sequence()
        {
            let event = source.accepted_event_at(sequence)?;
            if event.prior_frontier_root() != &expected {
                return Err(ProjectionError::Materialization(
                    "SQLite reference query tail does not continue its authenticated frontier"
                        .into(),
                ));
            }
            tail_batches = tail_batches.saturating_add(1);
            tail_bytes = tail_bytes.saturating_add(event.retained_bytes());
            if tail_batches > TAIL_MAX_BATCHES || tail_bytes > TAIL_MAX_BYTES {
                return Err(ProjectionError::Materialization(
                    "SQLite reference query requires an over-limit authenticated tail".into(),
                ));
            }
            let effect = SemanticEffect::decode(event.semantic_effect())
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
            let catalog_root = event.post_frontier_root().reference_catalog_root();
            for source_page_id in super::reference_catalog::affected_reference_sources(&effect) {
                let posting = engine
                    .reference_source_posting_at(catalog_root, source_page_id)
                    .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
                tail_sources.insert(source_page_id, posting);
            }
            expected = event.post_frontier_root().clone();
        }
        if expected != accepted {
            return Err(ProjectionError::Materialization(
                "SQLite reference query tail did not reach the requested authenticated frontier"
                    .into(),
            ));
        }
        Ok(FrontierReferenceQuery {
            database: self,
            engine,
            base_catalog_root,
            instrumentation: ReferenceQueryInstrumentation {
                tail_source_postings: tail_sources.len(),
                ..ReferenceQueryInstrumentation::default()
            },
            tail_sources,
        })
    }
}

impl FrontierReferenceQuery<'_> {
    fn limit(limit: usize) -> Result<usize, ProjectionError> {
        if limit == 0 || limit > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(format!(
                "reference query limit {limit} is outside 1..={}",
                super::MAX_MATERIALIZATION_QUERY_ROWS
            )));
        }
        Ok(limit)
    }

    fn sqlite_candidates_for_names(
        &self,
        names: &BTreeSet<String>,
    ) -> Result<BTreeSet<PageId>, ProjectionError> {
        let hard_limit = super::MAX_MATERIALIZATION_QUERY_ROWS;
        let mut candidates = BTreeSet::new();
        for name in names {
            let physical = self.database.physical.reference_page_candidates_for_name(
                name,
                i64::try_from(hard_limit.saturating_add(1)).map_err(|_| {
                    ProjectionError::Materialization(
                        "reference query candidate limit overflowed".into(),
                    )
                })?,
            )?;
            let sqlite_for_name = physical
                .into_iter()
                .map(|id| Ok(PageId::from_uuid(Uuid::from_bytes(id))))
                .collect::<Result<BTreeSet<_>, ProjectionError>>()?;
            if sqlite_for_name.len() > hard_limit {
                return Err(ProjectionError::Materialization(
                    "reference query candidate source limit exceeded".into(),
                ));
            }
            let logical = super::LogicalPageName::parse(name.clone())
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
            let authenticated = self
                .engine
                .reference_candidates_at(
                    &self.base_catalog_root,
                    super::reference_catalog::ReferenceCandidateTargetV2::PageName(
                        logical.key_digest(),
                    ),
                    hard_limit,
                )
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
            if sqlite_for_name != authenticated {
                return Err(ProjectionError::Materialization(
                    "SQLite page-reference candidates differ from the authenticated reverse catalog"
                        .into(),
                ));
            }
            candidates.extend(authenticated);
            if candidates.len() > hard_limit {
                return Err(ProjectionError::Materialization(
                    "reference query candidate source limit exceeded".into(),
                ));
            }
        }
        Ok(candidates)
    }

    fn sqlite_candidates_for_logseq_uuid(
        &self,
        logseq_uuid: LogseqUuid,
    ) -> Result<BTreeSet<PageId>, ProjectionError> {
        let hard_limit = super::MAX_MATERIALIZATION_QUERY_ROWS;
        let physical = self
            .database
            .physical
            .reference_page_candidates_for_logseq_uuid(
                logseq_uuid.as_uuid().into_bytes(),
                i64::try_from(hard_limit.saturating_add(1)).map_err(|_| {
                    ProjectionError::Materialization(
                        "reference query candidate limit overflowed".into(),
                    )
                })?,
            )?;
        let candidates = physical
            .into_iter()
            .map(|id| PageId::from_uuid(Uuid::from_bytes(id)))
            .collect::<BTreeSet<_>>();
        if candidates.len() > hard_limit {
            return Err(ProjectionError::Materialization(
                "reference query candidate source limit exceeded".into(),
            ));
        }
        let authenticated = self
            .engine
            .reference_candidates_at(
                &self.base_catalog_root,
                super::reference_catalog::ReferenceCandidateTargetV2::BlockUuid(logseq_uuid),
                hard_limit,
            )
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        if candidates != authenticated {
            return Err(ProjectionError::Materialization(
                "SQLite block-reference candidates differ from the authenticated reverse catalog"
                    .into(),
            ));
        }
        Ok(authenticated)
    }

    fn sqlite_alias_candidates(
        &self,
        normalized_alias: &str,
    ) -> Result<BTreeSet<PageId>, ProjectionError> {
        let candidates =
            self.database
                .physical
                .reference_page_candidates_for_alias(
                    normalized_alias,
                    i64::try_from(super::MAX_MATERIALIZATION_QUERY_ROWS.saturating_add(1))
                        .map_err(|_| {
                            ProjectionError::Materialization(
                                "reference alias candidate limit overflowed".into(),
                            )
                        })?,
                )?
                .into_iter()
                .map(|id| PageId::from_uuid(Uuid::from_bytes(id)))
                .collect::<BTreeSet<_>>();
        if candidates.len() > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(
                "reference alias candidate source limit exceeded".into(),
            ));
        }
        let logical = super::LogicalPageName::parse(normalized_alias.to_owned())
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        let authenticated = self
            .engine
            .reference_candidates_at(
                &self.base_catalog_root,
                super::reference_catalog::ReferenceCandidateTargetV2::PageAlias(
                    logical.key_digest(),
                ),
                super::MAX_MATERIALIZATION_QUERY_ROWS,
            )
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        if candidates != authenticated {
            return Err(ProjectionError::Materialization(
                "SQLite alias bindings differ from the authenticated reverse catalog".into(),
            ));
        }
        Ok(authenticated)
    }

    fn current_posting(
        &mut self,
        source_page_id: PageId,
    ) -> Result<super::ReferenceSourcePostingV2, ProjectionError> {
        let posting = self
            .engine
            .reference_source_posting(source_page_id)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
            .ok_or_else(|| {
                ProjectionError::Materialization(
                    "reference candidate has no current authenticated source posting".into(),
                )
            })?;
        self.instrumentation.revalidated_sources =
            self.instrumentation.revalidated_sources.saturating_add(1);
        Ok(posting)
    }

    fn resolve_name(
        &mut self,
        name: &super::LogicalPageName,
    ) -> Result<Option<PageId>, ProjectionError> {
        if let Some(page_id) = self
            .engine
            .resolve_logical_page_name(name)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
        {
            return Ok(Some(page_id));
        }
        let normalized_alias = crate::refs::page_key(name.as_str());
        let mut candidates = self.sqlite_alias_candidates(&normalized_alias)?;
        candidates.retain(|page_id| !self.tail_sources.contains_key(page_id));
        for (page_id, posting) in &self.tail_sources {
            if posting.as_ref().is_some_and(|posting| {
                posting.facts().iter().any(|fact| {
                    matches!(
                        fact,
                        ReferenceFactV1::PageName(fact)
                            if fact.kind == super::PageReferenceKindV1::AliasDeclaration
                                && fact.normalized_target == normalized_alias
                    )
                })
            }) {
                candidates.insert(*page_id);
            }
        }
        if candidates.len() > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(
                "reference alias candidate source limit exceeded".into(),
            ));
        }
        let mut verified = BTreeSet::new();
        for page_id in candidates {
            let posting = self.current_posting(page_id)?;
            if posting.facts().iter().any(|fact| {
                matches!(
                    fact,
                    ReferenceFactV1::PageName(fact)
                        if fact.kind == super::PageReferenceKindV1::AliasDeclaration
                            && fact.normalized_target == normalized_alias
                )
            }) {
                verified.insert(page_id);
            }
        }
        Ok((verified.len() == 1).then(|| *verified.first().expect("one alias candidate")))
    }

    fn names_for_target(
        &mut self,
        requested: &super::LogicalPageName,
        target: Option<PageId>,
    ) -> Result<BTreeSet<String>, ProjectionError> {
        let mut names = BTreeSet::from([crate::refs::page_key(requested.as_str())]);
        let Some(target) = target else {
            return Ok(names);
        };
        let page = self
            .engine
            .materialize_page(target)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        names.insert(crate::refs::page_key(page.name.as_str()));
        let posting = self.current_posting(target)?;
        names.extend(posting.facts().iter().filter_map(|fact| match fact {
            ReferenceFactV1::PageName(fact)
                if fact.kind == super::PageReferenceKindV1::AliasDeclaration =>
            {
                Some(fact.normalized_target.clone())
            }
            ReferenceFactV1::PageName(_) | ReferenceFactV1::Block(_) => None,
        }));
        if names.len() > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(
                "reference target has too many alias names".into(),
            ));
        }
        Ok(names)
    }

    /// Return exact raw page-reference evidence at the current authenticated
    /// frontier.  A missing target intentionally remains queryable as a
    /// dangling textual reference.
    pub fn references_to_page_name(
        &mut self,
        requested: &super::LogicalPageName,
        limit: usize,
    ) -> Result<FrontierReferenceResults, ProjectionError> {
        self.references_to_page_name_inner(requested, limit, false)
    }

    fn references_to_page_name_inner(
        &mut self,
        requested: &super::LogicalPageName,
        limit: usize,
        require_complete: bool,
    ) -> Result<FrontierReferenceResults, ProjectionError> {
        let limit = Self::limit(limit)?;
        let target = self.resolve_name(requested)?;
        let names = self.names_for_target(requested, target)?;
        let mut candidates = self.sqlite_candidates_for_names(&names)?;
        candidates.retain(|page_id| !self.tail_sources.contains_key(page_id));
        candidates.extend(self.tail_sources.iter().filter_map(|(page_id, posting)| {
            posting
                .as_ref()
                .is_some_and(|posting| {
                    posting.facts().iter().any(|fact| {
                        matches!(
                            fact,
                            ReferenceFactV1::PageName(fact)
                                if fact.kind != super::PageReferenceKindV1::AliasDeclaration
                                    && names.contains(&fact.normalized_target)
                        )
                    })
                })
                .then_some(*page_id)
        }));
        if candidates.len() > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(
                "reference query candidate source limit exceeded".into(),
            ));
        }
        let sqlite_candidates = candidates
            .iter()
            .filter(|page_id| !self.tail_sources.contains_key(page_id))
            .count();
        self.instrumentation.sqlite_candidate_sources = self
            .instrumentation
            .sqlite_candidate_sources
            .saturating_add(sqlite_candidates);
        let mut hits = Vec::new();
        let mut output_bytes = 0_usize;
        'sources: for source_page_id in candidates {
            let posting = self.current_posting(source_page_id)?;
            for fact in posting.facts() {
                let ReferenceFactV1::PageName(fact) = fact else {
                    continue;
                };
                if fact.kind == super::PageReferenceKindV1::AliasDeclaration
                    || !names.contains(&fact.normalized_target)
                {
                    continue;
                }
                let resolved_page_id = self.resolve_name(
                    &super::LogicalPageName::parse(fact.raw_target.clone())
                        .map_err(|error| ProjectionError::Materialization(error.to_string()))?,
                )?;
                if target.is_some() && resolved_page_id != target {
                    continue;
                }
                if hits.len() == limit {
                    if require_complete {
                        return Err(ProjectionError::Materialization(
                            "complete reference query result limit exceeded".into(),
                        ));
                    }
                    break 'sources;
                }
                let hit = FrontierReferenceHit {
                    source_page_id,
                    fact: ReferenceFactV1::PageName(fact.clone()),
                    resolved_page_id,
                    resolved_block_id: None,
                };
                retain_reference_hit_bounded(&mut output_bytes, &hit)?;
                hits.push(hit);
            }
        }
        hits.sort_unstable_by(|left, right| {
            (left.source_page_id, &left.fact).cmp(&(right.source_page_id, &right.fact))
        });
        Ok(FrontierReferenceResults {
            hits,
            instrumentation: self.instrumentation,
        })
    }

    /// Return raw block-reference evidence for an exact Logseq UUID claim.
    /// UUID binding is always resolved through the authenticated engine; an
    /// unclaimed or ambiguous UUID remains visible but never gains an active
    /// target from SQLite.
    pub fn references_to_logseq_uuid(
        &mut self,
        logseq_uuid: LogseqUuid,
        limit: usize,
    ) -> Result<FrontierReferenceResults, ProjectionError> {
        let limit = Self::limit(limit)?;
        let resolved_block_id = match self
            .engine
            .resolve_logseq_uuid(logseq_uuid)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
        {
            LogseqUuidResolution::Unique(claim) => Some(claim.block_id),
            LogseqUuidResolution::Unclaimed | LogseqUuidResolution::Ambiguous { .. } => None,
        };
        let mut candidates = self.sqlite_candidates_for_logseq_uuid(logseq_uuid)?;
        candidates.retain(|page_id| !self.tail_sources.contains_key(page_id));
        candidates.extend(self.tail_sources.iter().filter_map(|(page_id, posting)| {
            posting
                .as_ref()
                .is_some_and(|posting| {
                    posting.facts().iter().any(|fact| {
                        matches!(fact, ReferenceFactV1::Block(fact) if fact.logseq_uuid == logseq_uuid)
                    })
                })
                .then_some(*page_id)
        }));
        if candidates.len() > super::MAX_MATERIALIZATION_QUERY_ROWS {
            return Err(ProjectionError::Materialization(
                "reference query candidate source limit exceeded".into(),
            ));
        }
        let sqlite_candidates = candidates
            .iter()
            .filter(|page_id| !self.tail_sources.contains_key(page_id))
            .count();
        self.instrumentation.sqlite_candidate_sources = self
            .instrumentation
            .sqlite_candidate_sources
            .saturating_add(sqlite_candidates);
        let mut hits = Vec::new();
        let mut output_bytes = 0_usize;
        'sources: for source_page_id in candidates {
            let posting = self.current_posting(source_page_id)?;
            for fact in posting.facts() {
                let ReferenceFactV1::Block(fact) = fact else {
                    continue;
                };
                if fact.logseq_uuid != logseq_uuid {
                    continue;
                }
                if hits.len() == limit {
                    break 'sources;
                }
                let hit = FrontierReferenceHit {
                    source_page_id,
                    fact: ReferenceFactV1::Block(fact.clone()),
                    resolved_page_id: None,
                    resolved_block_id,
                };
                retain_reference_hit_bounded(&mut output_bytes, &hit)?;
                hits.push(hit);
            }
        }
        hits.sort_unstable_by(|left, right| {
            (left.source_page_id, &left.fact).cmp(&(right.source_page_id, &right.fact))
        });
        Ok(FrontierReferenceResults {
            hits,
            instrumentation: self.instrumentation,
        })
    }

    /// Build a rename transaction from the target plus reverse-indexed
    /// referrers.  Every candidate is reread from the authenticated catalog
    /// and its exact current source bytes are checked before the transaction is
    /// returned; SQLite therefore cannot authorize a projection write.
    pub fn plan_page_rename(
        &mut self,
        old_name: &super::LogicalPageName,
        new_name: super::LogicalPageName,
        new_path: super::ManagedPath,
    ) -> Result<FrontierRenamePlan, ProjectionError> {
        let target_page_id = self
            .engine
            .resolve_logical_page_name(old_name)
            .map_err(|error| ProjectionError::Materialization(error.to_string()))?
            .ok_or_else(|| {
                ProjectionError::Materialization(
                    "rename target has no authenticated exact page-name owner".into(),
                )
            })?;
        let results = self.references_to_page_name_inner(
            old_name,
            super::MAX_MATERIALIZATION_QUERY_ROWS,
            true,
        )?;
        let mut preamble_facts = BTreeMap::<PageId, Vec<super::PageNameReferenceFactV1>>::new();
        let mut block_facts = BTreeMap::<
            (PageId, DocumentId, super::BlockId),
            Vec<super::PageNameReferenceFactV1>,
        >::new();
        let mut touched = BTreeSet::from([target_page_id]);
        for hit in &results.hits {
            let ReferenceFactV1::PageName(fact) = &hit.fact else {
                continue;
            };
            if matches!(
                fact.kind,
                super::PageReferenceKindV1::AliasDeclaration
                    | super::PageReferenceKindV1::PropertyKeyPseudoPage
            ) {
                continue;
            }
            touched.insert(hit.source_page_id);
            match fact.source {
                ReferenceSourceLocatorV1::Preamble => {
                    preamble_facts
                        .entry(hit.source_page_id)
                        .or_default()
                        .push(fact.clone());
                }
                ReferenceSourceLocatorV1::Block {
                    block_id,
                    home_document_id,
                } => {
                    block_facts
                        .entry((hit.source_page_id, home_document_id, block_id))
                        .or_default()
                        .push(fact.clone());
                }
            }
        }
        let mut page_preamble_rewrites = Vec::new();
        let mut block_rewrites = Vec::new();
        for source_page_id in &touched {
            let posting = self.current_posting(*source_page_id)?;
            let page = self
                .engine
                .materialize_page(*source_page_id)
                .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
            if let Some(facts) = preamble_facts.get(source_page_id) {
                verify_current_page_facts(&posting, facts)?;
                let source = page.preamble.as_deref().ok_or_else(|| {
                    ProjectionError::Materialization(
                        "rename preamble candidate has no current source bytes".into(),
                    )
                })?;
                page_preamble_rewrites.push(super::PagePreambleRewrite {
                    page_id: *source_page_id,
                    new_preamble: Some(rewrite_raw_page_targets(source, facts, new_name.as_str())?),
                });
            }
            for ((page_id, home_document_id, block_id), facts) in block_facts
                .iter()
                .filter(|((page_id, _, _), _)| page_id == source_page_id)
            {
                verify_current_page_facts(&posting, facts)?;
                let block = page
                    .blocks
                    .iter()
                    .find(|block| {
                        block.block_id == *block_id && block.home_document_id == *home_document_id
                    })
                    .ok_or_else(|| {
                        ProjectionError::Materialization(
                            "rename block candidate has no current source state".into(),
                        )
                    })?;
                block_rewrites.push(super::BlockContentRewrite {
                    block: super::BlockLocation {
                        block_id: *block_id,
                        home_document_id: *home_document_id,
                    },
                    new_content: rewrite_raw_page_targets(
                        &block.content,
                        facts,
                        new_name.as_str(),
                    )?,
                });
                debug_assert_eq!(*page_id, *source_page_id);
            }
        }
        block_rewrites.sort_unstable_by_key(|rewrite| {
            (rewrite.block.home_document_id, rewrite.block.block_id)
        });
        page_preamble_rewrites.sort_unstable_by_key(|rewrite| rewrite.page_id);
        let transaction = super::OperationTransaction::new(vec![
            super::SemanticOperation::RenamePagesAndRewriteReferrers {
                page_changes: vec![super::PageRename {
                    page_id: target_page_id,
                    new_name,
                    new_path,
                }],
                block_rewrites,
                page_preamble_rewrites,
            },
        ])
        .map_err(|error| ProjectionError::Materialization(error.to_string()))?;
        Ok(FrontierRenamePlan {
            target_page_id,
            transaction,
            touched_sources: touched.into_iter().collect(),
            instrumentation: self.instrumentation,
        })
    }
}

fn verify_current_page_facts(
    posting: &super::ReferenceSourcePostingV2,
    facts: &[super::PageNameReferenceFactV1],
) -> Result<(), ProjectionError> {
    if facts.iter().any(|fact| {
        !posting
            .facts()
            .contains(&ReferenceFactV1::PageName(fact.clone()))
    }) {
        return Err(ProjectionError::Materialization(
            "rename candidate no longer matches its authenticated source posting".into(),
        ));
    }
    Ok(())
}

fn rewrite_raw_page_targets(
    source: &str,
    facts: &[super::PageNameReferenceFactV1],
    replacement: &str,
) -> Result<String, ProjectionError> {
    let mut facts = facts.to_vec();
    facts.sort_unstable_by(|left, right| {
        right
            .byte_start
            .cmp(&left.byte_start)
            .then_with(|| right.byte_end.cmp(&left.byte_end))
    });
    let mut next_start = source.len();
    let mut rewritten = source.to_owned();
    for fact in facts {
        let span_start = usize::try_from(fact.byte_start).map_err(|_| {
            ProjectionError::Materialization("reference source offset is invalid".into())
        })?;
        let span_end = usize::try_from(fact.byte_end).map_err(|_| {
            ProjectionError::Materialization("reference source offset is invalid".into())
        })?;
        if span_start >= span_end || span_end > next_start {
            return Err(ProjectionError::Materialization(
                "rename candidate source bytes no longer match raw reference evidence".into(),
            ));
        }
        let span = rewritten.get(span_start..span_end).ok_or_else(|| {
            ProjectionError::Materialization(
                "rename candidate source bytes no longer match raw reference evidence".into(),
            )
        })?;
        let mut occurrences = span.match_indices(&fact.raw_target);
        let Some((offset, _)) = occurrences.next() else {
            return Err(ProjectionError::Materialization(
                "rename candidate source bytes no longer match raw reference evidence".into(),
            ));
        };
        if occurrences.next().is_some() {
            return Err(ProjectionError::Materialization(
                "rename candidate source range has ambiguous raw reference evidence".into(),
            ));
        }
        let start = span_start.checked_add(offset).ok_or_else(|| {
            ProjectionError::Materialization("reference source offset overflowed".into())
        })?;
        let end = start.checked_add(fact.raw_target.len()).ok_or_else(|| {
            ProjectionError::Materialization("reference source offset overflowed".into())
        })?;
        rewritten.replace_range(start..end, replacement);
        next_start = span_start;
    }
    Ok(rewritten)
}

#[cfg(test)]
pub(crate) fn corrupt_equal_length_interior_block_payload(
    database_path: &Path,
    original: &[u8],
    counterfeit: &[u8],
) -> usize {
    corrupt_equal_length_interior_block_payload_with_coverage(
        database_path,
        original,
        counterfeit,
        false,
    )
}

#[cfg(test)]
fn corrupt_equal_length_sampled_interior_block_payload(
    database_path: &Path,
    original: &[u8],
    counterfeit: &[u8],
) -> usize {
    corrupt_equal_length_interior_block_payload_with_coverage(
        database_path,
        original,
        counterfeit,
        true,
    )
}

#[cfg(test)]
fn corrupt_equal_length_interior_block_payload_with_coverage(
    database_path: &Path,
    original: &[u8],
    counterfeit: &[u8],
    select_sampled: bool,
) -> usize {
    assert_eq!(
        original.len(),
        counterfeit.len(),
        "interior corruption helper requires an equal-length replacement"
    );
    assert!(
        !original.is_empty(),
        "interior corruption helper requires a non-empty payload"
    );
    let connection = Connection::open(database_path).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let page_size: usize = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, usize>(0))
        .unwrap();
    let block_pages = {
        let mut statement = connection
            .prepare("SELECT pageno FROM dbstat WHERE name = 'blocks' ORDER BY pageno")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, usize>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    drop(connection);

    let mut bytes = fs::read(database_path).unwrap();
    assert!(
        bytes.len() > storage_frontier::SQLITE_CHECKPOINT_EDGE_BYTES * 2,
        "fixture database is too small to have an edge-excluded interior"
    );
    let mut patched = 0;
    let sampled_ranges =
        storage_frontier::physical_checkpoint_interior_ranges_for_test(bytes.len() as u64);
    let interior_start = storage_frontier::SQLITE_CHECKPOINT_EDGE_BYTES;
    let interior_end = bytes.len() - storage_frontier::SQLITE_CHECKPOINT_EDGE_BYTES;
    for page in block_pages {
        let page_start = page.saturating_sub(1).saturating_mul(page_size);
        let start = page_start.max(interior_start);
        let end = page_start
            .saturating_add(page_size)
            .min(interior_end)
            .saturating_sub(original.len());
        if start > end {
            continue;
        }
        for offset in start..=end {
            if &bytes[offset..offset + original.len()] != original {
                continue;
            }
            let replacement_end = offset + counterfeit.len();
            let fully_sampled = sampled_ranges.iter().any(|(range_offset, range_length)| {
                let range_start = usize::try_from(*range_offset).unwrap();
                let range_end = range_start + range_length;
                offset >= range_start && replacement_end <= range_end
            });
            if !select_sampled || fully_sampled {
                bytes[offset..replacement_end].copy_from_slice(counterfeit);
                patched += 1;
            }
        }
    }
    assert!(
        patched > 0,
        "fixture found no block-table payload in the selected database interior coverage"
    );
    fs::write(database_path, bytes).unwrap();
    patched
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead as _, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::oplog::{
        AuthorBatch, BatchCausalDot, BatchDisposition, BatchOrigin, BlockId, BlockLocation,
        CausalPeerId, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies, DocumentId,
        ManagedPath, ManagedTextKind, MaterializationChange, MaterializedBlockInput,
        MaterializedEntityId, MaterializedPageInput, MaterializedProperty, MaterializedReference,
        MaterializedReferenceKind, MaterializedReferrerRow, MaterializedTask, OperationBatch,
        OperationObject, OperationTransaction, PageId, PageRename, PreparedBatch,
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
        root_transaction_named(ids, path, "Root Fixture Page", content)
    }

    fn root_transaction_named(
        ids: TestIds,
        path: &str,
        name: &str,
        content: &str,
    ) -> OperationTransaction {
        OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                name: crate::oplog::LogicalPageName::parse(name).unwrap(),
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

    fn rich_materialization(
        event: &AcceptedBatchEvent,
        ids: TestIds,
        path: &str,
        kind: ManagedTextKind,
        name: &str,
        content: &str,
    ) -> MaterializationChange {
        MaterializationChange::new(
            event.batch_id(),
            vec![MaterializedPageInput {
                page_id: ids.page,
                home_document_id: ids.document,
                name: name.into(),
                name_key: name.to_lowercase(),
                path: ManagedPath::parse(path).unwrap(),
                kind,
                preamble: None,
                searchable_text: format!("{name} page searchable"),
                references: vec![MaterializedReference {
                    target: MaterializedEntityId::Page(ids.page),
                    kind: MaterializedReferenceKind::PropertyReference,
                }],
                properties: vec![MaterializedProperty {
                    name: "alias".into(),
                    value: format!("{name} Alias"),
                }],
                tags: vec!["page-tag".into()],
                blocks: vec![MaterializedBlockInput {
                    block_id: ids.block,
                    home_document_id: ids.document,
                    parent: None,
                    order: "a".into(),
                    content: content.into(),
                    searchable_text: format!("{content} needle"),
                    heading_level: Some(2),
                    collapsed: true,
                    logseq_uuid: None,
                    logseq_identity_origin: None,
                    references: vec![
                        MaterializedReference {
                            target: MaterializedEntityId::Page(ids.page),
                            kind: MaterializedReferenceKind::Reference,
                        },
                        MaterializedReference {
                            target: MaterializedEntityId::Block(ids.block),
                            kind: MaterializedReferenceKind::Embed,
                        },
                    ],
                    properties: vec![MaterializedProperty {
                        name: "owner".into(),
                        value: "Ada".into(),
                    }],
                    tags: vec!["block-tag".into()],
                    task: Some(MaterializedTask {
                        marker: "TODO".into(),
                        priority: Some("A".into()),
                        scheduled: Some("2026-07-25 Sat".into()),
                        deadline: Some("2026-07-26 Sun".into()),
                    }),
                }],
            }],
            Vec::new(),
        )
        .unwrap()
    }

    fn publish_and_stage(
        engine: &mut ShardedHotEngine,
        store: &ObjectStore,
        prepared: &PreparedBatch,
    ) {
        store.publish_bootstrap_prepared_for_test(prepared).unwrap();
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
        store.publish_bootstrap_prepared_for_test(prepared).unwrap();
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
        store
            .publish_bootstrap_prepared_for_test(&prepared)
            .unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut accepted_engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        assert!(matches!(
            accepted_engine
                .stage_archive_batch(prepared.manifest().batch_id())
                .unwrap()
                .disposition,
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

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct OverlayLogicalState {
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
        authenticated_source_frontier: Option<AcceptedFrontierRoot>,
    }

    fn overlay_logical_state(overlay: &TailOverlay) -> OverlayLogicalState {
        OverlayLogicalState {
            hot_descriptors: overlay.hot_descriptors.clone(),
            retained_bytes: overlay.retained_bytes,
            authoritative_retained_bytes_total: overlay.authoritative_retained_bytes_total,
            applied_retained_bytes_total: overlay.applied_retained_bytes_total,
            authoritative_through: overlay.authoritative_through,
            applied_through: overlay.applied_through,
            descriptor_overflow: overlay.descriptor_overflow,
            reservations: overlay.reservations.clone(),
            reserved_bytes: overlay.reserved_bytes,
            next_reservation_id: overlay.next_reservation_id,
            authenticated_source_frontier: overlay.authenticated_source_frontier.clone(),
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct DatabaseLogicalState {
        frontier_root: AcceptedFrontierRoot,
        required_frontier_root: AcceptedFrontierRoot,
        applied_batch_count: usize,
        semantic_digest: ContentDigest,
    }

    fn database_logical_state(database: &SqliteFrontier) -> DatabaseLogicalState {
        DatabaseLogicalState {
            frontier_root: database.frontier_root().unwrap(),
            required_frontier_root: database.required_frontier_root.clone(),
            applied_batch_count: database.applied_batch_count().unwrap(),
            semantic_digest: database.semantic_projection_digest().unwrap(),
        }
    }

    fn inspect_connection(database: &SqliteFrontier) -> Connection {
        Connection::open(database.path()).unwrap()
    }

    fn stored_semantic_effects(database: &SqliteFrontier) -> Vec<SemanticEffect> {
        database
            .physical
            .stored_semantic_effects()
            .unwrap()
            .into_iter()
            .map(|bytes| SemanticEffect::decode(&bytes).unwrap())
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
        store
            .publish_bootstrap_prepared_for_test(&prepared)
            .unwrap();
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

        if mode == "workspace-lease-probe" {
            let runtime = ApplicationRuntimeRoot::open().unwrap();
            let engine = ids.engine();
            let stdin = std::io::stdin();
            let mut requests = stdin.lock().lines();
            let mut answers = std::io::stdout();
            while let Some(request) = requests.next() {
                assert_eq!(request.unwrap(), "probe");
                let answer = match SqliteFrontier::open_or_rebuild(
                    &root.join("probe/frontier.sqlite"),
                    &runtime,
                    ids.claim(),
                    RebuildSource::new(&engine, &store).unwrap(),
                ) {
                    Ok(opened) => {
                        drop(opened);
                        "acquired"
                    }
                    Err(ProjectionError::LeaseContended(_)) => "contended",
                    Err(error) => panic!("unexpected workspace lease probe error: {error}"),
                };
                writeln!(answers, "{WORKSPACE_LEASE_PROBE_MARKER}{answer}").unwrap();
                answers.flush().unwrap();
            }
            return;
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
            let engine_store = ObjectStore::open(&root.join("objects"), ids.workspace).unwrap();
            let mut accepted_engine =
                ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
            for manifest in store.committed_manifests().unwrap() {
                assert!(matches!(
                    accepted_engine
                        .stage_archive_batch(manifest.batch_id())
                        .unwrap()
                        .disposition,
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
        let engine_store = ObjectStore::open(&root.join("objects"), ids.workspace).unwrap();
        let mut accepted_engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        assert!(matches!(
            accepted_engine
                .stage_archive_batch(batch_id)
                .unwrap()
                .disposition,
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
            .physical
            .disable_wal_autocheckpoint_for_test()
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
        let connection = inspect_connection(&database);
        let application_id: u32 = connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap();
        let user_version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let journal_mode: String = connection
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
        drop(connection);
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
            match case {
                "type" | "primary-key" | "check" | "strict" => {
                    let (expected, corrupted) = match case {
                        "type" => ("batch_id BLOB", "batch_id TEXT"),
                        "primary-key" => {
                            ("sequence INTEGER PRIMARY KEY", "sequence INTEGER NOT NULL")
                        }
                        "check" => (
                            "retained_bytes INTEGER NOT NULL CHECK (retained_bytes >= 0)",
                            "retained_bytes INTEGER NOT NULL",
                        ),
                        "strict" => (") STRICT", ")"),
                        _ => unreachable!(),
                    };
                    database
                        .physical
                        .execute_corrupting_sql_for_test("PRAGMA writable_schema = ON")
                        .unwrap();
                    database
                        .physical
                        .execute_corrupting_statement_for_test(
                            "UPDATE sqlite_schema
                             SET sql = replace(sql, ?1, ?2)
                             WHERE type = 'table' AND name = 'applied_batches'",
                            params![expected, corrupted],
                        )
                        .unwrap();
                    database
                        .physical
                        .execute_corrupting_sql_for_test("PRAGMA writable_schema = OFF")
                        .unwrap();
                }
                "unique-index" => {
                    database
                        .physical
                        .execute_corrupting_sql_for_test("PRAGMA writable_schema = ON")
                        .unwrap();
                    database
                        .physical
                        .execute_corrupting_statement_for_test(
                            "UPDATE sqlite_schema
                             SET sql = replace(sql, 'CREATE UNIQUE INDEX', 'CREATE INDEX')
                             WHERE type = 'index' AND name = 'applied_batches_batch_id_uq'",
                            [],
                        )
                        .unwrap();
                    database
                        .physical
                        .execute_corrupting_sql_for_test("PRAGMA writable_schema = OFF")
                        .unwrap();
                }
                "user-version" => {
                    database
                        .physical
                        .set_corrupt_user_version_for_test(SQLITE_SCHEMA_VERSION - 1)
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(database);
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
            rebuilt
                .database
                .physical
                .validate_schema_and_claim(lower_physical_claim(ids.claim()))
                .unwrap();
        }
    }

    #[test]
    fn complete_materialization_is_atomic_bounded_queryable_and_reopenable() {
        let ids = TestIds::new(1_700);
        let dir = TestDir::new("complete-materialization");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let path = "knowledge/journals/deep/2026_07_24.org";
        let transaction = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                name: crate::oplog::LogicalPageName::parse("2026-07-24").unwrap(),
                path: ManagedPath::parse(path).unwrap(),
                kind: ManagedTextKind::Journal,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: ids.block,
                    home_document_id: ids.document,
                },
                page_id: ids.page,
                parent: None,
                order: "a".into(),
                content: "TODO authoritative journal".into(),
            },
        ])
        .unwrap();
        let prepared = engine
            .prepare_bootstrap_transaction(author(1_701), &transaction)
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        let complete = rich_materialization(
            &event,
            ids,
            path,
            ManagedTextKind::Journal,
            "2026-07-24",
            "TODO authoritative journal",
        );
        let incomplete =
            MaterializationChange::new(event.batch_id(), Vec::new(), Vec::new()).unwrap();
        assert!(matches!(
            database.apply_materialized_accepted(&event, &incomplete),
            Err(ProjectionError::Materialization(_))
        ));
        assert_eq!(database.applied_batch_count().unwrap(), 0);

        assert_eq!(
            database.apply_internal_with_materialization(
                &event,
                ApplyFault::ReturnAfterMaterialization,
                Some(&complete),
            ),
            Err(ProjectionError::InjectedFailure)
        );
        assert_eq!(database.applied_batch_count().unwrap(), 0);
        let materialized_rows: i64 = inspect_connection(&database)
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(materialized_rows, 0);

        assert_eq!(
            database
                .apply_materialized_accepted(&event, &complete)
                .unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(
            database
                .apply_materialized_accepted(&event, &complete)
                .unwrap(),
            ApplyDisposition::Duplicate
        );
        let read = database.materialized_read().unwrap();
        assert_eq!(read.acceptance_sequence(), 1);
        let page = read.page(ids.page).unwrap().unwrap();
        assert_eq!(page.kind, ManagedTextKind::Journal);
        assert_eq!(page.name, "2026-07-24");
        assert_eq!(page.name_key, "2026-07-24");
        assert_eq!(page.path.as_str(), path);
        assert_eq!(
            read.pages_by_name("2026-07-24", 10).unwrap(),
            vec![page.clone()]
        );
        assert_eq!(
            read.pages_by_name_key("2026-07-24", 10).unwrap(),
            vec![page.clone()]
        );
        assert_eq!(
            read.pages_by_path(&ManagedPath::parse(path).unwrap(), 10)
                .unwrap(),
            vec![page]
        );
        let block = read.block(ids.block).unwrap().unwrap();
        assert_eq!(block.heading_level, Some(2));
        assert!(block.collapsed);
        assert_eq!(read.blocks_on_page(ids.page, 10).unwrap(), vec![block]);
        let referrers = read
            .referrers_to(MaterializedEntityId::Page(ids.page), 10)
            .unwrap();
        assert_eq!(referrers.len(), 2);
        assert_eq!(
            referrers,
            read.referrers_to(MaterializedEntityId::Page(ids.page), 10)
                .unwrap()
        );
        assert_eq!(
            read.properties(MaterializedEntityId::Block(ids.block), 10)
                .unwrap()[0]
                .value,
            "Ada"
        );
        assert_eq!(read.tags("block-tag", 10).unwrap().len(), 1);
        assert_eq!(
            read.tasks(Some("TODO"), 10).unwrap()[0].priority.as_deref(),
            Some("A")
        );
        let search = read.search("needle", 10).unwrap();
        assert_eq!(search.len(), 1);
        assert_eq!(search, read.search("needle", 10).unwrap());
        assert!(read.search("needle", 0).is_err());

        let database_path = database.path().to_path_buf();
        drop(database);
        let reopened = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
        assert_eq!(
            reopened
                .database
                .materialized_read()
                .unwrap()
                .page(ids.page)
                .unwrap()
                .unwrap()
                .kind,
            ManagedTextKind::Journal
        );
    }

    #[test]
    fn frontier_reference_query_is_catalog_gated_raw_and_bounded_for_rename() {
        let ids = TestIds::new(1_750);
        let source_ids = TestIds {
            document: DocumentId::from_uuid(uuid(1_756)),
            page: PageId::from_uuid(uuid(1_757)),
            block: BlockId::from_uuid(uuid(1_758)),
            ..ids
        };
        let recreated_ids = TestIds {
            document: DocumentId::from_uuid(uuid(1_759)),
            page: PageId::from_uuid(uuid(1_760)),
            block: BlockId::from_uuid(uuid(1_761)),
            ..ids
        };
        let dir = TestDir::new("frontier-reference-query");
        let (mut database, mut engine, store) = open_empty(&dir, ids);

        let target_path = "nested/owners/target.md";
        let target = engine
            .prepare_bootstrap_transaction(
                author(1_751),
                &root_transaction_named(ids, target_path, "Target", "aliases:: Old Target"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &target);
        let target_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, target.manifest().batch_id())
                .unwrap();
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &target_event,
                rich_materialization(
                    &target_event,
                    ids,
                    target_path,
                    ManagedTextKind::Page,
                    "Target",
                    "aliases:: Old Target",
                ),
                &engine,
            )
            .unwrap();

        let source_path = "nested/referrers/arbitrary/depth/source.md";
        let raw_uuid = "6a55b643-1234-5678-9abc-def012345678";
        let source_content = format!("prefix [[Target]] and [[Old Target]] (({raw_uuid}))");
        let source = engine
            .prepare_bootstrap_transaction(
                author(1_752),
                &root_transaction_named(source_ids, source_path, "Source", &source_content),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &source);
        let source_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, source.manifest().batch_id())
                .unwrap();
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &source_event,
                rich_materialization(
                    &source_event,
                    source_ids,
                    source_path,
                    ManagedTextKind::Page,
                    "Source",
                    &source_content,
                ),
                &engine,
            )
            .unwrap();

        let target_name = crate::oplog::LogicalPageName::parse("Target").unwrap();
        let mut query = database.frontier_reference_query(&engine, &store).unwrap();
        let results = query.references_to_page_name(&target_name, 16).unwrap();
        assert_eq!(results.hits.len(), 2);
        assert_eq!(
            results
                .hits
                .iter()
                .map(|hit| match &hit.fact {
                    ReferenceFactV1::PageName(fact) => fact.raw_target.as_str(),
                    ReferenceFactV1::Block(_) => panic!("expected page reference"),
                })
                .collect::<Vec<_>>(),
            vec!["Old Target", "Target"],
        );
        assert!(results
            .hits
            .iter()
            .all(|hit| hit.source_page_id == source_ids.page
                && hit.resolved_page_id == Some(ids.page)));
        assert!(results.instrumentation.sqlite_candidate_sources <= 2);
        assert_eq!(results.instrumentation.tail_source_postings, 0);
        let limited = query.references_to_page_name(&target_name, 1).unwrap();
        assert_eq!(limited.hits.len(), 1);

        let uuid = LogseqUuid::parse(raw_uuid).unwrap();
        let uuid_results = query.references_to_logseq_uuid(uuid, 16).unwrap();
        assert_eq!(uuid_results.hits.len(), 1);
        assert!(matches!(
            &uuid_results.hits[0].fact,
            ReferenceFactV1::Block(fact) if fact.raw_claim == raw_uuid
        ));
        assert_eq!(uuid_results.hits[0].resolved_block_id, None);

        let old_alias = crate::oplog::LogicalPageName::parse("Old Target").unwrap();
        let alias_results = query.references_to_page_name(&old_alias, 16).unwrap();
        assert_eq!(alias_results.hits.len(), 2);
        assert!(alias_results
            .hits
            .iter()
            .all(|hit| hit.source_page_id == source_ids.page));
        assert!(matches!(
            query.plan_page_rename(
                &old_alias,
                crate::oplog::LogicalPageName::parse("Alias Must Not Rename").unwrap(),
                ManagedPath::parse("nested/alias-must-not-rename.md").unwrap(),
            ),
            Err(ProjectionError::Materialization(message))
                if message.contains("no authenticated exact page-name owner")
        ));
        let source_posting = engine
            .reference_source_posting(source_ids.page)
            .unwrap()
            .unwrap();
        for fact in source_posting.facts() {
            if let ReferenceFactV1::PageName(fact) = fact {
                let parser_owned_span =
                    &source_content[fact.byte_start as usize..fact.byte_end as usize];
                assert_eq!(parser_owned_span.match_indices(&fact.raw_target).count(), 1);
            }
        }

        let renamed = crate::oplog::LogicalPageName::parse("Renamed").unwrap();
        let plan = query
            .plan_page_rename(
                &target_name,
                renamed.clone(),
                ManagedPath::parse("nested/renamed/target.md").unwrap(),
            )
            .unwrap_or_else(|error| panic!("rename plan failed: {error:?}"));
        assert_eq!(plan.touched_sources(), &[ids.page, source_ids.page]);
        assert!(plan.instrumentation().revalidated_sources >= 2);
        let SemanticOperation::RenamePagesAndRewriteReferrers { block_rewrites, .. } =
            &plan.transaction().operations[0]
        else {
            panic!("expected rename transaction");
        };
        assert_eq!(block_rewrites.len(), 1);
        assert_eq!(block_rewrites[0].block.block_id, source_ids.block);
        assert_eq!(
            block_rewrites[0].new_content,
            format!("prefix [[Renamed]] and [[Renamed]] (({raw_uuid}))")
        );

        let delete = engine
            .prepare_bootstrap_transaction(
                author(1_753),
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: ids.page,
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &delete);
        let delete_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, delete.manifest().batch_id())
                .unwrap();
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &delete_event,
                MaterializationChange::new(delete_event.batch_id(), Vec::new(), vec![ids.page])
                    .unwrap(),
                &engine,
            )
            .unwrap();
        let mut dangling = database.frontier_reference_query(&engine, &store).unwrap();
        let dangling_results = dangling.references_to_page_name(&target_name, 16).unwrap();
        assert_eq!(dangling_results.hits.len(), 1);
        assert_eq!(dangling_results.hits[0].resolved_page_id, None);
        assert!(matches!(
            &dangling_results.hits[0].fact,
            ReferenceFactV1::PageName(fact) if fact.raw_target == "Target"
        ));

        let recreate_path = "nested/recreated/target.md";
        let recreate = engine
            .prepare_bootstrap_transaction(
                author(1_754),
                &root_transaction_named(
                    recreated_ids,
                    recreate_path,
                    "Target",
                    "recreated at a different nested path",
                ),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &recreate);
        let recreate_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, recreate.manifest().batch_id())
                .unwrap();
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &recreate_event,
                rich_materialization(
                    &recreate_event,
                    recreated_ids,
                    recreate_path,
                    ManagedTextKind::Page,
                    "Target",
                    "recreated at a different nested path",
                ),
                &engine,
            )
            .unwrap();
        let mut rebound = database.frontier_reference_query(&engine, &store).unwrap();
        let rebound_results = rebound.references_to_page_name(&target_name, 16).unwrap();
        assert_eq!(rebound_results.hits.len(), 1);
        assert_eq!(
            rebound_results.hits[0].resolved_page_id,
            Some(recreated_ids.page)
        );
    }

    #[test]
    fn frontier_reference_query_rejects_stale_sqlite_catalog_stamp() {
        let ids = TestIds::new(1_770);
        let dir = TestDir::new("frontier-reference-stale-stamp");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(1_771),
                &root_transaction(ids, "nested/stale.md", "[[Target]]"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &event,
                rich_materialization(
                    &event,
                    ids,
                    "nested/stale.md",
                    ManagedTextKind::Page,
                    "Root Fixture Page",
                    "[[Target]]",
                ),
                &engine,
            )
            .unwrap();
        let unapplied_tail = engine
            .prepare_bootstrap_transaction(
                author(1_772),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "[[Other]]".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &unapplied_tail);
        database
            .physical
            .execute_corrupting_statement_for_test(
                "UPDATE materialization_stamp SET catalog_root_digest = zeroblob(32) WHERE singleton = 1",
                [],
            )
            .unwrap();
        assert!(matches!(
            database.frontier_reference_query(&engine, &store),
            Err(ProjectionError::Materialization(message))
                if message.contains("not bound to its authenticated frontier")
        ));
    }

    #[test]
    fn frontier_reference_query_detects_posting_and_alias_binding_tamper_and_reads_tail() {
        let ids = TestIds::new(1_775);
        let dir = TestDir::new("frontier-reference-row-tamper");
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let content = "aliases:: Old Owner\n[[Target]]";
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(1_776),
                &root_transaction_named(ids, "nested/tamper/owner.md", "Owner", content),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        let base = rich_materialization(
            &event,
            ids,
            "nested/tamper/owner.md",
            ManagedTextKind::Page,
            "Owner",
            content,
        );
        database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &event,
                base.clone(),
                &engine,
            )
            .unwrap();
        let target = crate::oplog::LogicalPageName::parse("Target").unwrap();
        let old_owner = crate::oplog::LogicalPageName::parse("Old Owner").unwrap();
        let target_key = crate::refs::page_key("Target");
        let old_owner_key = crate::refs::page_key("Old Owner");

        assert_eq!(
            database
                .frontier_reference_query(&engine, &store)
                .unwrap()
                .references_to_page_name(&target, crate::oplog::MAX_MATERIALIZATION_QUERY_ROWS,)
                .unwrap()
                .hits
                .len(),
            1
        );
        for invalid_limit in [0, crate::oplog::MAX_MATERIALIZATION_QUERY_ROWS + 1] {
            assert!(database
                .frontier_reference_query(&engine, &store)
                .unwrap()
                .references_to_page_name(&target, invalid_limit)
                .is_err());
        }

        database
            .physical
            .execute_corrupting_statement_for_test(
                "DELETE FROM reference_postings WHERE normalized_name = ?1",
                params![target_key],
            )
            .unwrap();
        assert!(matches!(
            database
                .frontier_reference_query(&engine, &store)
                .unwrap()
                .references_to_page_name(&target, 16),
            Err(ProjectionError::Materialization(message))
                if message.contains("authenticated reverse catalog")
        ));
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base.clone()], &engine)
            .unwrap();

        database
            .physical
            .execute_corrupting_statement_for_test(
                "UPDATE reference_postings
                 SET normalized_name = 'tampered'
                 WHERE normalized_name = ?1",
                params![target_key],
            )
            .unwrap();
        assert!(database
            .frontier_reference_query(&engine, &store)
            .unwrap()
            .references_to_page_name(&target, 16)
            .is_err());
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base.clone()], &engine)
            .unwrap();

        let inserted_source = PageId::from_uuid(uuid(1_799));
        database
            .physical
            .execute_corrupting_statement_for_test(
                "INSERT INTO reference_postings (
                     source_page_id, source_entity_type, source_entity_id, source_locator,
                     ordinal, reference_kind, target_type, raw_name, normalized_name,
                     raw_uuid_claim, resolved_page_id, resolved_block_id
                 )
                 SELECT ?1, source_entity_type, source_entity_id, source_locator,
                        ordinal, reference_kind, target_type, raw_name, normalized_name,
                        raw_uuid_claim, resolved_page_id, resolved_block_id
                 FROM reference_postings WHERE normalized_name = ?2 LIMIT 1",
                params![inserted_source.as_uuid().as_bytes().as_slice(), target_key],
            )
            .unwrap();
        assert!(database
            .frontier_reference_query(&engine, &store)
            .unwrap()
            .references_to_page_name(&target, 16)
            .is_err());
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base.clone()], &engine)
            .unwrap();

        database
            .physical
            .execute_corrupting_statement_for_test(
                "DELETE FROM reference_alias_bindings WHERE normalized_alias = ?1",
                params![old_owner_key],
            )
            .unwrap();
        assert!(matches!(
            database
                .frontier_reference_query(&engine, &store)
                .unwrap()
                .references_to_page_name(&old_owner, 16),
            Err(ProjectionError::Materialization(message))
                if message.contains("alias bindings")
        ));
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base.clone()], &engine)
            .unwrap();

        database
            .physical
            .execute_corrupting_statement_for_test(
                "UPDATE reference_alias_bindings
                 SET normalized_alias = 'tampered'
                 WHERE normalized_alias = ?1",
                params![old_owner_key],
            )
            .unwrap();
        assert!(database
            .frontier_reference_query(&engine, &store)
            .unwrap()
            .references_to_page_name(&old_owner, 16)
            .is_err());
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base.clone()], &engine)
            .unwrap();

        database
            .physical
            .execute_corrupting_statement_for_test(
                "INSERT INTO reference_alias_bindings (
                     normalized_alias, candidate_ordinal, resolved_page_id, catalog_root_digest
                 )
                 SELECT normalized_alias, candidate_ordinal + 1000000, ?1, catalog_root_digest
                 FROM reference_alias_bindings WHERE normalized_alias = ?2 LIMIT 1",
                params![
                    inserted_source.as_uuid().as_bytes().as_slice(),
                    old_owner_key
                ],
            )
            .unwrap();
        assert!(database
            .frontier_reference_query(&engine, &store)
            .unwrap()
            .references_to_page_name(&old_owner, 16)
            .is_err());
        database
            .rebuild_authenticated_reference_catalog_materialization(vec![base], &engine)
            .unwrap();

        let tail_content = "aliases:: Old Owner\n[[Target]] and [[Tail Target]]";
        let tail = engine
            .prepare_bootstrap_transaction(
                author(1_777),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: tail_content.into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &tail);
        let tail_target = crate::oplog::LogicalPageName::parse("Tail Target").unwrap();
        let results = database
            .frontier_reference_query(&engine, &store)
            .unwrap()
            .references_to_page_name(&tail_target, 16)
            .unwrap();
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].source_page_id, ids.page);
        assert_eq!(results.instrumentation.tail_source_postings, 1);
    }

    #[test]
    fn rebuild_materialization_rebinds_authenticated_reference_transition() {
        let ids = TestIds::new(1_780);
        let dir = TestDir::new("rebuild-authenticated-reference-catalog");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(1_781),
                &root_transaction(ids, "nested/rebuild/source.md", "[[Target]]"),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        let change = attach_authenticated_reference_catalog(
            &engine,
            &event,
            rich_materialization(
                &event,
                ids,
                "nested/rebuild/source.md",
                ManagedTextKind::Page,
                "Root Fixture Page",
                "[[Target]]",
            ),
        )
        .unwrap();
        database
            .apply_materialized_accepted(&event, &change)
            .unwrap();
        let post_root = event.post_frontier_root().reference_catalog_root().clone();
        let forged_reference =
            super::super::sqlite_materialization::ReferenceCatalogMaterializationInput::new(
                event.prior_frontier_root()
                    .reference_catalog_root()
                    .clone(),
                post_root.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![
                    super::super::sqlite_materialization::SourceCoverageFacet {
                        source_page_id: ids.page,
                        source_digest: ContentDigest::of(b"caller-forged-source"),
                        extractor_dependency_stamp:
                            super::super::sqlite_materialization::ReferenceExtractorDependencyStamp::new(
                                post_root.extractor_digest(),
                                post_root.policy_digest(),
                            )
                            .unwrap(),
                    },
                ],
                Vec::new(),
            )
            .unwrap();
        let forged = MaterializationChange::new(
            event.batch_id(),
            change.replacements().to_vec(),
            change.deletions().to_vec(),
        )
        .unwrap()
        .with_authenticated_reference_catalog(forged_reference)
        .unwrap();
        assert!(matches!(
            database.rebuild_materialization(vec![forged.clone()]),
            Err(ProjectionError::Materialization(message))
                if message.contains("requires authenticated engine authority")
        ));
        assert_eq!(
            database
                .rebuild_authenticated_reference_catalog_materialization(vec![forged], &engine)
                .unwrap(),
            1
        );
        let mut query = database.frontier_reference_query(&engine, &store).unwrap();
        let hits = query
            .references_to_page_name(&crate::oplog::LogicalPageName::parse("Target").unwrap(), 16)
            .unwrap();
        assert_eq!(hits.hits.len(), 1);
        assert_eq!(hits.hits[0].source_page_id, ids.page);
    }

    #[test]
    fn rename_rewrite_fails_closed_when_current_source_bytes_no_longer_match() {
        let target = crate::oplog::LogicalPageName::parse("Target").unwrap();
        let fact = crate::oplog::PageNameReferenceFactV1 {
            source: ReferenceSourceLocatorV1::Preamble,
            kind: crate::oplog::PageReferenceKindV1::PageLink,
            raw_target: "Target".into(),
            normalized_target: crate::refs::page_key("Target"),
            target_key: target.key_digest(),
            byte_start: 0,
            byte_end: 10,
        };
        assert_eq!(
            rewrite_raw_page_targets("[[Target]]", &[fact.clone()], "Renamed").unwrap(),
            "[[Renamed]]"
        );
        assert!(matches!(
            rewrite_raw_page_targets("[[Other]]", &[fact], "Renamed"),
            Err(ProjectionError::Materialization(message))
                if message.contains("no longer match raw reference evidence")
        ));
    }

    #[test]
    fn materialization_rejects_name_and_key_contradictions_before_writing() {
        let ids = TestIds::new(1_750);
        let dir = TestDir::new("materialization-page-name-authority");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let path = "nested/unrelated/filename.md";
        let transaction = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                name: crate::oplog::LogicalPageName::parse("CAFÉ").unwrap(),
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
                content: "authoritative page name".into(),
            },
        ])
        .unwrap();
        let prepared = engine
            .prepare_bootstrap_transaction(author(1_751), &transaction)
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        let valid = rich_materialization(
            &event,
            ids,
            path,
            ManagedTextKind::Page,
            "CAFÉ",
            "authoritative page name",
        );

        for (name, name_key) in [
            ("Deep Journal", "café"),
            ("CAFÉ", "CAFÉ"),
            ("CAFÉ", "cafe\u{301}"),
        ] {
            let mut replacements = valid.replacements().to_vec();
            replacements[0].name = name.into();
            replacements[0].name_key = name_key.into();
            let contradiction =
                MaterializationChange::new(event.batch_id(), replacements, Vec::new()).unwrap();
            assert!(matches!(
                database.apply_materialized_accepted(&event, &contradiction),
                Err(ProjectionError::Materialization(_))
            ));
            assert_eq!(database.applied_batch_count().unwrap(), 0);
            let connection = inspect_connection(&database);
            let rows: i64 = connection
                .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
                .unwrap();
            assert_eq!(rows, 0);
            let materialized_batches: i64 = connection
                .query_row("SELECT COUNT(*) FROM materialization_batches", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(materialized_batches, 0);
            let materialization_sequence: i64 = connection
                .query_row(
                    "SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(materialization_sequence, 0);
        }

        database
            .apply_materialized_accepted(&event, &valid)
            .unwrap();
        let page = database
            .materialized_read()
            .unwrap()
            .page(ids.page)
            .unwrap()
            .unwrap();
        assert_eq!(page.name, "CAFÉ");
        assert_eq!(page.name_key, "café");
        assert_eq!(page.path.as_str(), path);
    }

    #[test]
    fn non_page_effect_materialization_preserves_prior_page_metadata_transactionally() {
        for (index, case) in ["block", "membership", "preamble"].into_iter().enumerate() {
            let ids = TestIds::new(1_760 + index as u128 * 100);
            let dir = TestDir::new(&format!("materialization-prior-page-metadata-{case}"));
            let (mut database, mut engine, store) = open_empty(&dir, ids);
            let path = "pages/root-fixture.md";
            let root_prepared = engine
                .prepare_bootstrap_transaction(
                    author(1_761 + index as u128 * 100),
                    &root_transaction(ids, path, "initial"),
                )
                .unwrap();
            publish_and_stage(&mut engine, &store, &root_prepared);
            let root_event = AcceptedBatchEvent::from_accepted(
                &engine,
                &store,
                root_prepared.manifest().batch_id(),
            )
            .unwrap();
            let root_change = rich_materialization(
                &root_event,
                ids,
                path,
                ManagedTextKind::Page,
                "Root Fixture Page",
                "initial",
            );
            database
                .apply_authenticated_reference_catalog_materialized_accepted(
                    &root_event,
                    root_change,
                    &engine,
                )
                .unwrap();
            let prior_page = database
                .materialized_read()
                .unwrap()
                .page(ids.page)
                .unwrap()
                .unwrap();

            let update = match case {
                "block" => OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "updated".into(),
                }])
                .unwrap(),
                "membership" => OperationTransaction::new(vec![SemanticOperation::ReorderBlock {
                    block_id: ids.block,
                    page_id: ids.page,
                    parent: None,
                    order: "b".into(),
                }])
                .unwrap(),
                "preamble" => OperationTransaction::new(vec![SemanticOperation::SetPagePreamble {
                    page_id: ids.page,
                    preamble: Some("updated preamble".into()),
                }])
                .unwrap(),
                _ => unreachable!("fixed test cases"),
            };
            let update_prepared = engine
                .prepare_bootstrap_transaction(author(1_762 + index as u128 * 100), &update)
                .unwrap();
            publish_and_stage(&mut engine, &store, &update_prepared);
            let update_event = AcceptedBatchEvent::from_accepted(
                &engine,
                &store,
                update_prepared.manifest().batch_id(),
            )
            .unwrap();
            let effect =
                crate::oplog::SemanticEffect::decode(update_event.semantic_effect()).unwrap();
            assert!(
                effect.pages().is_empty(),
                "{case} effect must not replace page metadata"
            );
            match case {
                "block" => assert!(!effect.blocks().is_empty()),
                "membership" => assert!(!effect.memberships().is_empty()),
                "preamble" => assert!(!effect.page_preambles().is_empty()),
                _ => unreachable!("fixed test cases"),
            }

            let mut exact_prior_metadata = rich_materialization(
                &update_event,
                ids,
                path,
                ManagedTextKind::Page,
                "Root Fixture Page",
                if case == "block" {
                    "updated"
                } else {
                    "initial"
                },
            );
            let mut exact_replacements = exact_prior_metadata.replacements().to_vec();
            if case == "membership" {
                exact_replacements[0].blocks[0].order = "b".into();
            }
            if case == "preamble" {
                exact_replacements[0].preamble = Some("updated preamble".into());
            }
            exact_prior_metadata =
                MaterializationChange::new(update_event.batch_id(), exact_replacements, Vec::new())
                    .unwrap();

            let corruptions: [(&str, fn(&mut MaterializedPageInput)); 5] = [
                ("name", |page| page.name = "Contradictory Name".into()),
                ("name key", |page| {
                    page.name_key = "contradictory-name".into()
                }),
                ("path", |page| {
                    page.path = ManagedPath::parse("pages/contradictory.md").unwrap()
                }),
                ("home document", |page| {
                    page.home_document_id = DocumentId::from_uuid(uuid(9_999))
                }),
                ("kind", |page| page.kind = ManagedTextKind::Journal),
            ];
            for (field, corrupt) in corruptions {
                let mut replacements = exact_prior_metadata.replacements().to_vec();
                corrupt(&mut replacements[0]);
                let contradiction =
                    MaterializationChange::new(update_event.batch_id(), replacements, Vec::new())
                        .unwrap();
                assert!(
                    matches!(
                        database.apply_materialized_accepted(&update_event, &contradiction),
                        Err(ProjectionError::Materialization(_))
                    ),
                    "{case} {field} contradiction must fail"
                );
                assert_eq!(database.applied_batch_count().unwrap(), 1, "{case} {field}");
                assert!(
                    matches!(
                        database.materialized_read(),
                        Err(ProjectionError::Materialization(_))
                    ),
                    "{case} {field} exposed stale rows after an accepted event failed"
                );
                let stamp: i64 = inspect_connection(&database)
                    .query_row(
                        "SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(stamp, 1, "{case} {field}");
            }

            assert_eq!(
                database
                    .apply_materialized_accepted(&update_event, &exact_prior_metadata)
                    .unwrap(),
                ApplyDisposition::Applied,
                "{case} exact prior metadata must be accepted"
            );
            let database_path = database.path().to_path_buf();
            drop(database);
            let reopened = open_test_projection(
                &database_path,
                ids.claim(),
                RebuildSource::new(&engine, &store).unwrap(),
            )
            .unwrap();
            let page = reopened
                .database
                .materialized_read()
                .unwrap()
                .page(ids.page)
                .unwrap()
                .unwrap();
            assert_eq!(page.name, prior_page.name, "{case}");
            assert_eq!(page.name_key, prior_page.name_key, "{case}");
            assert_eq!(page.path, prior_page.path, "{case}");
            assert_eq!(page.home_document_id, prior_page.home_document_id, "{case}");
            assert_eq!(page.kind, prior_page.kind, "{case}");
        }
    }

    #[test]
    fn rebuild_materialization_rejects_non_page_metadata_contradictions() {
        let ids = TestIds::new(2_000);
        let dir = TestDir::new("materialization-rebuild-prior-page-metadata");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let path = "pages/root-fixture.md";
        let root_prepared = engine
            .prepare_bootstrap_transaction(author(2_001), &root_transaction(ids, path, "initial"))
            .unwrap();
        publish_and_stage(&mut engine, &store, &root_prepared);
        let root_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, root_prepared.manifest().batch_id())
                .unwrap();
        let root_change = rich_materialization(
            &root_event,
            ids,
            path,
            ManagedTextKind::Page,
            "Root Fixture Page",
            "initial",
        );
        database
            .apply_materialized_accepted(&root_event, &root_change)
            .unwrap();

        let update_prepared = engine
            .prepare_bootstrap_transaction(
                author(2_002),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    content: "updated".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &update_prepared);
        let update_event = AcceptedBatchEvent::from_accepted(
            &engine,
            &store,
            update_prepared.manifest().batch_id(),
        )
        .unwrap();
        let valid = rich_materialization(
            &update_event,
            ids,
            path,
            ManagedTextKind::Page,
            "Root Fixture Page",
            "updated",
        );
        let mut contradictory_replacements = valid.replacements().to_vec();
        contradictory_replacements[0].path = ManagedPath::parse("pages/contradictory.md").unwrap();
        let contradictory = MaterializationChange::new(
            update_event.batch_id(),
            contradictory_replacements,
            Vec::new(),
        )
        .unwrap();

        database.apply_accepted(&update_event).unwrap();
        assert!(matches!(
            database.rebuild_materialization(vec![root_change.clone(), contradictory]),
            Err(ProjectionError::Materialization(_))
        ));
        let persisted: (String, String, String, i64) = inspect_connection(&database)
            .query_row(
                "SELECT name, name_key, path, text_kind FROM pages WHERE page_id = ?1",
                params![ids.page.as_uuid().as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            persisted,
            (
                "Root Fixture Page".into(),
                "root fixture page".into(),
                path.into(),
                0,
            )
        );
        assert!(matches!(
            database.materialized_read(),
            Err(ProjectionError::Materialization(_))
        ));

        assert_eq!(
            database
                .rebuild_materialization(vec![root_change, valid])
                .unwrap(),
            2
        );
        let page = database
            .materialized_read()
            .unwrap()
            .page(ids.page)
            .unwrap()
            .unwrap();
        assert_eq!(page.path.as_str(), path);
        assert_eq!(page.name, "Root Fixture Page");
        assert_eq!(page.name_key, "root fixture page");
    }

    #[test]
    fn prior_schema_reopen_rebuilds_without_serving_stale_contradictory_page_rows() {
        let ids = TestIds::new(1_775);
        let dir = TestDir::new("schema-7-materialization-name-rebuild");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let path = "nested/unrelated/filename.md";
        let transaction = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: ids.page,
                home_document_id: ids.document,
                name: crate::oplog::LogicalPageName::parse("CAFÉ").unwrap(),
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
                content: "authoritative page name".into(),
            },
        ])
        .unwrap();
        let prepared = engine
            .prepare_bootstrap_transaction(author(1_776), &transaction)
            .unwrap();
        publish_and_stage(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        let current = rich_materialization(
            &event,
            ids,
            path,
            ManagedTextKind::Page,
            "CAFÉ",
            "authoritative page name",
        );
        database
            .apply_materialized_accepted(&event, &current)
            .unwrap();
        database
            .physical
            .execute_corrupting_statement_for_test(
                "UPDATE pages SET name = 'stale contradictory', name_key = 'stale contradictory'",
                [],
            )
            .unwrap();
        database
            .physical
            .set_corrupt_user_version_for_test(SQLITE_SCHEMA_VERSION - 1)
            .unwrap();
        let database_path = database.path().to_path_buf();
        drop(database);

        let reopened = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let ProjectionRecovery::RebuiltPreservingEvidence { reason, .. } = &reopened.recovery
        else {
            panic!("prior-schema database was reopened without a disposable rebuild");
        };
        assert!(
            reason.contains(&format!(
                "user_version {} != {SQLITE_SCHEMA_VERSION}",
                SQLITE_SCHEMA_VERSION - 1
            )),
            "unexpected rebuild reason: {reason}"
        );
        let rebuilt_rows: i64 = inspect_connection(&reopened.database)
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rebuilt_rows, 1);
        let page = reopened
            .database
            .materialized_read()
            .unwrap()
            .page(ids.page)
            .unwrap()
            .unwrap();
        assert_eq!(page.name, "CAFÉ");
        assert_eq!(page.name_key, "café");
    }

    #[test]
    fn replacement_deletion_and_disposable_rebuild_remove_every_stale_row() {
        let ids = TestIds::new(1_800);
        let dir = TestDir::new("materialization-replace-delete-rebuild");
        let (mut database, mut engine, store) = open_empty(&dir, ids);
        let referrer_page = PageId::from_uuid(uuid(1_890));
        let referrer_document = DocumentId::from_uuid(uuid(1_891));
        let referrer_block = BlockId::from_uuid(uuid(1_892));
        let root_prepared = engine
            .prepare_bootstrap_transaction(
                author(1_801),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: ids.page,
                        home_document_id: ids.document,
                        name: crate::oplog::LogicalPageName::parse("original").unwrap(),
                        path: ManagedPath::parse("nested/pages/original.md").unwrap(),
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
                        content: "obsolete".into(),
                    },
                    SemanticOperation::CreatePage {
                        page_id: referrer_page,
                        home_document_id: referrer_document,
                        name: crate::oplog::LogicalPageName::parse("referrer").unwrap(),
                        path: ManagedPath::parse("nested/pages/referrer.md").unwrap(),
                        kind: ManagedTextKind::Page,
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: referrer_block,
                            home_document_id: referrer_document,
                        },
                        page_id: referrer_page,
                        parent: None,
                        order: "a".into(),
                        content: "stable referrer".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &root_prepared);
        let root_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, root_prepared.manifest().batch_id())
                .unwrap();
        let target_root_change = rich_materialization(
            &root_event,
            ids,
            "nested/pages/original.md",
            ManagedTextKind::Page,
            "original",
            "obsolete",
        );
        let mut root_replacements = target_root_change.replacements().to_vec();
        root_replacements.push(MaterializedPageInput {
            page_id: referrer_page,
            home_document_id: referrer_document,
            name: "referrer".into(),
            name_key: "referrer".into(),
            path: ManagedPath::parse("nested/pages/referrer.md").unwrap(),
            kind: ManagedTextKind::Page,
            preamble: None,
            searchable_text: "stable referrer page".into(),
            references: Vec::new(),
            properties: Vec::new(),
            tags: Vec::new(),
            blocks: vec![MaterializedBlockInput {
                block_id: referrer_block,
                home_document_id: referrer_document,
                parent: None,
                order: "a".into(),
                content: "stable referrer".into(),
                searchable_text: "stable referrer".into(),
                heading_level: None,
                collapsed: false,
                logseq_uuid: None,
                logseq_identity_origin: None,
                references: vec![
                    MaterializedReference {
                        target: MaterializedEntityId::Page(ids.page),
                        kind: MaterializedReferenceKind::Reference,
                    },
                    MaterializedReference {
                        target: MaterializedEntityId::Block(ids.block),
                        kind: MaterializedReferenceKind::Embed,
                    },
                ],
                properties: Vec::new(),
                tags: Vec::new(),
                task: None,
            }],
        });
        let root_change =
            MaterializationChange::new(root_event.batch_id(), root_replacements, Vec::new())
                .unwrap();
        database
            .apply_materialized_accepted(&root_event, &root_change)
            .unwrap();

        let replacement_transaction = OperationTransaction::new(vec![
            SemanticOperation::EditPagePath {
                page_id: ids.page,
                path: ManagedPath::parse("nested/pages/deeper/renamed.md").unwrap(),
            },
            SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block,
                    home_document_id: ids.document,
                },
                content: "fresh".into(),
            },
        ])
        .unwrap();
        let replacement_prepared = engine
            .prepare_bootstrap_transaction(author(1_802), &replacement_transaction)
            .unwrap();
        publish_and_stage(&mut engine, &store, &replacement_prepared);
        let replacement_event = AcceptedBatchEvent::from_accepted(
            &engine,
            &store,
            replacement_prepared.manifest().batch_id(),
        )
        .unwrap();
        let replacement_change = rich_materialization(
            &replacement_event,
            ids,
            "nested/pages/deeper/renamed.md",
            ManagedTextKind::Page,
            "original",
            "fresh",
        );
        database
            .apply_materialized_accepted(&replacement_event, &replacement_change)
            .unwrap();
        let read = database.materialized_read().unwrap();
        assert!(read
            .pages_by_path(&ManagedPath::parse("nested/pages/original.md").unwrap(), 10)
            .unwrap()
            .is_empty());
        assert!(read.search("obsolete", 10).unwrap().is_empty());
        let incoming = read
            .referrers_to(MaterializedEntityId::Page(ids.page), 10)
            .unwrap();
        assert!(incoming
            .iter()
            .any(|row| row.source == MaterializedEntityId::Block(referrer_block)));
        let expected_page = read.page(ids.page).unwrap().unwrap();
        let expected_block = read.block(ids.block).unwrap().unwrap();
        let expected_properties = read.properties_named("owner", None, 10).unwrap();
        let expected_tags = read.tags("block-tag", 10).unwrap();
        let expected_tasks = read.tasks(None, 10).unwrap();
        let expected_search = read.search("fresh", 10).unwrap();

        let database_path = database.path().to_path_buf();
        drop(database);
        super::remove_projection_files(&database_path).unwrap();
        let mut rebuilt = open_test_projection(
            &database_path,
            ids.claim(),
            RebuildSource::new(&ids.engine(), &store).unwrap(),
        )
        .unwrap()
        .database;
        assert_eq!(
            rebuilt.apply_accepted(&root_event).unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(
            rebuilt.apply_accepted(&replacement_event).unwrap(),
            ApplyDisposition::Applied
        );
        assert!(matches!(
            rebuilt.materialized_read(),
            Err(ProjectionError::Materialization(_))
        ));
        let rebuild_started = Instant::now();
        assert_eq!(
            rebuilt
                .rebuild_materialization(vec![root_change.clone(), replacement_change.clone()])
                .unwrap(),
            2
        );
        let rebuild_elapsed = rebuild_started.elapsed();
        let database_bytes = fs::metadata(rebuilt.path()).unwrap().len();
        eprintln!(
            "materialization rebuild smoke: 2 batches, 2 pages, {database_bytes} database bytes, {rebuild_elapsed:?}"
        );
        assert!(database_bytes > 0);
        let read = rebuilt.materialized_read().unwrap();
        assert_eq!(read.page(ids.page).unwrap(), Some(expected_page));
        assert_eq!(read.block(ids.block).unwrap(), Some(expected_block));
        assert_eq!(
            read.referrers_to(MaterializedEntityId::Page(ids.page), 10)
                .unwrap(),
            incoming
        );
        assert_eq!(
            read.properties_named("owner", None, 10).unwrap(),
            expected_properties
        );
        assert_eq!(read.tags("block-tag", 10).unwrap(), expected_tags);
        assert_eq!(read.tasks(None, 10).unwrap(), expected_tasks);
        assert_eq!(read.search("fresh", 10).unwrap(), expected_search);

        let delete_prepared = engine
            .prepare_bootstrap_transaction(
                author(1_803),
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: ids.page,
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage(&mut engine, &store, &delete_prepared);
        let delete_event = AcceptedBatchEvent::from_accepted(
            &engine,
            &store,
            delete_prepared.manifest().batch_id(),
        )
        .unwrap();
        let delete_change =
            MaterializationChange::new(delete_event.batch_id(), Vec::new(), vec![ids.page])
                .unwrap();
        rebuilt
            .apply_materialized_accepted(&delete_event, &delete_change)
            .unwrap();
        let read = rebuilt.materialized_read().unwrap();
        assert_eq!(read.page(ids.page).unwrap(), None);
        assert_eq!(read.block(ids.block).unwrap(), None);
        assert!(read.page(referrer_page).unwrap().is_some());
        assert!(read.block(referrer_block).unwrap().is_some());
        assert!(read
            .referrers_to(MaterializedEntityId::Page(ids.page), 10)
            .unwrap()
            .is_empty());
        assert!(read.properties_named("owner", None, 10).unwrap().is_empty());
        assert!(read.tags("block-tag", 10).unwrap().is_empty());
        assert!(read.tasks(None, 10).unwrap().is_empty());
        assert!(read.search("fresh", 10).unwrap().is_empty());
    }

    #[test]
    fn historical_adapter_preserves_rich_event_state_and_matches_clean_rebuild() {
        let ids = TestIds::new(2_200);
        let dir = TestDir::new("historical-rich-adapter");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let normal_path = dir.path().join("normal.sqlite");
        let mut normal = open_test_projection(
            &normal_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let child = BlockId::from_uuid(uuid(2_206));
        let original_content = "TODO [#A] Ship #project\n\
            SCHEDULED: <2026-07-25 Sat>\n\
            DEADLINE: <2026-07-26 Sun>\n\
            owner:: Ada\n\
            collapsed:: true";
        let create = engine
            .prepare_bootstrap_transaction(
                author(2_210),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: ids.page,
                        home_document_id: ids.document,
                        name: crate::oplog::LogicalPageName::parse("Historical Original").unwrap(),
                        path: ManagedPath::parse("pages/historical-original.md").unwrap(),
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
                        content: original_content.into(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: child,
                            home_document_id: ids.document,
                        },
                        page_id: ids.page,
                        parent: Some(ids.block),
                        order: "a".into(),
                        content: "nested original".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &create);
        let create_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, create.manifest().batch_id())
                .unwrap();

        let edited_content = "DONE [#B] Shipped #complete\nowner:: Grace";
        let edit = engine
            .prepare_bootstrap_transaction(
                author(2_211),
                &OperationTransaction::new(vec![
                    SemanticOperation::RenamePagesAndRewriteReferrers {
                        page_changes: vec![PageRename {
                            page_id: ids.page,
                            new_name: crate::oplog::LogicalPageName::parse("Historical Renamed")
                                .unwrap(),
                            new_path: ManagedPath::parse("pages/historical-renamed.md").unwrap(),
                        }],
                        block_rewrites: Vec::new(),
                        page_preamble_rewrites: Vec::new(),
                    },
                    SemanticOperation::EditBlockContent {
                        block: BlockLocation {
                            block_id: ids.block,
                            home_document_id: ids.document,
                        },
                        content: edited_content.into(),
                    },
                    SemanticOperation::ReorderBlock {
                        block_id: child,
                        page_id: ids.page,
                        parent: None,
                        order: "b".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &edit);
        let edit_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, edit.manifest().batch_id()).unwrap();
        assert_eq!(
            edit_event.authored_semantic_effect(),
            edit_event.semantic_effect(),
            "an ordinary rename must keep authored and effective bytes identical"
        );

        let historical = materialize_accepted_event(&engine, &create_event).unwrap();
        let historical_page = &historical.replacements()[0];
        assert_eq!(historical_page.name, "Historical Original");
        assert_eq!(
            historical_page.path.as_str(),
            "pages/historical-original.md"
        );
        let historical_parent = historical_page
            .blocks
            .iter()
            .find(|block| block.block_id == ids.block)
            .unwrap();
        let historical_child = historical_page
            .blocks
            .iter()
            .find(|block| block.block_id == child)
            .unwrap();
        assert_eq!(historical_parent.content, original_content);
        assert!(historical_parent.collapsed);
        assert_eq!(historical_parent.tags, vec!["project"]);
        assert_eq!(historical_parent.properties[0].name, "owner");
        assert_eq!(historical_parent.properties[0].value, "Ada");
        assert_eq!(historical_parent.task.as_ref().unwrap().marker, "TODO");
        assert_eq!(
            historical_parent.task.as_ref().unwrap().priority.as_deref(),
            Some("A")
        );
        assert_eq!(
            historical_parent
                .task
                .as_ref()
                .unwrap()
                .scheduled
                .as_deref(),
            Some("2026-07-25 Sat")
        );
        assert_eq!(
            historical_parent.task.as_ref().unwrap().deadline.as_deref(),
            Some("2026-07-26 Sun")
        );
        assert_eq!(historical_child.parent, Some(ids.block));

        let current = materialize_accepted_event(&engine, &edit_event).unwrap();
        let current_page = &current.replacements()[0];
        assert_eq!(current_page.name, "Historical Renamed");
        assert_eq!(current_page.path.as_str(), "pages/historical-renamed.md");
        let current_parent = current_page
            .blocks
            .iter()
            .find(|block| block.block_id == ids.block)
            .unwrap();
        let current_child = current_page
            .blocks
            .iter()
            .find(|block| block.block_id == child)
            .unwrap();
        assert_eq!(current_parent.content, edited_content);
        assert_eq!(current_parent.tags, vec!["complete"]);
        assert_eq!(current_parent.properties[0].value, "Grace");
        assert_eq!(current_parent.task.as_ref().unwrap().marker, "DONE");
        assert_eq!(current_child.parent, None);

        let mut forged = create_event.clone();
        forged.post_frontier_root = edit_event.post_frontier_root().clone();
        assert!(matches!(
            materialize_accepted_event(&engine, &forged),
            Err(ProjectionError::InvalidAcceptedEvent(_))
        ));

        let mut overlay = TailOverlay::empty_for_test(&engine);
        assert!(overlay
            .try_enqueue(&mut normal, &engine, &create_event)
            .unwrap());
        assert!(overlay
            .try_enqueue(&mut normal, &engine, &edit_event)
            .unwrap());
        let source = RebuildSource::new(&engine, &store).unwrap();
        assert_eq!(overlay.drain_ready(&mut normal, &source, 2).unwrap(), 2);
        let expected_frontier = normal.frontier_root().unwrap();
        let expected_digest = normal.semantic_projection_digest().unwrap();
        let read = normal.materialized_read().unwrap();
        let expected_page = read.page(ids.page).unwrap();
        let expected_parent = read.block(ids.block).unwrap();
        let expected_child = read.block(child).unwrap();
        let expected_properties = read.properties_named("owner", None, 10).unwrap();
        let expected_tags = read.tags("complete", 10).unwrap();
        let expected_tasks = read.tasks(Some("DONE"), 10).unwrap();
        let expected_search = read.search("Shipped", 10).unwrap();
        drop(normal);

        let rebuilt_path = dir.path().join("rebuilt.sqlite");
        let rebuilt = open_test_projection(
            &rebuilt_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            rebuilt.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 2 }
        );
        assert_eq!(rebuilt.database.frontier_root().unwrap(), expected_frontier);
        assert_eq!(
            rebuilt.database.semantic_projection_digest().unwrap(),
            expected_digest
        );
        let read = rebuilt.database.materialized_read().unwrap();
        assert_eq!(read.page(ids.page).unwrap(), expected_page);
        assert_eq!(read.block(ids.block).unwrap(), expected_parent);
        assert_eq!(read.block(child).unwrap(), expected_child);
        assert_eq!(
            read.properties_named("owner", None, 10).unwrap(),
            expected_properties
        );
        assert_eq!(read.tags("complete", 10).unwrap(), expected_tags);
        assert_eq!(read.tasks(Some("DONE"), 10).unwrap(), expected_tasks);
        assert_eq!(read.search("Shipped", 10).unwrap(), expected_search);
        drop(rebuilt);
        let reopened = open_test_projection(
            &normal_path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(reopened.recovery, ProjectionRecovery::OpenedExisting);
        let mut normal = reopened.database;

        let delete = engine
            .prepare_bootstrap_transaction(
                author(2_212),
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: ids.page,
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &delete);
        let delete_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, delete.manifest().batch_id())
                .unwrap();
        let deletion = materialize_accepted_event(&engine, &delete_event).unwrap();
        assert!(deletion.replacements().is_empty());
        assert_eq!(deletion.deletions(), &[ids.page]);

        assert!(overlay
            .try_enqueue(&mut normal, &engine, &delete_event)
            .unwrap());
        let source = RebuildSource::new(&engine, &store).unwrap();
        assert_eq!(overlay.drain_ready(&mut normal, &source, 1).unwrap(), 1);
        let read = normal.materialized_read().unwrap();
        assert_eq!(read.page(ids.page).unwrap(), None);
        assert_eq!(read.block(ids.block).unwrap(), None);
        assert_eq!(read.block(child).unwrap(), None);
        drop(normal);

        let deleted_rebuild = open_test_projection(
            &dir.path().join("deleted-rebuild.sqlite"),
            ids.claim(),
            source,
        )
        .unwrap();
        assert_eq!(
            deleted_rebuild.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 3 }
        );
        let read = deleted_rebuild.database.materialized_read().unwrap();
        assert_eq!(read.page(ids.page).unwrap(), None);
        assert_eq!(
            deleted_rebuild.database.frontier_root().unwrap(),
            engine.accepted_frontier_root().unwrap()
        );
    }

    #[test]
    fn event_scoped_materialization_matches_pointwise_for_rich_multi_page_transitions() {
        let ids = TestIds::new(2_225);
        let second_page = PageId::from_uuid(uuid(2_240));
        let second_document = DocumentId::from_uuid(uuid(2_241));
        let second_block = BlockId::from_uuid(uuid(2_242));
        let dir = TestDir::new("event-scoped-materialization");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut zero_budget_database = open_test_projection(
            &dir.path().join("zero-budget.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let create = engine
            .prepare_bootstrap_transaction(
                author(2_250),
                &OperationTransaction::new(vec![
                    SemanticOperation::CreatePage {
                        page_id: ids.page,
                        home_document_id: ids.document,
                        name: crate::oplog::LogicalPageName::parse("Rich Source").unwrap(),
                        path: ManagedPath::parse("pages/rich-source.md").unwrap(),
                        kind: ManagedTextKind::Page,
                    },
                    SemanticOperation::CreatePage {
                        page_id: second_page,
                        home_document_id: second_document,
                        name: crate::oplog::LogicalPageName::parse("Rich Target").unwrap(),
                        path: ManagedPath::parse("pages/rich-target.md").unwrap(),
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
                        content:
                            "TODO [#A] [[Rich Target]] #project\nowner:: Ada\nalias:: Source Alias"
                                .into(),
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: second_block,
                            home_document_id: second_document,
                        },
                        page_id: second_page,
                        parent: None,
                        order: "a".into(),
                        content: "Target content".into(),
                    },
                ])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &create);
        let create_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, create.manifest().batch_id())
                .unwrap();
        let (scoped_create, create_stats) =
            materialize_accepted_event_with_stats(&engine, &create_event).unwrap();
        let point_create = materialize_accepted_event_pointwise(&engine, &create_event).unwrap();
        let bulk_materializer = engine
            .bootstrap_bulk_materializer(create_event.post_frontier_root())
            .unwrap();
        let bulk_pages = bulk_materializer
            .materialize_pages(&[ids.page, second_page])
            .unwrap()
            .into_iter()
            .map(|page| materialized_page_input(page.unwrap()))
            .collect();
        let bulk_create =
            MaterializationChange::new(create_event.batch_id(), bulk_pages, vec![]).unwrap();
        let (zero_budget_create, zero_budget_stats) =
            materialize_inactive_bootstrap_event_bulk_with_budget(&engine, &create_event, 0)
                .unwrap();
        let (cached_create, cached_stats) = materialize_inactive_bootstrap_event_bulk_with_budget(
            &engine,
            &create_event,
            crate::oplog::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT,
        )
        .unwrap();
        assert_eq!(scoped_create, point_create);
        assert_eq!(bulk_create, scoped_create);
        assert_eq!(zero_budget_create, cached_create);
        assert_eq!(zero_budget_create, scoped_create);
        assert!(zero_budget_stats.accepted_frontier_session_oversize > 0);
        assert!(zero_budget_stats.external_exact_session_oversize > 0);
        assert_eq!(
            zero_budget_stats.accepted_frontier_session_peak_resident_bytes,
            0
        );
        assert_eq!(
            zero_budget_stats.external_exact_session_peak_resident_bytes,
            0
        );
        assert!(cached_stats.accepted_frontier_session_misses > 0);
        assert!(cached_stats.external_exact_session_misses > 0);
        assert!(
            cached_stats.accepted_frontier_session_peak_resident_bytes
                <= crate::oplog::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT
        );
        assert!(
            cached_stats.external_exact_session_peak_resident_bytes
                <= crate::oplog::hot_engine::BOOTSTRAP_LOOKUP_SESSION_BYTES_PER_ROOT
        );
        zero_budget_database
            .apply_authenticated_reference_catalog_materialized_accepted(
                &create_event,
                zero_budget_create,
                &engine,
            )
            .unwrap();
        let zero_budget_frontier = zero_budget_database.frontier_root().unwrap();
        let zero_budget_row_digest = zero_budget_database
            .materialized_row_digest_for_harness()
            .unwrap();
        drop(zero_budget_database);
        let cached_projection = open_test_projection(
            &dir.path().join("cached.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            zero_budget_frontier,
            cached_projection.database.frontier_root().unwrap()
        );
        assert_eq!(
            zero_budget_row_digest,
            cached_projection
                .database
                .materialized_row_digest_for_harness()
                .unwrap()
        );
        drop(cached_projection);
        let mut bulk_projection = bulk_materializer
            .materialize_pages_for_projection(&[ids.page, second_page])
            .unwrap();
        let mut point_projection = vec![
            Some(engine.materialize_page_for_projection(ids.page).unwrap()),
            Some(engine.materialize_page_for_projection(second_page).unwrap()),
        ];
        for state in bulk_projection
            .iter_mut()
            .chain(point_projection.iter_mut())
            .flatten()
        {
            state.page.stats = crate::oplog::MaterializationStats::default();
        }
        assert_eq!(
            bulk_projection, point_projection,
            "bounded bulk projection semantics must equal ordinary pointwise projection"
        );
        assert_eq!(scoped_create.replacements().len(), 2);
        assert_eq!(create_stats.accepted_root_authentications, 1);
        assert_eq!(create_stats.exact_catalog_loads, 1);
        assert_eq!(
            attach_authenticated_reference_catalog(&engine, &create_event, scoped_create.clone(),)
                .unwrap(),
            attach_authenticated_reference_catalog(&engine, &create_event, point_create).unwrap(),
            "aliases and raw reference evidence must be identical"
        );

        let moved = engine
            .prepare_bootstrap_transaction(
                author(2_251),
                &OperationTransaction::new(vec![SemanticOperation::MoveSubtree {
                    root: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.document,
                    },
                    from_page_id: ids.page,
                    to_page_id: second_page,
                    parent: None,
                    order: "b".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &moved);
        let move_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, moved.manifest().batch_id())
                .unwrap();
        let (scoped_move, move_stats) =
            materialize_accepted_event_with_stats(&engine, &move_event).unwrap();
        let point_move = materialize_accepted_event_pointwise(&engine, &move_event).unwrap();
        assert_eq!(scoped_move, point_move);
        assert_eq!(scoped_move.replacements().len(), 2);
        assert_eq!(move_stats.accepted_root_authentications, 1);
        assert_eq!(move_stats.exact_catalog_loads, 1);
        let moved_block = scoped_move
            .replacements()
            .iter()
            .find(|page| page.page_id == second_page)
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.block_id == ids.block)
            .unwrap();
        assert_eq!(moved_block.home_document_id, ids.document);
        assert_eq!(moved_block.task.as_ref().unwrap().marker, "TODO");
        assert_eq!(moved_block.properties[0].name, "owner");
        assert!(moved_block.tags.contains(&"project".to_owned()));

        let deleted = engine
            .prepare_bootstrap_transaction(
                author(2_252),
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: ids.page,
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &deleted);
        let delete_event =
            AcceptedBatchEvent::from_accepted(&engine, &store, deleted.manifest().batch_id())
                .unwrap();
        let (scoped_delete, delete_stats) =
            materialize_accepted_event_with_stats(&engine, &delete_event).unwrap();
        assert_eq!(
            scoped_delete,
            materialize_accepted_event_pointwise(&engine, &delete_event).unwrap()
        );
        assert_eq!(scoped_delete.deletions(), &[ids.page]);
        assert_eq!(delete_stats.accepted_root_authentications, 1);
        assert_eq!(delete_stats.exact_catalog_loads, 1);
    }

    #[test]
    fn historical_page_lookup_is_point_local_and_rejects_foreign_roots() {
        const PAGE_COUNT: u128 = 8;
        let ids = TestIds::new(2_300);
        let dir = TestDir::new("historical-root-locality");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut operations = Vec::new();
        for index in 0..PAGE_COUNT {
            let page_id = if index == 0 {
                ids.page
            } else {
                PageId::from_uuid(uuid(2_400 + index))
            };
            let document_id = if index == 0 {
                ids.document
            } else {
                DocumentId::from_uuid(uuid(2_500 + index))
            };
            let block_id = if index == 0 {
                ids.block
            } else {
                BlockId::from_uuid(uuid(2_600 + index))
            };
            operations.push(SemanticOperation::CreatePage {
                page_id,
                home_document_id: document_id,
                name: crate::oplog::LogicalPageName::parse(&format!("Locality {index}")).unwrap(),
                path: ManagedPath::parse(&format!("pages/locality-{index}.md")).unwrap(),
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
                content: format!("locality content {index}"),
            });
        }
        let create = engine
            .prepare_bootstrap_transaction(
                author(2_310),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &create);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, create.manifest().batch_id())
                .unwrap();
        let page = engine
            .materialize_page_at_accepted_root(event.post_frontier_root(), ids.page)
            .unwrap()
            .unwrap();
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.stats.distinct_home_documents, vec![ids.document]);
        assert!(
            page.stats.physical_manifest_reads <= 2,
            "one historical page read scanned unrelated manifests: {:?}",
            page.stats
        );
        assert!(
            page.stats.physical_object_reads <= 2,
            "one historical page read scanned unrelated objects: {:?}",
            page.stats
        );

        let foreign_ids = TestIds::new(2_700);
        let foreign_dir = TestDir::new("foreign-historical-root");
        let foreign_store =
            ObjectStore::open(&foreign_dir.path().join("objects"), foreign_ids.workspace).unwrap();
        let foreign_engine_store =
            ObjectStore::open(&foreign_dir.path().join("objects"), foreign_ids.workspace).unwrap();
        let mut foreign_engine = ShardedHotEngine::with_archive_store(
            foreign_engine_store,
            foreign_ids.lineage,
            foreign_ids.catalog,
        );
        let foreign_create = foreign_engine
            .prepare_bootstrap_transaction(
                author(2_710),
                &root_transaction(foreign_ids, "pages/foreign.md", "foreign original"),
            )
            .unwrap();
        publish_and_stage_archive(&mut foreign_engine, &foreign_store, &foreign_create);
        let foreign_first = AcceptedBatchEvent::from_accepted(
            &foreign_engine,
            &foreign_store,
            foreign_create.manifest().batch_id(),
        )
        .unwrap();
        assert!(engine
            .materialize_page_at_accepted_root(foreign_first.post_frontier_root(), ids.page)
            .is_err());

        let foreign_edit = foreign_engine
            .prepare_bootstrap_transaction(
                author(2_711),
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: foreign_ids.block,
                        home_document_id: foreign_ids.document,
                    },
                    content: "foreign edited".into(),
                }])
                .unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut foreign_engine, &foreign_store, &foreign_edit);
        let foreign_second = AcceptedBatchEvent::from_accepted(
            &foreign_engine,
            &foreign_store,
            foreign_edit.manifest().batch_id(),
        )
        .unwrap();
        let error = engine
            .materialize_page_at_accepted_root(foreign_second.post_frontier_root(), ids.page)
            .unwrap_err();
        assert!(
            error.to_string().contains("sequence 2 is unavailable"),
            "foreign unavailable root failed for the wrong reason: {error}"
        );
    }

    #[test]
    fn cross_page_move_subtree_preserves_inbound_block_referrers_in_both_page_orders() {
        fn page_replacement(
            page_id: PageId,
            home_document_id: DocumentId,
            path: &str,
            name: &str,
            blocks: Vec<MaterializedBlockInput>,
        ) -> MaterializedPageInput {
            MaterializedPageInput {
                page_id,
                home_document_id,
                name: name.into(),
                name_key: name.to_lowercase(),
                path: ManagedPath::parse(path).unwrap(),
                kind: ManagedTextKind::Page,
                preamble: None,
                searchable_text: format!("{name} searchable"),
                references: Vec::new(),
                properties: Vec::new(),
                tags: Vec::new(),
                blocks,
            }
        }

        fn block_replacement(
            block_id: BlockId,
            home_document_id: DocumentId,
            content: &str,
            references: Vec<MaterializedReference>,
        ) -> MaterializedBlockInput {
            MaterializedBlockInput {
                block_id,
                home_document_id,
                parent: None,
                order: "a".into(),
                content: content.into(),
                searchable_text: content.into(),
                heading_level: None,
                collapsed: false,
                logseq_uuid: None,
                logseq_identity_origin: None,
                references,
                properties: Vec::new(),
                tags: Vec::new(),
                task: None,
            }
        }

        for (case, source_before_destination) in [(0_u128, true), (1, false)] {
            let ids = TestIds::new(2_500 + case * 100);
            let dir = TestDir::new(&format!("cross-page-materialization-move-{case}"));
            let (mut database, mut engine, store) = open_empty(&dir, ids);
            let (source_page, destination_page) = if source_before_destination {
                (
                    PageId::from_uuid(uuid(2_600 + case * 100)),
                    PageId::from_uuid(uuid(2_601 + case * 100)),
                )
            } else {
                (
                    PageId::from_uuid(uuid(2_701 + case * 100)),
                    PageId::from_uuid(uuid(2_700 + case * 100)),
                )
            };
            let source_document = DocumentId::from_uuid(uuid(2_800 + case * 100));
            let destination_document = DocumentId::from_uuid(uuid(2_801 + case * 100));
            let referrer_page = PageId::from_uuid(uuid(2_802 + case * 100));
            let referrer_document = DocumentId::from_uuid(uuid(2_803 + case * 100));
            let target_block = BlockId::from_uuid(uuid(2_804 + case * 100));
            let referrer_block = BlockId::from_uuid(uuid(2_805 + case * 100));
            let source_path = format!("moves/{case}/source.md");
            let destination_path = format!("moves/{case}/destination.md");
            let referrer_path = format!("moves/{case}/referrer.md");

            let root_prepared = engine
                .prepare_bootstrap_transaction(
                    author(2_900 + case * 100),
                    &OperationTransaction::new(vec![
                        SemanticOperation::CreatePage {
                            page_id: source_page,
                            home_document_id: source_document,
                            name: crate::oplog::LogicalPageName::parse("Move Source").unwrap(),
                            path: ManagedPath::parse(&source_path).unwrap(),
                            kind: ManagedTextKind::Page,
                        },
                        SemanticOperation::CreatePage {
                            page_id: destination_page,
                            home_document_id: destination_document,
                            name: crate::oplog::LogicalPageName::parse("Move Destination").unwrap(),
                            path: ManagedPath::parse(&destination_path).unwrap(),
                            kind: ManagedTextKind::Page,
                        },
                        SemanticOperation::CreatePage {
                            page_id: referrer_page,
                            home_document_id: referrer_document,
                            name: crate::oplog::LogicalPageName::parse("Move Referrer").unwrap(),
                            path: ManagedPath::parse(&referrer_path).unwrap(),
                            kind: ManagedTextKind::Page,
                        },
                        SemanticOperation::CreateBlock {
                            block: BlockLocation {
                                block_id: target_block,
                                home_document_id: source_document,
                            },
                            page_id: source_page,
                            parent: None,
                            order: "a".into(),
                            content: "target".into(),
                        },
                        SemanticOperation::CreateBlock {
                            block: BlockLocation {
                                block_id: referrer_block,
                                home_document_id: referrer_document,
                            },
                            page_id: referrer_page,
                            parent: None,
                            order: "a".into(),
                            content: "referrer".into(),
                        },
                    ])
                    .unwrap(),
                )
                .unwrap();
            publish_and_stage(&mut engine, &store, &root_prepared);
            let root_event = AcceptedBatchEvent::from_accepted(
                &engine,
                &store,
                root_prepared.manifest().batch_id(),
            )
            .unwrap();
            let root_change = MaterializationChange::new(
                root_event.batch_id(),
                vec![
                    page_replacement(
                        source_page,
                        source_document,
                        &source_path,
                        "Move Source",
                        vec![block_replacement(
                            target_block,
                            source_document,
                            "target",
                            Vec::new(),
                        )],
                    ),
                    page_replacement(
                        destination_page,
                        destination_document,
                        &destination_path,
                        "Move Destination",
                        Vec::new(),
                    ),
                    page_replacement(
                        referrer_page,
                        referrer_document,
                        &referrer_path,
                        "Move Referrer",
                        vec![block_replacement(
                            referrer_block,
                            referrer_document,
                            "referrer",
                            vec![MaterializedReference {
                                target: MaterializedEntityId::Block(target_block),
                                kind: MaterializedReferenceKind::Reference,
                            }],
                        )],
                    ),
                ],
                Vec::new(),
            )
            .unwrap();
            database
                .apply_authenticated_reference_catalog_materialized_accepted(
                    &root_event,
                    root_change,
                    &engine,
                )
                .unwrap();

            let move_prepared = engine
                .prepare_bootstrap_transaction(
                    author(2_901 + case * 100),
                    &OperationTransaction::new(vec![SemanticOperation::MoveSubtree {
                        root: BlockLocation {
                            block_id: target_block,
                            home_document_id: source_document,
                        },
                        from_page_id: source_page,
                        to_page_id: destination_page,
                        parent: None,
                        order: "a".into(),
                    }])
                    .unwrap(),
                )
                .unwrap();
            publish_and_stage(&mut engine, &store, &move_prepared);
            let move_event = AcceptedBatchEvent::from_accepted(
                &engine,
                &store,
                move_prepared.manifest().batch_id(),
            )
            .unwrap();
            let move_change = materialize_accepted_event(&engine, &move_event).unwrap();
            let source = move_change
                .replacements()
                .iter()
                .find(|page| page.page_id == source_page)
                .unwrap();
            let destination = move_change
                .replacements()
                .iter()
                .find(|page| page.page_id == destination_page)
                .unwrap();
            assert!(source.blocks.is_empty());
            assert_eq!(destination.blocks.len(), 1);
            assert_eq!(destination.blocks[0].block_id, target_block);
            assert_eq!(
                destination.blocks[0].home_document_id, source_document,
                "a cross-page move must preserve the block's immutable home"
            );
            database
                .apply_authenticated_reference_catalog_materialized_accepted(
                    &move_event,
                    move_change,
                    &engine,
                )
                .unwrap();

            assert!(database.authenticated_reference_catalog_root().is_ok());

            let read = database.materialized_read().unwrap();
            assert_eq!(
                read.block(target_block).unwrap().unwrap().page_id,
                destination_page
            );
            assert_eq!(
                read.referrers_to(MaterializedEntityId::Block(target_block), 10)
                    .unwrap(),
                vec![MaterializedReferrerRow {
                    source: MaterializedEntityId::Block(referrer_block),
                    source_page_id: referrer_page,
                    kind: MaterializedReferenceKind::Reference,
                }],
                "case {case}: inbound referrer must survive moving across replacement pages"
            );
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
        let root_record = load_batch(&database.physical, root.batch_id())
            .unwrap()
            .unwrap();
        database
            .physical
            .execute_corrupting_statement_for_test(
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
            let child_record = load_batch(&database.physical, child.batch_id())
                .unwrap()
                .unwrap();
            match case {
                "batch-record" => {
                    database
                        .physical
                        .execute_corrupting_statement_for_test(
                            "DELETE FROM applied_batches WHERE batch_id = ?1",
                            [uuid_blob(&child.batch_id().as_uuid())],
                        )
                        .unwrap();
                }
                "clock-node" => {
                    database
                        .physical
                        .execute_corrupting_statement_for_test(
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
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let root = engine
            .prepare_bootstrap_transaction(
                author(2_200),
                &root_transaction(ids, "pages/root.md", "root"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &root);
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
        publish_and_stage_archive(&mut engine, &store, &child);
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
                name: crate::oplog::LogicalPageName::parse(format!("Wide {index}")).unwrap(),
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
                name: crate::oplog::LogicalPageName::parse(format!("Authenticated {index}"))
                    .unwrap(),
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
                &root_transaction_named(
                    base,
                    "pages/root-sequence-left.md",
                    "Root Sequence Left",
                    "left",
                ),
            )
            .unwrap();
        let right_batch = right
            .engine()
            .prepare_bootstrap_transaction(
                author(2_491),
                &root_transaction_named(
                    right,
                    "pages/root-sequence-right.md",
                    "Root Sequence Right",
                    "right",
                ),
            )
            .unwrap();
        store
            .publish_bootstrap_prepared_for_test(&left_batch)
            .unwrap();
        store
            .publish_bootstrap_prepared_for_test(&right_batch)
            .unwrap();
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
            .physical
            .execute_corrupting_statement_for_test(
                "DELETE FROM applied_batches WHERE sequence = 1",
                [],
            )
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
    fn sampled_interior_block_corruption_rebuilds_from_authority_and_preserves_evidence() {
        const BLOCK_COUNT: usize = 128;
        const ORIGINAL: &[u8] = b"authoritative-row";
        const COUNTERFEIT: &[u8] = b"counterfeited-row";

        let ids = TestIds::new(2_385);
        let dir = TestDir::new("sampled-interior-corruption");
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut operations = vec![SemanticOperation::CreatePage {
            page_id: ids.page,
            home_document_id: ids.document,
            name: crate::oplog::LogicalPageName::parse("Interior authority").unwrap(),
            path: ManagedPath::parse("pages/interior-authority.md").unwrap(),
            kind: ManagedTextKind::Page,
        }];
        let expected_contents = (0..BLOCK_COUNT)
            .map(|index| format!("authoritative-row-{index:04}-{}", "x".repeat(192)))
            .collect::<Vec<_>>();
        for (index, content) in expected_contents.iter().enumerate() {
            operations.push(SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(40_000 + index as u128)),
                    home_document_id: ids.document,
                },
                page_id: ids.page,
                parent: None,
                order: format!("{index:04}"),
                content: content.clone(),
            });
        }
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(2_386),
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
        drop(opened);

        let patched =
            corrupt_equal_length_sampled_interior_block_payload(&path, ORIGINAL, COUNTERFEIT);
        assert!(patched > 0);
        let recovered = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        let ProjectionRecovery::RebuiltPreservingEvidence { evidence, .. } = &recovered.recovery
        else {
            panic!(
                "sampled interior corruption was not quarantined: {:?}",
                recovered.recovery
            );
        };
        let database_evidence = evidence
            .iter()
            .find(|item| item.original_path == path)
            .expect("rebuild did not preserve the corrupt database");
        assert!(fs::read(&database_evidence.preserved_path)
            .unwrap()
            .windows(COUNTERFEIT.len())
            .any(|window| window == COUNTERFEIT));

        let connection = inspect_connection(&recovered.database);
        let mut statement = connection
            .prepare(
                "SELECT content FROM blocks
                 WHERE page_id = ?1
                 ORDER BY order_key",
            )
            .unwrap();
        let actual_contents = statement
            .query_map([ids.page.as_uuid().as_bytes().as_slice()], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(actual_contents, expected_contents);
        assert_eq!(
            recovered.database.frontier().unwrap(),
            engine.exact_frontier().unwrap()
        );
    }

    #[test]
    fn projection_checkpoint_wire_bytes_match_the_legacy_physical_dto_shape() {
        #[derive(Serialize)]
        struct LegacyBoundedFileCheckpoint {
            length: u64,
            first_chunk_digest: ContentDigest,
            last_chunk_digest: ContentDigest,
            interior_sample_digest: ContentDigest,
        }

        #[derive(Serialize)]
        struct LegacyProjectionCheckpoint {
            schema_version: u32,
            workspace_id: WorkspaceId,
            frontier_root_digest: ContentDigest,
            database: LegacyBoundedFileCheckpoint,
            wal: Option<LegacyBoundedFileCheckpoint>,
        }

        #[derive(Serialize)]
        struct LegacyProjectionCheckpointEnvelope {
            checkpoint: LegacyProjectionCheckpoint,
            digest: ContentDigest,
        }

        fn lower_legacy(checkpoint: PhysicalFileCheckpoint) -> LegacyBoundedFileCheckpoint {
            LegacyBoundedFileCheckpoint {
                length: checkpoint.length,
                first_chunk_digest: checkpoint.first_chunk_digest,
                last_chunk_digest: checkpoint.last_chunk_digest,
                interior_sample_digest: checkpoint.interior_sample_digest,
            }
        }

        let ids = TestIds::new(2_388);
        let dir = TestDir::new("checkpoint-wire-differential");
        let (database, _engine, _store) = open_empty(&dir, ids);
        let checkpoint_path = SqliteFileSet::new(database.path())
            .checkpoint_path()
            .to_path_buf();
        drop(database);
        let bytes = fs::read(checkpoint_path).unwrap();
        let envelope: ProjectionCheckpointEnvelope = postcard::from_bytes(&bytes).unwrap();
        let current_checkpoint_bytes = postcard::to_allocvec(&envelope.checkpoint).unwrap();
        let digest = envelope.digest;
        let checkpoint = envelope.checkpoint;
        let legacy_checkpoint = LegacyProjectionCheckpoint {
            schema_version: checkpoint.schema_version,
            workspace_id: checkpoint.workspace_id,
            frontier_root_digest: checkpoint.frontier_root_digest,
            database: lower_legacy(checkpoint.database),
            wal: checkpoint.wal.map(lower_legacy),
        };
        let legacy_checkpoint_bytes = postcard::to_allocvec(&legacy_checkpoint).unwrap();

        assert_eq!(legacy_checkpoint_bytes, current_checkpoint_bytes);
        assert_eq!(digest, ContentDigest::of(&legacy_checkpoint_bytes));
        assert_eq!(
            postcard::to_allocvec(&LegacyProjectionCheckpointEnvelope {
                checkpoint: legacy_checkpoint,
                digest,
            })
            .unwrap(),
            bytes
        );
    }

    #[test]
    fn core_constructs_the_exact_authenticated_bytes_that_storage_replaces() {
        let ids = TestIds::new(2_389);
        let dir = TestDir::new("checkpoint-core-bytes-storage-publication");
        let (database, _engine, _store) = open_empty(&dir, ids);
        let path = database.path().to_path_buf();
        let files = SqliteFileSet::new(&path);
        let root = AcceptedFrontierRoot::empty();
        let root_bytes = canonical_frontier_root_bytes(&root).unwrap();
        let physical = files.physical_checkpoint().unwrap();
        let checkpoint = ProjectionCheckpoint {
            schema_version: PROJECTION_CHECKPOINT_SCHEMA_VERSION,
            workspace_id: ids.workspace,
            frontier_root_digest: ContentDigest::of(&root_bytes),
            database: physical.database,
            wal: physical.wal,
        };
        let checkpoint_bytes = postcard::to_allocvec(&checkpoint).unwrap();
        let expected = postcard::to_allocvec(&ProjectionCheckpointEnvelope {
            digest: ContentDigest::of(&checkpoint_bytes),
            checkpoint,
        })
        .unwrap();
        fs::write(files.checkpoint_path(), b"predecessor checkpoint").unwrap();

        write_projection_checkpoint(&path, ids.claim(), &root).unwrap();

        assert_eq!(fs::read(files.checkpoint_path()).unwrap(), expected);
        validate_projection_checkpoint(&path, ids.claim(), &root).unwrap();
    }

    #[test]
    fn previous_projection_checkpoint_version_is_rebuilt_not_reinterpreted() {
        let ids = TestIds::new(2_387);
        let dir = TestDir::new("old-checkpoint-version");
        let (database, engine, store) = open_empty(&dir, ids);
        let path = database.path().to_path_buf();
        drop(database);

        let checkpoint_path = SqliteFileSet::new(&path).checkpoint_path().to_path_buf();
        let bytes = fs::read(&checkpoint_path).unwrap();
        let mut envelope: ProjectionCheckpointEnvelope = postcard::from_bytes(&bytes).unwrap();
        envelope.checkpoint.schema_version = PROJECTION_CHECKPOINT_SCHEMA_VERSION - 1;
        envelope.digest = ContentDigest::of(&postcard::to_allocvec(&envelope.checkpoint).unwrap());
        let prior_version_bytes = postcard::to_allocvec(&envelope).unwrap();
        fs::write(&checkpoint_path, &prior_version_bytes).unwrap();

        let recovered = open_test_projection(
            &path,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            recovered.recovery,
            ProjectionRecovery::RebuiltPreservingEvidence { .. }
        ));
        assert_eq!(
            recovered.database.frontier().unwrap(),
            FrontierV2::default()
        );
        let replacement = fs::read(&checkpoint_path).unwrap();
        assert_ne!(replacement, prior_version_bytes);
        validate_projection_checkpoint(&path, ids.claim(), &AcceptedFrontierRoot::empty()).unwrap();
        let temporary_prefix = ".frontier.sqlite-auth.tmp-";
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(temporary_prefix)
        }));
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
        let first = load_batch_at_sequence(&opened.database.physical, 1)
            .unwrap()
            .unwrap()
            .batch_id;
        let last = load_batch_at_sequence(&opened.database.physical, batch_count as i64)
            .unwrap()
            .unwrap()
            .batch_id;
        let (descends, batch_rows_read) =
            batch_descends_from_database_measured(&opened.database.physical, last, first).unwrap();
        assert!(descends);
        assert!(
            batch_rows_read <= 96,
            "authenticated ancestry point lookup read {batch_rows_read} rows"
        );
        let connection = inspect_connection(&opened.database);
        let (clock_nodes, clock_node_bytes): (i64, i64) = connection
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
        let (batch_nodes, batch_node_bytes): (i64, i64) = connection
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
        let page_count: i64 = connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let page_size: i64 = connection
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
        drop(connection);
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
        assert_eq!(small.physical_ordinary_transactions, 24);
        assert_eq!(small.physical_ordinary_durability_barriers, 24);
        assert_eq!(large.physical_ordinary_transactions, 48);
        assert_eq!(large.physical_ordinary_durability_barriers, 48);
        assert_eq!(small.physical_candidate_transactions, 0);
        assert_eq!(large.physical_candidate_transactions, 0);
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
                &root_transaction_named(base, "pages/left.md", "Concurrent Left", "left"),
            )
            .unwrap();
        let right_batch = right
            .engine()
            .prepare_bootstrap_transaction(
                author(2_501),
                &root_transaction_named(right, "pages/right.md", "Concurrent Right", "right"),
            )
            .unwrap();
        store
            .publish_bootstrap_prepared_for_test(&left_batch)
            .unwrap();
        store
            .publish_bootstrap_prepared_for_test(&right_batch)
            .unwrap();
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
        good_store
            .publish_bootstrap_prepared_for_test(&good)
            .unwrap();
        evil_store
            .publish_bootstrap_prepared_for_test(&evil)
            .unwrap();
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
                load_batch(&database.physical, root.batch_id())
                    .unwrap()
                    .unwrap()
                    .causal_record_digest()
                    .unwrap(),
            ),
            (
                child.batch_id(),
                load_batch(&database.physical, child.batch_id())
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
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let root_prepared = engine
            .prepare_bootstrap_transaction(
                author(4_010),
                &root_transaction(ids, "pages/overlay.md", "root"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &root_prepared);
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
        publish_and_stage_archive(&mut engine, &store, &child_prepared);
        let child = AcceptedBatchEvent::from_accepted(
            &engine,
            &store,
            child_prepared.manifest().batch_id(),
        )
        .unwrap();
        let source = RebuildSource::new(&engine, &store).unwrap();
        let mut overlay = TailOverlay::empty_for_test(&engine);
        assert!(overlay.try_enqueue(&mut database, &engine, &child).unwrap());
        assert_eq!(
            overlay.status().retained_bytes,
            usize::try_from(child.post_frontier_root().retained_bytes_total()).unwrap()
        );
        assert!(overlay.try_enqueue(&mut database, &engine, &root).unwrap());
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
        assert!(!overlay.try_enqueue(&mut database, &engine, &root).unwrap());
        assert_eq!(overlay.status().unapplied_batches, 0);

        let mut count_limited = TailOverlay::empty_for_test(&engine);
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

        let mut byte_limited = TailOverlay::empty_for_test(&engine);
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

    #[derive(Clone, Copy, Debug)]
    enum AuthoritySubstitutionPath {
        TryEnqueue,
        EnqueueReserved,
        DrainReady,
    }

    #[test]
    fn engine_owned_apply_rejects_honest_same_lineage_substitute_before_mutation() {
        let ids = TestIds::new(4_300);
        let dir = TestDir::new("engine-owned-apply-authority");
        let store = ObjectStore::open(&dir.path().join("primary-objects"), ids.workspace).unwrap();
        let engine_store =
            ObjectStore::open(&dir.path().join("primary-objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(4_310),
                &root_transaction(
                    ids,
                    "pages/engine-owned.md",
                    "TODO [#A] engine-owned #authority",
                ),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();

        let substitute_store =
            ObjectStore::open(&dir.path().join("substitute-objects"), ids.workspace).unwrap();
        substitute_store
            .publish_bootstrap_prepared_for_test(&prepared)
            .unwrap();
        let substitute_engine_store =
            ObjectStore::open(&dir.path().join("substitute-objects"), ids.workspace).unwrap();
        let mut substitute_engine =
            ShardedHotEngine::with_archive_store(substitute_engine_store, ids.lineage, ids.catalog);
        assert!(matches!(
            substitute_engine
                .stage_archive_batch(prepared.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));
        let substitute_event = AcceptedBatchEvent::from_accepted(
            &substitute_engine,
            &substitute_store,
            prepared.manifest().batch_id(),
        )
        .unwrap();
        assert_eq!(substitute_event.batch_id(), event.batch_id());
        assert_eq!(
            substitute_event.post_frontier_root(),
            event.post_frontier_root()
        );
        authenticate_event_for_engine(&substitute_engine, &event).unwrap();

        let logical_before = database_logical_state(&database);
        let rows_before = database
            .physical
            .authority_rejection_snapshot_for_test()
            .unwrap();
        let total_changes_before = database.physical.total_changes_for_test();
        let checkpoint_path = SqliteFileSet::new(database.path())
            .checkpoint_path()
            .to_path_buf();
        let checkpoint_before = fs::read(&checkpoint_path).unwrap();

        assert_eq!(
            database.apply_engine_owned_accepted(&event, &substitute_engine),
            Err(ProjectionError::AuthorityMismatch)
        );
        assert_eq!(database_logical_state(&database), logical_before);
        let rows_after = database
            .physical
            .authority_rejection_snapshot_for_test()
            .unwrap();
        assert_eq!(rows_after, rows_before);
        assert_eq!(
            database.physical.total_changes_for_test(),
            total_changes_before
        );
        assert_eq!(fs::read(&checkpoint_path).unwrap(), checkpoint_before);

        assert_eq!(
            database
                .apply_engine_owned_accepted(&event, &engine)
                .unwrap(),
            ApplyDisposition::Applied
        );
        assert_eq!(database.applied_batch_count().unwrap(), 1);
        let read = database.materialized_read().unwrap();
        assert_eq!(
            read.block(ids.block).unwrap().unwrap().content,
            "TODO [#A] engine-owned #authority"
        );
        assert_eq!(read.tasks(Some("TODO"), 10).unwrap().len(), 1);
        assert_eq!(read.tags("authority", 10).unwrap().len(), 1);
        assert_eq!(read.search("authority", 10).unwrap().len(), 1);
    }

    #[test]
    fn production_sqlite_mutation_surface_is_engine_owned() {
        let source = include_str!("sqlite.rs");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests")
            .map(|(production, _)| production)
            .expect("SQLite source keeps a distinct test module");
        for unavailable in [
            concat!("pub fn apply_", "accepted("),
            concat!("pub fn apply_", "materialized_accepted("),
            concat!(
                "pub fn apply_authenticated_reference_catalog_",
                "materialized_accepted("
            ),
            concat!("pub fn rebuild_", "materialization"),
            concat!(
                "pub fn rebuild_authenticated_reference_catalog_",
                "materialization"
            ),
            concat!("pub fn apply_internal_", "with_materialization("),
        ] {
            assert!(
                !production.contains(unavailable),
                "non-test caller-controlled SQLite mutation surface remains: {unavailable}"
            );
        }
        for test_only in [
            "#[cfg(test)]\n    fn apply_accepted(",
            "#[cfg(test)]\n    fn apply_materialized_accepted(",
            "#[cfg(test)]\n    fn apply_authenticated_reference_catalog_materialized_accepted(",
            "#[cfg(test)]\n    fn rebuild_materialization",
            "#[cfg(test)]\n    fn rebuild_authenticated_reference_catalog_materialization",
            "#[cfg(test)]\n    fn rebuild_materialization_inner",
            "#[cfg(test)]\n    fn apply_internal(",
        ] {
            assert!(
                production.contains(test_only),
                "fixture mutation helper is not test-only: {test_only}"
            );
        }
        assert!(production.contains("fn apply_engine_owned_accepted("));
    }

    fn assert_authority_substitution_is_atomic(
        seed: u128,
        foreign_workspace: bool,
        path: AuthoritySubstitutionPath,
    ) {
        let ids = TestIds::new(seed);
        let dir = TestDir::new("authority-substitution-local");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(seed + 10),
                &root_transaction(ids, "pages/local-tail.md", "local accepted"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let local =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();

        let substitute_ids = if foreign_workspace {
            TestIds::new(seed + 100)
        } else {
            ids
        };
        let substitute_dir = TestDir::new("authority-substitution-source");
        let substitute_store = ObjectStore::open(
            &substitute_dir.path().join("objects"),
            substitute_ids.workspace,
        )
        .unwrap();
        let substitute_engine_store = ObjectStore::open(
            &substitute_dir.path().join("objects"),
            substitute_ids.workspace,
        )
        .unwrap();
        let mut substitute_engine = ShardedHotEngine::with_archive_store(
            substitute_engine_store,
            substitute_ids.lineage,
            substitute_ids.catalog,
        );
        let substitute_prepared = substitute_engine
            .prepare_bootstrap_transaction(
                author(seed + 10),
                &root_transaction(
                    substitute_ids,
                    "pages/substitute-tail.md",
                    "substitute accepted",
                ),
            )
            .unwrap();
        publish_and_stage_archive(
            &mut substitute_engine,
            &substitute_store,
            &substitute_prepared,
        );
        let substitute = AcceptedBatchEvent::from_accepted(
            &substitute_engine,
            &substitute_store,
            substitute_prepared.manifest().batch_id(),
        )
        .unwrap();
        assert_eq!(
            local.acceptance_sequence(),
            substitute.acceptance_sequence()
        );
        if !foreign_workspace {
            assert_eq!(local.batch_id(), substitute.batch_id());
            assert_ne!(local.post_frontier_root(), substitute.post_frontier_root());
        }

        let mut overlay = TailOverlay::empty_for_test(&engine);
        let reservation = matches!(path, AuthoritySubstitutionPath::EnqueueReserved)
            .then(|| overlay.reserve_mutation(local.retained_bytes()).unwrap());
        let overlay_before = overlay_logical_state(&overlay);
        let database_before = database_logical_state(&database);
        let error = match path {
            AuthoritySubstitutionPath::TryEnqueue => overlay
                .try_enqueue(&mut database, &substitute_engine, &substitute)
                .unwrap_err(),
            AuthoritySubstitutionPath::EnqueueReserved => overlay
                .enqueue_reserved(
                    reservation.unwrap(),
                    &mut database,
                    &substitute_engine,
                    substitute,
                )
                .unwrap_err(),
            AuthoritySubstitutionPath::DrainReady => {
                let substitute_source =
                    RebuildSource::new(&substitute_engine, &substitute_store).unwrap();
                overlay
                    .drain_ready(&mut database, &substitute_source, 1)
                    .unwrap_err()
            }
        };
        assert_eq!(
            error,
            TailOverlayError::Projection(ProjectionError::AuthorityMismatch)
        );
        assert_eq!(overlay_logical_state(&overlay), overlay_before);
        assert_eq!(database_logical_state(&database), database_before);
        assert!(database.materialized_read().is_ok());
        if let Some(reservation) = reservation {
            overlay.cancel_reservation(reservation).unwrap();
        }

        assert!(overlay.try_enqueue(&mut database, &engine, &local).unwrap());
        let source = RebuildSource::new(&engine, &store).unwrap();
        assert_eq!(overlay.drain_ready(&mut database, &source, 1).unwrap(), 1);
        let block = database
            .materialized_read()
            .unwrap()
            .block(ids.block)
            .unwrap()
            .unwrap();
        assert_eq!(block.content, "local accepted");
    }

    #[test]
    fn foreign_engine_and_source_substitution_all_paths_are_atomic() {
        for (index, path) in [
            AuthoritySubstitutionPath::TryEnqueue,
            AuthoritySubstitutionPath::EnqueueReserved,
            AuthoritySubstitutionPath::DrainReady,
        ]
        .into_iter()
        .enumerate()
        {
            assert_authority_substitution_is_atomic(4_400 + index as u128 * 1_000, true, path);
        }
    }

    #[test]
    fn same_id_divergent_engine_and_source_substitution_all_paths_are_atomic() {
        for (index, path) in [
            AuthoritySubstitutionPath::TryEnqueue,
            AuthoritySubstitutionPath::EnqueueReserved,
            AuthoritySubstitutionPath::DrainReady,
        ]
        .into_iter()
        .enumerate()
        {
            assert_authority_substitution_is_atomic(8_400 + index as u128 * 1_000, false, path);
        }
    }

    #[test]
    fn post_authentication_retained_byte_failure_preserves_gate_accounting_and_reservation() {
        let ids = TestIds::new(12_400);
        let dir = TestDir::new("atomic-retained-byte-admission");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(12_410),
                &root_transaction(ids, "pages/atomic-tail.md", "atomic accepted"),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();

        let mut overlay = TailOverlay::empty_for_test(&engine);
        overlay.applied_retained_bytes_total = event
            .post_frontier_root()
            .retained_bytes_total()
            .checked_add(1)
            .unwrap();
        let reservation = overlay.reserve_mutation(event.retained_bytes()).unwrap();
        let overlay_before = overlay_logical_state(&overlay);
        let database_before = database_logical_state(&database);
        assert_eq!(
            overlay.enqueue_reserved(reservation, &mut database, &engine, event),
            Err(TailOverlayError::Projection(
                ProjectionError::FrontierRegression
            ))
        );
        assert_eq!(overlay_logical_state(&overlay), overlay_before);
        assert_eq!(database_logical_state(&database), database_before);
        overlay.cancel_reservation(reservation).unwrap();
        assert!(!overlay.reservations.contains_key(&reservation.id));
    }

    #[test]
    fn provider_tail_over_cap_retains_only_bounded_hot_descriptors() {
        let engine = TestIds::new(16_400).engine();
        let mut overlay = TailOverlay::empty_for_test(&engine);
        let tail = TAIL_MAX_BATCHES + 257;
        for index in (0..tail).rev() {
            let acceptance_sequence = index as u64 + 1;
            assert!(overlay
                .record_authoritative_descriptor(
                    acceptance_sequence,
                    acceptance_sequence,
                    TailDescriptor {
                        batch_id: batch(80_000 + index as u128),
                        manifest_digest: ContentDigest::of(&index.to_be_bytes()),
                        retained_bytes: 1,
                    },
                )
                .unwrap());
        }
        assert_eq!(overlay.status().unapplied_batches, tail);
        assert!(overlay.status().backpressured);
        assert_eq!(overlay.hot_descriptor_count(), TAIL_MAX_BATCHES);
        assert!(overlay.hot_descriptor_count() <= TAIL_MAX_BATCHES);

        assert!(!overlay
            .record_authoritative_descriptor(
                tail as u64,
                tail as u64,
                TailDescriptor {
                    batch_id: batch(80_000 + (tail - 1) as u128),
                    manifest_digest: ContentDigest::of(&(tail - 1).to_be_bytes()),
                    retained_bytes: 1,
                },
            )
            .unwrap());
        assert_eq!(overlay.hot_descriptor_count(), TAIL_MAX_BATCHES);
    }

    #[test]
    fn oversized_authoritative_event_is_retained_backpressured_and_drainable() {
        let ids = TestIds::new(4_100);
        let dir = TestDir::new("oversized-overlay");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine_store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let mut engine =
            ShardedHotEngine::with_archive_store(engine_store, ids.lineage, ids.catalog);
        let mut database = open_test_projection(
            &dir.path().join("frontier.sqlite"),
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
        )
        .unwrap()
        .database;
        let mut overlay = TailOverlay::empty_for_test(&engine);
        assert!(matches!(
            overlay.reserve_mutation(TAIL_MAX_BYTES + 1),
            Err(TailOverlayError::Backpressure(_))
        ));
        assert_eq!(overlay.status().unapplied_batches, 0);

        let content = "x".repeat(4 * 1024 * 1024);
        let mut operations = vec![SemanticOperation::CreatePage {
            page_id: ids.page,
            home_document_id: ids.document,
            name: crate::oplog::LogicalPageName::parse("oversized").unwrap(),
            path: ManagedPath::parse("pages/oversized.md").unwrap(),
            kind: ManagedTextKind::Page,
        }];
        for index in 0..2_u128 {
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
        let prepared = engine
            .prepare_bootstrap_transaction(
                author(4_300),
                &OperationTransaction::new(operations).unwrap(),
            )
            .unwrap();
        publish_and_stage_archive(&mut engine, &store, &prepared);
        let event =
            AcceptedBatchEvent::from_accepted(&engine, &store, prepared.manifest().batch_id())
                .unwrap();
        assert!(event.retained_bytes() > TAIL_MAX_BYTES);
        assert!(overlay.try_enqueue(&mut database, &engine, &event).unwrap());
        assert_eq!(overlay.status().unapplied_batches, 1);
        assert!(overlay.status().backpressured);
        assert!(overlay.status().retained_bytes > TAIL_MAX_BYTES);

        let source = RebuildSource::new(&engine, &store).unwrap();
        assert_eq!(overlay.drain_ready(&mut database, &source, 1).unwrap(), 1);
        assert_eq!(overlay.status().unapplied_batches, 0);
        assert!(database.contains_batch(event.batch_id()).unwrap());
        let blocks = database
            .materialized_read()
            .unwrap()
            .blocks_on_page(ids.page, 10)
            .unwrap();
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|block| block.content == content));
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
    fn one_workspace_runtime_lease_vends_one_applier_slot_at_a_time() {
        let ids = TestIds::new(8_200);
        let dir = TestDir::new("workspace-lease-affine-slot");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();

        let slot = lease.applier_slot().unwrap();
        assert!(matches!(
            lease.applier_slot(),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(slot);
        let next = lease.applier_slot().unwrap();
        assert!(matches!(
            lease.applier_slot(),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(next);
        drop(lease.applier_slot().unwrap());

        // A second lease over the same archive contends on the OS handle, so
        // slot affinity is a refinement of the workspace lock, not a substitute.
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(lease);
        drop(WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap());

        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, TestIds::new(8_250).workspace),
            Err(ProjectionError::WorkspaceMismatch { .. })
        ));
    }

    #[test]
    fn an_applier_slot_cannot_open_database_authority_for_another_workspace_or_archive() {
        let ids = TestIds::new(8_300);
        let foreign = TestIds::new(8_400);
        let dir = TestDir::new("applier-slot-authority");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let foreign_store =
            ObjectStore::open(&dir.path().join("foreign-objects"), foreign.workspace).unwrap();
        // The same workspace identity published into a substituted archive.
        let substituted_store =
            ObjectStore::open(&dir.path().join("substituted-objects"), ids.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();

        let foreign_engine = foreign.engine();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild_with_applier_slot(
                &dir.path().join("foreign.sqlite"),
                &runtime,
                foreign.claim(),
                RebuildSource::new(&foreign_engine, &foreign_store).unwrap(),
                lease.applier_slot().unwrap(),
            ),
            Err(ProjectionError::WorkspaceMismatch { .. })
        ));

        let engine = ids.engine();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild_with_applier_slot(
                &dir.path().join("substituted.sqlite"),
                &runtime,
                ids.claim(),
                RebuildSource::new(&engine, &substituted_store).unwrap(),
                lease.applier_slot().unwrap(),
            ),
            Err(ProjectionError::UnsafePath(_))
        ));

        // Neither rejected open took a database-adjacent lock, and each
        // returned its slot to the same lease.
        assert!(!dir
            .path()
            .join(".foreign.sqlite.database-applier.lock")
            .exists());
        assert!(!dir
            .path()
            .join(".substituted.sqlite.database-applier.lock")
            .exists());
        let opened = SqliteFrontier::open_or_rebuild_with_applier_slot(
            &dir.path().join("own.sqlite"),
            &runtime,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
            lease.applier_slot().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            opened.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        ));
    }

    #[test]
    fn the_database_adjacent_lock_contends_independently_of_the_workspace_lease() {
        let a = TestIds::new(8_500);
        let b = TestIds::new(8_600);
        let dir = TestDir::new("database-adjacent-lock");
        let store_a = ObjectStore::open(&dir.path().join("objects-a"), a.workspace).unwrap();
        let store_b = ObjectStore::open(&dir.path().join("objects-b"), b.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        // Distinct archives, so both workspace leases are held at once.
        let lease_a = WorkspaceRuntimeLease::acquire(&store_a, a.workspace).unwrap();
        let lease_b = WorkspaceRuntimeLease::acquire(&store_b, b.workspace).unwrap();
        let engine_a = a.engine();
        let engine_b = b.engine();
        let shared = dir.path().join("shared.sqlite");

        let opened = SqliteFrontier::open_or_rebuild_with_applier_slot(
            &shared,
            &runtime,
            a.claim(),
            RebuildSource::new(&engine_a, &store_a).unwrap(),
            lease_a.applier_slot().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            SqliteFrontier::open_or_rebuild_with_applier_slot(
                &shared,
                &runtime,
                b.claim(),
                RebuildSource::new(&engine_b, &store_b).unwrap(),
                lease_b.applier_slot().unwrap(),
            ),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(opened);
        let recovered = SqliteFrontier::open_or_rebuild_with_applier_slot(
            &shared,
            &runtime,
            b.claim(),
            RebuildSource::new(&engine_b, &store_b).unwrap(),
            lease_b.applier_slot().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            recovered.recovery,
            ProjectionRecovery::RebuiltPreservingEvidence { .. }
        ));
    }

    /// One retained workspace lease closes the inactive-bootstrap database and
    /// opens the promoted database from the same applier slot. A competing
    /// process, running under its own XDG/HOME roots, is probed at every step
    /// and stays blocked until the lease itself is released.
    #[test]
    fn a_retained_workspace_lease_hands_the_applier_slot_across_a_database_handoff() {
        let seed = 8_100;
        let ids = TestIds::new(seed);
        let dir = TestDir::new("workspace-lease-handoff");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let engine = ids.engine();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let probe_xdg = dir.path().join("probe-profile/xdg");
        let probe_home = dir.path().join("probe-profile/home");
        for path in [&probe_xdg, &probe_home] {
            fs::create_dir_all(path).unwrap();
        }

        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        // Destructured, not partially moved: the borrow checker refuses to drop
        // the lease while any part of a leased projection is still live, which
        // is the compile-time half of "the slot cannot outlive its lease".
        let LeasedOpenProjection {
            database: bootstrap,
            recovery: bootstrap_recovery,
            ..
        } = SqliteFrontier::open_or_rebuild_with_applier_slot(
            &dir.path().join("inactive-bootstrap.sqlite"),
            &runtime,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
            lease.applier_slot().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            bootstrap_recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        ));

        let mut probe = WorkspaceLeaseProbe::spawn(dir.path(), seed, &probe_xdg, &probe_home);
        assert_eq!(probe.probe(), "contended");

        let slot = bootstrap.close_returning_applier_slot();
        assert_eq!(probe.probe(), "contended");

        let promoted = SqliteFrontier::open_or_rebuild_with_applier_slot(
            &dir.path().join("promoted.sqlite"),
            &runtime,
            ids.claim(),
            RebuildSource::new(&engine, &store).unwrap(),
            slot,
        )
        .unwrap();
        assert!(matches!(
            promoted.recovery,
            ProjectionRecovery::RebuiltMissing { applied_batches: 0 }
        ));
        assert_eq!(promoted.database.database().claim(), ids.claim());
        assert_eq!(probe.probe(), "contended");

        drop(promoted);
        assert_eq!(probe.probe(), "contended");
        drop(lease);
        assert_eq!(probe.probe(), "acquired");
        probe.finish();
    }

    /// The owning runtime shape: one value that holds the archive-rooted
    /// workspace runtime lease *and* the single database opened under that
    /// lease's applier slot, so neither can be detached from the other.
    ///
    /// A failed open hands the lease back instead of leaking it, a closed
    /// database keeps the lease held, and the borrowed workspace proof the value
    /// vends authorizes exactly its own archive and workspace.
    #[test]
    fn a_leased_workspace_projection_owns_its_lease_and_hands_it_back_on_failure() {
        let ids = TestIds::new(8_700);
        let foreign = TestIds::new(8_800);
        let dir = TestDir::new("leased-workspace-projection");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let foreign_store =
            ObjectStore::open(&dir.path().join("foreign-objects"), foreign.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let engine = ids.engine();
        let foreign_engine = foreign.engine();

        // A failed open returns the lease, so the caller can retry.
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        let (lease, error) =
            LeasedWorkspaceProjection::open_under::<(), ProjectionError>(lease, |slot| {
                SqliteFrontier::open_or_rebuild_with_applier_slot(
                    &dir.path().join("foreign.sqlite"),
                    &runtime,
                    foreign.claim(),
                    RebuildSource::new(&foreign_engine, &foreign_store).unwrap(),
                    slot,
                )
                .map(|opened| (opened, ()))
            })
            .err()
            .expect("a foreign workspace must not open under this lease");
        assert!(matches!(error, ProjectionError::WorkspaceMismatch { .. }));

        // The same lease still works, and its single applier slot came back.
        let (mut projection, ()) =
            LeasedWorkspaceProjection::open_under::<(), ProjectionError>(lease, |slot| {
                SqliteFrontier::open_or_rebuild_with_applier_slot(
                    &dir.path().join("own.sqlite"),
                    &runtime,
                    ids.claim(),
                    RebuildSource::new(&engine, &store).unwrap(),
                    slot,
                )
                .map(|opened| (opened, ()))
            })
            .map_err(|(_lease, error)| error)
            .unwrap();
        assert_eq!(projection.database().claim(), ids.claim());
        // The production split: the mutable database handle and the borrowed
        // while-held lease-identity check, disjointly.
        let (database, workspace) = projection.database_and_lease_identity();
        assert_eq!(
            database.frontier_root().unwrap(),
            engine.accepted_frontier_root().unwrap()
        );
        workspace.revalidate().unwrap();

        // The borrowed workspace proof authorizes this archive and workspace,
        // and refuses a look-alike archive or a foreign workspace.
        let proof = projection.workspace_proof();
        assert_eq!(proof.workspace_id(), ids.workspace);
        proof.authorize_archive(&store, ids.workspace).unwrap();
        assert!(matches!(
            proof.authorize_archive(&foreign_store, foreign.workspace),
            Err(ProjectionError::WorkspaceMismatch { .. })
        ));
        let substituted =
            ObjectStore::open(&dir.path().join("substituted-objects"), ids.workspace).unwrap();
        assert!(matches!(
            proof.authorize_archive(&substituted, ids.workspace),
            Err(ProjectionError::UnsafePath(_))
        ));

        // Closing the database keeps the workspace lease, which is what makes a
        // bootstrap -> promoted handoff possible; only dropping it releases the
        // archive.
        let lease = projection.close_retaining_lease();
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(lease);
        drop(WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap());
    }

    fn workspace_lease_path(archive_root: &Path, workspace: WorkspaceId) -> PathBuf {
        archive_root
            .join(OBJECT_STORE_LEASE_NAMESPACE)
            .join(SQLITE_WORKSPACE_LEASE_NAMESPACE)
            .join(workspace.to_string())
            .join(SQLITE_APPLIER_LEASE_FILE)
    }

    fn open_leased_projection(
        lease: WorkspaceRuntimeLease,
        database_path: &Path,
        runtime: &ApplicationRuntimeRoot,
        ids: TestIds,
        engine: &ShardedHotEngine,
        store: &ObjectStore,
    ) -> LeasedWorkspaceProjection {
        LeasedWorkspaceProjection::open_under::<(), ProjectionError>(lease, |slot| {
            SqliteFrontier::open_or_rebuild_with_applier_slot(
                database_path,
                runtime,
                ids.claim(),
                RebuildSource::new(engine, store).unwrap(),
                slot,
            )
            .map(|opened| (opened, ()))
        })
        .map_err(|(_lease, error)| error)
        .expect("leased projection")
        .0
    }

    /// The declared field order of `LeasedWorkspaceProjection` is the whole
    /// reason a *different* application-data profile cannot take the archive
    /// while this process still has a live database applier. Assert the two
    /// releases in the order they happen, and assert directly that the archive
    /// was still contended at the instant the database lock went away.
    ///
    /// Inverting the field order flips both halves of this oracle.
    #[test]
    fn a_leased_workspace_projection_releases_the_database_lock_before_the_workspace_lease() {
        let ids = TestIds::new(8_900);
        let dir = TestDir::new("leased-projection-drop-order");
        let store = ObjectStore::open(&dir.path().join("objects"), ids.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let engine = ids.engine();

        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        let projection = open_leased_projection(
            lease,
            &dir.path().join("own.sqlite"),
            &runtime,
            ids,
            &engine,
            &store,
        );

        let ((), releases) = recorded_applier_lock_releases(|| drop(projection));
        assert_eq!(
            releases,
            vec![
                ApplierLockRelease::Database {
                    workspace_still_contended: true
                },
                ApplierLockRelease::Workspace,
            ],
            "the database applier must be torn down while this process still owns the archive"
        );
        drop(WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap());
    }

    /// The bootstrap -> promoted database handoff, measured at the lock layer:
    /// closing the database emits exactly one release — the database-adjacent
    /// one — and the archive stays contended straight through into the next
    /// database opened from the same retained lease.
    #[test]
    fn closing_a_leased_workspace_projection_retains_workspace_contention_through_the_handoff() {
        let ids = TestIds::new(9_000);
        let dir = TestDir::new("leased-projection-handoff-order");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let engine = ids.engine();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);

        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        let bootstrap = open_leased_projection(
            lease,
            &dir.path().join("bootstrap.sqlite"),
            &runtime,
            ids,
            &engine,
            &store,
        );

        let (lease, releases) =
            recorded_applier_lock_releases(|| bootstrap.close_retaining_lease());
        assert_eq!(
            releases,
            vec![ApplierLockRelease::Database {
                workspace_still_contended: true
            }],
            "closing the bootstrap database must not release the archive"
        );
        // The database-adjacent lock really is free now, and the archive really
        // is not.
        assert!(workspace_lock_is_contended(&lease_path));
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));

        let promoted = open_leased_projection(
            lease,
            &dir.path().join("promoted.sqlite"),
            &runtime,
            ids,
            &engine,
            &store,
        );
        let ((), releases) = recorded_applier_lock_releases(|| drop(promoted));
        assert_eq!(
            releases,
            vec![
                ApplierLockRelease::Database {
                    workspace_still_contended: true
                },
                ApplierLockRelease::Workspace,
            ]
        );
        assert!(!workspace_lock_is_contended(&lease_path));
    }

    /// The workspace lease file lives *inside* the archive, which the supported
    /// multi-device configuration replicates through Syncthing/Dropbox. An
    /// inode-scoped `flock` survives that only if no provider ever has a reason
    /// to replace the file, so the file must be created empty and never written
    /// again — no pid, no platform, no acquisition timestamp.
    #[test]
    fn the_workspace_lock_file_is_empty_and_no_acquisition_ever_rewrites_it() {
        let ids = TestIds::new(9_100);
        let dir = TestDir::new("workspace-lock-bytes");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);

        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        assert!(fs::read(&lease_path).unwrap().is_empty());
        let first = fs::metadata(&lease_path).unwrap();
        drop(lease);

        // A second acquisition — the case a provider would see as a change —
        // leaves the bytes and the modification time exactly as they were, so
        // there is nothing to replicate and no conflict to resolve.
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        let second = fs::metadata(&lease_path).unwrap();
        assert!(fs::read(&lease_path).unwrap().is_empty());
        assert_eq!(second.len(), 0);
        assert_eq!(first.len(), second.len());
        assert_eq!(first.modified().unwrap(), second.modified().unwrap());

        // A provider's conflict copy is a *sibling*; ownership is decided by one
        // exact name, so the extra file changes nothing.
        let conflict =
            lease_path.with_file_name("sqlite-applier.sync-conflict-20260101-000000-AAAAAAA.lock");
        fs::write(&conflict, b"someone else's copy").unwrap();
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(lease);
        drop(WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap());
        assert_eq!(fs::read(&conflict).unwrap(), b"someone else's copy");
    }

    /// Replace the workspace lock file with a byte-identical new file, exactly
    /// the way an out-of-band action lands one: write a temporary file beside
    /// the target and rename it over the name.
    ///
    /// The bytes are already identical on both sides — that is the point. This
    /// is what a Syncthing receive-only "Revert local changes", a folder
    /// reset/re-add, a delete-then-restore, a `.stversions` restore, or a user
    /// removing `.tine-runtime` by hand does to the locked file: it changes the
    /// file's *identity* while changing nothing an observer of its contents,
    /// length, or name could see.
    fn replace_workspace_lock_file(path: &Path) {
        let incoming = path.with_extension("lock.incoming");
        fs::write(&incoming, b"").unwrap();
        fs::rename(&incoming, path).unwrap();
    }

    /// Replacing the workspace lock file out of band used to split the local
    /// lock: `flock` follows the file, not the name, so the old holder kept a
    /// lock nobody could reach by name while a newcomer opening the same name
    /// locked the new file and also succeeded. Two runtimes, one workspace.
    ///
    /// This is the fail-closed regression for that. The replacement itself
    /// cannot be prevented from inside this crate — and the newcomer is not
    /// wrong, its handle and its name agree — so the invariant is the weaker,
    /// achievable one: **the old holder and a replacement opener can never both
    /// be valid authority**. The old holder's lock stops proving anything the
    /// moment it stops naming the file it locked.
    #[test]
    fn replacing_the_workspace_lock_file_out_of_band_fails_closed_instead_of_splitting_the_lock() {
        let ids = TestIds::new(9_200);
        let dir = TestDir::new("workspace-lock-replacement");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);

        let held = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        held.proof()
            .authorize_archive(&store, ids.workspace)
            .unwrap();
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));

        // The cut: replaced after a successful acquisition, while held.
        replace_workspace_lock_file(&lease_path);

        // The old holder is no longer authority, and says so in a way every
        // caller already routes as fail-closed.
        let error = held
            .proof()
            .authorize_archive(&store, ids.workspace)
            .expect_err("a replaced lease path must not keep authorizing this archive");
        assert!(
            matches!(error, ProjectionError::LeaseIdentityReplaced(_)),
            "unexpected replaced-lease error: {error}"
        );

        // A newcomer legitimately takes the file that is now at the name: it is
        // a fresh, unlocked file and its handle and name agree. That is the
        // half this crate cannot and should not refuse.
        let newcomer = WorkspaceRuntimeLease::acquire(&store, ids.workspace)
            .expect("the replacement file is unlocked, so a newcomer may take it");
        newcomer
            .proof()
            .authorize_archive(&store, ids.workspace)
            .unwrap();

        // The invariant: exactly one of the two is authority, never both.
        assert!(
            held.proof()
                .authorize_archive(&store, ids.workspace)
                .is_err(),
            "the old holder and the replacement opener must not both be authority"
        );
        drop(newcomer);
        drop(held);
    }

    #[test]
    fn removing_a_workspace_lease_component_is_precise_terminal_identity_loss() {
        let ids = TestIds::new(9_205);
        let dir = TestDir::new("workspace-lock-component-missing");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();

        fs::remove_file(&lease_path).unwrap();
        fs::remove_dir(lease_path.parent().unwrap()).unwrap();
        let error = lease
            .proof()
            .authorize_archive(&store, ids.workspace)
            .expect_err("a missing lease-path component positively proves replacement");
        assert!(
            matches!(error, ProjectionError::LeaseIdentityReplaced(_)),
            "unexpected missing-component classification: {error}"
        );
    }

    /// The open-then-lock window inside `WorkspaceRuntimeLease::acquire` is the
    /// other cut: the file is open, the lock is not taken yet, and a replacement
    /// landing in that instant would hand this process a lock on a file no
    /// pathname reaches.
    ///
    /// Driven directly through the acquisition's test interposition point. A
    /// race that keeps winning must exhaust a small explicit number of attempts
    /// and fail closed as the ordinary contended-lease error — never block,
    /// never retry unboundedly, and never return a lock that proves nothing.
    #[test]
    fn a_workspace_lease_losing_the_open_lock_race_retries_boundedly_then_fails_closed() {
        let ids = TestIds::new(9_210);
        let dir = TestDir::new("workspace-lock-open-race");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);

        // Every attempt loses the race.
        let opens = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let counter = std::rc::Rc::clone(&opens);
        set_workspace_lease_open_hook_for_test(Box::new(move |path| {
            counter.set(counter.get() + 1);
            replace_workspace_lock_file(path);
        }));
        let Err(error) = WorkspaceRuntimeLease::acquire(&store, ids.workspace) else {
            panic!("an acquisition that never wins the race must fail closed");
        };
        clear_workspace_lease_open_hook_for_test();
        assert!(
            matches!(&error, ProjectionError::LeaseContended(path) if path == &lease_path),
            "unexpected exhausted-retry error: {error}"
        );
        assert_eq!(
            opens.get(),
            applier_lease::WORKSPACE_LEASE_IDENTITY_ATTEMPTS,
            "the retry must be bounded by one small explicit constant"
        );
        // Every losing attempt released the lock it took, so nothing is stranded.
        assert!(!workspace_lock_is_contended(&lease_path));

        // One lost race is survivable: the next attempt takes the file that is
        // actually at the name, and is authority.
        let opens = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let counter = std::rc::Rc::clone(&opens);
        set_workspace_lease_open_hook_for_test(Box::new(move |path| {
            counter.set(counter.get() + 1);
            if counter.get() == 1 {
                replace_workspace_lock_file(path);
            }
        }));
        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        clear_workspace_lease_open_hook_for_test();
        assert_eq!(opens.get(), 2, "exactly one retry was needed");
        lease
            .proof()
            .authorize_archive(&store, ids.workspace)
            .unwrap();
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&store, ids.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));
    }

    /// The third cut: the replacement lands immediately before an authority
    /// revalidation on a lease that is already carrying a live database.
    ///
    /// Every way that lease can still be spent is refused — the proof the
    /// bootstrap -> promoted handover and the crash-takeover compare-and-swap
    /// consume, the standalone while-held check the promoted runtime's
    /// boundaries use, and the applier slot that authorizes the next database.
    #[test]
    fn a_leased_projection_whose_lease_path_was_replaced_refuses_at_every_authority_boundary() {
        let ids = TestIds::new(9_220);
        let dir = TestDir::new("workspace-lock-replacement-held");
        let archive_root = dir.path().join("objects");
        let store = ObjectStore::open(&archive_root, ids.workspace).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&dir.path().join("runtime")).unwrap();
        let engine = ids.engine();
        let lease_path = workspace_lease_path(&archive_root, ids.workspace);

        let lease = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        let projection = open_leased_projection(
            lease,
            &dir.path().join("bootstrap.sqlite"),
            &runtime,
            ids,
            &engine,
            &store,
        );
        projection
            .workspace_proof()
            .authorize_archive(&store, ids.workspace)
            .unwrap();
        projection.revalidate_workspace_lease_identity().unwrap();

        replace_workspace_lock_file(&lease_path);

        let error = projection
            .revalidate_workspace_lease_identity()
            .expect_err("the while-held boundary check must see the replacement");
        assert!(
            matches!(error, ProjectionError::LeaseIdentityReplaced(_)),
            "unexpected while-held error: {error}"
        );
        assert!(matches!(
            projection
                .workspace_proof()
                .authorize_archive(&store, ids.workspace),
            Err(ProjectionError::LeaseIdentityReplaced(_))
        ));

        // The bootstrap -> promoted handoff cannot proceed either: the applier
        // slot refuses before a second database is opened, and the refusal hands
        // the exact lease back rather than releasing it.
        let lease = projection.close_retaining_lease();
        let (returned, error) =
            LeasedWorkspaceProjection::open_under::<(), ProjectionError>(lease, |slot| {
                SqliteFrontier::open_or_rebuild_with_applier_slot(
                    &dir.path().join("promoted.sqlite"),
                    &runtime,
                    ids.claim(),
                    RebuildSource::new(&engine, &store).unwrap(),
                    slot,
                )
                .map(|opened| (opened, ()))
            })
            .err()
            .expect("a replaced lease path must not authorize a new database");
        assert!(
            matches!(error, ProjectionError::LeaseIdentityReplaced(_)),
            "unexpected applier-slot error: {error}"
        );

        // And again: a newcomer owns the replacement, the old holder owns
        // nothing, and the two are never both authority.
        let newcomer = WorkspaceRuntimeLease::acquire(&store, ids.workspace).unwrap();
        newcomer
            .proof()
            .authorize_archive(&store, ids.workspace)
            .unwrap();
        assert!(returned
            .proof()
            .authorize_archive(&store, ids.workspace)
            .is_err());
    }

    /// A pipe-coordinated child that reports, on demand, whether it can take
    /// the archive-rooted workspace lease right now. Every step is caused by a
    /// request/response exchange, so the test never sleeps or polls.
    struct WorkspaceLeaseProbe {
        child: Child,
        answers: BufReader<std::process::ChildStdout>,
        requests: std::process::ChildStdin,
    }

    impl WorkspaceLeaseProbe {
        fn spawn(root: &Path, seed: u128, xdg: &Path, home: &Path) -> Self {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .arg("--exact")
                .arg("oplog::sqlite::tests::sqlite_subprocess_helper")
                .arg("--nocapture")
                .env("TINE_SQLITE_HELPER_MODE", "workspace-lease-probe")
                .env("TINE_SQLITE_HELPER_ROOT", root)
                .env("TINE_SQLITE_HELPER_SEED", seed.to_string())
                .env("XDG_DATA_HOME", xdg)
                .env("HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
            let mut child = command.spawn().unwrap();
            let answers = BufReader::new(child.stdout.take().unwrap());
            let requests = child.stdin.take().unwrap();
            Self {
                child,
                answers,
                requests,
            }
        }

        fn probe(&mut self) -> String {
            writeln!(self.requests, "probe").unwrap();
            self.requests.flush().unwrap();
            loop {
                let mut line = String::new();
                assert!(
                    self.answers.read_line(&mut line).unwrap() != 0,
                    "workspace lease probe closed its output before answering"
                );
                // The child is a libtest binary, so its own harness lines share
                // this pipe; only the marked answer is protocol.
                if let Some((_, answer)) = line.rsplit_once(WORKSPACE_LEASE_PROBE_MARKER) {
                    return answer.trim().to_string();
                }
            }
        }

        fn finish(mut self) {
            drop(self.requests);
            assert!(self.child.wait().unwrap().success());
        }
    }

    const WORKSPACE_LEASE_PROBE_MARKER: &str = "workspace-lease-probe:";

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

    /// The exact on-disk substitutions the archive-rooted workspace lock must
    /// refuse. One table serves both the compatibility wrapper and the hoisted
    /// [`WorkspaceRuntimeLease`], so the two cannot drift apart.
    #[cfg(unix)]
    const WORKSPACE_LEASE_CAPABILITY_SUBSTITUTIONS: [&str; 5] = [
        "lease-object-store-namespace-symlink",
        "lease-sqlite-namespace-symlink",
        "lease-workspace-symlink",
        "lease-file-symlink",
        "lease-group-writable-namespace",
    ];

    #[cfg(unix)]
    fn substitute_workspace_lease_capability(case: &str, store: &Path, workspace: WorkspaceId) {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        match case {
            "lease-object-store-namespace-symlink" => {
                fs::create_dir(store.join("redirect")).unwrap();
                symlink(
                    store.join("redirect"),
                    store.join(OBJECT_STORE_LEASE_NAMESPACE),
                )
                .unwrap();
            }
            "lease-sqlite-namespace-symlink" => {
                fs::create_dir(store.join(OBJECT_STORE_LEASE_NAMESPACE)).unwrap();
                fs::create_dir(store.join("redirect")).unwrap();
                symlink(
                    store.join("redirect"),
                    store
                        .join(OBJECT_STORE_LEASE_NAMESPACE)
                        .join(SQLITE_WORKSPACE_LEASE_NAMESPACE),
                )
                .unwrap();
            }
            "lease-workspace-symlink" => {
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
            }
            "lease-file-symlink" => {
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
            }
            "lease-group-writable-namespace" => {
                let namespace = store.join(OBJECT_STORE_LEASE_NAMESPACE);
                fs::create_dir(&namespace).unwrap();
                fs::set_permissions(&namespace, fs::Permissions::from_mode(0o770)).unwrap();
            }
            other => panic!("unknown workspace lease capability substitution: {other}"),
        }
    }

    #[cfg(unix)]
    fn assert_workspace_lease_capability_substitutions_fail_closed(
        seed_base: u128,
        mut assert_rejected: impl FnMut(&Path, &ObjectStore, TestIds),
    ) {
        for case in WORKSPACE_LEASE_CAPABILITY_SUBSTITUTIONS {
            let ids = TestIds::new(seed_base + case.len() as u128 * 100);
            let dir = TestDir::new(case);
            let store_path = dir.path().join("objects");
            let store = ObjectStore::open(&store_path, ids.workspace).unwrap();
            substitute_workspace_lease_capability(case, &store_path, ids.workspace);
            assert_rejected(dir.path(), &store, ids);
        }
    }

    #[cfg(unix)]
    #[test]
    fn object_store_lease_rejects_symlinked_namespaces_workspace_and_file() {
        assert_workspace_lease_capability_substitutions_fail_closed(7_700, |dir, store, ids| {
            let runtime = ApplicationRuntimeRoot::open_for_test(&dir.join("runtime")).unwrap();
            let engine = ids.engine();
            assert!(matches!(
                SqliteFrontier::open_or_rebuild(
                    &dir.join("frontier.sqlite"),
                    &runtime,
                    ids.claim(),
                    RebuildSource::new(&engine, store).unwrap(),
                ),
                Err(ProjectionError::UnsafePath(_))
            ));
        });
    }

    /// The hoisted lease keeps the same no-follow, ownership, and mode
    /// validators the combined applier lease had, proven against the same
    /// substitution table rather than a restated copy of it.
    #[cfg(unix)]
    #[test]
    fn workspace_runtime_lease_rejects_symlinked_namespaces_workspace_and_file() {
        assert_workspace_lease_capability_substitutions_fail_closed(8_700, |_dir, store, ids| {
            assert!(matches!(
                WorkspaceRuntimeLease::acquire(store, ids.workspace),
                Err(ProjectionError::UnsafePath(_))
            ));
        });
    }

    #[cfg(windows)]
    #[test]
    fn windows_entry_file_identity_classifies_reparse_lease_as_replaced() {
        use std::os::windows::fs::symlink_file;

        let dir = TestDir::new("windows-entry-file-identity-reparse");
        let target = dir.path().join("target");
        let lease = dir.path().join("lease");
        fs::write(&target, b"not authoritative").unwrap();
        symlink_file(&target, &lease).unwrap();
        let directory = CapDir::open_ambient_dir(dir.path(), ambient_authority()).unwrap();

        assert!(matches!(
            entry_file_identity(&directory, "lease", &lease),
            Err(LeasePathResolutionError::Replaced(_))
        ));
        assert_eq!(fs::read(&target).unwrap(), b"not authoritative");
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
                name: crate::oplog::LogicalPageName::parse("SQLite").unwrap(),
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
        store
            .publish_bootstrap_prepared_for_test(&prepared)
            .unwrap();
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
                    name: crate::oplog::LogicalPageName::parse("SQLite").unwrap(),
                    path: ManagedPath::parse("shared/SQLite.md").unwrap(),
                    kind: ManagedTextKind::Page,
                }])
                .unwrap(),
            )
            .unwrap();
        store.publish_bootstrap_prepared_for_test(&create).unwrap();
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
        store.publish_bootstrap_prepared_for_test(&change).unwrap();
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
                assert!(
                    fs::metadata(SqliteFileSet::new(&path).wal_path())
                        .unwrap()
                        .len()
                        >= 32
                );
            }
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
            let files = SqliteFileSet::new(&path);
            let target = if mutation.starts_with("wal") {
                files.wal_path()
            } else {
                files.shm_path()
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
            let files = SqliteFileSet::new(&path);
            fs::write(files.wal_path(), b"partial wal").unwrap();
            fs::write(files.shm_path(), b"partial shm").unwrap();
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
        publish_and_stage_archive(&mut accepted_engine, &store, &child);
        fs::write(&path, b"corrupt before rebuild").unwrap();
        let mut helper = spawn_test_helper(
            "recover",
            dir.path(),
            seed,
            &[("TINE_SQLITE_REBUILD_ABORT_AFTER", "1")],
        );
        wait_for_file(&dir.path().join("helper-ready"));
        assert!(!helper.wait().unwrap().success());
        if path.exists() {
            assert_eq!(
                fs::read(&path).unwrap(),
                b"corrupt before rebuild",
                "an aborted candidate must not replace the prior projection path"
            );
        }
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
        assert_eq!(reopened.rebuild.reference_coverage_full_scans, 1);
        assert_eq!(
            reopened.rebuild.reference_coverage_inductive_checks,
            reopened.rebuild.accepted_events_applied
        );
        let retry_semantic_digest = reopened.database.semantic_projection_digest().unwrap();
        let retry_row_digest = reopened
            .database
            .materialized_row_digest_for_harness()
            .unwrap();
        drop(reopened);
        let clean = open_test_projection(
            &dir.path().join("clean-rebuild.sqlite"),
            ids.claim(),
            RebuildSource::new(&accepted_engine, &store).unwrap(),
        )
        .unwrap();
        assert_eq!(
            clean.database.semantic_projection_digest().unwrap(),
            retry_semantic_digest
        );
        assert_eq!(
            clean
                .database
                .materialized_row_digest_for_harness()
                .unwrap(),
            retry_row_digest
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
        SqliteFileSet::new(path)
            .remove()
            .unwrap_or_else(|error| panic!("cannot remove test projection: {error}"));
    }
}
