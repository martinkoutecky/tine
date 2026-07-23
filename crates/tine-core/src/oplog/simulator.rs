//! Deterministic, store-backed adversarial replay for the oplog boundary.
//!
//! This module deliberately keeps the provider as bytes only.  Every replica
//! owns a distinct archive directory and stages bytes through `ObjectStore`;
//! no `PreparedBatch`, `ValidatedBatch`, or engine state crosses replicas.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    AuthorBatch, BatchDisposition, BatchId, BatchInspection, CanonicalSnapshot, CrdtPeerId,
    DeviceId, DocumentId, EngineError, EngineStatus, ImmutableHomeEvidence, LineageDigest,
    ObjectStore, OperationTransaction, SemanticOperation, SessionId, ShardedHotEngine,
    StageOutcome, WorkspaceId, WorkspaceStatus,
};

pub const SCENARIO_SCHEMA_VERSION: u32 = 2;
pub const MAX_SCENARIO_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SCENARIO_ACTIONS: usize = 16_384;
pub const MAX_SCENARIO_DEVICES: usize = 8;
pub const MAX_SCENARIO_WIRE_ITEMS: usize = 4_096;
pub const MAX_SCENARIO_WIRE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TRANSFERS_PER_DEVICE: usize = 32;
pub const MAX_TRANSFER_BYTES: usize = 1024 * 1024;
pub const FAILURE_CAPSULE_SCHEMA_VERSION: u32 = 2;
pub const MAX_FAILURE_CAPSULE_BYTES: usize = 64 * 1024;
pub const MAX_MINIMIZATION_REPLAYS: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioWorkspace {
    pub workspace_id: WorkspaceId,
    pub lineage_digest: LineageDigest,
    pub catalog_document_id: DocumentId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDevice {
    pub name: String,
    pub device_id: DeviceId,
    pub crdt_peer_id: CrdtPeerId,
}

/// Canonical unpadded base64url bytes used by scenario fixtures.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WireBytes(pub Vec<u8>);

impl From<Vec<u8>> for WireBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl AsRef<[u8]> for WireBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for WireBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64url_encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for WireBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        let bytes = base64url_decode(&encoded).map_err(serde::de::Error::custom)?;
        if base64url_encode(&bytes) != encoded {
            return Err(serde::de::Error::custom(
                "wire bytes are not canonical base64url",
            ));
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderItemKind {
    Object,
    Manifest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireItem {
    pub item_id: String,
    pub bytes_b64: WireBytes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireBatch {
    pub name: String,
    pub batch_id: BatchId,
    pub manifest: WireItem,
    pub objects: Vec<WireItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialReplica {
    pub device: String,
    pub stored_items: Vec<String>,
    pub expected: ReplicaExpectation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedWorkspaceState {
    Operational,
    Blocked { evidence: ImmutableHomeEvidence },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicaExpectation {
    pub accepted: Vec<BatchId>,
    pub offered: Vec<BatchId>,
    pub state: ExpectedWorkspaceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<CanonicalSnapshot>,
}

impl ReplicaExpectation {
    pub fn operational(accepted: Vec<BatchId>, offered: Vec<BatchId>) -> Self {
        Self {
            accepted,
            offered,
            state: ExpectedWorkspaceState::Operational,
            snapshot: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalFileFixture {
    /// Relative to the scenario root, never to a replica archive.
    pub path: String,
    pub bytes_b64: WireBytes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteMutation {
    Exact,
    Truncate {
        len: usize,
    },
    XorByte {
        offset: usize,
        mask: u8,
    },
    Insert {
        offset: usize,
        bytes_b64: WireBytes,
    },
    ReplaceRange {
        start: usize,
        end: usize,
        bytes_b64: WireBytes,
    },
    /// Submit another provider object's bytes using this action's item kind.
    /// This models a stale/conflicting provider copy or wrong-object lookup.
    Substitute {
        item_id: String,
    },
}

impl Default for ByteMutation {
    fn default() -> Self {
        Self::Exact
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressExpectation {
    Accepted,
    Rejected { error: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageExpectation {
    Accepted,
    Incomplete,
    Rejected,
    Quarantined,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantAssertion {
    Replica {
        device: String,
        expected: ReplicaExpectation,
    },
    Converged {
        devices: Vec<String>,
    },
    NoVisibleEffect {
        device: String,
        snapshot: CanonicalSnapshot,
    },
    Ingress {
        event_id: u64,
        expected: IngressExpectation,
    },
    LineageIsolation {
        device: String,
        accepted: Vec<BatchId>,
    },
    RestartReplay {
        device: String,
    },
    UntouchedExternalFiles,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledActionKind {
    AuthorLocal {
        device: String,
        batch_id: BatchId,
        session_id: SessionId,
        transaction: OperationTransaction,
    },
    DeliverItem {
        device: String,
        item_id: String,
        #[serde(default)]
        mutation: ByteMutation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<IngressExpectation>,
    },
    /// A provider-side immutable copy.  It deliberately copies only raw bytes
    /// and their declared ingress kind, never a decoded protocol value.
    CopyProviderItem {
        source_item_id: String,
        copy_item_id: String,
    },
    /// A provider outage/drop.  Reordering and delay are represented by the
    /// trace's `(tick, event_id)` ordering, and duplication by repeated delivery.
    DropProviderItem {
        item_id: String,
    },
    BeginTransfer {
        device: String,
        transfer_id: String,
        item_id: String,
    },
    AppendTransfer {
        device: String,
        transfer_id: String,
        len: usize,
    },
    CommitTransfer {
        device: String,
        transfer_id: String,
        #[serde(default)]
        mutation: ByteMutation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<IngressExpectation>,
    },
    AbortTransfer {
        device: String,
        transfer_id: String,
    },
    ProbeBatch {
        device: String,
        batch_id: BatchId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected: Option<StageExpectation>,
    },
    Crash {
        device: String,
    },
    Restart {
        device: String,
    },
    AssertInvariant {
        assertion: InvariantAssertion,
    },
    /// Compatibility-only whole-batch delivery. New v2 scenarios use exact
    /// raw items and an explicit probe instead.
    LegacyDeliver {
        device: String,
        batch_id: BatchId,
    },
    LegacyAssertConverged {
        devices: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledAction {
    pub event_id: u64,
    pub tick: u64,
    pub action: ScheduledActionKind,
}

/// Compatibility input retained for the hot-engine tests that predate the
/// v2 corpus.  It is normalized to scheduled raw-byte actions at runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioAction {
    LocalTransaction {
        device: usize,
        batch_id: BatchId,
        session_id: SessionId,
        transaction: OperationTransaction,
    },
    Deliver {
        device: usize,
        batch_id: BatchId,
    },
    DuplicateDelivery {
        device: usize,
        batch_id: BatchId,
    },
    AssertConverged {
        devices: Vec<usize>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    scenario_schema_version: u32,
    pub family: String,
    pub seed: u64,
    pub workspace: ScenarioWorkspace,
    pub devices: Vec<ScenarioDevice>,
    #[serde(default)]
    pub wire_batches: Vec<WireBatch>,
    #[serde(default)]
    pub initial_replicas: Vec<InitialReplica>,
    pub actions: Vec<ScheduledAction>,
    #[serde(default)]
    pub terminal: Vec<InitialReplica>,
    #[serde(default)]
    pub external_files: Vec<ExternalFileFixture>,
}

impl Scenario {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        family: impl Into<String>,
        seed: u64,
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
        catalog_document_id: DocumentId,
        devices: Vec<ScenarioDevice>,
        actions: Vec<ScenarioAction>,
    ) -> Result<Self, ScenarioError> {
        let actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| {
                let device_name = |device: usize| {
                    devices
                        .get(device)
                        .map(|device| device.name.clone())
                        .ok_or(ScenarioError::UnknownDevice(device))
                };
                let action = match action {
                    ScenarioAction::LocalTransaction {
                        device,
                        batch_id,
                        session_id,
                        transaction,
                    } => ScheduledActionKind::AuthorLocal {
                        device: device_name(device)?,
                        batch_id,
                        session_id,
                        transaction,
                    },
                    ScenarioAction::Deliver { device, batch_id }
                    | ScenarioAction::DuplicateDelivery { device, batch_id } => {
                        ScheduledActionKind::LegacyDeliver {
                            device: device_name(device)?,
                            batch_id,
                        }
                    }
                    ScenarioAction::AssertConverged { devices: asserted } => {
                        ScheduledActionKind::LegacyAssertConverged {
                            devices: asserted
                                .into_iter()
                                .map(device_name)
                                .collect::<Result<_, _>>()?,
                        }
                    }
                };
                Ok(ScheduledAction {
                    event_id: index as u64 + 1,
                    tick: index as u64,
                    action,
                })
            })
            .collect::<Result<Vec<_>, ScenarioError>>()?;
        Self::from_schedule(
            family,
            seed,
            ScenarioWorkspace {
                workspace_id,
                lineage_digest,
                catalog_document_id,
            },
            devices,
            Vec::new(),
            Vec::new(),
            actions,
            Vec::new(),
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_schedule(
        family: impl Into<String>,
        seed: u64,
        workspace: ScenarioWorkspace,
        devices: Vec<ScenarioDevice>,
        wire_batches: Vec<WireBatch>,
        initial_replicas: Vec<InitialReplica>,
        actions: Vec<ScheduledAction>,
        terminal: Vec<InitialReplica>,
        external_files: Vec<ExternalFileFixture>,
    ) -> Result<Self, ScenarioError> {
        let scenario = Self {
            scenario_schema_version: SCENARIO_SCHEMA_VERSION,
            family: family.into(),
            seed,
            workspace,
            devices,
            wire_batches,
            initial_replicas,
            actions,
            terminal,
            external_files,
        };
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn encode(&self) -> Result<Vec<u8>, ScenarioError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| ScenarioError::Encode(error.to_string()))?;
        if bytes.len() > MAX_SCENARIO_BYTES {
            return Err(ScenarioError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ScenarioError> {
        if bytes.len() > MAX_SCENARIO_BYTES {
            return Err(ScenarioError::TooLarge(bytes.len()));
        }
        let scenario: Self = serde_json::from_slice(bytes)
            .map_err(|error| ScenarioError::Decode(error.to_string()))?;
        scenario.validate()?;
        if scenario.encode()?.as_slice() != bytes {
            return Err(ScenarioError::NonCanonical);
        }
        Ok(scenario)
    }

    /// A specified xorshift permutation for deterministic generators.  The
    /// trace itself is serialized, so replay never depends on this RNG.
    pub fn permutation(&self, length: usize) -> Vec<usize> {
        let mut values: Vec<_> = (0..length).collect();
        let mut state = self.seed ^ 0x9e37_79b9_7f4a_7c15;
        for index in (1..length).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            values.swap(index, (state as usize) % (index + 1));
        }
        values
    }

    /// Deterministic ddmin-style reduction.  A candidate is retained only if
    /// its first failure has the exact original identity, never merely a broad
    /// "diverged" classification.
    pub fn minimize_failure(&self) -> Result<MinimizedScenario, ScenarioError> {
        self.validate()?;
        let original_replay = replay_failure_details(self)?.ok_or(ScenarioError::NotFailing)?;
        let original_identity = original_replay
            .error
            .failure_identity()
            .ok_or(ScenarioError::UnstableFailure)?;
        let original_action_count = self.actions.len();
        let mut minimized = self.clone();
        let mut replays = 1usize;
        let mut exhausted = false;
        let protected = match &original_identity {
            FailureIdentity::Invariant(signature) => Some(signature.assertion_or_event_id),
            _ => None,
        };

        let mut granularity = 2usize;
        while minimized.actions.len() >= 2 && replays < MAX_MINIMIZATION_REPLAYS {
            let len = minimized.actions.len();
            let chunk = len.div_ceil(granularity);
            let mut reduced = false;
            for start in (0..len).step_by(chunk) {
                let end = (start + chunk).min(len);
                if protected.is_some_and(|event| {
                    minimized.actions[start..end]
                        .iter()
                        .any(|action| action.event_id == event)
                }) {
                    continue;
                }
                let mut candidate = minimized.clone();
                candidate.actions.drain(start..end);
                repair_trace(&mut candidate.actions);
                if candidate.validate().is_err() {
                    continue;
                }
                replays += 1;
                if replays > MAX_MINIMIZATION_REPLAYS {
                    exhausted = true;
                    break;
                }
                if reproduces(&candidate, &original_identity)? {
                    minimized = candidate;
                    granularity = granularity.saturating_sub(1).max(2);
                    reduced = true;
                    break;
                }
            }
            if exhausted {
                break;
            }
            if !reduced {
                if granularity >= minimized.actions.len() {
                    break;
                }
                granularity = (granularity * 2).min(minimized.actions.len());
            }
        }

        for index in (0..minimized.actions.len()).rev() {
            if replays >= MAX_MINIMIZATION_REPLAYS {
                exhausted = true;
                break;
            }
            if protected == Some(minimized.actions[index].event_id) {
                continue;
            }
            let mut candidate = minimized.clone();
            candidate.actions.remove(index);
            repair_trace(&mut candidate.actions);
            if candidate.validate().is_err() {
                continue;
            }
            replays += 1;
            if reproduces(&candidate, &original_identity)? {
                minimized = candidate;
            }
        }

        // The final pass keeps action identity/timing stable while reducing the
        // remaining witness payloads. This is intentionally small and
        // deterministic: it is a failure explanation aid, not a fuzzer.
        let mut changed = true;
        while changed && replays < MAX_MINIMIZATION_REPLAYS {
            changed = false;
            for index in 0..minimized.actions.len() {
                for action in shrink_action_candidates(&minimized.actions[index]) {
                    if replays >= MAX_MINIMIZATION_REPLAYS {
                        exhausted = true;
                        break;
                    }
                    let mut candidate = minimized.clone();
                    candidate.actions[index] = action;
                    repair_trace(&mut candidate.actions);
                    if candidate.validate().is_err() {
                        continue;
                    }
                    replays += 1;
                    if reproduces(&candidate, &original_identity)? {
                        minimized = candidate;
                        changed = true;
                        break;
                    }
                }
                if changed || exhausted {
                    break;
                }
            }
        }

        let capsule = FailureCapsule {
            schema_version: FAILURE_CAPSULE_SCHEMA_VERSION,
            family: self.family.clone(),
            original_seed: self.seed,
            failure: original_identity,
            tested_commit: option_env!("TINE_GIT_COMMIT").unwrap_or("unknown").into(),
            scenario_hash: scenario_hash(self)?,
            first_failing_event: first_failure_event(&original_replay.error),
            ingress_receipt: original_replay.ingress_receipt,
            accepted_witness: original_replay.accepted_witness,
            offered_witness: original_replay.offered_witness,
            status_witness: original_replay.status_witness,
            expected_snapshot_hash: original_replay.expected_snapshot_hash,
            observed_snapshot_hash: original_replay.observed_snapshot_hash,
            first_canonical_difference: original_replay.first_canonical_difference,
            original_action_count,
            minimized_action_count: minimized.actions.len(),
            minimization_replays: replays,
            minimization_budget: MAX_MINIMIZATION_REPLAYS,
            minimization_budget_exhausted: exhausted,
        };
        capsule.validate()?;
        Ok(MinimizedScenario {
            scenario: minimized,
            capsule,
        })
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        if self.scenario_schema_version != SCENARIO_SCHEMA_VERSION {
            return Err(ScenarioError::UnknownVersion(self.scenario_schema_version));
        }
        if !valid_name(&self.family, 256) {
            return Err(ScenarioError::InvalidFamily);
        }
        if self.devices.is_empty() || self.devices.len() > MAX_SCENARIO_DEVICES {
            return Err(ScenarioError::InvalidDeviceCount(self.devices.len()));
        }
        if self.actions.len() > MAX_SCENARIO_ACTIONS {
            return Err(ScenarioError::TooManyActions(self.actions.len()));
        }
        let mut names = BTreeSet::new();
        let mut device_ids = BTreeSet::new();
        let mut peer_ids = BTreeSet::new();
        for device in &self.devices {
            if !valid_name(&device.name, 128)
                || device.crdt_peer_id.as_u64() == 0
                || !names.insert(device.name.clone())
                || !device_ids.insert(device.device_id)
                || !peer_ids.insert(device.crdt_peer_id)
            {
                return Err(ScenarioError::InvalidDevice);
            }
        }

        let mut item_ids = BTreeSet::new();
        let mut batches = BTreeSet::new();
        let mut wire_bytes = 0usize;
        for batch in &self.wire_batches {
            if !valid_name(&batch.name, 256) || !batches.insert(batch.batch_id) {
                return Err(ScenarioError::InvalidWireBatch);
            }
            for item in std::iter::once(&batch.manifest).chain(&batch.objects) {
                if !valid_name(&item.item_id, 256) || !item_ids.insert(item.item_id.clone()) {
                    return Err(ScenarioError::InvalidWireItem(item.item_id.clone()));
                }
                wire_bytes = wire_bytes.saturating_add(item.bytes_b64.0.len());
            }
        }
        if item_ids.len() > MAX_SCENARIO_WIRE_ITEMS || wire_bytes > MAX_SCENARIO_WIRE_BYTES {
            return Err(ScenarioError::WireTooLarge(wire_bytes));
        }
        let mut event_ids = BTreeSet::new();
        let mut last_schedule = None;
        for action in &self.actions {
            if action.event_id == 0 || !event_ids.insert(action.event_id) {
                return Err(ScenarioError::InvalidSchedule);
            }
            let key = (action.tick, action.event_id);
            if last_schedule.is_some_and(|last| last >= key) {
                return Err(ScenarioError::InvalidSchedule);
            }
            last_schedule = Some(key);
            validate_scheduled_action(&action.action, &names)?;
        }
        for replica in self.initial_replicas.iter().chain(&self.terminal) {
            validate_replica_expectation(replica, &names, &item_ids)?;
        }
        let mut external_paths = BTreeSet::new();
        for file in &self.external_files {
            if !valid_relative_path(&file.path) || !external_paths.insert(file.path.clone()) {
                return Err(ScenarioError::InvalidExternalFile(file.path.clone()));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Scenario {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            scenario_schema_version: u32,
            family: String,
            seed: u64,
            workspace: ScenarioWorkspace,
            devices: Vec<ScenarioDevice>,
            #[serde(default)]
            wire_batches: Vec<WireBatch>,
            #[serde(default)]
            initial_replicas: Vec<InitialReplica>,
            actions: Vec<ScheduledAction>,
            #[serde(default)]
            terminal: Vec<InitialReplica>,
            #[serde(default)]
            external_files: Vec<ExternalFileFixture>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let scenario = Self {
            scenario_schema_version: wire.scenario_schema_version,
            family: wire.family,
            seed: wire.seed,
            workspace: wire.workspace,
            devices: wire.devices,
            wire_batches: wire.wire_batches,
            initial_replicas: wire.initial_replicas,
            actions: wire.actions,
            terminal: wire.terminal,
            external_files: wire.external_files,
        };
        scenario.validate().map_err(serde::de::Error::custom)?;
        Ok(scenario)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantPredicate {
    SameAcceptedClosureSnapshot,
    SameOfferedClosureStatus,
    NoVisibleEffect,
    IngressOutcome,
    LineageIsolation,
    RestartReplay,
    ExpectedReplica,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantSignature {
    pub assertion_or_event_id: u64,
    pub predicate: InvariantPredicate,
    pub subject: Vec<String>,
    pub required_closure_or_lineage: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureIdentity {
    Invariant(InvariantSignature),
    // Kept so the pre-v2 compatibility tests retain their public assertion.
    Action(String),
    Diverged,
    GlobalOracleDiverged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressReceipt {
    pub event_id: u64,
    pub device: String,
    pub item_id: String,
    pub item_kind: ProviderItemKind,
    pub byte_len: usize,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureCapsule {
    schema_version: u32,
    pub family: String,
    pub original_seed: u64,
    pub failure: FailureIdentity,
    pub tested_commit: String,
    pub scenario_hash: String,
    pub first_failing_event: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress_receipt: Option<IngressReceipt>,
    #[serde(default)]
    pub accepted_witness: BTreeMap<String, Vec<BatchId>>,
    #[serde(default)]
    pub offered_witness: BTreeMap<String, Vec<BatchId>>,
    #[serde(default)]
    pub status_witness: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_canonical_difference: Option<String>,
    pub original_action_count: usize,
    pub minimized_action_count: usize,
    pub minimization_replays: usize,
    pub minimization_budget: usize,
    pub minimization_budget_exhausted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MinimizedScenario {
    pub scenario: Scenario,
    pub capsule: FailureCapsule,
}

impl FailureCapsule {
    pub fn encode(&self) -> Result<Vec<u8>, ScenarioError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| ScenarioError::Encode(error.to_string()))?;
        if bytes.len() > MAX_FAILURE_CAPSULE_BYTES {
            return Err(ScenarioError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ScenarioError> {
        if bytes.len() > MAX_FAILURE_CAPSULE_BYTES {
            return Err(ScenarioError::TooLarge(bytes.len()));
        }
        let capsule: Self = serde_json::from_slice(bytes)
            .map_err(|error| ScenarioError::Decode(error.to_string()))?;
        capsule.validate()?;
        if capsule.encode()?.as_slice() != bytes {
            return Err(ScenarioError::NonCanonical);
        }
        Ok(capsule)
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        if self.schema_version != FAILURE_CAPSULE_SCHEMA_VERSION
            || !valid_name(&self.family, 256)
            || self.minimized_action_count > self.original_action_count
            || self.minimization_replays > self.minimization_budget
        {
            return Err(ScenarioError::InvalidFailureCapsule);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimulatorDeviceState {
    Operational(CanonicalSnapshot),
    Blocked(ImmutableHomeEvidence),
}

#[derive(Clone)]
struct ProviderItem {
    batch_id: Option<BatchId>,
    kind: ProviderItemKind,
    bytes: Arc<[u8]>,
}

#[derive(Default)]
struct ProviderMailbox {
    items: BTreeMap<String, ProviderItem>,
    dropped: BTreeSet<String>,
}

impl ProviderMailbox {
    fn insert(&mut self, item_id: String, item: ProviderItem) -> Result<(), ScenarioError> {
        if self.items.insert(item_id.clone(), item).is_some() {
            return Err(ScenarioError::InvalidWireItem(item_id));
        }
        Ok(())
    }

    fn item(&self, item_id: &str) -> Result<&ProviderItem, ScenarioError> {
        if self.dropped.contains(item_id) {
            return Err(ScenarioError::ProviderItemDropped(item_id.into()));
        }
        self.items
            .get(item_id)
            .ok_or_else(|| ScenarioError::UnknownItem(item_id.into()))
    }

    fn batch_items(&self, batch_id: BatchId) -> Vec<String> {
        let mut objects = Vec::new();
        let mut manifests = Vec::new();
        for (item_id, item) in &self.items {
            if item.batch_id == Some(batch_id) && !self.dropped.contains(item_id) {
                match item.kind {
                    ProviderItemKind::Object => objects.push(item_id.clone()),
                    ProviderItemKind::Manifest => manifests.push(item_id.clone()),
                }
            }
        }
        objects.extend(manifests);
        objects
    }
}

struct Transfer {
    item_id: String,
    kind: ProviderItemKind,
    source: Arc<[u8]>,
    next: usize,
    bytes: Vec<u8>,
}

struct DeviceRuntime {
    name: String,
    root: PathBuf,
    store: Option<ObjectStore>,
    engine: Option<ShardedHotEngine>,
    transfers: BTreeMap<String, Transfer>,
}

impl DeviceRuntime {
    fn open(
        root: PathBuf,
        identity: &ScenarioDevice,
        workspace: &ScenarioWorkspace,
    ) -> Result<Self, ScenarioError> {
        fs::create_dir_all(&root).map_err(|error| ScenarioError::Io(error.to_string()))?;
        let archive_path = root.join("archive");
        let store = ObjectStore::open(&archive_path, workspace.workspace_id)
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let engine_store = ObjectStore::open(&archive_path, workspace.workspace_id)
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let engine = ShardedHotEngine::with_archive_store(
            engine_store,
            workspace.lineage_digest,
            workspace.catalog_document_id,
        );
        Ok(Self {
            name: identity.name.clone(),
            root,
            store: Some(store),
            engine: Some(engine),
            transfers: BTreeMap::new(),
        })
    }

    fn archive_path(&self) -> PathBuf {
        self.root.join("archive")
    }

    fn store(&self) -> Result<&ObjectStore, ScenarioError> {
        self.store
            .as_ref()
            .ok_or_else(|| ScenarioError::DeviceCrashed(self.name.clone()))
    }

    fn engine(&self) -> Result<&ShardedHotEngine, ScenarioError> {
        self.engine
            .as_ref()
            .ok_or_else(|| ScenarioError::DeviceCrashed(self.name.clone()))
    }

    fn engine_mut(&mut self) -> Result<&mut ShardedHotEngine, ScenarioError> {
        self.engine
            .as_mut()
            .ok_or_else(|| ScenarioError::DeviceCrashed(self.name.clone()))
    }

    fn crash(&mut self) {
        self.engine.take();
        self.store.take();
        self.transfers.clear();
    }

    fn restart(
        &mut self,
        workspace: &ScenarioWorkspace,
    ) -> Result<Vec<StageOutcome>, ScenarioError> {
        if self.engine.is_some() || self.store.is_some() {
            return Err(ScenarioError::AlreadyRunning(self.name.clone()));
        }
        let store = ObjectStore::open(&self.archive_path(), workspace.workspace_id)
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let manifests = store
            .committed_manifests()
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let engine_store = ObjectStore::open(&self.archive_path(), workspace.workspace_id)
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let mut engine = ShardedHotEngine::with_archive_store(
            engine_store,
            workspace.lineage_digest,
            workspace.catalog_document_id,
        );
        let mut outcomes = Vec::new();
        for manifest in manifests {
            outcomes.push(
                engine
                    .stage_archive_batch(manifest.batch_id())
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?,
            );
        }
        self.store = Some(store);
        self.engine = Some(engine);
        Ok(outcomes)
    }
}

struct ScenarioRoot(PathBuf);

impl ScenarioRoot {
    fn new() -> Result<Self, ScenarioError> {
        let path = std::env::temp_dir().join(format!("tine-oplog-simulator-{}", Uuid::new_v4()));
        fs::create_dir(&path).map_err(|error| ScenarioError::Io(error.to_string()))?;
        Ok(Self(path))
    }
}

impl Drop for ScenarioRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct DeterministicSimulator {
    scenario: Scenario,
    root: ScenarioRoot,
    devices: BTreeMap<String, DeviceRuntime>,
    mailbox: ProviderMailbox,
    outcomes: Vec<StageOutcome>,
    receipts: BTreeMap<u64, IngressReceipt>,
}

impl DeterministicSimulator {
    pub fn new(scenario: Scenario) -> Result<Self, ScenarioError> {
        scenario.validate()?;
        let root = ScenarioRoot::new()?;
        let mut mailbox = ProviderMailbox::default();
        for wire_batch in &scenario.wire_batches {
            mailbox.insert(
                wire_batch.manifest.item_id.clone(),
                ProviderItem {
                    batch_id: Some(wire_batch.batch_id),
                    kind: ProviderItemKind::Manifest,
                    bytes: Arc::from(wire_batch.manifest.bytes_b64.0.clone()),
                },
            )?;
            for object in &wire_batch.objects {
                mailbox.insert(
                    object.item_id.clone(),
                    ProviderItem {
                        batch_id: Some(wire_batch.batch_id),
                        kind: ProviderItemKind::Object,
                        bytes: Arc::from(object.bytes_b64.0.clone()),
                    },
                )?;
            }
        }
        for file in &scenario.external_files {
            let path = root.0.join(&file.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| ScenarioError::Io(error.to_string()))?;
            }
            fs::write(path, &file.bytes_b64.0)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
        }
        let mut devices = BTreeMap::new();
        for identity in &scenario.devices {
            let runtime =
                DeviceRuntime::open(root.0.join(&identity.name), identity, &scenario.workspace)?;
            devices.insert(identity.name.clone(), runtime);
        }
        let mut simulator = Self {
            scenario,
            root,
            devices,
            mailbox,
            outcomes: Vec::new(),
            receipts: BTreeMap::new(),
        };
        for replica in simulator.scenario.initial_replicas.clone() {
            for item_id in replica.stored_items {
                simulator.ingress_item(0, &replica.device, &item_id, &ByteMutation::Exact, None)?;
            }
            let manifests = simulator
                .device(&replica.device)?
                .store()?
                .committed_manifests()
                .map_err(|error| ScenarioError::Store(error.to_string()))?;
            for manifest in manifests {
                let outcome = simulator
                    .device_mut(&replica.device)?
                    .engine_mut()?
                    .stage_archive_batch(manifest.batch_id())
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?;
                simulator.outcomes.push(outcome);
            }
            simulator.assert_replica(0, &replica.device, &replica.expected)?;
        }
        Ok(simulator)
    }

    pub fn run(&mut self) -> Result<(), ScenarioError> {
        let mut actions = self.scenario.actions.clone();
        actions.sort_unstable_by_key(|action| (action.tick, action.event_id));
        for action in actions {
            self.run_action(&action)?;
            self.check_global_oracle(action.event_id)?;
        }
        for replica in self.scenario.terminal.clone() {
            self.assert_replica(0, &replica.device, &replica.expected)?;
        }
        Ok(())
    }

    pub fn snapshots(&self) -> Result<Vec<CanonicalSnapshot>, EngineError> {
        self.scenario
            .devices
            .iter()
            .filter_map(|device| self.devices.get(&device.name))
            .filter_map(|device| device.engine.as_ref())
            .map(ShardedHotEngine::canonical_snapshot)
            .collect()
    }

    pub fn states(&self) -> Result<Vec<SimulatorDeviceState>, ScenarioError> {
        self.scenario
            .devices
            .iter()
            .filter_map(|device| self.devices.get(&device.name))
            .filter(|device| device.engine.is_some())
            .map(|device| device_state(device, 0))
            .collect()
    }

    pub fn statuses(&self) -> Vec<EngineStatus> {
        self.scenario
            .devices
            .iter()
            .filter_map(|device| self.devices.get(&device.name))
            .filter_map(|device| device.engine.as_ref())
            .map(ShardedHotEngine::status)
            .collect()
    }

    pub fn outcomes(&self) -> &[StageOutcome] {
        &self.outcomes
    }

    pub fn ingress_receipts(&self) -> &BTreeMap<u64, IngressReceipt> {
        &self.receipts
    }

    fn run_action(&mut self, scheduled: &ScheduledAction) -> Result<(), ScenarioError> {
        let event_id = scheduled.event_id;
        match &scheduled.action {
            ScheduledActionKind::AuthorLocal {
                device,
                batch_id,
                session_id,
                transaction,
            } => self.author_local(event_id, device, *batch_id, *session_id, transaction),
            ScheduledActionKind::DeliverItem {
                device,
                item_id,
                mutation,
                expected,
            } => self.ingress_item(event_id, device, item_id, mutation, expected.as_ref()),
            ScheduledActionKind::CopyProviderItem {
                source_item_id,
                copy_item_id,
            } => {
                let source = self.mailbox.item(source_item_id)?.clone();
                self.mailbox.insert(copy_item_id.clone(), source)
            }
            ScheduledActionKind::DropProviderItem { item_id } => {
                self.mailbox.item(item_id)?;
                self.mailbox.dropped.insert(item_id.clone());
                Ok(())
            }
            ScheduledActionKind::BeginTransfer {
                device,
                transfer_id,
                item_id,
            } => {
                let item = self.mailbox.item(item_id)?.clone();
                let runtime = self.device_mut(device)?;
                if runtime.transfers.len() >= MAX_TRANSFERS_PER_DEVICE
                    || runtime.transfers.contains_key(transfer_id)
                {
                    return Err(ScenarioError::InvalidTransfer(transfer_id.clone()));
                }
                runtime.transfers.insert(
                    transfer_id.clone(),
                    Transfer {
                        item_id: item_id.clone(),
                        kind: item.kind,
                        source: item.bytes,
                        next: 0,
                        bytes: Vec::new(),
                    },
                );
                Ok(())
            }
            ScheduledActionKind::AppendTransfer {
                device,
                transfer_id,
                len,
            } => {
                let runtime = self.device_mut(device)?;
                let transfer = runtime
                    .transfers
                    .get_mut(transfer_id)
                    .ok_or_else(|| ScenarioError::UnknownTransfer(transfer_id.clone()))?;
                let end = transfer
                    .next
                    .checked_add(*len)
                    .filter(|end| *end <= transfer.source.len())
                    .ok_or_else(|| ScenarioError::InvalidTransfer(transfer_id.clone()))?;
                if transfer.bytes.len().saturating_add(*len) > MAX_TRANSFER_BYTES {
                    return Err(ScenarioError::InvalidTransfer(transfer_id.clone()));
                }
                transfer
                    .bytes
                    .extend_from_slice(&transfer.source[transfer.next..end]);
                transfer.next = end;
                Ok(())
            }
            ScheduledActionKind::CommitTransfer {
                device,
                transfer_id,
                mutation,
                expected,
            } => {
                let transfer = self
                    .device_mut(device)?
                    .transfers
                    .remove(transfer_id)
                    .ok_or_else(|| ScenarioError::UnknownTransfer(transfer_id.clone()))?;
                self.ingress_bytes(
                    event_id,
                    device,
                    &transfer.item_id,
                    transfer.kind,
                    transfer.bytes,
                    mutation,
                    expected.as_ref(),
                )
            }
            ScheduledActionKind::AbortTransfer {
                device,
                transfer_id,
            } => {
                self.device_mut(device)?
                    .transfers
                    .remove(transfer_id)
                    .ok_or_else(|| ScenarioError::UnknownTransfer(transfer_id.clone()))?;
                Ok(())
            }
            ScheduledActionKind::ProbeBatch {
                device,
                batch_id,
                expected,
            } => {
                let outcome = self
                    .device_mut(device)?
                    .engine_mut()?
                    .stage_archive_batch(*batch_id)
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?;
                if let Some(expected) = expected {
                    if !stage_matches(expected, &outcome.disposition) {
                        return Err(self.invariant(
                            event_id,
                            InvariantPredicate::IngressOutcome,
                            vec![device.clone()],
                            vec![batch_id.to_string()],
                            "probe outcome differed from expectation",
                        ));
                    }
                }
                self.outcomes.push(outcome);
                Ok(())
            }
            ScheduledActionKind::Crash { device } => {
                self.device_mut(device)?.crash();
                Ok(())
            }
            ScheduledActionKind::Restart { device } => {
                let workspace = self.scenario.workspace.clone();
                let outcomes = self.device_mut(device)?.restart(&workspace)?;
                self.outcomes.extend(outcomes);
                self.assert_restart_replay(event_id, device)
            }
            ScheduledActionKind::AssertInvariant { assertion } => {
                self.assert_invariant(event_id, assertion)
            }
            ScheduledActionKind::LegacyDeliver { device, batch_id } => {
                for item_id in self.mailbox.batch_items(*batch_id) {
                    self.ingress_item(event_id, device, &item_id, &ByteMutation::Exact, None)?;
                }
                let outcome = self
                    .device_mut(device)?
                    .engine_mut()?
                    .stage_archive_batch(*batch_id)
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?;
                self.outcomes.push(outcome);
                Ok(())
            }
            ScheduledActionKind::LegacyAssertConverged { devices } => {
                let mut observations = devices
                    .iter()
                    .map(|device| self.observation(device, event_id))
                    .collect::<Result<Vec<_>, _>>()?;
                let Some(first) = observations.pop() else {
                    return Ok(());
                };
                if observations.into_iter().any(|other| other != first) {
                    return Err(ScenarioError::Diverged {
                        action_index: event_id as usize - 1,
                    });
                }
                Ok(())
            }
        }
    }

    fn author_local(
        &mut self,
        event_id: u64,
        device: &str,
        batch_id: BatchId,
        session_id: SessionId,
        transaction: &OperationTransaction,
    ) -> Result<(), ScenarioError> {
        let identity = self.identity(device)?.clone();
        let prepared = self
            .device(device)?
            .engine()?
            .prepare_transaction(
                AuthorBatch {
                    batch_id,
                    author_device_id: identity.device_id,
                    author_session_id: session_id,
                    crdt_peer_id: identity.crdt_peer_id,
                },
                transaction,
            )
            .map_err(|error| ScenarioError::Engine(error.to_string()))?;
        let mut objects = Vec::new();
        for (index, object) in prepared.objects().iter().enumerate() {
            let item_id = authored_item_id(batch_id, "object", index);
            let bytes = object
                .encode()
                .map_err(|error| ScenarioError::Engine(error.to_string()))?;
            objects.push((item_id, bytes));
        }
        let manifest_id = authored_item_id(batch_id, "manifest", 0);
        let manifest = prepared
            .manifest()
            .encode()
            .map_err(|error| ScenarioError::Engine(error.to_string()))?;
        // The prepared value is intentionally not retained after serialization.
        drop(prepared);
        for (item_id, bytes) in objects {
            self.mailbox.insert(
                item_id.clone(),
                ProviderItem {
                    batch_id: Some(batch_id),
                    kind: ProviderItemKind::Object,
                    bytes: Arc::from(bytes.clone()),
                },
            )?;
            self.ingress_bytes(
                event_id,
                device,
                &item_id,
                ProviderItemKind::Object,
                bytes,
                &ByteMutation::Exact,
                None,
            )?;
        }
        self.mailbox.insert(
            manifest_id.clone(),
            ProviderItem {
                batch_id: Some(batch_id),
                kind: ProviderItemKind::Manifest,
                bytes: Arc::from(manifest.clone()),
            },
        )?;
        self.ingress_bytes(
            event_id,
            device,
            &manifest_id,
            ProviderItemKind::Manifest,
            manifest,
            &ByteMutation::Exact,
            None,
        )?;
        let outcome = self
            .device_mut(device)?
            .engine_mut()?
            .stage_archive_batch(batch_id)
            .map_err(|error| ScenarioError::Engine(error.to_string()))?;
        self.outcomes.push(outcome);
        Ok(())
    }

    fn ingress_item(
        &mut self,
        event_id: u64,
        device: &str,
        item_id: &str,
        mutation: &ByteMutation,
        expected: Option<&IngressExpectation>,
    ) -> Result<(), ScenarioError> {
        let item = self.mailbox.item(item_id)?.clone();
        self.ingress_bytes(
            event_id,
            device,
            item_id,
            item.kind,
            item.bytes.to_vec(),
            mutation,
            expected,
        )
    }

    fn ingress_bytes(
        &mut self,
        event_id: u64,
        device: &str,
        item_id: &str,
        kind: ProviderItemKind,
        bytes: Vec<u8>,
        mutation: &ByteMutation,
        expected: Option<&IngressExpectation>,
    ) -> Result<(), ScenarioError> {
        let bytes = self.mutate_bytes(bytes, mutation)?;
        let result = match kind {
            ProviderItemKind::Object => self
                .device(device)?
                .store()?
                .stage_object_bytes(&bytes)
                .map(|_| ()),
            ProviderItemKind::Manifest => self
                .device(device)?
                .store()?
                .stage_manifest_bytes(&bytes)
                .map(|_| ()),
        };
        let receipt = IngressReceipt {
            event_id,
            device: device.into(),
            item_id: item_id.into(),
            item_kind: kind,
            byte_len: bytes.len(),
            accepted: result.is_ok(),
            error: result.as_ref().err().map(ToString::to_string),
        };
        if let Some(expected) = expected {
            let matches = match expected {
                IngressExpectation::Accepted => receipt.accepted,
                IngressExpectation::Rejected { error } => receipt.error.as_deref() == Some(error),
            };
            if !matches {
                return Err(self.invariant(
                    event_id,
                    InvariantPredicate::IngressOutcome,
                    vec![device.into(), item_id.into()],
                    Vec::new(),
                    "ingress receipt differed from expectation",
                ));
            }
        }
        self.receipts.insert(event_id, receipt);
        Ok(())
    }

    fn mutate_bytes(
        &self,
        mut bytes: Vec<u8>,
        mutation: &ByteMutation,
    ) -> Result<Vec<u8>, ScenarioError> {
        match mutation {
            ByteMutation::Exact => {}
            ByteMutation::Truncate { len } => bytes.truncate(*len),
            ByteMutation::XorByte { offset, mask } => {
                let byte = bytes.get_mut(*offset).ok_or_else(|| {
                    ScenarioError::InvalidMutation("xor offset outside item".into())
                })?;
                *byte ^= *mask;
            }
            ByteMutation::Insert { offset, bytes_b64 } => {
                if *offset > bytes.len() {
                    return Err(ScenarioError::InvalidMutation(
                        "insert offset outside item".into(),
                    ));
                }
                bytes.splice(*offset..*offset, bytes_b64.0.iter().copied());
            }
            ByteMutation::ReplaceRange {
                start,
                end,
                bytes_b64,
            } => {
                if start > end || *end > bytes.len() {
                    return Err(ScenarioError::InvalidMutation(
                        "replacement range outside item".into(),
                    ));
                }
                bytes.splice(*start..*end, bytes_b64.0.iter().copied());
            }
            ByteMutation::Substitute { item_id } => {
                bytes = self.mailbox.item(item_id)?.bytes.to_vec()
            }
        }
        Ok(bytes)
    }

    fn assert_invariant(
        &self,
        event_id: u64,
        assertion: &InvariantAssertion,
    ) -> Result<(), ScenarioError> {
        match assertion {
            InvariantAssertion::Replica { device, expected } => {
                self.assert_replica(event_id, device, expected)
            }
            InvariantAssertion::Converged { devices } => self.assert_converged(event_id, devices),
            InvariantAssertion::NoVisibleEffect { device, snapshot } => {
                let observed = self
                    .device(device)?
                    .engine()?
                    .canonical_snapshot()
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?;
                if &observed != snapshot {
                    return Err(self.invariant(
                        event_id,
                        InvariantPredicate::NoVisibleEffect,
                        vec![device.clone()],
                        Vec::new(),
                        "visible snapshot changed",
                    ));
                }
                Ok(())
            }
            InvariantAssertion::Ingress {
                event_id: receipt_event,
                expected,
            } => {
                let receipt = self
                    .receipts
                    .get(receipt_event)
                    .ok_or_else(|| ScenarioError::MissingReceipt(*receipt_event))?;
                let matches = match expected {
                    IngressExpectation::Accepted => receipt.accepted,
                    IngressExpectation::Rejected { error } => {
                        receipt.error.as_deref() == Some(error)
                    }
                };
                if matches {
                    Ok(())
                } else {
                    Err(self.invariant(
                        event_id,
                        InvariantPredicate::IngressOutcome,
                        vec![receipt.device.clone(), receipt.item_id.clone()],
                        Vec::new(),
                        "recorded ingress receipt differed from expectation",
                    ))
                }
            }
            InvariantAssertion::LineageIsolation { device, accepted } => {
                let found = self
                    .device(device)?
                    .engine()?
                    .status()
                    .accepted_batch_ids()
                    .map_err(|error| ScenarioError::Engine(error.to_string()))?;
                if &found != accepted {
                    return Err(self.invariant(
                        event_id,
                        InvariantPredicate::LineageIsolation,
                        vec![device.clone()],
                        accepted.iter().map(ToString::to_string).collect(),
                        "foreign lineage changed accepted frontier",
                    ));
                }
                Ok(())
            }
            InvariantAssertion::RestartReplay { device } => {
                self.assert_restart_replay(event_id, device)
            }
            InvariantAssertion::UntouchedExternalFiles => self.assert_external_files(event_id),
        }
    }

    fn assert_replica(
        &self,
        event_id: u64,
        device: &str,
        expected: &ReplicaExpectation,
    ) -> Result<(), ScenarioError> {
        let observation = self.observation(device, event_id)?;
        let actual_state = match observation.state {
            SimulatorDeviceState::Operational(snapshot) => {
                (ExpectedWorkspaceState::Operational, Some(snapshot))
            }
            SimulatorDeviceState::Blocked(evidence) => {
                (ExpectedWorkspaceState::Blocked { evidence }, None)
            }
        };
        if observation.accepted != expected.accepted
            || observation.offered != expected.offered
            || actual_state.0 != expected.state
            || expected
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| actual_state.1.as_ref() != Some(snapshot))
        {
            return Err(self.invariant(
                event_id,
                InvariantPredicate::ExpectedReplica,
                vec![device.into()],
                expected.accepted.iter().map(ToString::to_string).collect(),
                "replica expectation differed",
            ));
        }
        Ok(())
    }

    fn assert_converged(&self, event_id: u64, devices: &[String]) -> Result<(), ScenarioError> {
        let mut observations = devices
            .iter()
            .map(|device| self.observation(device, event_id))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(first) = observations.pop() else {
            return Ok(());
        };
        for other in observations {
            if other != first {
                return Err(self.invariant(
                    event_id,
                    InvariantPredicate::SameAcceptedClosureSnapshot,
                    sorted_subject(devices),
                    first.accepted.iter().map(ToString::to_string).collect(),
                    "replicas did not converge",
                ));
            }
        }
        Ok(())
    }

    fn assert_restart_replay(&self, event_id: u64, device: &str) -> Result<(), ScenarioError> {
        let observed = self.observation(device, event_id)?;
        let runtime = self.device(device)?;
        let store = ObjectStore::open(
            &runtime.archive_path(),
            self.scenario.workspace.workspace_id,
        )
        .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let manifests = store
            .committed_manifests()
            .map_err(|error| ScenarioError::Store(error.to_string()))?;
        let mut replay = ShardedHotEngine::with_archive_store(
            store,
            self.scenario.workspace.lineage_digest,
            self.scenario.workspace.catalog_document_id,
        );
        for manifest in manifests {
            replay
                .stage_archive_batch(manifest.batch_id())
                .map_err(|error| ScenarioError::Engine(error.to_string()))?;
        }
        let replayed = observation_from_engine(&replay, event_id)?;
        if observed != replayed {
            return Err(self.invariant(
                event_id,
                InvariantPredicate::RestartReplay,
                vec![device.into()],
                observed.accepted.iter().map(ToString::to_string).collect(),
                "restart differed from clean archive replay",
            ));
        }
        Ok(())
    }

    fn assert_external_files(&self, event_id: u64) -> Result<(), ScenarioError> {
        for file in &self.scenario.external_files {
            let bytes = fs::read(self.root.0.join(&file.path))
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            if bytes != file.bytes_b64.0 {
                return Err(self.invariant(
                    event_id,
                    InvariantPredicate::NoVisibleEffect,
                    vec![file.path.clone()],
                    Vec::new(),
                    "external fixture changed",
                ));
            }
        }
        Ok(())
    }

    fn check_global_oracle(&self, event_id: u64) -> Result<(), ScenarioError> {
        self.assert_external_files(event_id)?;
        let mut snapshots = BTreeMap::<Vec<BatchId>, (CanonicalSnapshot, String)>::new();
        let mut offered = BTreeMap::<Vec<BatchId>, (OfferedState, String)>::new();
        for (name, runtime) in &self.devices {
            if runtime.engine.is_none() {
                continue;
            }
            let observation = self.observation(name, event_id)?;
            for batch_id in &observation.accepted {
                if !matches!(
                    runtime
                        .store()?
                        .inspect_batch(*batch_id)
                        .map_err(|error| ScenarioError::Store(error.to_string()))?,
                    BatchInspection::Ready(_)
                ) {
                    return Err(self.invariant(
                        event_id,
                        InvariantPredicate::IngressOutcome,
                        vec![name.clone()],
                        vec![batch_id.to_string()],
                        "accepted batch is not locally ready",
                    ));
                }
            }
            match &observation.state {
                SimulatorDeviceState::Operational(snapshot) => {
                    if let Some((expected, other)) = snapshots.get(&observation.accepted) {
                        if expected != snapshot {
                            return Err(self.invariant(
                                event_id,
                                InvariantPredicate::SameAcceptedClosureSnapshot,
                                sorted_subject(&[name.clone(), other.clone()]),
                                observation
                                    .accepted
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect(),
                                "equal accepted closures had different snapshots",
                            ));
                        }
                    } else {
                        snapshots.insert(
                            observation.accepted.clone(),
                            (snapshot.clone(), name.clone()),
                        );
                    }
                    let state = OfferedState::Operational(observation.accepted.clone());
                    record_offered(
                        &mut offered,
                        observation.offered,
                        state,
                        name,
                        event_id,
                        self,
                    )?;
                }
                SimulatorDeviceState::Blocked(evidence) => {
                    record_offered(
                        &mut offered,
                        observation.offered,
                        OfferedState::Blocked(evidence.clone()),
                        name,
                        event_id,
                        self,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn observation(&self, device: &str, event_id: u64) -> Result<DeviceObservation, ScenarioError> {
        observation_from_engine(self.device(device)?.engine()?, event_id)
    }

    fn device(&self, name: &str) -> Result<&DeviceRuntime, ScenarioError> {
        self.devices
            .get(name)
            .ok_or_else(|| ScenarioError::UnknownDeviceName(name.into()))
    }

    fn device_mut(&mut self, name: &str) -> Result<&mut DeviceRuntime, ScenarioError> {
        self.devices
            .get_mut(name)
            .ok_or_else(|| ScenarioError::UnknownDeviceName(name.into()))
    }

    fn identity(&self, name: &str) -> Result<&ScenarioDevice, ScenarioError> {
        self.scenario
            .devices
            .iter()
            .find(|device| device.name == name)
            .ok_or_else(|| ScenarioError::UnknownDeviceName(name.into()))
    }

    fn invariant(
        &self,
        event_id: u64,
        predicate: InvariantPredicate,
        subject: Vec<String>,
        required_closure_or_lineage: Vec<String>,
        message: &str,
    ) -> ScenarioError {
        ScenarioError::Invariant {
            signature: InvariantSignature {
                assertion_or_event_id: event_id,
                predicate,
                subject: sorted_subject(&subject),
                required_closure_or_lineage,
            },
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeviceObservation {
    accepted: Vec<BatchId>,
    offered: Vec<BatchId>,
    state: SimulatorDeviceState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OfferedState {
    Operational(Vec<BatchId>),
    Blocked(ImmutableHomeEvidence),
}

fn observation_from_engine(
    engine: &ShardedHotEngine,
    event_id: u64,
) -> Result<DeviceObservation, ScenarioError> {
    let status = engine.status();
    let accepted = status
        .accepted_batch_ids()
        .map_err(|error| ScenarioError::Action {
            action_index: event_id as usize,
            error: error.to_string(),
        })?;
    let offered = status
        .offered_batch_ids()
        .map_err(|error| ScenarioError::Action {
            action_index: event_id as usize,
            error: error.to_string(),
        })?
        .to_vec();
    let state = match status.workspace().clone() {
        WorkspaceStatus::Operational => SimulatorDeviceState::Operational(
            engine
                .canonical_snapshot()
                .map_err(|error| ScenarioError::Engine(error.to_string()))?,
        ),
        WorkspaceStatus::Blocked(_) => {
            SimulatorDeviceState::Blocked(collect_fatal_evidence(engine)?)
        }
    };
    Ok(DeviceObservation {
        accepted,
        offered,
        state,
    })
}

fn collect_fatal_evidence(
    engine: &ShardedHotEngine,
) -> Result<ImmutableHomeEvidence, ScenarioError> {
    if let Some(evidence) = engine.fatal_evidence() {
        return Ok(evidence.clone());
    }
    let mut cursor = None;
    let mut conflicts = Vec::new();
    loop {
        let page = engine
            .fatal_evidence_page(cursor, 32)
            .map_err(|error| ScenarioError::Engine(error.to_string()))?
            .ok_or_else(|| {
                ScenarioError::Engine("fatal evidence handle could not be streamed".into())
            })?;
        conflicts.extend_from_slice(page.conflicts());
        cursor = page.next();
        if cursor.is_none() {
            return Ok(ImmutableHomeEvidence::new(conflicts));
        }
    }
}

fn device_state(
    runtime: &DeviceRuntime,
    event_id: u64,
) -> Result<SimulatorDeviceState, ScenarioError> {
    Ok(observation_from_engine(runtime.engine()?, event_id)?.state)
}

fn record_offered(
    offered: &mut BTreeMap<Vec<BatchId>, (OfferedState, String)>,
    frontier: Vec<BatchId>,
    state: OfferedState,
    device: &str,
    event_id: u64,
    simulator: &DeterministicSimulator,
) -> Result<(), ScenarioError> {
    if let Some((expected, other)) = offered.get(&frontier) {
        if expected != &state {
            return Err(simulator.invariant(
                event_id,
                InvariantPredicate::SameOfferedClosureStatus,
                sorted_subject(&[device.into(), other.clone()]),
                frontier.iter().map(ToString::to_string).collect(),
                "equal offered closures had different status or evidence",
            ));
        }
    } else {
        offered.insert(frontier, (state, device.into()));
    }
    Ok(())
}

fn validate_scheduled_action(
    action: &ScheduledActionKind,
    names: &BTreeSet<String>,
) -> Result<(), ScenarioError> {
    let known = |name: &str| names.contains(name);
    match action {
        ScheduledActionKind::AuthorLocal { device, .. }
        | ScheduledActionKind::DeliverItem { device, .. }
        | ScheduledActionKind::BeginTransfer { device, .. }
        | ScheduledActionKind::AppendTransfer { device, .. }
        | ScheduledActionKind::CommitTransfer { device, .. }
        | ScheduledActionKind::AbortTransfer { device, .. }
        | ScheduledActionKind::ProbeBatch { device, .. }
        | ScheduledActionKind::Crash { device }
        | ScheduledActionKind::Restart { device }
            if !known(device) =>
        {
            return Err(ScenarioError::UnknownDeviceName(device.clone()))
        }
        ScheduledActionKind::AssertInvariant { assertion } => match assertion {
            InvariantAssertion::Replica { device, .. }
            | InvariantAssertion::NoVisibleEffect { device, .. }
            | InvariantAssertion::LineageIsolation { device, .. }
            | InvariantAssertion::RestartReplay { device }
                if !known(device) =>
            {
                return Err(ScenarioError::UnknownDeviceName(device.clone()))
            }
            InvariantAssertion::Converged { devices }
                if devices.is_empty() || devices.iter().any(|device| !known(device)) =>
            {
                return Err(ScenarioError::InvalidInvariant)
            }
            _ => {}
        },
        ScheduledActionKind::LegacyDeliver { device, .. } if !known(device) => {
            return Err(ScenarioError::UnknownDeviceName(device.clone()))
        }
        ScheduledActionKind::LegacyAssertConverged { devices }
            if devices.is_empty() || devices.iter().any(|device| !known(device)) =>
        {
            return Err(ScenarioError::InvalidInvariant)
        }
        _ => {}
    }
    Ok(())
}

fn validate_replica_expectation(
    replica: &InitialReplica,
    names: &BTreeSet<String>,
    item_ids: &BTreeSet<String>,
) -> Result<(), ScenarioError> {
    if !names.contains(&replica.device)
        || replica
            .stored_items
            .iter()
            .any(|item| !item_ids.contains(item))
    {
        return Err(ScenarioError::InvalidReplica);
    }
    if !strictly_sorted(&replica.expected.accepted) || !strictly_sorted(&replica.expected.offered) {
        return Err(ScenarioError::InvalidReplica);
    }
    Ok(())
}

fn repair_trace(actions: &mut Vec<ScheduledAction>) {
    let authored: BTreeSet<_> = actions
        .iter()
        .filter_map(|action| match &action.action {
            ScheduledActionKind::AuthorLocal { batch_id, .. } => Some(*batch_id),
            _ => None,
        })
        .collect();
    actions.retain(|action| match &action.action {
        ScheduledActionKind::DeliverItem { item_id, .. }
        | ScheduledActionKind::BeginTransfer { item_id, .. } => {
            !dynamic_item_batch(item_id).is_some_and(|batch| !authored.contains(&batch))
        }
        ScheduledActionKind::ProbeBatch { batch_id, .. } => {
            authored.contains(batch_id) || !dynamic_batch_id(*batch_id)
        }
        _ => true,
    });
    let transfers: BTreeSet<_> = actions
        .iter()
        .filter_map(|action| match &action.action {
            ScheduledActionKind::BeginTransfer {
                device,
                transfer_id,
                ..
            } => Some((device.clone(), transfer_id.clone())),
            _ => None,
        })
        .collect();
    actions.retain(|action| match &action.action {
        ScheduledActionKind::AppendTransfer {
            device,
            transfer_id,
            ..
        }
        | ScheduledActionKind::CommitTransfer {
            device,
            transfer_id,
            ..
        }
        | ScheduledActionKind::AbortTransfer {
            device,
            transfer_id,
        } => transfers.contains(&(device.clone(), transfer_id.clone())),
        _ => true,
    });
    let mut crashed = BTreeSet::new();
    actions.retain(|action| match &action.action {
        ScheduledActionKind::Crash { device } => {
            crashed.insert(device.clone());
            true
        }
        ScheduledActionKind::Restart { device } => crashed.contains(device),
        _ => true,
    });
}

fn shrink_action_candidates(action: &ScheduledAction) -> Vec<ScheduledAction> {
    let mut candidates = Vec::new();
    match &action.action {
        ScheduledActionKind::AuthorLocal { transaction, .. } => {
            for index in 0..transaction.operations.len() {
                if transaction.operations.len() == 1 {
                    break;
                }
                let mut candidate = action.clone();
                let ScheduledActionKind::AuthorLocal { transaction, .. } = &mut candidate.action
                else {
                    unreachable!("author action stayed an author action")
                };
                transaction.operations.remove(index);
                candidates.push(candidate);
            }
            for index in 0..transaction.operations.len() {
                let Some(operation) = shrink_operation_content(&transaction.operations[index])
                else {
                    continue;
                };
                let mut candidate = action.clone();
                let ScheduledActionKind::AuthorLocal { transaction, .. } = &mut candidate.action
                else {
                    unreachable!("author action stayed an author action")
                };
                transaction.operations[index] = operation;
                candidates.push(candidate);
            }
        }
        ScheduledActionKind::DeliverItem { mutation, .. }
        | ScheduledActionKind::CommitTransfer { mutation, .. } => {
            for mutation in shrink_mutation_candidates(mutation) {
                let mut candidate = action.clone();
                match &mut candidate.action {
                    ScheduledActionKind::DeliverItem {
                        mutation: found, ..
                    }
                    | ScheduledActionKind::CommitTransfer {
                        mutation: found, ..
                    } => {
                        *found = mutation;
                    }
                    _ => unreachable!("delivery action changed kind"),
                }
                candidates.push(candidate);
            }
        }
        ScheduledActionKind::AppendTransfer { len, .. } if *len > 0 => {
            let mut candidate = action.clone();
            let ScheduledActionKind::AppendTransfer { len, .. } = &mut candidate.action else {
                unreachable!("append action changed kind")
            };
            *len /= 2;
            candidates.push(candidate);
        }
        _ => {}
    }
    candidates
}

fn shrink_operation_content(operation: &SemanticOperation) -> Option<SemanticOperation> {
    match operation {
        SemanticOperation::CreateBlock { content, .. } if !content.is_empty() => {
            let mut operation = operation.clone();
            let SemanticOperation::CreateBlock { content, .. } = &mut operation else {
                unreachable!("cloned create block changed kind")
            };
            content.truncate(content.len() / 2);
            Some(operation)
        }
        SemanticOperation::EditBlockContent { content, .. } if !content.is_empty() => {
            let mut operation = operation.clone();
            let SemanticOperation::EditBlockContent { content, .. } = &mut operation else {
                unreachable!("cloned content edit changed kind")
            };
            content.truncate(content.len() / 2);
            Some(operation)
        }
        SemanticOperation::RenamePageAndRewriteReferrers { referrers, .. }
            if !referrers.is_empty() =>
        {
            let mut operation = operation.clone();
            let SemanticOperation::RenamePageAndRewriteReferrers { referrers, .. } = &mut operation
            else {
                unreachable!("cloned rename changed kind")
            };
            referrers.pop();
            Some(operation)
        }
        _ => None,
    }
}

fn shrink_mutation_candidates(mutation: &ByteMutation) -> Vec<ByteMutation> {
    match mutation {
        ByteMutation::Exact | ByteMutation::Substitute { .. } => Vec::new(),
        ByteMutation::Truncate { len } if *len > 0 => vec![ByteMutation::Truncate { len: len / 2 }],
        ByteMutation::XorByte { offset, mask } => {
            let mut candidates = Vec::new();
            if *offset > 0 {
                candidates.push(ByteMutation::XorByte {
                    offset: 0,
                    mask: *mask,
                });
            }
            if *mask != 1 {
                candidates.push(ByteMutation::XorByte {
                    offset: *offset,
                    mask: 1,
                });
            }
            candidates
        }
        ByteMutation::Insert { offset, bytes_b64 } if !bytes_b64.0.is_empty() => {
            vec![ByteMutation::Insert {
                offset: *offset,
                bytes_b64: WireBytes(bytes_b64.0[..bytes_b64.0.len() / 2].to_vec()),
            }]
        }
        ByteMutation::ReplaceRange {
            start,
            end,
            bytes_b64,
        } => {
            let mut candidates = Vec::new();
            if start < end {
                candidates.push(ByteMutation::ReplaceRange {
                    start: *start,
                    end: start + (end - start) / 2,
                    bytes_b64: bytes_b64.clone(),
                });
            }
            if !bytes_b64.0.is_empty() {
                candidates.push(ByteMutation::ReplaceRange {
                    start: *start,
                    end: *end,
                    bytes_b64: WireBytes(bytes_b64.0[..bytes_b64.0.len() / 2].to_vec()),
                });
            }
            candidates
        }
        _ => Vec::new(),
    }
}

fn dynamic_item_batch(item_id: &str) -> Option<BatchId> {
    let rest = item_id.strip_prefix("auth/")?;
    let (batch, _) = rest.split_once('/')?;
    batch.parse().ok()
}

fn dynamic_batch_id(_batch: BatchId) -> bool {
    false
}

struct ReplayFailure {
    error: ScenarioError,
    ingress_receipt: Option<IngressReceipt>,
    accepted_witness: BTreeMap<String, Vec<BatchId>>,
    offered_witness: BTreeMap<String, Vec<BatchId>>,
    status_witness: BTreeMap<String, String>,
    expected_snapshot_hash: Option<String>,
    observed_snapshot_hash: Option<String>,
    first_canonical_difference: Option<String>,
}

fn replay_failure_details(scenario: &Scenario) -> Result<Option<ReplayFailure>, ScenarioError> {
    let mut simulator = DeterministicSimulator::new(scenario.clone())?;
    let Err(error) = simulator.run() else {
        return Ok(None);
    };
    let mut accepted_witness = BTreeMap::new();
    let mut offered_witness = BTreeMap::new();
    let mut status_witness = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    for (name, runtime) in &simulator.devices {
        let Some(engine) = runtime.engine.as_ref() else {
            status_witness.insert(name.clone(), "crashed".into());
            continue;
        };
        let observation = observation_from_engine(engine, 0)?;
        accepted_witness.insert(name.clone(), observation.accepted.clone());
        offered_witness.insert(name.clone(), observation.offered.clone());
        match observation.state {
            SimulatorDeviceState::Operational(snapshot) => {
                status_witness.insert(name.clone(), "operational".into());
                snapshots.insert(name.clone(), snapshot);
            }
            SimulatorDeviceState::Blocked(evidence) => {
                status_witness.insert(name.clone(), format!("blocked:{evidence}"));
            }
        }
    }
    let failure_event = first_failure_event(&error);
    let ingress_receipt = simulator.receipts.get(&failure_event).cloned();
    let (expected_snapshot_hash, observed_snapshot_hash, first_canonical_difference) =
        snapshot_failure_witness(scenario, &error, &snapshots);
    Ok(Some(ReplayFailure {
        error,
        ingress_receipt,
        accepted_witness,
        offered_witness,
        status_witness,
        expected_snapshot_hash,
        observed_snapshot_hash,
        first_canonical_difference,
    }))
}

fn replay_failure(scenario: &Scenario) -> Result<Option<ScenarioError>, ScenarioError> {
    Ok(replay_failure_details(scenario)?.map(|failure| failure.error))
}

fn snapshot_failure_witness(
    scenario: &Scenario,
    error: &ScenarioError,
    snapshots: &BTreeMap<String, CanonicalSnapshot>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(signature) = error
        .failure_identity()
        .and_then(|identity| match identity {
            FailureIdentity::Invariant(signature) => Some(signature),
            _ => None,
        })
    else {
        return (None, None, None);
    };
    let expected = scenario.actions.iter().find_map(|scheduled| {
        if scheduled.event_id != signature.assertion_or_event_id {
            return None;
        }
        match &scheduled.action {
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect { snapshot, .. },
            } => Some(snapshot.clone()),
            _ => None,
        }
    });
    let observed = signature
        .subject
        .iter()
        .find_map(|device| snapshots.get(device).cloned())
        .or_else(|| snapshots.values().next().cloned());
    let expected_hash = expected.as_ref().map(snapshot_hash);
    let observed_hash = observed.as_ref().map(snapshot_hash);
    let difference = match (expected.as_ref(), observed.as_ref()) {
        (Some(expected), Some(observed)) if expected != observed => {
            Some(first_snapshot_difference(expected, observed))
        }
        _ => None,
    };
    (expected_hash, observed_hash, difference)
}

fn snapshot_hash(snapshot: &CanonicalSnapshot) -> String {
    let bytes = serde_json::to_vec(snapshot).expect("canonical snapshots are serializable");
    format!("{:x}", Sha256::digest(bytes))
}

fn first_snapshot_difference(expected: &CanonicalSnapshot, observed: &CanonicalSnapshot) -> String {
    let expected = serde_json::to_vec(expected).expect("canonical snapshots are serializable");
    let observed = serde_json::to_vec(observed).expect("canonical snapshots are serializable");
    let offset = expected
        .iter()
        .zip(&observed)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(observed.len()));
    format!("canonical JSON byte {offset}")
}

fn reproduces(candidate: &Scenario, identity: &FailureIdentity) -> Result<bool, ScenarioError> {
    Ok(
        replay_failure(candidate)?.and_then(|error| error.failure_identity())
            == Some(identity.clone()),
    )
}

fn first_failure_event(error: &ScenarioError) -> u64 {
    match error {
        ScenarioError::Invariant { signature, .. } => signature.assertion_or_event_id,
        ScenarioError::Action { action_index, .. }
        | ScenarioError::Diverged { action_index }
        | ScenarioError::GlobalOracleDiverged { action_index } => *action_index as u64,
        _ => 0,
    }
}

fn scenario_hash(scenario: &Scenario) -> Result<String, ScenarioError> {
    let bytes = scenario.encode()?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn authored_item_id(batch_id: BatchId, kind: &str, index: usize) -> String {
    format!("auth/{batch_id}/{kind}/{index}")
}

fn stage_matches(expected: &StageExpectation, found: &BatchDisposition) -> bool {
    matches!(
        (expected, found),
        (
            StageExpectation::Accepted,
            BatchDisposition::Accepted { .. } | BatchDisposition::DuplicateAccepted { .. }
        ) | (
            StageExpectation::Incomplete,
            BatchDisposition::IncompleteStaged { .. }
        ) | (
            StageExpectation::Rejected,
            BatchDisposition::Rejected { .. }
        ) | (StageExpectation::Quarantined, BatchDisposition::Quarantined)
    )
}

fn valid_name(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).is_relative()
        && !Path::new(path)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn sorted_subject(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    values.dedup();
    values
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(value & 0x3f) as usize] as char);
        }
    }
    encoded
}

fn base64url_decode(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 4 == 1 {
        return Err("invalid base64url length".into());
    }
    let decode = |byte: u8| -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    };
    let input = value.as_bytes();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let a = decode(chunk[0]).ok_or_else(|| "invalid base64url character".to_string())?;
        let b = decode(
            *chunk
                .get(1)
                .ok_or_else(|| "invalid base64url length".to_string())?,
        )
        .ok_or_else(|| "invalid base64url character".to_string())?;
        let c = chunk
            .get(2)
            .map(|byte| decode(*byte).ok_or_else(|| "invalid base64url character".to_string()))
            .transpose()?;
        let d = chunk
            .get(3)
            .map(|byte| decode(*byte).ok_or_else(|| "invalid base64url character".to_string()))
            .transpose()?;
        output.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            output.push((b << 4) | (c >> 2));
            if let Some(d) = d {
                output.push((c << 6) | d);
            }
        }
    }
    Ok(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioError {
    Decode(String),
    Encode(String),
    UnknownVersion(u32),
    TooLarge(usize),
    TooManyActions(usize),
    WireTooLarge(usize),
    InvalidFamily,
    InvalidDevice,
    InvalidDeviceCount(usize),
    UnknownDevice(usize),
    UnknownDeviceName(String),
    DeviceCrashed(String),
    AlreadyRunning(String),
    UnknownBatch(BatchId),
    DuplicateBatch(BatchId),
    UnknownItem(String),
    ProviderItemDropped(String),
    UnknownTransfer(String),
    InvalidTransfer(String),
    InvalidMutation(String),
    InvalidWireBatch,
    InvalidWireItem(String),
    InvalidReplica,
    InvalidExternalFile(String),
    InvalidInvariant,
    InvalidSchedule,
    MissingReceipt(u64),
    Io(String),
    Store(String),
    Engine(String),
    NonCanonical,
    Action {
        action_index: usize,
        error: String,
    },
    Invariant {
        signature: InvariantSignature,
        message: String,
    },
    // Compatibility failures retained for the prior public API.
    Diverged {
        action_index: usize,
    },
    GlobalOracleDiverged {
        action_index: usize,
    },
    NotFailing,
    UnstableFailure,
    InvalidFailureCapsule,
}

impl ScenarioError {
    pub fn failure_identity(&self) -> Option<FailureIdentity> {
        match self {
            Self::Invariant { signature, .. } => {
                Some(FailureIdentity::Invariant(signature.clone()))
            }
            Self::Action { error, .. } => Some(FailureIdentity::Action(error.clone())),
            Self::Diverged { .. } => Some(FailureIdentity::Diverged),
            Self::GlobalOracleDiverged { .. } => Some(FailureIdentity::GlobalOracleDiverged),
            _ => None,
        }
    }
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "scenario decode failed: {error}"),
            Self::Encode(error) => write!(f, "scenario encode failed: {error}"),
            Self::UnknownVersion(found) => write!(
                f,
                "unknown scenario schema {found}; expected {SCENARIO_SCHEMA_VERSION}"
            ),
            Self::TooLarge(bytes) => write!(f, "scenario is too large: {bytes} bytes"),
            Self::TooManyActions(actions) => write!(f, "scenario has too many actions: {actions}"),
            Self::WireTooLarge(bytes) => {
                write!(f, "scenario wire corpus is too large: {bytes} bytes")
            }
            Self::InvalidFamily => f.write_str("scenario family is invalid"),
            Self::InvalidDevice => f.write_str("scenario device is invalid"),
            Self::InvalidDeviceCount(count) => write!(f, "invalid scenario device count {count}"),
            Self::UnknownDevice(device) => write!(f, "unknown scenario device index {device}"),
            Self::UnknownDeviceName(device) => write!(f, "unknown scenario device {device}"),
            Self::DeviceCrashed(device) => write!(f, "scenario device {device} is crashed"),
            Self::AlreadyRunning(device) => {
                write!(f, "scenario device {device} is already running")
            }
            Self::UnknownBatch(batch) => write!(f, "unknown scenario batch {batch}"),
            Self::DuplicateBatch(batch) => write!(f, "duplicate scenario batch {batch}"),
            Self::UnknownItem(item) => write!(f, "unknown provider item {item}"),
            Self::ProviderItemDropped(item) => write!(f, "provider item {item} was dropped"),
            Self::UnknownTransfer(transfer) => write!(f, "unknown transfer {transfer}"),
            Self::InvalidTransfer(transfer) => write!(f, "invalid transfer {transfer}"),
            Self::InvalidMutation(error) => write!(f, "invalid byte mutation: {error}"),
            Self::InvalidWireBatch => f.write_str("wire batch is invalid"),
            Self::InvalidWireItem(item) => write!(f, "wire item is invalid: {item}"),
            Self::InvalidReplica => f.write_str("replica expectation is invalid"),
            Self::InvalidExternalFile(path) => {
                write!(f, "external fixture path is invalid: {path}")
            }
            Self::InvalidInvariant => f.write_str("invariant assertion is invalid"),
            Self::InvalidSchedule => f.write_str("schedule is invalid"),
            Self::MissingReceipt(event) => write!(f, "missing ingress receipt for event {event}"),
            Self::Io(error) => write!(f, "scenario filesystem operation failed: {error}"),
            Self::Store(error) => write!(f, "scenario object-store operation failed: {error}"),
            Self::Engine(error) => write!(f, "scenario engine operation failed: {error}"),
            Self::NonCanonical => f.write_str("scenario bytes are not canonical"),
            Self::Action {
                action_index,
                error,
            } => write!(f, "scenario action {action_index} failed: {error}"),
            Self::Invariant { signature, message } => write!(
                f,
                "scenario invariant {:?} at event {} failed: {message}",
                signature.predicate, signature.assertion_or_event_id
            ),
            Self::Diverged { action_index } => write!(
                f,
                "scenario convergence assertion failed at action {action_index}"
            ),
            Self::GlobalOracleDiverged { action_index } => {
                write!(f, "scenario global oracle failed at action {action_index}")
            }
            Self::NotFailing => f.write_str("scenario does not fail and cannot be minimized"),
            Self::UnstableFailure => {
                f.write_str("scenario failure has no stable minimization identity")
            }
            Self::InvalidFailureCapsule => f.write_str("failure capsule is invalid"),
        }
    }
}

impl std::error::Error for ScenarioError {}
