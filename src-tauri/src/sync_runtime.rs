//! Explicit Tauri-facing sparse-v2 runtime composition.
//!
//! A durable caller-owned binding in private app data is the opt-in marker.
//! Ordinary graph loading never creates it. Once present, startup discovers
//! sparse state and never falls back to a legacy `Graph` writer.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tine_core::model::GraphMeta;
use tine_core::oplog::{
    DeviceId, DocumentId, LineageDigest, ProjectionEndpointId, SessionId, WorkspaceId,
};
use tine_core::sync_runtime::{
    inspect_shared_enrollment_for_cold_discovery, SyncAmbiguousEvidence,
    SyncLocalActivationIdentities, SyncLocalActivationPhase, SyncLocalActivationProgress,
    SyncLocalActivationRequest, SyncLocalActivationResult, SyncLocalActivationStage,
    SyncLocalActivationStatus, SyncNonActiveStage, SyncRuntimeComponent, SyncRuntimeHandle,
    SyncRuntimeLifecycle, SyncRuntimeOpenProgress, SyncRuntimeOpenRequest, SyncRuntimeOpenResult,
    SyncRuntimeOpenStatus, SyncRuntimeRecovery, SyncRuntimeStatusSnapshot, SyncRuntimeTick,
    SyncSharedEnrollmentDescriptor, SyncSharedPhase, SyncSharedRole, SyncShutdownOutcome,
    SyncStorageProfile,
};
use uuid::Uuid;

const BINDING_SCHEMA_VERSION: u32 = 2;
const SPARSE_BINDING_DIR: &str = "sparse-v2";
const SPARSE_BINDING_FILE: &str = "binding.json";
const SPARSE_RECOVERY_DIR: &str = "sparse-v2-recovery";
static BINDING_WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SparseV2ActivationRecord {
    schema_version: u32,
    graph_root: String,
    graph_meta: GraphMeta,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    endpoint_id: ProjectionEndpointId,
    device_id: DeviceId,
    preparation_id: Uuid,
    activation_session_id: SessionId,
}

impl SparseV2ActivationRecord {
    fn new(graph_root: &Path, graph_meta: GraphMeta, device_id: DeviceId) -> Self {
        let lineage_seed = Uuid::new_v4();
        Self {
            schema_version: BINDING_SCHEMA_VERSION,
            graph_root: graph_root.display().to_string(),
            graph_meta,
            workspace_id: WorkspaceId::new(),
            lineage_digest: LineageDigest::of(lineage_seed.as_bytes()),
            catalog_document_id: DocumentId::new(),
            endpoint_id: ProjectionEndpointId::new(),
            device_id,
            preparation_id: Uuid::new_v4(),
            activation_session_id: SessionId::new(),
        }
    }

    fn from_shared(
        graph_root: &Path,
        graph_meta: GraphMeta,
        device_id: DeviceId,
        descriptor: &SyncSharedEnrollmentDescriptor,
    ) -> Self {
        Self {
            schema_version: BINDING_SCHEMA_VERSION,
            graph_root: graph_root.display().to_string(),
            graph_meta,
            workspace_id: descriptor.workspace_id,
            lineage_digest: descriptor.lineage_digest,
            catalog_document_id: descriptor.catalog_document_id,
            endpoint_id: ProjectionEndpointId::new(),
            device_id,
            preparation_id: Uuid::new_v4(),
            activation_session_id: SessionId::new(),
        }
    }

    fn validate_for(&self, graph_root: &Path) -> Result<(), String> {
        if self.schema_version != BINDING_SCHEMA_VERSION {
            return Err("Tine-managed storage has an unsupported local setup version.".into());
        }
        if self.graph_root != graph_root.display().to_string()
            || self.graph_meta.root != self.graph_root
        {
            return Err("Tine-managed storage data belongs to a different graph.".into());
        }
        Ok(())
    }

    fn private_root(&self, app: &tauri::AppHandle) -> Result<PathBuf, String> {
        sparse_private_root(app, Path::new(&self.graph_root))
    }

    fn open_request(&self, app: &tauri::AppHandle) -> Result<SyncRuntimeOpenRequest, String> {
        let private = self.private_root(app)?;
        Ok(SyncRuntimeOpenRequest {
            profile: SyncStorageProfile::ExperimentalLocal,
            graph_root: PathBuf::from(&self.graph_root),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
            migration_backup_root: private.join("migration-backup"),
            provider_root: PathBuf::from(&self.graph_root).join(".tine-sync/v2/shared"),
            provider_journal_root: private.join("provider/device/journal"),
        })
    }

    fn activation_request(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<SyncLocalActivationRequest, String> {
        let private = self.private_root(app)?;
        Ok(SyncLocalActivationRequest {
            graph_root: PathBuf::from(&self.graph_root),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
            migration_backup_root: private.join("migration-backup"),
            capture_root: private.join("capture"),
            preparation_root: private.join("preparation"),
            provider_root: PathBuf::from(&self.graph_root).join(".tine-sync/v2/shared"),
            provider_journal_root: private.join("provider/device/journal"),
            identities: SyncLocalActivationIdentities {
                workspace_id: self.workspace_id,
                lineage_digest: self.lineage_digest,
                catalog_document_id: self.catalog_document_id,
                endpoint_id: self.endpoint_id,
                device_id: self.device_id,
                preparation_id: self.preparation_id,
                session_id: self.activation_session_id,
            },
        })
    }
}

fn sparse_private_root(app: &tauri::AppHandle, graph_root: &Path) -> Result<PathBuf, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("couldn't resolve private app-data directory: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(b"tine/sparse-v2/app-binding/v1\0");
    digest.update(graph_root.as_os_str().as_encoded_bytes());
    let key = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(app_data.join(SPARSE_BINDING_DIR).join(key))
}

fn binding_path(app: &tauri::AppHandle, graph_root: &Path) -> Result<PathBuf, String> {
    Ok(sparse_private_root(app, graph_root)?.join(SPARSE_BINDING_FILE))
}

fn sparse_recovery_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|root| root.join(SPARSE_RECOVERY_DIR))
        .map_err(|error| format!("couldn't resolve private app-data directory: {error}"))
}

fn read_binding_at(
    path: &Path,
    graph_root: &Path,
) -> Result<Option<SparseV2ActivationRecord>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("Couldn't read Tine-managed storage data: {error}"));
        }
    };
    let record: SparseV2ActivationRecord = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Tine-managed storage data is corrupted: {error}"))?;
    record.validate_for(graph_root)?;
    Ok(Some(record))
}

fn persist_binding_at(path: &Path, record: &SparseV2ActivationRecord) -> Result<(), String> {
    let encoded = serde_json::to_string_pretty(record)
        .map(|mut value| {
            value.push('\n');
            value
        })
        .map_err(|error| error.to_string())?;
    tine_core::model::atomic_update(path, &BINDING_WRITE, |existing| {
        if existing.trim().is_empty() || existing.trim() == "{}" {
            return Ok(encoded.clone());
        }
        let found: SparseV2ActivationRecord = serde_json::from_str(existing)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let found_value = serde_json::to_value(&found)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let expected_value = serde_json::to_value(record)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if found_value != expected_value {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Tine-managed storage is already set up for this graph.",
            ));
        }
        Ok(encoded.clone())
    })
    .map_err(|error| format!("Couldn't save Tine-managed storage setup: {error}"))
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum SparseV2Availability {
    LegacyDefault,
    Joinable {
        descriptor_digest: String,
    },
    Active,
    Retryable {
        stage: String,
        detail: String,
    },
    Blocked {
        reason_code: String,
    },
    Refused {
        reason_code: String,
        detail: Option<String>,
    },
}

impl SparseV2Availability {
    fn from_open(status: SyncRuntimeOpenStatus) -> Self {
        match status {
            SyncRuntimeOpenStatus::LegacyDefault => Self::LegacyDefault,
            SyncRuntimeOpenStatus::Active => Self::Active,
            SyncRuntimeOpenStatus::Absent => Self::Retryable {
                stage: "absent".into(),
                detail: "Tine-managed storage setup has not completed.".into(),
            },
            SyncRuntimeOpenStatus::ExistingNonActive(stage) => Self::Retryable {
                stage: non_active_stage(stage).into(),
                detail: "Tine-managed storage setup can be resumed.".into(),
            },
            SyncRuntimeOpenStatus::Blocked { reason_code } => Self::Blocked { reason_code },
            SyncRuntimeOpenStatus::UnsupportedOrIncompatible(component) => Self::Refused {
                reason_code: format!("unsupported_{}", component_name(component)),
                detail: None,
            },
            SyncRuntimeOpenStatus::CorruptOrUnreadable(component) => Self::Refused {
                reason_code: format!("corrupt_{}", component_name(component)),
                detail: None,
            },
            SyncRuntimeOpenStatus::AmbiguousOrForeignResidue(evidence) => Self::Refused {
                reason_code: format!("ambiguous_{}", ambiguous_name(evidence)),
                detail: None,
            },
            SyncRuntimeOpenStatus::OpenRefused { detail } => Self::Retryable {
                stage: "local_active".into(),
                detail,
            },
        }
    }

    fn from_activation(status: SyncLocalActivationStatus) -> Self {
        match status {
            SyncLocalActivationStatus::Active => Self::Active,
            SyncLocalActivationStatus::Retryable {
                durable_stage,
                detail,
            } => Self::Retryable {
                stage: activation_stage(durable_stage).into(),
                detail,
            },
            SyncLocalActivationStatus::Blocked { reason_code } => Self::Blocked { reason_code },
            SyncLocalActivationStatus::LegacyV1Refused => Self::Refused {
                reason_code: "legacy_v1_present".into(),
                detail: Some(
                    "Tine-managed storage will not alter incompatible existing storage data."
                        .into(),
                ),
            },
            SyncLocalActivationStatus::UnsupportedOrIncompatible(component) => Self::Refused {
                reason_code: format!("unsupported_{}", component_name(component)),
                detail: None,
            },
            SyncLocalActivationStatus::CorruptOrUnreadable(component) => Self::Refused {
                reason_code: format!("corrupt_{}", component_name(component)),
                detail: None,
            },
            SyncLocalActivationStatus::AmbiguousOrForeignResidue(evidence) => Self::Refused {
                reason_code: format!("ambiguous_{}", ambiguous_name(evidence)),
                detail: None,
            },
        }
    }
}

fn activation_stage(stage: SyncLocalActivationStage) -> &'static str {
    match stage {
        SyncLocalActivationStage::Absent => "absent",
        SyncLocalActivationStage::ShadowImport => "shadow_import",
        SyncLocalActivationStage::VerifiedLocal => "verified_local",
        SyncLocalActivationStage::LocalActive => "local_active",
    }
}

fn non_active_stage(stage: SyncNonActiveStage) -> &'static str {
    match stage {
        SyncNonActiveStage::ShadowImport => "shadow_import",
        SyncNonActiveStage::VerifiedLocal => "verified_local",
    }
}

fn component_name(component: SyncRuntimeComponent) -> &'static str {
    match component {
        SyncRuntimeComponent::Enrollment => "enrollment",
        SyncRuntimeComponent::Archive => "archive",
    }
}

fn ambiguous_name(evidence: SyncAmbiguousEvidence) -> &'static str {
    match evidence {
        SyncAmbiguousEvidence::EnrollmentResidue => "enrollment_residue",
        SyncAmbiguousEvidence::EnrollmentNamespace => "enrollment_namespace",
        SyncAmbiguousEvidence::EnrollmentGraphBinding => "enrollment_graph_binding",
        SyncAmbiguousEvidence::ArchiveResidue => "archive_residue",
        SyncAmbiguousEvidence::ArchiveNamespace => "archive_namespace",
        SyncAmbiguousEvidence::ArchiveBinding => "archive_binding",
        SyncAmbiguousEvidence::ActiveArchiveMismatch => "active_archive_mismatch",
    }
}

pub(crate) struct SparseV2Binding {
    availability: SparseV2Availability,
    handle: Option<SyncRuntimeHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SparseV2BindingAction {
    ReturnRetained,
    ReopenActive,
    ActivateOrResume,
}

fn action_for_runtime_lifecycle(lifecycle: &SyncRuntimeLifecycle) -> SparseV2BindingAction {
    match lifecycle {
        SyncRuntimeLifecycle::StoppedSafe | SyncRuntimeLifecycle::StoppedCrashed => {
            SparseV2BindingAction::ReopenActive
        }
        SyncRuntimeLifecycle::Active | SyncRuntimeLifecycle::Terminal => {
            SparseV2BindingAction::ReturnRetained
        }
    }
}

impl SparseV2Binding {
    fn from_open(result: SyncRuntimeOpenResult) -> Self {
        Self {
            availability: SparseV2Availability::from_open(result.status),
            handle: result.handle,
        }
    }

    fn from_activation(result: SyncLocalActivationResult) -> Self {
        Self {
            availability: SparseV2Availability::from_activation(result.status),
            handle: result.handle,
        }
    }

    pub(crate) fn handle(&self) -> Option<&SyncRuntimeHandle> {
        self.handle.as_ref()
    }

    /// A managed binding with no live actor, for tests that only need the slot
    /// to *be* sparse -- e.g. proving that a read command is routed to the
    /// read-only view instead of being refused for lacking legacy authority.
    #[cfg(test)]
    pub(crate) fn without_actor_for_test() -> Self {
        Self {
            availability: SparseV2Availability::Active,
            handle: None,
        }
    }

    pub(crate) fn availability(&self) -> &SparseV2Availability {
        &self.availability
    }

    fn action(&self) -> SparseV2BindingAction {
        match &self.handle {
            Some(handle) => handle
                .status()
                .as_ref()
                .map(|snapshot| action_for_runtime_lifecycle(&snapshot.lifecycle))
                .unwrap_or(SparseV2BindingAction::ReopenActive),
            None if matches!(
                &self.availability,
                SparseV2Availability::Retryable { stage, .. }
                    if matches!(
                        stage.as_str(),
                        "local_active" | "share_prepared" | "joining" | "shared_active"
                    )
            ) =>
            {
                SparseV2BindingAction::ReopenActive
            }
            None => SparseV2BindingAction::ActivateOrResume,
        }
    }
}

fn retryable_binding(stage: &str, detail: String) -> SparseV2Binding {
    SparseV2Binding {
        availability: SparseV2Availability::Retryable {
            stage: stage.into(),
            detail,
        },
        handle: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2WatcherStatusDto {
    latest_enqueue: u64,
    acknowledged: u64,
    drain_in_flight: bool,
    pending: bool,
    pending_requires_full_scan: bool,
    deferred: bool,
    quiescing: bool,
    sequence_exhausted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2TickDto {
    state: String,
    detail: Option<String>,
    epoch: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2RuntimeStatusDto {
    lifecycle: String,
    recovery: Option<String>,
    watcher: SparseV2WatcherStatusDto,
    last_tick: Option<SparseV2TickDto>,
    detail: Option<String>,
    shared_role: Option<String>,
    shared_phase: Option<String>,
    provider_pending: usize,
    managed_local_pending: usize,
    managed_local_checkpointed_sequence: u64,
    managed_local_next_sequence: u64,
    managed_local_stage: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2StatusDto {
    #[serde(flatten)]
    availability: SparseV2Availability,
    runtime: Option<SparseV2RuntimeStatusDto>,
    can_activate: bool,
    can_retry: bool,
    can_cancel: bool,
    cancel_reason: Option<String>,
    binding_generation: u64,
}

impl SparseV2StatusDto {
    pub(crate) fn legacy(binding_generation: u64) -> Self {
        Self {
            availability: SparseV2Availability::LegacyDefault,
            runtime: None,
            can_activate: true,
            can_retry: false,
            can_cancel: false,
            cancel_reason: None,
            binding_generation,
        }
    }

    pub(crate) fn joinable(
        binding_generation: u64,
        descriptor: &SyncSharedEnrollmentDescriptor,
    ) -> Self {
        Self {
            availability: SparseV2Availability::Joinable {
                descriptor_digest: descriptor.descriptor_digest.clone(),
            },
            runtime: None,
            can_activate: false,
            can_retry: false,
            can_cancel: false,
            cancel_reason: Some(
                "This graph is already synced with another device, so returning to Direct files is unavailable."
                    .into(),
            ),
            binding_generation,
        }
    }

    pub(crate) fn from_binding(binding: &SparseV2Binding, binding_generation: u64) -> Self {
        let retained_status = binding.handle().map(SyncRuntimeHandle::status);
        let runtime = retained_status
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .cloned()
            .map(runtime_status);
        let availability = match runtime.as_ref().map(|status| status.lifecycle.as_str()) {
            Some("stopped_safe") => SparseV2Availability::Retryable {
                stage: "local_active".into(),
                detail: "Tine-managed storage stopped safely and needs to be reopened.".into(),
            },
            Some("stopped_crashed") => SparseV2Availability::Retryable {
                stage: "local_active".into(),
                detail: "Tine-managed storage needs to be reopened after it stopped unexpectedly."
                    .into(),
            },
            None if retained_status.as_ref().is_some_and(Result::is_err) => {
                SparseV2Availability::Retryable {
                    stage: "local_active".into(),
                    detail: "Tine-managed storage needs to be reopened.".into(),
                }
            }
            _ => binding.availability().clone(),
        };
        let can_retry = matches!(availability, SparseV2Availability::Retryable { .. });
        Self {
            availability,
            runtime,
            can_activate: false,
            can_retry,
            can_cancel: false,
            cancel_reason: None,
            binding_generation,
        }
    }
}

pub(crate) fn runtime_status(snapshot: SyncRuntimeStatusSnapshot) -> SparseV2RuntimeStatusDto {
    // Core's bounded provider diagnostic includes an uninitialized recovery-
    // coverage sentinel even for a purely local runtime. At the app boundary it
    // is provider work only once a shared role/phase exists.
    let provider_pending = if snapshot.shared_role.is_some() || snapshot.shared_phase.is_some() {
        snapshot.provider_pending
    } else {
        0
    };
    SparseV2RuntimeStatusDto {
        lifecycle: match snapshot.lifecycle {
            SyncRuntimeLifecycle::Active => "active",
            SyncRuntimeLifecycle::Terminal => "terminal",
            SyncRuntimeLifecycle::StoppedSafe => "stopped_safe",
            SyncRuntimeLifecycle::StoppedCrashed => "stopped_crashed",
        }
        .into(),
        recovery: snapshot.recovery.map(|recovery| {
            match recovery {
                SyncRuntimeRecovery::FirstPromotion => "first_promotion",
                SyncRuntimeRecovery::ResumedOwnUnsafe => "resumed_own_unsafe",
                SyncRuntimeRecovery::AdoptedSafeHandoff => "adopted_safe_handoff",
                SyncRuntimeRecovery::TookOverCrashedUnsafe => "took_over_crashed_unsafe",
            }
            .into()
        }),
        watcher: SparseV2WatcherStatusDto {
            latest_enqueue: snapshot.watcher.latest_enqueue,
            acknowledged: snapshot.watcher.acknowledged,
            drain_in_flight: snapshot.watcher.drain_in_flight,
            pending: snapshot.watcher.pending,
            pending_requires_full_scan: snapshot.watcher.pending_requires_full_scan,
            deferred: snapshot.watcher.deferred,
            quiescing: snapshot.watcher.quiescing,
            sequence_exhausted: snapshot.watcher.sequence_exhausted,
        },
        last_tick: snapshot.last_tick.map(tick_dto),
        detail: snapshot.detail,
        shared_role: snapshot.shared_role.map(|role| match role {
            SyncSharedRole::Initiator => "initiator".into(),
            SyncSharedRole::Joiner => "joiner".into(),
        }),
        shared_phase: snapshot.shared_phase.map(|phase| match phase {
            SyncSharedPhase::SharePrepared => "share_prepared".into(),
            SyncSharedPhase::Joining => "joining".into(),
            SyncSharedPhase::Active => "active".into(),
        }),
        provider_pending,
        managed_local_pending: snapshot.managed_local_pending,
        managed_local_checkpointed_sequence: snapshot.managed_local_checkpointed_sequence,
        managed_local_next_sequence: snapshot.managed_local_next_sequence,
        managed_local_stage: snapshot.managed_local_stage,
    }
}

pub(crate) fn tick_dto(tick: SyncRuntimeTick) -> SparseV2TickDto {
    match tick {
        SyncRuntimeTick::Idle => tick_value("idle", None, None),
        SyncRuntimeTick::LocalMutation(outcome) => {
            tick_value("local_mutation", Some(format!("{outcome:?}")), None)
        }
        SyncRuntimeTick::RecoveryBlocked(detail) => {
            tick_value("recovery_blocked", Some(detail), None)
        }
        SyncRuntimeTick::Recovering => tick_value("recovering", None, None),
        SyncRuntimeTick::RetryFull => tick_value("retry_full", None, None),
        SyncRuntimeTick::Blocked(detail) => tick_value("blocked", Some(detail), None),
        SyncRuntimeTick::Failed(detail) => tick_value("failed", Some(detail), None),
        SyncRuntimeTick::AdmittedNoop { epoch } => tick_value("admitted_noop", None, Some(epoch)),
        SyncRuntimeTick::AdmittedComplete { epoch } => {
            tick_value("admitted_complete", None, Some(epoch))
        }
        SyncRuntimeTick::Terminal(detail) => tick_value("terminal", Some(detail), None),
    }
}

fn tick_value(state: &str, detail: Option<String>, epoch: Option<u64>) -> SparseV2TickDto {
    SparseV2TickDto {
        state: state.into(),
        detail,
        epoch,
    }
}

pub(crate) fn shutdown_status(outcome: SyncShutdownOutcome) -> SparseV2RuntimeStatusDto {
    match outcome {
        SyncShutdownOutcome::Safe(snapshot) | SyncShutdownOutcome::Terminal(snapshot) => {
            runtime_status(snapshot)
        }
    }
}

fn provider_namespace_has_evidence(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Couldn't inspect sync data: {error}")),
    }
}

fn binding_names_shared_state(binding: &SparseV2Binding) -> bool {
    matches!(
        binding.availability(),
        SparseV2Availability::Retryable { stage, .. }
            if matches!(
                stage.as_str(),
                "share_prepared" | "joining" | "shared_active"
            )
    )
}

fn cancel_eligibility(binding: &SparseV2Binding, provider_namespace: &Path) -> Result<(), String> {
    let provider_evidence = provider_namespace_has_evidence(provider_namespace)?;
    if binding_names_shared_state(binding) {
        return Err(
            "This graph is synced with another device, so returning to Direct files is unavailable."
                .into(),
        );
    }
    if let Some(handle) = binding.handle() {
        let status = handle.status().map_err(|error| {
            format!("Couldn't verify that returning to Direct files is safe: {error}")
        })?;
        let names_shared_runtime = status.shared_role.is_some() || status.shared_phase.is_some();
        // A local-only core snapshot currently counts its absent provider
        // recovery-coverage sentinel as one pending item. It cannot represent
        // provider work when both shared runtime identity and the complete
        // graph-local provider namespace are absent.
        if names_shared_runtime || (status.provider_pending != 0 && provider_evidence) {
            return Err(
                "This graph is synced with another device, so returning to Direct files is unavailable."
                    .into(),
            );
        }
    }
    if provider_evidence {
        return Err(
            "This graph is synced with another device, so returning to Direct files is unavailable."
                .into(),
        );
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct SyncRuntimeFacade;

impl SyncRuntimeFacade {
    pub(crate) fn binding_record(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<Option<SparseV2ActivationRecord>, String> {
        let private = sparse_private_root(app, graph_root)?;
        let record = read_binding_at(&private.join(SPARSE_BINDING_FILE), graph_root)?;
        if record.is_none() && std::fs::symlink_metadata(&private).is_ok() {
            return Err(
                "Tine-managed storage data is incomplete, so this graph could not be opened safely."
                    .into(),
            );
        }
        Ok(record)
    }

    pub(crate) fn prepare_binding_record(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
        graph_meta: GraphMeta,
    ) -> Result<SparseV2ActivationRecord, String> {
        match self.binding_record(app, graph_root)? {
            Some(record) => Ok(record),
            None => Ok(SparseV2ActivationRecord::new(
                graph_root,
                graph_meta,
                DeviceId::from_uuid(crate::settings::managed_sync_device_id(app)?),
            )),
        }
    }

    pub(crate) fn persist_binding_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<(), String> {
        let root = Path::new(&record.graph_root);
        persist_binding_at(&binding_path(app, root)?, record)
    }

    pub(crate) fn graph_meta(record: &SparseV2ActivationRecord) -> GraphMeta {
        record.graph_meta.clone()
    }

    pub(crate) fn open_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<SparseV2Binding, String> {
        crate::debug::diag("managed storage open: begin authenticated existing-state recovery");
        let opened = SyncRuntimeHandle::open_with_progress(record.open_request(app)?, |update| {
            match update {
                SyncRuntimeOpenProgress::Phase { phase, elapsed } => crate::debug::diag(format!(
                    "managed storage open: phase {phase:?} at {} ms",
                    elapsed.as_millis()
                )),
                SyncRuntimeOpenProgress::Waiting { phase, elapsed } => crate::debug::diag(format!(
                    "managed storage open: still waiting in {phase:?} at {} ms",
                    elapsed.as_millis()
                )),
                SyncRuntimeOpenProgress::RecoveryDiagnostics { diagnostics } => {
                    crate::debug::diag(format!(
                            "managed storage open: promoted recovery recovery={} retention={} retained_runs={} resume_candidate={} detached_bootstrap_reconstruction={} full_bootstrap_replay={} manifests={} manifest_enumeration_ms={} resume_selection_ms={} bootstrap_reconstruction_ms={} engine_open_ms={} sqlite_open_ms={} tail_construction_ms={} total_ms={}",
                            diagnostics.recovery,
                            diagnostics.retention_plan,
                            diagnostics.retained_run_count,
                            diagnostics.resume_candidate,
                            diagnostics.detached_bootstrap_reconstruction,
                            diagnostics.full_bootstrap_replay,
                            diagnostics.manifest_count,
                            diagnostics.manifest_enumeration.as_millis(),
                            diagnostics.resume_selection.as_millis(),
                            diagnostics.bootstrap_reconstruction.map(|elapsed| elapsed.as_millis()).map_or_else(|| "not_attempted".to_owned(), |elapsed| elapsed.to_string()),
                            diagnostics.engine_open.as_millis(),
                            diagnostics.sqlite_open.as_millis(),
                            diagnostics.tail_construction.as_millis(),
                            diagnostics.total.as_millis(),
                        ));
                    crate::debug::diag(format!(
                            "managed storage open: projection recovery={} reason={:?} sidecar_shape_ms={} checkpoint_auth_ms={} read_only_open_ms={} schema_claim_ms={} structural_ms={} materialization_stamp_ms={} forensics_ms={} rebuild_ms={} applied_batches={} bulk_pages_materialized={} ancestry_full_scans={}",
                            diagnostics.projection_recovery,
                            diagnostics.projection_reason,
                            diagnostics.projection_sidecar_shape.as_millis(),
                            diagnostics.projection_checkpoint_authentication.as_millis(),
                            diagnostics.projection_read_only_open.as_millis(),
                            diagnostics.projection_schema_and_claim.as_millis(),
                            diagnostics.projection_structural_validation.as_millis(),
                            diagnostics.projection_materialization_stamp.as_millis(),
                            diagnostics.projection_forensics_preservation.as_millis(),
                            diagnostics.projection_rebuild.as_millis(),
                            diagnostics.projection_applied_batches,
                            diagnostics.projection_bulk_pages_materialized,
                            diagnostics.projection_ancestry_full_scans,
                        ));
                    crate::debug::diag(format!(
                            "managed storage open: engine stages prepare_replay_ms={} predecessor_restore_ms={} bootstrap_part_replay_ms={} archived_tail_replay_ms={} finish_replay_ms={} bootstrap_parts_replayed={} archived_manifests_offered={} archived_manifests_replayed={} resume_adopted={} resume_refused={} replay_base_generation={} live_history_generation={} replayed_generations={}",
                            diagnostics.prepare_replay.as_millis(),
                            diagnostics.predecessor_restore.as_millis(),
                            diagnostics.bootstrap_part_replay.as_millis(),
                            diagnostics.archived_tail_replay.as_millis(),
                            diagnostics.finish_replay.as_millis(),
                            diagnostics.bootstrap_parts_replayed,
                            diagnostics.archived_manifests_offered,
                            diagnostics.archived_manifests_replayed,
                            diagnostics.resume_adopted,
                            diagnostics.resume_refused,
                            diagnostics.replay_base_generation,
                            diagnostics.live_history_generation,
                            diagnostics.replayed_generations,
                        ));
                }
            }
        });
        crate::debug::diag(format!(
            "managed storage open: completed with {:?}",
            opened.status
        ));
        Ok(SparseV2Binding::from_open(opened))
    }

    pub(crate) fn activate_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<SparseV2Binding, String> {
        self.activate_record_with_progress(app, record, |_| {})
    }

    pub(crate) fn activate_record_with_progress(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
        progress: impl FnMut(SyncLocalActivationPhase),
    ) -> Result<SparseV2Binding, String> {
        Ok(SparseV2Binding::from_activation(
            SyncRuntimeHandle::activate_or_resume_local_with_progress(
                record.activation_request(app)?,
                progress,
            ),
        ))
    }

    pub(crate) fn activate_record_with_detailed_progress(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
        progress: impl FnMut(SyncLocalActivationProgress),
    ) -> Result<SparseV2Binding, String> {
        Ok(SparseV2Binding::from_activation(
            SyncRuntimeHandle::activate_or_resume_local_with_detailed_progress(
                record.activation_request(app)?,
                progress,
            ),
        ))
    }

    #[cfg(test)]
    fn open_explicit(&self, request: SyncRuntimeOpenRequest) -> SyncRuntimeOpenResult {
        SyncRuntimeHandle::open(request)
    }
}

const LEGACY_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVATION_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const ACTIVATION_PROGRESS_EVENT: &str = "sparse-v2-activation-progress";
pub(crate) const SPARSE_V2_NOT_ACTIVE: &str =
    "Tine-managed storage is not ready. Retry setup or return to Direct files.";

struct ActivationHeartbeat {
    stop: mpsc::Sender<()>,
    join: Option<JoinHandle<()>>,
}

fn latest_activation_progress_name(
    latest_progress: &Arc<Mutex<Option<SyncLocalActivationProgress>>>,
) -> String {
    latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(SyncLocalActivationProgress::diagnostic_name)
        .unwrap_or_else(|| "core bootstrap setup".into())
}

impl ActivationHeartbeat {
    fn start(
        started: Instant,
        latest_progress: Arc<Mutex<Option<SyncLocalActivationProgress>>>,
    ) -> Self {
        let (stop, stopped) = mpsc::channel();
        let join = std::thread::spawn(move || loop {
            match stopped.recv_timeout(ACTIVATION_HEARTBEAT_INTERVAL) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let progress = latest_activation_progress_name(&latest_progress);
                    crate::debug::diag(format!(
                        "sparse-v2 activation heartbeat after {} ms: progress={progress}",
                        started.elapsed().as_millis()
                    ));
                }
            }
        });
        Self {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for ActivationHeartbeat {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
struct SparseV2ActivationProgressEvent {
    binding_generation: u64,
    progress: SyncLocalActivationProgress,
}

fn activate_record_with_diagnostics(
    facade: &SyncRuntimeFacade,
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    record: &SparseV2ActivationRecord,
) -> Result<SparseV2Binding, String> {
    let started = Instant::now();
    let latest_progress = Arc::new(Mutex::new(None));
    let heartbeat = ActivationHeartbeat::start(started, Arc::clone(&latest_progress));
    let result = facade.activate_record_with_detailed_progress(app, record, |progress| {
        let diagnostic = progress.diagnostic_name();
        let _ = app.emit_to(
            label,
            ACTIVATION_PROGRESS_EVENT,
            SparseV2ActivationProgressEvent {
                binding_generation,
                progress: progress.clone(),
            },
        );
        *latest_progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(progress);
        crate::debug::diag(format!(
            "sparse-v2 activation progress after {} ms: {diagnostic}",
            started.elapsed().as_millis(),
        ));
    });
    drop(heartbeat);
    result
}

pub(crate) fn active_handle(
    slot: &crate::state::GraphSlot,
) -> Result<&tine_core::sync_runtime::SyncRuntimeHandle, String> {
    slot.sparse_runtime()
        .ok_or_else(|| SPARSE_V2_NOT_ACTIVE.to_string())
}

fn sparse_v2_status_for_slot(slot: &crate::state::GraphSlot) -> Result<SparseV2StatusDto, String> {
    Ok(match slot.sparse_binding() {
        Some(binding) => {
            let mut status = SparseV2StatusDto::from_binding(binding, slot.binding_generation);
            match cancel_eligibility(binding, &slot.root_key.join(".tine-sync/v2")) {
                Ok(()) => {
                    status.can_cancel = true;
                    status.cancel_reason = None;
                }
                Err(reason) => {
                    status.can_cancel = false;
                    status.cancel_reason = Some(reason);
                }
            }
            status
        }
        None => {
            match inspect_shared_enrollment_for_cold_discovery(
                &slot.root_key.join(".tine-sync/v2/shared"),
            )? {
                Some(descriptor) => {
                    SparseV2StatusDto::joinable(slot.binding_generation, &descriptor)
                }
                None => SparseV2StatusDto::legacy(slot.binding_generation),
            }
        }
    })
}

#[tauri::command]
pub(crate) async fn sparse_v2_status(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        sparse_v2_status_for_slot(&slot)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Explicitly retire one legacy authority and activate/resume sparse v2.
///
/// The durable opt-in record is published only after the legacy watcher,
/// detached background work, and every in-flight legacy command have released
/// their tracked graph leases. Once the record exists, every result (including
/// retryable/blocked) is published as sparse authority; there is no writer
/// fallback.
#[tauri::command]
pub(crate) async fn activate_sparse_v2(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, String> {
    let label = state.window.label().to_string();
    let binding_generation = state.binding_generation.ok_or("missing-graph-binding")?;
    drop(state);
    tauri::async_runtime::spawn_blocking(move || {
        activate_sparse_v2_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(|error| format!("Tine-managed storage setup worker failed: {error}"))?
}

fn activate_sparse_v2_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, String> {
    let started = Instant::now();
    let state = app.state::<crate::state::AppState>();
    crate::debug::diag("sparse-v2 activation requested");
    let _transition = state.graph_load.lock().unwrap();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))?;
    let root = slot.root_key.clone();

    if let Some(binding) = slot.sparse_binding() {
        let action = binding.action();
        crate::debug::diag(format!(
            "sparse-v2 activation resuming retained authority: action={action:?}, availability={:?}",
            binding.availability()
        ));
        if action == SparseV2BindingAction::ReturnRetained {
            let result = sparse_v2_status_for_slot(&slot);
            crate::debug::diag(format!(
                "sparse-v2 retained activation completed after {} ms: {result:?}",
                started.elapsed().as_millis()
            ));
            return result;
        }
        let record = state
            .sync_runtime
            .binding_record(app, &root)?
            .ok_or("Tine-managed storage setup is missing.")?;
        let graph_meta = SyncRuntimeFacade::graph_meta(&record);
        let core_started = Instant::now();
        let binding = match action {
            SparseV2BindingAction::ReopenActive => state.sync_runtime.open_record(app, &record)?,
            SparseV2BindingAction::ActivateOrResume => activate_record_with_diagnostics(
                &state.sync_runtime,
                app,
                label,
                binding_generation,
                &record,
            )?,
            SparseV2BindingAction::ReturnRetained => {
                unreachable!("retained bindings return before replacement")
            }
        };
        crate::debug::diag(format!(
            "sparse-v2 retained core operation completed after {} ms: availability={:?}",
            core_started.elapsed().as_millis(),
            binding.availability()
        ));
        let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
            binding, root, graph_meta,
        ));
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&replacement))?;
        crate::state::poke_watcher(&state);
        let result = sparse_v2_status_for_slot(&replacement);
        crate::debug::diag(format!(
            "sparse-v2 retained activation published after {} ms: {result:?}",
            started.elapsed().as_millis()
        ));
        return result;
    }

    let graph = slot.legacy_graph()?;
    let graph_meta = graph.meta();
    drop(graph);
    let record = state
        .sync_runtime
        .prepare_binding_record(app, &root, graph_meta.clone())?;
    crate::debug::diag(format!(
        "sparse-v2 fresh activation prepared private binding after {} ms",
        started.elapsed().as_millis()
    ));

    slot.begin_legacy_retirement()?;
    crate::debug::diag("sparse-v2 legacy authority retirement started");
    let removed = state.graphs.write().unwrap().remove(label);
    if removed
        .as_ref()
        .is_none_or(|removed| removed.binding_generation != slot.binding_generation)
    {
        slot.cancel_legacy_retirement()?;
        return Err(
            "The graph changed while Tine-managed storage was being set up. Retry setup.".into(),
        );
    }
    crate::state::poke_watcher(&state);

    if let Err(error) = slot.wait_for_legacy_drain(LEGACY_DRAIN_TIMEOUT) {
        crate::debug::diag(format!(
            "sparse-v2 legacy authority drain failed after {} ms: {error}",
            started.elapsed().as_millis()
        ));
        slot.cancel_legacy_retirement()?;
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&slot))?;
        crate::state::poke_watcher(&state);
        return Err(format!(
            "Tine-managed storage setup can be retried: {error}"
        ));
    }
    crate::debug::diag(format!(
        "sparse-v2 legacy authority drained after {} ms",
        started.elapsed().as_millis()
    ));

    if let Err(error) = state.sync_runtime.persist_binding_record(app, &record) {
        crate::debug::diag(format!(
            "sparse-v2 private binding persistence failed after {} ms: {error}",
            started.elapsed().as_millis()
        ));
        slot.cancel_legacy_retirement()?;
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&slot))?;
        crate::state::poke_watcher(&state);
        return Err(error);
    }
    crate::debug::diag(format!(
        "sparse-v2 private binding persisted after {} ms; starting core bootstrap",
        started.elapsed().as_millis()
    ));

    let core_started = Instant::now();
    let binding = activate_record_with_diagnostics(
        &state.sync_runtime,
        app,
        label,
        binding_generation,
        &record,
    )?;
    crate::debug::diag(format!(
        "sparse-v2 core bootstrap completed after {} ms: availability={:?}",
        core_started.elapsed().as_millis(),
        binding.availability()
    ));
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        binding, root, graph_meta,
    ));
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.to_string(), Arc::clone(&replacement))?;
    crate::state::poke_watcher(&state);
    let result = sparse_v2_status_for_slot(&replacement);
    crate::debug::diag(format!(
        "sparse-v2 fresh activation published after {} ms: {result:?}",
        started.elapsed().as_millis()
    ));
    result
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2CancelResult {
    status: SparseV2StatusDto,
    binding_generation: u64,
    recovery_statement: String,
}

fn archive_private_root(private_root: &Path, recovery_root: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(private_root).map_err(|error| {
        format!("Couldn't inspect Tine-managed storage recovery state: {error}")
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("Tine-managed storage recovery state is not a local directory, so returning to Direct files is unavailable.".into());
    }
    std::fs::create_dir_all(recovery_root).map_err(|error| {
        format!("Couldn't prepare Tine-managed storage recovery state: {error}")
    })?;
    let key = private_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Tine-managed storage recovery state has no valid local key.")?;
    let destination = recovery_root.join(format!("{key}-{}", Uuid::new_v4()));
    std::fs::rename(private_root, &destination).map_err(|error| {
        format!("Couldn't preserve Tine-managed storage recovery state: {error}")
    })?;
    Ok(destination)
}

fn require_safe_sparse_shutdown(slot: &crate::state::GraphSlot) -> Result<(), String> {
    let Some(handle) = slot.sparse_runtime() else {
        return Ok(());
    };
    match handle.clean_shutdown() {
        Ok(SyncShutdownOutcome::Safe(_)) => Ok(()),
        Ok(SyncShutdownOutcome::Terminal(_)) => {
            Err("Tine-managed storage could not verify a safe local stop.".into())
        }
        Err(error) => Err(format!(
            "Tine-managed storage could not stop safely: {error}"
        )),
    }
}

fn restore_sparse_slot(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    reason: String,
) -> Result<SparseV2CancelResult, String> {
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.to_string(), slot)
        .map_err(|restore| {
            format!("{reason}; Tine-managed storage could not be restored in memory: {restore}")
        })?;
    crate::state::poke_watcher(state);
    Err(reason)
}

fn cancel_sparse_v2_at_paths_with_archive(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(&crate::state::GraphSlot) -> Result<(), String>,
    archive: impl FnOnce(&Path, &Path) -> Result<PathBuf, String>,
) -> Result<SparseV2CancelResult, String> {
    let binding = slot
        .sparse_binding()
        .ok_or("This graph is already using Direct files.")?;
    let record = read_binding_at(&private_root.join(SPARSE_BINDING_FILE), &slot.root_key)?
        .ok_or("Tine-managed storage setup for this graph is missing.")?;
    if record.graph_meta.root != slot.root_key.display().to_string()
        || slot.graph_meta().root != record.graph_meta.root
    {
        return Err("Tine-managed storage data does not match this graph.".into());
    }
    cancel_eligibility(binding, &slot.root_key.join(".tine-sync/v2"))?;

    let removed = state.graphs.write().unwrap().remove(label);
    if removed.is_some() {
        crate::state::poke_watcher(state);
    }
    if removed.as_ref().is_none_or(|current| {
        current.binding_generation != slot.binding_generation || current.root_key != slot.root_key
    }) {
        if let Some(current) = removed {
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), current)?;
            crate::state::poke_watcher(state);
        }
        return Err("The graph changed while returning to Direct files. Try again.".into());
    }

    if let Err(error) = shutdown(&slot) {
        return restore_sparse_slot(state, label, slot, error);
    }

    if let Err(error) = archive(private_root, recovery_root) {
        return restore_sparse_slot(state, label, slot, error);
    }

    let graph = tine_core::model::Graph::open_checked_with_assets(
        &record.graph_root,
        approved_assets,
    )
    .map_err(|error| {
        format!(
            "Tine-managed storage recovery state was preserved, but Direct files could not reopen: {error}. Restart Tine to reopen the unchanged Markdown/Org graph."
        )
    })?;
    let replacement = Arc::new(crate::state::GraphSlot::new(graph, slot.root_key.clone()));
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.to_string(), Arc::clone(&replacement))
        .map_err(|error| {
            format!(
                "Tine-managed storage recovery state was preserved, but Direct files could not be restored: {error}. Restart Tine to reopen the unchanged Markdown/Org graph."
            )
        })?;
    crate::state::poke_watcher(state);
    let status = SparseV2StatusDto::legacy(replacement.binding_generation);
    Ok(SparseV2CancelResult {
        binding_generation: replacement.binding_generation,
        status,
        recovery_statement: "Direct file mode is active. Complete recovery state was preserved."
            .into(),
    })
}

fn cancel_sparse_v2_at_paths(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(&crate::state::GraphSlot) -> Result<(), String>,
) -> Result<SparseV2CancelResult, String> {
    cancel_sparse_v2_at_paths_with_archive(
        state,
        label,
        slot,
        private_root,
        recovery_root,
        approved_assets,
        shutdown,
        archive_private_root,
    )
}

#[tauri::command]
pub(crate) async fn cancel_sparse_v2(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2CancelResult, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        cancel_sparse_v2_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn cancel_sparse_v2_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2CancelResult, String> {
    let state = app.state::<crate::state::AppState>();
    let _transition = state.graph_load.lock().unwrap();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))?;
    let private_root = sparse_private_root(&app, &slot.root_key)?;
    let recovery_root = sparse_recovery_root(&app)?;
    let approved_assets = crate::settings::approved_external_assets(&app, &slot.root_key);
    cancel_sparse_v2_at_paths(
        &state,
        label,
        slot,
        &private_root,
        &recovery_root,
        approved_assets.as_deref(),
        require_safe_sparse_shutdown,
    )
}

/// Publish the already-safe local archive into the single shared v2
/// namespace, then reopen the same private device binding as SharedActive.
#[tauri::command]
pub(crate) async fn prepare_sparse_v2_share(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        prepare_sparse_v2_share_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn prepare_sparse_v2_share_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, String> {
    let state = app.state::<crate::state::AppState>();
    let _transition = state.graph_load.lock().unwrap();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))?;
    let record = state
        .sync_runtime
        .binding_record(&app, &slot.root_key)?
        .ok_or("Tine-managed storage setup is missing.")?;
    active_handle(&slot)?
        .prepare_shared()
        .map_err(|error| error.to_string())?;
    let binding = match state.sync_runtime.open_record(&app, &record) {
        Ok(binding) => binding,
        Err(error) => {
            let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                retryable_binding("share_prepared", error.clone()),
                slot.root_key.clone(),
                SyncRuntimeFacade::graph_meta(&record),
            ));
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)?;
            crate::state::poke_watcher(&state);
            return Err(error);
        }
    };
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        binding,
        slot.root_key.clone(),
        SyncRuntimeFacade::graph_meta(&record),
    ));
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.to_string(), Arc::clone(&replacement))?;
    crate::state::poke_watcher(&state);
    sparse_v2_status_for_slot(&replacement)
}

/// Explicitly retire the second device's legacy reader/watcher, derive its
/// private identity from exact provider descriptor evidence, and join.
#[tauri::command]
pub(crate) async fn join_sparse_v2_shared(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        join_sparse_v2_shared_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn join_sparse_v2_shared_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, String> {
    let state = app.state::<crate::state::AppState>();
    let _transition = state.graph_load.lock().unwrap();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))?;
    let descriptor =
        inspect_shared_enrollment_for_cold_discovery(&slot.root_key.join(".tine-sync/v2/shared"))?
            .ok_or("This graph does not yet contain sync data from another device.")?;
    if slot.sparse_binding().is_some() {
        let record = state
            .sync_runtime
            .binding_record(app, &slot.root_key)?
            .ok_or("Tine-managed storage setup is missing.")?;
        active_handle(&slot)?
            .join_shared(descriptor)
            .map_err(|error| error.to_string())?;
        let reopened = match state.sync_runtime.open_record(app, &record) {
            Ok(binding) => binding,
            Err(error) => {
                let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                    retryable_binding("shared_active", error.clone()),
                    slot.root_key.clone(),
                    SyncRuntimeFacade::graph_meta(&record),
                ));
                state
                    .graphs
                    .write()
                    .unwrap()
                    .bind(label.to_string(), replacement)?;
                crate::state::poke_watcher(&state);
                return Err(error);
            }
        };
        let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
            reopened,
            slot.root_key.clone(),
            SyncRuntimeFacade::graph_meta(&record),
        ));
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&replacement))?;
        crate::state::poke_watcher(&state);
        return sparse_v2_status_for_slot(&replacement);
    }
    let graph = slot.legacy_graph()?;
    let graph_meta = graph.meta();
    drop(graph);
    let record = SparseV2ActivationRecord::from_shared(
        &slot.root_key,
        graph_meta.clone(),
        DeviceId::from_uuid(crate::settings::managed_sync_device_id(app)?),
        &descriptor,
    );

    slot.begin_legacy_retirement()?;
    let removed = state.graphs.write().unwrap().remove(label);
    if removed
        .as_ref()
        .is_none_or(|removed| removed.binding_generation != slot.binding_generation)
    {
        slot.cancel_legacy_retirement()?;
        return Err("The graph changed while joining sync. Try again.".into());
    }
    crate::state::poke_watcher(&state);
    if let Err(error) = slot.wait_for_legacy_drain(LEGACY_DRAIN_TIMEOUT) {
        slot.cancel_legacy_retirement()?;
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&slot))?;
        crate::state::poke_watcher(&state);
        return Err(format!("Joining sync can be retried: {error}"));
    }
    if let Err(error) = state.sync_runtime.persist_binding_record(app, &record) {
        slot.cancel_legacy_retirement()?;
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&slot))?;
        crate::state::poke_watcher(&state);
        return Err(error);
    }
    let activated = match state.sync_runtime.activate_record(app, &record) {
        Ok(activated) => activated,
        Err(error) => {
            let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                SparseV2Binding {
                    availability: SparseV2Availability::Retryable {
                        stage: "activation_request".into(),
                        detail: error.clone(),
                    },
                    handle: None,
                },
                slot.root_key.clone(),
                graph_meta,
            ));
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)?;
            crate::state::poke_watcher(&state);
            return Err(error);
        }
    };
    let Some(handle) = activated.handle() else {
        let detail = format!(
            "join bootstrap did not reach LocalActive: {:?}",
            activated.availability()
        );
        let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
            activated,
            slot.root_key.clone(),
            graph_meta,
        ));
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), replacement)?;
        crate::state::poke_watcher(&state);
        return Err(detail);
    };
    if let Err(error) = handle.join_shared(descriptor) {
        let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
            activated,
            slot.root_key.clone(),
            graph_meta,
        ));
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), replacement)?;
        crate::state::poke_watcher(&state);
        return Err(error.to_string());
    }
    let binding = match state.sync_runtime.open_record(app, &record) {
        Ok(binding) => binding,
        Err(error) => {
            let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                retryable_binding("shared_active", error.clone()),
                slot.root_key.clone(),
                SyncRuntimeFacade::graph_meta(&record),
            ));
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)?;
            crate::state::poke_watcher(&state);
            return Err(error);
        }
    };
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        binding,
        slot.root_key.clone(),
        graph_meta,
    ));
    state
        .graphs
        .write()
        .unwrap()
        .bind(label.to_string(), Arc::clone(&replacement))?;
    crate::state::poke_watcher(&state);
    sparse_v2_status_for_slot(&replacement)
}

#[tauri::command]
pub(crate) async fn sparse_v2_query(
    request: tine_core::sync_runtime::SyncRuntimeQueryRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncRuntimeQueryReply, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        active_handle(&slot)?
            .query(request)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn sparse_v2_editor_load(
    request: tine_core::sync_runtime::SyncEditorLoadRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncEditorLoadOutcome, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        active_handle(&slot)?
            .load_editor_page(request)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn sparse_v2_editor_save(
    request: tine_core::sync_runtime::SyncEditorSaveRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncEditorSaveOutcome, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        active_handle(&slot)?
            .save_editor_page(request)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn sparse_v2_tick(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2TickDto, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        active_handle(&slot)?
            .tick()
            .map(tick_dto)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn sparse_v2_clean_shutdown(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2RuntimeStatusDto, String> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))?;
        active_handle(&slot)?
            .clean_shutdown()
            .map(shutdown_status)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// A graph slot can authorize a process/window exit only after its managed
/// runtime has reached the specific `Safe` shutdown outcome. A terminal actor
/// has stopped accepting work, but it did not prove the clean-stop invariant;
/// collapsing both outcomes because they expose a status snapshot would let an
/// exit discard the recovery path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CleanShutdownSlot {
    Direct,
    Safe,
}

fn clean_shutdown_outcome(outcome: SyncShutdownOutcome) -> Result<CleanShutdownSlot, String> {
    match outcome {
        SyncShutdownOutcome::Safe(_) => Ok(CleanShutdownSlot::Safe),
        SyncShutdownOutcome::Terminal(snapshot) => Err(format!(
            "Tine-managed storage reached a terminal state and cannot authorize process exit: {}",
            snapshot
                .detail
                .unwrap_or_else(|| "no terminal detail was recorded".into())
        )),
    }
}

pub(crate) fn clean_shutdown_slot(
    slot: &crate::state::GraphSlot,
) -> Result<CleanShutdownSlot, String> {
    let Some(handle) = slot.sparse_runtime() else {
        return Ok(CleanShutdownSlot::Direct);
    };
    handle
        .clean_shutdown()
        .map_err(|error| error.to_string())
        .and_then(clean_shutdown_outcome)
}

#[cfg(test)]
mod clean_shutdown_slot_tests {
    use super::*;

    fn snapshot(
        lifecycle: SyncRuntimeLifecycle,
        detail: Option<&str>,
    ) -> SyncRuntimeStatusSnapshot {
        SyncRuntimeStatusSnapshot {
            lifecycle,
            recovery: None,
            watcher: Default::default(),
            last_tick: None,
            detail: detail.map(str::to_owned),
            shared_role: None,
            shared_phase: None,
            provider_pending: 0,
            managed_local_pending: 0,
            managed_local_checkpointed_sequence: 0,
            managed_local_next_sequence: 0,
            managed_local_stage: None,
        }
    }

    #[test]
    fn terminal_shutdown_outcome_cannot_authorize_an_exit() {
        let terminal = SyncShutdownOutcome::Terminal(snapshot(
            SyncRuntimeLifecycle::Terminal,
            Some("authority lease was revoked"),
        ));
        assert!(clean_shutdown_outcome(terminal)
            .unwrap_err()
            .contains("cannot authorize process exit"));

        let safe = SyncShutdownOutcome::Safe(snapshot(SyncRuntimeLifecycle::StoppedSafe, None));
        assert_eq!(clean_shutdown_outcome(safe), Ok(CleanShutdownSlot::Safe));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tine_core::model::Graph;
    use tine_core::sync_runtime::{
        SyncApplicationPageLoadOutcome, SyncApplicationPageLoadRequest,
        SyncApplicationPageSaveOutcome, SyncApplicationPageSaveRequest,
        SyncApplicationPageSaveTarget, SyncApplicationPageSelector, SyncEditorBlockDto,
        SyncEditorBlockKey, SyncEditorLoadOutcome, SyncEditorLoadRequest, SyncEditorPageSelector,
        SyncEditorSaveOutcome, SyncEditorSaveRequest, SyncEditorSaveTarget, SyncEntityId,
        SyncLocalMutationOutcome, SyncPageKind, SyncPageNameResolutionDto, SyncRuntimeQueryReply,
        SyncRuntimeQueryRequest, SyncSearchHitDto, SyncWatcherObservation,
    };

    #[test]
    fn activation_command_owns_the_complete_transition_inside_spawn_blocking() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("pub(crate) async fn activate_sparse_v2")
            .expect("activation command must be async");
        let end = source[start..]
            .find("#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]")
            .map(|offset| start + offset)
            .expect("activation command boundary");
        let command = &source[start..end];
        let blocking = command
            .find("tauri::async_runtime::spawn_blocking(move ||")
            .expect("activation transition must enter spawn_blocking");
        let before_blocking = &command[..blocking];
        assert!(
            !before_blocking.contains("graph_load.lock")
                && !before_blocking.contains("slot_for_context"),
            "graph authority and the transition lock must be resolved inside the owned blocking operation"
        );
        assert!(
            command[blocking..]
                .contains("activate_sparse_v2_blocking(&app, &label, binding_generation)"),
            "the blocking operation must re-resolve the exact graph generation from owned inputs"
        );
    }

    #[test]
    fn every_explicit_managed_actor_command_re_resolves_off_the_async_command_thread() {
        let source = include_str!("sync_runtime.rs");
        for name in [
            "sparse_v2_status",
            "cancel_sparse_v2",
            "prepare_sparse_v2_share",
            "join_sparse_v2_shared",
            "sparse_v2_query",
            "sparse_v2_editor_load",
            "sparse_v2_editor_save",
            "sparse_v2_tick",
            "sparse_v2_clean_shutdown",
        ] {
            let signature = format!("pub(crate) async fn {name}(");
            let start = source
                .find(&signature)
                .expect("managed command stays async");
            let tail = &source[start..];
            let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
            let command = &tail[..end];
            assert!(
                command.contains("owned_graph_context(state)?"),
                "{name} must own the exact window binding before await"
            );
            assert!(
                command.contains("tauri::async_runtime::spawn_blocking(move ||"),
                "{name} must move every possible managed actor wait to the blocking pool"
            );
            assert!(
                command.contains("slot_for_bound_window")
                    && command.contains("Some(binding_generation)"),
                "{name} must re-resolve the captured generation inside the blocking operation"
            );
        }
    }

    #[test]
    fn activation_heartbeat_stops_and_joins_without_waiting_for_the_interval() {
        let started = Instant::now();
        let heartbeat = ActivationHeartbeat::start(started, Arc::new(Mutex::new(None)));
        drop(heartbeat);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "heartbeat shutdown waited for the ten-second reporting interval"
        );
    }

    #[test]
    fn activation_heartbeat_reports_latest_detailed_part_progress() {
        let latest = Arc::new(Mutex::new(Some(
            SyncLocalActivationProgress::BootstrapDetachedAuthoring {
                completed: 2,
                total: 5,
            },
        )));
        assert_eq!(
            latest_activation_progress_name(&latest),
            "bootstrap preparation: detached authoring 2/5 parts"
        );
        let event = SparseV2ActivationProgressEvent {
            binding_generation: 17,
            progress: latest.lock().unwrap().clone().unwrap(),
        };
        let serialized = serde_json::to_value(event).unwrap();
        assert_eq!(serialized["binding_generation"], 17);
        assert_eq!(
            serialized["progress"]["kind"],
            "bootstrap_detached_authoring"
        );
        assert_eq!(serialized["progress"]["completed"], 2);
        assert_eq!(serialized["progress"]["total"], 5);
    }

    struct RollbackFixture {
        root: PathBuf,
        graph_root: PathBuf,
        private_root: PathBuf,
        recovery_root: PathBuf,
        markdown_path: PathBuf,
        markdown_bytes: Vec<u8>,
        binding_bytes: Vec<u8>,
        state: crate::state::AppState,
        slot: Arc<crate::state::GraphSlot>,
    }

    impl RollbackFixture {
        fn new(stage: Option<&str>) -> Self {
            let root =
                std::env::temp_dir().join(format!("tine-sparse-rollback-{}", Uuid::new_v4()));
            let graph_root = root.join("graph");
            let private_root = root.join("app-data/sparse-v2/graph-key");
            let recovery_root = root.join("app-data/sparse-v2-recovery");
            let markdown_path = graph_root.join("pages/rollback.md");
            let markdown_bytes = b"- Markdown remains authoritative\n".to_vec();
            std::fs::create_dir_all(graph_root.join("pages")).unwrap();
            std::fs::create_dir_all(graph_root.join("journals")).unwrap();
            std::fs::write(&markdown_path, &markdown_bytes).unwrap();
            let graph = Graph::open(&graph_root);
            let meta = graph.meta();
            drop(graph);
            let record = SparseV2ActivationRecord::new(&graph_root, meta.clone(), DeviceId::new());
            persist_binding_at(&private_root.join(SPARSE_BINDING_FILE), &record).unwrap();
            std::fs::write(private_root.join("diagnostic-bytes"), b"preserve exactly").unwrap();
            let binding_bytes = std::fs::read(private_root.join(SPARSE_BINDING_FILE)).unwrap();
            let binding = retryable_binding(
                stage.unwrap_or("shadow_import"),
                "incomplete local activation".into(),
            );
            let slot = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                binding,
                graph_root.clone(),
                meta,
            ));
            let state = crate::state::AppState {
                graphs: std::sync::RwLock::new(crate::state::GraphRegistry::default()),
                graph_load: Mutex::new(()),
                watch_ctl: Mutex::new(None),
                last_focused: Mutex::new(None),
                capture_graph: Mutex::new(None),
                sync_runtime: SyncRuntimeFacade,
                #[cfg(desktop)]
                next_window: std::sync::atomic::AtomicU64::new(1),
            };
            state
                .graphs
                .write()
                .unwrap()
                .bind("main".into(), Arc::clone(&slot))
                .unwrap();
            Self {
                root,
                graph_root,
                private_root,
                recovery_root,
                markdown_path,
                markdown_bytes,
                binding_bytes,
                state,
                slot,
            }
        }

        fn make_active(&mut self) {
            let record = read_binding_at(
                &self.private_root.join(SPARSE_BINDING_FILE),
                &self.graph_root,
            )
            .unwrap()
            .unwrap();
            let activated =
                SyncRuntimeHandle::activate_or_resume_local(SyncLocalActivationRequest {
                    graph_root: self.graph_root.clone(),
                    archive_root: self.private_root.join("archive"),
                    enrollment_root: self.private_root.join("enrollment"),
                    receipt_root: self.private_root.join("receipts"),
                    database_path: self.private_root.join("projection/materialization.sqlite"),
                    application_runtime_root: self.private_root.join("runtime"),
                    migration_backup_root: self.private_root.join("migration-backup"),
                    capture_root: self.private_root.join("capture"),
                    preparation_root: self.private_root.join("preparation"),
                    provider_root: self.graph_root.join(".tine-sync/v2/shared"),
                    provider_journal_root: self.private_root.join("provider/device/journal"),
                    identities: SyncLocalActivationIdentities {
                        workspace_id: record.workspace_id,
                        lineage_digest: record.lineage_digest,
                        catalog_document_id: record.catalog_document_id,
                        endpoint_id: record.endpoint_id,
                        device_id: record.device_id,
                        preparation_id: record.preparation_id,
                        session_id: record.activation_session_id,
                    },
                });
            assert_eq!(activated.status, SyncLocalActivationStatus::Active);
            let active = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                SparseV2Binding::from_activation(activated),
                self.graph_root.clone(),
                SyncRuntimeFacade::graph_meta(&record),
            ));
            self.state
                .graphs
                .write()
                .unwrap()
                .bind("main".into(), Arc::clone(&active))
                .unwrap();
            self.slot = active;
            self.binding_bytes =
                std::fs::read(self.private_root.join(SPARSE_BINDING_FILE)).unwrap();
        }
    }

    impl Drop for RollbackFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(base: &Path, current: &Path, found: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let mut entries = std::fs::read_dir(current)
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    visit(base, &path, found);
                } else {
                    found.insert(
                        path.strip_prefix(base).unwrap().to_path_buf(),
                        std::fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut found = BTreeMap::new();
        if root.is_dir() {
            visit(root, root, &mut found);
        }
        found
    }

    #[test]
    fn sparse_binding_without_live_handle_gives_actionable_recovery() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        assert_eq!(
            active_handle(&fixture.slot).unwrap_err(),
            SPARSE_V2_NOT_ACTIVE
        );
        assert!(SPARSE_V2_NOT_ACTIVE.contains("Retry setup"));
        assert!(SPARSE_V2_NOT_ACTIVE.contains("return to Direct files"));
    }

    #[test]
    fn transition_status_uses_the_exact_slots_rollback_eligibility() {
        let local = RollbackFixture::new(Some("shadow_import"));
        let local_status = sparse_v2_status_for_slot(&local.slot).unwrap();
        assert!(matches!(
            local_status.availability,
            SparseV2Availability::Retryable { ref stage, .. } if stage == "shadow_import"
        ));
        assert!(local_status.can_cancel);
        assert_eq!(local_status.cancel_reason, None);
        assert_eq!(
            local_status.binding_generation,
            local.slot.binding_generation
        );

        for stage in ["share_prepared", "joining", "shared_active"] {
            let shared = RollbackFixture::new(Some(stage));
            let shared_status = sparse_v2_status_for_slot(&shared.slot).unwrap();
            assert!(!shared_status.can_cancel, "{stage}");
            assert!(
                shared_status
                    .cancel_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("synced with another device")),
                "{stage}: {:?}",
                shared_status.cancel_reason
            );
        }

        let provider = RollbackFixture::new(Some("shadow_import"));
        std::fs::create_dir_all(provider.graph_root.join(".tine-sync/v2")).unwrap();
        let provider_status = sparse_v2_status_for_slot(&provider.slot).unwrap();
        assert!(!provider_status.can_cancel);
        assert!(provider_status
            .cancel_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("synced with another device")));
    }

    #[test]
    fn incomplete_local_activation_retires_without_touching_markdown_and_preserves_private_bytes() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            require_safe_sparse_shutdown,
        )
        .unwrap();

        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert_eq!(result.binding_generation, result.status.binding_generation);
        assert!(result
            .recovery_statement
            .contains("Complete recovery state was preserved"));
        assert!(!fixture.private_root.exists());
        let archives = std::fs::read_dir(&fixture.recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert_eq!(
            std::fs::read(archives[0].join(SPARSE_BINDING_FILE)).unwrap(),
            fixture.binding_bytes
        );
        assert_eq!(
            std::fs::read(archives[0].join("diagnostic-bytes")).unwrap(),
            b"preserve exactly"
        );
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            fixture.markdown_bytes
        );
        assert!(fixture
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());
        assert!(read_binding_at(
            &fixture.private_root.join(SPARSE_BINDING_FILE),
            &fixture.graph_root
        )
        .unwrap()
        .is_none());
        assert!(Graph::open_checked(&fixture.graph_root).is_ok());
    }

    #[test]
    fn rollback_reload_save_uses_current_disk_revision_and_later_external_write_conflicts() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            require_safe_sparse_shutdown,
        )
        .unwrap();

        let replacement = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        let graph = replacement.legacy_graph().unwrap();
        let mut reloaded = graph
            .load_named("rollback", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap();
        let baseline = reloaded.rev.take().unwrap();
        reloaded.blocks[0].raw = "ordinary edit after rollback".into();
        let saved = graph.save_page(&reloaded, Some(&baseline)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&fixture.markdown_path).unwrap(),
            "- ordinary edit after rollback\n"
        );

        let mut current = graph
            .load_named("rollback", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(current.rev.take().as_deref(), Some(saved.as_str()));
        current.blocks[0].raw = "must not overwrite external bytes".into();
        std::fs::write(&fixture.markdown_path, b"- genuinely external edit\n").unwrap();

        assert_eq!(
            graph.save_page(&current, Some(&saved)).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            std::fs::read_to_string(&fixture.markdown_path).unwrap(),
            "- genuinely external edit\n"
        );
    }

    #[test]
    fn active_local_rollback_requires_and_completes_a_clean_safe_shutdown() {
        std::thread::Builder::new()
            .name("tine-sparse-active-rollback-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(active_local_rollback_requires_and_completes_a_clean_safe_shutdown_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn active_local_rollback_requires_and_completes_a_clean_safe_shutdown_inner() {
        let mut fixture = RollbackFixture::new(Some("shadow_import"));
        fixture.make_active();
        let handle = fixture.slot.sparse_runtime().unwrap();
        for _ in 0..128 {
            let before = handle.status().unwrap();
            if !before.watcher.pending && before.provider_pending == 0 {
                break;
            }
            handle.tick().unwrap();
        }
        let before = fixture.slot.sparse_runtime().unwrap().status().unwrap();
        assert_eq!(before.lifecycle, SyncRuntimeLifecycle::Active);
        assert!(
            before.shared_role.is_none() && before.shared_phase.is_none(),
            "fresh local activation unexpectedly named shared work: {before:?}"
        );
        assert_eq!(runtime_status(before).provider_pending, 0);

        cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            require_safe_sparse_shutdown,
        )
        .unwrap();

        assert_eq!(
            fixture
                .slot
                .sparse_runtime()
                .unwrap()
                .status()
                .unwrap()
                .lifecycle,
            SyncRuntimeLifecycle::StoppedSafe
        );
        assert!(fixture
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());
    }

    #[test]
    fn shutdown_refusal_restores_sparse_authority_and_changes_no_durable_bytes() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        let private_before = snapshot_tree(&fixture.private_root);
        let markdown_before = std::fs::read(&fixture.markdown_path).unwrap();
        let generation = fixture.slot.binding_generation;

        let error = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            |_| Err("injected clean shutdown refusal".into()),
        )
        .unwrap_err();

        assert!(error.contains("injected clean shutdown refusal"));
        let restored = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(restored.is_sparse_v2());
        assert_eq!(restored.binding_generation, generation);
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            markdown_before
        );
        assert!(!fixture.recovery_root.exists());
    }

    #[test]
    fn archive_rename_failure_restores_the_same_sparse_slot_and_all_bytes() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        let private_before = snapshot_tree(&fixture.private_root);
        let markdown_before = std::fs::read(&fixture.markdown_path).unwrap();
        let generation = fixture.slot.binding_generation;

        let error = cancel_sparse_v2_at_paths_with_archive(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            |_| {
                assert!(fixture.state.graphs.read().unwrap().slot("main").is_none());
                Ok(())
            },
            |private_root, recovery_root| {
                assert_eq!(private_root, fixture.private_root);
                assert_eq!(recovery_root, fixture.recovery_root);
                assert!(fixture.state.graphs.read().unwrap().slot("main").is_none());
                Err("injected archive rename failure".into())
            },
        )
        .unwrap_err();

        assert!(error.contains("injected archive rename failure"));
        let restored = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(Arc::ptr_eq(&restored, &fixture.slot));
        assert_eq!(restored.binding_generation, generation);
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            markdown_before
        );
        assert!(!fixture.recovery_root.exists());
    }

    #[test]
    fn any_shared_or_provider_evidence_refuses_rollback_before_shutdown() {
        let provider = RollbackFixture::new(Some("shadow_import"));
        std::fs::create_dir_all(provider.graph_root.join(".tine-sync/v2")).unwrap();
        std::fs::write(
            provider.graph_root.join(".tine-sync/v2/provider-evidence"),
            b"shared",
        )
        .unwrap();
        let provider_error = cancel_sparse_v2_at_paths(
            &provider.state,
            "main",
            Arc::clone(&provider.slot),
            &provider.private_root,
            &provider.recovery_root,
            None,
            |_| panic!("provider evidence must refuse before shutdown"),
        )
        .unwrap_err();
        assert!(provider_error.contains("synced with another device"));
        assert!(provider
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .is_sparse_v2());

        let shared = RollbackFixture::new(Some("joining"));
        let shared_error = cancel_sparse_v2_at_paths(
            &shared.state,
            "main",
            Arc::clone(&shared.slot),
            &shared.private_root,
            &shared.recovery_root,
            None,
            |_| panic!("shared lifecycle must refuse before shutdown"),
        )
        .unwrap_err();
        assert!(shared_error.contains("synced with another device"));
        assert!(shared.private_root.exists());
    }

    #[test]
    fn facade_legacy_default_inspects_nothing_and_retains_nothing() {
        let facade = SyncRuntimeFacade;
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SyncRuntimeFacade>();

        let root = std::env::temp_dir().join(format!("tine-sync-facade-legacy-{}", Uuid::new_v4()));
        let opened = facade.open_explicit(SyncRuntimeOpenRequest {
            profile: SyncStorageProfile::LegacyDefault,
            graph_root: root.join("missing-graph"),
            enrollment_root: root.join("missing-enrollment"),
            archive_root: root.join("missing-archive"),
            receipt_root: root.join("missing-receipts"),
            database_path: root.join("missing.sqlite"),
            application_runtime_root: root.join("missing-runtime"),
            migration_backup_root: root.join("missing-migration-backup"),
            provider_root: root.join("missing-provider"),
            provider_journal_root: root.join("missing-provider-journal/device/journal"),
        });
        assert_eq!(opened.status, SyncRuntimeOpenStatus::LegacyDefault);
        assert!(opened.handle.is_none());
        assert!(!root.exists());
    }

    #[test]
    fn binding_record_rejects_unknown_fields_and_wrong_roots() {
        let root = std::env::temp_dir().join(format!("tine-sparse-binding-{}", Uuid::new_v4()));
        let graph = root.join("graph");
        let other = root.join("other");
        let meta = GraphMeta {
            root: graph.display().to_string(),
            journals_dir: "journals".into(),
            pages_dir: "pages".into(),
            preferred_workflow: "now".into(),
            shortcuts: Default::default(),
            start_of_week: 6,
            block_hidden_properties: Vec::new(),
            default_journal_template: None,
            favorites: Vec::new(),
            journal_page_title_format: "MMM do, yyyy".into(),
            journal_file_name_format: "yyyy_MM_dd".into(),
            preferred_format: "md".into(),
            macros: Default::default(),
            enable_timetracking: true,
            show_brackets: true,
            doc_mode_enter_for_new_block: false,
            logical_outdenting: false,
            logbook_with_second_support: true,
            logbook_enabled_in_timestamped_blocks: false,
            logbook_enabled_in_all_blocks: false,
            guide_announced: false,
        };
        let record = SparseV2ActivationRecord::new(&graph, meta, DeviceId::new());
        let path = root.join("binding.json");
        persist_binding_at(&path, &record).unwrap();
        let reopened = read_binding_at(&path, &graph).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(reopened).unwrap(),
            serde_json::to_value(&record).unwrap()
        );
        assert!(read_binding_at(&path, &other).is_err());

        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["unknown"] = serde_json::json!(true);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(read_binding_at(&path, &graph).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stopped_sparse_bindings_are_retryable_reopen_candidates() {
        assert_eq!(
            action_for_runtime_lifecycle(&SyncRuntimeLifecycle::StoppedSafe),
            SparseV2BindingAction::ReopenActive
        );
        assert_eq!(
            action_for_runtime_lifecycle(&SyncRuntimeLifecycle::StoppedCrashed),
            SparseV2BindingAction::ReopenActive
        );
        assert_eq!(
            action_for_runtime_lifecycle(&SyncRuntimeLifecycle::Active),
            SparseV2BindingAction::ReturnRetained
        );
        assert_eq!(
            action_for_runtime_lifecycle(&SyncRuntimeLifecycle::Terminal),
            SparseV2BindingAction::ReturnRetained
        );
        for stage in ["local_active", "share_prepared", "joining", "shared_active"] {
            assert_eq!(
                retryable_binding(stage, "transient reopen failure".into()).action(),
                SparseV2BindingAction::ReopenActive,
                "{stage} must not strand a stopped Tauri slot"
            );
        }
    }

    #[test]
    fn public_query_wire_uses_exact_kind_and_value_envelopes() {
        let request = SyncRuntimeQueryRequest::Search {
            query: "exact wire".into(),
            limit: 7,
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "kind": "search",
                "query": "exact wire",
                "limit": 7
            })
        );

        let reply = SyncRuntimeQueryReply::Search(vec![SyncSearchHitDto {
            entity: SyncEntityId::Block("block-opaque".into()),
            page_id: "page-opaque".into(),
            text: "exact wire".into(),
            rank: -0.25,
        }]);
        assert_eq!(
            serde_json::to_value(reply).unwrap(),
            serde_json::json!({
                "kind": "search",
                "value": [{
                    "entity": {
                        "entity_type": "block",
                        "id": "block-opaque"
                    },
                    "page_id": "page-opaque",
                    "text": "exact wire",
                    "rank": -0.25
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(SyncRuntimeQueryReply::PageName(
                SyncPageNameResolutionDto::Missing
            ))
            .unwrap(),
            serde_json::json!({
                "kind": "page_name",
                "value": {
                    "status": "missing"
                }
            })
        );
    }

    #[test]
    fn app_boundary_activation_editor_watcher_shutdown_and_reopen_journey() {
        std::thread::Builder::new()
            .name("tine-sparse-app-boundary-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(app_boundary_activation_editor_watcher_shutdown_and_reopen_journey_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn app_boundary_activation_editor_watcher_shutdown_and_reopen_journey_inner() {
        let root = std::env::temp_dir().join(format!("tine-sparse-app-journey-{}", Uuid::new_v4()));
        let graph_root = root.join("graph");
        let private = root.join("private");
        let relative = "archive/層/Résumé 日本語.md";
        std::fs::create_dir_all(graph_root.join("pages")).unwrap();
        std::fs::create_dir_all(graph_root.join("journals")).unwrap();
        std::fs::create_dir_all(graph_root.join("archive/層")).unwrap();
        std::fs::write(
            graph_root.join(relative),
            "- nested UTF original content — café 日本語\n",
        )
        .unwrap();

        let graph = Graph::open(&graph_root);
        let meta = graph.meta();
        drop(graph);
        let record = SparseV2ActivationRecord::new(&graph_root, meta.clone(), DeviceId::new());
        let request = SyncLocalActivationRequest {
            graph_root: graph_root.clone(),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
            migration_backup_root: private.join("migration-backup"),
            capture_root: private.join("capture"),
            preparation_root: private.join("preparation"),
            provider_root: graph_root.join(".tine-sync/v2/shared"),
            provider_journal_root: private.join("provider/device/journal"),
            identities: SyncLocalActivationIdentities {
                workspace_id: record.workspace_id,
                lineage_digest: record.lineage_digest,
                catalog_document_id: record.catalog_document_id,
                endpoint_id: record.endpoint_id,
                device_id: record.device_id,
                preparation_id: record.preparation_id,
                session_id: record.activation_session_id,
            },
        };
        let open_request = SyncRuntimeOpenRequest {
            profile: SyncStorageProfile::ExperimentalLocal,
            graph_root: request.graph_root.clone(),
            enrollment_root: request.enrollment_root.clone(),
            archive_root: request.archive_root.clone(),
            receipt_root: request.receipt_root.clone(),
            database_path: request.database_path.clone(),
            application_runtime_root: request.application_runtime_root.clone(),
            migration_backup_root: request.migration_backup_root.clone(),
            provider_root: request.provider_root.clone(),
            provider_journal_root: request.provider_journal_root.clone(),
        };

        let activated = SyncRuntimeHandle::activate_or_resume_local(request);
        assert_eq!(activated.status, SyncLocalActivationStatus::Active);
        let binding = SparseV2Binding::from_activation(activated);
        let slot =
            crate::state::GraphSlot::from_sparse_v2(binding, graph_root.clone(), meta.clone());
        assert_eq!(
            slot.legacy_graph().err().as_deref(),
            Some(crate::state::SPARSE_V2_UNSUPPORTED)
        );
        let handle = slot
            .sparse_runtime()
            .expect("active sparse slot must retain the actor");
        for _ in 0..128 {
            match handle.tick().unwrap() {
                SyncRuntimeTick::Idle
                | SyncRuntimeTick::AdmittedNoop { .. }
                | SyncRuntimeTick::AdmittedComplete { .. }
                    if !handle.status().unwrap().watcher.pending =>
                {
                    break;
                }
                SyncRuntimeTick::Idle
                | SyncRuntimeTick::AdmittedNoop { .. }
                | SyncRuntimeTick::AdmittedComplete { .. }
                | SyncRuntimeTick::Recovering
                | SyncRuntimeTick::RetryFull
                | SyncRuntimeTick::Failed(_) => {}
                other => panic!(
                    "initial app-boundary feed did not settle: {other:?}; status={:?}",
                    handle.status().unwrap()
                ),
            }
        }

        let loaded = handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath {
                    path: relative.into(),
                },
            })
            .unwrap();
        let SyncApplicationPageLoadOutcome::Loaded { mut page, revision } = loaded else {
            panic!(
                "activation did not expose the existing page through the app gateway: {loaded:?}"
            );
        };
        page.blocks[0]
            .raw
            .push_str(" sparse v2 saved existing UTF page");
        let saved = handle
            .save_application_page(SyncApplicationPageSaveRequest {
                target: SyncApplicationPageSaveTarget::Existing {
                    path: relative.into(),
                    revision,
                },
                page,
            })
            .unwrap();
        let SyncApplicationPageSaveOutcome::Saved {
            page: saved_page,
            revision: saved_revision,
            ..
        } = saved
        else {
            panic!(
                "activation-imported existing page did not save through the app gateway: {saved:?}"
            );
        };
        assert_eq!(saved_page.path, relative);
        assert_eq!(
            saved_page.blocks[0].raw,
            "nested UTF original content — café 日本語 sparse v2 saved existing UTF page"
        );
        let reloaded = handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath {
                    path: relative.into(),
                },
            })
            .unwrap();
        assert!(
            matches!(
                &reloaded,
                SyncApplicationPageLoadOutcome::Loaded {
                    page,
                    revision,
                } if page.path == relative
                    && page.blocks[0].raw
                        == "nested UTF original content — café 日本語 sparse v2 saved existing UTF page"
                    && revision == &saved_revision
            ),
            "application gateway did not reload its accepted semantic result: {reloaded:?}"
        );
        assert_eq!(
            std::fs::read_to_string(graph_root.join(relative)).unwrap(),
            "- nested UTF original content — café 日本語 sparse v2 saved existing UTF page\n"
        );
        let independently_parsed = Graph::open(&graph_root)
            .load_by_path(relative)
            .unwrap()
            .expect("materialized nested UTF page must remain independently parseable");
        assert_eq!(independently_parsed.path, relative);
        assert_eq!(
            independently_parsed.blocks[0].raw,
            "nested UTF original content — café 日本語 sparse v2 saved existing UTF page"
        );

        let loaded = handle
            .load_editor_page(SyncEditorLoadRequest {
                page: SyncEditorPageSelector::Name {
                    name: "Boundary page".into(),
                    page_kind: SyncPageKind::Page,
                },
            })
            .unwrap();
        let SyncEditorLoadOutcome::NewPage { draft } = loaded else {
            panic!("activation did not expose a frontier-bound new-page draft: {loaded:?}");
        };
        let saved = handle
            .save_editor_page(SyncEditorSaveRequest {
                target: SyncEditorSaveTarget::New {
                    name: draft.name,
                    page_kind: draft.page_kind,
                    revision: draft.revision,
                    format: None,
                },
                preamble: None,
                blocks: vec![SyncEditorBlockDto {
                    key: SyncEditorBlockKey::Temporary("first".into()),
                    parent: None,
                    content: "edited through Tauri boundary".into(),
                }],
            })
            .unwrap();
        if matches!(
            saved,
            SyncEditorSaveOutcome::Deferred {
                state: tine_core::sync_runtime::SyncEditorDeferred::RetryableRetainedPublication { .. },
                ..
            }
        ) {
            let mut durable = false;
            for _ in 0..64 {
                if matches!(
                    handle.tick().unwrap(),
                    SyncRuntimeTick::LocalMutation(SyncLocalMutationOutcome::Durable { .. })
                ) {
                    durable = true;
                    break;
                }
            }
            assert!(durable, "retained new-page save did not become durable");
        } else {
            assert!(
                matches!(saved, SyncEditorSaveOutcome::Durable { .. }),
                "new-page save after existing-page save was not accepted: {saved:?}"
            );
        }

        let searched = handle
            .query(SyncRuntimeQueryRequest::Search {
                query: "Tauri boundary".into(),
                limit: 10,
            })
            .unwrap();
        assert!(
            matches!(searched, SyncRuntimeQueryReply::Search(ref rows) if !rows.is_empty()),
            "actor query must see the durable editor save: {searched:?}"
        );
        std::fs::write(
            graph_root.join(relative),
            "- externally imported through watcher\n- second external block\n",
        )
        .unwrap();
        handle
            .observe_watcher(vec![SyncWatcherObservation::managed_path(relative).unwrap()])
            .unwrap();
        let mut imported = false;
        for _ in 0..64 {
            match handle.tick().unwrap() {
                SyncRuntimeTick::AdmittedComplete { .. } | SyncRuntimeTick::AdmittedNoop { .. } => {
                    imported = true;
                    break;
                }
                SyncRuntimeTick::Idle
                | SyncRuntimeTick::Recovering
                | SyncRuntimeTick::RetryFull
                | SyncRuntimeTick::LocalMutation(_) => {}
                other => panic!("watcher import failed at app boundary: {other:?}"),
            }
        }
        assert!(
            imported,
            "watcher import did not settle within its bounded turns"
        );
        let reloaded = handle
            .load_editor_page(SyncEditorLoadRequest {
                page: SyncEditorPageSelector::Name {
                    name: "Résumé 日本語".into(),
                    page_kind: SyncPageKind::Page,
                },
            })
            .unwrap();
        assert!(
            matches!(
                reloaded,
                SyncEditorLoadOutcome::Loaded { ref page }
                    if page.blocks.len() == 2
                        && page.blocks[0].content == "externally imported through watcher"
                        && page.blocks[1].content == "second external block"
            ),
            "editor load must observe the watcher-authored batch: {reloaded:?}"
        );
        let application_reloaded = handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath {
                    path: relative.into(),
                },
            })
            .unwrap();
        assert!(
            matches!(
                application_reloaded,
                SyncApplicationPageLoadOutcome::Loaded { ref page, .. }
                    if page.blocks.len() == 2
                        && page.blocks[0].raw == "externally imported through watcher"
                        && page.blocks[1].raw == "second external block"
            ),
            "application load must observe the watcher-authored batch: {application_reloaded:?}"
        );

        assert!(matches!(
            clean_shutdown_slot(&slot).unwrap(),
            CleanShutdownSlot::Safe
        ));
        let stopped = slot
            .sparse_binding()
            .expect("the stopped slot must remain sparse");
        assert_eq!(stopped.action(), SparseV2BindingAction::ReopenActive);
        let stopped_status = SparseV2StatusDto::from_binding(stopped, slot.binding_generation);
        assert!(matches!(
            stopped_status.availability,
            SparseV2Availability::Retryable { ref stage, ref detail }
                if stage == "local_active" && detail.contains("stopped safely")
        ));
        assert!(stopped_status.can_retry);
        drop(slot);

        let reopened = SyncRuntimeHandle::open(open_request);
        assert_eq!(reopened.status, SyncRuntimeOpenStatus::Active);
        let reopened = SparseV2Binding::from_open(reopened);
        let reopened_slot = crate::state::GraphSlot::from_sparse_v2(reopened, graph_root, meta);
        let reply = reopened_slot
            .sparse_runtime()
            .unwrap()
            .query(SyncRuntimeQueryRequest::Search {
                query: "externally imported".into(),
                limit: 10,
            })
            .unwrap();
        assert!(matches!(
            reply,
            SyncRuntimeQueryReply::Search(ref rows) if !rows.is_empty()
        ));
        assert!(matches!(
            clean_shutdown_slot(&reopened_slot).unwrap(),
            CleanShutdownSlot::Safe
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
