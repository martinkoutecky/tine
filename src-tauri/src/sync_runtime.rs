//! Explicit Tauri-facing sparse-v2 runtime composition.
//!
//! A durable caller-owned binding in private app data is the opt-in marker.
//! Ordinary graph loading never creates it. Once present, startup discovers
//! sparse state and never falls back to a legacy `Graph` writer.

use crate::storage_mode_supervisor::{
    StableStorageMode, StorageTransitionKind, StorageTransitionOutcome, StorageTransitionPhase,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tine_core::model::GraphMeta;
use tine_core::oplog::sync_layout::{
    PRIVATE_BINDING_DIR as SPARSE_BINDING_DIR, PRIVATE_BINDING_FILE as SPARSE_BINDING_FILE,
    PRIVATE_RECOVERY_DIR as SPARSE_RECOVERY_DIR, PROVIDER_INBOX_DIR, PROVIDER_OUTBOX_DIR,
    SHARED_ENROLLMENT_DESCRIPTOR_PATH,
};
use tine_core::oplog::SHARED_PROVIDER_TREE_NAMESPACES;
use tine_core::oplog::{
    DeviceId, DocumentId, LineageDigest, ProjectionEndpointId, SessionId, WorkspaceId,
};
use tine_core::sync_runtime::{
    inspect_shared_enrollment_for_cold_discovery, ManagedStorageRefusalScenario,
    SyncAmbiguousEvidence, SyncApplicationMoveSubtreesOutcome, SyncApplicationMoveSubtreesRequest,
    SyncApplicationPageInventoryOutcome, SyncApplicationPageLoadOutcome,
    SyncApplicationPageLoadRequest, SyncApplicationPageSelector,
    SyncCheckpointPublicationDiagnostics, SyncHistoryRecoveryStatus, SyncLocalActivationIdentities,
    SyncLocalActivationPhase, SyncLocalActivationProgress, SyncLocalActivationRequest,
    SyncLocalActivationResult, SyncLocalActivationStage, SyncLocalActivationStatus,
    SyncNonActiveStage, SyncRuntimeComponent, SyncRuntimeHandle, SyncRuntimeLifecycle,
    SyncRuntimeOpenPhase, SyncRuntimeOpenProgress, SyncRuntimeOpenRequest, SyncRuntimeOpenResult,
    SyncRuntimeOpenStatus, SyncRuntimeRecovery, SyncRuntimeStatusSnapshot, SyncRuntimeTick,
    SyncSharedEnrollmentDescriptor, SyncSharedPhase, SyncSharedRole, SyncShutdownOutcome,
    SyncStorageProfile,
};
use uuid::Uuid;

const BINDING_SCHEMA_VERSION: u32 = 2;
static BINDING_WRITE: Mutex<()> = Mutex::new(());
const DIRECT_SELECTION_SCHEMA_VERSION: u32 = 1;
const DIRECT_SELECTION_DIR: &str = "storage-mode-selections";
const DIRECT_SELECTION_FILE_SUFFIX: &str = ".direct-v1.json";
const BLANK_SLATE_REBUILD_REASON: &str = "pre_0_7_blank_slate_rebuild_pending";
const BLANK_SLATE_BACKUP_COMPLETE_SUFFIX: &str = "-blank-slate-original-preserved";
const BLANK_SLATE_FAILED_CANDIDATE_SUFFIX: &str = "-blank-slate-failed-candidate";
static DIRECT_SELECTION_WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DirectSelectionReceipt {
    schema_version: u32,
    graph_root: String,
    reason: String,
}

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

    fn validate_for(&self, graph_root: &Path) -> Result<(), crate::command_error::CommandError> {
        if self.schema_version != BINDING_SCHEMA_VERSION {
            return Err(crate::command_error::CommandError::prose(
                "Tine-managed storage has an unsupported local setup version.",
            ));
        }
        if self.graph_root != graph_root.display().to_string()
            || self.graph_meta.root != self.graph_root
        {
            return Err(crate::command_error::CommandError::prose(
                "Tine-managed storage data belongs to a different graph.",
            ));
        }
        Ok(())
    }

    fn private_root(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<PathBuf, crate::command_error::CommandError> {
        sparse_private_root(app, Path::new(&self.graph_root))
    }

    fn open_request(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<SyncRuntimeOpenRequest, crate::command_error::CommandError> {
        let private = self.private_root(app)?;
        Ok(self.open_request_at(&private))
    }

    fn open_request_at(&self, private: &Path) -> SyncRuntimeOpenRequest {
        SyncRuntimeOpenRequest {
            profile: SyncStorageProfile::ExperimentalLocal,
            clean_identities: Some(SyncLocalActivationIdentities {
                workspace_id: self.workspace_id,
                lineage_digest: self.lineage_digest,
                catalog_document_id: self.catalog_document_id,
                endpoint_id: self.endpoint_id,
                device_id: self.device_id,
                preparation_id: self.preparation_id,
                session_id: self.activation_session_id,
            }),
            graph_root: PathBuf::from(&self.graph_root),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
            provider_root: PathBuf::from(&self.graph_root).join(".tine-sync/v2/shared"),
            provider_journal_root: private.join("provider/device/journal"),
        }
    }

    fn activation_request(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<SyncLocalActivationRequest, crate::command_error::CommandError> {
        let private = self.private_root(app)?;
        Ok(self.activation_request_at(&private))
    }

    fn activation_request_at(&self, private: &Path) -> SyncLocalActivationRequest {
        SyncLocalActivationRequest {
            graph_root: PathBuf::from(&self.graph_root),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
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
        }
    }
}

fn graph_private_key(graph_root: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tine/sparse-v2/app-binding/v1\0");
    digest.update(graph_root.as_os_str().as_encoded_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn sparse_private_root(
    app: &tauri::AppHandle,
    graph_root: &Path,
) -> Result<PathBuf, crate::command_error::CommandError> {
    let app_data = app.path().app_data_dir().map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "couldn't resolve private app-data directory: {error}"
        ))
    })?;
    Ok(app_data
        .join(SPARSE_BINDING_DIR)
        .join(graph_private_key(graph_root)))
}

fn direct_selection_path(
    app: &tauri::AppHandle,
    graph_root: &Path,
) -> Result<PathBuf, crate::command_error::CommandError> {
    let app_data = app.path().app_data_dir().map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "couldn't resolve private app-data directory: {error}"
        ))
    })?;
    Ok(direct_selection_path_at(&app_data, graph_root))
}

fn direct_selection_path_at(app_data: &Path, graph_root: &Path) -> PathBuf {
    app_data.join(DIRECT_SELECTION_DIR).join(format!(
        "{}{}",
        graph_private_key(graph_root),
        DIRECT_SELECTION_FILE_SUFFIX
    ))
}

fn direct_selection_is_active(
    app: &tauri::AppHandle,
    graph_root: &Path,
) -> Result<bool, crate::command_error::CommandError> {
    let path = direct_selection_path(app, graph_root)?;
    direct_selection_is_active_at(&path, graph_root)
}

fn direct_selection_is_active_at(
    path: &Path,
    graph_root: &Path,
) -> Result<bool, crate::command_error::CommandError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't read Direct Files selection: {error}"
            )))
        }
    };
    match serde_json::from_slice::<DirectSelectionReceipt>(&bytes) {
        Ok(receipt)
            if receipt.schema_version == DIRECT_SELECTION_SCHEMA_VERSION
                && Path::new(&receipt.graph_root) == graph_root =>
        {
            Ok(true)
        }
        Ok(_) | Err(_) => {
            // The digest-addressed receipt can only have been written for this
            // canonical root. A torn/corrupt app-private receipt fails toward
            // Direct Files; it must never resurrect the managed selector.
            crate::debug::diag(format!(
                "Direct Files selection receipt is malformed at {}; retaining Direct selection",
                path.display()
            ));
            Ok(true)
        }
    }
}

fn direct_selection_requests_blank_slate_rebuild_at(path: &Path, graph_root: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    matches!(
        serde_json::from_slice::<DirectSelectionReceipt>(&bytes),
        Ok(receipt)
            if receipt.schema_version == DIRECT_SELECTION_SCHEMA_VERSION
                && Path::new(&receipt.graph_root) == graph_root
                && receipt.reason == BLANK_SLATE_REBUILD_REASON
    )
}

fn publish_direct_selection(
    app: &tauri::AppHandle,
    graph_root: &Path,
    reason: &str,
) -> Result<(), crate::command_error::CommandError> {
    let path = direct_selection_path(app, graph_root)?;
    publish_direct_selection_at(&path, graph_root, reason)
}

fn publish_direct_selection_at(
    path: &Path,
    graph_root: &Path,
    reason: &str,
) -> Result<(), crate::command_error::CommandError> {
    let receipt = DirectSelectionReceipt {
        schema_version: DIRECT_SELECTION_SCHEMA_VERSION,
        graph_root: graph_root.display().to_string(),
        reason: reason.to_owned(),
    };
    let encoded = serde_json::to_string_pretty(&receipt)
        .map(|mut value| {
            value.push('\n');
            value
        })
        .map_err(crate::command_error::CommandError::from)?;
    tine_core::model::durable_private_authority_update(path, &DIRECT_SELECTION_WRITE, |_| {
        Ok(encoded.clone())
    })
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't select Direct Files for this graph: {error}"
        ))
    })
}

fn retire_direct_selection(
    app: &tauri::AppHandle,
    graph_root: &Path,
) -> Result<(), crate::command_error::CommandError> {
    let path = direct_selection_path(app, graph_root)?;
    retire_direct_selection_at(&path)
}

fn retire_direct_selection_at(path: &Path) -> Result<(), crate::command_error::CommandError> {
    tine_core::model::durable_private_authority_retire(path, &DIRECT_SELECTION_WRITE).map_err(
        |error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't retire the prior Direct Files selection: {error}"
            ))
        },
    )
}

fn binding_path(
    app: &tauri::AppHandle,
    graph_root: &Path,
) -> Result<PathBuf, crate::command_error::CommandError> {
    Ok(sparse_private_root(app, graph_root)?.join(SPARSE_BINDING_FILE))
}

fn sparse_recovery_root(
    app: &tauri::AppHandle,
) -> Result<PathBuf, crate::command_error::CommandError> {
    app.path()
        .app_data_dir()
        .map(|root| root.join(SPARSE_RECOVERY_DIR))
        .map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "couldn't resolve private app-data directory: {error}"
            ))
        })
}

fn read_binding_at(
    path: &Path,
    graph_root: &Path,
) -> Result<Option<SparseV2ActivationRecord>, crate::command_error::CommandError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't read Tine-managed storage data: {error}"
            )));
        }
    };
    let record: SparseV2ActivationRecord = serde_json::from_slice(&bytes).map_err(|error| {
        crate::command_error::CommandError::json(format!(
            "Tine-managed storage data is corrupted: {error}"
        ))
    })?;
    record.validate_for(graph_root)?;
    Ok(Some(record))
}

fn persist_binding_at(
    path: &Path,
    record: &SparseV2ActivationRecord,
) -> Result<(), crate::command_error::CommandError> {
    let encoded = serde_json::to_string_pretty(record)
        .map(|mut value| {
            value.push('\n');
            value
        })
        .map_err(crate::command_error::CommandError::from)?;
    tine_core::model::durable_private_authority_update(path, &BINDING_WRITE, |existing| {
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
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't save Tine-managed storage setup: {error}"
        ))
    })
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
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
        scenario_id: String,
    },
    Refused {
        reason_code: String,
        scenario_id: String,
        detail: Option<String>,
    },
}

impl SparseV2Availability {
    fn from_open(status: SyncRuntimeOpenStatus) -> Self {
        let scenario = status.durable_refusal_scenario();
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
            SyncRuntimeOpenStatus::Blocked { reason_code } => Self::Blocked {
                reason_code,
                scenario_id: required_scenario_id(scenario),
            },
            SyncRuntimeOpenStatus::UnsupportedOrIncompatible { component, .. } => Self::Refused {
                reason_code: format!("unsupported_{}", component_name(component)),
                scenario_id: required_scenario_id(scenario),
                detail: None,
            },
            SyncRuntimeOpenStatus::CorruptOrUnreadable { component, .. } => Self::Refused {
                reason_code: format!("corrupt_{}", component_name(component)),
                scenario_id: required_scenario_id(scenario),
                detail: None,
            },
            SyncRuntimeOpenStatus::AmbiguousOrForeignResidue { evidence, .. } => Self::Refused {
                reason_code: format!("ambiguous_{}", ambiguous_name(evidence)),
                scenario_id: required_scenario_id(scenario),
                detail: None,
            },
            SyncRuntimeOpenStatus::OpenRefused { detail } => match scenario {
                Some(scenario) => Self::Refused {
                    reason_code: "open_refused".into(),
                    scenario_id: scenario.as_str().into(),
                    detail: Some(detail),
                },
                None => Self::Retryable {
                    stage: "local_active".into(),
                    detail,
                },
            },
            // Not a durable refusal: the startup path retires the state and
            // reopens the graph in Direct Files. This mapping only surfaces if
            // a non-startup caller races the retirement.
            SyncRuntimeOpenStatus::SupersededLegacyState => Self::Refused {
                reason_code: SUPERSEDED_LEGACY_REASON.into(),
                scenario_id: SUPERSEDED_LEGACY_REASON.into(),
                detail: Some(
                    "Pre-0.7 managed-storage state was found; it is set aside and the graph opens in Direct files."
                        .into(),
                ),
            },
        }
    }

    fn from_activation(status: SyncLocalActivationStatus) -> Self {
        let scenario = status.durable_refusal_scenario();
        match status {
            SyncLocalActivationStatus::Active => Self::Active,
            SyncLocalActivationStatus::Retryable {
                durable_stage,
                detail,
            } => Self::Retryable {
                stage: activation_stage(durable_stage).into(),
                detail,
            },
            SyncLocalActivationStatus::Blocked { reason_code } => Self::Blocked {
                reason_code,
                scenario_id: required_scenario_id(scenario),
            },
            SyncLocalActivationStatus::UnsupportedOrIncompatible { component, .. } => {
                Self::Refused {
                    reason_code: format!("unsupported_{}", component_name(component)),
                    scenario_id: required_scenario_id(scenario),
                    detail: None,
                }
            }
            SyncLocalActivationStatus::CorruptOrUnreadable { component, .. } => Self::Refused {
                reason_code: format!("corrupt_{}", component_name(component)),
                scenario_id: required_scenario_id(scenario),
                detail: None,
            },
            SyncLocalActivationStatus::AmbiguousOrForeignResidue { evidence, .. } => {
                Self::Refused {
                    reason_code: format!("ambiguous_{}", ambiguous_name(evidence)),
                    scenario_id: required_scenario_id(scenario),
                    detail: None,
                }
            }
        }
    }
}

fn required_scenario_id(scenario: Option<ManagedStorageRefusalScenario>) -> String {
    scenario
        .expect("every durable managed-storage refusal has a contract scenario")
        .as_str()
        .into()
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

/// Reason code for pre-0.7 managed state discovered at open. The graph-open
/// path treats it as an instruction (set aside, reopen Direct), never as an
/// error to show.
pub(crate) const SUPERSEDED_LEGACY_REASON: &str = "superseded_legacy_state";

pub(crate) struct SparseV2Binding {
    availability: SparseV2Availability,
    handle: Option<SyncRuntimeHandle>,
}

impl SparseV2Binding {
    /// Pre-0.7 private state is disposable authority, but never disposable
    /// evidence.  The graph-open boundary archives it and reconstructs the
    /// one current format from the Markdown/Org tree.
    pub(crate) fn requires_blank_slate_rebuild(&self) -> bool {
        matches!(
            &self.availability,
            SparseV2Availability::Refused { reason_code, .. }
                if reason_code == SUPERSEDED_LEGACY_REASON
        ) || matches!(
            &self.availability,
            SparseV2Availability::Refused { scenario_id, .. }
                if scenario_id == ManagedStorageRefusalScenario::ProtocolIncompatible.as_str()
        )
    }
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

fn runtime_status_admits_application_pages(status: &SyncRuntimeStatusSnapshot) -> bool {
    runtime_observation_admits_application_pages(
        &status.lifecycle,
        status.application_pages_writable,
    )
}

fn runtime_observation_admits_application_pages(
    lifecycle: &SyncRuntimeLifecycle,
    application_pages_writable: bool,
) -> bool {
    matches!(lifecycle, SyncRuntimeLifecycle::Active) && application_pages_writable
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

    /// The frontend may preflight a bulk page write only while the retained
    /// runtime has positively reported that it is still accepting work.  A
    /// stopped or terminal handle deliberately remains retained for recovery,
    /// so handle presence is not application-write authority.
    pub(crate) fn has_active_application_handle(&self) -> bool {
        self.handle
            .as_ref()
            .and_then(|handle| handle.status().ok())
            .is_some_and(|snapshot| runtime_status_admits_application_pages(&snapshot))
    }

    /// Explain why this binding cannot be published as the application's page
    /// authority. Core open returns typed status separately from its optional
    /// actor handle; collapsing those two facts into a GraphSlot used to hide
    /// the real OpenRefused/Retryable detail behind SPARSE_V2_NOT_ACTIVE.
    pub(crate) fn serving_failure_detail(&self) -> Option<String> {
        if let Some(handle) = &self.handle {
            return match handle.status() {
                Ok(snapshot) if runtime_status_admits_application_pages(&snapshot) => None,
                Ok(snapshot) => Some(snapshot.detail.unwrap_or_else(|| {
                    format!(
                        "managed storage runtime is not serving pages (lifecycle: {:?})",
                        snapshot.lifecycle
                    )
                })),
                Err(error) => Some(format!("managed storage runtime status failed: {error}")),
            };
        }
        Some(match &self.availability {
            SparseV2Availability::Retryable { detail, .. } => detail.clone(),
            SparseV2Availability::Blocked {
                reason_code,
                scenario_id,
            } => format!(
                "managed storage open was blocked (reason code: {reason_code}; scenario: {scenario_id})"
            ),
            SparseV2Availability::Refused {
                reason_code,
                scenario_id,
                detail,
            } => detail.clone().unwrap_or_else(|| {
                format!(
                    "managed storage open was refused (reason code: {reason_code}; scenario: {scenario_id})"
                )
            }),
            SparseV2Availability::LegacyDefault => {
                "managed storage setup is not present for this graph".into()
            }
            SparseV2Availability::Joinable { .. } => {
                "managed storage must be joined before it can serve pages".into()
            }
            SparseV2Availability::Active => {
                "managed storage open reported active without a serving actor".into()
            }
        })
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

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
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

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct SparseV2TickDto {
    state: String,
    detail: Option<String>,
    epoch: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct SparseV2RuntimeStatusDto {
    lifecycle: String,
    recovery: Option<String>,
    watcher: SparseV2WatcherStatusDto,
    last_tick: Option<SparseV2TickDto>,
    detail: Option<String>,
    shared_role: Option<String>,
    shared_phase: Option<String>,
    provider_pending: usize,
    /// The actor's own scheduling predicate: shared-active AND holding provider
    /// work a tick can advance. `provider_pending` alone is ambiguous — it is a
    /// broad protocol inventory that legitimately stays non-zero — so a capsule
    /// showing `provider_pending > 0` with an idle watcher cannot be read
    /// without this. See `docs/storage-sync-contract.md` §2.3.
    provider_runnable: bool,
    search_index_building: bool,
    managed_local_pending: usize,
    managed_local_checkpointed_sequence: u64,
    managed_local_next_sequence: u64,
    managed_local_stage: Option<String>,
    history_recovery: Option<SyncHistoryRecoveryStatus>,
    checkpoint_diagnostics: Option<SyncCheckpointPublicationDiagnostics>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct SparseV2StatusDto {
    #[serde(flatten)]
    availability: SparseV2Availability,
    runtime: Option<SparseV2RuntimeStatusDto>,
    can_activate: bool,
    can_retry: bool,
    can_cancel: bool,
    cancel_reason: Option<String>,
    binding_generation: u64,
    application_page_admission: crate::state::ApplicationPageAdmission,
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
            application_page_admission: crate::state::ApplicationPageAdmission::direct(
                binding_generation,
            ),
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
            // A joinable descriptor is discovered while this GraphSlot still
            // writes through Direct Files. `sparse_v2_status_for_slot` replaces
            // this from the actual slot as a single final step.
            application_page_admission: crate::state::ApplicationPageAdmission::direct(
                binding_generation,
            ),
        }
    }

    pub(crate) fn from_binding(binding: &SparseV2Binding, binding_generation: u64) -> Self {
        let retained_status = binding.handle().map(SyncRuntimeHandle::status);
        let application_page_admission = retained_status
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map_or_else(
                || crate::state::ApplicationPageAdmission::managed_unavailable(binding_generation),
                |snapshot| {
                    crate::state::ApplicationPageAdmission::from_managed_runtime_status(
                        binding_generation,
                        snapshot,
                    )
                },
            );
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
            application_page_admission,
        }
    }

    fn from_binding_observation(
        binding: &SparseV2Binding,
        binding_generation: u64,
        snapshot: SyncRuntimeStatusSnapshot,
    ) -> Self {
        let availability = match snapshot.lifecycle {
            SyncRuntimeLifecycle::StoppedSafe => SparseV2Availability::Retryable {
                stage: "local_active".into(),
                detail: "Tine-managed storage stopped safely and needs to be reopened.".into(),
            },
            SyncRuntimeLifecycle::StoppedCrashed => SparseV2Availability::Retryable {
                stage: "local_active".into(),
                detail: "Tine-managed storage needs to be reopened after it stopped unexpectedly."
                    .into(),
            },
            SyncRuntimeLifecycle::Active | SyncRuntimeLifecycle::Terminal => {
                binding.availability().clone()
            }
        };
        let can_retry = matches!(availability, SparseV2Availability::Retryable { .. });
        let application_page_admission =
            crate::state::ApplicationPageAdmission::from_managed_runtime_status(
                binding_generation,
                &snapshot,
            );
        Self {
            availability,
            runtime: Some(runtime_status(snapshot)),
            can_activate: false,
            can_retry,
            can_cancel: false,
            cancel_reason: None,
            binding_generation,
            application_page_admission,
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
                SyncRuntimeRecovery::CleanActivation => "clean_activation",
                SyncRuntimeRecovery::CleanManifestReplay => "clean_manifest_replay",
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
        provider_runnable: snapshot.provider_runnable,
        search_index_building: snapshot.search_index_building,
        managed_local_pending: snapshot.managed_local_pending,
        managed_local_checkpointed_sequence: snapshot.managed_local_checkpointed_sequence,
        managed_local_next_sequence: snapshot.managed_local_next_sequence,
        managed_local_stage: snapshot.managed_local_stage,
        history_recovery: snapshot.history_recovery,
        checkpoint_diagnostics: snapshot.checkpoint_diagnostics,
    }
}

pub(crate) fn tick_dto(tick: SyncRuntimeTick) -> SparseV2TickDto {
    match tick {
        SyncRuntimeTick::Idle => tick_value("idle", None, None),
        SyncRuntimeTick::CheckpointCaptureSkipped { .. } => {
            tick_value("checkpoint_capture_skipped", None, None)
        }
        SyncRuntimeTick::LocalMutation(outcome) => {
            tick_value("local_mutation", Some(format!("{outcome:?}")), None)
        }
        SyncRuntimeTick::ProviderMutation { batch_id } => {
            tick_value("provider_mutation", Some(batch_id.to_string()), None)
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

/// What the graph-local provider namespace proves for the narrowly scoped
/// "Return to Direct files" escape hatch.
///
/// A first local activation writes NOTHING under `.tine-sync/` — it is
/// write-shy about the graph folder until the user asks to share, and
/// `tine_core::sync_runtime::tests::local_activation_writes_nothing_into_the_graphs_sync_folder`
/// holds it to that. The empty namespace skeleton is written by the SHARED
/// TRANSPORT (`ProviderRuntime::open`), which runs when a share is prepared or
/// joined and on every shared reopen — always before any descriptor or
/// publication exists. A file-sync client can also deliver that skeleton to a
/// second device ahead of its contents.
///
/// So an exactly-empty skeleton still proves what this check needs — no other
/// device can depend on this graph through it — but not for the reason this
/// comment used to give. A descriptor, provider work, or anything that does not
/// exactly match the empty skeleton is treated as shared/unknown and remains
/// fail-closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderNamespaceEvidence {
    LocalOnly,
    SharedOrUnknown,
}

const PROVIDER_SCAFFOLD_TREES: [&str; 2] = [PROVIDER_INBOX_DIR, PROVIDER_OUTBOX_DIR];
/// Exactly what the shared provider transport writes when it opens a tree —
/// taken from the core that writes it, never re-listed here. A hand-copied list
/// drifted the moment clean baselines were added: the transport wrote eleven
/// namespaces while this check still expected ten, so a tree that had only ever
/// been opened failed to match its own skeleton and the escape hatch warned the
/// user their graph might be shared with another device.
const PROVIDER_SCAFFOLD_NAMESPACES: [&str; 8] = SHARED_PROVIDER_TREE_NAMESPACES;

fn sorted_directory_entries(
    path: &Path,
) -> Result<Vec<std::fs::DirEntry>, crate::command_error::CommandError> {
    let mut entries = std::fs::read_dir(path)
        .map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect sync data: {error}"
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect sync data: {error}"
            ))
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn has_exact_directory_names(entries: &[std::fs::DirEntry], expected: &[&str]) -> bool {
    entries.len() == expected.len()
        && expected.iter().all(|expected| {
            entries
                .iter()
                .any(|entry| entry.file_name().to_string_lossy() == *expected)
        })
}

/// Is this tree exactly what opening a shared provider transport leaves, with
/// nothing published into it? Not "what activation writes" — activation writes
/// nothing here at all.
fn is_empty_local_provider_scaffold(
    shared_root: &Path,
) -> Result<bool, crate::command_error::CommandError> {
    let root_entries = sorted_directory_entries(shared_root)?;
    if !has_exact_directory_names(&root_entries, &PROVIDER_SCAFFOLD_TREES) {
        return Ok(false);
    }

    for tree in root_entries {
        let file_type = tree.file_type().map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect sync data: {error}"
            ))
        })?;
        if !file_type.is_dir() || file_type.is_symlink() {
            return Ok(false);
        }
        let namespaces = sorted_directory_entries(&tree.path())?;
        if !has_exact_directory_names(&namespaces, &PROVIDER_SCAFFOLD_NAMESPACES) {
            return Ok(false);
        }
        for namespace in namespaces {
            let file_type = namespace.file_type().map_err(|error| {
                crate::command_error::CommandError::sync_runtime(format!(
                    "Couldn't inspect sync data: {error}"
                ))
            })?;
            if !file_type.is_dir()
                || file_type.is_symlink()
                || !sorted_directory_entries(&namespace.path())?.is_empty()
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn provider_namespace_evidence(
    path: &Path,
) -> Result<ProviderNamespaceEvidence, crate::command_error::CommandError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProviderNamespaceEvidence::LocalOnly);
        }
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect sync data: {error}"
            )))
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(ProviderNamespaceEvidence::SharedOrUnknown);
    }

    let entries = sorted_directory_entries(path)?;
    if entries.is_empty() {
        return Ok(ProviderNamespaceEvidence::LocalOnly);
    }
    if entries.len() != 1 || entries[0].file_name() != "shared" {
        return Ok(ProviderNamespaceEvidence::SharedOrUnknown);
    }
    let shared = &entries[0];
    let file_type = shared.file_type().map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't inspect sync data: {error}"
        ))
    })?;
    if !file_type.is_dir() || file_type.is_symlink() {
        return Ok(ProviderNamespaceEvidence::SharedOrUnknown);
    }

    match inspect_shared_enrollment_for_cold_discovery(&shared.path()) {
        Ok(Some(_)) => Ok(ProviderNamespaceEvidence::SharedOrUnknown),
        // A canonical empty provider topology is what opening the shared
        // transport leaves, before any authority/share publication.  Any other
        // descriptor-inspection failure remains evidence, rather than being
        // guessed to be local-only.
        Ok(None) | Err(_) if is_empty_local_provider_scaffold(&shared.path())? => {
            Ok(ProviderNamespaceEvidence::LocalOnly)
        }
        Ok(None) | Err(_) => Ok(ProviderNamespaceEvidence::SharedOrUnknown),
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

/// A shared or malformed provider namespace is a warning for the explicit
/// archive-and-return action, not a reason to strand the user in a refused
/// managed runtime.  The action preserves the complete private managed state
/// before Direct Files is installed, so the user can later inspect or recover
/// it.  The frontend requires an acknowledgement before invoking the command.
fn cancel_warning(binding: &SparseV2Binding, provider_namespace: &Path) -> Option<String> {
    let provider_evidence = provider_namespace_evidence(provider_namespace)
        .unwrap_or(ProviderNamespaceEvidence::SharedOrUnknown);
    let mut shared_or_unknown = binding_names_shared_state(binding)
        || provider_evidence == ProviderNamespaceEvidence::SharedOrUnknown;
    if let Some(handle) = binding.handle() {
        match handle.status() {
            Ok(status) => {
                shared_or_unknown |= status.shared_role.is_some()
                    || status.shared_phase.is_some()
                    // A local-only snapshot can retain its absent-provider
                    // sentinel.  Pending work matters here only when provider
                    // evidence says this is not the exact local scaffold.
                    || (status.provider_pending != 0
                        && provider_evidence == ProviderNamespaceEvidence::SharedOrUnknown);
            }
            // The warning is deliberately conservative, but inability to
            // inspect an already-refused runtime must not suppress the escape.
            Err(_) => shared_or_unknown = true,
        }
    }
    shared_or_unknown.then(|| {
        "Managed storage may contain shared or pending provider state. Tine will archive the complete private managed-storage state before reopening this graph with Direct files. Other devices will not receive further managed-storage updates from this device.".into()
    })
}

fn cancel_warning_for_observation(
    binding: &SparseV2Binding,
    provider_namespace: &Path,
    snapshot: &SyncRuntimeStatusSnapshot,
) -> Option<String> {
    let provider_evidence = provider_namespace_evidence(provider_namespace)
        .unwrap_or(ProviderNamespaceEvidence::SharedOrUnknown);
    let shared_or_unknown = binding_names_shared_state(binding)
        || provider_evidence == ProviderNamespaceEvidence::SharedOrUnknown
        || snapshot.shared_role.is_some()
        || snapshot.shared_phase.is_some()
        || (snapshot.provider_pending != 0
            && provider_evidence == ProviderNamespaceEvidence::SharedOrUnknown);
    shared_or_unknown.then(|| {
        "Managed storage may contain shared or pending provider state. Tine will archive the complete private managed-storage state before reopening this graph with Direct files. Other devices will not receive further managed-storage updates from this device.".into()
    })
}

#[derive(Default)]
pub(crate) struct SyncRuntimeFacade;

/// The sole ordinary graph-open decision for storage authority.
///
/// A validated private binding is the durable, device-local proof that Martin
/// explicitly enabled managed storage for this graph. Its absence means Direct
/// Files. In particular, graph-local `.tine-sync` bytes are never consulted to
/// infer or force managed mode; they are inspected only after the user invokes
/// the explicit join flow.
pub(crate) enum ExplicitStorageSelection {
    DirectFiles,
    Managed(SparseV2ActivationRecord),
}

impl SyncRuntimeFacade {
    pub(crate) fn explicit_storage_selection(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<ExplicitStorageSelection, crate::command_error::CommandError> {
        Ok(match self.binding_record(app, graph_root)? {
            Some(record) => ExplicitStorageSelection::Managed(record),
            None => ExplicitStorageSelection::DirectFiles,
        })
    }

    pub(crate) fn binding_record(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<Option<SparseV2ActivationRecord>, crate::command_error::CommandError> {
        if direct_selection_is_active(app, graph_root)? {
            return Ok(None);
        }
        let private = sparse_private_root(app, graph_root)?;
        let record = read_binding_at(&private.join(SPARSE_BINDING_FILE), graph_root)?;
        // Candidate construction is deliberately private and disposable until
        // the selector is published. A crash may therefore leave candidate
        // bytes without a binding record; absence of that record means Direct
        // Files, and the next explicit activation quarantines the residue.
        Ok(record)
    }

    /// True when a binding file exists and is safely readable, but ordinary
    /// discovery could not decode it as the one current pre-0.7 format.
    /// Startup archives the entire private root and reconstructs current state
    /// from Markdown/Org. An I/O failure remains a loud error: it is not proof
    /// that the format itself is unrecognized.
    pub(crate) fn unrecognized_binding_file(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> bool {
        let Ok(private) = sparse_private_root(app, graph_root) else {
            return false;
        };
        let Ok(bytes) = std::fs::read(private.join(SPARSE_BINDING_FILE)) else {
            return false;
        };
        match serde_json::from_slice::<SparseV2ActivationRecord>(&bytes) {
            Ok(record) => record.validate_for(graph_root).is_err(),
            Err(_) => true,
        }
    }

    /// Pre-0.7 policy: experimental state is not migrated. Archive the whole
    /// app-private root into the recovery directory - never hard-delete - and
    /// select Direct Files as the safe source for an automatic current-format
    /// rebuild from Markdown/Org.
    pub(crate) fn archive_unrecognized_private_state(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<(), crate::command_error::CommandError> {
        let private = sparse_private_root(app, graph_root)?;
        let recovery = sparse_recovery_root(app)?;
        clear_blank_slate_backup_complete(&private, &recovery)?;
        // Publish the durable reconstruction intent before moving private
        // state. A crash at any later cut therefore reopens the Markdown/Org
        // tree in Direct Files and retries the automatic rebuild.
        publish_direct_selection(app, graph_root, BLANK_SLATE_REBUILD_REASON)?;
        let archived = archive_private_root(&private, &recovery).map_err(|error| {
            crate::command_error::CommandError::sync_runtime({
                format!("Couldn't set aside pre-0.7 managed-storage state: {error}")
            })
        })?;
        mark_blank_slate_backup_complete(&private, &recovery)?;
        crate::debug::diag(format!(
            "unrecognized pre-0.7 managed state archived: archived={}",
            archived
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "nothing-to-archive".into())
        ));
        Ok(())
    }

    pub(crate) fn prepare_binding_record(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
        graph_meta: GraphMeta,
    ) -> Result<SparseV2ActivationRecord, crate::command_error::CommandError> {
        let direct_selected = direct_selection_is_active(app, graph_root)?;
        let blank_slate_rebuild = self.blank_slate_rebuild_pending(app, graph_root)?;
        if !direct_selected {
            if let Some(record) = self.binding_record(app, graph_root)? {
                return Ok(record);
            }
        }
        let private = sparse_private_root(app, graph_root)?;
        let recovery = sparse_recovery_root(app)?;
        let record = SparseV2ActivationRecord::new(
            graph_root,
            graph_meta,
            DeviceId::from_uuid(crate::settings::managed_sync_device_id(app)?),
        );
        if blank_slate_rebuild {
            prepare_blank_slate_retry_at_paths(&private, &recovery, record)
        } else {
            prepare_fresh_authority_at_paths(&private, &recovery, record, "fresh activation")
        }
    }

    pub(crate) fn prepare_shared_binding_record(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
        graph_meta: GraphMeta,
        descriptor: &SyncSharedEnrollmentDescriptor,
    ) -> Result<SparseV2ActivationRecord, crate::command_error::CommandError> {
        let private = sparse_private_root(app, graph_root)?;
        let recovery = sparse_recovery_root(app)?;
        prepare_shared_binding_record_at_paths(
            &private,
            &recovery,
            graph_root,
            graph_meta,
            DeviceId::from_uuid(crate::settings::managed_sync_device_id(app)?),
            descriptor,
        )
    }

    pub(crate) fn persist_binding_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<(), crate::command_error::CommandError> {
        let root = Path::new(&record.graph_root);
        persist_binding_at(&binding_path(app, root)?, record)?;
        retire_direct_selection(app, root)
    }

    pub(crate) fn direct_selection_is_active(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<bool, crate::command_error::CommandError> {
        direct_selection_is_active(app, graph_root)
    }

    pub(crate) fn blank_slate_rebuild_pending(
        &self,
        app: &tauri::AppHandle,
        graph_root: &Path,
    ) -> Result<bool, crate::command_error::CommandError> {
        let path = direct_selection_path(app, graph_root)?;
        Ok(direct_selection_requests_blank_slate_rebuild_at(
            &path, graph_root,
        ))
    }

    pub(crate) fn graph_meta(record: &SparseV2ActivationRecord) -> GraphMeta {
        record.graph_meta.clone()
    }

    pub(crate) fn open_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
        self.open_record_with_progress(app, record, |_| {})
    }

    /// Open a retained managed runtime while forwarding only bounded phase
    /// names and elapsed time to the caller.  Detailed recovery telemetry
    /// remains in the terminal diagnostic stream; it must not be copied into a
    /// startup webview before a graph slot exists.
    fn open_record_with_progress(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
        mut progress: impl FnMut(SyncRuntimeOpenProgress),
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
        crate::debug::diag("managed storage open: begin authenticated existing-state recovery");
        let opened = SyncRuntimeHandle::open_with_progress(record.open_request(app)?, |update| {
            match &update {
                SyncRuntimeOpenProgress::Phase { phase, elapsed } => crate::debug::diag(format!(
                    "managed storage open: phase={} elapsed_ms={}",
                    managed_open_phase_name(*phase),
                    elapsed.as_millis()
                )),
                SyncRuntimeOpenProgress::Waiting { phase, elapsed } => crate::debug::diag(format!(
                    "managed storage open: phase={} elapsed_ms={}",
                    managed_open_waiting_phase_name(*phase),
                    elapsed.as_millis()
                )),
                SyncRuntimeOpenProgress::RecoveryStage { stage, elapsed } => {
                    crate::debug::diag(format!(
                        "managed storage open: recovery_stage={} elapsed_ms={}",
                        stage.diagnostic_name(),
                        elapsed.as_millis()
                    ))
                }
                SyncRuntimeOpenProgress::RecoveryDiagnostics { diagnostics } => {
                    crate::debug::diag(format!(
                            "managed storage open: recovery={} retention={} retained_runs={} resume_candidate={} detached_bootstrap_reconstruction={} full_bootstrap_replay={} manifests={} manifest_enumeration_ms={} resume_selection_ms={} bootstrap_reconstruction_attempted={} bootstrap_reconstruction_ms={} engine_open_ms={} sqlite_open_ms={} tail_construction_ms={} total_ms={}",
                            diagnostics.recovery,
                            diagnostics.retention_plan,
                            diagnostics.retained_run_count,
                            diagnostics.resume_candidate,
                            u8::from(diagnostics.detached_bootstrap_reconstruction),
                            u8::from(diagnostics.full_bootstrap_replay),
                            diagnostics.manifest_count,
                            diagnostics.manifest_enumeration.as_millis(),
                            diagnostics.resume_selection.as_millis(),
                            u8::from(diagnostics.bootstrap_reconstruction.is_some()),
                            diagnostics.bootstrap_reconstruction.unwrap_or_default().as_millis(),
                            diagnostics.engine_open.as_millis(),
                            diagnostics.sqlite_open.as_millis(),
                            diagnostics.tail_construction.as_millis(),
                            diagnostics.total.as_millis(),
                        ));
                    crate::debug::diag(format!(
                            "managed storage open: projection recovery={} sidecar_shape_ms={} checkpoint_auth_ms={} read_only_open_ms={} schema_claim_ms={} structural_ms={} materialization_stamp_ms={} forensics_ms={} rebuild_ms={} applied_batches={} bulk_pages_materialized={} ancestry_full_scans={}",
                            diagnostics.projection_recovery,
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
                SyncRuntimeOpenProgress::CleanOpenCounters { counters } => {
                    crate::debug::diag(format!(
                            "managed storage open: clean open counters accepted_batches={} committed_tail_replayed={} sweep_chains={} receipt_evidence_names={} receipt_content_reads={} receipt_full_catalog_passes={} summary_content_reads={} summary_rebuilt={} summary_delta_completions={} summary_delta_intents={} local_completion_names={} local_completion_content_reads={} local_completion_rebuilt={} local_completion_entries={} retired_own_intent_probes={} retired_own_receipt_artifacts={} archive_directory_enumerations={} archive_manifest_reads={} archive_object_reads={} archive_inspected_manifests={} archive_inspected_objects={}",
                            counters.accepted_batches,
                            counters.committed_tail_replayed,
                            counters.sweep_chains,
                            counters.receipt_evidence_names,
                            counters.receipt_content_reads,
                            counters.receipt_full_catalog_passes,
                            counters.summary_content_reads,
                            counters.summary_rebuilt,
                            counters.summary_delta_completions,
                            counters.summary_delta_intents,
                            counters.local_completion_names,
                            counters.local_completion_content_reads,
                            counters.local_completion_rebuilt,
                            counters.local_completion_entries,
                            counters.retired_own_intent_probes,
                            counters.retired_own_receipt_artifacts,
                            counters.archive_directory_enumerations,
                            counters.archive_manifest_reads,
                            counters.archive_object_reads,
                            counters.archive_inspected_manifests,
                            counters.archive_inspected_objects,
                        ));
                }
            }
            progress(update);
        });
        crate::debug::diag(format!(
            "managed storage open: completed outcome={}",
            managed_open_outcome_code(&opened.status)
        ));
        Ok(SparseV2Binding::from_open(opened))
    }

    /// The startup graph-open path has no `GraphSlot` until this operation
    /// succeeds.  Publish the same bounded progress vocabulary directly to
    /// that webview so a long recovery is visible without granting it any
    /// additional storage authority.
    pub(crate) fn open_record_for_window(
        &self,
        app: &tauri::AppHandle,
        _label: &str,
        record: &SparseV2ActivationRecord,
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
        // The supervisor owns the public typed operation. Detailed engine
        // phases remain native diagnostics and must not create a second
        // frontend recovery state machine.
        self.open_record_with_progress(app, record, |_| {})
    }

    pub(crate) fn activate_record(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
        self.activate_record_with_progress(app, record, |_| {})
    }

    pub(crate) fn activate_record_with_progress(
        &self,
        app: &tauri::AppHandle,
        record: &SparseV2ActivationRecord,
        progress: impl FnMut(SyncLocalActivationPhase),
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
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
    ) -> Result<SparseV2Binding, crate::command_error::CommandError> {
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

fn managed_open_phase_name(phase: SyncRuntimeOpenPhase) -> &'static str {
    match phase {
        SyncRuntimeOpenPhase::RetainingGraph => "managed_open.retaining_graph",
        SyncRuntimeOpenPhase::RecoveringCleanManifestRuntime => {
            "managed_open.recovering_clean_manifest_runtime"
        }
        SyncRuntimeOpenPhase::DiscoveringEnrollment => "managed_open.discovering_enrollment",
        SyncRuntimeOpenPhase::OpeningActorGraph => "managed_open.opening_actor_graph",
        SyncRuntimeOpenPhase::RevalidatingEnrollment => "managed_open.revalidating_enrollment",
        SyncRuntimeOpenPhase::OpeningEnrollment => "managed_open.opening_enrollment",
        SyncRuntimeOpenPhase::OpeningProjectionReceipts => {
            "managed_open.opening_projection_receipts"
        }
        SyncRuntimeOpenPhase::OpeningReconciliationBaseline => {
            "managed_open.opening_reconciliation_baseline"
        }
        SyncRuntimeOpenPhase::RecoveringPromotedRuntime => {
            "managed_open.recovering_promoted_runtime"
        }
        SyncRuntimeOpenPhase::AssemblingActor => "managed_open.assembling_actor",
    }
}

fn managed_open_waiting_phase_name(phase: SyncRuntimeOpenPhase) -> &'static str {
    match phase {
        SyncRuntimeOpenPhase::RetainingGraph => "managed_open.waiting_retaining_graph",
        SyncRuntimeOpenPhase::RecoveringCleanManifestRuntime => {
            "managed_open.waiting_recovering_clean_manifest_runtime"
        }
        SyncRuntimeOpenPhase::DiscoveringEnrollment => {
            "managed_open.waiting_discovering_enrollment"
        }
        SyncRuntimeOpenPhase::OpeningActorGraph => "managed_open.waiting_opening_actor_graph",
        SyncRuntimeOpenPhase::RevalidatingEnrollment => {
            "managed_open.waiting_revalidating_enrollment"
        }
        SyncRuntimeOpenPhase::OpeningEnrollment => "managed_open.waiting_opening_enrollment",
        SyncRuntimeOpenPhase::OpeningProjectionReceipts => {
            "managed_open.waiting_opening_projection_receipts"
        }
        SyncRuntimeOpenPhase::OpeningReconciliationBaseline => {
            "managed_open.waiting_opening_reconciliation_baseline"
        }
        SyncRuntimeOpenPhase::RecoveringPromotedRuntime => {
            "managed_open.waiting_recovering_promoted_runtime"
        }
        SyncRuntimeOpenPhase::AssemblingActor => "managed_open.waiting_assembling_actor",
    }
}

/// A safe, bounded terminal code for unconditional managed-open diagnostics.
/// `SyncRuntimeOpenStatus` also carries user/storage details for normal command
/// replies; those details must never be formatted into the startup trace.
fn managed_open_outcome_code(status: &SyncRuntimeOpenStatus) -> &'static str {
    match status {
        SyncRuntimeOpenStatus::LegacyDefault => "legacy_default",
        SyncRuntimeOpenStatus::Absent => "absent",
        SyncRuntimeOpenStatus::ExistingNonActive(SyncNonActiveStage::ShadowImport) => {
            "existing_shadow_import"
        }
        SyncRuntimeOpenStatus::ExistingNonActive(SyncNonActiveStage::VerifiedLocal) => {
            "existing_verified_local"
        }
        SyncRuntimeOpenStatus::Blocked { .. } => "blocked",
        SyncRuntimeOpenStatus::SupersededLegacyState => "superseded_legacy_state",
        SyncRuntimeOpenStatus::UnsupportedOrIncompatible {
            component: SyncRuntimeComponent::Enrollment,
            ..
        } => "unsupported_enrollment",
        SyncRuntimeOpenStatus::UnsupportedOrIncompatible {
            component: SyncRuntimeComponent::Archive,
            ..
        } => "unsupported_archive",
        SyncRuntimeOpenStatus::CorruptOrUnreadable {
            component: SyncRuntimeComponent::Enrollment,
            ..
        } => "corrupt_enrollment",
        SyncRuntimeOpenStatus::CorruptOrUnreadable {
            component: SyncRuntimeComponent::Archive,
            ..
        } => "corrupt_archive",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::EnrollmentResidue,
            ..
        } => "ambiguous_enrollment_residue",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::EnrollmentNamespace,
            ..
        } => "ambiguous_enrollment_namespace",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::EnrollmentGraphBinding,
            ..
        } => "ambiguous_enrollment_graph_binding",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::ArchiveResidue,
            ..
        } => "ambiguous_archive_residue",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::ArchiveNamespace,
            ..
        } => "ambiguous_archive_namespace",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::ArchiveBinding,
            ..
        } => "ambiguous_archive_binding",
        SyncRuntimeOpenStatus::AmbiguousOrForeignResidue {
            evidence: SyncAmbiguousEvidence::ActiveArchiveMismatch,
            ..
        } => "ambiguous_active_archive_mismatch",
        SyncRuntimeOpenStatus::Active => "active",
        SyncRuntimeOpenStatus::OpenRefused { .. } => "open_refused",
    }
}

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

struct PreparedActivationBinding {
    binding: SparseV2Binding,
    largest_page_path: Option<String>,
}

fn attach_latest_progress_to_activation_result(
    binding: &mut SparseV2Binding,
    progress: &str,
) -> Option<String> {
    let SparseV2Availability::Retryable { detail, .. } = &mut binding.availability else {
        return None;
    };
    let contextual = format!("Tine-managed storage setup failed during {progress}: {detail}");
    *detail = contextual.clone();
    Some(contextual)
}

fn activate_record_with_diagnostics(
    facade: &SyncRuntimeFacade,
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    record: &SparseV2ActivationRecord,
) -> Result<PreparedActivationBinding, crate::command_error::CommandError> {
    let started = Instant::now();
    let latest_progress = Arc::new(Mutex::new(None));
    let largest_page_path = Arc::new(Mutex::new(None));
    let heartbeat = ActivationHeartbeat::start(started, Arc::clone(&latest_progress));
    let largest_page_path_for_progress = Arc::clone(&largest_page_path);
    let result = facade.activate_record_with_detailed_progress(app, record, |progress| {
        if let SyncLocalActivationProgress::ReadinessSample { largest_page_path } = &progress {
            *largest_page_path_for_progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = largest_page_path.clone();
        }
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
    let progress = latest_activation_progress_name(&latest_progress);
    match result {
        Ok(mut binding) => {
            if let Some(detail) =
                attach_latest_progress_to_activation_result(&mut binding, &progress)
            {
                crate::debug::diag(format!(
                    "sparse-v2 activation failed after {} ms: {detail}",
                    started.elapsed().as_millis()
                ));
            }
            Ok(PreparedActivationBinding {
                binding,
                largest_page_path: largest_page_path
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            })
        }
        Err(error) => {
            let detail = format!("Tine-managed storage setup failed during {progress}: {error}");
            crate::debug::diag(format!(
                "sparse-v2 activation failed after {} ms: {detail}",
                started.elapsed().as_millis()
            ));
            Err(crate::command_error::CommandError::sync_runtime(detail))
        }
    }
}

pub(crate) fn active_handle(
    slot: &crate::state::GraphSlot,
) -> Result<&tine_core::sync_runtime::SyncRuntimeHandle, crate::command_error::CommandError> {
    slot.sparse_runtime()
        .ok_or_else(|| crate::command_error::CommandError::sync_runtime(SPARSE_V2_NOT_ACTIVE))
}

fn sparse_v2_status_for_slot(
    slot: &crate::state::GraphSlot,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let mut status = match slot.sparse_binding() {
        Some(binding) => {
            let mut status = SparseV2StatusDto::from_binding(binding, slot.binding_generation);
            // Once the binding itself belongs to this graph, the explicit
            // archive-and-return command is always available.  Shared/pending
            // state is surfaced as a confirmation warning, never as a lockout.
            status.can_cancel = true;
            status.cancel_reason = cancel_warning(binding, &slot.root_key.join(".tine-sync/v2"));
            status
        }
        // Status is observational and runs on every ordinary graph bind. It
        // must not turn Direct Files into an implicit managed-storage probe.
        // The explicit Join command performs provider discovery after the user
        // asks for it.
        None => SparseV2StatusDto::legacy(slot.binding_generation),
    };
    // Status names describe enrollment/recovery. This record describes the
    // exact writer retained by the slot that will service `save_page`.
    status.application_page_admission = slot.application_page_admission();
    Ok(status)
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedReadinessReceipt {
    page_count: usize,
    sampled_paths: Vec<String>,
    inventory_ms: u64,
    sample_load_ms: u64,
    total_ms: u64,
}

/// Prove the application observation boundary before a managed candidate is
/// published. Installing an actor-backed GraphSlot is only native
/// construction; users need its ordinary inventory and representative
/// page-load commands to observe the complete SQLite materialization at the
/// candidate's own authenticated accepted frontier.
pub(crate) fn prove_managed_application_ready(
    slot: &crate::state::GraphSlot,
    largest_page_path: Option<&str>,
) -> Result<ManagedReadinessReceipt, crate::command_error::CommandError> {
    let started = Instant::now();
    let handle = active_handle(slot)?;
    let inventory_started = Instant::now();
    let pages = match handle.application_page_inventory().map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "managed readiness page inventory failed: {error}"
        ))
    })? {
        SyncApplicationPageInventoryOutcome::Loaded { pages } => pages,
        SyncApplicationPageInventoryOutcome::Deferred { .. } => {
            return Err(crate::command_error::CommandError::prose(
                "managed readiness page inventory remained deferred",
            ))
        }
    };
    let inventory_ms = inventory_started.elapsed().as_millis() as u64;
    let mut sampled_paths = Vec::new();
    if let Some(page) = pages.first() {
        sampled_paths.push(page.rel_path.clone());
    }
    if let Some(path) = largest_page_path
        .filter(|path| pages.iter().any(|page| page.rel_path == *path))
        .filter(|path| !sampled_paths.iter().any(|sample| sample == path))
    {
        sampled_paths.push(path.to_owned());
    }
    let sample_started = Instant::now();
    for path in &sampled_paths {
        match handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath { path: path.clone() },
            })
            .map_err(|error| {
                crate::command_error::CommandError::sync_runtime(format!(
                    "managed readiness representative page failed: {error}"
                ))
            })? {
            SyncApplicationPageLoadOutcome::Loaded { .. } => {}
            SyncApplicationPageLoadOutcome::Missing { .. }
            | SyncApplicationPageLoadOutcome::Ambiguous
            | SyncApplicationPageLoadOutcome::Deferred { .. } => {
                return Err(crate::command_error::CommandError::prose(
                    "managed readiness could not open its representative page",
                ))
            }
        }
    }
    let receipt = ManagedReadinessReceipt {
        page_count: pages.len(),
        sampled_paths,
        inventory_ms,
        sample_load_ms: sample_started.elapsed().as_millis() as u64,
        total_ms: started.elapsed().as_millis() as u64,
    };
    crate::debug::diag(format!("managed readiness proved: {receipt:?}"));
    Ok(receipt)
}

fn sparse_v2_status_for_observation(
    slot: &crate::state::GraphSlot,
    snapshot: SyncRuntimeStatusSnapshot,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let binding = slot.sparse_binding().ok_or_else(|| {
        crate::command_error::CommandError::sync_runtime(
            "managed cross-page move recovery requires managed storage",
        )
    })?;
    let cancel_reason =
        cancel_warning_for_observation(binding, &slot.root_key.join(".tine-sync/v2"), &snapshot);
    let mut status =
        SparseV2StatusDto::from_binding_observation(binding, slot.binding_generation, snapshot);
    status.can_cancel = true;
    status.cancel_reason = cancel_reason;
    Ok(status)
}

fn move_outcome_episode_id(outcome: &SyncApplicationMoveSubtreesOutcome) -> &str {
    match outcome {
        SyncApplicationMoveSubtreesOutcome::Committed { episode_id, .. }
        | SyncApplicationMoveSubtreesOutcome::NoCommit { episode_id, .. }
        | SyncApplicationMoveSubtreesOutcome::Deferred { episode_id, .. } => episode_id,
    }
}

fn move_recovery_result(
    previous_binding_generation: u64,
    slot: &crate::state::GraphSlot,
    expected_episode_id: &str,
    outcome: SyncApplicationMoveSubtreesOutcome,
    snapshot: SyncRuntimeStatusSnapshot,
) -> Result<
    crate::commands::ManagedApplicationMoveSubtreesRecoveryResult,
    crate::command_error::CommandError,
> {
    if move_outcome_episode_id(&outcome) != expected_episode_id {
        return Err(crate::command_error::CommandError::prose(
            "managed cross-page move recovery returned a different episode",
        ));
    }
    if snapshot.lifecycle != SyncRuntimeLifecycle::Active {
        return Err(crate::command_error::CommandError::sync_runtime(
            "managed cross-page move recovery requires process reopen after a terminal runtime",
        ));
    }
    let status = sparse_v2_status_for_observation(slot, snapshot)?;
    let application_page_admission = status.application_page_admission.clone();
    Ok(
        crate::commands::ManagedApplicationMoveSubtreesRecoveryResult {
            previous_binding_generation,
            binding_generation: slot.binding_generation,
            status,
            application_page_admission,
            episode_id: expected_episode_id.to_owned(),
            outcome,
        },
    )
}

fn recover_managed_application_subtrees_with(
    state: &crate::state::AppState,
    label: &str,
    binding_generation: u64,
    request: SyncApplicationMoveSubtreesRequest,
    reopen: impl FnOnce(
        &Path,
    ) -> Result<
        (SparseV2Binding, tine_core::model::GraphMeta),
        crate::command_error::CommandError,
    >,
) -> Result<
    crate::commands::ManagedApplicationMoveSubtreesRecoveryResult,
    crate::command_error::CommandError,
> {
    let root = crate::state::slot_for_bound_window(state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    let predecessor = crate::state::slot_for_bound_window(state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if predecessor.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while move recovery waited for its transition lane",
        ));
    }
    let action = predecessor
        .sparse_binding()
        .ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime(
                "managed cross-page move recovery requires managed storage",
            )
        })?
        .action();

    match action {
        SparseV2BindingAction::ReturnRetained => {
            let handle = predecessor.sparse_runtime().ok_or_else(|| {
                crate::command_error::CommandError::sync_runtime({
                    "managed cross-page move recovery has no retained actor".to_owned()
                })
            })?;
            let observation = handle
                .resolve_application_move_subtrees(request.clone())
                .map_err(crate::command_error::CommandError::from)?;
            let result = move_recovery_result(
                binding_generation,
                &predecessor,
                &request.episode_id,
                observation.move_outcome,
                observation.runtime_snapshot,
            )?;
            crate::state::poke_watcher(state);
            Ok(result)
        }
        SparseV2BindingAction::ReopenActive => {
            let (binding, graph_meta) = reopen(&root)?;
            let successor = Arc::new(crate::state::GraphSlot::from_sparse_v2(
                binding,
                root.clone(),
                graph_meta,
            ));
            let observation = successor
                .sparse_runtime()
                .ok_or_else(|| {
                    crate::command_error::CommandError::sync_runtime({
                        "managed cross-page move recovery could not reopen an active actor"
                            .to_owned()
                    })
                })?
                .resolve_application_move_subtrees(request.clone())
                .map_err(crate::command_error::CommandError::from)?;
            let result = move_recovery_result(
                binding_generation,
                &successor,
                &request.episode_id,
                observation.move_outcome,
                observation.runtime_snapshot,
            )?;

            state.storage_supervisor.publish_managed_maintenance(|| {
                state
                    .graphs
                    .write()
                    .unwrap()
                    .replace_if_current(label, binding_generation, &root, Arc::clone(&successor))
                    .map_err(crate::command_error::CommandError::from)
            })?;
            crate::state::poke_watcher(state);
            Ok(result)
        }
        SparseV2BindingAction::ActivateOrResume => Err(crate::command_error::CommandError::prose(
            "managed cross-page move recovery cannot activate an incomplete runtime",
        )),
    }
}

pub(crate) fn recover_managed_application_subtrees_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    request: SyncApplicationMoveSubtreesRequest,
) -> Result<
    crate::commands::ManagedApplicationMoveSubtreesRecoveryResult,
    crate::command_error::CommandError,
> {
    let state = app.state::<crate::state::AppState>();
    recover_managed_application_subtrees_with(&state, label, binding_generation, request, |root| {
        let record = state
            .sync_runtime
            .binding_record(app, root)?
            .ok_or_else(|| {
                crate::command_error::CommandError::sync_runtime(
                    "Tine-managed storage setup is missing.",
                )
            })?;
        let graph_meta = SyncRuntimeFacade::graph_meta(&record);
        let binding = state.sync_runtime.open_record(app, &record)?;
        Ok((binding, graph_meta))
    })
}

#[tauri::command]
pub(crate) async fn sparse_v2_status(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        sparse_v2_status_for_slot(&slot)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

/// Explicitly retire one Direct authority and activate/resume managed storage.
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
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let label = state.window.label().to_string();
    let binding_generation = state
        .binding_generation
        .ok_or_else(|| crate::command_error::CommandError::prose("missing-graph-binding"))?;
    drop(state);
    tauri::async_runtime::spawn_blocking(move || {
        activate_sparse_v2_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Tine-managed storage setup worker failed: {error}"
        ))
    })?
}

pub(crate) fn activate_sparse_v2_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let root = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition = state.storage_supervisor.begin_guard(
        app,
        label,
        root.clone(),
        StorageTransitionKind::ActivateManaged,
    )?;
    transition.advance(StorageTransitionPhase::WaitingForTransition)?;
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    if !transition.is_current() {
        return Err(crate::command_error::CommandError::prose(
            "managed activation was superseded while waiting for its graph lane",
        ));
    }
    transition.advance(StorageTransitionPhase::ValidatingTarget)?;
    transition.advance(StorageTransitionPhase::ActivatingManaged)?;
    let prepared = match prepare_sparse_v2_activation(app, label, binding_generation, root) {
        Ok(prepared) => prepared,
        Err(error) => {
            transition.fail("activation_failed");
            return Err(error);
        }
    };
    let PreparedActivationOutcome::Candidate(candidate) = prepared else {
        let PreparedActivationOutcome::AlreadyCurrent(status) = prepared else {
            unreachable!()
        };
        transition.succeed(StableStorageMode::Managed)?;
        return Ok(status);
    };
    let state = app.state::<crate::state::AppState>();
    let (published, status) =
        transition.publish(|| publish_managed_candidate(app, &state, label, candidate))?;
    published.succeed(StableStorageMode::Managed)?;
    Ok(status)
}

struct PreparedManagedCandidate {
    predecessor: Arc<crate::state::GraphSlot>,
    replacement: Arc<crate::state::GraphSlot>,
    record: SparseV2ActivationRecord,
    readiness: ManagedReadinessReceipt,
    direct_source_generation: Option<u64>,
}

enum PreparedActivationOutcome {
    AlreadyCurrent(SparseV2StatusDto),
    Candidate(PreparedManagedCandidate),
}

fn prepare_sparse_v2_activation(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    root: PathBuf,
) -> Result<PreparedActivationOutcome, crate::command_error::CommandError> {
    let started = Instant::now();
    let state = app.state::<crate::state::AppState>();
    crate::debug::diag("sparse-v2 activation requested");
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if slot.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while activation waited for its transition lane",
        ));
    }

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
            return result.map(PreparedActivationOutcome::AlreadyCurrent);
        }
        let record = state
            .sync_runtime
            .binding_record(app, &root)?
            .ok_or_else(|| {
                crate::command_error::CommandError::prose("Tine-managed storage setup is missing.")
            })?;
        let graph_meta = SyncRuntimeFacade::graph_meta(&record);
        let core_started = Instant::now();
        let prepared = match action {
            SparseV2BindingAction::ReopenActive => state.sync_runtime.open_record(app, &record)?,
            SparseV2BindingAction::ActivateOrResume => {
                activate_record_with_diagnostics(
                    &state.sync_runtime,
                    app,
                    label,
                    binding_generation,
                    &record,
                )?
                .binding
            }
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
            prepared, root, graph_meta,
        ));
        let readiness = prove_managed_application_ready(&replacement, None)?;
        crate::debug::diag(format!(
            "sparse-v2 retained activation candidate ready after {} ms: {readiness:?}",
            started.elapsed().as_millis()
        ));
        return Ok(PreparedActivationOutcome::Candidate(
            PreparedManagedCandidate {
                predecessor: slot,
                replacement,
                record,
                readiness,
                direct_source_generation: None,
            },
        ));
    }

    let graph = slot
        .legacy_graph()
        .map_err(crate::command_error::CommandError::from)?;
    let graph_meta = graph.meta();
    let direct_source_generation = graph.guarded_graph_text_identity_report().generation;
    drop(graph);
    let record = state
        .sync_runtime
        .prepare_binding_record(app, &root, graph_meta.clone())?;
    crate::debug::diag(format!(
        "sparse-v2 fresh activation prepared private binding after {} ms",
        started.elapsed().as_millis()
    ));

    crate::debug::diag(format!(
        "sparse-v2 private candidate prepared after {} ms; starting core bootstrap while Direct Files continues serving",
        started.elapsed().as_millis()
    ));

    let core_started = Instant::now();
    let prepared = activate_record_with_diagnostics(
        &state.sync_runtime,
        app,
        label,
        binding_generation,
        &record,
    )?;
    crate::debug::diag(format!(
        "sparse-v2 core bootstrap completed after {} ms: availability={:?}",
        core_started.elapsed().as_millis(),
        prepared.binding.availability()
    ));
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        prepared.binding,
        root.clone(),
        graph_meta.clone(),
    ));
    let readiness =
        prove_managed_application_ready(&replacement, prepared.largest_page_path.as_deref())?;
    crate::debug::diag(format!(
        "sparse-v2 fresh activation candidate ready after {} ms: {readiness:?}",
        started.elapsed().as_millis()
    ));
    Ok(PreparedActivationOutcome::Candidate(
        PreparedManagedCandidate {
            predecessor: slot,
            replacement,
            record,
            readiness,
            direct_source_generation: Some(direct_source_generation),
        },
    ))
}

fn publish_managed_candidate(
    app: &tauri::AppHandle,
    state: &crate::state::AppState,
    label: &str,
    candidate: PreparedManagedCandidate,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let record = candidate.record.clone();
    publish_managed_candidate_with(
        state,
        label,
        candidate,
        || state.sync_runtime.persist_binding_record(app, &record),
        |root| {
            publish_direct_selection(app, root, "managed publication lost its exact predecessor")
        },
    )
}

fn publish_managed_candidate_with(
    state: &crate::state::AppState,
    label: &str,
    candidate: PreparedManagedCandidate,
    persist_successor: impl FnOnce() -> Result<(), crate::command_error::CommandError>,
    restore_direct_selection: impl FnOnce(&Path) -> Result<(), crate::command_error::CommandError>,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let current =
        state.graphs.read().unwrap().slot(label).ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime("stale-graph-binding")
        })?;
    if current.binding_generation != candidate.predecessor.binding_generation
        || current.root_key != candidate.predecessor.root_key
    {
        return Err(crate::command_error::CommandError::sync_runtime(
            "The graph changed while the managed candidate was prepared. Retry setup.",
        ));
    }
    // Acquire the predecessor's graph-text mutation authority before taking
    // the registry write lock.  This both avoids a lock-order inversion and
    // keeps the source generation stable through selector + slot publication.
    let source_graph = candidate
        .direct_source_generation
        .map(|_| current.legacy_graph())
        .transpose()
        .map_err(crate::command_error::CommandError::from)?;
    let _source_publication = if let Some(expected_generation) = candidate.direct_source_generation
    {
        let graph = source_graph
            .as_ref()
            .expect("a Direct source generation always retains its graph lease");
        Some(
            graph
                .lock_graph_text_identity_publication(expected_generation)
                .map_err(crate::command_error::CommandError::from)?
                .ok_or_else(|| crate::command_error::CommandError::sync_runtime("The Markdown/Org graph changed while managed storage was being prepared. Direct Files remains active; retry setup."))?,
        )
    } else {
        None
    };

    let mut graphs = state.graphs.write().unwrap();
    let current = graphs
        .slot(label)
        .ok_or_else(|| crate::command_error::CommandError::sync_runtime("stale-graph-binding"))?;
    if current.binding_generation != candidate.predecessor.binding_generation
        || current.root_key != candidate.predecessor.root_key
    {
        return Err(crate::command_error::CommandError::sync_runtime(
            "The graph changed while the managed candidate was prepared. Retry setup.",
        ));
    }
    let predecessor_was_direct = candidate.direct_source_generation.is_some();
    if let Err(error) = persist_successor() {
        if predecessor_was_direct {
            let rollback = restore_direct_selection(&candidate.predecessor.root_key);
            return Err(crate::command_error::CommandError::sync_runtime(match rollback {
                Ok(()) => format!(
                    "managed selector publication failed ({error}); Direct Files selection was restored"
                ),
                Err(rollback) => format!(
                    "managed selector publication failed ({error}) and Direct Files selection could not be restored ({rollback})"
                ),
            }));
        }
        return Err(crate::command_error::CommandError::sync_runtime(format!(
            "managed runtime publication failed: {error}"
        )));
    }
    if let Err(error) = graphs.replace_if_current(
        label,
        candidate.predecessor.binding_generation,
        &candidate.predecessor.root_key,
        Arc::clone(&candidate.replacement),
    ) {
        if !predecessor_was_direct {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "managed runtime publication was superseded: {error}"
            )));
        }
        let rollback = restore_direct_selection(&candidate.predecessor.root_key);
        return Err(crate::command_error::CommandError::sync_runtime(match rollback {
            Ok(()) => format!("managed publication was superseded: {error}"),
            Err(rollback) => format!(
                "managed publication was superseded ({error}) and Direct Files selection could not be restored ({rollback})"
            ),
        }));
    }
    drop(graphs);
    crate::state::poke_watcher(state);
    crate::debug::diag(format!(
        "managed candidate published once: predecessor_generation={}, successor_generation={}, readiness={:?}",
        candidate.predecessor.binding_generation,
        candidate.replacement.binding_generation,
        candidate.readiness,
    ));
    sparse_v2_status_for_slot(&candidate.replacement)
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2CancelResult {
    status: SparseV2StatusDto,
    binding_generation: u64,
    recovery_statement: String,
}

/// Cold startup recovery intentionally has the same public result shape as an
/// in-app Direct Files return.  The caller's attempt is validated at the
/// mutation boundary, not trusted as storage authority or reflected to the UI.
pub(crate) type SparseV2ColdCancelResult = SparseV2CancelResult;

fn archive_private_root(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<Option<PathBuf>, crate::command_error::CommandError> {
    let metadata = match std::fs::symlink_metadata(private_root) {
        Ok(metadata) => metadata,
        // A failed or partially-created activation need not have retained any
        // app-private state.  There is then nothing to preserve, not a reason
        // to strand the user.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect Tine-managed storage recovery state: {error}"
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose("Tine-managed storage recovery state is a symbolic link, so it could not be archived safely."));
    }
    std::fs::create_dir_all(recovery_root).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't prepare Tine-managed storage recovery state: {error}")
        })
    })?;
    let recovery_metadata = std::fs::symlink_metadata(recovery_root).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't inspect Tine-managed storage recovery state: {error}")
        })
    })?;
    if !recovery_metadata.is_dir() || recovery_metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose("Tine-managed storage recovery state is not a local directory, so it could not be archived safely."));
    }
    let key = private_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            crate::command_error::CommandError::prose(
                "Tine-managed storage recovery state has no valid local key.",
            )
        })?;
    let destination = recovery_root.join(format!("{key}-{}", Uuid::new_v4()));
    std::fs::rename(private_root, &destination).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't preserve Tine-managed storage recovery state: {error}")
        })
    })?;
    sync_provider_namespace_rename(
        destination.parent().ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime({
                "Tine-managed storage recovery destination has no parent directory.".to_string()
            })
        })?,
        private_root.parent().ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime({
                "Tine-managed storage recovery source has no parent directory.".to_string()
            })
        })?,
    )
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't durably preserve Tine-managed storage recovery state: {error}")
        })
    })?;
    Ok(Some(destination))
}

fn blank_slate_recovery_key(
    private_root: &Path,
) -> Result<&str, crate::command_error::CommandError> {
    private_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            crate::command_error::CommandError::prose(
                "Tine-managed storage recovery state has no valid local key.",
            )
        })
}

fn blank_slate_backup_complete_path(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<PathBuf, crate::command_error::CommandError> {
    Ok(recovery_root.join(format!(
        "{}{BLANK_SLATE_BACKUP_COMPLETE_SUFFIX}",
        blank_slate_recovery_key(private_root)?
    )))
}

fn blank_slate_backup_is_complete(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<bool, crate::command_error::CommandError> {
    let marker = blank_slate_backup_complete_path(private_root, recovery_root)?;
    match std::fs::read(&marker) {
        Ok(bytes) => Ok(bytes == b"complete\n"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't inspect the pre-0.7 managed-storage backup marker: {error}"
        ))),
    }
}

fn clear_blank_slate_backup_complete(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<(), crate::command_error::CommandError> {
    let marker = blank_slate_backup_complete_path(private_root, recovery_root)?;
    match std::fs::remove_file(marker) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't reset the pre-0.7 managed-storage backup marker: {error}"
        ))),
    }
}

fn mark_blank_slate_backup_complete(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<(), crate::command_error::CommandError> {
    std::fs::create_dir_all(recovery_root).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't prepare Tine-managed storage recovery state: {error}")
        })
    })?;
    let marker = blank_slate_backup_complete_path(private_root, recovery_root)?;
    tine_core::model::atomic_update(&marker, &DIRECT_SELECTION_WRITE, |_| {
        Ok("complete\n".to_owned())
    })
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't record the pre-0.7 managed-storage backup: {error}"
        ))
    })
}

/// Retain at most one disposable failed rebuild candidate. The original
/// unrecognized private state has its own immutable archive and completion
/// marker; retry products are reconstructed from Markdown/Org and must not
/// create an unbounded recovery directory on every launch.
fn replace_failed_blank_slate_candidate(
    private_root: &Path,
    recovery_root: &Path,
) -> Result<Option<PathBuf>, crate::command_error::CommandError> {
    let metadata = match std::fs::symlink_metadata(private_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect the failed managed-storage rebuild: {error}"
            )));
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose(
            "The failed managed-storage rebuild is not a safe local directory.",
        ));
    }
    std::fs::create_dir_all(recovery_root).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't prepare Tine-managed storage recovery state: {error}")
        })
    })?;
    let destination = recovery_root.join(format!(
        "{}{BLANK_SLATE_FAILED_CANDIDATE_SUFFIX}",
        blank_slate_recovery_key(private_root)?
    ));
    match std::fs::symlink_metadata(&destination) {
        Ok(existing) if existing.is_dir() && !existing.file_type().is_symlink() => {
            std::fs::remove_dir_all(&destination).map_err(|error| {
                crate::command_error::CommandError::sync_runtime({
                    format!("Couldn't retire the prior failed managed-storage rebuild: {error}")
                })
            })?;
        }
        Ok(_) => {
            std::fs::remove_file(&destination).map_err(|error| {
                crate::command_error::CommandError::sync_runtime({
                    format!("Couldn't retire the prior failed managed-storage rebuild: {error}")
                })
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect the prior failed managed-storage rebuild: {error}"
            )));
        }
    }
    std::fs::rename(private_root, &destination).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't set aside the failed managed-storage rebuild: {error}")
        })
    })?;
    sync_provider_namespace_rename(
        destination.parent().ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime({
                "The failed managed-storage recovery destination has no parent directory."
                    .to_string()
            })
        })?,
        private_root.parent().ok_or_else(|| {
            crate::command_error::CommandError::sync_runtime({
                "The failed managed-storage rebuild has no parent directory.".to_string()
            })
        })?,
    )
    .map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't durably set aside the failed managed-storage rebuild: {error}")
        })
    })?;
    Ok(Some(destination))
}

fn prepare_blank_slate_retry_at_paths(
    private_root: &Path,
    recovery_root: &Path,
    record: SparseV2ActivationRecord,
) -> Result<SparseV2ActivationRecord, crate::command_error::CommandError> {
    if blank_slate_backup_is_complete(private_root, recovery_root)? {
        replace_failed_blank_slate_candidate(private_root, recovery_root)?;
    } else {
        // The durable Direct intent may have been published just before a
        // crash or archive failure. Preserve the original bytes before any
        // current-format reconstruction begins. If the prior rename already
        // succeeded, absence is sufficient and the marker closes the gap.
        archive_private_root(private_root, recovery_root).map_err(|error| {
            crate::command_error::CommandError::sync_runtime({
                format!("Couldn't preserve the original pre-0.7 managed-storage state: {error}")
            })
        })?;
        mark_blank_slate_backup_complete(private_root, recovery_root)?;
    }
    Ok(record)
}

fn prepare_shared_binding_record_at_paths(
    private_root: &Path,
    recovery_root: &Path,
    graph_root: &Path,
    graph_meta: GraphMeta,
    device_id: DeviceId,
    descriptor: &SyncSharedEnrollmentDescriptor,
) -> Result<SparseV2ActivationRecord, crate::command_error::CommandError> {
    let record =
        SparseV2ActivationRecord::from_shared(graph_root, graph_meta, device_id, descriptor);
    prepare_fresh_authority_at_paths(private_root, recovery_root, record, "shared-graph join")
}

fn prepare_fresh_authority_at_paths(
    private_root: &Path,
    recovery_root: &Path,
    record: SparseV2ActivationRecord,
    operation: &str,
) -> Result<SparseV2ActivationRecord, crate::command_error::CommandError> {
    archive_private_root(private_root, recovery_root).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!(
            "Couldn't quarantine prior unselected managed-storage state before {operation}: {error}"
        )
        })
    })?;
    Ok(record)
}

#[derive(Debug)]
enum ProviderNamespaceArchive {
    Absent,
    Moved {
        source: PathBuf,
        destination: PathBuf,
    },
}

/// Preserve graph-local provider state outside the live `.tine-sync/v2`
/// namespace.  A Direct Files restart deliberately refuses an unclaimed v2
/// namespace, so archiving only private app-data would leave a delayed
/// lockout.  This is a same-filesystem rename, never a delete or copy.
fn archive_graph_provider_namespace(
    graph_root: &Path,
) -> Result<ProviderNamespaceArchive, crate::command_error::CommandError> {
    archive_graph_provider_namespace_with(graph_root, |shared| {
        matches!(
            inspect_shared_enrollment_for_cold_discovery(shared),
            Ok(Some(_))
        )
    })
}

fn archive_graph_provider_namespace_with(
    graph_root: &Path,
    joinable: impl FnOnce(&Path) -> bool,
) -> Result<ProviderNamespaceArchive, crate::command_error::CommandError> {
    let source = graph_root.join(".tine-sync/v2");
    let metadata = match std::fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProviderNamespaceArchive::Absent);
        }
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't inspect graph-local managed-storage state before returning to Direct files: {error}"
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose("Graph-local managed-storage state is a symbolic link, so it could not be archived safely."));
    }
    // A COMPLETE provider tree is the other device's live enrollment, and this
    // folder is synced: archiving it here removes the descriptor from the
    // device that is still sharing, which is how Martin's graph ended up with
    // two `recovery/v2-*` archives and no `v2` at all. The archive exists only
    // to stop an UNCLAIMED namespace from locking a later Direct Files open
    // out, and a joinable tree does not — `refuse_unclaimed_sparse_archive`
    // admits it and the panel offers Join beside it.
    if joinable(&source.join("shared")) {
        crate::debug::diag(
            "sparse-v2 direct-files return: phase=graph_provider; outcome=preserved_joinable_peer_evidence",
        );
        return Ok(ProviderNamespaceArchive::Absent);
    }
    let tine_sync = source.parent().ok_or_else(|| {
        crate::command_error::CommandError::prose(
            "Graph-local managed-storage state has no .tine-sync parent.",
        )
    })?;
    let parent_metadata = std::fs::symlink_metadata(tine_sync).map_err(|error| crate::command_error::CommandError::sync_runtime({
        format!(
            "Couldn't inspect graph-local managed-storage parent before returning to Direct files: {error}"
        )
    }))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose("Graph-local managed-storage parent is not a local directory, so it could not be archived safely."));
    }
    let recovery = tine_sync.join("recovery");
    match std::fs::create_dir(&recovery) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Couldn't prepare graph-local managed-storage recovery state: {error}"
            )));
        }
    }
    let recovery_metadata = std::fs::symlink_metadata(&recovery).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't inspect graph-local managed-storage recovery state: {error}")
        })
    })?;
    if !recovery_metadata.is_dir() || recovery_metadata.file_type().is_symlink() {
        return Err(crate::command_error::CommandError::prose("Graph-local managed-storage recovery state is not a local directory, so it could not be archived safely."));
    }
    // Re-barrier an existing recovery entry too: it may be the visible residue
    // of a prior attempt whose directory flush was refused.
    tine_core::model::sync_dir_for_rename(tine_sync).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't durably prepare graph-local managed-storage recovery state: {error}")
        })
    })?;
    let destination = recovery.join(format!("v2-{}", Uuid::new_v4()));
    std::fs::rename(&source, &destination).map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Couldn't preserve graph-local managed-storage state: {error}"
        ))
    })?;
    sync_provider_namespace_rename(&recovery, tine_sync).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            format!("Couldn't durably preserve graph-local managed-storage state: {error}")
        })
    })?;
    Ok(ProviderNamespaceArchive::Moved {
        source,
        destination,
    })
}

/// Adoption's counterpart to `archive_graph_provider_namespace`: it preserves
/// `<graph>/.tine-sync/v2` exactly where it is.
///
/// The graceful Direct Files return archives that subtree because an unclaimed
/// v2 namespace would otherwise lock a later Direct Files restart out. Adoption
/// is not staying in Direct Files: the subtree it would archive is the OTHER
/// device's shared evidence, and archiving it would delete the descriptor the
/// second half of adoption is about to read — and, under a folder-syncing
/// tool, propagate that removal back to the device that is sharing.
fn preserve_graph_provider_namespace(
    _graph_root: &Path,
) -> Result<ProviderNamespaceArchive, crate::command_error::CommandError> {
    Ok(ProviderNamespaceArchive::Absent)
}

fn restore_graph_provider_namespace(
    archive: ProviderNamespaceArchive,
) -> Result<(), crate::command_error::CommandError> {
    let ProviderNamespaceArchive::Moved {
        source,
        destination,
    } = archive
    else {
        return Ok(());
    };
    std::fs::rename(&destination, &source).map_err(|error| crate::command_error::CommandError::sync_runtime({
        format!(
            "Tine-managed storage could not restore graph-local provider state after preserving private recovery state failed: {error}"
        )
    }))?;
    let destination_parent = source.parent().ok_or_else(|| {
        crate::command_error::CommandError::sync_runtime({
            "Tine-managed storage restore destination has no parent directory.".to_string()
        })
    })?;
    let source_parent = destination.parent().ok_or_else(|| {
        crate::command_error::CommandError::sync_runtime({
            "Tine-managed storage recovery source has no parent directory.".to_string()
        })
    })?;
    sync_provider_namespace_rename(destination_parent, source_parent).map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Tine-managed storage could not durably restore graph-local provider state: {error}"
        ))
    })
}

/// A cross-directory rename changes two directory entries. Persist the
/// destination first so a crash cannot leave the only retained name on a
/// source directory whose removal was acknowledged first.
fn sync_provider_namespace_rename(
    destination_parent: &Path,
    source_parent: &Path,
) -> std::io::Result<()> {
    tine_core::model::sync_dir_for_rename(destination_parent)?;
    tine_core::model::sync_dir_for_rename(source_parent)
}

#[derive(Debug)]
enum DirectFilesShutdown {
    Clean,
    Forced { detail: String },
}

impl DirectFilesShutdown {
    fn retry_detail(&self) -> String {
        match self {
            Self::Clean => "Tine-managed storage stopped before returning to Direct files, but archival did not complete. Retry setup can reopen the retained state.".into(),
            Self::Forced { detail } => format!(
                "Tine-managed storage was explicitly stopped without a clean drain ({detail}), and archival did not complete. Retry setup can reopen the retained state; any in-memory managed edits that had not reached durable storage may be absent."
            ),
        }
    }

    fn completion_note(&self) -> Option<&str> {
        match self {
            Self::Clean => None,
            Self::Forced { .. } => Some(
                "Managed storage could not complete a clean drain before this confirmed return. Its retained state was archived, but any in-memory managed edits that had not reached durable storage may be absent.",
            ),
        }
    }
}

/// Attempt the normal drain first.  A confirmed return to Direct Files is an
/// explicit escape hatch, so a refusal does not leave the user trapped: the
/// actor is then crash-stopped and joined before any managed files move.
fn shutdown_for_direct_files_escape(
    slot: &crate::state::GraphSlot,
) -> Result<DirectFilesShutdown, crate::command_error::CommandError> {
    let Some(handle) = slot.sparse_runtime() else {
        return Ok(DirectFilesShutdown::Clean);
    };
    match handle.clean_shutdown() {
        Ok(SyncShutdownOutcome::Safe(_)) => Ok(DirectFilesShutdown::Clean),
        Ok(SyncShutdownOutcome::Terminal(snapshot)) => {
            let detail = snapshot
                .detail
                .unwrap_or_else(|| "the managed actor reached a terminal state".into());
            handle.stop_without_clean_drain().map_err(|error| crate::command_error::CommandError::sync_runtime({
                format!("Tine-managed storage could not stop for the confirmed Direct Files return: {error}")
            }))?;
            Ok(DirectFilesShutdown::Forced { detail })
        }
        Err(error) => {
            let detail = error.to_string();
            handle.stop_without_clean_drain().map_err(|stop| crate::command_error::CommandError::sync_runtime({
                format!("Tine-managed storage could not stop for the confirmed Direct Files return after its clean drain failed ({detail}): {stop}")
            }))?;
            Ok(DirectFilesShutdown::Forced { detail })
        }
    }
}

fn shutdown_for_graceful_direct_files(
    slot: &crate::state::GraphSlot,
) -> Result<DirectFilesShutdown, crate::command_error::CommandError> {
    let Some(handle) = slot.sparse_runtime() else {
        return Ok(DirectFilesShutdown::Clean);
    };
    match handle.clean_shutdown() {
        Ok(SyncShutdownOutcome::Safe(_)) => Ok(DirectFilesShutdown::Clean),
        Ok(SyncShutdownOutcome::Terminal(snapshot)) => Err(crate::command_error::CommandError::sync_runtime(format!(
            "Tine-managed storage could not confirm a safe projection before returning to Direct Files: {}. Use the emergency return if you accept that managed operations may be newer than Markdown.",
            snapshot
                .detail
                .unwrap_or_else(|| "the managed actor reached a terminal state".into())
        ))),
        Err(error) => Err(crate::command_error::CommandError::sync_runtime(format!(
            "Tine-managed storage could not drain safely before returning to Direct Files: {error}. Use the emergency return if you accept that managed operations may be newer than Markdown."
        ))),
    }
}

fn publish_stopped_managed_recovery_slot(
    state: &crate::state::AppState,
    label: &str,
    root_key: PathBuf,
    graph_meta: GraphMeta,
    detail: String,
    transition: Option<&crate::storage_mode_supervisor::StorageRecoveryTransitionGuard<'_>>,
) -> Result<Arc<crate::state::GraphSlot>, crate::command_error::CommandError> {
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        retryable_binding("local_active", detail),
        root_key,
        graph_meta,
    ));
    commit_transition(transition, || {
        state
            .graphs
            .write()
            .unwrap()
            .bind(label.to_string(), Arc::clone(&replacement))
            .map_err(crate::command_error::CommandError::from)?;
        crate::state::poke_watcher(state);
        Ok(())
    })?;
    Ok(replacement)
}

fn commit_transition<T>(
    transition: Option<&crate::storage_mode_supervisor::StorageRecoveryTransitionGuard<'_>>,
    publish: impl FnOnce() -> Result<T, crate::command_error::CommandError>,
) -> Result<T, crate::command_error::CommandError> {
    match transition {
        Some(transition) => transition.commit_recovery_step(publish),
        None => publish(),
    }
}

#[allow(clippy::too_many_arguments)]
fn cancel_sparse_v2_at_paths_with_archive_and_publish(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(
        &crate::state::GraphSlot,
    ) -> Result<DirectFilesShutdown, crate::command_error::CommandError>,
    archive: impl FnOnce(&Path, &Path) -> Result<Option<PathBuf>, crate::command_error::CommandError>,
    archive_provider: impl FnOnce(
        &Path,
    ) -> Result<
        ProviderNamespaceArchive,
        crate::command_error::CommandError,
    >,
    publish_direct: impl FnOnce(&Path, Option<&Path>) -> Result<u64, crate::command_error::CommandError>,
    transition: Option<&crate::storage_mode_supervisor::StorageRecoveryTransitionGuard<'_>>,
    after_shutdown: impl FnOnce(&DirectFilesShutdown) -> Result<(), crate::command_error::CommandError>,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
    slot.sparse_binding().ok_or_else(|| {
        crate::command_error::CommandError::prose("This graph is already using Direct files.")
    })?;
    // The slot is the live, exact graph binding.  Explicit recovery must not
    // require parsing a possibly-corrupt or absent private binding merely to
    // learn a path we already own.
    let direct_root = slot.root_key.clone();
    let graph_meta = slot.graph_meta();
    let removed = commit_transition(transition, || {
        Ok(state.graphs.write().unwrap().remove(label))
    })?;
    if removed.is_some() {
        crate::state::poke_watcher(state);
    }
    if removed.as_ref().is_none_or(|current| {
        current.binding_generation != slot.binding_generation || current.root_key != slot.root_key
    }) {
        if let Some(current) = removed {
            commit_transition(transition, || {
                state
                    .graphs
                    .write()
                    .unwrap()
                    .bind(label.to_string(), current)
                    .map_err(crate::command_error::CommandError::from)?;
                crate::state::poke_watcher(state);
                Ok(())
            })?;
        }
        return Err(crate::command_error::CommandError::prose(
            "The graph changed while returning to Direct files. Try again.",
        ));
    }

    let shutdown = match shutdown(&slot) {
        Ok(shutdown) => shutdown,
        Err(error) => {
            // No archive has started; the live slot remains usable when a
            // force-stop itself could not be completed.
            commit_transition(transition, || {
                state
                    .graphs
                    .write()
                    .unwrap()
                    .bind(label.to_string(), slot)
                    .map_err(|restore| crate::command_error::CommandError::sync_runtime({
                        format!(
                            "{error}; Tine-managed storage could not be restored in memory: {restore}"
                        )
                    }))?;
                crate::state::poke_watcher(state);
                Ok(())
            })?;
            return Err(error);
        }
    };
    after_shutdown(&shutdown)?;

    // `clean_shutdown` consumes the live actor even when it succeeds.  If a
    // later archive step fails, re-publishing that old slot would advertise a
    // dead handle.  Publish a fresh no-handle retry route, then release every
    // reference to the retired actor before touching its storage.
    let retryable = publish_stopped_managed_recovery_slot(
        state,
        label,
        direct_root.clone(),
        graph_meta,
        shutdown.retry_detail(),
        transition,
    )?;
    let retry_generation = retryable.binding_generation;
    drop(removed);
    drop(slot);

    let provider_archive = match archive_provider(&direct_root) {
        Ok(archive) => archive,
        Err(error) => return Err(error),
    };
    if let Err(error) = archive(private_root, recovery_root) {
        let reason = match restore_graph_provider_namespace(provider_archive) {
            Ok(()) => error,
            Err(restore) => {
                crate::command_error::CommandError::sync_runtime(format!("{error}; {restore}"))
            }
        };
        return Err(reason);
    }

    let binding_generation = commit_transition(transition, || {
        publish_direct(&direct_root, approved_assets)
    }).map_err(|error| crate::command_error::CommandError::sync_runtime({
        // Before Direct publication the retryable no-actor slot remains the
        // only authority.  Remove only that slot; if a later lifecycle step
        // already published Direct Files, leave its usable binding intact.
        let removed = {
            let mut graphs = state.graphs.write().unwrap();
            graphs
                .slot(label)
                .is_some_and(|slot| slot.binding_generation == retry_generation)
                .then(|| graphs.remove(label))
                .flatten()
        };
        if removed.is_some() {
            crate::state::poke_watcher(state);
        }
        format!(
            "Tine-managed storage recovery state was preserved, but Direct files could not reopen: {error}. Restart Tine to reopen the unchanged Markdown/Org graph."
        )
    }))?;
    let status = SparseV2StatusDto::legacy(binding_generation);
    Ok(SparseV2CancelResult {
        binding_generation,
        status,
        recovery_statement: match shutdown.completion_note() {
            Some(note) => format!(
                "Direct file mode is active. Complete managed-storage recovery state was preserved. {note}"
            ),
            None => "Direct file mode is active. Complete managed-storage recovery state was preserved.".into(),
        },
    })
}

fn cancel_sparse_v2_at_paths_with_archive(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(
        &crate::state::GraphSlot,
    ) -> Result<DirectFilesShutdown, crate::command_error::CommandError>,
    archive: impl FnOnce(&Path, &Path) -> Result<Option<PathBuf>, crate::command_error::CommandError>,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
    cancel_sparse_v2_at_paths_with_archive_and_publish(
        state,
        label,
        slot,
        private_root,
        recovery_root,
        approved_assets,
        shutdown,
        archive,
        archive_graph_provider_namespace,
        |direct_root, approved_assets| {
            let graph =
                tine_core::model::Graph::open_checked_with_assets(direct_root, approved_assets)
                    .map_err(crate::command_error::CommandError::from)?;
            let replacement = Arc::new(crate::state::GraphSlot::new(
                graph,
                direct_root.to_path_buf(),
            ));
            let binding_generation = replacement.binding_generation;
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)
                .map_err(crate::command_error::CommandError::from)?;
            crate::state::poke_watcher(state);
            Ok(binding_generation)
        },
        None,
        |_| Ok(()),
    )
}

/// The path-level shape of adoption's first half, wired to exactly the
/// production provider-namespace decision. `set_aside_managed_history_for_adoption`
/// composes the same call with the supervisor guard around it.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn set_aside_managed_history_at_paths(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(
        &crate::state::GraphSlot,
    ) -> Result<DirectFilesShutdown, crate::command_error::CommandError>,
    archive: impl FnOnce(&Path, &Path) -> Result<Option<PathBuf>, crate::command_error::CommandError>,
    publish_direct: impl FnOnce(&Path, Option<&Path>) -> Result<u64, crate::command_error::CommandError>,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
    cancel_sparse_v2_at_paths_with_archive_and_publish(
        state,
        label,
        slot,
        private_root,
        recovery_root,
        approved_assets,
        shutdown,
        archive,
        preserve_graph_provider_namespace,
        publish_direct,
        None,
        |_| Ok(()),
    )
}

fn cancel_sparse_v2_at_paths(
    state: &crate::state::AppState,
    label: &str,
    slot: Arc<crate::state::GraphSlot>,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    shutdown: impl FnOnce(
        &crate::state::GraphSlot,
    ) -> Result<DirectFilesShutdown, crate::command_error::CommandError>,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
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

/// Android app-UID boundary for the final leg of the managed-storage journey.
///
/// The core journey proves activation/share/join/reopen. This function then
/// drives the SAME graceful Return-to-Direct-Files composition as the command:
/// stop the live actor, preserve the complete private root, apply the
/// production provider-namespace policy, and publish one Direct Files slot.
#[cfg(all(target_os = "android", debug_assertions))]
pub(crate) fn run_android_managed_return_to_direct_files(
    graph_root: &Path,
    private_root: &Path,
    open_request: SyncRuntimeOpenRequest,
) -> Result<String, crate::command_error::CommandError> {
    let markdown_path = graph_root.join(tine_core::managed_storage_journey::JOURNEY_EDITED_PAGE);
    let markdown_before = std::fs::read(&markdown_path).map_err(|error| {
        crate::command_error::CommandError::sync_runtime(format!(
            "Return journey could not read its Markdown witness: {error}"
        ))
    })?;
    let opened = SyncRuntimeHandle::open(open_request);
    if opened.status != SyncRuntimeOpenStatus::Active {
        return Err(crate::command_error::CommandError::sync_runtime(format!(
            "Return journey could not reopen managed storage: {:?}",
            opened.status
        )));
    }
    let graph_meta = tine_core::model::Graph::open_checked(graph_root)
        .map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Return journey could not inspect Direct Files: {error}"
            ))
        })?
        .meta();
    let slot = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        SparseV2Binding::from_open(opened),
        graph_root.to_path_buf(),
        graph_meta,
    ));
    let state = crate::state::AppState {
        graphs: std::sync::RwLock::new(crate::state::GraphRegistry::default()),
        storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(),
        watch_ctl: Mutex::new(None),
        last_focused: Mutex::new(None),
        capture_graph: Mutex::new(Default::default()),
        sync_runtime: SyncRuntimeFacade,
        #[cfg(desktop)]
        next_window: std::sync::atomic::AtomicU64::new(1),
    };
    state
        .graphs
        .write()
        .unwrap()
        // No `map_err` here: `GraphRegistry::bind` is typed (W4-E2) and this
        // helper now returns `CommandError` too (W4-E2b), so `?` is the whole
        // conversion. Nothing on a Linux host compiles this
        // `#[cfg(target_os = "android")]` body — CI is the first gate that
        // does, which is how the E2 version of this line broke Android twice.
        .bind("managed-storage-smoke".into(), Arc::clone(&slot))?;

    let private_name = private_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("managed-storage-smoke");
    let recovery_root = private_root
        .parent()
        .unwrap_or(private_root)
        .join(format!(".{private_name}-return-recovery"));
    let _ = std::fs::remove_dir_all(&recovery_root);
    let proof = (|| {
        let returned = cancel_sparse_v2_at_paths(
            &state,
            "managed-storage-smoke",
            slot,
            private_root,
            &recovery_root,
            None,
            shutdown_for_graceful_direct_files,
        )?;
        let direct = state
            .graphs
            .read()
            .unwrap()
            .slot("managed-storage-smoke")
            .ok_or_else(|| {
                crate::command_error::CommandError::prose("Return journey published no graph slot")
            })?;
        direct.legacy_graph().map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Return journey did not publish Direct Files: {error}"
            ))
        })?;
        if private_root.exists() {
            return Err(crate::command_error::CommandError::prose(
                "Return journey left the live managed private root in place",
            ));
        }
        let archived = std::fs::read_dir(&recovery_root)
            .map_err(|error| {
                crate::command_error::CommandError::sync_runtime(format!(
                    "Return journey published no private-state archive: {error}"
                ))
            })?
            .count();
        if archived != 1 {
            return Err(crate::command_error::CommandError::sync_runtime(format!(
                "Return journey expected one private-state archive, found {archived}"
            )));
        }
        let markdown_after = std::fs::read(&markdown_path).map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Return journey lost its Markdown witness: {error}"
            ))
        })?;
        if markdown_after != markdown_before {
            return Err(crate::command_error::CommandError::prose(
                "Return journey changed the authoritative Markdown witness",
            ));
        }
        Ok(format!(
            "return_to_direct=ok binding_generation={} private_archives={archived}",
            returned.binding_generation
        ))
    })();
    let cleanup = if recovery_root.exists() {
        std::fs::remove_dir_all(&recovery_root).map_err(|error| {
            crate::command_error::CommandError::sync_runtime(format!(
                "Return journey cleanup failed after proof: {error}"
            ))
        })
    } else {
        Ok(())
    };
    match (proof, cleanup) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(crate::command_error::CommandError::sync_runtime(
            format!("{error}; {cleanup}"),
        )),
    }
}

#[cfg(test)]
fn cold_recovery_graph_meta(
    private_root: &Path,
    root_key: &Path,
) -> Result<GraphMeta, crate::command_error::CommandError> {
    match read_binding_at(&private_root.join(SPARSE_BINDING_FILE), root_key) {
        Ok(Some(record)) => Ok(SyncRuntimeFacade::graph_meta(&record)),
        // A second device is expected to have shared graph-local state before
        // it has any private binding.  The explicit Direct Files escape also
        // has to work when a failed setup left that private binding corrupt.
        // The Markdown graph supplies the reservation metadata; both the
        // private directory (if any) and the complete graph-local v2 namespace
        // are still archived byte-for-byte before Direct Files is published.
        Ok(None) | Err(_) => tine_core::model::Graph::open_checked(root_key)
            .map(|graph| graph.meta())
            .map_err(|error| crate::command_error::CommandError::sync_runtime({
                format!(
                    "Tine could not verify the Markdown/Org graph before returning to Direct files: {error}. Nothing was changed."
                )
            })),
    }
}

/// Legacy cold-return fixture retained only to exercise archival rollback.
/// Production emergency return never reserves a managed slot.
#[cfg(test)]
fn reserve_cold_recovery_slot(
    state: &crate::state::AppState,
    label: &str,
    root_key: PathBuf,
    graph_meta: GraphMeta,
) -> Result<Arc<crate::state::GraphSlot>, crate::command_error::CommandError> {
    let mut graphs = state.graphs.write().unwrap();
    if graphs.slot(label).is_some() {
        return Err(crate::command_error::CommandError::sync_runtime(
            "This window already owns a graph while recovery was requested. Nothing was changed; retry from the current recovery panel.",
        ));
    }
    if graphs.entries().into_iter().any(|(_, slot)| {
        slot.root_key.starts_with(&root_key) || root_key.starts_with(&slot.root_key)
    }) {
        return Err(crate::command_error::CommandError::sync_runtime(
            "The remembered graph is already open or opening in another window. Nothing was changed.",
        ));
    }
    let slot = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        retryable_binding(
            "local_active",
            "Cold-start recovery reserved this managed binding before reopening Direct files."
                .into(),
        ),
        root_key,
        graph_meta,
    ));
    graphs
        .bind(label.to_string(), Arc::clone(&slot))
        .map_err(|_| {
            crate::command_error::CommandError::sync_runtime({
                "Tine could not reserve the remembered graph for recovery. Nothing was changed."
            })
        })?;
    drop(graphs);
    crate::state::poke_watcher(state);
    Ok(slot)
}

/// Legacy cold-return fixture retained only to exercise archival rollback.
/// Native operation IDs own production supersession.
#[cfg(test)]
fn exact_live_cold_recovery_slot(
    state: &crate::state::AppState,
    label: &str,
    root_key: &Path,
) -> Result<Option<Arc<crate::state::GraphSlot>>, crate::command_error::CommandError> {
    let slot = state.graphs.read().unwrap().slot(label);
    let Some(slot) = slot else {
        return Ok(None);
    };
    if slot.root_key != root_key {
        return Err(crate::command_error::CommandError::sync_runtime(
            "This recovery action is stale because the window opened a different graph. Nothing was changed.",
        ));
    }
    if !slot.is_sparse_v2() {
        return Err(crate::command_error::CommandError::sync_runtime(
            "This recovery action is stale because the window is already using Direct files. Nothing was changed.",
        ));
    }
    Ok(Some(slot))
}

#[cfg(test)]
fn cancel_sparse_v2_cold_at_paths_with_archive(
    state: &crate::state::AppState,
    label: &str,
    root_key: PathBuf,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
    archive: impl FnOnce(&Path, &Path) -> Result<Option<PathBuf>, crate::command_error::CommandError>,
) -> Result<SparseV2ColdCancelResult, crate::command_error::CommandError> {
    if let Some(slot) = exact_live_cold_recovery_slot(state, label, &root_key)? {
        return cancel_sparse_v2_at_paths_with_archive(
            state,
            label,
            slot,
            private_root,
            recovery_root,
            approved_assets,
            shutdown_for_direct_files_escape,
            archive,
        );
    }
    let graph_meta = cold_recovery_graph_meta(private_root, &root_key)?;
    let slot = reserve_cold_recovery_slot(state, label, root_key, graph_meta)?;
    cancel_sparse_v2_at_paths_with_archive(
        state,
        label,
        slot,
        private_root,
        recovery_root,
        approved_assets,
        shutdown_for_direct_files_escape,
        archive,
    )
}

#[cfg(test)]
fn cancel_sparse_v2_cold_at_paths(
    state: &crate::state::AppState,
    label: &str,
    root_key: PathBuf,
    private_root: &Path,
    recovery_root: &Path,
    approved_assets: Option<&Path>,
) -> Result<SparseV2ColdCancelResult, crate::command_error::CommandError> {
    cancel_sparse_v2_cold_at_paths_with_archive(
        state,
        label,
        root_key,
        private_root,
        recovery_root,
        approved_assets,
        archive_private_root,
    )
}

#[tauri::command]
pub(crate) async fn cancel_sparse_v2_cold(
    path: String,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<SparseV2ColdCancelResult, crate::command_error::CommandError> {
    let label = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || cancel_sparse_v2_cold_blocking(&app, &label, path))
        .await
        .map_err(|_| {
            crate::command_error::CommandError::prose(
                "Cold Direct Files recovery worker stopped before completion.",
            )
        })?
}

fn cancel_sparse_v2_cold_blocking(
    app: &tauri::AppHandle,
    label: &str,
    path: String,
) -> Result<SparseV2ColdCancelResult, crate::command_error::CommandError> {
    // Canonicalization is read-only. Emergency return intentionally does not
    // wait for the managed root lane: its first durable mutation selects the
    // current Markdown tree as Direct Files, and stale managed workers are
    // barred from later publication by the supervisor operation ID.
    let submitted_root = crate::state::canonical_graph_root(&path).map_err(|_| crate::command_error::CommandError::sync_runtime({
        "The selected recovery folder is unavailable. Retry graph lookup or choose another graph."
            .to_string()
    }))?;
    let state = app.state::<crate::state::AppState>();
    let operation_id =
        state
            .storage_supervisor
            .begin_emergency_return(app, label, submitted_root.clone())?;
    let mut failure_code = "target_validation_failed";
    let result = (|| {
        state.storage_supervisor.advance_transition(
            app,
            operation_id,
            StorageTransitionPhase::ValidatingTarget,
        )?;
        failure_code = "direct_selection_failed";
        state.storage_supervisor.advance_transition(
            app,
            operation_id,
            StorageTransitionPhase::QuarantiningManagedSelection,
        )?;
        publish_direct_selection(app, &submitted_root, "emergency_return")?;

        failure_code = "direct_open_failed";
        state.storage_supervisor.advance_transition(
            app,
            operation_id,
            StorageTransitionPhase::PublishingDirect,
        )?;
        let prepared = crate::graph::prepare_direct_files_open(app, submitted_root.clone())
            .map_err(|error| crate::command_error::CommandError::sync_runtime({
                format!(
                    "Direct Files was selected and managed evidence remains preserved, but the Markdown/Org graph could not open: {error}. Retry opening this graph or choose another graph."
                )
            }))?;
        state.storage_supervisor.commit_if_current(operation_id, || {
            crate::graph::publish_prepared_direct_files(app, label, &state, prepared)
                .map_err(|error| {
                    crate::command_error::CommandError::sync_runtime(format!(
                        "Direct Files was selected and managed evidence remains preserved, but the Markdown/Org graph could not publish: {error}. Retry opening this graph or choose another graph."
                    ))
                })
        })
    })();

    match result {
        Ok(direct) => {
            state.storage_supervisor.finish_transition(
                app,
                operation_id,
                StorageTransitionOutcome::Succeeded,
                Some(StableStorageMode::Direct),
                None,
            )?;
            Ok(SparseV2CancelResult {
                binding_generation: direct.binding_generation,
                status: SparseV2StatusDto::legacy(direct.binding_generation),
                recovery_statement: "Direct Files is active from the current Markdown/Org tree. Managed-storage evidence was left untouched and may contain operations newer than Markdown; it will never be silently reopened or merged.".into(),
            })
        }
        Err(error) => {
            let _ = state.storage_supervisor.finish_transition(
                app,
                operation_id,
                StorageTransitionOutcome::Failed,
                None,
                Some(failure_code.into()),
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) async fn cancel_sparse_v2(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        cancel_sparse_v2_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

fn cancel_sparse_v2_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2CancelResult, crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let root = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition = state.storage_supervisor.begin_recovery_guard(
        app,
        label,
        root.clone(),
        StorageTransitionKind::ReturnGracefully,
    )?;
    transition.advance(StorageTransitionPhase::WaitingForTransition)?;
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    if !transition.is_current() {
        return Err(crate::command_error::CommandError::prose(
            "the graceful Direct Files return was superseded while waiting",
        ));
    }
    transition.advance(StorageTransitionPhase::ValidatingTarget)?;
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if slot.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while return waited for its transition lane",
        ));
    }
    let private_root = sparse_private_root(&app, &slot.root_key)?;
    let recovery_root = sparse_recovery_root(&app)?;
    let approved_assets = crate::settings::approved_external_assets(&app, &slot.root_key);
    transition.advance(StorageTransitionPhase::DrainingManaged)?;
    let result = cancel_sparse_v2_at_paths_with_archive_and_publish(
        &state,
        label,
        slot,
        &private_root,
        &recovery_root,
        approved_assets.as_deref(),
        shutdown_for_graceful_direct_files,
        archive_private_root,
        archive_graph_provider_namespace,
        |direct_root, approved_assets| {
            let graph =
                tine_core::model::Graph::open_checked_with_assets(direct_root, approved_assets)
                    .map_err(crate::command_error::CommandError::from)?;
            let replacement = Arc::new(crate::state::GraphSlot::new(
                graph,
                direct_root.to_path_buf(),
            ));
            let binding_generation = replacement.binding_generation;
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)
                .map_err(crate::command_error::CommandError::from)?;
            crate::state::poke_watcher(&state);
            Ok(binding_generation)
        },
        Some(&transition),
        |_| {
            transition.advance(StorageTransitionPhase::ConfirmingProjection)?;
            transition.advance(StorageTransitionPhase::PublishingDirect)
        },
    );
    match result {
        Ok(result) => {
            transition.succeed(StableStorageMode::Direct)?;
            Ok(result)
        }
        Err(error) => {
            transition.fail("graceful_return_failed");
            Err(error)
        }
    }
}

/// Publish the already-safe local archive into the shared namespace. Core
/// deliberately retires the local-only actor after committing the enrollment
/// transition, so the Tauri composition must reopen that durable state and
/// atomically replace the serving slot before reporting success.
#[tauri::command]
pub(crate) async fn prepare_sparse_v2_share(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        prepare_sparse_v2_share_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

fn prepare_sparse_v2_share_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let root = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if slot.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while share setup waited for its transition lane",
        ));
    }
    let record = state
        .sync_runtime
        .binding_record(app, &root)?
        .ok_or_else(|| {
            crate::command_error::CommandError::prose("Tine-managed storage setup is missing.")
        })?;
    active_handle(&slot)?
        .prepare_shared()
        .map_err(crate::command_error::CommandError::from)?;
    let candidate = prepare_reopened_managed_candidate(
        app,
        &state,
        slot,
        record,
        "share enrollment runtime reopen",
    )?;
    publish_managed_candidate(app, &state, label, candidate)
}

/// The exact file a joining device is waiting for.
pub(crate) fn shared_enrollment_descriptor_path(graph_root: &Path) -> PathBuf {
    graph_root
        .join(".tine-sync/v2/shared/outbox")
        .join(SHARED_ENROLLMENT_DESCRIPTOR_PATH)
}

/// Fixed-shape wire refusal for absent shared data. The frontend owns the safe
/// wording and actionable provider-path remedy.
pub(crate) fn shared_enrollment_not_here_yet(
    _graph_root: &Path,
) -> crate::command_error::CommandError {
    crate::command_error::CommandError::tagged("sync-data-unavailable", None::<String>, None)
}

pub(crate) fn adoption_archived_error() -> crate::command_error::CommandError {
    crate::command_error::CommandError::tagged("adoption-archived", None::<String>, None)
}

/// Reconstitute the sole application actor after a durable enrollment cut.
/// `prepare_shared` and a completed `join_shared` intentionally stop their
/// predecessor actor: continuing to serve that handle would mix two enrollment
/// epochs. This helper therefore proves the replacement through ordinary page
/// APIs before its caller publishes it with `replace_if_current`.
fn prepare_reopened_managed_candidate(
    app: &tauri::AppHandle,
    state: &crate::state::AppState,
    predecessor: Arc<crate::state::GraphSlot>,
    record: SparseV2ActivationRecord,
    stage: &str,
) -> Result<PreparedManagedCandidate, crate::command_error::CommandError> {
    let graph_meta = SyncRuntimeFacade::graph_meta(&record);
    let binding = state
        .sync_runtime
        .open_record(app, &record)
        .map_err(|error| {
            crate::command_error::CommandError::sync_runtime({
                let detail = format!("{stage} failed: {error}");
                crate::debug::diag(&detail);
                detail
            })
        })?;
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        binding,
        predecessor.root_key.clone(),
        graph_meta,
    ));
    let readiness = prove_managed_application_ready(&replacement, None).map_err(|error| {
        crate::command_error::CommandError::sync_runtime({
            let detail = format!("{stage} readiness failed: {error}");
            crate::debug::diag(&detail);
            detail
        })
    })?;
    Ok(PreparedManagedCandidate {
        predecessor,
        replacement,
        record,
        readiness,
        direct_source_generation: None,
    })
}

/// Explicitly retire the second device's legacy reader/watcher, derive its
/// private identity from exact provider descriptor evidence, and join.
#[tauri::command]
pub(crate) async fn join_sparse_v2_shared(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        join_sparse_v2_shared_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

fn join_sparse_v2_shared_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2StatusDto, crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let root = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition = state.storage_supervisor.begin_guard(
        app,
        label,
        root.clone(),
        StorageTransitionKind::JoinManaged,
    )?;
    transition.advance(StorageTransitionPhase::WaitingForTransition)?;
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    if !transition.is_current() {
        return Err(crate::command_error::CommandError::prose(
            "managed join was superseded while waiting for its graph lane",
        ));
    }
    transition.advance(StorageTransitionPhase::ValidatingTarget)?;
    transition.advance(StorageTransitionPhase::JoiningManaged)?;
    let prepared = match prepare_sparse_v2_join(app, label, binding_generation, root) {
        Ok(prepared) => prepared,
        Err(error) => {
            transition.fail("join_failed");
            return Err(error);
        }
    };
    let PreparedActivationOutcome::Candidate(candidate) = prepared else {
        let PreparedActivationOutcome::AlreadyCurrent(status) = prepared else {
            unreachable!()
        };
        transition.succeed(StableStorageMode::Managed)?;
        return Ok(status);
    };
    let state = app.state::<crate::state::AppState>();
    let (published, status) =
        transition.publish(|| publish_managed_candidate(app, &state, label, candidate))?;
    published.succeed(StableStorageMode::Managed)?;
    Ok(status)
}

fn prepare_sparse_v2_join(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
    root: PathBuf,
) -> Result<PreparedActivationOutcome, crate::command_error::CommandError> {
    fn join_failure(
        stage: &str,
        error: impl std::fmt::Display,
    ) -> crate::command_error::CommandError {
        let detail = error.to_string();
        crate::debug::diag(format!(
            "managed sync join failed: stage={stage}; detail={detail}"
        ));
        crate::command_error::CommandError::sync_runtime(format!(
            "managed sync join failed at {stage}: {detail}"
        ))
    }

    fn join_runtime_failure(
        stage: &str,
        error: tine_core::sync_runtime::SyncRuntimeRequestError,
    ) -> crate::command_error::CommandError {
        match error {
            tine_core::sync_runtime::SyncRuntimeRequestError::TaggedBackend(payload) => {
                crate::command_error::CommandError::from(
                    tine_core::sync_runtime::SyncRuntimeRequestError::TaggedBackend(payload),
                )
            }
            other => join_failure(stage, other),
        }
    }

    let state = app.state::<crate::state::AppState>();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if slot.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while join waited for its transition lane",
        ));
    }
    let descriptor =
        inspect_shared_enrollment_for_cold_discovery(&slot.root_key.join(".tine-sync/v2/shared"))
            .map_err(|error| join_failure("provider discovery", error))?
            .ok_or_else(|| shared_enrollment_not_here_yet(&slot.root_key))?;
    if slot.sparse_binding().is_some() {
        let record = state
            .sync_runtime
            .binding_record(app, &root)?
            .ok_or_else(|| {
                crate::command_error::CommandError::prose("Tine-managed storage setup is missing.")
            })?;
        active_handle(&slot)?
            .join_shared(descriptor)
            .map_err(|error| join_runtime_failure("provider scan", error))?;
        let candidate = prepare_reopened_managed_candidate(
            app,
            &state,
            slot,
            record,
            "joined enrollment runtime reopen",
        )?;
        return Ok(PreparedActivationOutcome::Candidate(candidate));
    }
    let graph = slot
        .legacy_graph()
        .map_err(crate::command_error::CommandError::from)?;
    let graph_meta = graph.meta();
    let direct_source_generation = graph.guarded_graph_text_identity_report().generation;
    drop(graph);
    let record = state.sync_runtime.prepare_shared_binding_record(
        app,
        &slot.root_key,
        graph_meta.clone(),
        &descriptor,
    )?;

    let activated = activate_record_with_diagnostics(
        &state.sync_runtime,
        app,
        label,
        binding_generation,
        &record,
    )
    .map_err(|error| join_failure("local activation", error))?;
    let Some(handle) = activated.binding.handle() else {
        return Err(crate::command_error::CommandError::sync_runtime(
            join_bootstrap_unavailable_detail(activated.binding.availability()),
        ));
    };
    handle
        .join_shared(descriptor)
        .map_err(|error| join_runtime_failure("provider scan", error))?;
    let binding = state
        .sync_runtime
        .open_record(app, &record)
        .map_err(|error| join_failure("runtime reopen", error))?;
    let replacement = Arc::new(crate::state::GraphSlot::from_sparse_v2(
        binding,
        slot.root_key.clone(),
        graph_meta,
    ));
    let readiness =
        prove_managed_application_ready(&replacement, activated.largest_page_path.as_deref())?;
    Ok(PreparedActivationOutcome::Candidate(
        PreparedManagedCandidate {
            predecessor: slot,
            replacement,
            record,
            readiness,
            direct_source_generation: Some(direct_source_generation),
        },
    ))
}

/// Keep the retry's useful stage and reason in ordinary bounded text. Debug-
/// formatting the tagged availability object wrapped both in braces, so the
/// frontend's privacy sanitizer correctly collapsed the entire diagnostic to
/// `[details]` and a phone report could not identify the failing operation.
fn join_bootstrap_unavailable_detail(availability: &SparseV2Availability) -> String {
    match availability {
        SparseV2Availability::Retryable { stage, detail } => format!(
            "join bootstrap did not reach LocalActive during {stage}: {detail}"
        ),
        SparseV2Availability::Blocked {
            reason_code,
            scenario_id,
        } => format!(
            "join bootstrap did not reach LocalActive: blocked ({reason_code}, scenario {scenario_id})"
        ),
        SparseV2Availability::Refused {
            reason_code,
            scenario_id,
            detail,
        } => format!(
            "join bootstrap did not reach LocalActive: refused ({reason_code}, scenario {scenario_id}){}",
            detail
                .as_deref()
                .map(|detail| format!(": {detail}"))
                .unwrap_or_default(),
        ),
        other => format!(
            "join bootstrap did not reach LocalActive: unexpected availability {}",
            match other {
                SparseV2Availability::LegacyDefault => "legacy_default",
                SparseV2Availability::Joinable { .. } => "joinable",
                SparseV2Availability::Active => "active_without_handle",
                SparseV2Availability::Retryable { .. }
                | SparseV2Availability::Blocked { .. }
                | SparseV2Availability::Refused { .. } => unreachable!(),
            }
        ),
    }
}

/// How this device's own managed identity stands to the graph another device
/// is sharing.
///
/// The three identities are minted together by one activation
/// (`SparseV2ActivationRecord::new`) and copied together from a descriptor
/// (`from_shared`), so in practice they either all agree or all differ. A
/// mixture is evidence that one side's identity was rewritten or truncated,
/// which is exactly the state adoption must not guess about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedGraphRelation {
    /// Every identity matches: this device already holds the shared lineage.
    SameGraph,
    /// No identity matches: two independent activations of the same folder.
    Independent,
    /// Some identities match and some do not.
    PartialLineage,
}

fn shared_graph_relation(
    local: (WorkspaceId, LineageDigest, DocumentId),
    shared: (WorkspaceId, LineageDigest, DocumentId),
) -> SharedGraphRelation {
    let matches = [
        local.0 == shared.0,
        local.1 == shared.1,
        local.2 == shared.2,
    ];
    if matches.iter().all(|matched| *matched) {
        return SharedGraphRelation::SameGraph;
    }
    if matches.iter().all(|matched| !*matched) {
        return SharedGraphRelation::Independent;
    }
    SharedGraphRelation::PartialLineage
}

/// Both non-adoptable relations, said in the words the panel repeats.
fn shared_graph_relation_refusal(relation: SharedGraphRelation) -> Option<&'static str> {
    match relation {
        SharedGraphRelation::Independent => None,
        SharedGraphRelation::SameGraph => Some(
            "This device's Tine-managed storage already holds the same history the other device is sharing, so there is nothing to set aside. Nothing was changed. Use \"Join a synced graph from another device\" instead.",
        ),
        SharedGraphRelation::PartialLineage => Some(
            "This device's Tine-managed storage matches part of the shared graph's identity and not the rest, so Tine cannot tell which history is which. Nothing was changed. Let your file-sync tool finish delivering the hidden .tine-sync folder and try again; if it persists, return this device to Direct files and join from there.",
        ),
    }
}

/// Adoption abandons this device's own managed history. When that history is
/// itself shared, abandoning it also abandons whatever other devices are
/// joined to it, so adoption refuses instead of deciding that for them.
fn shared_cut_refusal(shared_phase: Option<&str>) -> Option<&'static str> {
    match shared_phase? {
        "share_prepared" => Some(
            "This device's Tine-managed storage has an unfinished sync cut of its own. Nothing was changed. Finish it with \"Retry setup\", or return this device to Direct files, before adopting another device's graph.",
        ),
        "joining" => Some(
            "This device's Tine-managed storage is part-way through a join already. Nothing was changed. Let that join finish or fail first.",
        ),
        _ => Some(
            "This device's Tine-managed storage is already shared with, or joined to, another device. Nothing was changed. Adopting a different graph would abandon that; return this device to Direct files first if that is what you want.",
        ),
    }
}

/// What adoption did, and where the predecessor went.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub(crate) struct SparseV2AdoptionResult {
    status: SparseV2StatusDto,
    binding_generation: u64,
    /// The archived pre-adoption managed history, named in full. `None` only
    /// when a failed or partially-created activation had retained nothing.
    archive_location: Option<String>,
    adoption_statement: String,
}

/// Where a set-aside managed history is archived. Stable, and knowable before
/// the point of no return, which is why the panel can name it in the
/// confirmation rather than only in the receipt.
#[tauri::command]
pub(crate) async fn sparse_v2_recovery_location(
    app: tauri::AppHandle,
) -> Result<String, crate::command_error::CommandError> {
    sparse_recovery_root(&app).map(|root| root.display().to_string())
}

/// Adopt a graph another device is sharing on a device that already holds a
/// managed graph of its own.
///
/// This is a composition, not a new storage operation: the graceful Direct
/// Files return already archives this device's complete managed history under
/// `sparse-v2-recovery`, and the Direct Files join branch already bootstraps a
/// binding out of provider evidence. Adoption sequences exactly those two,
/// with one difference that a literal "Return to Direct files, then Join"
/// cannot express — the return must NOT archive `<graph>/.tine-sync/v2`,
/// because that subtree is the other device's shared evidence and archiving it
/// removes the very descriptor the join is about to read.
///
/// Each half is a complete supervisor transition with its own stable end mode.
/// A crash between them therefore lands on Direct Files with the predecessor
/// archived and the shared graph still joinable, which is a state the panel
/// already offers a button for.
#[tauri::command]
pub(crate) async fn adopt_sparse_v2_shared(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2AdoptionResult, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        adopt_sparse_v2_shared_blocking(&app, &label, binding_generation)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

fn adopt_sparse_v2_shared_blocking(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<SparseV2AdoptionResult, crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    let root = slot.root_key.clone();
    if slot.sparse_binding().is_none() {
        return Err(crate::command_error::CommandError::sync_runtime(
            "This device is already using Direct files, so it has no Tine-managed storage history to set aside. Nothing was changed. Use \"Join a synced graph from another device\" instead.",
        ));
    }
    // An incomplete provider tree is "not yet", not a reason to start
    // dismantling this device's own storage.
    crate::graph::refuse_unclaimed_sparse_archive(&root)?;
    let descriptor =
        inspect_shared_enrollment_for_cold_discovery(&root.join(".tine-sync/v2/shared"))
            .map_err(|error| {
                crate::command_error::CommandError::sync_runtime(format!(
                    "Couldn't read the shared sync data: {error}"
                ))
            })?
            .ok_or_else(|| shared_enrollment_not_here_yet(&root))?;
    let record = state
        .sync_runtime
        .binding_record(app, &root)?
        .ok_or_else(|| {
            crate::command_error::CommandError::prose("Tine-managed storage setup is missing.")
        })?;
    let relation = shared_graph_relation(
        (
            record.workspace_id,
            record.lineage_digest,
            record.catalog_document_id,
        ),
        (
            descriptor.workspace_id,
            descriptor.lineage_digest,
            descriptor.catalog_document_id,
        ),
    );
    if let Some(refusal) = shared_graph_relation_refusal(relation) {
        return Err(crate::command_error::CommandError::prose(refusal));
    }
    let status = sparse_v2_status_for_slot(&slot)?;
    if let Some(refusal) = shared_cut_refusal(
        status
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.shared_phase.as_deref()),
    ) {
        return Err(crate::command_error::CommandError::prose(refusal));
    }
    drop(slot);

    let (set_aside, archive_location) =
        set_aside_managed_history_for_adoption(app, label, binding_generation)?;
    let archive_location = archive_location.map(|path| path.display().to_string());
    // The fixed kind lets the panel attach the archive location it already
    // read without repeating a native path through the error boundary.
    let status = join_sparse_v2_shared_blocking(app, label, set_aside.binding_generation).map_err(
        |error| {
            crate::debug::diag(format!("managed adoption failed after archive: {error}"));
            adoption_archived_error()
        },
    )?;
    let binding_generation = status.binding_generation;
    Ok(SparseV2AdoptionResult {
        adoption_statement: match &archive_location {
            Some(location) => format!(
                "This device now serves the graph shared by your other device. Its own previous Tine-managed history was archived at {location} and was not merged."
            ),
            None => "This device now serves the graph shared by your other device. It had no retained managed history to archive, and nothing was merged.".into(),
        },
        status,
        binding_generation,
        archive_location,
    })
}

/// The first half of adoption: drain and stop this device's managed runtime,
/// archive its complete private managed root, and publish Direct Files from
/// the unchanged Markdown/Org tree — leaving `<graph>/.tine-sync/v2` exactly
/// where it is so the second half can still read the shared descriptor.
fn set_aside_managed_history_for_adoption(
    app: &tauri::AppHandle,
    label: &str,
    binding_generation: u64,
) -> Result<(SparseV2CancelResult, Option<PathBuf>), crate::command_error::CommandError> {
    let state = app.state::<crate::state::AppState>();
    let root = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?
        .root_key
        .clone();
    let transition = state.storage_supervisor.begin_recovery_guard(
        app,
        label,
        root.clone(),
        StorageTransitionKind::ReturnGracefully,
    )?;
    transition.advance(StorageTransitionPhase::WaitingForTransition)?;
    let transition_gate = state.storage_supervisor.transition_lane(&root);
    let _transition = transition_gate.lock().unwrap();
    if !transition.is_current() {
        return Err(crate::command_error::CommandError::prose(
            "adoption was superseded while waiting for its graph lane",
        ));
    }
    transition.advance(StorageTransitionPhase::ValidatingTarget)?;
    let slot = crate::state::slot_for_bound_window(&state, label, Some(binding_generation))
        .map_err(crate::command_error::CommandError::from)?;
    if slot.root_key != root {
        return Err(crate::command_error::CommandError::prose(
            "graph changed while adoption waited for its transition lane",
        ));
    }
    let private_root = sparse_private_root(app, &slot.root_key)?;
    let recovery_root = sparse_recovery_root(app)?;
    let approved_assets = crate::settings::approved_external_assets(app, &slot.root_key);
    transition.advance(StorageTransitionPhase::DrainingManaged)?;
    let archived = std::cell::RefCell::new(None);
    let result = cancel_sparse_v2_at_paths_with_archive_and_publish(
        &state,
        label,
        slot,
        &private_root,
        &recovery_root,
        approved_assets.as_deref(),
        shutdown_for_graceful_direct_files,
        |private_root, recovery_root| {
            let destination = archive_private_root(private_root, recovery_root)?;
            *archived.borrow_mut() = destination.clone();
            Ok(destination)
        },
        preserve_graph_provider_namespace,
        |direct_root, approved_assets| {
            let graph =
                tine_core::model::Graph::open_checked_with_assets(direct_root, approved_assets)
                    .map_err(crate::command_error::CommandError::from)?;
            let replacement = Arc::new(crate::state::GraphSlot::new(
                graph,
                direct_root.to_path_buf(),
            ));
            let binding_generation = replacement.binding_generation;
            state
                .graphs
                .write()
                .unwrap()
                .bind(label.to_string(), replacement)
                .map_err(crate::command_error::CommandError::from)?;
            crate::state::poke_watcher(&state);
            Ok(binding_generation)
        },
        Some(&transition),
        |_| {
            transition.advance(StorageTransitionPhase::ConfirmingProjection)?;
            transition.advance(StorageTransitionPhase::PublishingDirect)
        },
    );
    match result {
        Ok(result) => {
            transition.succeed(StableStorageMode::Direct)?;
            Ok((result, archived.into_inner()))
        }
        Err(error) => {
            transition.fail("adoption_set_aside_failed");
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) async fn sparse_v2_query(
    request: tine_core::sync_runtime::SyncRuntimeQueryRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncRuntimeQueryReply, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .query(request)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn sparse_v2_editor_load(
    request: tine_core::sync_runtime::SyncEditorLoadRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncEditorLoadOutcome, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .load_editor_page(request)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn sparse_v2_editor_save(
    request: tine_core::sync_runtime::SyncEditorSaveRequest,
    state: crate::state::GraphContext<'_>,
) -> Result<tine_core::sync_runtime::SyncEditorSaveOutcome, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .save_editor_page(request)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn sparse_v2_tick(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2TickDto, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        let tick = active_handle(&slot)?
            .tick()
            .map_err(crate::command_error::CommandError::from)?;
        if let SyncRuntimeTick::CheckpointCaptureSkipped { reason } = &tick {
            crate::debug::record_checkpoint_capture_skip(*reason);
        }
        Ok(tick_dto(tick))
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn list_absence_sweeps(
    state: crate::state::GraphContext<'_>,
) -> Result<Vec<tine_core::sync_runtime::SyncAbsenceSweepEvent>, crate::command_error::CommandError>
{
    // managed-command-routing: managed. Absence sweeps exist only under managed
    // storage and this reaches the sparse actor through `active_handle` +
    // `ActorRequest`, not `sparse_application_handle`, so the source scanner
    // cannot see the route. Declaring NoGraphSlot here would be false: restore
    // and reapply change graph content.
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .absence_sweep_events()
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn reapply_absence_sweep(
    sweep_id: String,
    state: crate::state::GraphContext<'_>,
) -> Result<
    tine_core::sync_runtime::SyncAbsenceSweepActionOutcome,
    crate::command_error::CommandError,
> {
    // managed-command-routing: managed. Absence sweeps exist only under managed
    // storage and this reaches the sparse actor through `active_handle` +
    // `ActorRequest`, not `sparse_application_handle`, so the source scanner
    // cannot see the route. Declaring NoGraphSlot here would be false: restore
    // and reapply change graph content.
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .reapply_absence_sweep(&sweep_id)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn restore_absence_sweep(
    sweep_id: String,
    state: crate::state::GraphContext<'_>,
) -> Result<
    tine_core::sync_runtime::SyncAbsenceSweepRestoreOutcome,
    crate::command_error::CommandError,
> {
    // managed-command-routing: managed. Absence sweeps exist only under managed
    // storage and this reaches the sparse actor through `active_handle` +
    // `ActorRequest`, not `sparse_application_handle`, so the source scanner
    // cannot see the route. Declaring NoGraphSlot here would be false: restore
    // and reapply change graph content.
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .restore_absence_sweep(&sweep_id)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn keep_absence_sweep_deletion(
    sweep_id: String,
    state: crate::state::GraphContext<'_>,
) -> Result<(), crate::command_error::CommandError> {
    // managed-command-routing: managed. Absence sweeps exist only under managed
    // storage and this reaches the sparse actor through `active_handle` +
    // `ActorRequest`, not `sparse_application_handle`, so the source scanner
    // cannot see the route. Declaring NoGraphSlot here would be false: restore
    // and reapply change graph content.
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .dispose_absence_sweep_keep_deletion(&sweep_id)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
}

#[tauri::command]
pub(crate) async fn sparse_v2_clean_shutdown(
    state: crate::state::GraphContext<'_>,
) -> Result<SparseV2RuntimeStatusDto, crate::command_error::CommandError> {
    let (app, label, binding_generation) = crate::state::owned_graph_context(state)
        .map_err(crate::command_error::CommandError::from)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::state::AppState>();
        let slot = crate::state::slot_for_bound_window(&state, &label, Some(binding_generation))
            .map_err(crate::command_error::CommandError::from)?;
        active_handle(&slot)?
            .clean_shutdown()
            .map(shutdown_status)
            .map_err(crate::command_error::CommandError::from)
    })
    .await
    .map_err(crate::command_error::CommandError::worker)?
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

fn clean_shutdown_outcome(
    outcome: SyncShutdownOutcome,
) -> Result<CleanShutdownSlot, crate::command_error::CommandError> {
    match outcome {
        SyncShutdownOutcome::Safe(_) => Ok(CleanShutdownSlot::Safe),
        SyncShutdownOutcome::Terminal(snapshot) => {
            Err(crate::command_error::CommandError::sync_runtime(format!(
            "Tine-managed storage reached a terminal state and cannot authorize process exit: {}",
            snapshot
                .detail
                .unwrap_or_else(|| "no terminal detail was recorded".into())
        )))
        }
    }
}

pub(crate) fn clean_shutdown_slot(
    slot: &crate::state::GraphSlot,
) -> Result<CleanShutdownSlot, crate::command_error::CommandError> {
    let Some(handle) = slot.sparse_runtime() else {
        return Ok(CleanShutdownSlot::Direct);
    };
    handle
        .clean_shutdown()
        .map_err(crate::command_error::CommandError::from)
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
            lifecycle: lifecycle.clone(),
            recovery: None,
            watcher: Default::default(),
            last_tick: None,
            detail: detail.map(str::to_owned),
            shared_role: None,
            shared_phase: None,
            provider_pending: 0,
            provider_runnable: false,
            search_index_building: false,
            move_episode_cleanup_pending: false,
            managed_local_pending: 0,
            managed_local_checkpointed_sequence: 0,
            managed_local_next_sequence: 0,
            managed_local_stage: None,
            history_recovery: None,
            history_recovery_diagnostics: None,
            application_pages_writable: matches!(lifecycle, SyncRuntimeLifecycle::Active),
            checkpoint_diagnostics: None,
            sweep_deadline_remaining: None,
            sweep_deadline_due: false,
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
        SyncApplicationMoveAdmission, SyncApplicationMovePlacement, SyncApplicationMoveRoot,
        SyncApplicationPageLoadOutcome, SyncApplicationPageLoadRequest,
        SyncApplicationPageSaveOutcome, SyncApplicationPageSaveRequest,
        SyncApplicationPageSaveTarget, SyncApplicationPageSelector, SyncEditorBlockDto,
        SyncEditorBlockKey, SyncEditorLoadOutcome, SyncEditorLoadRequest, SyncEditorPageSelector,
        SyncEditorSaveOutcome, SyncEditorSaveRequest, SyncEditorSaveTarget, SyncEntityId,
        SyncLocalMutationOutcome, SyncPageKind, SyncPageNameResolutionDto, SyncRuntimeQueryReply,
        SyncRuntimeQueryRequest, SyncSearchHitDto, SyncWatcherObservation,
    };

    #[test]
    fn checkpoint_capture_skip_has_its_own_tick_value() {
        let skipped = tick_dto(SyncRuntimeTick::CheckpointCaptureSkipped {
            reason: tine_core::sync_runtime::SyncCheckpointCaptureSkip::RuntimeNotAttached,
        });
        let recovering = tick_dto(SyncRuntimeTick::Recovering);
        assert_eq!(skipped.state, "checkpoint_capture_skipped");
        assert_ne!(
            skipped.state, recovering.state,
            "I-11: a skipped disposable checkpoint capture is not recovery; keep the diagnostics tick vocabulary distinct"
        );
        let contract = include_str!("../../docs/contracts/diagnostics.md");
        assert!(
            contract.contains("`checkpoint_capture_skipped`")
                && contract.contains("checkpoint_capture_skip_has_its_own_tick_value"),
            "I-11: the diagnostics contract and native tick vocabulary must change together"
        );
    }

    #[test]
    fn emergency_direct_selection_is_atomic_sticky_and_graph_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        std::fs::create_dir_all(&graph).unwrap();
        let receipt = direct_selection_path_at(temp.path(), &graph);
        assert!(!direct_selection_is_active_at(&receipt, &graph).unwrap());
        publish_direct_selection_at(&receipt, &graph, "emergency_return").unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());
        let other = temp.path().join("other");
        let other_receipt = direct_selection_path_at(temp.path(), &other);
        assert!(!direct_selection_is_active_at(&other_receipt, &other).unwrap());
    }

    #[test]
    fn blank_slate_rebuild_intent_is_durable_and_distinct_from_explicit_direct() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        let receipt = direct_selection_path_at(temp.path(), &graph);

        publish_direct_selection_at(&receipt, &graph, BLANK_SLATE_REBUILD_REASON).unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());
        assert!(direct_selection_requests_blank_slate_rebuild_at(
            &receipt, &graph
        ));

        publish_direct_selection_at(&receipt, &graph, "emergency_return").unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());
        assert!(!direct_selection_requests_blank_slate_rebuild_at(
            &receipt, &graph
        ));

        std::fs::write(&receipt, b"{").unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());
        assert!(!direct_selection_requests_blank_slate_rebuild_at(
            &receipt, &graph
        ));
    }

    #[test]
    fn malformed_emergency_receipt_fails_toward_direct_files() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        let receipt = direct_selection_path_at(temp.path(), &graph);
        std::fs::create_dir_all(receipt.parent().unwrap()).unwrap();
        std::fs::write(&receipt, b"{").unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());
    }

    #[test]
    fn managed_activation_durably_retires_the_direct_selector_name() {
        let temp = tempfile::tempdir().unwrap();
        let graph = temp.path().join("graph");
        let receipt = direct_selection_path_at(temp.path(), &graph);
        publish_direct_selection_at(&receipt, &graph, "emergency_return").unwrap();
        assert!(direct_selection_is_active_at(&receipt, &graph).unwrap());

        retire_direct_selection_at(&receipt).unwrap();
        assert!(!direct_selection_is_active_at(&receipt, &graph).unwrap());
        assert!(!receipt.exists());
        assert!(std::fs::read_dir(receipt.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("retired-")));

        retire_direct_selection_at(&receipt).unwrap();
    }

    #[test]
    fn storage_mode_selectors_use_typed_durable_private_authority_publication() {
        let source = include_str!("sync_runtime.rs");
        let direct_start = source
            .find("fn publish_direct_selection_at(")
            .expect("Direct Files selector publisher");
        let direct_end = source[direct_start..]
            .find("\nfn binding_path(")
            .map(|offset| direct_start + offset)
            .expect("end of Direct Files selector helpers");
        let direct = &source[direct_start..direct_end];
        assert!(direct.contains("durable_private_authority_update"));
        assert!(direct.contains("durable_private_authority_retire"));
        assert!(!direct.contains("std::fs::remove_file"));

        let binding_start = source
            .find("fn persist_binding_at(")
            .expect("Managed Storage binding publisher");
        let binding_end = source[binding_start..]
            .find("\n#[derive(")
            .map(|offset| binding_start + offset)
            .expect("end of Managed Storage binding publisher");
        let binding = &source[binding_start..binding_end];
        assert!(binding.contains("durable_private_authority_update"));
        assert!(!binding.contains("target_os = \"android\""));
        assert!(!binding.contains("OpenOptions::new"));
    }

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
    fn fresh_activation_proves_candidate_owned_readiness_before_slot_publication() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("fn prepare_sparse_v2_activation(")
            .expect("fresh activation starts from the Direct Files graph");
        let body = &source[start
            ..source[start..]
                .find("fn publish_managed_candidate(")
                .map(|offset| start + offset)
                .expect("fresh activation completion")];
        assert!(
            !body.contains("graph.list_pages()") && !body.contains("expected_pages"),
            "managed readiness must not compare against a retired Direct Files cache"
        );
        let readiness = body
            .find("prove_managed_application_ready")
            .expect("managed application readiness proof");
        assert!(readiness > 0);
        assert!(!body.contains("begin_legacy_retirement"));
        assert!(!body.contains("wait_for_legacy_drain"));
        assert!(!body.contains("graphs.write().unwrap().remove"));
        assert!(!body.contains("persist_binding_record"));
    }

    #[test]
    fn fresh_activation_failures_leave_direct_files_serving_until_one_publication() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("fn prepare_sparse_v2_activation(")
            .expect("activation preparation exists");
        let body = &source[start
            ..source[start..]
                .find("fn publish_managed_candidate(")
                .map(|offset| start + offset)
                .expect("activation publication boundary")];

        assert!(
            body.contains("activate_record_with_diagnostics("),
            "candidate build remains fallible before publication"
        );
        assert!(
            body.contains("prove_managed_application_ready("),
            "candidate readiness remains fallible before publication"
        );
        assert!(!body.contains("persist_binding_record"));
        assert!(!body.contains("graphs.write()"));
        let publish = &source[source
            .find("fn publish_managed_candidate(")
            .expect("single publication function")..];
        let persist = publish.find("persist_binding_record").unwrap();
        let replace = publish.find("replace_if_current").unwrap();
        let status = publish.find("sparse_v2_status_for_slot").unwrap();
        assert!(persist < replace && replace < status);
    }

    #[test]
    fn every_storage_mode_change_is_owned_by_the_native_supervisor() {
        let source = include_str!("sync_runtime.rs");
        for (function, kind) in [
            (
                "fn activate_sparse_v2_blocking(",
                "StorageTransitionKind::ActivateManaged",
            ),
            (
                "fn join_sparse_v2_shared_blocking(",
                "StorageTransitionKind::JoinManaged",
            ),
            (
                "fn cancel_sparse_v2_blocking(",
                "StorageTransitionKind::ReturnGracefully",
            ),
        ] {
            let start = source.find(function).expect("transition function exists");
            let body = &source[start
                ..source[start..]
                    .find("\n}\n")
                    .map(|offset| start + offset + 3)
                    .unwrap_or(source.len())];
            let guard = if kind == "StorageTransitionKind::ReturnGracefully" {
                "begin_recovery_guard("
            } else {
                "begin_guard("
            };
            assert!(body.contains(guard), "{function} bypasses supervisor");
            assert!(
                body.contains(kind),
                "{function} uses the wrong transition kind"
            );
        }
        let emergency = &source[source
            .find("fn cancel_sparse_v2_cold_blocking(")
            .expect("emergency return exists")..];
        assert!(emergency.contains("begin_emergency_return("));
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
            let compact: String = command.split_whitespace().collect();
            assert!(
                compact.contains(
                    "owned_graph_context(state).map_err(crate::command_error::CommandError::from)?"
                ),
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
    fn managed_open_stderr_receipts_use_bounded_codes_not_storage_details() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("fn open_record_with_progress(")
            .expect("managed open diagnostic boundary");
        let open = &source[start
            ..source[start..]
                .find("    /// The startup graph-open path")
                .map(|end| start + end)
                .expect("managed open diagnostic end")];
        assert!(open.contains("managed_open_phase_name(*phase)"));
        assert!(open.contains("managed_open_waiting_phase_name(*phase)"));
        assert!(open.contains("stage.diagnostic_name()"));
        assert!(open.contains("managed_open_outcome_code(&opened.status)"));
        assert!(
            !open.contains("diagnostics.projection_reason")
                && !open.contains("completed with {:?}"),
            "unconditional diagnostics must not format arbitrary storage details"
        );
        assert_eq!(
            managed_open_outcome_code(&SyncRuntimeOpenStatus::Blocked {
                reason_code: "/private/path/injected-detail".into(),
            }),
            "blocked"
        );
        assert_eq!(
            managed_open_outcome_code(&SyncRuntimeOpenStatus::OpenRefused {
                detail: "/private/path/injected-detail".into(),
            }),
            "open_refused"
        );
    }

    #[test]
    fn durable_managed_refusals_export_their_contract_scenario() {
        let refused = SparseV2Availability::from_open(SyncRuntimeOpenStatus::CorruptOrUnreadable {
            component: SyncRuntimeComponent::Enrollment,
            scenario: ManagedStorageRefusalScenario::DiskCorrupt,
        });
        let json = serde_json::to_value(refused).unwrap();
        assert_eq!(
            json.get("scenario_id").and_then(serde_json::Value::as_str),
            Some("MS-REF-DISK-CORRUPT"),
            "a durable refusal must identify the in-scope failure it defends against"
        );
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

    /// The native boundary carries only the fixed kind; no path or prose is
    /// load-bearing.
    #[test]
    fn a_join_with_no_sync_data_emits_only_the_fixed_kind() {
        let message = shared_enrollment_not_here_yet(Path::new("/graphs/notes"));
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            r#"{"kind":"sync-data-unavailable"}"#
        );
        assert!(!message.to_string().contains("/graphs/notes"));
        assert_eq!(
            shared_enrollment_descriptor_path(Path::new("/graphs/notes")),
            PathBuf::from(
                "/graphs/notes/.tine-sync/v2/shared/outbox/enrollment/shared-enrollment-v1.json"
            )
        );
    }

    /// The fixed native kind and frontend subclass are pinned together. The
    /// frontend can reword its message without changing control flow.
    #[test]
    fn the_not_yet_refusal_reaches_the_panel_as_a_typed_kind() {
        let message = shared_enrollment_not_here_yet(Path::new("/graphs/notes"));
        let panel = include_str!("../../src/components/Settings.tsx");

        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            r#"{"kind":"sync-data-unavailable"}"#
        );
        assert!(panel.contains("error instanceof SyncDataUnavailableError"));
        assert!(!panel.contains("detail.includes(\"does not yet contain sync data\")"));

        // The relative path the panel names must be the one this message means.
        let relative = ".tine-sync/v2/shared/outbox/enrollment/shared-enrollment-v1.json";
        assert!(
            shared_enrollment_descriptor_path(Path::new("/graphs/notes"))
                .to_string_lossy()
                .ends_with(relative),
            "the relative path drifted from the descriptor path"
        );
        assert!(
            panel.contains(&format!("\n  \"{relative}\";")),
            "the panel must name the same relative path"
        );
    }

    /// One source of truth for what a provider tree contains: the check that
    /// recognizes an untouched skeleton must expect exactly what the shared
    /// provider transport writes when it opens a tree, or a graph that has only
    /// ever opened one reads as shared.
    #[test]
    fn the_scaffold_check_expects_what_the_shared_transport_actually_writes() {
        assert_eq!(
            PROVIDER_SCAFFOLD_NAMESPACES.len(),
            SHARED_PROVIDER_TREE_NAMESPACES.len()
        );
        assert!(PROVIDER_SCAFFOLD_NAMESPACES.contains(&"clean-baselines-v1"));

        let root = std::env::temp_dir().join(format!("tine-scaffold-{}", Uuid::new_v4()));
        let shared = root.join("shared");
        for tree in PROVIDER_SCAFFOLD_TREES {
            for namespace in PROVIDER_SCAFFOLD_NAMESPACES {
                std::fs::create_dir_all(shared.join(tree).join(namespace)).unwrap();
            }
        }
        assert!(is_empty_local_provider_scaffold(&shared).unwrap());
        std::fs::write(
            shared.join("outbox/enrollment/shared-enrollment-v1.json"),
            b"x",
        )
        .unwrap();
        assert!(!is_empty_local_provider_scaffold(&shared).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn retryable_activation_keeps_the_last_exact_progress_in_its_public_detail() {
        let mut binding = SparseV2Binding {
            availability: SparseV2Availability::Retryable {
                stage: "shadow_import".into(),
                detail: "Permission denied (os error 13)".into(),
            },
            handle: None,
        };
        assert_eq!(
            attach_latest_progress_to_activation_result(
                &mut binding,
                "source capture",
            )
            .as_deref(),
            Some(
                "Tine-managed storage setup failed during source capture: Permission denied (os error 13)"
            )
        );
        assert_eq!(
            binding.availability,
            SparseV2Availability::Retryable {
                stage: "shadow_import".into(),
                detail: "Tine-managed storage setup failed during source capture: Permission denied (os error 13)".into(),
            }
        );

        let mut active = SparseV2Binding {
            availability: SparseV2Availability::Active,
            handle: None,
        };
        assert_eq!(
            attach_latest_progress_to_activation_result(&mut active, "source capture"),
            None
        );
        assert_eq!(active.availability, SparseV2Availability::Active);
    }

    #[test]
    fn retryable_join_bootstrap_preserves_stage_and_detail_without_debug_wrappers() {
        let detail = join_bootstrap_unavailable_detail(&SparseV2Availability::Retryable {
            stage: "shadow_import".into(),
            detail: "source proof stopped at pages/研究.md".into(),
        });
        assert_eq!(
            detail,
            "join bootstrap did not reach LocalActive during shadow_import: source proof stopped at pages/研究.md"
        );
        assert!(!detail.contains('{') && !detail.contains('}'));
    }

    #[test]
    fn direct_slot_serializes_direct_application_page_admission() {
        let root = std::env::temp_dir().join(format!("tine-admission-direct-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let slot = crate::state::GraphSlot::new(Graph::open(&root), root.clone());

        let status = sparse_v2_status_for_slot(&slot).unwrap();
        let wire = serde_json::to_value(status).unwrap();

        assert_eq!(wire["binding_generation"], slot.binding_generation);
        assert_eq!(
            wire["application_page_admission"],
            serde_json::json!({
                "binding_generation": slot.binding_generation,
                "authority": "direct",
            })
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_lifecycle_and_actor_capability_jointly_admit_application_pages() {
        assert!(runtime_observation_admits_application_pages(
            &SyncRuntimeLifecycle::Active,
            true,
        ));
        assert!(!runtime_observation_admits_application_pages(
            &SyncRuntimeLifecycle::Active,
            false,
        ));
        for lifecycle in [
            SyncRuntimeLifecycle::StoppedSafe,
            SyncRuntimeLifecycle::StoppedCrashed,
            SyncRuntimeLifecycle::Terminal,
        ] {
            assert!(!runtime_observation_admits_application_pages(
                &lifecycle, true,
            ));
        }
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
                storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(
                ),
                watch_ctl: Mutex::new(None),
                last_focused: Mutex::new(None),
                capture_graph: Mutex::new(Default::default()),
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

    #[test]
    fn stopped_managed_recovery_rebinds_an_empty_registry() {
        for detail in [
            "injected candidate-build failure",
            "injected readiness failure",
        ] {
            let fixture = RollbackFixture::new(Some("local_active"));
            fixture.state.graphs.write().unwrap().remove("main");
            let graph_meta = Graph::open(&fixture.graph_root).meta();

            let replacement = publish_stopped_managed_recovery_slot(
                &fixture.state,
                "main",
                fixture.graph_root.clone(),
                graph_meta,
                detail.into(),
                None,
            )
            .unwrap();

            let bound = fixture.state.graphs.read().unwrap().slot("main").unwrap();
            assert!(Arc::ptr_eq(&bound, &replacement));
            assert!(bound.is_sparse_v2());
            assert!(bound.sparse_runtime().is_none());
            let status = sparse_v2_status_for_slot(&bound).unwrap();
            assert!(status.can_retry);
            assert!(status.can_cancel);
            assert!(matches!(
                status.availability,
                SparseV2Availability::Retryable { ref detail, .. }
                    if detail.contains("injected")
            ));
        }
    }

    #[test]
    fn managed_candidate_publication_is_exactly_once_and_source_fenced() {
        let root = std::env::temp_dir().join(format!("tine-candidate-publish-{}", Uuid::new_v4()));
        let graph_root = root.join("graph");
        std::fs::create_dir_all(graph_root.join("pages")).unwrap();
        let graph = Graph::open(&graph_root);
        let meta = graph.meta();
        let source_generation = graph.guarded_graph_text_identity_report().generation;
        let predecessor = Arc::new(crate::state::GraphSlot::new(graph, graph_root.clone()));
        let state = crate::state::AppState {
            graphs: std::sync::RwLock::new(crate::state::GraphRegistry::default()),
            storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(None),
            capture_graph: Mutex::new(Default::default()),
            sync_runtime: SyncRuntimeFacade,
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        };
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), Arc::clone(&predecessor))
            .unwrap();
        let record = SparseV2ActivationRecord::new(&graph_root, meta.clone(), DeviceId::new());
        let candidate = |expected_generation| PreparedManagedCandidate {
            predecessor: Arc::clone(&predecessor),
            replacement: Arc::new(crate::state::GraphSlot::from_sparse_v2(
                SparseV2Binding::without_actor_for_test(),
                graph_root.clone(),
                meta.clone(),
            )),
            record: record.clone(),
            readiness: ManagedReadinessReceipt {
                page_count: 0,
                sampled_paths: Vec::new(),
                inventory_ms: 0,
                sample_load_ms: 0,
                total_ms: 0,
            },
            direct_source_generation: expected_generation,
        };

        let persisted = std::sync::atomic::AtomicUsize::new(0);
        let error = publish_managed_candidate_with(
            &state,
            "main",
            candidate(Some(source_generation.saturating_add(1))),
            || {
                persisted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(error.contains("changed while managed storage was being prepared"));
        assert_eq!(persisted.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(
            state
                .graphs
                .read()
                .unwrap()
                .slot("main")
                .unwrap()
                .binding_generation,
            predecessor.binding_generation
        );

        let restored = std::sync::atomic::AtomicUsize::new(0);
        let error = publish_managed_candidate_with(
            &state,
            "main",
            candidate(Some(source_generation)),
            || {
                Err(crate::command_error::CommandError::prose(
                    "ambiguous managed selector publication",
                ))
            },
            |_| {
                restored.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.contains("Direct Files selection was restored"));
        assert_eq!(restored.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            state
                .graphs
                .read()
                .unwrap()
                .slot("main")
                .unwrap()
                .binding_generation,
            predecessor.binding_generation
        );

        let error = publish_managed_candidate_with(
            &state,
            "main",
            candidate(None),
            || {
                Err(crate::command_error::CommandError::prose(
                    "managed-to-managed publication failure",
                ))
            },
            |_| {
                restored.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.contains("managed runtime publication failed"));
        assert_eq!(
            restored.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a managed predecessor must not be rolled back to Direct Files"
        );

        publish_managed_candidate_with(
            &state,
            "main",
            candidate(Some(source_generation)),
            || {
                persisted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(persisted.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_ne!(
            state
                .graphs
                .read()
                .unwrap()
                .slot("main")
                .unwrap()
                .binding_generation,
            predecessor.binding_generation
        );
        let _ = std::fs::remove_dir_all(root);
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

    /// Seam 1's device is still SERVING: `make_active` left a live runtime that
    /// owns its SQLite database and sidecars, its receipts, its writer-lane
    /// locks and its managed-local journal, and writes them on its own
    /// schedule. Byte-stability of those files is not a property adoption can
    /// promise, so asserting it made this test fail for reasons that had
    /// nothing to do with the seam — twice, in two different ways: once with 13
    /// entries common, 0 missing and 16 ADDED by lazy initialization, and once
    /// with `projection/materialization.sqlite-shm` REWRITTEN in place at the
    /// same length. Both happened inside a loaded full-suite run; standalone the
    /// test shows zero churn in eight consecutive runs, which is why this looked
    /// like a flake rather than a bad oracle.
    ///
    /// So the byte-level comparison is scoped to what the test is named for —
    /// the device's own HISTORY, which adoption is the only writer of, and which
    /// `archive_private_root` would have moved had the drain succeeded. The
    /// runtime's volatile working state is excluded by construction, not by
    /// enumerating the churn observed so far.
    ///
    /// What this deliberately does not catch: adoption ADDING a file, or
    /// touching the runtime's own working state. The seam's outcomes are
    /// asserted directly instead — no recovery root, Markdown bytes untouched,
    /// and the slot still serving sparse-v2. Seams 2 and 3 use a
    /// `shadow_import` fixture with no live runtime, so they keep full tree
    /// equality and must not be loosened to match this one.
    fn durable_managed_history(tree: &BTreeMap<PathBuf, Vec<u8>>) -> BTreeMap<PathBuf, Vec<u8>> {
        tree.iter()
            .filter(|(path, _)| {
                path.starts_with("archive")
                    || path.starts_with("enrollment")
                    || path.as_os_str() == "binding.json"
            })
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect()
    }

    fn assert_nothing_lost_or_rewritten(
        before: &BTreeMap<PathBuf, Vec<u8>>,
        after: &BTreeMap<PathBuf, Vec<u8>>,
        what: &str,
    ) {
        for (path, bytes) in before {
            match after.get(path) {
                None => panic!("{what}: adoption removed {}", path.display()),
                Some(now) => assert_eq!(
                    now,
                    bytes,
                    "{what}: adoption rewrote {} ({} bytes before, {} after)",
                    path.display(),
                    bytes.len(),
                    now.len()
                ),
            }
        }
    }

    fn create_empty_provider_transport_scaffold(graph_root: &Path) {
        let shared = graph_root.join(".tine-sync/v2/shared");
        for tree in PROVIDER_SCAFFOLD_TREES {
            for namespace in PROVIDER_SCAFFOLD_NAMESPACES {
                std::fs::create_dir_all(shared.join(tree).join(namespace)).unwrap();
            }
        }
    }

    fn cold_fixture(stage: Option<&str>) -> RollbackFixture {
        let fixture = RollbackFixture::new(stage);
        let removed = fixture.state.graphs.write().unwrap().remove("main");
        assert!(
            removed.is_some(),
            "cold recovery starts before any graph slot"
        );
        fixture
    }

    #[test]
    fn direct_status_does_not_inspect_an_existing_shared_descriptor() {
        std::thread::Builder::new()
            .name("tine-direct-status-quarantine-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(direct_status_does_not_inspect_an_existing_shared_descriptor_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn direct_status_does_not_inspect_an_existing_shared_descriptor_inner() {
        let mut fixture = RollbackFixture::new(Some("shadow_import"));
        fixture.make_active();
        fixture
            .slot
            .sparse_runtime()
            .expect("active fixture must retain its runtime")
            .prepare_shared()
            .unwrap();

        // A second process discovering this literal shared descriptor has not
        // opted into sparse-v2 yet, so it keeps its Direct Files writer.
        let direct = crate::state::GraphSlot::new(
            Graph::open(&fixture.graph_root),
            fixture.graph_root.clone(),
        );
        let status = sparse_v2_status_for_slot(&direct).unwrap();
        assert!(matches!(
            status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert_eq!(
            serde_json::to_value(status).unwrap()["application_page_admission"],
            serde_json::json!({
                "binding_generation": direct.binding_generation,
                "authority": "direct",
            })
        );
    }

    #[test]
    fn enrollment_commands_reopen_the_retired_actor_before_reporting_success() {
        let source = include_str!("sync_runtime.rs");
        let share_start = source
            .find("fn prepare_sparse_v2_share_blocking(")
            .expect("share command implementation");
        let share_end = source[share_start..]
            .find("/// Reconstitute the sole application actor")
            .map(|offset| share_start + offset)
            .expect("share command boundary");
        let share = &source[share_start..share_end];
        let share_cut = share.find(".prepare_shared()").expect("share cut");
        let share_reopen = share
            .find("prepare_reopened_managed_candidate(")
            .expect("share runtime reopen");
        let share_publish = share
            .find("publish_managed_candidate(")
            .expect("share replacement publication");
        assert!(share_cut < share_reopen && share_reopen < share_publish);
        assert!(
            !share.contains("sparse_v2_status_for_slot(&slot)"),
            "the actor that commits a share cut is intentionally stopped"
        );

        let join_start = source
            .find("fn prepare_sparse_v2_join(")
            .expect("join command implementation");
        let join_end = source[join_start..]
            .find("#[tauri::command]\npub(crate) async fn sparse_v2_query")
            .map(|offset| join_start + offset)
            .expect("join command boundary");
        let join = &source[join_start..join_end];
        let compact_join: String = join.split_whitespace().collect();
        let retained_branch = join
            .find("if slot.sparse_binding().is_some()")
            .expect("already-managed join branch");
        let direct_branch = compact_join
            .find(
                "letgraph=slot.legacy_graph().map_err(crate::command_error::CommandError::from)?;",
            )
            .expect("Direct Files join branch");
        let direct = &compact_join[direct_branch..];
        let archive_predecessor = direct
            .find("prepare_shared_binding_record(")
            .expect("Direct Files join must prepare a fresh descriptor-bound private root");
        let bootstrap = direct
            .find("activate_record_with_diagnostics(")
            .expect("Direct Files join bootstrap");
        assert!(archive_predecessor < bootstrap);
        assert!(
            !direct.contains("SparseV2ActivationRecord::from_shared("),
            "the production join path must not construct shared identities without first archiving unselected private state"
        );
        let retained = &join[retained_branch..];
        let join_cut = retained.find(".join_shared(").expect("join cut");
        let join_reopen = retained
            .find("prepare_reopened_managed_candidate(")
            .expect("join runtime reopen");
        assert!(join_cut < join_reopen);
        assert!(
            !retained[..join_reopen].contains("PreparedActivationOutcome::AlreadyCurrent"),
            "a completed join must not publish the retired actor as current"
        );
    }

    #[test]
    fn cold_emergency_return_has_no_managed_runtime_or_archive_dependency() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("fn cancel_sparse_v2_cold_blocking(")
            .expect("cold recovery blocking boundary");
        let command = &source[start
            ..source[start..]
                .find("#[tauri::command]\npub(crate) async fn cancel_sparse_v2(")
                .map(|end| start + end)
                .expect("next managed command")];
        for required in [
            "begin_emergency_return",
            "publish_direct_selection",
            "prepare_direct_files_open",
            "commit_if_current",
            "publish_prepared_direct_files",
        ] {
            assert!(
                command.contains(required),
                "cold recovery must retain `{required}`"
            );
        }
        for forbidden in [
            "transition_lane",
            "archive_private_root",
            "cancel_sparse_v2_cold_at_paths_with_archive_and_publish",
            "shutdown_for_direct_files_escape",
            "sparse_private_root",
            "startup_recovery",
            "startup_recovery_target_is_remembered",
        ] {
            assert!(
                !command.contains(forbidden),
                "emergency return must not depend on `{forbidden}`"
            );
        }
    }

    #[test]
    fn blank_slate_private_root_renames_barrier_both_parents() {
        let source = include_str!("sync_runtime.rs");
        for signature in [
            "fn archive_private_root(",
            "fn replace_failed_blank_slate_candidate(",
        ] {
            let start = source
                .find(signature)
                .expect("rename helper remains present");
            let body = &source[start
                ..source[start..]
                    .find("\n}\n")
                    .map(|offset| start + offset + 3)
                    .expect("rename helper remains bounded")];
            let rename = body
                .find("std::fs::rename(")
                .expect("private-root publication remains a rename");
            let barrier = body
                .find("sync_provider_namespace_rename(")
                .expect("private-root rename must sync destination and source parents");
            assert!(
                rename < barrier,
                "the rename must precede its directory barriers"
            );
        }
    }

    fn adoption_identities(seed: u128) -> (WorkspaceId, LineageDigest, DocumentId) {
        (
            WorkspaceId::from_uuid(Uuid::from_u128(seed)),
            LineageDigest::of(format!("lineage-{seed}").as_bytes()),
            DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
        )
    }

    #[test]
    fn two_independent_activations_are_adoptable_and_every_other_relation_is_refused() {
        let mine = adoption_identities(0xad10);
        let theirs = adoption_identities(0xad20);

        assert_eq!(
            shared_graph_relation(mine, theirs),
            SharedGraphRelation::Independent
        );
        assert!(shared_graph_relation_refusal(SharedGraphRelation::Independent).is_none());

        assert_eq!(
            shared_graph_relation(mine, mine),
            SharedGraphRelation::SameGraph
        );
        let same = shared_graph_relation_refusal(SharedGraphRelation::SameGraph).unwrap();
        assert!(same.contains("Nothing was changed"));
        assert!(same.contains("Join a synced graph from another device"));

        // Each single agreeing identity is a partial lineage, and none of them
        // is allowed to look like an independent activation.
        for partial in [
            (theirs.0, mine.1, mine.2),
            (mine.0, theirs.1, mine.2),
            (mine.0, mine.1, theirs.2),
            (theirs.0, theirs.1, mine.2),
            (theirs.0, mine.1, theirs.2),
            (mine.0, theirs.1, theirs.2),
        ] {
            assert_eq!(
                shared_graph_relation(mine, partial),
                SharedGraphRelation::PartialLineage,
                "{partial:?} must not be treated as adoptable"
            );
        }
        let partial = shared_graph_relation_refusal(SharedGraphRelation::PartialLineage).unwrap();
        assert!(partial.contains("matches part of the shared graph's identity"));
        assert!(partial.contains("Nothing was changed"));
        assert!(partial.contains("return this device to Direct files"));
    }

    #[test]
    fn every_adoption_refusal_names_a_reason_and_a_remedy_that_survives_panel_redaction() {
        let mut refusals = vec![
            shared_graph_relation_refusal(SharedGraphRelation::SameGraph).unwrap(),
            shared_graph_relation_refusal(SharedGraphRelation::PartialLineage).unwrap(),
        ];
        for phase in ["share_prepared", "joining", "active"] {
            refusals.push(shared_cut_refusal(Some(phase)).expect("a shared cut refuses adoption"));
        }
        // A purely local managed device has no shared phase at all, and that is
        // precisely the device adoption exists for.
        assert!(shared_cut_refusal(None).is_none());

        for refusal in refusals {
            assert!(
                refusal.contains("Nothing was changed"),
                "refusal must say what it did not do: {refusal}"
            );
            // The panel keeps only the first line of a native message and drops
            // any line with no recognised diagnostic class, so a refusal that
            // fails this is a refusal the user never reads.
            assert!(
                !refusal.contains('\n'),
                "refusal must survive first-line truncation: {refusal}"
            );
            assert!(
                refusal.contains("storage") || refusal.contains("sync"),
                "refusal must carry a diagnostic class the panel keeps: {refusal}"
            );
        }
    }

    /// Seam 1 — the drain. Nothing has been archived, so the managed slot must
    /// come back and not one byte of the device's own managed history may be
    /// removed or rewritten. This device is still SERVING, so its live runtime
    /// keeps writing its own working state throughout; `durable_managed_history`
    /// says where the line is and why it is drawn there.
    #[test]
    fn adoption_seam_shutdown_failure_keeps_the_managed_device_serving_its_own_history() {
        let mut fixture = RollbackFixture::new(Some("local_active"));
        fixture.make_active();
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));

        let error = set_aside_managed_history_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            |_| {
                Err(crate::command_error::CommandError::prose(
                    "injected adoption drain failure",
                ))
            },
            archive_private_root,
            |_, _| panic!("adoption must not publish Direct files after a failed drain"),
        )
        .unwrap_err();

        assert!(error.contains("injected adoption drain failure"));
        assert_nothing_lost_or_rewritten(
            &durable_managed_history(&private_before),
            &durable_managed_history(&snapshot_tree(&fixture.private_root)),
            "private root managed history",
        );
        assert_nothing_lost_or_rewritten(
            &provider_before,
            &snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            "provider transport",
        );
        assert!(!fixture.recovery_root.exists());
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            fixture.markdown_bytes
        );
        let serving = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(serving.is_sparse_v2());
        assert!(serving.sparse_runtime().is_some());
    }

    /// Seam 2 — the archive itself. The rename is the only durable step, so a
    /// failure here must leave the device exactly where it was, retryable.
    #[test]
    fn adoption_seam_archive_failure_keeps_both_histories_where_they_were() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));

        let error = set_aside_managed_history_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
            |_, _| {
                Err(crate::command_error::CommandError::prose(
                    "injected adoption archive failure",
                ))
            },
            |_, _| panic!("adoption must not publish Direct files after a failed archive"),
        )
        .unwrap_err();

        assert!(error.contains("injected adoption archive failure"));
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert_eq!(
            snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            provider_before
        );
        assert!(!fixture.recovery_root.exists());
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            fixture.markdown_bytes
        );
        let retryable = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(retryable.is_sparse_v2());
        assert!(retryable.sparse_runtime().is_none());
    }

    /// Seam 3 — after the archive, before Direct Files opens. The device serves
    /// its own unchanged Markdown/Org tree either way, which is still the
    /// pre-adoption state; the archive and the shared evidence both survive, so
    /// both halves remain retryable.
    #[test]
    fn adoption_seam_direct_publication_failure_keeps_the_archive_and_the_shared_evidence() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));

        let error = set_aside_managed_history_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
            archive_private_root,
            |_, _| {
                Err(crate::command_error::CommandError::prose(
                    "injected adoption Direct files failure",
                ))
            },
        )
        .unwrap_err();

        assert!(error.contains("recovery state was preserved"));
        assert!(!fixture.private_root.exists());
        let archives = std::fs::read_dir(&fixture.recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert_eq!(snapshot_tree(&archives[0]), private_before);
        assert_eq!(
            snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            provider_before,
            "the shared evidence must survive so the join half can still run"
        );
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            fixture.markdown_bytes
        );
        assert!(fixture.state.graphs.read().unwrap().slot("main").is_none());
        assert!(Graph::open_checked(&fixture.graph_root).is_ok());
    }

    /// The first half, completed. This is the one behaviour a literal "Return
    /// to Direct files, then Join" cannot produce: the predecessor archived AND
    /// the other device's shared namespace still in place.
    #[test]
    fn adoption_set_aside_archives_the_predecessor_and_leaves_the_shared_namespace_in_place() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let descriptor = fixture
            .graph_root
            .join(".tine-sync/v2/shared/outbox/enrollment/shared-enrollment-v1.json");
        std::fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        std::fs::write(&descriptor, b"the other device's descriptor").unwrap();
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));

        let result = set_aside_managed_history_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
            archive_private_root,
            |direct_root, approved_assets| {
                let graph =
                    tine_core::model::Graph::open_checked_with_assets(direct_root, approved_assets)
                        .map_err(crate::command_error::CommandError::from)?;
                let replacement = Arc::new(crate::state::GraphSlot::new(
                    graph,
                    direct_root.to_path_buf(),
                ));
                let binding_generation = replacement.binding_generation;
                fixture
                    .state
                    .graphs
                    .write()
                    .unwrap()
                    .bind("main".into(), replacement)
                    .map_err(crate::command_error::CommandError::from)?;
                Ok(binding_generation)
            },
        )
        .unwrap();

        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!fixture.private_root.exists());
        let archives = std::fs::read_dir(&fixture.recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert_eq!(snapshot_tree(&archives[0]), private_before);
        assert_eq!(
            std::fs::read(archives[0].join(SPARSE_BINDING_FILE)).unwrap(),
            fixture.binding_bytes
        );
        // The graceful Direct Files return moves this subtree under
        // `.tine-sync/recovery`. Adoption must not.
        assert!(!fixture.graph_root.join(".tine-sync/recovery").exists());
        assert_eq!(
            snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            provider_before
        );
        assert_eq!(
            std::fs::read(&descriptor).unwrap(),
            b"the other device's descriptor"
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
    }

    #[test]
    fn adoption_is_composed_of_the_two_existing_transitions_and_never_archives_shared_evidence() {
        let source = include_str!("sync_runtime.rs");
        let start = source
            .find("fn adopt_sparse_v2_shared_blocking(")
            .expect("adoption composition exists");
        let body = &source[start
            ..source[start..]
                .find("\n}\n")
                .map(|offset| start + offset + 3)
                .unwrap_or(source.len())];
        // Every refusal is decided before the first durable step.
        let set_aside = body
            .find("set_aside_managed_history_for_adoption(")
            .expect("adoption archives before it joins");
        for refusal in [
            "refuse_unclaimed_sparse_archive(",
            "shared_enrollment_not_here_yet(",
            "shared_graph_relation_refusal(",
            "shared_cut_refusal(",
        ] {
            let at = body.find(refusal).unwrap_or_else(|| {
                panic!("adoption must decide `{refusal}` for itself");
            });
            assert!(at < set_aside, "`{refusal}` must precede the archive");
        }
        assert!(
            body.find("join_sparse_v2_shared_blocking(").unwrap() > set_aside,
            "adoption joins only after the predecessor is archived"
        );

        let start = source
            .find("fn set_aside_managed_history_for_adoption(")
            .expect("adoption's first half exists");
        let first_half = &source[start
            ..source[start..]
                .find("\n}\n")
                .map(|offset| start + offset + 3)
                .unwrap_or(source.len())];
        assert!(first_half.contains("preserve_graph_provider_namespace"));
        assert!(
            !first_half.contains("archive_graph_provider_namespace"),
            "adoption must never archive the other device's shared evidence"
        );
        assert!(first_half.contains("StorageTransitionKind::ReturnGracefully"));
        assert!(first_half.contains("begin_recovery_guard("));

        let contract = include_str!("../../docs/storage-sync-contract.md");
        assert!(contract
            .contains("### 2.3a Adoption: a device that already has a managed graph of its own"));
        assert!(contract.contains("it does **not** archive `<graph>/.tine-sync/v2`"));
        assert!(contract.contains("never a merge of two divergent histories"));
        assert!(contract.contains("sparse-v2-recovery"));
    }

    #[test]
    fn cold_return_without_slot_archives_local_and_shared_provider_evidence_preserving_bytes() {
        let local = cold_fixture(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&local.graph_root);
        let local_provider = snapshot_tree(&local.graph_root.join(".tine-sync/v2"));
        let local_result = cancel_sparse_v2_cold_at_paths(
            &local.state,
            "main",
            local.graph_root.clone(),
            &local.private_root,
            &local.recovery_root,
            None,
        )
        .unwrap();
        assert!(matches!(
            local_result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!local.private_root.exists());
        assert_eq!(
            snapshot_tree(
                &std::fs::read_dir(local.graph_root.join(".tine-sync/recovery"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path(),
            ),
            local_provider
        );
        assert_eq!(
            std::fs::read(&local.markdown_path).unwrap(),
            local.markdown_bytes
        );
        assert!(local
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());

        let shared = cold_fixture(Some("joining"));
        std::fs::create_dir_all(shared.graph_root.join(".tine-sync/v2/shared")).unwrap();
        std::fs::write(
            shared
                .graph_root
                .join(".tine-sync/v2/shared/provider-evidence"),
            b"shared provider bytes",
        )
        .unwrap();
        let shared_provider = snapshot_tree(&shared.graph_root.join(".tine-sync/v2"));
        cancel_sparse_v2_cold_at_paths(
            &shared.state,
            "main",
            shared.graph_root.clone(),
            &shared.private_root,
            &shared.recovery_root,
            None,
        )
        .unwrap();
        let archived_provider = std::fs::read_dir(shared.graph_root.join(".tine-sync/recovery"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(snapshot_tree(&archived_provider), shared_provider);
        assert!(!shared.private_root.exists());
        assert!(shared
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());
    }

    /// A Direct Files return must not take the OTHER device's enrollment with
    /// it. The graph folder is synced, so archiving a live provider tree here
    /// removes the descriptor from the device that is still sharing — Martin's
    /// graph reached two `recovery/v2-*` archives and no `v2` exactly that way,
    /// and the phone was then told the graph "does not yet contain sync data".
    ///
    /// The archive exists only so that an UNCLAIMED namespace cannot lock a
    /// later Direct Files open out. A joinable tree does not: the cold check
    /// admits it and the panel offers Join beside it.
    #[test]
    fn a_direct_files_return_preserves_a_joinable_peer_tree_and_archives_an_unclaimed_one() {
        let joinable =
            std::env::temp_dir().join(format!("tine-df-return-joinable-{}", Uuid::new_v4()));
        std::fs::create_dir_all(joinable.join(".tine-sync/v2/shared")).unwrap();
        std::fs::write(joinable.join(".tine-sync/v2/shared/evidence"), b"peer").unwrap();
        let before = snapshot_tree(&joinable.join(".tine-sync/v2"));
        let archived = archive_graph_provider_namespace_with(&joinable, |shared| {
            assert_eq!(shared, joinable.join(".tine-sync/v2/shared"));
            true
        })
        .unwrap();
        assert!(matches!(archived, ProviderNamespaceArchive::Absent));
        assert_eq!(snapshot_tree(&joinable.join(".tine-sync/v2")), before);
        assert!(!joinable.join(".tine-sync/recovery").exists());

        let unclaimed =
            std::env::temp_dir().join(format!("tine-df-return-unclaimed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(unclaimed.join(".tine-sync/v2/shared")).unwrap();
        std::fs::write(unclaimed.join(".tine-sync/v2/shared/evidence"), b"mine").unwrap();
        let before = snapshot_tree(&unclaimed.join(".tine-sync/v2"));
        let archived = archive_graph_provider_namespace_with(&unclaimed, |_| false).unwrap();
        let ProviderNamespaceArchive::Moved { destination, .. } = archived else {
            panic!("an unclaimed namespace must still be archived");
        };
        assert!(!unclaimed.join(".tine-sync/v2").exists());
        assert_eq!(snapshot_tree(&destination), before);
    }

    #[test]
    fn provider_set_aside_and_rollback_barrier_every_changed_directory_entry() {
        use tine_core::durability_counters::{Barrier, BarrierSession};

        let root = std::env::temp_dir().join(format!("tine-provider-set-aside-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".tine-sync/v2/shared")).unwrap();
        std::fs::write(root.join(".tine-sync/v2/shared/evidence"), b"retained").unwrap();

        let barriers = BarrierSession::begin();
        let archive = archive_graph_provider_namespace_with(&root, |_| false).unwrap();
        assert_eq!(
            barriers.counts().get(Barrier::Directory),
            3,
            "creating recovery and moving v2 across two parents requires three directory barriers"
        );

        barriers.reset();
        restore_graph_provider_namespace(archive).unwrap();
        assert_eq!(
            barriers.counts().get(Barrier::Directory),
            2,
            "rollback changes one name in each of the source and destination parents"
        );
        assert_eq!(
            std::fs::read(root.join(".tine-sync/v2/shared/evidence")).unwrap(),
            b"retained"
        );
        BarrierSession::detach_current_thread();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cold_return_archive_failure_keeps_private_provider_and_markdown_bytes_retryable() {
        let fixture = cold_fixture(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));
        let markdown_before = std::fs::read(&fixture.markdown_path).unwrap();
        let error = cancel_sparse_v2_cold_at_paths_with_archive(
            &fixture.state,
            "main",
            fixture.graph_root.clone(),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            |_, _| {
                Err(crate::command_error::CommandError::prose(
                    "injected cold archive failure",
                ))
            },
        )
        .unwrap_err();
        assert!(error.contains("injected cold archive failure"));
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert_eq!(
            snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            provider_before
        );
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            markdown_before
        );
        assert!(!fixture.recovery_root.exists());
        let retryable = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(retryable.is_sparse_v2());
        assert!(retryable.sparse_runtime().is_none());
    }

    #[test]
    fn cold_return_archives_and_opens_direct_with_missing_or_corrupt_binding() {
        let missing = cold_fixture(Some("shadow_import"));
        std::fs::remove_file(missing.private_root.join(SPARSE_BINDING_FILE)).unwrap();
        let missing_enrollment = missing
            .graph_root
            .join(".tine-sync/v2/shared/outbox/enrollment");
        std::fs::create_dir_all(&missing_enrollment).unwrap();
        std::fs::write(
            missing_enrollment.join("shared-enrollment-v1.json"),
            b"partially delivered descriptor",
        )
        .unwrap();
        let missing_before = snapshot_tree(&missing.private_root);
        let missing_provider = snapshot_tree(&missing.graph_root.join(".tine-sync/v2"));
        let missing_result = cancel_sparse_v2_cold_at_paths(
            &missing.state,
            "main",
            missing.graph_root.clone(),
            &missing.private_root,
            &missing.recovery_root,
            None,
        )
        .unwrap();
        assert!(matches!(
            missing_result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!missing.private_root.exists());
        assert_eq!(
            snapshot_tree(
                &std::fs::read_dir(&missing.recovery_root)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path(),
            ),
            missing_before
        );
        assert!(!missing.graph_root.join(".tine-sync/v2").exists());
        assert_eq!(
            snapshot_tree(
                &std::fs::read_dir(missing.graph_root.join(".tine-sync/recovery"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path(),
            ),
            missing_provider
        );
        assert!(missing
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());

        let corrupt = cold_fixture(Some("shadow_import"));
        std::fs::write(corrupt.private_root.join(SPARSE_BINDING_FILE), b"{").unwrap();
        let corrupt_before = snapshot_tree(&corrupt.private_root);
        let corrupt_result = cancel_sparse_v2_cold_at_paths(
            &corrupt.state,
            "main",
            corrupt.graph_root.clone(),
            &corrupt.private_root,
            &corrupt.recovery_root,
            None,
        )
        .unwrap();
        assert!(matches!(
            corrupt_result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!corrupt.private_root.exists());
        assert_eq!(
            snapshot_tree(
                &std::fs::read_dir(&corrupt.recovery_root)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path(),
            ),
            corrupt_before
        );
        assert!(corrupt
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
    fn cold_return_with_exact_live_managed_slot_uses_the_live_shutdown_and_archive_path() {
        std::thread::Builder::new()
            .name("tine-sparse-cold-live-return-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(cold_return_with_exact_live_managed_slot_uses_the_live_shutdown_and_archive_path_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn cold_return_with_exact_live_managed_slot_uses_the_live_shutdown_and_archive_path_inner() {
        let mut fixture = RollbackFixture::new(Some("shadow_import"));
        fixture.make_active();
        let markdown_before = std::fs::read(&fixture.markdown_path).unwrap();
        let result = cancel_sparse_v2_cold_at_paths(
            &fixture.state,
            "main",
            fixture.graph_root.clone(),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
        )
        .unwrap();
        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!fixture.private_root.exists());
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            markdown_before
        );
        assert!(fixture.recovery_root.exists());
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
    fn cold_return_refuses_an_exact_direct_slot_without_archiving() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        let direct = Arc::new(crate::state::GraphSlot::new(
            Graph::open(&fixture.graph_root),
            fixture.graph_root.clone(),
        ));
        fixture
            .state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), direct)
            .unwrap();
        let private_before = snapshot_tree(&fixture.private_root);
        let error = cancel_sparse_v2_cold_at_paths(
            &fixture.state,
            "main",
            fixture.graph_root.clone(),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
        )
        .unwrap_err();
        assert!(error.contains("already using Direct files"));
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert!(!fixture.recovery_root.exists());
    }

    #[test]
    fn cold_return_refuses_a_slot_for_a_different_root_without_archiving() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        let other_root = fixture.root.join("other-graph");
        std::fs::create_dir_all(&other_root).unwrap();
        let private_before = snapshot_tree(&fixture.private_root);
        let error = cancel_sparse_v2_cold_at_paths(
            &fixture.state,
            "main",
            other_root,
            &fixture.private_root,
            &fixture.recovery_root,
            None,
        )
        .unwrap_err();
        assert!(error.contains("opened a different graph"));
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert!(!fixture.recovery_root.exists());
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
        let status = sparse_v2_status_for_slot(&fixture.slot).unwrap();
        assert_eq!(
            serde_json::to_value(status).unwrap()["application_page_admission"]["authority"],
            "managed_unavailable"
        );
    }

    #[test]
    fn retryable_open_binding_preserves_its_real_failure_before_page_readiness() {
        let binding = SparseV2Binding::from_open(SyncRuntimeOpenResult {
            status: SyncRuntimeOpenStatus::OpenRefused {
                detail: "clean managed runtime open failed: SQLite checkpoint is stale".into(),
            },
            handle: None,
        });
        assert_eq!(
            binding.serving_failure_detail().as_deref(),
            Some("clean managed runtime open failed: SQLite checkpoint is stale")
        );
    }

    #[test]
    fn transition_status_keeps_archive_and_direct_escape_available_with_warnings() {
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
            assert!(shared_status.can_cancel, "{stage}");
            assert!(
                shared_status
                    .cancel_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("archive the complete private")),
                "{stage}: {:?}",
                shared_status.cancel_reason
            );
        }

        let provider = RollbackFixture::new(Some("shadow_import"));
        std::fs::create_dir_all(provider.graph_root.join(".tine-sync/v2")).unwrap();
        std::fs::write(
            provider.graph_root.join(".tine-sync/v2/provider-evidence"),
            b"unclassifiable provider state",
        )
        .unwrap();
        let provider_status = sparse_v2_status_for_slot(&provider.slot).unwrap();
        assert!(provider_status.can_cancel);
        assert!(provider_status
            .cancel_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("archive the complete private")));
    }

    #[test]
    fn incomplete_local_activation_retires_without_touching_markdown_and_preserves_private_bytes() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));
        assert_eq!(
            provider_namespace_evidence(&fixture.graph_root.join(".tine-sync/v2")).unwrap(),
            ProviderNamespaceEvidence::LocalOnly
        );
        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();

        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert_eq!(result.binding_generation, result.status.binding_generation);
        assert!(result
            .recovery_statement
            .contains("Complete managed-storage recovery state was preserved"));
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
        assert!(!fixture.graph_root.join(".tine-sync/v2").exists());
        let provider_archives = std::fs::read_dir(fixture.graph_root.join(".tine-sync/recovery"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(provider_archives.len(), 1);
        assert_eq!(snapshot_tree(&provider_archives[0]), provider_before);
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
            shutdown_for_direct_files_escape,
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

    fn assert_return_to_direct_files_after_unreadable_private_state(fixture: &RollbackFixture) {
        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
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
    fn missing_private_binding_does_not_block_explicit_return_to_direct_files() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        std::fs::remove_file(fixture.private_root.join(SPARSE_BINDING_FILE)).unwrap();

        assert_return_to_direct_files_after_unreadable_private_state(&fixture);
        let archived = std::fs::read_dir(&fixture.recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archived.len(), 1);
        assert!(archived[0].join("diagnostic-bytes").is_file());
        assert!(!archived[0].join(SPARSE_BINDING_FILE).exists());
    }

    #[test]
    fn corrupt_private_binding_does_not_block_explicit_return_to_direct_files() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        std::fs::write(fixture.private_root.join(SPARSE_BINDING_FILE), b"{").unwrap();

        assert_return_to_direct_files_after_unreadable_private_state(&fixture);
        let archived = std::fs::read_dir(&fixture.recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archived.len(), 1);
        assert_eq!(
            std::fs::read(archived[0].join(SPARSE_BINDING_FILE)).unwrap(),
            b"{"
        );
    }

    #[test]
    fn absent_private_root_does_not_block_explicit_return_to_direct_files() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        std::fs::remove_dir_all(&fixture.private_root).unwrap();

        assert_return_to_direct_files_after_unreadable_private_state(&fixture);
        assert!(!fixture.recovery_root.exists());
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
            shutdown_for_direct_files_escape,
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
    fn terminal_runtime_uses_real_confirmed_override_and_reaches_direct_files() {
        std::thread::Builder::new()
            .name("tine-sparse-forced-direct-return-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(terminal_runtime_uses_real_confirmed_override_and_reaches_direct_files_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn terminal_runtime_uses_real_confirmed_override_and_reaches_direct_files_inner() {
        let mut fixture = RollbackFixture::new(Some("shadow_import"));
        fixture.make_active();
        let handle = fixture.slot.sparse_runtime().unwrap();
        handle.stop_without_clean_drain().unwrap();
        assert_eq!(
            handle.status().unwrap().lifecycle,
            SyncRuntimeLifecycle::StoppedCrashed
        );

        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(result
            .recovery_statement
            .contains("had not reached durable storage may be absent"));
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
            |_| {
                Err(crate::command_error::CommandError::prose(
                    "injected force-stop refusal",
                ))
            },
        )
        .unwrap_err();

        assert!(error.contains("injected force-stop refusal"));
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
    fn archive_failure_publishes_a_fresh_retryable_slot_after_shutdown() {
        let fixture = RollbackFixture::new(Some("shadow_import"));
        create_empty_provider_transport_scaffold(&fixture.graph_root);
        let private_before = snapshot_tree(&fixture.private_root);
        let provider_before = snapshot_tree(&fixture.graph_root.join(".tine-sync/v2"));
        let markdown_before = std::fs::read(&fixture.markdown_path).unwrap();
        let error = cancel_sparse_v2_at_paths_with_archive(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            |_| {
                assert!(fixture.state.graphs.read().unwrap().slot("main").is_none());
                Ok(DirectFilesShutdown::Clean)
            },
            |private_root, recovery_root| {
                assert_eq!(private_root, fixture.private_root);
                assert_eq!(recovery_root, fixture.recovery_root);
                let retryable = fixture.state.graphs.read().unwrap().slot("main").unwrap();
                assert!(retryable.is_sparse_v2());
                assert!(retryable.sparse_runtime().is_none());
                Err(crate::command_error::CommandError::prose(
                    "injected archive rename failure",
                ))
            },
        )
        .unwrap_err();

        assert!(error.contains("injected archive rename failure"));
        let restored = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(!Arc::ptr_eq(&restored, &fixture.slot));
        assert!(restored.is_sparse_v2());
        assert!(restored.sparse_runtime().is_none());
        let retry_status = sparse_v2_status_for_slot(&restored).unwrap();
        assert!(retry_status.can_retry);
        assert!(retry_status.can_cancel);
        assert_eq!(snapshot_tree(&fixture.private_root), private_before);
        assert_eq!(
            snapshot_tree(&fixture.graph_root.join(".tine-sync/v2")),
            provider_before
        );
        assert_eq!(
            std::fs::read(&fixture.markdown_path).unwrap(),
            markdown_before
        );
        assert!(!fixture.recovery_root.exists());

        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&restored),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
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
    fn real_clean_shutdown_archive_failure_never_republishes_a_dead_handle() {
        std::thread::Builder::new()
            .name("tine-sparse-archive-failure-retry-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(real_clean_shutdown_archive_failure_never_republishes_a_dead_handle_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn real_clean_shutdown_archive_failure_never_republishes_a_dead_handle_inner() {
        let mut fixture = RollbackFixture::new(Some("shadow_import"));
        fixture.make_active();
        let error = cancel_sparse_v2_at_paths_with_archive(
            &fixture.state,
            "main",
            Arc::clone(&fixture.slot),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
            |_, _| {
                Err(crate::command_error::CommandError::prose(
                    "injected private archive failure",
                ))
            },
        )
        .unwrap_err();
        assert!(error.contains("injected private archive failure"));

        let retryable = fixture.state.graphs.read().unwrap().slot("main").unwrap();
        assert!(!Arc::ptr_eq(&retryable, &fixture.slot));
        assert!(retryable.sparse_runtime().is_none());
        let retry_status = sparse_v2_status_for_slot(&retryable).unwrap();
        assert!(retry_status.can_retry);
        assert!(retry_status.can_cancel);
        assert!(matches!(
            retry_status.availability,
            SparseV2Availability::Retryable { ref stage, .. } if stage == "local_active"
        ));

        let result = cancel_sparse_v2_at_paths(
            &fixture.state,
            "main",
            Arc::clone(&retryable),
            &fixture.private_root,
            &fixture.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
    }

    #[test]
    fn shared_or_provider_evidence_is_archived_before_returning_to_direct_files() {
        let provider = RollbackFixture::new(Some("shadow_import"));
        std::fs::create_dir_all(provider.graph_root.join(".tine-sync/v2")).unwrap();
        std::fs::write(
            provider.graph_root.join(".tine-sync/v2/provider-evidence"),
            b"shared",
        )
        .unwrap();
        let provider_result = cancel_sparse_v2_at_paths(
            &provider.state,
            "main",
            Arc::clone(&provider.slot),
            &provider.private_root,
            &provider.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            provider_result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!provider.private_root.exists());
        assert!(provider.recovery_root.is_dir());
        assert!(!provider.graph_root.join(".tine-sync/v2").exists());
        assert!(provider.graph_root.join(".tine-sync/recovery").is_dir());
        assert!(provider
            .state
            .graphs
            .read()
            .unwrap()
            .slot("main")
            .unwrap()
            .legacy_graph()
            .is_ok());

        let shared = RollbackFixture::new(Some("joining"));
        let shared_result = cancel_sparse_v2_at_paths(
            &shared.state,
            "main",
            Arc::clone(&shared.slot),
            &shared.private_root,
            &shared.recovery_root,
            None,
            shutdown_for_direct_files_escape,
        )
        .unwrap();
        assert!(matches!(
            shared_result.status.availability,
            SparseV2Availability::LegacyDefault
        ));
        assert!(!shared.private_root.exists());
        assert!(shared.recovery_root.is_dir());
    }

    #[test]
    fn facade_legacy_default_inspects_nothing_and_retains_nothing() {
        let facade = SyncRuntimeFacade;
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SyncRuntimeFacade>();

        let root = std::env::temp_dir().join(format!("tine-sync-facade-legacy-{}", Uuid::new_v4()));
        let opened = facade.open_explicit(SyncRuntimeOpenRequest {
            profile: SyncStorageProfile::LegacyDefault,
            clean_identities: None,
            graph_root: root.join("missing-graph"),
            enrollment_root: root.join("missing-enrollment"),
            archive_root: root.join("missing-archive"),
            receipt_root: root.join("missing-receipts"),
            database_path: root.join("missing.sqlite"),
            application_runtime_root: root.join("missing-runtime"),
            provider_root: root.join("missing-provider"),
            provider_journal_root: root.join("missing-provider-journal/device/journal"),
        });
        assert_eq!(opened.status, SyncRuntimeOpenStatus::LegacyDefault);
        assert!(opened.handle.is_none());
        assert!(!root.exists());
    }

    #[test]
    fn blank_slate_rebuild_is_only_for_unrecognized_pre_07_state() {
        for availability in [
            SparseV2Availability::Refused {
                reason_code: SUPERSEDED_LEGACY_REASON.into(),
                scenario_id: SUPERSEDED_LEGACY_REASON.into(),
                detail: None,
            },
            SparseV2Availability::Refused {
                reason_code: "open_refused".into(),
                scenario_id: ManagedStorageRefusalScenario::ProtocolIncompatible
                    .as_str()
                    .into(),
                detail: None,
            },
        ] {
            assert!(SparseV2Binding {
                availability,
                handle: None,
            }
            .requires_blank_slate_rebuild());
        }

        assert!(!SparseV2Binding {
            availability: SparseV2Availability::Refused {
                reason_code: "open_refused".into(),
                scenario_id: ManagedStorageRefusalScenario::DiskCorrupt.as_str().into(),
                detail: None,
            },
            handle: None,
        }
        .requires_blank_slate_rebuild());
    }

    #[test]
    fn blank_slate_archive_preserves_every_private_byte() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private/current");
        let recovery = root.path().join("recovery");
        std::fs::create_dir_all(private.join("nested")).unwrap();
        std::fs::write(private.join("binding.json"), b"unrecognized binding bytes").unwrap();
        std::fs::write(
            private.join("nested/evidence.bin"),
            b"\0\x01retained evidence",
        )
        .unwrap();

        let archived = archive_private_root(&private, &recovery)
            .unwrap()
            .expect("present private state must be archived");

        assert!(!private.exists());
        assert_eq!(
            std::fs::read(archived.join("binding.json")).unwrap(),
            b"unrecognized binding bytes"
        );
        assert_eq!(
            std::fs::read(archived.join("nested/evidence.bin")).unwrap(),
            b"\0\x01retained evidence"
        );
    }

    #[test]
    fn blank_slate_retry_preserves_original_once_and_bounds_failed_candidates() {
        let root = tempfile::tempdir().unwrap();
        let graph = root.path().join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        let private = root.path().join("private/current");
        let recovery = root.path().join("recovery");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("original.bin"), b"unrecognized original").unwrap();
        let record =
            SparseV2ActivationRecord::new(&graph, Graph::open(&graph).meta(), DeviceId::new());

        prepare_blank_slate_retry_at_paths(&private, &recovery, record.clone()).unwrap();
        assert!(blank_slate_backup_is_complete(&private, &recovery).unwrap());

        for attempt in [
            b"first failed candidate".as_slice(),
            b"latest failed candidate",
        ] {
            std::fs::create_dir_all(&private).unwrap();
            std::fs::write(private.join("attempt.bin"), attempt).unwrap();
            prepare_blank_slate_retry_at_paths(&private, &recovery, record.clone()).unwrap();
        }

        let entries = std::fs::read_dir(&recovery)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        let failed = entries
            .iter()
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(BLANK_SLATE_FAILED_CANDIDATE_SUFFIX)
            })
            .collect::<Vec<_>>();
        assert_eq!(failed.len(), 1);
        assert_eq!(
            std::fs::read(failed[0].join("attempt.bin")).unwrap(),
            b"latest failed candidate"
        );
        let originals = entries
            .iter()
            .filter(|path| {
                path.is_dir()
                    && !path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with(BLANK_SLATE_FAILED_CANDIDATE_SUFFIX)
            })
            .collect::<Vec<_>>();
        assert_eq!(originals.len(), 1);
        assert_eq!(
            std::fs::read(originals[0].join("original.bin")).unwrap(),
            b"unrecognized original"
        );
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
            default_home: None,
            favorites: Vec::new(),
            favorites_page: None,
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
            linked_references_collapsed_threshold: 100,
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
    fn candidate_publication_crash_cuts_select_only_direct_or_managed() {
        let root = std::env::temp_dir().join(format!("tine-selector-cuts-{}", Uuid::new_v4()));
        let graph = root.join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        let meta = Graph::open(&graph).meta();
        let record = SparseV2ActivationRecord::new(&graph, meta, DeviceId::new());
        let private = root.join("private");
        let binding = private.join(SPARSE_BINDING_FILE);
        let direct = root.join("direct.json");

        std::fs::create_dir_all(private.join("candidate-residue")).unwrap();
        assert!(read_binding_at(&binding, &graph).unwrap().is_none());
        assert!(!direct_selection_is_active_at(&direct, &graph).unwrap());

        persist_binding_at(&binding, &record).unwrap();
        assert!(read_binding_at(&binding, &graph).unwrap().is_some());

        publish_direct_selection_at(&direct, &graph, "publication rollback").unwrap();
        assert!(direct_selection_is_active_at(&direct, &graph).unwrap());
        assert!(read_binding_at(&binding, &graph).unwrap().is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Set `TINE_SYNC_JOIN_RETAINED_AUTHORITY_GRAPH` to a read-only graph
    /// corpus to run this same journey at real graph scale. Both participants
    /// are copied before activation; the source corpus is never opened for
    /// writes.
    #[test]
    fn direct_join_archives_a_retained_different_authority_before_local_active() {
        std::thread::Builder::new()
            .name("tine-direct-join-retained-authority-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(direct_join_archives_a_retained_different_authority_before_local_active_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn direct_join_archives_a_retained_different_authority_before_local_active_inner() {
        fn settle_initial_feed(handle: &SyncRuntimeHandle) {
            for _ in 0..128 {
                let status = handle.status().unwrap();
                if !status.watcher.pending && status.provider_pending == 0 {
                    return;
                }
                handle.tick().unwrap();
            }
            panic!("managed activation did not settle its initial feed");
        }

        fn copy_tree(source: &Path, destination: &Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }

        let root = std::env::temp_dir().join(format!(
            "tine-direct-join-retained-authority-{}",
            Uuid::new_v4()
        ));
        let initiator_graph = root.join("initiator-graph");
        let joiner_graph = root.join("joiner-graph");
        let initiator_private = root.join("initiator-private");
        let joiner_private = root.join("joiner-private");
        let recovery_root = root.join("recovery");
        let real_graph =
            std::env::var_os("TINE_SYNC_JOIN_RETAINED_AUTHORITY_GRAPH").map(PathBuf::from);
        for graph in [&initiator_graph, &joiner_graph] {
            if let Some(source) = &real_graph {
                copy_tree(source, graph);
                let inherited_managed = graph.join(".tine-sync");
                if inherited_managed.exists() {
                    std::fs::remove_dir_all(inherited_managed).unwrap();
                }
            }
            std::fs::create_dir_all(graph.join("pages")).unwrap();
            std::fs::create_dir_all(graph.join("journals")).unwrap();
            std::fs::write(
                graph.join("pages/shared.md"),
                b"- byte-identical shared outline\n",
            )
            .unwrap();
        }

        let initiator_record = SparseV2ActivationRecord::new(
            &initiator_graph,
            Graph::open(&initiator_graph).meta(),
            DeviceId::new(),
        );
        let initiator = SyncRuntimeHandle::activate_or_resume_local(
            initiator_record.activation_request_at(&initiator_private),
        );
        assert_eq!(initiator.status, SyncLocalActivationStatus::Active);
        let initiator = initiator.handle.unwrap();
        settle_initial_feed(&initiator);
        let descriptor = initiator.prepare_shared().unwrap();

        let old_record = SparseV2ActivationRecord::new(
            &joiner_graph,
            Graph::open(&joiner_graph).meta(),
            DeviceId::new(),
        );
        let old_request = old_record.activation_request_at(&joiner_private);
        let old = SyncRuntimeHandle::activate_or_resume_local(old_request.clone());
        assert_eq!(old.status, SyncLocalActivationStatus::Active);
        let old = old.handle.unwrap();
        settle_initial_feed(&old);
        assert!(matches!(
            old.clean_shutdown(),
            Ok(SyncShutdownOutcome::Safe(_))
        ));
        drop(old);
        persist_binding_at(&joiner_private.join(SPARSE_BINDING_FILE), &old_record).unwrap();
        std::fs::write(
            joiner_private.join("retained-diagnostic-bytes"),
            b"preserve this predecessor exactly",
        )
        .unwrap();
        let predecessor = snapshot_tree(&joiner_private);

        copy_tree(
            &initiator_graph.join(".tine-sync/v2/shared"),
            &joiner_graph.join(".tine-sync/v2/shared"),
        );
        let joined_record = prepare_shared_binding_record_at_paths(
            &joiner_private,
            &recovery_root,
            &joiner_graph,
            Graph::open(&joiner_graph).meta(),
            DeviceId::new(),
            &descriptor,
        )
        .unwrap();
        let joined_request = joined_record.activation_request_at(&joiner_private);
        let activated = SyncRuntimeHandle::activate_or_resume_local(joined_request.clone());
        let activated = SparseV2Binding::from_activation(activated);
        assert!(
            matches!(activated.availability(), SparseV2Availability::Active),
            "a Direct Files join must not try to reopen the retained different catalog authority: {:?}",
            activated.availability()
        );
        let joined = activated.handle().unwrap().clone();
        settle_initial_feed(&joined);
        joined.join_shared(descriptor).unwrap();
        drop(joined);

        let reopened = SyncRuntimeHandle::open(joined_record.open_request_at(&joiner_private));
        assert_eq!(reopened.status, SyncRuntimeOpenStatus::Active);
        let reopened = reopened.handle.unwrap();
        assert_eq!(
            reopened.status().unwrap().shared_role,
            Some(SyncSharedRole::Joiner)
        );
        assert!(matches!(
            reopened.clean_shutdown(),
            Ok(SyncShutdownOutcome::Safe(_))
        ));

        assert!(!joiner_private.join("retained-diagnostic-bytes").exists());
        let archives = std::fs::read_dir(&recovery_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        assert_eq!(snapshot_tree(&archives[0]), predecessor);
        assert_eq!(
            std::fs::read(joiner_graph.join("pages/shared.md")).unwrap(),
            b"- byte-identical shared outline\n"
        );
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
            serde_json::to_value(SyncRuntimeQueryReply::SearchBuilding {
                horizon_sequence: 12,
            })
            .unwrap(),
            serde_json::json!({
                "kind": "search_building",
                "value": { "horizon_sequence": 12 }
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
            clean_identities: Some(request.identities.clone()),
            graph_root: request.graph_root.clone(),
            enrollment_root: request.enrollment_root.clone(),
            archive_root: request.archive_root.clone(),
            receipt_root: request.receipt_root.clone(),
            database_path: request.database_path.clone(),
            application_runtime_root: request.application_runtime_root.clone(),
            provider_root: request.provider_root.clone(),
            provider_journal_root: request.provider_journal_root.clone(),
        };

        let activated = SyncRuntimeHandle::activate_or_resume_local(request);
        assert_eq!(activated.status, SyncLocalActivationStatus::Active);
        let binding = SparseV2Binding::from_activation(activated);
        let slot =
            crate::state::GraphSlot::from_sparse_v2(binding, graph_root.clone(), meta.clone());
        assert_eq!(
            slot.legacy_graph().err().unwrap().to_string(),
            crate::state::SPARSE_V2_UNSUPPORTED
        );
        let handle = slot
            .sparse_runtime()
            .expect("active sparse slot must retain the actor");
        let admission = slot.application_page_admission();
        assert_eq!(admission.binding_generation, slot.binding_generation);
        assert!(matches!(
            admission.authority,
            crate::state::ApplicationPageAdmissionAuthority::ManagedWritable {
                application_save_page_blocks: tine_core::sync_runtime::MAX_SYNC_EDITOR_BLOCKS,
                application_page_request_text_bytes:
                    tine_core::sync_runtime::MAX_SYNC_EDITOR_REQUEST_BYTES,
                application_page_max_depth: tine_core::sync_runtime::MAX_SYNC_EDITOR_DEPTH,
            }
        ));
        assert_eq!(
            serde_json::to_value(sparse_v2_status_for_slot(&slot).unwrap()).unwrap()
                ["application_page_admission"]["application_save_page_blocks"],
            511
        );
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

        // The legacy raw SQLite query surface is derivative by design; the
        // application page/navigation surfaces carry the foreground overlay.
        // Drive the actor until that durable local prefix reaches SQLite, then
        // prove the lower-level query sees the same accepted page.
        let mut searched = SyncRuntimeQueryReply::Search(Vec::new());
        let mut last_tick = None;
        for _ in 0..64 {
            searched = handle
                .query(SyncRuntimeQueryRequest::Search {
                    query: "Tauri boundary".into(),
                    limit: 10,
                })
                .unwrap();
            if matches!(searched, SyncRuntimeQueryReply::Search(ref rows) if !rows.is_empty()) {
                break;
            }
            last_tick = Some(handle.tick().unwrap());
        }
        assert!(
            matches!(searched, SyncRuntimeQueryReply::Search(ref rows) if !rows.is_empty()),
            "actor query must see the durable editor save: {searched:?}; last_tick={last_tick:?}; status={:?}",
            handle.status().unwrap()
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
        assert_eq!(
            serde_json::to_value(stopped_status).unwrap()["application_page_admission"]
                ["authority"],
            "managed_unavailable"
        );
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

    #[test]
    fn stopped_move_episode_reopens_and_replaces_one_window_writer() {
        std::thread::Builder::new()
            .name("tine-move-recovery-handoff-test".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(stopped_move_episode_reopens_and_replaces_one_window_writer_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn stopped_move_episode_reopens_and_replaces_one_window_writer_inner() {
        let root =
            std::env::temp_dir().join(format!("tine-move-recovery-handoff-{}", Uuid::new_v4()));
        let graph_root = root.join("graph");
        let private = root.join("private");
        let source_path = "pages/Recovery Source.md";
        let destination_path = "pages/Recovery Destination.md";
        std::fs::create_dir_all(graph_root.join("pages")).unwrap();
        std::fs::create_dir_all(graph_root.join("journals")).unwrap();
        std::fs::write(graph_root.join(source_path), b"- source root\n").unwrap();
        std::fs::write(graph_root.join(destination_path), b"- destination root\n").unwrap();

        let meta = Graph::open(&graph_root).meta();
        let record = SparseV2ActivationRecord::new(&graph_root, meta.clone(), DeviceId::new());
        let activation_request = SyncLocalActivationRequest {
            graph_root: graph_root.clone(),
            archive_root: private.join("archive"),
            enrollment_root: private.join("enrollment"),
            receipt_root: private.join("receipts"),
            database_path: private.join("projection/materialization.sqlite"),
            application_runtime_root: private.join("runtime"),
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
            clean_identities: Some(activation_request.identities.clone()),
            graph_root: activation_request.graph_root.clone(),
            enrollment_root: activation_request.enrollment_root.clone(),
            archive_root: activation_request.archive_root.clone(),
            receipt_root: activation_request.receipt_root.clone(),
            database_path: activation_request.database_path.clone(),
            application_runtime_root: activation_request.application_runtime_root.clone(),
            provider_root: activation_request.provider_root.clone(),
            provider_journal_root: activation_request.provider_journal_root.clone(),
        };

        let activated = SyncRuntimeHandle::activate_or_resume_local(activation_request);
        assert_eq!(activated.status, SyncLocalActivationStatus::Active);
        let predecessor = Arc::new(crate::state::GraphSlot::from_sparse_v2(
            SparseV2Binding::from_activation(activated),
            graph_root.clone(),
            meta.clone(),
        ));
        let state = crate::state::AppState {
            graphs: std::sync::RwLock::new(crate::state::GraphRegistry::default()),
            storage_supervisor: crate::storage_mode_supervisor::StorageModeSupervisor::default(),
            watch_ctl: Mutex::new(None),
            last_focused: Mutex::new(None),
            capture_graph: Mutex::new(Default::default()),
            sync_runtime: SyncRuntimeFacade,
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        };
        state
            .graphs
            .write()
            .unwrap()
            .bind("main".into(), Arc::clone(&predecessor))
            .unwrap();
        let handle = predecessor.sparse_runtime().unwrap();
        for _ in 0..128 {
            let status = handle.status().unwrap();
            if !status.watcher.pending && status.provider_pending == 0 {
                break;
            }
            handle.tick().unwrap();
        }
        let source = match handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath {
                    path: source_path.into(),
                },
            })
            .unwrap()
        {
            SyncApplicationPageLoadOutcome::Loaded { page, revision } => (page, revision),
            other => panic!("source page did not load: {other:?}"),
        };
        let destination = match handle
            .load_application_page(SyncApplicationPageLoadRequest {
                page: SyncApplicationPageSelector::ExactPath {
                    path: destination_path.into(),
                },
            })
            .unwrap()
        {
            SyncApplicationPageLoadOutcome::Loaded { page, revision } => (page, revision),
            other => panic!("destination page did not load: {other:?}"),
        };
        let request = SyncApplicationMoveSubtreesRequest {
            episode_id: Uuid::new_v4().to_string(),
            source_path: source.0.path.clone(),
            source_revision: source.1,
            destination_path: destination.0.path.clone(),
            destination_revision: destination.1,
            roots: vec![SyncApplicationMoveRoot {
                identity: source.0.blocks[0].id.clone(),
                raw_rewrite: None,
            }],
            placement: SyncApplicationMovePlacement::Root { position: 0 },
            admission: SyncApplicationMoveAdmission {
                application_save_page_blocks: tine_core::sync_runtime::MAX_SYNC_EDITOR_BLOCKS,
                application_page_request_text_bytes:
                    tine_core::sync_runtime::MAX_SYNC_EDITOR_REQUEST_BYTES,
                application_page_max_depth: tine_core::sync_runtime::MAX_SYNC_EDITOR_DEPTH,
            },
        };
        // The ordinary move surface creates and commits the durable episode.
        // `resolve_application_move_subtrees` is recovery-only: asking it to
        // resolve a missing episode must remain a NoCommit rather than
        // silently initiating a user mutation (GH #333).
        let mut committed = handle.move_application_subtrees(request.clone()).unwrap();
        for _ in 0..512 {
            if matches!(
                committed,
                SyncApplicationMoveSubtreesOutcome::Committed { .. }
            ) {
                break;
            }
            assert!(
                matches!(
                    committed,
                    SyncApplicationMoveSubtreesOutcome::Deferred { .. }
                ),
                "initial move was refused: {committed:?}"
            );
            let _ = handle.tick().unwrap();
            committed = handle.move_application_subtrees(request.clone()).unwrap();
        }
        assert!(
            matches!(
                committed,
                SyncApplicationMoveSubtreesOutcome::Committed { .. }
            ),
            "initial move did not settle: {committed:?}; status={:?}",
            handle.status().unwrap(),
        );
        assert!(matches!(
            clean_shutdown_slot(&predecessor).unwrap(),
            CleanShutdownSlot::Safe
        ));
        assert_eq!(
            predecessor.sparse_binding().unwrap().action(),
            SparseV2BindingAction::ReopenActive
        );
        let previous_generation = predecessor.binding_generation;

        let failed = recover_managed_application_subtrees_with(
            &state,
            "main",
            previous_generation,
            request.clone(),
            |_| {
                Err(crate::command_error::CommandError::prose(
                    "pre-publication reopen failure",
                ))
            },
        );
        assert_eq!(failed.unwrap_err(), "pre-publication reopen failure");
        assert_eq!(
            state
                .graphs
                .read()
                .unwrap()
                .slot("main")
                .unwrap()
                .binding_generation,
            previous_generation
        );

        let result = recover_managed_application_subtrees_with(
            &state,
            "main",
            previous_generation,
            request.clone(),
            |_| {
                let opened = SyncRuntimeHandle::open(open_request.clone());
                assert_eq!(opened.status, SyncRuntimeOpenStatus::Active);
                Ok((SparseV2Binding::from_open(opened), meta.clone()))
            },
        )
        .unwrap();
        assert_eq!(result.previous_binding_generation, previous_generation);
        assert_ne!(result.binding_generation, previous_generation);
        assert_eq!(result.episode_id, request.episode_id);
        assert_eq!(
            result.application_page_admission.binding_generation,
            result.binding_generation
        );
        assert_eq!(
            result.status.application_page_admission,
            result.application_page_admission
        );
        let result_generation = result.binding_generation;
        match result.outcome {
            SyncApplicationMoveSubtreesOutcome::Committed {
                recovered: true,
                source,
                destination,
                ..
            } => {
                assert!(source.page.blocks.is_empty());
                assert_eq!(destination.page.blocks.len(), 2);
            }
            other => panic!("reopened episode did not resolve: {other:?}"),
        }
        let current = state.graphs.read().unwrap().slot("main").unwrap();
        assert_eq!(current.binding_generation, result_generation);
        assert_eq!(
            predecessor
                .sparse_runtime()
                .unwrap()
                .status()
                .unwrap()
                .lifecycle,
            SyncRuntimeLifecycle::StoppedSafe
        );
        let duplicate = SyncRuntimeHandle::open(open_request);
        assert!(matches!(
            duplicate.status,
            SyncRuntimeOpenStatus::OpenRefused { .. }
        ));
        assert!(duplicate.handle.is_none());
        assert!(matches!(
            clean_shutdown_slot(&current).unwrap(),
            CleanShutdownSlot::Safe
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
