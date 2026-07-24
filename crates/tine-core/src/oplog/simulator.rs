//! Deterministic, store-backed adversarial replay for the oplog boundary.
//!
//! Every replica owns an isolated archive and provider inbox/outbox tree.
//! Provider movement and receiver rescans are explicit trace actions; only
//! bytes staged through `ObjectStore` cross replicas.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::object_store::{ensure_directory_nofollow, open_dir_nofollow, sync_dir_required};
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
/// Provider trees are deliberately small: rescan is a trace operation, not a
/// background watcher, and must never turn an adversarial trace into an
/// unbounded walk of a host filesystem.
pub const MAX_PROVIDER_RESCAN_ENTRIES: usize = 4_096;
pub const MAX_PROVIDER_RESCAN_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PROVIDER_RESCAN_DEPTH: usize = 16;
pub const MAX_PROVIDER_RESIDUE_ENTRIES: usize = 512;
pub const MAX_PROVIDER_PATH_BYTES: usize = 512;
pub const MAX_PROVIDER_JOURNAL_PENDING: usize = 4;
pub const MAX_PROVIDER_JOURNAL_COMPLETED: usize = MAX_SCENARIO_ACTIONS;
pub const MAX_PROVIDER_JOURNAL_BLOB_BYTES: usize = MAX_PROVIDER_RESCAN_BYTES;
pub const MAX_PROVIDER_JOURNAL_RECORD_BYTES: usize = 4 * 1024;
pub const MAX_PROVIDER_JOURNAL_COMPLETION_BYTES: usize =
    MAX_PROVIDER_JOURNAL_COMPLETED * MAX_PROVIDER_JOURNAL_RECORD_BYTES;
pub const MAX_PROVIDER_JOURNAL_FILES: usize = MAX_PROVIDER_JOURNAL_PENDING * 2 + 1;
pub const MAX_PROVIDER_JOURNAL_BYTES: usize =
    MAX_PROVIDER_JOURNAL_BLOB_BYTES
        + (MAX_PROVIDER_JOURNAL_PENDING + 1) * MAX_PROVIDER_JOURNAL_RECORD_BYTES;
const PROVIDER_JOURNAL_SCHEMA_VERSION: u32 = 1;
const PROVIDER_AUTHORITY_SCHEMA_VERSION: u32 = 1;
const PROVIDER_DEVICE_AUTHORITY_NAME: &str = "provider-transaction.authority";
const MAX_PROVIDER_AUTHORITY_BYTES: usize = 1024;
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

/// The only two roots exposed by a simulated filesystem provider.  Their
/// names are fixed by the harness; scenario paths are always relative to one
/// of these roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTree {
    Inbox,
    Outbox,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderLocation {
    pub device: String,
    pub tree: ProviderTree,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSource {
    Mailbox { item_id: String },
    Tree { location: ProviderLocation },
}

/// Deterministic diagnostic state for a provider tree.  It is intentionally
/// path and digest based, so a ddmin replay can preserve provider visibility
/// without retaining a host-specific temporary directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTreeEntry {
    pub tree: ProviderTree,
    pub path: String,
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_kind: Option<ProviderItemKind>,
    pub byte_len: usize,
    pub digest: String,
    pub temporary: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTreeSnapshot {
    pub device: String,
    pub partitioned: bool,
    pub entries: Vec<ProviderTreeEntry>,
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
    ProviderResidue {
        device: String,
        max_entries: usize,
        max_bytes: usize,
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
    /// Copy one complete immutable provider file.  The copy is an explicit
    /// scheduled action; it does not cause a receiver to ingest anything.
    ProviderCopy {
        source: ProviderSource,
        destination: ProviderLocation,
    },
    /// Start an on-disk partial write.  Until `FinishProviderWrite` performs
    /// the final atomic rename, rescans treat it solely as visible residue.
    BeginProviderWrite {
        source: ProviderSource,
        destination: ProviderLocation,
        transfer_id: String,
    },
    AppendProviderWrite {
        device: String,
        transfer_id: String,
        len: usize,
    },
    FinishProviderWrite {
        device: String,
        transfer_id: String,
    },
    ProviderRename {
        device: String,
        tree: ProviderTree,
        from_path: String,
        to_path: String,
    },
    ProviderRemove {
        location: ProviderLocation,
    },
    /// A partition prevents provider copies into this device and makes its
    /// receiver rescan a no-op.  Rejoin is the same action with `false`.
    SetProviderPartition {
        device: String,
        partitioned: bool,
    },
    /// Explicit bounded receiver scan of that device's inbox.
    ReceiverRescan {
        device: String,
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
            if !valid_scenario_device_name(&device.name)
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
    ProviderResidue,
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

struct ResolvedProviderItem {
    bytes: Arc<[u8]>,
    source_binding: String,
    source_identity: Option<ProviderIdentityRecord>,
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

struct ProviderWrite {
    destination: ProviderLocation,
    source: Arc<[u8]>,
    source_provenance: String,
    next: usize,
    file: ProviderStagingFile,
}

struct ProviderStagingFile {
    file: fs::File,
    /// Present only when the staging file has a directory entry. Publication
    /// must consume this exact name; it may never create a second hard link.
    name: Option<String>,
}

impl std::ops::Deref for ProviderStagingFile {
    type Target = fs::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl std::ops::DerefMut for ProviderStagingFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderJournalOperation {
    Put,
    Rename,
    Remove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderJournalPhase {
    Prepared,
    Staged,
    PublishIntent,
    Published,
    RetireIntent,
    Retired,
    Cleanup,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderIdentityRecord {
    platform: String,
    first: u64,
    second: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderJournalRecord {
    journal_schema_version: u32,
    operation_id: String,
    operation: ProviderJournalOperation,
    operation_binding: String,
    source_provenance: String,
    tree: ProviderTree,
    from_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_identity: Option<ProviderIdentityRecord>,
    source_len: u64,
    source_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob_name: Option<String>,
    phase: ProviderJournalPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    staging_identity: Option<ProviderIdentityRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    destination_identity: Option<ProviderIdentityRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    staging_name: Option<String>,
    #[serde(default)]
    staging_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagnostic_path: Option<String>,
    authentication_tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderAuthorityRecord {
    authority_schema_version: u32,
    authentication_key: String,
    device_identity: ProviderIdentityRecord,
    journal_identity: ProviderIdentityRecord,
    authority_key_identity: ProviderIdentityRecord,
    records_identity: ProviderIdentityRecord,
    blobs_identity: ProviderIdentityRecord,
    quarantine_identity: ProviderIdentityRecord,
    completed_identity: ProviderIdentityRecord,
}

struct ProviderRetryJournal {
    root: PathBuf,
    name: String,
    directory: Dir,
    directory_identity: ProviderFileIdentity,
    records: Dir,
    records_identity: ProviderFileIdentity,
    blobs: Dir,
    blobs_identity: ProviderFileIdentity,
    quarantine: Dir,
    quarantine_identity: ProviderFileIdentity,
    completed: Dir,
    completed_identity: ProviderFileIdentity,
    authentication_key: [u8; 32],
    transaction_authority: Arc<ProviderTransactionAuthority>,
}

/// The device directory is the outer authority: it is outside both mutable
/// provider residue and the retry journal. Unix locks that retained directory
/// descriptor directly. Windows retains the device directory and locks this
/// authority file, whose handle denies delete sharing so its name cannot be
/// replaced while any process scope is live.
struct ProviderTransactionAuthority {
    device_parent: Dir,
    device_name: String,
    device_directory: Dir,
    device_identity: ProviderFileIdentity,
    authority_file: fs::File,
    authority_identity: ProviderFileIdentity,
    authority_record_bytes: Vec<u8>,
    authority_key_file: fs::File,
    authority_key_identity: ProviderFileIdentity,
    local_held: AtomicBool,
}

/// An owned, typed capability proving that the one provider transaction gate
/// is held. It can cross Rust borrow boundaries, but helpers reject a token
/// minted by any other journal authority.
struct ProviderTransactionGate {
    authority: Arc<ProviderTransactionAuthority>,
    lock_file: fs::File,
}

enum ProviderSourceTransactionGate<'a> {
    Mailbox,
    Tree(&'a ProviderTransactionGate),
}

impl Drop for ProviderTransactionGate {
    fn drop(&mut self) {
        provider_unlock_file(&self.lock_file);
        self.authority.local_held.store(false, Ordering::Release);
    }
}

/// A validated provider file whose descriptor remains the authority for any
/// subsequent operation.  Keeping the handle and the bytes together prevents
/// an open-then-pathname publication or deletion race.
struct OpenProviderFile {
    file: fs::File,
    bytes: Vec<u8>,
}

/// A stable identity captured from the retained file handle.  Path checks are
/// only useful when they are tied back to this identity: an exposed provider
/// path may be renamed or replaced between trace actions.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProviderFileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProviderFileIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProviderFileIdentity;

struct ProviderRuntime {
    root: PathBuf,
    inbox: Dir,
    outbox: Dir,
    writes: BTreeMap<String, ProviderWrite>,
    partitioned: bool,
}

impl ProviderRuntime {
    fn open(root: PathBuf) -> Result<Self, ScenarioError> {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ScenarioError::UnsafeProviderEntry(root.display().to_string()))?;
        let parent = root
            .parent()
            .ok_or_else(|| ScenarioError::UnsafeProviderEntry(root.display().to_string()))?;
        let canonical_parent =
            fs::canonicalize(parent).map_err(|error| ScenarioError::Io(error.to_string()))?;
        let parent_capability = Dir::open_ambient_dir(&canonical_parent, ambient_authority())
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        ensure_provider_directory(&parent_capability, name)?;
        let provider = open_provider_directory(&parent_capability, name)?;
        for tree in ["inbox", "outbox"] {
            ensure_provider_directory(&provider, tree)?;
            let tree = open_provider_directory(&provider, tree)?;
            for namespace in [
                PROVIDER_OBJECTS_NAMESPACE,
                PROVIDER_MANIFESTS_NAMESPACE,
                PROVIDER_TEMP_NAMESPACE,
                PROVIDER_REMOVED_NAMESPACE,
                PROVIDER_RENAME_EVIDENCE_NAMESPACE,
            ] {
                ensure_provider_directory(&tree, namespace)?;
                let _ = open_provider_directory(&tree, namespace)?;
            }
        }
        let inbox = open_provider_directory(&provider, "inbox")?;
        let outbox = open_provider_directory(&provider, "outbox")?;
        Ok(Self {
            root: canonical_parent.join(name),
            inbox,
            outbox,
            writes: BTreeMap::new(),
            partitioned: false,
        })
    }

    fn tree_path(&self, tree: ProviderTree) -> PathBuf {
        self.root.join(match tree {
            ProviderTree::Inbox => "inbox",
            ProviderTree::Outbox => "outbox",
        })
    }

    fn tree(&self, tree: ProviderTree) -> &Dir {
        match tree {
            ProviderTree::Inbox => &self.inbox,
            ProviderTree::Outbox => &self.outbox,
        }
    }

    fn parent_and_name(
        &self,
        tree: ProviderTree,
        path: &str,
        create: bool,
    ) -> Result<(Dir, String), ScenarioError> {
        if !valid_provider_path(path) {
            return Err(ScenarioError::InvalidProviderPath(path.into()));
        }
        let mut components = path.split('/').peekable();
        let mut parent = self
            .tree(tree)
            .try_clone()
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                return Ok((parent, component.into()));
            }
            if create {
                ensure_provider_directory(&parent, component)?;
            }
            parent = open_provider_directory(&parent, component)?;
        }
        Err(ScenarioError::InvalidProviderPath(path.into()))
    }

    fn put_complete(
        &mut self,
        journal: &ProviderRetryJournal,
        gate: &ProviderTransactionGate,
        operation_binding: &str,
        source_provenance: &str,
        location: &ProviderLocation,
        bytes: &[u8],
        source_identity: Option<ProviderIdentityRecord>,
        initial_staging_name: Option<String>,
    ) -> Result<(), ScenarioError> {
        journal.require_transaction_gate(gate)?;
        reject_provider_temporary_path(&location.path)?;
        let supplied_source_identity = source_identity.clone();
        let (destination_dir, destination_name) =
            self.parent_and_name(location.tree, &location.path, true)?;
        let temporary_dir =
            open_provider_directory(self.tree(location.tree), PROVIDER_TEMP_NAMESPACE)?;
        let mut record = match journal.load(
            gate,
            ProviderJournalOperation::Put,
            operation_binding,
            source_provenance,
            location.tree,
            &location.path,
            None,
        )? {
            Some(record) => record,
            None => {
                if open_provider_regular_optional(
                    &destination_dir,
                    &destination_name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    &location.path,
                )?
                .is_some()
                {
                    return Err(ScenarioError::ProviderConflictingBytes(
                        location.path.clone(),
                    ));
                }
                let operation_id = ProviderRetryJournal::operation_id(
                    ProviderJournalOperation::Put,
                    operation_binding,
                    source_provenance,
                    location.tree,
                    &location.path,
                    None,
                    u64::try_from(bytes.len())
                        .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                    &provider_digest(bytes),
                );
                let initial_identity = if initial_staging_name.is_some() {
                    source_identity.clone()
                } else {
                    None
                };
                let initial_phase = if initial_staging_name.is_some() {
                    ProviderJournalPhase::Staged
                } else {
                    ProviderJournalPhase::Prepared
                };
                let initial_generation = if initial_staging_name.is_none()
                    && operation_binding.starts_with("transfer:")
                {
                    1
                } else {
                    0
                };
                let record = ProviderJournalRecord {
                    journal_schema_version: PROVIDER_JOURNAL_SCHEMA_VERSION,
                    operation_id: operation_id.clone(),
                    operation: ProviderJournalOperation::Put,
                    operation_binding: operation_binding.into(),
                    source_provenance: source_provenance.into(),
                    tree: location.tree,
                    from_path: location.path.clone(),
                    to_path: None,
                    source_identity,
                    source_len: u64::try_from(bytes.len())
                        .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                    source_digest: provider_digest(bytes),
                    blob_name: Some(ProviderRetryJournal::blob_name(&operation_id)),
                    phase: initial_phase,
                    staging_identity: initial_identity,
                    destination_identity: None,
                    staging_name: Some(initial_staging_name.unwrap_or_else(|| {
                        ProviderRetryJournal::staging_name(
                            &operation_id,
                            initial_generation,
                        )
                    })),
                    staging_generation: initial_generation,
                    diagnostic_path: None,
                    authentication_tag: String::new(),
                };
                journal.create(gate, &record, Some(bytes))?;
                provider_journal_after_phase_hook(record.phase)?;
                record
            }
        };
        if u64::try_from(bytes.len()).ok() != Some(record.source_len)
            || provider_digest(bytes) != record.source_digest
            || record.source_provenance != source_provenance
            || supplied_source_identity
                .as_ref()
                .is_some_and(|identity| record.source_identity.as_ref() != Some(identity))
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                record.operation_id.clone(),
            ));
        }
        if record.phase == ProviderJournalPhase::Cleanup {
            validate_put_destination(
                &destination_dir,
                &destination_name,
                &location.path,
                bytes,
                &record,
            )?;
            return journal.complete(gate, &record);
        }
        let expected = journal.read_blob(gate, &record)?;
        if record.phase == ProviderJournalPhase::Prepared {
            if open_provider_regular_optional(
                &destination_dir,
                &destination_name,
                MAX_PROVIDER_RESCAN_BYTES,
                &location.path,
            )?
            .is_some()
            {
                return Err(ScenarioError::ProviderConflictingBytes(
                    location.path.clone(),
                ));
            }
            loop {
                let staging_name = record.staging_name.as_deref().ok_or_else(|| {
                    ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
                })?;
                if open_provider_regular_optional(
                    &temporary_dir,
                    staging_name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    staging_name,
                )?
                .is_none()
                {
                    break;
                }
                quarantine_unowned_staging(
                    journal,
                    gate,
                    &temporary_dir,
                    staging_name,
                    self.tree(location.tree),
                    &record.operation_id,
                    record.staging_generation,
                )?;
                record.staging_generation = record
                    .staging_generation
                    .checked_add(1)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                record.staging_name = Some(ProviderRetryJournal::staging_name(
                    &record.operation_id,
                    record.staging_generation,
                ));
                journal.store(gate, &record)?;
            }
            let staging_name = record.staging_name.as_deref().ok_or_else(|| {
                ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
            })?;
            let mut staged =
                create_provider_journal_staging(&temporary_dir, staging_name, &location.path)?;
            staged
                .write_all(&expected)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            staged
                .sync_all()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            validate_provider_file_bytes(&mut staged, &expected, &location.path)?;
            record.staging_identity =
                Some(provider_identity_record(provider_file_identity(&staged.file)?));
            record.phase = ProviderJournalPhase::Staged;
            journal.store(gate, &record)?;
            provider_journal_after_phase_hook(ProviderJournalPhase::Staged)?;
        }
        if record.phase == ProviderJournalPhase::Staged {
            validate_journal_staging(&temporary_dir, &record, &expected, &location.path)?;
            record.phase = ProviderJournalPhase::PublishIntent;
            journal.store(gate, &record)?;
            provider_journal_after_phase_hook(ProviderJournalPhase::PublishIntent)?;
        }
        if record.phase == ProviderJournalPhase::PublishIntent {
            publish_journal_destination(
                journal,
                gate,
                &mut record,
                &temporary_dir,
                self.tree(location.tree),
                &destination_dir,
                &destination_name,
                &expected,
                &location.path,
            )?;
            validate_put_destination(
                &destination_dir,
                &destination_name,
                &location.path,
                &expected,
                &record,
            )?;
            sync_provider_publication_directories(&destination_dir, Some(&temporary_dir))?;
            record.phase = ProviderJournalPhase::Published;
            journal.store(gate, &record)?;
            provider_journal_after_phase_hook(ProviderJournalPhase::Published)?;
        }
        validate_put_destination(
            &destination_dir,
            &destination_name,
            &location.path,
            &expected,
            &record,
        )?;
        journal.complete(gate, &record)
    }

    fn snapshot(&self, device: &str) -> Result<ProviderTreeSnapshot, ScenarioError> {
        let mut entries = Vec::new();
        let mut remaining_entries = MAX_PROVIDER_RESIDUE_ENTRIES;
        let mut remaining_bytes = MAX_PROVIDER_RESCAN_BYTES;
        for tree in [ProviderTree::Inbox, ProviderTree::Outbox] {
            let files = bounded_provider_files(
                self.tree(tree),
                true,
                remaining_entries,
                remaining_bytes,
            )?;
            remaining_entries = remaining_entries
                .checked_sub(files.len())
                .ok_or_else(|| ScenarioError::ProviderResidueLimit(device.into()))?;
            let tree_bytes = files.iter().try_fold(0_usize, |total, file| {
                total.checked_add(file.bytes.len())
            });
            remaining_bytes = remaining_bytes
                .checked_sub(
                    tree_bytes
                        .ok_or_else(|| ScenarioError::ProviderResidueLimit(device.into()))?,
                )
                .ok_or_else(|| ScenarioError::ProviderResidueLimit(device.into()))?;
            for file in files {
                entries.push(ProviderTreeEntry {
                    tree,
                    item_id: format!("provider/{}/{}", provider_tree_name(tree), file.path),
                    item_kind: provider_item_kind(&file.path),
                    byte_len: file.bytes.len(),
                    digest: format!("{:x}", Sha256::digest(&file.bytes)),
                    path: file.path,
                    temporary: file.temporary,
                });
            }
        }
        entries.sort_by(|left, right| {
            (left.tree, &left.path, left.temporary).cmp(&(right.tree, &right.path, right.temporary))
        });
        let total_bytes = entries
            .iter()
            .try_fold(0_usize, |total, entry| total.checked_add(entry.byte_len));
        if entries.len() > MAX_PROVIDER_RESIDUE_ENTRIES
            || total_bytes.is_none_or(|bytes| bytes > MAX_PROVIDER_RESCAN_BYTES)
        {
            return Err(ScenarioError::ProviderResidueLimit(device.into()));
        }
        Ok(ProviderTreeSnapshot {
            device: device.into(),
            partitioned: self.partitioned,
            entries,
        })
    }
}

impl ProviderRetryJournal {
    fn open(root: PathBuf) -> Result<Self, ScenarioError> {
        let journal_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(root.display().to_string()))?;
        let device_path = root
            .parent()
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(root.display().to_string()))?;
        let device_name = device_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(root.display().to_string()))?;
        let device_parent_path = device_path
            .parent()
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(root.display().to_string()))?;
        let canonical_device_parent = fs::canonicalize(device_parent_path)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        let device_parent =
            Dir::open_ambient_dir(&canonical_device_parent, ambient_authority())
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
        let device_directory = open_provider_directory(&device_parent, device_name)?;
        let device_identity = provider_directory_identity(&device_directory)?;
        let (authority_file, initial_lock_file, authority_created) =
            open_and_lock_provider_outer_authority(&device_directory)?;
        let authority_identity = provider_file_identity(&authority_file)?;
        let mut initial_lock_file = Some(initial_lock_file);
        let result = (|| {
            let existing_authority = if authority_created {
                None
            } else {
                Some(read_provider_authority_record(&authority_file)?)
            };

            let directory = if authority_created {
                ensure_provider_directory(&device_directory, journal_name)?;
                open_provider_directory(&device_directory, journal_name)?
            } else {
                open_provider_directory(&device_directory, journal_name)
                    .map_err(|_| {
                        ScenarioError::UnsafeProviderJournal(
                            "provider journal root was replaced".into(),
                        )
                    })?
            };
            let directory_identity = provider_directory_identity(&directory)?;
            if existing_authority.as_ref().is_some_and(|(_, record)| {
                provider_identity_record(directory_identity) != record.journal_identity
            }) {
                return Err(ScenarioError::UnsafeProviderJournal(
                    "provider journal root identity changed".into(),
                ));
            }

            for child in ["records", "blobs", "quarantine", "completed"] {
                if authority_created {
                    ensure_provider_directory(&directory, child)?;
                }
            }
            let records = open_provider_directory(&directory, "records")?;
            let blobs = open_provider_directory(&directory, "blobs")?;
            let quarantine = open_provider_directory(&directory, "quarantine")?;
            let completed = open_provider_directory(&directory, "completed")?;
            let records_identity = provider_directory_identity(&records)?;
            let blobs_identity = provider_directory_identity(&blobs)?;
            let quarantine_identity = provider_directory_identity(&quarantine)?;
            let completed_identity = provider_directory_identity(&completed)?;
            if existing_authority.as_ref().is_some_and(|(_, record)| {
                provider_identity_record(records_identity) != record.records_identity
                    || provider_identity_record(blobs_identity) != record.blobs_identity
                    || provider_identity_record(quarantine_identity)
                        != record.quarantine_identity
                    || provider_identity_record(completed_identity)
                        != record.completed_identity
            }) {
                return Err(ScenarioError::UnsafeProviderJournal(
                    "provider journal namespace identity changed".into(),
                ));
            }

            let (mut authority_key_file, authentication_key, authority_key_identity) =
                if let Some((authority_record_bytes, authority_record)) =
                    existing_authority.as_ref()
                {
                    let key = decode_provider_authentication_key(authority_record)?;
                    let mut opened =
                        open_provider_authority_key_nofollow(&directory, "authority.key")?;
                    validate_local_file_bytes(&mut opened, &key, "authority.key")?;
                    let identity = provider_file_identity(&opened)?;
                    if provider_identity_record(device_identity)
                        != authority_record.device_identity
                        || provider_identity_record(identity)
                            != authority_record.authority_key_identity
                    {
                        return Err(ScenarioError::UnsafeProviderJournal(
                            "provider authority binding changed".into(),
                        ));
                    }
                    let mut outer = authority_file
                        .try_clone()
                        .map_err(|error| ScenarioError::Io(error.to_string()))?;
                    validate_local_file_bytes(
                        &mut outer,
                        authority_record_bytes,
                        PROVIDER_DEVICE_AUTHORITY_NAME,
                    )?;
                    (opened, key, identity)
                } else {
                    if open_provider_authority_key_optional(&directory, "authority.key")?
                        .is_some()
                        || records
                            .entries()
                            .map_err(|error| ScenarioError::Io(error.to_string()))?
                            .next()
                            .is_some()
                        || blobs
                            .entries()
                            .map_err(|error| ScenarioError::Io(error.to_string()))?
                            .next()
                            .is_some()
                        || quarantine
                            .entries()
                            .map_err(|error| ScenarioError::Io(error.to_string()))?
                            .next()
                            .is_some()
                        || completed
                            .entries()
                            .map_err(|error| ScenarioError::Io(error.to_string()))?
                            .next()
                            .is_some()
                    {
                        return Err(ScenarioError::UnsafeProviderJournal(
                            "missing outer provider authority".into(),
                        ));
                    }
                    let first = Uuid::new_v4();
                    let second = Uuid::new_v4();
                    let mut key = [0_u8; 32];
                    key[..16].copy_from_slice(first.as_bytes());
                    key[16..].copy_from_slice(second.as_bytes());
                    let mut file =
                        create_provider_authority_key_exclusive(&directory, "authority.key")?;
                    file.write_all(&key)
                        .map_err(|error| ScenarioError::Io(error.to_string()))?;
                    file.sync_all()
                        .map_err(|error| ScenarioError::Io(error.to_string()))?;
                    validate_local_file_bytes(&mut file, &key, "authority.key")?;
                    sync_provider_directory(&directory)?;
                    let identity = provider_file_identity(&file)?;
                    (file, key, identity)
                };

            let authority_record = existing_authority
                .as_ref()
                .map(|(_, record)| record.clone())
                .unwrap_or_else(|| ProviderAuthorityRecord {
                    authority_schema_version: PROVIDER_AUTHORITY_SCHEMA_VERSION,
                    authentication_key: base64url_encode(&authentication_key),
                    device_identity: provider_identity_record(device_identity),
                    journal_identity: provider_identity_record(directory_identity),
                    authority_key_identity: provider_identity_record(authority_key_identity),
                    records_identity: provider_identity_record(records_identity),
                    blobs_identity: provider_identity_record(blobs_identity),
                    quarantine_identity: provider_identity_record(quarantine_identity),
                    completed_identity: provider_identity_record(completed_identity),
                });
            let authority_record_bytes = canonical_provider_authority_bytes(&authority_record)?;
            if authority_created {
                let mut outer = authority_file
                    .try_clone()
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                outer
                    .write_all(&authority_record_bytes)
                    .and_then(|()| outer.sync_all())
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                validate_local_file_bytes(
                    &mut outer,
                    &authority_record_bytes,
                    PROVIDER_DEVICE_AUTHORITY_NAME,
                )?;
                sync_provider_directory(&device_directory)?;
            }
            validate_local_file_bytes(
                &mut authority_key_file,
                &authentication_key,
                "authority.key",
            )?;

            let transaction_authority = Arc::new(ProviderTransactionAuthority {
                device_parent: device_parent
                    .try_clone()
                    .map_err(|error| ScenarioError::Io(error.to_string()))?,
                device_name: device_name.into(),
                device_directory: device_directory
                    .try_clone()
                    .map_err(|error| ScenarioError::Io(error.to_string()))?,
                device_identity,
                authority_file,
                authority_identity,
                authority_record_bytes,
                authority_key_file,
                authority_key_identity,
                local_held: AtomicBool::new(true),
            });
            let journal = Self {
                root: canonical_device_parent.join(device_name).join(journal_name),
                name: journal_name.into(),
                directory,
                directory_identity,
                records,
                records_identity,
                blobs,
                blobs_identity,
                quarantine,
                quarantine_identity,
                completed,
                completed_identity,
                authentication_key,
                transaction_authority: Arc::clone(&transaction_authority),
            };
            let gate = ProviderTransactionGate {
                authority: transaction_authority,
                lock_file: initial_lock_file.take().ok_or_else(|| {
                    ScenarioError::UnsafeProviderJournal(
                        "provider transaction gate was lost".into(),
                    )
                })?,
            };
            journal.validate_transaction_binding(&gate)?;
            journal.validate_raw_pending_usage(&gate)?;
            // Quarantine recovery precedes graph validation because a crash
            // may leave authenticated creation bytes privately retained
            // while their signed update still names `.creating`.
            journal.reconcile_orphan_quarantine(&gate)?;
            // Validate the complete authenticated graph before reconciliation
            // can rename or remove anything. A valid-looking filename never
            // grants update, record, completion, or blob authority.
            let orphan_blobs = journal.validate_authenticated_graph(&gate)?;
            journal.retire_orphan_blobs(&gate, &orphan_blobs)?;
            journal.reconcile_updates(&gate)?;
            journal.reconcile_completed_updates(&gate)?;
            journal.validate_usage(&gate, 0, 0, false)?;
            journal.validate_completed_usage(&gate, 0, 0)?;
            drop(gate);
            Ok(journal)
        })();
        if let Some(lock_file) = initial_lock_file.as_ref() {
            provider_unlock_file(lock_file);
        }
        result
    }

    fn validate_raw_pending_usage(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut files = 0_usize;
        let mut bytes = 0_usize;
        let mut operations = BTreeSet::new();
        for (directory, kind) in [
            (&self.records, "record"),
            (&self.blobs, "blob"),
            (&self.quarantine, "quarantine"),
        ] {
            for entry in directory
                .entries()
                .map_err(|error| ScenarioError::Io(error.to_string()))?
            {
                let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
                files = files
                    .checked_add(1)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                if files > MAX_PROVIDER_JOURNAL_FILES {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
                let operation_id = match kind {
                    "record" => name
                        .strip_suffix(".json")
                        .or_else(|| name.strip_suffix(".update")),
                    "blob" => name
                        .strip_suffix(".blob")
                        .or_else(|| name.strip_suffix(".creating")),
                    "quarantine" => name.strip_suffix(".creating"),
                    _ => None,
                }
                .filter(|value| valid_provider_journal_id(value))
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                operations.insert(operation_id.to_owned());
                if operations.len() > MAX_PROVIDER_JOURNAL_PENDING {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
                let file = open_provider_file_nofollow(directory, &name)
                    .map_err(|error| ScenarioError::UnsafeProviderJournal(error.to_string()))?;
                let metadata = validate_provider_regular_file(&file, &name)
                    .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                let len = usize::try_from(metadata.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?;
                if (kind == "record" && len > MAX_PROVIDER_JOURNAL_RECORD_BYTES)
                    || (kind != "record" && len > MAX_PROVIDER_JOURNAL_BLOB_BYTES)
                {
                    return Err(ScenarioError::UnsafeProviderJournal(name));
                }
                bytes = bytes
                    .checked_add(len)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                if bytes > MAX_PROVIDER_JOURNAL_BYTES {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
            }
        }
        Ok(())
    }

    fn acquire_transaction_gate(&self) -> Result<ProviderTransactionGate, ScenarioError> {
        if self
            .transaction_authority
            .local_held
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                "provider transaction gate is busy".into(),
            ));
        }
        let lock_file = match provider_transaction_lock_handle(&self.transaction_authority) {
            Ok(lock_file) => lock_file,
            Err(error) => {
                self.transaction_authority
                    .local_held
                    .store(false, Ordering::Release);
                return Err(ScenarioError::Io(error.to_string()));
            }
        };
        let acquired = match provider_lock_file_exclusive_nonblocking(&lock_file) {
            Ok(acquired) => acquired,
            Err(error) => {
                self.transaction_authority
                    .local_held
                    .store(false, Ordering::Release);
                return Err(ScenarioError::Io(error.to_string()));
            }
        };
        if !acquired {
            self.transaction_authority
                .local_held
                .store(false, Ordering::Release);
            return Err(ScenarioError::UnsafeProviderJournal(
                "provider transaction gate is held by another process".into(),
            ));
        }
        let gate = ProviderTransactionGate {
            authority: Arc::clone(&self.transaction_authority),
            lock_file,
        };
        self.validate_transaction_binding(&gate)?;
        Ok(gate)
    }

    fn require_transaction_gate(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<(), ScenarioError> {
        if Arc::ptr_eq(&self.transaction_authority, &gate.authority)
            && self
                .transaction_authority
                .local_held
                .load(Ordering::Acquire)
        {
            Ok(())
        } else {
            Err(ScenarioError::UnsafeProviderJournal(
                "wrong provider transaction gate".into(),
            ))
        }
    }

    fn validate_transaction_binding(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let authority = &self.transaction_authority;
        let named_device =
            open_provider_directory(&authority.device_parent, &authority.device_name)
                .map_err(|_| {
                    ScenarioError::UnsafeProviderJournal(
                        "device authority path was replaced".into(),
                    )
                })?;
        if provider_directory_identity(&named_device)? != authority.device_identity
            || provider_directory_identity(&authority.device_directory)?
                != authority.device_identity
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                "device authority identity changed".into(),
            ));
        }
        let mut named_outer = open_provider_outer_authority_file_nofollow(
            &authority.device_directory,
            PROVIDER_DEVICE_AUTHORITY_NAME,
        )?;
        if provider_file_identity(&named_outer)? != authority.authority_identity
            || provider_file_identity(&authority.authority_file)?
                != authority.authority_identity
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                "outer provider authority identity changed".into(),
            ));
        }
        validate_local_file_bytes(
            &mut named_outer,
            &authority.authority_record_bytes,
            PROVIDER_DEVICE_AUTHORITY_NAME,
        )?;

        validate_named_provider_directory(
            &authority.device_directory,
            &self.name,
            &self.directory,
            self.directory_identity,
        )?;
        validate_named_provider_directory(
            &self.directory,
            "records",
            &self.records,
            self.records_identity,
        )?;
        validate_named_provider_directory(
            &self.directory,
            "blobs",
            &self.blobs,
            self.blobs_identity,
        )?;
        validate_named_provider_directory(
            &self.directory,
            "quarantine",
            &self.quarantine,
            self.quarantine_identity,
        )?;
        validate_named_provider_directory(
            &self.directory,
            "completed",
            &self.completed,
            self.completed_identity,
        )?;
        let mut named_key =
            open_provider_authority_key_nofollow(&self.directory, "authority.key")?;
        if provider_file_identity(&named_key)? != authority.authority_key_identity
            || provider_file_identity(&authority.authority_key_file)?
                != authority.authority_key_identity
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                "authority.key identity changed".into(),
            ));
        }
        validate_local_file_bytes(&mut named_key, &self.authentication_key, "authority.key")?;

        let mut root_entries = 0_usize;
        for entry in self
            .directory
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            root_entries = root_entries
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if root_entries > 5 {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            let file_type = entry
                .file_type()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            let valid = match name.as_str() {
                "records" | "blobs" | "quarantine" | "completed" => file_type.is_dir(),
                "authority.key" => file_type.is_file(),
                _ => false,
            };
            if !valid {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
        }
        Ok(())
    }

    fn validate_authenticated_graph(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<Vec<String>, ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut blob_owners = BTreeMap::<String, usize>::new();
        let mut pending_files = 0_usize;
        let mut pending_bytes = 0_usize;
        for entry in self
            .records
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            pending_files = pending_files
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if pending_files > MAX_PROVIDER_JOURNAL_FILES {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            let operation_id = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".update"))
                .filter(|value| valid_provider_journal_id(value))
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            let opened = open_provider_regular_optional(
                &self.records,
                &name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            pending_bytes = pending_bytes
                .checked_add(opened.bytes.len())
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if pending_bytes > MAX_PROVIDER_JOURNAL_BYTES {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let record = self.decode_record(&opened.bytes, &name)?;
            self.validate_record_shape(gate, &record, false)?;
            if record.operation_id != operation_id {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            let mut canonical_owner = name.ends_with(".json");
            let mut creation_owner = false;
            if name.ends_with(".update") {
                let current_name = Self::record_name(operation_id);
                let current = open_provider_regular_optional(
                    &self.records,
                    &current_name,
                    MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                    &current_name,
                )
                .map_err(|_| ScenarioError::UnsafeProviderJournal(current_name.clone()))?;
                if let Some(current) = current {
                    let current_record = self.decode_record(&current.bytes, &current_name)?;
                    self.validate_record_shape(gate, &current_record, false)?;
                    if current_record.operation_id != record.operation_id
                        || provider_journal_phase_rank(record.phase)
                            < provider_journal_phase_rank(current_record.phase)
                    {
                        return Err(ScenarioError::UnsafeProviderJournal(name));
                    }
                } else {
                    if !matches!(
                        record.phase,
                        ProviderJournalPhase::Prepared | ProviderJournalPhase::Staged
                    ) {
                        return Err(ScenarioError::UnsafeProviderJournal(name));
                    }
                    canonical_owner = true;
                    creation_owner = true;
                }
            }
            if canonical_owner {
                if let Some(blob_name) = record.blob_name.as_ref() {
                    let creating_name = Self::creating_blob_name(&record.operation_id);
                    let blob_exists = self.blobs.exists(blob_name);
                    let creating_exists = self.blobs.exists(&creating_name);
                    let owner_name = if blob_exists && !creating_exists {
                        Some(blob_name.clone())
                    } else if !blob_exists && creating_exists && creation_owner {
                        Some(creating_name)
                    } else if !blob_exists
                        && !creating_exists
                        && record.phase == ProviderJournalPhase::Cleanup
                    {
                        None
                    } else {
                        return Err(ScenarioError::UnsafeProviderJournal(
                            blob_name.clone(),
                        ));
                    };
                    if let Some(owner_name) = owner_name {
                        let owners = blob_owners.entry(owner_name).or_default();
                        *owners = owners
                            .checked_add(1)
                            .ok_or(ScenarioError::ProviderJournalLimit)?;
                    }
                }
            }
        }

        let mut completed_files = 0_usize;
        for entry in self
            .completed
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            completed_files = completed_files
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if completed_files > MAX_PROVIDER_JOURNAL_COMPLETED + 1 {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            let operation_id = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(".update"))
                .filter(|value| valid_provider_journal_id(value))
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            let opened = open_provider_regular_optional(
                &self.completed,
                &name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            let record = self.decode_record(&opened.bytes, &name)?;
            self.validate_record_shape(gate, &record, false)?;
            if record.operation_id != operation_id
                || record.phase != ProviderJournalPhase::Cleanup
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
        }

        let mut blobs = 0_usize;
        let mut orphan_blobs = Vec::new();
        for entry in self
            .blobs
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            blobs = blobs
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if blobs > MAX_PROVIDER_JOURNAL_PENDING {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            let (operation_id, creating) = if let Some(operation_id) = name
                .strip_suffix(".blob")
                .filter(|value| valid_provider_journal_id(value))
            {
                (operation_id, false)
            } else if let Some(operation_id) = name
                .strip_suffix(".creating")
                .filter(|value| valid_provider_journal_id(value))
            {
                (operation_id, true)
            } else {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            };
            if name
                != if creating {
                    Self::creating_blob_name(operation_id)
                } else {
                    Self::blob_name(operation_id)
                }
                || blob_owners
                    .get(&name)
                    .is_some_and(|owners| *owners != 1)
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            let opened = open_provider_regular_optional(
                &self.blobs,
                &name,
                MAX_PROVIDER_JOURNAL_BLOB_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            pending_bytes = pending_bytes
                .checked_add(opened.bytes.len())
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if pending_bytes > MAX_PROVIDER_JOURNAL_BYTES {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            if !blob_owners.contains_key(&name) {
                if creating {
                    orphan_blobs.push(name);
                    continue;
                }
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            let record_name = Self::record_name(operation_id);
            let (record, owner_name) = if creating {
                let update_name = format!("{operation_id}.update");
                let update = open_provider_regular_optional(
                    &self.records,
                    &update_name,
                    MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                    &update_name,
                )
                .map_err(|_| ScenarioError::UnsafeProviderJournal(update_name.clone()))?
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                (update, update_name)
            } else {
                let record = open_provider_regular_optional(
                    &self.records,
                    &record_name,
                    MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                    &record_name,
                )
                .map_err(|_| ScenarioError::UnsafeProviderJournal(record_name.clone()))?;
                if let Some(record) = record {
                    (record, record_name)
                } else {
                    let update_name = format!("{operation_id}.update");
                    let update = open_provider_regular_optional(
                        &self.records,
                        &update_name,
                        MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                        &update_name,
                    )
                    .map_err(|_| ScenarioError::UnsafeProviderJournal(update_name.clone()))?
                    .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                    (update, update_name)
                }
            };
            let record = self.decode_record(&record.bytes, &owner_name)?;
            let expected_blob_name = Self::blob_name(operation_id);
            if record.blob_name.as_deref() != Some(expected_blob_name.as_str())
                || u64::try_from(opened.bytes.len()).ok() != Some(record.source_len)
                || provider_digest(&opened.bytes) != record.source_digest
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
        }
        if blob_owners
            .iter()
            .any(|(name, owners)| *owners != 1 || !self.blobs.exists(name))
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                "missing or shared blob ownership".into(),
            ));
        }
        Ok(orphan_blobs)
    }

    fn authenticated_creation_owner(
        &self,
        gate: &ProviderTransactionGate,
        operation_id: &str,
    ) -> Result<Option<ProviderJournalRecord>, ScenarioError> {
        self.require_transaction_gate(gate)?;
        let record_name = Self::record_name(operation_id);
        if open_provider_regular_optional(
            &self.records,
            &record_name,
            MAX_PROVIDER_JOURNAL_RECORD_BYTES,
            &record_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(record_name.clone()))?
        .is_some()
        {
            return Err(ScenarioError::UnsafeProviderJournal(record_name));
        }
        let update_name = format!("{operation_id}.update");
        let Some(update) = open_provider_regular_optional(
            &self.records,
            &update_name,
            MAX_PROVIDER_JOURNAL_RECORD_BYTES,
            &update_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(update_name.clone()))?
        else {
            return Ok(None);
        };
        let record = self.decode_record(&update.bytes, &update_name)?;
        self.validate_record_shape(gate, &record, false)?;
        if record.operation_id != operation_id
            || !matches!(
                record.phase,
                ProviderJournalPhase::Prepared | ProviderJournalPhase::Staged
            )
            || record.blob_name.as_deref() != Some(Self::blob_name(operation_id).as_str())
        {
            return Err(ScenarioError::UnsafeProviderJournal(update_name));
        }
        Ok(Some(record))
    }

    fn reconcile_orphan_quarantine(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut names = Vec::new();
        for entry in self
            .quarantine
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            if names.len() >= MAX_PROVIDER_JOURNAL_PENDING {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            if !entry
                .file_type()
                .map_err(|error| ScenarioError::Io(error.to_string()))?
                .is_file()
                || !name
                    .strip_suffix(".creating")
                    .is_some_and(valid_provider_journal_id)
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            names.push(name);
        }
        names.sort();
        for name in names {
            self.resolve_orphan_quarantine(gate, &name)?;
        }
        Ok(())
    }

    fn resolve_orphan_quarantine(
        &self,
        gate: &ProviderTransactionGate,
        quarantine_name: &str,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let operation_id = quarantine_name
            .strip_suffix(".creating")
            .filter(|value| valid_provider_journal_id(value))
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(quarantine_name.into()))?;
        let quarantined = open_provider_regular_optional(
            &self.quarantine,
            quarantine_name,
            MAX_PROVIDER_JOURNAL_BLOB_BYTES,
            quarantine_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(quarantine_name.into()))?
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(quarantine_name.into()))?;
        let quarantined_identity = provider_file_identity(&quarantined.file)?;
        let owner = self.authenticated_creation_owner(gate, operation_id)?;
        provider_journal_boundary_hook(ProviderJournalBoundary::OrphanOwnershipRechecked)?;
        if let Some(owner) = owner {
            if u64::try_from(quarantined.bytes.len()).ok() != Some(owner.source_len)
                || provider_digest(&quarantined.bytes) != owner.source_digest
                || self.blobs.exists(quarantine_name)
                || self.blobs.exists(&Self::blob_name(operation_id))
            {
                return Err(ScenarioError::UnsafeProviderJournal(quarantine_name.into()));
            }
            provider_rename_named_noreplace(
                &self.quarantine,
                quarantine_name,
                &self.blobs,
                quarantine_name,
            )
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_publication_directories(&self.blobs, Some(&self.quarantine))?;
            provider_journal_boundary_hook(ProviderJournalBoundary::OrphanRestored)?;
            return Ok(());
        }
        provider_orphan_before_private_delete_hook();
        let retained = open_provider_regular_optional(
            &self.quarantine,
            quarantine_name,
            MAX_PROVIDER_JOURNAL_BLOB_BYTES,
            quarantine_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(quarantine_name.into()))?
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(quarantine_name.into()))?;
        if provider_file_identity(&retained.file)? != quarantined_identity {
            return Err(ScenarioError::UnsafeProviderJournal(quarantine_name.into()));
        }
        self.quarantine
            .remove_file(quarantine_name)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        sync_provider_directory(&self.quarantine)?;
        provider_journal_boundary_hook(ProviderJournalBoundary::OrphanPrivateDeleted)
    }

    fn retire_orphan_blobs(
        &self,
        gate: &ProviderTransactionGate,
        orphan_blobs: &[String],
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        for blob_name in orphan_blobs {
            if self.quarantine.exists(blob_name) {
                return Err(ScenarioError::UnsafeProviderJournal(blob_name.clone()));
            }
            provider_rename_named_noreplace(
                &self.blobs,
                blob_name,
                &self.quarantine,
                blob_name,
            )
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_publication_directories(&self.quarantine, Some(&self.blobs))?;
            provider_journal_boundary_hook(ProviderJournalBoundary::OrphanQuarantined)?;
            provider_orphan_after_quarantine_hook();
            self.resolve_orphan_quarantine(gate, blob_name)?;
        }
        Ok(())
    }

    fn operation_id(
        operation: ProviderJournalOperation,
        operation_binding: &str,
        source_provenance: &str,
        tree: ProviderTree,
        from_path: &str,
        to_path: Option<&str>,
        source_len: u64,
        source_digest: &str,
    ) -> String {
        let mut digest = Sha256::new();
        digest.update(b"tine-provider-local-journal-operation-v2\0");
        digest.update(match operation {
            ProviderJournalOperation::Put => b"put".as_slice(),
            ProviderJournalOperation::Rename => b"rename".as_slice(),
            ProviderJournalOperation::Remove => b"remove".as_slice(),
        });
        digest.update(b"\0");
        digest.update(operation_binding.as_bytes());
        digest.update(b"\0");
        digest.update(source_provenance.as_bytes());
        digest.update(b"\0");
        digest.update(provider_tree_name(tree).as_bytes());
        digest.update(b"\0");
        digest.update(from_path.as_bytes());
        digest.update(b"\0");
        if let Some(to_path) = to_path {
            digest.update(to_path.as_bytes());
        }
        digest.update(b"\0");
        digest.update(source_len.to_le_bytes());
        digest.update(b"\0");
        digest.update(source_digest.as_bytes());
        format!("{:x}", digest.finalize())
    }

    fn record_name(operation_id: &str) -> String {
        format!("{operation_id}.json")
    }

    fn blob_name(operation_id: &str) -> String {
        format!("{operation_id}.blob")
    }

    fn creating_blob_name(operation_id: &str) -> String {
        format!("{operation_id}.creating")
    }

    fn staging_name(operation_id: &str, generation: u32) -> String {
        format!("publish-{operation_id}-{generation}")
    }

    fn expected_staging_name(record: &ProviderJournalRecord) -> String {
        if record.operation == ProviderJournalOperation::Put
            && record.staging_generation == 0
        {
            if let Some(transfer_id) = record.operation_binding.strip_prefix("transfer:") {
                return format!("{transfer_id}.part");
            }
        }
        Self::staging_name(&record.operation_id, record.staging_generation)
    }

    fn sign_record(&self, record: &mut ProviderJournalRecord) -> Result<(), ScenarioError> {
        record.authentication_tag.clear();
        let bytes =
            serde_json::to_vec(record).map_err(|error| ScenarioError::Io(error.to_string()))?;
        record.authentication_tag = hmac_sha256_hex(&self.authentication_key, &bytes);
        Ok(())
    }

    fn decode_record(
        &self,
        bytes: &[u8],
        name: &str,
    ) -> Result<ProviderJournalRecord, ScenarioError> {
        let record: ProviderJournalRecord = serde_json::from_slice(bytes)
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
        let canonical =
            serde_json::to_vec(&record).map_err(|error| ScenarioError::Io(error.to_string()))?;
        if canonical != bytes || record.authentication_tag.len() != 64 {
            return Err(ScenarioError::UnsafeProviderJournal(name.into()));
        }
        let mut unsigned = record.clone();
        let supplied = std::mem::take(&mut unsigned.authentication_tag);
        let unsigned_bytes =
            serde_json::to_vec(&unsigned).map_err(|error| ScenarioError::Io(error.to_string()))?;
        let expected = hmac_sha256_hex(&self.authentication_key, &unsigned_bytes);
        if !constant_time_bytes_equal(supplied.as_bytes(), expected.as_bytes()) {
            return Err(ScenarioError::UnsafeProviderJournal(name.into()));
        }
        Ok(record)
    }

    fn reconcile_updates(&self, gate: &ProviderTransactionGate) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut scanned = 0_usize;
        let mut updates = Vec::new();
        for entry in self
            .records
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            scanned = scanned
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if scanned > MAX_PROVIDER_JOURNAL_FILES {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            if name.ends_with(".update") {
                updates.push(name);
            }
        }
        updates.sort();
        for update_name in updates {
            let operation_id = update_name
                .strip_suffix(".update")
                .filter(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(update_name.clone()))?;
            let update = open_provider_regular_optional(
                &self.records,
                &update_name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &update_name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(update_name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(update_name.clone()))?;
            let update_record = self.decode_record(&update.bytes, &update_name)?;
            if update_record.operation_id != operation_id {
                return Err(ScenarioError::UnsafeProviderJournal(update_name));
            }
            self.validate_record_shape(gate, &update_record, false)?;
            let record_name = Self::record_name(operation_id);
            let current = open_provider_regular_optional(
                &self.records,
                &record_name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &record_name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(record_name.clone()))?;
            if let Some(current) = current.as_ref() {
                let current_record = self.decode_record(&current.bytes, &record_name)?;
                if current_record.operation_id != operation_id
                    || provider_journal_phase_rank(update_record.phase)
                        < provider_journal_phase_rank(current_record.phase)
                {
                    return Err(ScenarioError::UnsafeProviderJournal(update_name));
                }
            }
            if current.is_none() {
                if let Some(blob_name) = update_record.blob_name.as_deref() {
                    let creating_name = Self::creating_blob_name(operation_id);
                    if !self.blobs.exists(blob_name) {
                        let creating = open_provider_regular_optional(
                            &self.blobs,
                            &creating_name,
                            MAX_PROVIDER_JOURNAL_BLOB_BYTES,
                            &creating_name,
                        )
                        .map_err(|_| {
                            ScenarioError::UnsafeProviderJournal(creating_name.clone())
                        })?
                        .ok_or_else(|| {
                            ScenarioError::UnsafeProviderJournal(blob_name.into())
                        })?;
                        if u64::try_from(creating.bytes.len()).ok()
                            != Some(update_record.source_len)
                            || provider_digest(&creating.bytes)
                                != update_record.source_digest
                        {
                            return Err(ScenarioError::UnsafeProviderJournal(
                                creating_name,
                            ));
                        }
                        self.blobs
                            .rename(&creating_name, &self.blobs, blob_name)
                            .map_err(|error| ScenarioError::Io(error.to_string()))?;
                        sync_provider_directory(&self.blobs)?;
                        provider_journal_boundary_hook(
                            ProviderJournalBoundary::BlobInstalled,
                        )?;
                    }
                }
            }
            self.records
                .rename(&update_name, &self.records, &record_name)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.records)?;
        }
        Ok(())
    }

    fn reconcile_completed_updates(
        &self,
        gate: &ProviderTransactionGate,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut scanned = 0_usize;
        let mut updates = Vec::new();
        for entry in self
            .completed
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            scanned = scanned
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if scanned > MAX_PROVIDER_JOURNAL_COMPLETED + 1 {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            if name.ends_with(".update") {
                updates.push(name);
            }
        }
        updates.sort();
        for update_name in updates {
            let operation_id = update_name
                .strip_suffix(".update")
                .filter(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(update_name.clone()))?;
            let update = open_provider_regular_optional(
                &self.completed,
                &update_name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &update_name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(update_name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(update_name.clone()))?;
            let record = self.decode_record(&update.bytes, &update_name)?;
            if record.operation_id != operation_id
                || record.phase != ProviderJournalPhase::Cleanup
            {
                return Err(ScenarioError::UnsafeProviderJournal(update_name));
            }
            self.validate_record_shape(gate, &record, false)?;
            self.completed
                .rename(
                    &update_name,
                    &self.completed,
                    Self::record_name(operation_id),
                )
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.completed)?;
        }
        Ok(())
    }

    fn validate_completed_usage(
        &self,
        gate: &ProviderTransactionGate,
        additional_files: usize,
        additional_bytes: usize,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut files = 0_usize;
        let mut total_bytes = 0_usize;
        for entry in self
            .completed
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            files = files
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if files
                .checked_add(additional_files)
                .is_none_or(|files| files > MAX_PROVIDER_JOURNAL_COMPLETED)
            {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            if !name.ends_with(".json")
                || name.len() != 64 + ".json".len()
                || !name[..64]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            let opened = open_provider_regular_optional(
                &self.completed,
                &name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            total_bytes = total_bytes
                .checked_add(opened.bytes.len())
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if total_bytes
                .checked_add(additional_bytes)
                .is_none_or(|bytes| bytes > MAX_PROVIDER_JOURNAL_COMPLETION_BYTES)
            {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let record = self.decode_record(&opened.bytes, &name)?;
            if record.phase != ProviderJournalPhase::Cleanup
                || Self::record_name(&record.operation_id) != name
            {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            self.validate_record_shape(gate, &record, false)?;
        }
        Ok(())
    }

    fn validate_usage(
        &self,
        gate: &ProviderTransactionGate,
        additional_blob_bytes: usize,
        additional_record_bytes: usize,
        reserve_record: bool,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut pending = 0_usize;
        let mut files = 0_usize;
        let mut total_bytes = 0_usize;
        for (directory, count_pending, quarantine) in [
            (&self.records, true, false),
            (&self.blobs, false, false),
            (&self.quarantine, false, true),
        ] {
            for entry in directory
                .entries()
                .map_err(|error| ScenarioError::Io(error.to_string()))?
            {
                let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
                files = files
                    .checked_add(1)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                if files > MAX_PROVIDER_JOURNAL_FILES {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
                let valid_name = if count_pending {
                    name.strip_suffix(".json")
                } else if quarantine {
                    name.strip_suffix(".creating")
                } else {
                    name.strip_suffix(".blob")
                        .or_else(|| name.strip_suffix(".creating"))
                }
                .is_some_and(valid_provider_journal_id);
                if !valid_name
                    || !entry
                        .file_type()
                        .map_err(|error| ScenarioError::Io(error.to_string()))?
                        .is_file()
                {
                    return Err(ScenarioError::UnsafeProviderJournal(name));
                }
                let file = open_provider_file_nofollow(directory, &name)
                    .map_err(|error| ScenarioError::UnsafeProviderJournal(error.to_string()))?;
                let metadata = validate_provider_regular_file(&file, &name)
                    .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                let len = usize::try_from(metadata.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?;
                if count_pending {
                    pending = pending
                        .checked_add(1)
                        .ok_or(ScenarioError::ProviderJournalLimit)?;
                    if pending > MAX_PROVIDER_JOURNAL_PENDING {
                        return Err(ScenarioError::ProviderJournalLimit);
                    }
                    if len > MAX_PROVIDER_JOURNAL_RECORD_BYTES {
                        return Err(ScenarioError::UnsafeProviderJournal(name));
                    }
                }
                total_bytes = total_bytes
                    .checked_add(len)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                if total_bytes > MAX_PROVIDER_JOURNAL_BYTES {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
            }
        }
        total_bytes = total_bytes
            .checked_add(additional_blob_bytes)
            .and_then(|total| total.checked_add(additional_record_bytes))
            .ok_or(ScenarioError::ProviderJournalLimit)?;
        if pending + usize::from(reserve_record) > MAX_PROVIDER_JOURNAL_PENDING
            || files
                + usize::from(reserve_record)
                + usize::from(additional_blob_bytes != 0)
                > MAX_PROVIDER_JOURNAL_FILES - 1
            || total_bytes > MAX_PROVIDER_JOURNAL_BYTES
            || additional_blob_bytes > MAX_PROVIDER_JOURNAL_BLOB_BYTES
        {
            return Err(ScenarioError::ProviderJournalLimit);
        }
        Ok(())
    }

    fn load(
        &self,
        gate: &ProviderTransactionGate,
        operation: ProviderJournalOperation,
        operation_binding: &str,
        source_provenance: &str,
        tree: ProviderTree,
        from_path: &str,
        to_path: Option<&str>,
    ) -> Result<Option<ProviderJournalRecord>, ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut found = None;
        for (directory, completed, limit) in [
            (&self.records, false, MAX_PROVIDER_JOURNAL_PENDING),
            (&self.completed, true, MAX_PROVIDER_JOURNAL_COMPLETED),
        ] {
            let mut scanned = 0_usize;
            for entry in directory
                .entries()
                .map_err(|error| ScenarioError::Io(error.to_string()))?
            {
                let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
                scanned = scanned
                    .checked_add(1)
                    .ok_or(ScenarioError::ProviderJournalLimit)?;
                if scanned > limit {
                    return Err(ScenarioError::ProviderJournalLimit);
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
                let opened = open_provider_regular_optional(
                    directory,
                    &name,
                    MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                    &name,
                )
                .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
                let record = self.decode_record(&opened.bytes, &name)?;
                self.validate_record_shape(gate, &record, !completed)?;
                if completed && record.phase != ProviderJournalPhase::Cleanup {
                    return Err(ScenarioError::UnsafeProviderJournal(name));
                }
                if record.operation == operation
                    && record.operation_binding == operation_binding
                    && record.source_provenance == source_provenance
                    && record.tree == tree
                    && record.from_path == from_path
                    && record.to_path.as_deref() == to_path
                {
                    if found.replace(record).is_some() {
                        return Err(ScenarioError::UnsafeProviderJournal(
                            operation_binding.into(),
                        ));
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        Ok(found)
    }

    fn load_put_for_binding(
        &self,
        gate: &ProviderTransactionGate,
        operation_binding: &str,
    ) -> Result<Option<ProviderJournalRecord>, ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut found = None;
        let mut scanned = 0_usize;
        for entry in self
            .records
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            scanned = scanned
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if scanned > MAX_PROVIDER_JOURNAL_PENDING {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            if !name.ends_with(".json") {
                return Err(ScenarioError::UnsafeProviderJournal(name));
            }
            let opened = open_provider_regular_optional(
                &self.records,
                &name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            let record = self.decode_record(&opened.bytes, &name)?;
            self.validate_record_shape(gate, &record, true)?;
            if record.operation == ProviderJournalOperation::Put
                && record.operation_binding == operation_binding
            {
                if found.replace(record).is_some() {
                    return Err(ScenarioError::UnsafeProviderJournal(
                        operation_binding.into(),
                    ));
                }
            }
        }
        if found.is_some() {
            return Ok(found);
        }
        let mut completed_scanned = 0_usize;
        for entry in self
            .completed
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            completed_scanned = completed_scanned
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            if completed_scanned > MAX_PROVIDER_JOURNAL_COMPLETED {
                return Err(ScenarioError::ProviderJournalLimit);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ScenarioError::UnsafeProviderJournal("non-UTF-8 entry".into()))?;
            let opened = open_provider_regular_optional(
                &self.completed,
                &name,
                MAX_PROVIDER_JOURNAL_RECORD_BYTES,
                &name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.clone()))?
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.clone()))?;
            let record = self.decode_record(&opened.bytes, &name)?;
            self.validate_record_shape(gate, &record, false)?;
            if record.operation == ProviderJournalOperation::Put
                && record.operation_binding == operation_binding
            {
                if found.replace(record).is_some() {
                    return Err(ScenarioError::UnsafeProviderJournal(
                        operation_binding.into(),
                    ));
                }
            }
        }
        Ok(found)
    }

    fn validate_record(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
    ) -> Result<(), ScenarioError> {
        self.validate_record_shape(gate, record, true)
    }

    fn validate_record_shape(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
        validate_blob: bool,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let expected_operation_id = Self::operation_id(
            record.operation,
            &record.operation_binding,
            &record.source_provenance,
            record.tree,
            &record.from_path,
            record.to_path.as_deref(),
            record.source_len,
            &record.source_digest,
        );
        if record.journal_schema_version != PROVIDER_JOURNAL_SCHEMA_VERSION
            || record.operation_id != expected_operation_id
            || record
                .source_identity
            .as_ref()
            .is_some_and(|identity| !valid_provider_identity_record(identity))
            || record
                .staging_identity
                .as_ref()
                .is_some_and(|identity| !valid_provider_identity_record(identity))
            || record
                .destination_identity
                .as_ref()
                .is_some_and(|identity| !valid_provider_identity_record(identity))
            || record.operation_binding.is_empty()
            || record.operation_binding.len() > MAX_PROVIDER_PATH_BYTES
            || record.source_provenance.is_empty()
            || record.source_provenance.len() > MAX_PROVIDER_PATH_BYTES
            || !valid_provider_user_path(&record.from_path)
            || record
                .to_path
                .as_deref()
                .is_some_and(|path| !valid_provider_user_path(path))
            || record.source_digest.len() != 64
            || !record
                .source_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || record.source_len
                > u64::try_from(MAX_PROVIDER_JOURNAL_BLOB_BYTES).unwrap_or(u64::MAX)
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                record.operation_id.clone(),
            ));
        }
        match record.operation {
            ProviderJournalOperation::Put | ProviderJournalOperation::Rename => {
                let expected_blob = Self::blob_name(&record.operation_id);
                if record.blob_name.as_deref() != Some(expected_blob.as_str())
                    || (record.operation == ProviderJournalOperation::Rename
                        && record.to_path.is_none())
                    || (record.operation == ProviderJournalOperation::Put
                        && record.to_path.is_some())
                    || (record.operation == ProviderJournalOperation::Put
                        && matches!(
                            record.phase,
                            ProviderJournalPhase::RetireIntent
                                | ProviderJournalPhase::Retired
                        ))
                    || (record.operation == ProviderJournalOperation::Rename
                        && record.source_identity.is_none())
                {
                    return Err(ScenarioError::UnsafeProviderJournal(
                        record.operation_id.clone(),
                    ));
                }
                if validate_blob && record.phase != ProviderJournalPhase::Cleanup {
                    let bytes = self.read_blob(gate, record)?;
                    if u64::try_from(bytes.len()).ok() != Some(record.source_len)
                        || provider_digest(&bytes) != record.source_digest
                    {
                        return Err(ScenarioError::UnsafeProviderJournal(expected_blob));
                    }
                }
            }
            ProviderJournalOperation::Remove => {
                if record.blob_name.is_some()
                    || record.to_path.is_some()
                    || record.source_identity.is_none()
                    || matches!(
                        record.phase,
                        ProviderJournalPhase::Staged
                            | ProviderJournalPhase::PublishIntent
                            | ProviderJournalPhase::Published
                    )
                {
                    return Err(ScenarioError::UnsafeProviderJournal(
                        record.operation_id.clone(),
                    ));
                }
            }
        }
        let destination_required = matches!(
            record.phase,
            ProviderJournalPhase::Published
                | ProviderJournalPhase::RetireIntent
                | ProviderJournalPhase::Retired
                | ProviderJournalPhase::Cleanup
        ) && record.operation == ProviderJournalOperation::Rename;
        let put_destination_required = matches!(
            record.phase,
            ProviderJournalPhase::Published
                | ProviderJournalPhase::Cleanup
        ) && record.operation == ProviderJournalOperation::Put;
        let retirement_path_required = record.operation != ProviderJournalOperation::Put
            && matches!(
                record.phase,
                ProviderJournalPhase::RetireIntent
                    | ProviderJournalPhase::Retired
                    | ProviderJournalPhase::Cleanup
            );
        let staging_required = record.operation != ProviderJournalOperation::Remove
            && record.phase != ProviderJournalPhase::Cleanup;
        let staging_identity_required = record.operation != ProviderJournalOperation::Remove
            && matches!(
                record.phase,
                ProviderJournalPhase::Staged
                    | ProviderJournalPhase::PublishIntent
                    | ProviderJournalPhase::Published
                    | ProviderJournalPhase::RetireIntent
                    | ProviderJournalPhase::Retired
            );
        let removal_retirement_identity_allowed =
            record.operation == ProviderJournalOperation::Remove
                && matches!(
                    record.phase,
                    ProviderJournalPhase::RetireIntent | ProviderJournalPhase::Retired
                );
        let any_destination_required = destination_required || put_destination_required;
        let destination_allowed = any_destination_required
            || record.operation != ProviderJournalOperation::Remove
                && record.phase == ProviderJournalPhase::PublishIntent;
        if (any_destination_required && record.destination_identity.is_none())
            || (!destination_allowed && record.destination_identity.is_some())
            || (staging_identity_required && record.staging_identity.is_none())
            || (!staging_identity_required
                && !removal_retirement_identity_allowed
                && record.staging_identity.is_some())
            || staging_required != record.staging_name.is_some()
            || record.staging_name.as_deref().is_some_and(|name| {
                name != Self::expected_staging_name(record)
            })
            || retirement_path_required != record.diagnostic_path.is_some()
            || record.diagnostic_path.as_deref().is_some_and(|path| {
                !path.starts_with(&format!("{PROVIDER_REMOVED_NAMESPACE}/"))
                    || !valid_provider_path(path)
                    || path
                        != format!(
                            "{PROVIDER_REMOVED_NAMESPACE}/retired-{}",
                            record.operation_id
                        )
            })
        {
            return Err(ScenarioError::UnsafeProviderJournal(
                record.operation_id.clone(),
            ));
        }
        Ok(())
    }

    fn read_blob(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
    ) -> Result<Vec<u8>, ScenarioError> {
        self.require_transaction_gate(gate)?;
        let name = record
            .blob_name
            .as_deref()
            .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
        open_provider_regular_optional(
            &self.blobs,
            name,
            MAX_PROVIDER_JOURNAL_BLOB_BYTES,
            name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?
        .map(|opened| opened.bytes)
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(name.into()))
    }

    fn create(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
        blob: Option<&[u8]>,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut signed = record.clone();
        self.sign_record(&mut signed)?;
        let record_bytes =
            serde_json::to_vec(&signed).map_err(|error| ScenarioError::Io(error.to_string()))?;
        if record_bytes.len() > MAX_PROVIDER_JOURNAL_RECORD_BYTES {
            return Err(ScenarioError::ProviderJournalLimit);
        }
        let existing_blob = if let Some(name) = record.blob_name.as_deref() {
            open_provider_regular_optional(
                &self.blobs,
                name,
                MAX_PROVIDER_JOURNAL_BLOB_BYTES,
                name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?
        } else {
            None
        };
        let creating_name = Self::creating_blob_name(&record.operation_id);
        let existing_creating_blob = if record.blob_name.is_some() {
            open_provider_regular_optional(
                &self.blobs,
                &creating_name,
                MAX_PROVIDER_JOURNAL_BLOB_BYTES,
                &creating_name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(creating_name.clone()))?
        } else {
            None
        };
        if existing_blob.is_some() && existing_creating_blob.is_some() {
            return Err(ScenarioError::UnsafeProviderJournal(
                record.operation_id.clone(),
            ));
        }
        self.validate_usage(
            gate,
            if existing_blob.is_some() || existing_creating_blob.is_some() {
                0
            } else {
                blob.map_or(0, <[u8]>::len)
            },
            record_bytes.len(),
            true,
        )?;
        let record_name = Self::record_name(&record.operation_id);
        let provisional_name = format!("{}.update", record.operation_id);
        if let Some(blob) = blob {
            let name = record
                .blob_name
                .as_deref()
                .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
            if existing_blob.is_some() {
                return Err(ScenarioError::UnsafeProviderJournal(name.into()));
            }
            // Crash-closed creation order:
            // 1. sync bounded bytes under an ownerless `.creating` name;
            // 2. sync the authenticated update that binds those exact bytes;
            // 3. promote the bytes to the canonical `.blob` and sync;
            // 4. install and sync the canonical record.
            // Reopen may delete only state (1). States (2) and (3) are
            // authenticated and deterministically finish the two promotions.
            provider_journal_boundary_hook(ProviderJournalBoundary::BeforeBlobDurable)?;
            if let Some(mut existing) = existing_creating_blob {
                validate_local_file_bytes(&mut existing.file, blob, &creating_name)?;
            } else {
                let mut file = create_local_file_exclusive(&self.blobs, &creating_name)?;
                file.write_all(blob)
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                file.sync_all()
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                validate_local_file_bytes(&mut file, blob, &creating_name)?;
                sync_provider_directory(&self.blobs)?;
            }
            provider_journal_boundary_hook(ProviderJournalBoundary::BlobDurable)?;
            let mut provisional = create_local_file_exclusive(&self.records, &provisional_name)?;
            provisional
                .write_all(&record_bytes)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            provisional
                .sync_all()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            validate_local_file_bytes(&mut provisional, &record_bytes, &provisional_name)?;
            sync_provider_directory(&self.records)?;
            provider_journal_boundary_hook(
                ProviderJournalBoundary::CreationRecordDurable,
            )?;
            self.blobs
                .rename(&creating_name, &self.blobs, name)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.blobs)?;
            provider_journal_boundary_hook(ProviderJournalBoundary::BlobInstalled)?;
            self.records
                .rename(&provisional_name, &self.records, &record_name)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.records)?;
        } else {
            let mut file = create_local_file_exclusive(&self.records, &record_name)?;
            file.write_all(&record_bytes)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            validate_local_file_bytes(&mut file, &record_bytes, &record_name)?;
            sync_provider_directory(&self.records)?;
        }
        provider_journal_boundary_hook(ProviderJournalBoundary::RecordDurable)
    }

    fn store(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        self.validate_record(gate, record)?;
        let mut signed = record.clone();
        self.sign_record(&mut signed)?;
        let bytes =
            serde_json::to_vec(&signed).map_err(|error| ScenarioError::Io(error.to_string()))?;
        if bytes.len() > MAX_PROVIDER_JOURNAL_RECORD_BYTES {
            return Err(ScenarioError::ProviderJournalLimit);
        }
        let temporary_name = format!("{}.update", record.operation_id);
        let mut temporary = create_local_file_exclusive(&self.records, &temporary_name)?;
        temporary
            .write_all(&bytes)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        temporary
            .sync_all()
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        validate_local_file_bytes(&mut temporary, &bytes, &temporary_name)?;
        sync_provider_directory(&self.records)?;
        provider_journal_boundary_hook(ProviderJournalBoundary::UpdateDurable)?;
        self.records
            .rename(
                &temporary_name,
                &self.records,
                Self::record_name(&record.operation_id),
            )
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        sync_provider_directory(&self.records)?;
        provider_journal_boundary_hook(ProviderJournalBoundary::UpdateInstalled)
    }

    fn complete(
        &self,
        gate: &ProviderTransactionGate,
        record: &ProviderJournalRecord,
    ) -> Result<(), ScenarioError> {
        self.require_transaction_gate(gate)?;
        let mut cleanup = record.clone();
        cleanup.phase = ProviderJournalPhase::Cleanup;
        cleanup.staging_name = None;
        cleanup.staging_identity = None;
        let record_name = Self::record_name(&record.operation_id);
        if open_provider_regular_optional(
            &self.records,
            &record_name,
            MAX_PROVIDER_JOURNAL_RECORD_BYTES,
            &record_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(record_name.clone()))?
        .is_some()
        {
            self.store(gate, &cleanup)?;
            provider_journal_after_phase_hook(ProviderJournalPhase::Cleanup)?;
        }
        if let Some(blob_name) = record.blob_name.as_deref() {
            if open_provider_regular_optional(
                &self.blobs,
                blob_name,
                MAX_PROVIDER_JOURNAL_BLOB_BYTES,
                blob_name,
            )
            .map_err(|_| ScenarioError::UnsafeProviderJournal(blob_name.into()))?
            .is_some()
            {
                self.blobs
                    .remove_file(blob_name)
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
            }
            sync_provider_directory(&self.blobs)?;
            provider_journal_boundary_hook(ProviderJournalBoundary::BlobRemoved)?;
        }
        let mut signed = cleanup;
        self.sign_record(&mut signed)?;
        let completion_bytes =
            serde_json::to_vec(&signed).map_err(|error| ScenarioError::Io(error.to_string()))?;
        let completion_name = Self::record_name(&record.operation_id);
        if let Some(mut completed) = open_provider_regular_optional(
            &self.completed,
            &completion_name,
            MAX_PROVIDER_JOURNAL_RECORD_BYTES,
            &completion_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(completion_name.clone()))?
        {
            validate_local_file_bytes(
                &mut completed.file,
                &completion_bytes,
                &completion_name,
            )?;
        } else {
            self.validate_completed_usage(gate, 1, completion_bytes.len())?;
            let update_name = format!("{}.update", record.operation_id);
            let mut update = create_local_file_exclusive(&self.completed, &update_name)?;
            update
                .write_all(&completion_bytes)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            update
                .sync_all()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            validate_local_file_bytes(
                &mut update,
                &completion_bytes,
                &update_name,
            )?;
            sync_provider_directory(&self.completed)?;
            self.completed
                .rename(&update_name, &self.completed, &completion_name)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.completed)?;
        }
        provider_journal_boundary_hook(ProviderJournalBoundary::CompletionDurable)?;
        if open_provider_regular_optional(
            &self.records,
            &record_name,
            MAX_PROVIDER_JOURNAL_RECORD_BYTES,
            &record_name,
        )
        .map_err(|_| ScenarioError::UnsafeProviderJournal(record_name.clone()))?
        .is_some()
        {
            self.records
                .remove_file(&record_name)
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_directory(&self.records)?;
        }
        provider_journal_boundary_hook(ProviderJournalBoundary::RecordRemoved)?;
        Ok(())
    }
}

struct DeviceRuntime {
    name: String,
    root: PathBuf,
    store: Option<ObjectStore>,
    engine: Option<ShardedHotEngine>,
    transfers: BTreeMap<String, Transfer>,
    provider: ProviderRuntime,
    provider_journal: Option<ProviderRetryJournal>,
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
        let provider = ProviderRuntime::open(root.join("provider"))?;
        let provider_journal = ProviderRetryJournal::open(root.join("provider-local-journal"))?;
        Ok(Self {
            name: identity.name.clone(),
            root,
            store: Some(store),
            engine: Some(engine),
            transfers: BTreeMap::new(),
            provider,
            provider_journal: Some(provider_journal),
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
        // A Tine process crash drops every process-owned engine, store, and
        // open transfer handle. The provider is a separate disk-visible
        // system: its bytes, partition state, and abandoned `.part` files
        // intentionally survive. The bounded device-local retry journal is
        // durable simulator state and is deliberately retained across this
        // process restart.
        self.engine.take();
        self.store.take();
        self.transfers.clear();
        self.provider.writes.clear();
        self.provider_journal.take();
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
        self.provider_journal =
            Some(ProviderRetryJournal::open(self.root.join("provider-local-journal"))?);
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
    provider_receipts: BTreeMap<(u64, String), IngressReceipt>,
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
        for (ordinal, identity) in scenario.devices.iter().enumerate() {
            // Device names belong to the scenario vocabulary only. Keep them
            // as map keys, but never let them select a host-path component.
            let runtime = DeviceRuntime::open(
                root.0.join(format!("device-{ordinal:04}")),
                identity,
                &scenario.workspace,
            )?;
            devices.insert(identity.name.clone(), runtime);
        }
        let mut simulator = Self {
            scenario,
            root,
            devices,
            mailbox,
            outcomes: Vec::new(),
            receipts: BTreeMap::new(),
            provider_receipts: BTreeMap::new(),
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

    pub fn provider_ingress_receipts(&self) -> &BTreeMap<(u64, String), IngressReceipt> {
        &self.provider_receipts
    }

    pub fn provider_snapshots(&self) -> Result<Vec<ProviderTreeSnapshot>, ScenarioError> {
        self.scenario
            .devices
            .iter()
            .map(|device| self.device(&device.name)?.provider.snapshot(&device.name))
            .collect()
    }

    /// Physical simulator scratch path exposed only so integration tests can
    /// independently inspect and adversarially mutate provider bytes.
    pub fn provider_tree_path(
        &self,
        device: &str,
        tree: ProviderTree,
    ) -> Result<PathBuf, ScenarioError> {
        Ok(self.device(device)?.provider.tree_path(tree))
    }

    /// Device-local retry state is deliberately outside the untrusted provider
    /// tree. This path is exposed for adversarial corruption and substitution
    /// tests; production operations retain directory capabilities instead.
    pub fn provider_journal_path(&self, device: &str) -> Result<PathBuf, ScenarioError> {
        self.device(device)?
            .provider_journal
            .as_ref()
            .map(|journal| journal.root.clone())
            .ok_or_else(|| ScenarioError::DeviceCrashed(device.into()))
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
            ScheduledActionKind::ProviderCopy {
                source,
                destination,
            } => {
                let gates =
                    self.acquire_provider_transaction_gates(source, &destination.device)?;
                let gate = gates.get(&destination.device).ok_or_else(|| {
                    ScenarioError::UnsafeProviderJournal(
                        "destination provider transaction gate was lost".into(),
                    )
                })?;
                let source_gate = match source {
                    ProviderSource::Mailbox { .. } => ProviderSourceTransactionGate::Mailbox,
                    ProviderSource::Tree { location } => {
                        ProviderSourceTransactionGate::Tree(
                            gates.get(&location.device).ok_or_else(|| {
                                ScenarioError::UnsafeProviderJournal(
                                    "source provider transaction gate was lost".into(),
                                )
                            })?,
                        )
                    }
                };
                let item = self.provider_source(source, source_gate)?;
                if self.device(&destination.device)?.provider.partitioned {
                    return Err(ScenarioError::ProviderPartitioned(
                        destination.device.clone(),
                    ));
                }
                let operation_binding =
                    format!("event:{event_id}:{}", item.source_binding);
                let runtime = self.device_mut(&destination.device)?;
                let journal = runtime
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
                runtime.provider.put_complete(
                    journal,
                    &gate,
                    &operation_binding,
                    &item.source_binding,
                    destination,
                    &item.bytes,
                    item.source_identity,
                    None,
                )
            }
            ScheduledActionKind::BeginProviderWrite {
                source,
                destination,
                transfer_id,
            } => {
                let gates =
                    self.acquire_provider_transaction_gates(source, &destination.device)?;
                let gate = gates.get(&destination.device).ok_or_else(|| {
                    ScenarioError::UnsafeProviderJournal(
                        "destination provider transaction gate was lost".into(),
                    )
                })?;
                let source_gate = match source {
                    ProviderSource::Mailbox { .. } => ProviderSourceTransactionGate::Mailbox,
                    ProviderSource::Tree { location } => {
                        ProviderSourceTransactionGate::Tree(
                            gates.get(&location.device).ok_or_else(|| {
                                ScenarioError::UnsafeProviderJournal(
                                    "source provider transaction gate was lost".into(),
                                )
                            })?,
                        )
                    }
                };
                let item = self.provider_source(source, source_gate)?;
                let runtime = self.device_mut(&destination.device)?;
                let journal = runtime
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
                journal.require_transaction_gate(&gate)?;
                if runtime.provider.partitioned
                    || runtime.provider.writes.len() >= MAX_TRANSFERS_PER_DEVICE
                    || runtime.provider.writes.contains_key(transfer_id)
                {
                    return Err(ScenarioError::InvalidTransfer(transfer_id.clone()));
                }
                reject_provider_temporary_path(&destination.path)?;
                if !valid_name(transfer_id, 128) {
                    return Err(ScenarioError::InvalidTransfer(transfer_id.clone()));
                }
                let temporary_dir = open_provider_directory(
                    runtime.provider.tree(destination.tree),
                    PROVIDER_TEMP_NAMESPACE,
                )?;
                let temporary_name = format!("{transfer_id}.part");
                let file = create_provider_file_exclusive(
                    &temporary_dir,
                    &temporary_name,
                    &destination.path,
                )?;
                runtime.provider.writes.insert(
                    transfer_id.clone(),
                    ProviderWrite {
                        destination: destination.clone(),
                        source: item.bytes,
                        source_provenance: item.source_binding,
                        next: 0,
                        file,
                    },
                );
                Ok(())
            }
            ScheduledActionKind::AppendProviderWrite {
                device,
                transfer_id,
                len,
            } => {
                let gate = self
                    .device(device)?
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(device.clone()))?
                    .acquire_transaction_gate()?;
                let runtime = self.device_mut(device)?;
                let journal = runtime
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
                journal.require_transaction_gate(&gate)?;
                if runtime.provider.partitioned {
                    return Err(ScenarioError::ProviderPartitioned(device.clone()));
                }
                let write = runtime
                    .provider
                    .writes
                    .get_mut(transfer_id)
                    .ok_or_else(|| ScenarioError::UnknownTransfer(transfer_id.clone()))?;
                let end = write
                    .next
                    .checked_add(*len)
                    .filter(|end| *end <= write.source.len())
                    .ok_or_else(|| ScenarioError::InvalidTransfer(transfer_id.clone()))?;
                if end > MAX_TRANSFER_BYTES {
                    return Err(ScenarioError::InvalidTransfer(transfer_id.clone()));
                }
                write
                    .file
                    .seek(SeekFrom::Start(u64::try_from(write.next).map_err(
                        |_| ScenarioError::InvalidTransfer(transfer_id.clone()),
                    )?))
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                write
                    .file
                    .write_all(&write.source[write.next..end])
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                write.next = end;
                Ok(())
            }
            ScheduledActionKind::FinishProviderWrite {
                device,
                transfer_id,
            } => {
                let gate = self
                    .device(device)?
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(device.clone()))?
                    .acquire_transaction_gate()?;
                provider_finish_after_gate_hook();
                let runtime = self.device_mut(device)?;
                if runtime.provider.partitioned {
                    return Err(ScenarioError::ProviderPartitioned(device.clone()));
                }
                let operation_binding = format!("transfer:{transfer_id}");
                let journal = runtime
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
                let (
                    destination,
                    bytes,
                    source_provenance,
                    source_identity,
                    staging_name_candidate,
                ) =
                    if let Some(write) = runtime.provider.writes.get_mut(transfer_id) {
                    if write.next != write.source.len() {
                        return Err(ScenarioError::PartialProviderWrite(transfer_id.clone()));
                    }
                    write
                        .file
                        .sync_all()
                        .map_err(|error| ScenarioError::Io(error.to_string()))?;
                    validate_provider_file_bytes(
                        &mut write.file,
                        &write.source,
                        &write.destination.path,
                    )?;
                        (
                            write.destination.clone(),
                            Arc::clone(&write.source),
                            write.source_provenance.clone(),
                            Some(provider_identity_record(provider_file_identity(
                                &write.file.file,
                            )?)),
                            write.file.name.clone(),
                        )
                    } else {
                        let record = journal
                            .load_put_for_binding(&gate, &operation_binding)?
                            .ok_or_else(|| ScenarioError::UnknownTransfer(transfer_id.clone()))?;
                        let destination = ProviderLocation {
                            device: device.clone(),
                            tree: record.tree,
                            path: record.from_path.clone(),
                        };
                        let bytes = if record.phase == ProviderJournalPhase::Cleanup {
                            let (parent, name) = runtime.provider.parent_and_name(
                                record.tree,
                                &record.from_path,
                                false,
                            )?;
                            Arc::from(
                                open_provider_regular_optional(
                                    &parent,
                                    &name,
                                    MAX_PROVIDER_RESCAN_BYTES,
                                    &record.from_path,
                                )?
                                .ok_or_else(|| {
                                    ScenarioError::UnsafeProviderEntry(record.from_path.clone())
                                })?
                                .bytes,
                            )
                        } else {
                            Arc::from(journal.read_blob(&gate, &record)?)
                        };
                        (
                            destination,
                            bytes,
                            record.source_provenance,
                            record.source_identity,
                            None,
                        )
                    };
                let initial_staging_name = if let (Some(name), Some(identity)) = (
                    staging_name_candidate,
                    source_identity.as_ref(),
                ) {
                    let staging = open_provider_directory(
                        runtime.provider.tree(destination.tree),
                        PROVIDER_TEMP_NAMESPACE,
                    )?;
                    match open_provider_regular_optional(
                        &staging,
                        &name,
                        MAX_PROVIDER_RESCAN_BYTES,
                        &format!("{PROVIDER_TEMP_NAMESPACE}/{name}"),
                    )? {
                        Some(current)
                            if provider_file_matches_identity(&current.file, identity)? =>
                        {
                            Some(name)
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                runtime.provider.put_complete(
                    journal,
                    &gate,
                    &operation_binding,
                    &source_provenance,
                    &destination,
                    &bytes,
                    source_identity,
                    initial_staging_name,
                )?;
                runtime.provider.writes.remove(transfer_id);
                Ok(())
            }
            ScheduledActionKind::ProviderRename {
                device,
                tree,
                from_path,
                to_path,
            } => {
                let runtime = self.device(device)?;
                run_provider_rename(runtime, event_id, *tree, from_path, to_path)
            }
            ScheduledActionKind::ProviderRemove { location } => {
                let runtime = self.device(&location.device)?;
                run_provider_remove(runtime, event_id, location.tree, &location.path)
            }
            ScheduledActionKind::SetProviderPartition {
                device,
                partitioned,
            } => {
                self.device_mut(device)?.provider.partitioned = *partitioned;
                Ok(())
            }
            ScheduledActionKind::ReceiverRescan { device } => {
                self.receiver_rescan(event_id, device)
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

    fn acquire_provider_transaction_gates(
        &self,
        source: &ProviderSource,
        destination_device: &str,
    ) -> Result<BTreeMap<String, ProviderTransactionGate>, ScenarioError> {
        provider_transaction_device_names(source, destination_device)
            .into_iter()
            .map(|device| {
                let gate = self
                    .device(&device)?
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(device.clone()))?
                    .acquire_transaction_gate()?;
                Ok((device, gate))
            })
            .collect()
    }

    fn provider_source(
        &self,
        source: &ProviderSource,
        gate: ProviderSourceTransactionGate<'_>,
    ) -> Result<ResolvedProviderItem, ScenarioError> {
        match (source, gate) {
            (
                ProviderSource::Mailbox { item_id },
                ProviderSourceTransactionGate::Mailbox,
            ) => {
                let item = self.mailbox.item(item_id)?;
                Ok(ResolvedProviderItem {
                    bytes: Arc::clone(&item.bytes),
                    source_binding: format!("mailbox:{item_id}"),
                    source_identity: None,
                })
            }
            (
                ProviderSource::Tree { location },
                ProviderSourceTransactionGate::Tree(gate),
            ) => {
                let runtime = self.device(&location.device)?;
                let journal = runtime
                    .provider_journal
                    .as_ref()
                    .ok_or_else(|| ScenarioError::DeviceCrashed(location.device.clone()))?;
                journal.require_transaction_gate(gate)?;
                provider_source_inspection_visit();
                reject_provider_temporary_path(&location.path)?;
                let _kind = provider_item_kind(&location.path)
                    .ok_or_else(|| ScenarioError::UnknownProviderPath(location.path.clone()))?;
                let (parent, name) =
                    runtime
                        .provider
                        .parent_and_name(location.tree, &location.path, false)?;
                let opened = open_provider_regular_optional(
                    &parent,
                    &name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    &location.path,
                )?
                .ok_or_else(|| ScenarioError::UnknownProviderPath(location.path.clone()))?;
                Ok(ResolvedProviderItem {
                    bytes: Arc::from(opened.bytes),
                    source_binding: format!(
                        "provider:{}:{}:{}",
                        location.device,
                        provider_tree_name(location.tree),
                        location.path
                    ),
                    source_identity: Some(provider_identity_record(provider_file_identity(
                        &opened.file,
                    )?)),
                })
            }
            _ => Err(ScenarioError::UnsafeProviderJournal(
                "wrong provider source transaction gate".into(),
            )),
        }
    }

    fn receiver_rescan(&mut self, event_id: u64, device: &str) -> Result<(), ScenarioError> {
        let files = {
            let runtime = self.device(device)?;
            if runtime.provider.partitioned {
                return Ok(());
            }
            bounded_provider_files(
                runtime.provider.tree(ProviderTree::Inbox),
                false,
                MAX_PROVIDER_RESCAN_ENTRIES,
                MAX_PROVIDER_RESCAN_BYTES,
            )?
        };
        let mut manifests = BTreeSet::new();
        for file in files {
            let Some(kind) = provider_item_kind(&file.path) else {
                continue;
            };
            let item_id = format!("provider/inbox/{}", file.path);
            let result = match kind {
                ProviderItemKind::Object => self
                    .device(device)?
                    .store()?
                    .stage_object_bytes(&file.bytes)
                    .map(|_| None),
                ProviderItemKind::Manifest => self
                    .device(device)?
                    .store()?
                    .stage_manifest_bytes(&file.bytes)
                    .map(Some),
            };
            let receipt = IngressReceipt {
                event_id,
                device: device.into(),
                item_id: item_id.clone(),
                item_kind: kind,
                byte_len: file.bytes.len(),
                accepted: result.is_ok(),
                error: result.as_ref().err().map(ToString::to_string),
            };
            self.provider_receipts
                .insert((event_id, file.path.clone()), receipt);
            match result {
                Ok(Some(batch_id)) => {
                    manifests.insert(batch_id);
                }
                Ok(None) => {}
                Err(error) => return Err(ScenarioError::Store(error.to_string())),
            }
        }
        for batch_id in manifests {
            let outcome = self
                .device_mut(device)?
                .engine_mut()?
                .stage_archive_batch(batch_id)
                .map_err(|error| ScenarioError::Engine(error.to_string()))?;
            self.outcomes.push(outcome);
        }
        Ok(())
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
            .prepare_bootstrap_transaction(
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
            InvariantAssertion::ProviderResidue {
                device,
                max_entries,
                max_bytes,
            } => {
                let snapshot = self.device(device)?.provider.snapshot(device)?;
                let bytes = snapshot
                    .entries
                    .iter()
                    .try_fold(0_usize, |total, entry| total.checked_add(entry.byte_len));
                if snapshot.entries.len() <= *max_entries
                    && bytes.is_some_and(|bytes| bytes <= *max_bytes)
                {
                    Ok(())
                } else {
                    Err(self.invariant(
                        event_id,
                        InvariantPredicate::ProviderResidue,
                        vec![device.clone()],
                        Vec::new(),
                        "provider residue exceeded assertion bound",
                    ))
                }
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

fn provider_transaction_device_names(
    source: &ProviderSource,
    destination_device: &str,
) -> Vec<String> {
    let mut devices = BTreeSet::from([destination_device.to_owned()]);
    if let ProviderSource::Tree { location } = source {
        devices.insert(location.device.clone());
    }
    devices.into_iter().collect()
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
        | ScheduledActionKind::AppendProviderWrite { device, .. }
        | ScheduledActionKind::FinishProviderWrite { device, .. }
        | ScheduledActionKind::ProviderRename { device, .. }
        | ScheduledActionKind::SetProviderPartition { device, .. }
        | ScheduledActionKind::ReceiverRescan { device }
            if !known(device) =>
        {
            return Err(ScenarioError::UnknownDeviceName(device.clone()));
        }
        ScheduledActionKind::AssertInvariant { assertion } => match assertion {
            InvariantAssertion::Replica { device, .. }
            | InvariantAssertion::NoVisibleEffect { device, .. }
            | InvariantAssertion::LineageIsolation { device, .. }
            | InvariantAssertion::RestartReplay { device }
            | InvariantAssertion::ProviderResidue { device, .. }
                if !known(device) =>
            {
                return Err(ScenarioError::UnknownDeviceName(device.clone()));
            }
            InvariantAssertion::Converged { devices }
                if devices.is_empty() || devices.iter().any(|device| !known(device)) =>
            {
                return Err(ScenarioError::InvalidInvariant);
            }
            _ => {}
        },
        ScheduledActionKind::LegacyDeliver { device, .. } if !known(device) => {
            return Err(ScenarioError::UnknownDeviceName(device.clone()));
        }
        ScheduledActionKind::LegacyAssertConverged { devices }
            if devices.is_empty() || devices.iter().any(|device| !known(device)) =>
        {
            return Err(ScenarioError::InvalidInvariant);
        }
        ScheduledActionKind::ProviderCopy {
            source,
            destination,
        }
        | ScheduledActionKind::BeginProviderWrite {
            source,
            destination,
            ..
        } => {
            validate_provider_source(source, names)?;
            validate_provider_location(destination, names)?;
        }
        ScheduledActionKind::ProviderRemove { location } => {
            validate_provider_location(location, names)?
        }
        ScheduledActionKind::ProviderRename {
            from_path, to_path, ..
        } if !valid_provider_user_path(from_path) || !valid_provider_user_path(to_path) => {
            return Err(ScenarioError::InvalidProviderPath(format!(
                "{from_path} -> {to_path}"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn validate_provider_location(
    location: &ProviderLocation,
    names: &BTreeSet<String>,
) -> Result<(), ScenarioError> {
    if !names.contains(&location.device) {
        return Err(ScenarioError::UnknownDeviceName(location.device.clone()));
    }
    if !valid_provider_user_path(&location.path) {
        return Err(ScenarioError::InvalidProviderPath(location.path.clone()));
    }
    Ok(())
}

fn validate_provider_source(
    source: &ProviderSource,
    names: &BTreeSet<String>,
) -> Result<(), ScenarioError> {
    match source {
        ProviderSource::Mailbox { item_id } if valid_name(item_id, 256) => Ok(()),
        ProviderSource::Tree { location } => validate_provider_location(location, names),
        ProviderSource::Mailbox { item_id } => Err(ScenarioError::InvalidWireItem(item_id.clone())),
    }
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
        ScheduledActionKind::ProviderCopy {
            source: ProviderSource::Mailbox { item_id },
            ..
        }
        | ScheduledActionKind::BeginProviderWrite {
            source: ProviderSource::Mailbox { item_id },
            ..
        } => !dynamic_item_batch(item_id).is_some_and(|batch| !authored.contains(&batch)),
        ScheduledActionKind::ProbeBatch { batch_id, .. } => {
            authored.contains(batch_id) || !dynamic_batch_id(*batch_id)
        }
        _ => true,
    });
    let mut transfers = BTreeSet::new();
    let mut provider_transfers = BTreeMap::new();
    let mut provider_files = BTreeSet::new();
    let mut partitioned = BTreeSet::new();
    let mut crashed = BTreeSet::new();
    actions.retain(|action| match &action.action {
        ScheduledActionKind::BeginTransfer {
            device,
            transfer_id,
            ..
        } => {
            transfers.insert((device.clone(), transfer_id.clone()));
            true
        }
        ScheduledActionKind::AppendTransfer {
            device,
            transfer_id,
            ..
        } => transfers.contains(&(device.clone(), transfer_id.clone())),
        ScheduledActionKind::CommitTransfer {
            device,
            transfer_id,
            ..
        }
        | ScheduledActionKind::AbortTransfer {
            device,
            transfer_id,
        } => transfers.remove(&(device.clone(), transfer_id.clone())),
        ScheduledActionKind::ProviderCopy {
            source,
            destination,
        } => {
            if !provider_source_is_available(source, &provider_files) {
                return false;
            }
            if !partitioned.contains(&destination.device) {
                provider_files.insert(destination.clone());
            }
            true
        }
        ScheduledActionKind::BeginProviderWrite {
            source,
            destination,
            transfer_id,
        } => {
            if !provider_source_is_available(source, &provider_files) {
                return false;
            }
            if partitioned.contains(&destination.device) {
                return true;
            }
            provider_transfers.insert(
                (destination.device.clone(), transfer_id.clone()),
                destination.clone(),
            );
            true
        }
        ScheduledActionKind::AppendProviderWrite {
            device,
            transfer_id,
            ..
        } => provider_transfers.contains_key(&(device.clone(), transfer_id.clone())),
        ScheduledActionKind::FinishProviderWrite {
            device,
            transfer_id,
        } => {
            if partitioned.contains(device) {
                return provider_transfers.contains_key(&(device.clone(), transfer_id.clone()));
            }
            // A surviving finish action is not evidence of publication.  It
            // can still fail because the append was partial, sync failed, the
            // temporary name was replaced, or the destination collided.  The
            // reducer cannot know the original byte extent here, so retain the
            // transfer dependency but never invent a tree file for later
            // actions to consume.
            provider_transfers.contains_key(&(device.clone(), transfer_id.clone()))
        }
        ScheduledActionKind::ProviderRename {
            device,
            tree,
            from_path,
            to_path,
        } => {
            let from = ProviderLocation {
                device: device.clone(),
                tree: *tree,
                path: from_path.clone(),
            };
            if !provider_files.remove(&from) {
                return false;
            }
            provider_files.insert(ProviderLocation {
                device: device.clone(),
                tree: *tree,
                path: to_path.clone(),
            });
            true
        }
        ScheduledActionKind::ProviderRemove { location } => provider_files.remove(location),
        ScheduledActionKind::SetProviderPartition {
            device,
            partitioned: is_partitioned,
        } => {
            if *is_partitioned {
                partitioned.insert(device.clone());
            } else {
                partitioned.remove(device);
            }
            true
        }
        ScheduledActionKind::Crash { device } => {
            transfers.retain(|(transfer_device, _)| transfer_device != device);
            provider_transfers.retain(|(transfer_device, _), _| transfer_device != device);
            crashed.insert(device.clone());
            true
        }
        ScheduledActionKind::Restart { device } => crashed.contains(device),
        _ => true,
    });
}

fn provider_source_is_available(
    source: &ProviderSource,
    provider_files: &BTreeSet<ProviderLocation>,
) -> bool {
    match source {
        ProviderSource::Mailbox { .. } => true,
        ProviderSource::Tree { location } => provider_files.contains(location),
    }
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

/// Scenario device names are display identifiers, not filesystem names. The
/// check deliberately recognizes Windows syntax even when the simulator runs
/// on Unix, so a serialized trace means the same thing everywhere.
fn valid_scenario_device_name(value: &str) -> bool {
    if !valid_name(value, 128)
        || matches!(value, "." | "..")
        || value.ends_with(' ')
        || value.ends_with('.')
        || value.chars().any(|character| {
            matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        })
    {
        return false;
    }

    let mut components = Path::new(value).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return false;
    }

    let base = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(
        base.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Data-safe provider rename semantics: publish an independent destination
/// inode/file from the bounded local blob, durably validate its recorded
/// identity, then move the validated source into diagnostic `removed/`.
/// Unix retirement uses an atomic exchange with a single-link placeholder so
/// a racing replacement is preserved only as diagnostic residue. Windows
/// retirement is handle-bound. No provider-visible residue authorizes retry.
fn run_provider_rename(
    runtime: &DeviceRuntime,
    event_id: u64,
    tree: ProviderTree,
    from_path: &str,
    to_path: &str,
) -> Result<(), ScenarioError> {
    let journal = runtime
        .provider_journal
        .as_ref()
        .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
    let gate = journal.acquire_transaction_gate()?;
    reject_provider_temporary_path(from_path)?;
    reject_provider_temporary_path(to_path)?;
    let (from_dir, from_name) = runtime.provider.parent_and_name(tree, from_path, false)?;
    if from_path == to_path {
        open_provider_regular_optional(
            &from_dir,
            &from_name,
            MAX_PROVIDER_RESCAN_BYTES,
            from_path,
        )?
        .ok_or_else(|| ScenarioError::UnknownProviderPath(from_path.into()))?;
        return Ok(());
    }
    let (to_dir, to_name) = runtime.provider.parent_and_name(tree, to_path, true)?;
    let operation_binding = format!(
        "event:{event_id}:rename:{}:{from_path}:{to_path}",
        provider_tree_name(tree)
    );
    let source_provenance = format!(
        "provider:{}:{}:{from_path}",
        runtime.name,
        provider_tree_name(tree)
    );
    let mut record = match journal.load(
        &gate,
        ProviderJournalOperation::Rename,
        &operation_binding,
        &source_provenance,
        tree,
        from_path,
        Some(to_path),
    )? {
        Some(record) => record,
        None => {
            let source = open_provider_regular_optional(
                &from_dir,
                &from_name,
                MAX_PROVIDER_RESCAN_BYTES,
                from_path,
            )?
            .ok_or_else(|| ScenarioError::UnknownProviderPath(from_path.into()))?;
            let operation_id = ProviderRetryJournal::operation_id(
                ProviderJournalOperation::Rename,
                &operation_binding,
                &source_provenance,
                tree,
                from_path,
                Some(to_path),
                u64::try_from(source.bytes.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                &provider_digest(&source.bytes),
            );
            let record = ProviderJournalRecord {
                journal_schema_version: PROVIDER_JOURNAL_SCHEMA_VERSION,
                operation_id: operation_id.clone(),
                operation: ProviderJournalOperation::Rename,
                operation_binding: operation_binding.clone(),
                source_provenance: source_provenance.clone(),
                tree,
                from_path: from_path.into(),
                to_path: Some(to_path.into()),
                source_identity: Some(provider_identity_record(provider_file_identity(
                    &source.file,
                )?)),
                source_len: u64::try_from(source.bytes.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                source_digest: provider_digest(&source.bytes),
                blob_name: Some(ProviderRetryJournal::blob_name(&operation_id)),
                phase: ProviderJournalPhase::Prepared,
                staging_identity: None,
                destination_identity: None,
                staging_name: Some(ProviderRetryJournal::staging_name(&operation_id, 0)),
                staging_generation: 0,
                diagnostic_path: None,
                authentication_tag: String::new(),
            };
            journal.create(&gate, &record, Some(&source.bytes))?;
            provider_journal_after_phase_hook(ProviderJournalPhase::Prepared)?;
            provider_post_validation_hook(ProviderPostValidationOperation::Rename);
            record
        }
    };
    let removed =
        open_provider_directory(runtime.provider.tree(tree), PROVIDER_REMOVED_NAMESPACE)?;
    let retirement_evidence = open_provider_directory(
        runtime.provider.tree(tree),
        PROVIDER_RENAME_EVIDENCE_NAMESPACE,
    )?;
    let expected = if record.phase == ProviderJournalPhase::Cleanup {
        open_provider_regular_optional(&to_dir, &to_name, MAX_PROVIDER_RESCAN_BYTES, to_path)?
            .ok_or_else(|| ScenarioError::UnsafeProviderEntry(to_path.into()))?
            .bytes
    } else {
        journal.read_blob(&gate, &record)?
    };
    if record.phase == ProviderJournalPhase::Cleanup {
        validate_journal_destination(
            journal,
            &gate,
            &runtime.provider,
            &record,
            &expected,
            &removed,
        )?;
        validate_retired_source(&runtime.provider, &record)?;
        return journal.complete(&gate, &record);
    }
    let temporary_dir =
        open_provider_directory(runtime.provider.tree(tree), PROVIDER_TEMP_NAMESPACE)?;
    if record.phase == ProviderJournalPhase::Prepared {
        if open_provider_regular_optional(
            &to_dir,
            &to_name,
            MAX_PROVIDER_RESCAN_BYTES,
            to_path,
        )?
        .is_some()
        {
            return Err(ScenarioError::ProviderConflictingBytes(to_path.into()));
        }
        loop {
            let staging_name = record.staging_name.as_deref().ok_or_else(|| {
                ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
            })?;
            if open_provider_regular_optional(
                &temporary_dir,
                staging_name,
                MAX_PROVIDER_RESCAN_BYTES,
                staging_name,
            )?
            .is_none()
            {
                break;
            }
            quarantine_unowned_staging(
                journal,
                &gate,
                &temporary_dir,
                staging_name,
                runtime.provider.tree(tree),
                &record.operation_id,
                record.staging_generation,
            )?;
            record.staging_generation = record
                .staging_generation
                .checked_add(1)
                .ok_or(ScenarioError::ProviderJournalLimit)?;
            record.staging_name = Some(ProviderRetryJournal::staging_name(
                &record.operation_id,
                record.staging_generation,
            ));
            journal.store(&gate, &record)?;
        }
        let staging_name = record.staging_name.as_deref().ok_or_else(|| {
            ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
        })?;
        let mut staged =
            create_provider_journal_staging(&temporary_dir, staging_name, to_path)?;
        staged
            .write_all(&expected)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        staged
            .sync_all()
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        validate_provider_file_bytes(&mut staged, &expected, to_path)?;
        record.staging_identity =
            Some(provider_identity_record(provider_file_identity(&staged.file)?));
        record.phase = ProviderJournalPhase::Staged;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::Staged)?;
    }

    if record.phase == ProviderJournalPhase::Staged {
        validate_journal_staging(&temporary_dir, &record, &expected, to_path)?;
        record.phase = ProviderJournalPhase::PublishIntent;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::PublishIntent)?;
    }
    if record.phase == ProviderJournalPhase::PublishIntent {
        publish_journal_destination(
            journal,
            &gate,
            &mut record,
            &temporary_dir,
            runtime.provider.tree(tree),
            &to_dir,
            &to_name,
            &expected,
            to_path,
        )?;
        sync_provider_publication_directories(&to_dir, Some(&temporary_dir))?;
        record.phase = ProviderJournalPhase::Published;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::Published)?;
    }

    validate_journal_destination(
        journal,
        &gate,
        &runtime.provider,
        &record,
        &expected,
        &removed,
    )?;
    if record.phase == ProviderJournalPhase::Published {
        record.diagnostic_path = Some(format!(
            "{PROVIDER_REMOVED_NAMESPACE}/retired-{}",
            record.operation_id
        ));
        record.phase = ProviderJournalPhase::RetireIntent;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::RetireIntent)?;
    }
    if record.phase == ProviderJournalPhase::RetireIntent {
        reconcile_provider_retirement(
            journal,
            &gate,
            &from_dir,
            &from_name,
            &removed,
            &retirement_evidence,
            from_path,
            &mut record,
        )?;
        provider_rename_after_move_hook()?;
        record.phase = ProviderJournalPhase::Retired;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::Retired)?;
    }
    validate_retired_source(&runtime.provider, &record)?;
    journal.complete(&gate, &record)
}

fn run_provider_remove(
    runtime: &DeviceRuntime,
    event_id: u64,
    tree: ProviderTree,
    path: &str,
) -> Result<(), ScenarioError> {
    let journal = runtime
        .provider_journal
        .as_ref()
        .ok_or_else(|| ScenarioError::DeviceCrashed(runtime.name.clone()))?;
    let gate = journal.acquire_transaction_gate()?;
    reject_provider_temporary_path(path)?;
    let (parent, name) = runtime.provider.parent_and_name(tree, path, false)?;
    let operation_binding =
        format!("event:{event_id}:remove:{}:{path}", provider_tree_name(tree));
    let source_provenance = format!(
        "provider:{}:{}:{path}",
        runtime.name,
        provider_tree_name(tree)
    );
    let mut record = match journal.load(
        &gate,
        ProviderJournalOperation::Remove,
        &operation_binding,
        &source_provenance,
        tree,
        path,
        None,
    )? {
        Some(record) => record,
        None => {
            let source = open_provider_regular_optional(
                &parent,
                &name,
                MAX_PROVIDER_RESCAN_BYTES,
                path,
            )?
            .ok_or_else(|| ScenarioError::UnknownProviderPath(path.into()))?;
            let operation_id = ProviderRetryJournal::operation_id(
                ProviderJournalOperation::Remove,
                &operation_binding,
                &source_provenance,
                tree,
                path,
                None,
                u64::try_from(source.bytes.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                &provider_digest(&source.bytes),
            );
            let record = ProviderJournalRecord {
                journal_schema_version: PROVIDER_JOURNAL_SCHEMA_VERSION,
                operation_id: operation_id.clone(),
                operation: ProviderJournalOperation::Remove,
                operation_binding: operation_binding.clone(),
                source_provenance: source_provenance.clone(),
                tree,
                from_path: path.into(),
                to_path: None,
                source_identity: Some(provider_identity_record(provider_file_identity(
                    &source.file,
                )?)),
                source_len: u64::try_from(source.bytes.len())
                    .map_err(|_| ScenarioError::ProviderJournalLimit)?,
                source_digest: provider_digest(&source.bytes),
                blob_name: None,
                phase: ProviderJournalPhase::Prepared,
                staging_identity: None,
                destination_identity: None,
                staging_name: None,
                staging_generation: 0,
                diagnostic_path: None,
                authentication_tag: String::new(),
            };
            journal.create(&gate, &record, None)?;
            provider_journal_after_phase_hook(ProviderJournalPhase::Prepared)?;
            provider_post_validation_hook(ProviderPostValidationOperation::Remove);
            record
        }
    };
    let removed =
        open_provider_directory(runtime.provider.tree(tree), PROVIDER_REMOVED_NAMESPACE)?;
    let retirement_evidence = open_provider_directory(
        runtime.provider.tree(tree),
        PROVIDER_RENAME_EVIDENCE_NAMESPACE,
    )?;
    if record.phase == ProviderJournalPhase::Cleanup {
        validate_retired_source(&runtime.provider, &record)?;
        return journal.complete(&gate, &record);
    }
    if record.phase == ProviderJournalPhase::Prepared {
        ensure_provider_diagnostic_capacity(&removed, PROVIDER_REMOVED_NAMESPACE, 1)?;
        record.diagnostic_path = Some(format!(
            "{PROVIDER_REMOVED_NAMESPACE}/retired-{}",
            record.operation_id
        ));
        record.phase = ProviderJournalPhase::RetireIntent;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::RetireIntent)?;
    }
    if record.phase == ProviderJournalPhase::RetireIntent {
        reconcile_provider_retirement(
            journal,
            &gate,
            &parent,
            &name,
            &removed,
            &retirement_evidence,
            path,
            &mut record,
        )?;
        record.phase = ProviderJournalPhase::Retired;
        journal.store(&gate, &record)?;
        provider_journal_after_phase_hook(ProviderJournalPhase::Retired)?;
    }
    validate_retired_source(&runtime.provider, &record)?;
    journal.complete(&gate, &record)
}

fn validate_journal_destination(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    provider: &ProviderRuntime,
    record: &ProviderJournalRecord,
    expected: &[u8],
    removed: &Dir,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let to_path = record
        .to_path
        .as_deref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let expected_identity = record
        .destination_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let (parent, name) = provider.parent_and_name(record.tree, to_path, false)?;
    let destination = match open_provider_regular_optional(
        &parent,
        &name,
        MAX_PROVIDER_RESCAN_BYTES,
        to_path,
    ) {
        Ok(Some(destination)) => destination,
        Ok(None) => return Err(ScenarioError::UnsafeProviderEntry(to_path.into())),
        Err(_) => {
            quarantine_provider_name(
                journal,
                gate,
                &parent,
                &name,
                removed,
                "destination-mismatch",
            )?;
            return Err(ScenarioError::UnsafeProviderEntry(to_path.into()));
        }
    };
    if destination.bytes != expected
        || !provider_file_matches_identity(&destination.file, expected_identity)?
    {
        quarantine_provider_name(
            journal,
            gate,
            &parent,
            &name,
            removed,
            "destination-mismatch",
        )?;
        return Err(ScenarioError::UnsafeProviderEntry(to_path.into()));
    }
    Ok(())
}

fn validate_retired_source(
    provider: &ProviderRuntime,
    record: &ProviderJournalRecord,
) -> Result<(), ScenarioError> {
    if !matches!(
        record.phase,
        ProviderJournalPhase::Retired | ProviderJournalPhase::Cleanup
    ) {
        return Err(ScenarioError::UnsafeProviderJournal(
            record.operation_id.clone(),
        ));
    }
    let diagnostic_path = record
        .diagnostic_path
        .as_deref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let (parent, name) = provider.parent_and_name(record.tree, diagnostic_path, false)?;
    let source = open_provider_regular_optional(
        &parent,
        &name,
        MAX_PROVIDER_RESCAN_BYTES,
        diagnostic_path,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(diagnostic_path.into()))?;
    let identity = record
        .source_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    if !provider_file_matches_identity(&source.file, identity)?
        || provider_digest(&source.bytes) != record.source_digest
        || u64::try_from(source.bytes.len()).ok() != Some(record.source_len)
    {
        return Err(ScenarioError::UnsafeProviderEntry(diagnostic_path.into()));
    }
    Ok(())
}

fn validate_retired_file(
    removed: &Dir,
    diagnostic_name: &str,
    diagnostic_path: &str,
    record: &ProviderJournalRecord,
) -> Result<(), ScenarioError> {
    let identity = record
        .source_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let retired = open_provider_regular_optional(
        removed,
        diagnostic_name,
        MAX_PROVIDER_RESCAN_BYTES,
        diagnostic_path,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(diagnostic_path.into()))?;
    if !provider_file_matches_identity(&retired.file, identity)?
        || provider_digest(&retired.bytes) != record.source_digest
        || u64::try_from(retired.bytes.len()).ok() != Some(record.source_len)
    {
        return Err(ScenarioError::UnsafeProviderEntry(diagnostic_path.into()));
    }
    Ok(())
}

fn validate_provider_name_identity_or_quarantine(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    parent: &Dir,
    name: &str,
    retained: &fs::File,
    removed: &Dir,
    path: &str,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let named = match open_provider_regular_optional(
        parent,
        name,
        MAX_PROVIDER_RESCAN_BYTES,
        path,
    ) {
        Ok(named) => named,
        Err(_) => {
            quarantine_provider_name(
                journal,
                gate,
                parent,
                name,
                removed,
                "destination-race",
            )?;
            return Err(ScenarioError::UnsafeProviderEntry(path.into()));
        }
    };
    if let Some(named) = named.as_ref() {
        if provider_files_have_same_identity(retained, &named.file)? {
            return Ok(());
        }
    }
    if named.is_some() {
        quarantine_provider_name(
            journal,
            gate,
            parent,
            name,
            removed,
            "destination-race",
        )?;
    }
    Err(ScenarioError::UnsafeProviderEntry(path.into()))
}

fn quarantine_provider_name(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    source_dir: &Dir,
    source_name: &str,
    removed: &Dir,
    prefix: &str,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    ensure_provider_diagnostic_capacity(removed, PROVIDER_REMOVED_NAMESPACE, 1)?;
    let source = open_provider_regular_optional(
        source_dir,
        source_name,
        MAX_PROVIDER_RESCAN_BYTES,
        source_name,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(source_name.into()))?;
    let diagnostic_name =
        provider_quarantine_diagnostic_name(prefix, source_name, &source.bytes);
    if open_provider_regular_optional(
        removed,
        &diagnostic_name,
        MAX_PROVIDER_RESCAN_BYTES,
        &format!("{PROVIDER_REMOVED_NAMESPACE}/{diagnostic_name}"),
    )?
    .is_some()
    {
        return Err(ScenarioError::UnsafeProviderEntry(format!(
            "{PROVIDER_REMOVED_NAMESPACE}/{diagnostic_name}"
        )));
    }
    provider_rename_named_noreplace(source_dir, source_name, removed, &diagnostic_name)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    sync_provider_publication_directories(removed, Some(source_dir))
}

fn provider_quarantine_diagnostic_name(prefix: &str, source_name: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tine-provider-diagnostic-name-v1\0");
    digest.update(prefix.as_bytes());
    digest.update(b"\0");
    digest.update(source_name.as_bytes());
    digest.update(b"\0");
    digest.update(provider_digest(bytes).as_bytes());
    format!("{prefix}-{:x}", digest.finalize())
}

fn ensure_provider_diagnostic_capacity(
    directory: &Dir,
    namespace: &str,
    additional_entries: usize,
) -> Result<(), ScenarioError> {
    let mut entries = 0_usize;
    for entry in directory
        .entries()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
    {
        let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
        entries = entries
            .checked_add(1)
            .ok_or(ScenarioError::ProviderRescanLimit)?;
        if entries
            .checked_add(additional_entries)
            .is_none_or(|entries| entries > MAX_PROVIDER_RESIDUE_ENTRIES)
        {
            return Err(ScenarioError::ProviderRescanLimit);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ScenarioError::UnsafeProviderEntry(format!("{namespace}/non-UTF-8")))?;
        if !entry
            .file_type()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
            .is_file()
        {
            return Err(ScenarioError::UnsafeProviderEntry(format!(
                "{namespace}/{name}"
            )));
        }
        let file = open_provider_file_nofollow(directory, &name)
            .map_err(|error| ScenarioError::UnsafeProviderEntry(error.to_string()))?;
        validate_provider_regular_file(
            &file,
            &format!("{namespace}/{name}"),
        )?;
    }
    Ok(())
}

fn reconcile_provider_retirement(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    source_dir: &Dir,
    source_name: &str,
    removed: &Dir,
    evidence: &Dir,
    source_path: &str,
    record: &mut ProviderJournalRecord,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let identity = record
        .source_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let diagnostic_path = record
        .diagnostic_path
        .as_deref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let diagnostic_name = diagnostic_path
        .strip_prefix(&format!("{PROVIDER_REMOVED_NAMESPACE}/"))
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let evidence_name = format!("retire-placeholder-{}", record.operation_id);
    ensure_provider_retirement_evidence(evidence, 0, 0)?;

    #[cfg(windows)]
    {
        let source = open_provider_regular_optional(
            source_dir,
            source_name,
            MAX_PROVIDER_RESCAN_BYTES,
            source_path,
        )?;
        let retired = open_provider_regular_optional(
            removed,
            diagnostic_name,
            MAX_PROVIDER_RESCAN_BYTES,
            diagnostic_path,
        )?;
        match (source, retired) {
            (Some(source), None) => {
                if !provider_file_matches_identity(&source.file, identity)?
                    || provider_digest(&source.bytes) != record.source_digest
                    || u64::try_from(source.bytes.len()).ok() != Some(record.source_len)
                {
                    return Err(ScenarioError::UnsafeProviderEntry(source_path.into()));
                }
                provider_retirement_after_validation_hook();
                provider_rename_handle_noreplace(&source.file, removed, diagnostic_name)
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                sync_provider_publication_directories(removed, Some(source_dir))?;
            }
            (None, Some(retired))
                if provider_file_matches_identity(&retired.file, identity)?
                    && provider_digest(&retired.bytes) == record.source_digest
                    && u64::try_from(retired.bytes.len()).ok() == Some(record.source_len) => {}
            (Some(_), Some(_)) => {
                return Err(ScenarioError::UnsafeProviderEntry(diagnostic_path.into()));
            }
            _ => return Err(ScenarioError::UnsafeProviderEntry(source_path.into())),
        }
    }

    #[cfg(unix)]
    {
        reconcile_private_retirement_evidence(evidence, &evidence_name, record)?;
        let mut source = open_provider_regular_optional(
            source_dir,
            source_name,
            MAX_PROVIDER_RESCAN_BYTES,
            source_path,
        )?;
        let mut retired = open_provider_regular_optional(
            removed,
            diagnostic_name,
            MAX_PROVIDER_RESCAN_BYTES,
            diagnostic_path,
        )?;

        if source.is_none() {
            let retired = retired
                .as_ref()
                .ok_or_else(|| ScenarioError::UnsafeProviderEntry(source_path.into()))?;
            if !provider_file_matches_identity(&retired.file, identity)?
                || provider_digest(&retired.bytes) != record.source_digest
                || u64::try_from(retired.bytes.len()).ok() != Some(record.source_len)
            {
                return Err(ScenarioError::UnsafeProviderEntry(diagnostic_path.into()));
            }
        } else {
            if retired.is_none() {
                let opened = source.as_ref().unwrap();
                if !provider_file_matches_identity(&opened.file, identity)?
                    || provider_digest(&opened.bytes) != record.source_digest
                    || u64::try_from(opened.bytes.len()).ok() != Some(record.source_len)
                {
                    return Err(ScenarioError::UnsafeProviderEntry(source_path.into()));
                }
                ensure_provider_diagnostic_capacity(
                    removed,
                    PROVIDER_REMOVED_NAMESPACE,
                    1,
                )?;
                let placeholder = create_provider_destination_exclusive(
                    removed,
                    diagnostic_name,
                    diagnostic_path,
                )?;
                placeholder
                    .sync_all()
                    .map_err(|error| ScenarioError::Io(error.to_string()))?;
                record.staging_identity =
                    Some(provider_identity_record(provider_file_identity(&placeholder)?));
                journal.store(gate, record)?;
                sync_provider_directory(removed)?;
                provider_journal_boundary_hook(
                    ProviderJournalBoundary::RetirementPlaceholderDurable,
                )?;
                retired = open_provider_regular_optional(
                    removed,
                    diagnostic_name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    diagnostic_path,
                )?;
            }

            let placeholder_identity = record.staging_identity.as_ref().ok_or_else(|| {
                ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
            })?;
            let source_is_original = source.as_ref().is_some_and(|opened| {
                provider_file_matches_identity(&opened.file, identity).unwrap_or(false)
                    && provider_digest(&opened.bytes) == record.source_digest
                    && u64::try_from(opened.bytes.len()).ok() == Some(record.source_len)
            });
            let source_is_placeholder = source.as_ref().is_some_and(|opened| {
                provider_file_matches_identity(&opened.file, placeholder_identity)
                    .unwrap_or(false)
            });
            let retired_is_original = retired.as_ref().is_some_and(|opened| {
                provider_file_matches_identity(&opened.file, identity).unwrap_or(false)
                    && provider_digest(&opened.bytes) == record.source_digest
                    && u64::try_from(opened.bytes.len()).ok() == Some(record.source_len)
            });
            let retired_is_placeholder = retired.as_ref().is_some_and(|opened| {
                provider_file_matches_identity(&opened.file, placeholder_identity)
                    .unwrap_or(false)
            });

            if source_is_original && retired_is_placeholder {
                provider_retirement_after_validation_hook();
                provider_exchange_names(
                    source_dir,
                    source_name,
                    removed,
                    diagnostic_name,
                )
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
                sync_provider_publication_directories(removed, Some(source_dir))?;
                provider_journal_boundary_hook(
                    ProviderJournalBoundary::RetirementExchangeDurable,
                )?;
                source = open_provider_regular_optional(
                    source_dir,
                    source_name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    source_path,
                )?;
                retired = open_provider_regular_optional(
                    removed,
                    diagnostic_name,
                    MAX_PROVIDER_RESCAN_BYTES,
                    diagnostic_path,
                )?;
            } else if !(source_is_placeholder && retired_is_original) {
                return Err(ScenarioError::UnsafeProviderEntry(source_path.into()));
            }

            let exchanged_source = source
                .as_ref()
                .ok_or_else(|| ScenarioError::UnsafeProviderEntry(source_path.into()))?;
            let exchanged_retired = retired
                .as_ref()
                .ok_or_else(|| ScenarioError::UnsafeProviderEntry(diagnostic_path.into()))?;
            if !provider_file_matches_identity(&exchanged_source.file, placeholder_identity)?
                || !provider_file_matches_identity(&exchanged_retired.file, identity)?
                || provider_digest(&exchanged_retired.bytes) != record.source_digest
                || u64::try_from(exchanged_retired.bytes.len()).ok() != Some(record.source_len)
            {
                let _ = provider_exchange_names(
                    source_dir,
                    source_name,
                    removed,
                    diagnostic_name,
                );
                let _ = sync_provider_publication_directories(removed, Some(source_dir));
                return Err(ScenarioError::UnsafeProviderEntry(source_path.into()));
            }

            provider_retirement_before_private_move_hook();
            provider_rename_named_noreplace(
                source_dir,
                source_name,
                evidence,
                &evidence_name,
            )
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
            sync_provider_publication_directories(evidence, Some(source_dir))?;
            provider_journal_boundary_hook(
                ProviderJournalBoundary::RetirementPlaceholderQuarantined,
            )?;
            if let Err(error) =
                reconcile_private_retirement_evidence(evidence, &evidence_name, record)
            {
                return Err(error);
            }
            if let Some(replacement) = open_provider_regular_optional(
                source_dir,
                source_name,
                MAX_PROVIDER_RESCAN_BYTES,
                source_path,
            )? {
                preserve_retirement_race(
                    source_dir,
                    source_name,
                    evidence,
                    &replacement.bytes,
                )?;
                return Err(ScenarioError::UnsafeProviderEntry(source_path.into()));
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    return Err(ScenarioError::UnsafeProviderEntry(format!(
        "{source_path}: handle-safe retirement is unsupported"
    )));

    validate_retired_file(removed, diagnostic_name, diagnostic_path, record)
}

fn ensure_provider_retirement_evidence(
    evidence: &Dir,
    additional_entries: usize,
    additional_bytes: usize,
) -> Result<(), ScenarioError> {
    let mut count = 0_usize;
    let mut bytes = 0_usize;
    for entry in evidence
        .entries()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
    {
        let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
        count = count
            .checked_add(1)
            .ok_or(ScenarioError::ProviderRescanLimit)?;
        if count
            .checked_add(additional_entries)
            .is_none_or(|count| count > MAX_PROVIDER_RESIDUE_ENTRIES)
        {
            return Err(ScenarioError::ProviderRescanLimit);
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ScenarioError::UnsafeProviderEntry(
                format!("{PROVIDER_RENAME_EVIDENCE_NAMESPACE}/non-UTF-8"),
            ))?;
        let valid_name = name
            .strip_prefix("retire-placeholder-")
            .or_else(|| name.strip_prefix("retirement-race-"))
            .is_some_and(valid_provider_journal_id);
        if !valid_name
            || !entry
                .file_type()
                .map_err(|error| ScenarioError::Io(error.to_string()))?
                .is_file()
        {
            return Err(ScenarioError::UnsafeProviderEntry(format!(
                "{PROVIDER_RENAME_EVIDENCE_NAMESPACE}/{name}"
            )));
        }
        let file = open_provider_file_nofollow(evidence, &name)
            .map_err(|error| ScenarioError::UnsafeProviderEntry(error.to_string()))?;
        let metadata = validate_provider_regular_file(
            &file,
            &format!("{PROVIDER_RENAME_EVIDENCE_NAMESPACE}/{name}"),
        )?;
        bytes = bytes
            .checked_add(
                usize::try_from(metadata.len())
                    .map_err(|_| ScenarioError::ProviderRescanLimit)?,
            )
            .ok_or(ScenarioError::ProviderRescanLimit)?;
        if bytes
            .checked_add(additional_bytes)
            .is_none_or(|bytes| bytes > MAX_PROVIDER_RESCAN_BYTES)
        {
            return Err(ScenarioError::ProviderRescanLimit);
        }
    }
    if count
        .checked_add(additional_entries)
        .is_none_or(|count| count > MAX_PROVIDER_RESIDUE_ENTRIES)
        || bytes
            .checked_add(additional_bytes)
            .is_none_or(|bytes| bytes > MAX_PROVIDER_RESCAN_BYTES)
    {
        return Err(ScenarioError::ProviderRescanLimit);
    }
    Ok(())
}

fn reconcile_private_retirement_evidence(
    evidence: &Dir,
    evidence_name: &str,
    record: &ProviderJournalRecord,
) -> Result<(), ScenarioError> {
    let Some(opened) = open_provider_regular_optional(
        evidence,
        evidence_name,
        MAX_PROVIDER_RESCAN_BYTES,
        evidence_name,
    )?
    else {
        return Ok(());
    };
    let placeholder_identity = record.staging_identity.as_ref().ok_or_else(|| {
        ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
    })?;
    if !provider_file_matches_identity(&opened.file, placeholder_identity)? {
        preserve_retirement_race(
            evidence,
            evidence_name,
            evidence,
            &opened.bytes,
        )?;
        return Err(ScenarioError::UnsafeProviderEntry(evidence_name.into()));
    }
    let retained_identity = provider_file_identity(&opened.file)?;
    provider_retirement_before_private_delete_hook();
    let retained = open_provider_regular_optional(
        evidence,
        evidence_name,
        MAX_PROVIDER_RESCAN_BYTES,
        evidence_name,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(evidence_name.into()))?;
    if provider_file_identity(&retained.file)? != retained_identity {
        return Err(ScenarioError::UnsafeProviderEntry(evidence_name.into()));
    }
    evidence
        .remove_file(evidence_name)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    sync_provider_directory(evidence)?;
    provider_journal_boundary_hook(
        ProviderJournalBoundary::RetirementPlaceholderPrivateDeleted,
    )
}

fn preserve_retirement_race(
    source_dir: &Dir,
    source_name: &str,
    evidence: &Dir,
    bytes: &[u8],
) -> Result<(), ScenarioError> {
    ensure_provider_retirement_evidence(evidence, 1, bytes.len())?;
    let race_name = provider_quarantine_diagnostic_name(
        "retirement-race",
        source_name,
        bytes,
    );
    provider_rename_named_noreplace(source_dir, source_name, evidence, &race_name)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    sync_provider_publication_directories(evidence, Some(source_dir))
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).is_relative()
        && !Path::new(path)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

fn valid_provider_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PROVIDER_PATH_BYTES
        && !path.contains('\\')
        && !path.starts_with('/')
        && path.split('/').all(|component| {
            valid_name(component, 128)
                && component != "."
                && component != ".."
                && !component.contains(':')
        })
}

fn valid_provider_journal_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const PROVIDER_OBJECTS_NAMESPACE: &str = "objects";
const PROVIDER_MANIFESTS_NAMESPACE: &str = "manifests";
const PROVIDER_TEMP_NAMESPACE: &str = ".part";
const PROVIDER_REMOVED_NAMESPACE: &str = "removed";
const PROVIDER_RENAME_EVIDENCE_NAMESPACE: &str = "rename-evidence";

fn valid_provider_user_path(path: &str) -> bool {
    valid_provider_path(path)
        && path
            .split('/')
            .next()
            .is_some_and(|namespace| {
                ![
                    PROVIDER_TEMP_NAMESPACE,
                    PROVIDER_REMOVED_NAMESPACE,
                    PROVIDER_RENAME_EVIDENCE_NAMESPACE,
                ]
                .contains(&namespace)
            })
}

fn reject_provider_temporary_path(path: &str) -> Result<(), ScenarioError> {
    if valid_provider_user_path(path) {
        Ok(())
    } else {
        Err(ScenarioError::InvalidProviderPath(path.into()))
    }
}

fn provider_item_kind(path: &str) -> Option<ProviderItemKind> {
    let (namespace, remainder) = path.split_once('/')?;
    if remainder.is_empty() {
        return None;
    }
    match namespace {
        PROVIDER_OBJECTS_NAMESPACE => Some(ProviderItemKind::Object),
        PROVIDER_MANIFESTS_NAMESPACE => Some(ProviderItemKind::Manifest),
        _ => None,
    }
}

fn provider_tree_name(tree: ProviderTree) -> &'static str {
    match tree {
        ProviderTree::Inbox => "inbox",
        ProviderTree::Outbox => "outbox",
    }
}

fn ensure_provider_directory(parent: &Dir, name: &str) -> Result<(), ScenarioError> {
    ensure_directory_nofollow(parent, name)
        .map_err(|error| ScenarioError::UnsafeProviderEntry(format!("{name}: {error}")))
}

fn open_provider_directory(parent: &Dir, name: &str) -> Result<Dir, ScenarioError> {
    let directory = open_dir_nofollow(parent, name)
        .map_err(|error| ScenarioError::UnsafeProviderEntry(format!("{name}: {error}")))?;
    validate_provider_directory_owner(&directory, name)?;
    Ok(directory)
}

#[cfg(unix)]
fn validate_provider_directory_owner(directory: &Dir, name: &str) -> Result<(), ScenarioError> {
    let metadata = directory
        .try_clone()
        .and_then(|directory| directory.into_std_file().metadata())
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    // SAFETY: geteuid has no preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(ScenarioError::UnsafeProviderEntry(format!(
            "{name} has the wrong owner"
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_provider_directory_owner(_directory: &Dir, _name: &str) -> Result<(), ScenarioError> {
    Ok(())
}

#[cfg(unix)]
fn open_provider_file_nofollow(parent: &Dir, name: &str) -> std::io::Result<fs::File> {
    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider filename"))?;
    // SAFETY: the filename is a live C string and openat resolves it beneath
    // the retained parent capability. O_NOFOLLOW binds validation and reading
    // to one opened handle; O_NONBLOCK prevents special-file blocking.
    let fd = unsafe {
        libc::openat(
            parent.as_fd().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: openat returned a newly owned file descriptor.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_provider_file_write_nofollow(parent: &Dir, name: &str) -> std::io::Result<fs::File> {
    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider filename"))?;
    // SAFETY: the name is resolved beneath the retained parent capability;
    // O_NOFOLLOW and O_NONBLOCK reject link/special-file substitution.
    let fd = unsafe {
        libc::openat(
            parent.as_fd().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: openat returned a newly owned descriptor.
        Ok(unsafe { fs::File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
fn open_provider_file_nofollow(parent: &Dir, name: &str) -> std::io::Result<fs::File> {
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(FollowSymlinks::No)
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    let file = parent.open_with(name, &options)?.into_std();
    let metadata = file.metadata()?;
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "provider entry is a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_provider_file_write_nofollow(parent: &Dir, name: &str) -> std::io::Result<fs::File> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .follow(FollowSymlinks::No)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    let file = parent.open_with(name, &options)?.into_std();
    let metadata = file.metadata()?;
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "provider entry is a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_provider_file_nofollow(_parent: &Dir, _name: &str) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic provider no-follow reads are unsupported",
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_provider_file_write_nofollow(
    _parent: &Dir,
    _name: &str,
) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "provider no-follow writes are unsupported",
    ))
}

#[cfg(unix)]
fn validate_provider_regular_file(
    file: &fs::File,
    path: &str,
) -> Result<fs::Metadata, ScenarioError> {
    validate_provider_regular_file_with_link_count(file, path, true)
}

#[cfg(unix)]
fn validate_provider_regular_file_with_link_count(
    file: &fs::File,
    path: &str,
    require_single_link: bool,
) -> Result<fs::Metadata, ScenarioError> {
    let metadata = file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    // SAFETY: geteuid has no preconditions.
    if !metadata.is_file()
        || (require_single_link && metadata.nlink() != 1)
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    Ok(metadata)
}

#[cfg(windows)]
fn validate_provider_regular_file(
    file: &fs::File,
    path: &str,
) -> Result<fs::Metadata, ScenarioError> {
    validate_provider_regular_file_with_link_count(file, path, true)
}

#[cfg(windows)]
fn validate_provider_regular_file_with_link_count(
    file: &fs::File,
    path: &str,
    require_single_link: bool,
) -> Result<fs::Metadata, ScenarioError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
    };

    let metadata = file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: `file` owns a live handle, `standard` is writable for its full
    // declared size, and GetFileInformationByHandleEx does not retain either.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(ScenarioError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    if !metadata.is_file()
        || metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        || (require_single_link && standard.NumberOfLinks != 1)
    {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    Ok(metadata)
}

#[cfg(not(any(unix, windows)))]
fn validate_provider_regular_file(
    _file: &fs::File,
    path: &str,
) -> Result<fs::Metadata, ScenarioError> {
    Err(ScenarioError::UnsafeProviderEntry(path.into()))
}

#[cfg(not(any(unix, windows)))]
fn validate_provider_regular_file_with_link_count(
    _file: &fs::File,
    path: &str,
    _require_single_link: bool,
) -> Result<fs::Metadata, ScenarioError> {
    Err(ScenarioError::UnsafeProviderEntry(path.into()))
}

#[cfg(unix)]
fn provider_file_identity(file: &fs::File) -> Result<ProviderFileIdentity, ScenarioError> {
    let metadata = file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    Ok(ProviderFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn provider_file_identity(file: &fs::File) -> Result<ProviderFileIdentity, ScenarioError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information = FILE_ID_INFO::default();
    // SAFETY: `file` owns a live handle, `information` is writable for its
    // full declared size, and the system call retains neither pointer.
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(ScenarioError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(ProviderFileIdentity {
        volume: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
fn provider_file_identity(_file: &fs::File) -> Result<ProviderFileIdentity, ScenarioError> {
    Err(ScenarioError::UnsafeProviderEntry(
        "provider file identity is unsupported".into(),
    ))
}

fn provider_files_have_same_identity(
    left: &fs::File,
    right: &fs::File,
) -> Result<bool, ScenarioError> {
    Ok(provider_file_identity(left)? == provider_file_identity(right)?)
}

#[cfg(unix)]
fn provider_identity_record(
    identity: ProviderFileIdentity,
) -> ProviderIdentityRecord {
    ProviderIdentityRecord {
        platform: "unix".into(),
        first: identity.device,
        second: identity.inode.to_string(),
    }
}

#[cfg(windows)]
fn provider_identity_record(
    identity: ProviderFileIdentity,
) -> ProviderIdentityRecord {
    ProviderIdentityRecord {
        platform: "windows".into(),
        first: identity.volume,
        second: base64url_encode(&identity.file_id),
    }
}

#[cfg(not(any(unix, windows)))]
fn provider_identity_record(
    _identity: ProviderFileIdentity,
) -> ProviderIdentityRecord {
    ProviderIdentityRecord {
        platform: "unsupported".into(),
        first: 0,
        second: String::new(),
    }
}

fn provider_file_matches_identity(
    file: &fs::File,
    expected: &ProviderIdentityRecord,
) -> Result<bool, ScenarioError> {
    Ok(provider_identity_record(provider_file_identity(file)?) == *expected)
}

#[cfg(unix)]
fn valid_provider_identity_record(identity: &ProviderIdentityRecord) -> bool {
    identity.platform == "unix"
        && identity
            .second
            .parse::<u64>()
            .is_ok_and(|inode| inode.to_string() == identity.second)
}

#[cfg(windows)]
fn valid_provider_identity_record(identity: &ProviderIdentityRecord) -> bool {
    identity.platform == "windows"
        && base64url_decode(&identity.second).is_ok_and(|file_id| {
            file_id.len() == 16 && base64url_encode(&file_id) == identity.second
        })
}

#[cfg(not(any(unix, windows)))]
fn valid_provider_identity_record(_identity: &ProviderIdentityRecord) -> bool {
    false
}

fn provider_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hmac_sha256_hex(key: &[u8; 32], bytes: &[u8]) -> String {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, byte) in key.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(bytes);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    format!("{:x}", outer.finalize())
}

fn constant_time_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn provider_journal_phase_rank(phase: ProviderJournalPhase) -> u8 {
    match phase {
        ProviderJournalPhase::Prepared => 0,
        ProviderJournalPhase::Staged => 1,
        ProviderJournalPhase::PublishIntent => 2,
        ProviderJournalPhase::Published => 3,
        ProviderJournalPhase::RetireIntent => 4,
        ProviderJournalPhase::Retired => 5,
        ProviderJournalPhase::Cleanup => 6,
    }
}

fn canonical_provider_authority_bytes(
    record: &ProviderAuthorityRecord,
) -> Result<Vec<u8>, ScenarioError> {
    if record.authority_schema_version != PROVIDER_AUTHORITY_SCHEMA_VERSION
        || !valid_provider_identity_record(&record.device_identity)
        || !valid_provider_identity_record(&record.journal_identity)
        || !valid_provider_identity_record(&record.authority_key_identity)
        || !valid_provider_identity_record(&record.records_identity)
        || !valid_provider_identity_record(&record.blobs_identity)
        || !valid_provider_identity_record(&record.quarantine_identity)
        || !valid_provider_identity_record(&record.completed_identity)
    {
        return Err(ScenarioError::UnsafeProviderJournal(
            PROVIDER_DEVICE_AUTHORITY_NAME.into(),
        ));
    }
    let bytes =
        serde_json::to_vec(record).map_err(|error| ScenarioError::Io(error.to_string()))?;
    if bytes.len() > MAX_PROVIDER_AUTHORITY_BYTES {
        return Err(ScenarioError::ProviderJournalLimit);
    }
    Ok(bytes)
}

fn decode_provider_authentication_key(
    record: &ProviderAuthorityRecord,
) -> Result<[u8; 32], ScenarioError> {
    let decoded = base64url_decode(&record.authentication_key)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(PROVIDER_DEVICE_AUTHORITY_NAME.into()))?;
    if base64url_encode(&decoded) != record.authentication_key {
        return Err(ScenarioError::UnsafeProviderJournal(
            PROVIDER_DEVICE_AUTHORITY_NAME.into(),
        ));
    }
    decoded
        .try_into()
        .map_err(|_| ScenarioError::UnsafeProviderJournal(PROVIDER_DEVICE_AUTHORITY_NAME.into()))
}

fn read_provider_authority_record(
    authority_file: &fs::File,
) -> Result<(Vec<u8>, ProviderAuthorityRecord), ScenarioError> {
    let mut file = authority_file
        .try_clone()
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let metadata = validate_provider_regular_file(&file, PROVIDER_DEVICE_AUTHORITY_NAME)
        .map_err(|_| {
            ScenarioError::UnsafeProviderJournal(PROVIDER_DEVICE_AUTHORITY_NAME.into())
        })?;
    let advertised =
        usize::try_from(metadata.len()).map_err(|_| ScenarioError::ProviderJournalLimit)?;
    if advertised > MAX_PROVIDER_AUTHORITY_BYTES {
        return Err(ScenarioError::ProviderJournalLimit);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let mut bytes = Vec::with_capacity(advertised);
    Read::by_ref(&mut file)
        .take(
            u64::try_from(MAX_PROVIDER_AUTHORITY_BYTES + 1)
                .map_err(|_| ScenarioError::ProviderJournalLimit)?,
        )
        .read_to_end(&mut bytes)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if bytes.len() != advertised || bytes.len() > MAX_PROVIDER_AUTHORITY_BYTES {
        return Err(ScenarioError::UnsafeProviderJournal(
            PROVIDER_DEVICE_AUTHORITY_NAME.into(),
        ));
    }
    let record: ProviderAuthorityRecord = serde_json::from_slice(&bytes)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(PROVIDER_DEVICE_AUTHORITY_NAME.into()))?;
    if canonical_provider_authority_bytes(&record)? != bytes {
        return Err(ScenarioError::UnsafeProviderJournal(
            PROVIDER_DEVICE_AUTHORITY_NAME.into(),
        ));
    }
    decode_provider_authentication_key(&record)?;
    Ok((bytes, record))
}

fn provider_directory_identity(directory: &Dir) -> Result<ProviderFileIdentity, ScenarioError> {
    let file = directory
        .try_clone()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
        .into_std_file();
    provider_file_identity(&file)
}

fn validate_named_provider_directory(
    parent: &Dir,
    name: &str,
    retained: &Dir,
    expected: ProviderFileIdentity,
) -> Result<(), ScenarioError> {
    let named = open_provider_directory(parent, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(format!("{name} was replaced")))?;
    if provider_directory_identity(&named)? != expected
        || provider_directory_identity(retained)? != expected
    {
        return Err(ScenarioError::UnsafeProviderJournal(format!(
            "{name} identity changed"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn open_provider_outer_authority_file_nofollow(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    let file = open_provider_file_write_nofollow(parent, name)
        .map_err(|error| ScenarioError::UnsafeProviderJournal(format!("{name}: {error}")))?;
    validate_provider_regular_file(&file, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
    Ok(file)
}

#[cfg(windows)]
fn open_provider_outer_authority_file_nofollow(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .follow(FollowSymlinks::No)
        .access_mode(GENERIC_READ | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderJournal(format!("{name}: {error}")))?
        .into_std();
    validate_provider_regular_file(&file, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_provider_outer_authority_file_nofollow(
    _parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    Err(ScenarioError::UnsafeProviderJournal(format!(
        "{name}: provider authority is unsupported"
    )))
}

fn create_provider_outer_authority_file_exclusive(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        options
            .follow(FollowSymlinks::No)
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderJournal(format!("{name}: {error}")))?
        .into_std();
    validate_provider_regular_file(&file, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
    Ok(file)
}

fn open_or_create_provider_outer_authority(
    device_directory: &Dir,
) -> Result<(fs::File, bool), ScenarioError> {
    match open_provider_outer_authority_file_nofollow(
        device_directory,
        PROVIDER_DEVICE_AUTHORITY_NAME,
    ) {
        Ok(file) => Ok((file, false)),
        Err(ScenarioError::UnsafeProviderJournal(_))
            if !device_directory.exists(PROVIDER_DEVICE_AUTHORITY_NAME) =>
        {
            match create_provider_outer_authority_file_exclusive(
                device_directory,
                PROVIDER_DEVICE_AUTHORITY_NAME,
            ) {
                Ok(file) => Ok((file, true)),
                Err(_) => open_provider_outer_authority_file_nofollow(
                    device_directory,
                    PROVIDER_DEVICE_AUTHORITY_NAME,
                )
                .map(|file| (file, false)),
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn open_and_lock_provider_outer_authority(
    device_directory: &Dir,
) -> Result<(fs::File, fs::File, bool), ScenarioError> {
    let lock_file = device_directory
        .try_clone()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
        .into_std_file();
    if !provider_lock_file_exclusive_nonblocking(&lock_file)
        .map_err(|error| ScenarioError::Io(error.to_string()))?
    {
        return Err(ScenarioError::UnsafeProviderJournal(
            "provider transaction gate is held by another process".into(),
        ));
    }
    match open_or_create_provider_outer_authority(device_directory) {
        Ok((authority, created)) => Ok((authority, lock_file, created)),
        Err(error) => {
            provider_unlock_file(&lock_file);
            Err(error)
        }
    }
}

#[cfg(windows)]
fn open_and_lock_provider_outer_authority(
    device_directory: &Dir,
) -> Result<(fs::File, fs::File, bool), ScenarioError> {
    let (authority, created) = open_or_create_provider_outer_authority(device_directory)?;
    let lock_file = authority
        .try_clone()
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if !provider_lock_file_exclusive_nonblocking(&lock_file)
        .map_err(|error| ScenarioError::Io(error.to_string()))?
    {
        return Err(ScenarioError::UnsafeProviderJournal(
            "provider transaction gate is held by another process".into(),
        ));
    }
    Ok((authority, lock_file, created))
}

#[cfg(not(any(unix, windows)))]
fn open_and_lock_provider_outer_authority(
    _device_directory: &Dir,
) -> Result<(fs::File, fs::File, bool), ScenarioError> {
    Err(ScenarioError::UnsafeProviderJournal(
        "provider transaction authority is unsupported".into(),
    ))
}

#[cfg(unix)]
fn provider_transaction_lock_handle(
    authority: &ProviderTransactionAuthority,
) -> std::io::Result<fs::File> {
    authority
        .device_directory
        .try_clone()
        .map(Dir::into_std_file)
}

#[cfg(windows)]
fn provider_transaction_lock_handle(
    authority: &ProviderTransactionAuthority,
) -> std::io::Result<fs::File> {
    authority.authority_file.try_clone()
}

#[cfg(not(any(unix, windows)))]
fn provider_transaction_lock_handle(
    _authority: &ProviderTransactionAuthority,
) -> std::io::Result<fs::File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "provider transaction locking is unsupported",
    ))
}

fn open_provider_authority_key_nofollow(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    open_provider_outer_authority_file_nofollow(parent, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))
}

fn open_provider_authority_key_optional(
    parent: &Dir,
    name: &str,
) -> Result<Option<fs::File>, ScenarioError> {
    if !parent.exists(name) {
        return Ok(None);
    }
    open_provider_authority_key_nofollow(parent, name).map(Some)
}

fn create_provider_authority_key_exclusive(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    create_provider_outer_authority_file_exclusive(parent, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))
}

fn create_local_file_exclusive(
    parent: &Dir,
    name: &str,
) -> Result<fs::File, ScenarioError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderJournal(format!("{name}: {error}")))?
        .into_std();
    validate_provider_regular_file(&file, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
    Ok(file)
}

fn validate_local_file_bytes(
    file: &mut fs::File,
    expected: &[u8],
    name: &str,
) -> Result<(), ScenarioError> {
    validate_provider_regular_file(file, name)
        .map_err(|_| ScenarioError::UnsafeProviderJournal(name.into()))?;
    let expected_len =
        u64::try_from(expected.len()).map_err(|_| ScenarioError::ProviderJournalLimit)?;
    if file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
        .len()
        != expected_len
    {
        return Err(ScenarioError::UnsafeProviderJournal(name.into()));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let mut actual = Vec::with_capacity(expected.len());
    Read::by_ref(file)
        .take(expected_len.saturating_add(1))
        .read_to_end(&mut actual)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if actual != expected {
        return Err(ScenarioError::UnsafeProviderJournal(name.into()));
    }
    Ok(())
}

fn open_provider_regular_optional(
    parent: &Dir,
    name: &str,
    limit: usize,
    path: &str,
) -> Result<Option<OpenProviderFile>, ScenarioError> {
    let mut file = match open_provider_file_nofollow(parent, name) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ScenarioError::UnsafeProviderEntry(format!(
                "{path}: {error}"
            )));
        }
    };
    let metadata = validate_provider_regular_file(&file, path)?;
    let advertised =
        usize::try_from(metadata.len()).map_err(|_| ScenarioError::ProviderRescanLimit)?;
    if advertised > limit {
        return Err(ScenarioError::ProviderRescanLimit);
    }
    let read_limit = u64::try_from(limit)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .ok_or(ScenarioError::ProviderRescanLimit)?;
    let mut bytes = Vec::with_capacity(advertised);
    Read::by_ref(&mut file)
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if bytes.len() > limit || bytes.len() != advertised {
        return Err(ScenarioError::ProviderRescanLimit);
    }
    Ok(Some(OpenProviderFile { file, bytes }))
}

/// Read from the retained handle rather than a provider name.  A named
/// provider entry can be swapped between actions; only the retained handle is
/// authoritative for the bytes that are eligible for publication.
fn validate_provider_file_bytes(
    staged: &mut ProviderStagingFile,
    expected: &[u8],
    path: &str,
) -> Result<(), ScenarioError> {
    // Anonymous staging has zero links before publication; every named staging
    // object must remain single-link even though its pathname is never trusted
    // as publication authority.
    validate_provider_regular_file_with_link_count(
        &staged.file,
        path,
        staged.name.is_some(),
    )?;
    let expected_len =
        u64::try_from(expected.len()).map_err(|_| ScenarioError::ProviderRescanLimit)?;
    if staged
        .file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
        .len()
        != expected_len
    {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    staged.file.seek(SeekFrom::Start(0))
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let mut actual = Vec::with_capacity(expected.len());
    Read::by_ref(&mut staged.file)
        .take(expected_len.saturating_add(1))
        .read_to_end(&mut actual)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if actual != expected {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    staged
        .file
        .seek(SeekFrom::Start(expected_len))
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    Ok(())
}

enum ProviderDestinationState {
    Absent,
    ExactBytes,
    ConflictingBytes,
}

/// Reconcile a destination against the exact retained bytes. This is the
/// publication state machine's recovery point: a previous call may have made
/// the name durable before an injected or validation error was returned.
fn provider_destination_state(
    destination_dir: &Dir,
    destination_name: &str,
    expected: &[u8],
    destination_path: &str,
) -> Result<ProviderDestinationState, ScenarioError> {
    match open_provider_regular_optional(
        destination_dir,
        destination_name,
        MAX_PROVIDER_RESCAN_BYTES,
        destination_path,
    )? {
        None => Ok(ProviderDestinationState::Absent),
        Some(opened) if opened.bytes == expected => Ok(ProviderDestinationState::ExactBytes),
        Some(_) => Ok(ProviderDestinationState::ConflictingBytes),
    }
}

fn validate_journal_staging(
    staging: &Dir,
    record: &ProviderJournalRecord,
    expected: &[u8],
    path: &str,
) -> Result<(), ScenarioError> {
    let staging_name = record
        .staging_name
        .as_deref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let identity = record
        .staging_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let staged = open_provider_regular_optional(
        staging,
        staging_name,
        MAX_PROVIDER_RESCAN_BYTES,
        staging_name,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(path.into()))?;
    if staged.bytes != expected || !provider_file_matches_identity(&staged.file, identity)? {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    Ok(())
}

/// Publish journaled bytes through an exclusively created destination handle.
/// The replaceable staging pathname is never the source of a production
/// rename. Its authenticated bytes come from the journal blob, and the new
/// destination identity is made durable in the record before that retained
/// handle is populated. A retry may therefore finish an empty or partial
/// destination only when its opened identity is the recorded one.
fn publish_journal_destination(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    record: &mut ProviderJournalRecord,
    staging: &Dir,
    tree: &Dir,
    destination_dir: &Dir,
    destination_name: &str,
    expected: &[u8],
    destination_path: &str,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let existing = open_provider_regular_optional(
        destination_dir,
        destination_name,
        MAX_PROVIDER_RESCAN_BYTES,
        destination_path,
    )?;
    let mut destination = if let Some(existing) = existing {
        let identity = record.destination_identity.as_ref().ok_or_else(|| {
            ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
        })?;
        if !provider_file_matches_identity(&existing.file, identity)? {
            return Err(ScenarioError::ProviderConflictingBytes(
                destination_path.into(),
            ));
        }
        if existing.bytes == expected {
            cleanup_journal_staging(journal, gate, record, staging, tree)?;
            return Ok(());
        }
        open_provider_file_write_nofollow(destination_dir, destination_name)
            .map_err(|error| ScenarioError::UnsafeProviderEntry(format!(
                "{destination_path}: {error}"
            )))?
    } else {
        validate_journal_staging(staging, record, expected, destination_path)?;
        provider_publication_source_after_validation_hook();
        let destination = create_provider_destination_exclusive(
            destination_dir,
            destination_name,
            destination_path,
        )?;
        record.destination_identity =
            Some(provider_identity_record(provider_file_identity(&destination)?));
        journal.store(gate, record)?;
        destination
    };
    let identity = record.destination_identity.as_ref().ok_or_else(|| {
        ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
    })?;
    if !provider_file_matches_identity(&destination, identity)? {
        return Err(ScenarioError::UnsafeProviderEntry(destination_path.into()));
    }
    destination
        .set_len(0)
        .and_then(|()| destination.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| destination.write_all(expected))
        .and_then(|()| destination.sync_all())
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    validate_provider_open_file_bytes(&mut destination, expected, destination_path)?;
    provider_publication_after_publish_hook()?;

    cleanup_journal_staging(journal, gate, record, staging, tree)?;
    Ok(())
}

fn cleanup_journal_staging(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    record: &ProviderJournalRecord,
    staging: &Dir,
    tree: &Dir,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let Some(staging_name) = record.staging_name.as_deref() else {
        return Ok(());
    };
    if open_provider_regular_optional(
        staging,
        staging_name,
        MAX_PROVIDER_RESCAN_BYTES,
        staging_name,
    )?
    .is_none()
    {
        return Ok(());
    }
    quarantine_unowned_staging(
        journal,
        gate,
        staging,
        staging_name,
        tree,
        &record.operation_id,
        record.staging_generation,
    )?;
    let diagnostic_name = format!(
        "orphan-{}-{}",
        record.operation_id, record.staging_generation
    );
    let removed = open_provider_directory(tree, PROVIDER_REMOVED_NAMESPACE)?;
    let diagnostic = open_provider_regular_optional(
        &removed,
        &diagnostic_name,
        MAX_PROVIDER_RESCAN_BYTES,
        &format!("{PROVIDER_REMOVED_NAMESPACE}/{diagnostic_name}"),
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(diagnostic_name.clone()))?;
    let identity = record.staging_identity.as_ref().ok_or_else(|| {
        ScenarioError::UnsafeProviderJournal(record.operation_id.clone())
    })?;
    if provider_file_matches_identity(&diagnostic.file, identity)? {
        removed
            .remove_file(&diagnostic_name)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        sync_provider_directory(&removed)?;
    }
    Ok(())
}

fn validate_put_destination(
    destination_dir: &Dir,
    destination_name: &str,
    destination_path: &str,
    expected: &[u8],
    record: &ProviderJournalRecord,
) -> Result<(), ScenarioError> {
    let identity = record
        .destination_identity
        .as_ref()
        .ok_or_else(|| ScenarioError::UnsafeProviderJournal(record.operation_id.clone()))?;
    let destination = open_provider_regular_optional(
        destination_dir,
        destination_name,
        MAX_PROVIDER_RESCAN_BYTES,
        destination_path,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(destination_path.into()))?;
    if destination.bytes != expected
        || !provider_file_matches_identity(&destination.file, identity)?
    {
        return Err(ScenarioError::ProviderConflictingBytes(
            destination_path.into(),
        ));
    }
    Ok(())
}

fn quarantine_unowned_staging(
    journal: &ProviderRetryJournal,
    gate: &ProviderTransactionGate,
    staging: &Dir,
    staging_name: &str,
    tree: &Dir,
    operation_id: &str,
    generation: u32,
) -> Result<(), ScenarioError> {
    journal.require_transaction_gate(gate)?;
    let removed = open_provider_directory(tree, PROVIDER_REMOVED_NAMESPACE)?;
    ensure_provider_diagnostic_capacity(&removed, PROVIDER_REMOVED_NAMESPACE, 1)?;
    let diagnostic_name = format!("orphan-{operation_id}-{generation}");
    if open_provider_regular_optional(
        &removed,
        &diagnostic_name,
        MAX_PROVIDER_RESCAN_BYTES,
        &format!("{PROVIDER_REMOVED_NAMESPACE}/{diagnostic_name}"),
    )?
    .is_some()
    {
        return Err(ScenarioError::UnsafeProviderEntry(format!(
            "{PROVIDER_REMOVED_NAMESPACE}/{diagnostic_name}"
        )));
    }
    provider_rename_named_noreplace(staging, staging_name, &removed, &diagnostic_name)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    sync_provider_publication_directories(&removed, Some(staging))
}

/// Publish anonymous staging from its handle, or consume the exact named
/// staging entry with an atomic no-replace rename. In both cases the published
/// destination must match the retained handle and bytes before success.
#[cfg(test)]
fn publish_provider_file_noreplace(
    source_file: &ProviderStagingFile,
    destination_dir: &Dir,
    destination_name: &str,
    expected: &[u8],
    destination_path: &str,
    source_directory: Option<&Dir>,
) -> Result<(), ScenarioError> {
    let mut validation_handle = ProviderStagingFile {
        file: source_file
            .file
            .try_clone()
            .map_err(|error| ScenarioError::Io(error.to_string()))?,
        name: source_file.name.clone(),
    };
    validate_provider_file_bytes(&mut validation_handle, expected, destination_path)?;
    if source_file.name.is_some() {
        // A named staging pathname is replaceable after validation on every
        // supported platform. Create the destination itself exclusively and
        // write only the already validated expected bytes through that
        // retained destination handle; the staging name is diagnostic residue.
        let mut destination_file = create_provider_destination_exclusive(
            destination_dir,
            destination_name,
            destination_path,
        )?;
        destination_file
            .write_all(expected)
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        destination_file
            .sync_all()
            .map_err(|error| ScenarioError::Io(error.to_string()))?;
        validate_provider_regular_file(&destination_file, destination_path)?;
        validate_provider_open_file_bytes(&mut destination_file, expected, destination_path)?;
        provider_publication_after_publish_hook()?;
        let destination = open_provider_regular_optional(
            destination_dir,
            destination_name,
            MAX_PROVIDER_RESCAN_BYTES,
            destination_path,
        )?
        .ok_or_else(|| ScenarioError::UnsafeProviderEntry(destination_path.into()))?;
        if destination.bytes != expected
            || !provider_files_have_same_identity(&destination_file, &destination.file)?
        {
            return Err(ScenarioError::UnsafeProviderEntry(destination_path.into()));
        }
        if let (Some(source_directory), Some(source_name)) =
            (source_directory, source_file.name.as_deref())
        {
            preserve_named_staging_diagnostic(
                source_directory,
                source_name,
                destination_path,
                expected,
            )?;
            sync_provider_publication_directories(
                destination_dir,
                Some(source_directory),
            )?;
        } else {
            sync_provider_publication_directories(destination_dir, None)?;
        }
        return Ok(());
    }
    provider_publish_staged_file_noreplace(
        source_file,
        source_directory,
        destination_dir,
        destination_name,
    )
    .map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                ScenarioError::ProviderConflictingBytes(destination_path.into())
            } else {
                ScenarioError::Io(error.to_string())
            }
        })?;
    provider_publication_after_publish_hook()?;
    let destination = open_provider_regular_optional(
        destination_dir,
        destination_name,
        MAX_PROVIDER_RESCAN_BYTES,
        destination_path,
    )?
    .ok_or_else(|| ScenarioError::UnsafeProviderEntry(destination_path.into()))?;
    if destination.bytes != expected
        || !provider_files_have_same_identity(&source_file.file, &destination.file)?
    {
        return Err(ScenarioError::UnsafeProviderEntry(destination_path.into()));
    }
    sync_provider_publication_directories(destination_dir, source_directory)?;
    Ok(())
}

#[cfg(test)]
fn preserve_named_staging_diagnostic(
    staging: &Dir,
    staging_name: &str,
    destination_path: &str,
    expected: &[u8],
) -> Result<(), ScenarioError> {
    ensure_provider_diagnostic_capacity(staging, PROVIDER_TEMP_NAMESPACE, 0)?;
    let Some(current) = open_provider_regular_optional(
        staging,
        staging_name,
        MAX_PROVIDER_RESCAN_BYTES,
        &format!("{PROVIDER_TEMP_NAMESPACE}/{staging_name}"),
    )?
    else {
        return Ok(());
    };
    validate_provider_regular_file(
        &current.file,
        &format!("{PROVIDER_TEMP_NAMESPACE}/{staging_name}"),
    )?;
    let mut digest = Sha256::new();
    digest.update(b"tine-provider-named-staging-diagnostic-v1\0");
    digest.update(destination_path.as_bytes());
    digest.update(b"\0");
    digest.update(expected);
    let diagnostic_name = format!("published-{:x}", digest.finalize());
    provider_rename_named_noreplace(staging, staging_name, staging, &diagnostic_name)
        .map_err(|error| ScenarioError::Io(error.to_string()))
}

fn create_provider_destination_exclusive(
    parent: &Dir,
    name: &str,
    path: &str,
) -> Result<fs::File, ScenarioError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    options.follow(FollowSymlinks::No);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                ScenarioError::ProviderConflictingBytes(path.into())
            } else {
                ScenarioError::UnsafeProviderEntry(format!("{path}: {error}"))
            }
        })?
        .into_std();
    validate_provider_regular_file(&file, path)?;
    Ok(file)
}

fn create_provider_journal_staging(
    parent: &Dir,
    name: &str,
    path: &str,
) -> Result<ProviderStagingFile, ScenarioError> {
    let file = create_provider_destination_exclusive(parent, name, path)?;
    Ok(ProviderStagingFile {
        file,
        name: Some(name.into()),
    })
}

fn validate_provider_open_file_bytes(
    file: &mut fs::File,
    expected: &[u8],
    path: &str,
) -> Result<(), ScenarioError> {
    validate_provider_regular_file(file, path)?;
    let expected_len =
        u64::try_from(expected.len()).map_err(|_| ScenarioError::ProviderRescanLimit)?;
    if file
        .metadata()
        .map_err(|error| ScenarioError::Io(error.to_string()))?
        .len()
        != expected_len
    {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    let mut actual = Vec::with_capacity(expected.len());
    Read::by_ref(file)
        .take(expected_len.saturating_add(1))
        .read_to_end(&mut actual)
        .map_err(|error| ScenarioError::Io(error.to_string()))?;
    if actual != expected {
        return Err(ScenarioError::UnsafeProviderEntry(path.into()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderPublicationDurabilityStep {
    Published,
    DestinationDirectorySynced,
    SourceDirectorySynced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderJournalBoundary {
    BeforeBlobDurable,
    BlobDurable,
    CreationRecordDurable,
    BlobInstalled,
    RecordDurable,
    UpdateDurable,
    UpdateInstalled,
    OrphanQuarantined,
    OrphanOwnershipRechecked,
    OrphanRestored,
    OrphanPrivateDeleted,
    RetirementPlaceholderDurable,
    RetirementExchangeDurable,
    RetirementPlaceholderQuarantined,
    RetirementPlaceholderPrivateDeleted,
    BlobRemoved,
    CompletionDurable,
    RecordRemoved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderPostValidationOperation {
    Rename,
    Remove,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderRemovalDurabilityStep {
    DeletePending,
    HandleDropped,
    DirectorySyncing,
}

fn sync_provider_directory(directory: &Dir) -> Result<(), ScenarioError> {
    sync_dir_required(directory).map_err(|error| ScenarioError::Io(error.to_string()))
}

#[cfg(unix)]
fn provider_lock_file_exclusive_nonblocking(file: &fs::File) -> std::io::Result<bool> {
    // SAFETY: flock only observes the retained authority-key descriptor.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn provider_unlock_file(file: &fs::File) {
    // SAFETY: flock only observes the retained authority-key descriptor.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn provider_lock_file_exclusive_nonblocking(file: &fs::File) -> std::io::Result<bool> {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, FALSE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and OVERLAPPED remain live for the synchronous call.
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != FALSE {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(windows)]
fn provider_unlock_file(file: &fs::File) {
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    let mut overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and OVERLAPPED remain live for the synchronous call.
    let _ = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as _,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
}

#[cfg(not(any(unix, windows)))]
fn provider_lock_file_exclusive_nonblocking(_file: &fs::File) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "provider retirement locking is unsupported",
    ))
}

#[cfg(not(any(unix, windows)))]
fn provider_unlock_file(_file: &fs::File) {}

/// The destination name is only considered published after its directory is
/// durable. A move-style operation also syncs the retained source/staging
/// directory after the destination, so recovery observes the same ordering.
fn sync_provider_publication_directories(
    destination_directory: &Dir,
    source_directory: Option<&Dir>,
) -> Result<(), ScenarioError> {
    sync_provider_directory(destination_directory)?;
    provider_publication_durability_hook(ProviderPublicationDurabilityStep::DestinationDirectorySynced);
    if let Some(source_directory) = source_directory {
        sync_provider_directory(source_directory)?;
        provider_publication_durability_hook(ProviderPublicationDurabilityStep::SourceDirectorySynced);
    }
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static FAIL_PROVIDER_PUBLICATION_AFTER_PHYSICAL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_PROVIDER_RENAME_AFTER_PHYSICAL_MOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PROVIDER_PUBLICATION_DURABILITY_STEPS: std::cell::RefCell<Vec<ProviderPublicationDurabilityStep>> = const { std::cell::RefCell::new(Vec::new()) };
    static PROVIDER_REMOVAL_DURABILITY_STEPS: std::cell::RefCell<Vec<ProviderRemovalDurabilityStep>> = const { std::cell::RefCell::new(Vec::new()) };
    static PROVIDER_POST_VALIDATION_HOOK: std::cell::RefCell<Option<(ProviderPostValidationOperation, Box<dyn FnOnce()>)>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_PUBLICATION_SOURCE_VALIDATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_RETIREMENT_VALIDATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_RETIREMENT_BEFORE_PRIVATE_MOVE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_RETIREMENT_BEFORE_PRIVATE_DELETE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_ORPHAN_AFTER_QUARANTINE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_ORPHAN_BEFORE_PRIVATE_DELETE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_FINISH_AFTER_GATE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static FAIL_PROVIDER_JOURNAL_AFTER_PHASE: std::cell::RefCell<Option<ProviderJournalPhase>> = const { std::cell::RefCell::new(None) };
    static FAIL_PROVIDER_JOURNAL_BOUNDARY: std::cell::RefCell<Option<ProviderJournalBoundary>> = const { std::cell::RefCell::new(None) };
    static PROVIDER_SCAN_ENTRY_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PROVIDER_SOURCE_INSPECTION_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderStagingMode {
    Automatic,
    NamedFallback,
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
std::thread_local! {
    static PROVIDER_STAGING_MODE: std::cell::Cell<ProviderStagingMode> = const { std::cell::Cell::new(ProviderStagingMode::Automatic) };
}

fn provider_finish_after_gate_hook() {
    #[cfg(test)]
    PROVIDER_FINISH_AFTER_GATE_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_source_inspection_visit() {
    #[cfg(test)]
    PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| visits.set(visits.get() + 1));
}

fn provider_post_validation_hook(_operation: ProviderPostValidationOperation) {
    #[cfg(test)]
    PROVIDER_POST_VALIDATION_HOOK.with(|hook| {
        let Some((expected, callback)) = hook.borrow_mut().take() else {
            return;
        };
        assert_eq!(expected, _operation, "provider validation hook operation");
        callback();
    });
}

fn provider_retirement_after_validation_hook() {
    #[cfg(test)]
    PROVIDER_RETIREMENT_VALIDATION_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_retirement_before_private_move_hook() {
    #[cfg(test)]
    PROVIDER_RETIREMENT_BEFORE_PRIVATE_MOVE_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_retirement_before_private_delete_hook() {
    #[cfg(test)]
    PROVIDER_RETIREMENT_BEFORE_PRIVATE_DELETE_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_orphan_after_quarantine_hook() {
    #[cfg(test)]
    PROVIDER_ORPHAN_AFTER_QUARANTINE_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_orphan_before_private_delete_hook() {
    #[cfg(test)]
    PROVIDER_ORPHAN_BEFORE_PRIVATE_DELETE_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

fn provider_publication_source_after_validation_hook() {
    #[cfg(test)]
    PROVIDER_PUBLICATION_SOURCE_VALIDATION_HOOK.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

#[cfg(test)]
fn provider_journal_after_phase_hook(phase: ProviderJournalPhase) -> Result<(), ScenarioError> {
    let fail = FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| {
        if hook.borrow().as_ref() == Some(&phase) {
            hook.borrow_mut().take();
            true
        } else {
            false
        }
    });
    if fail {
        Err(ScenarioError::Io(format!(
            "injected provider journal crash after {phase:?}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn provider_journal_after_phase_hook(_phase: ProviderJournalPhase) -> Result<(), ScenarioError> {
    Ok(())
}

#[cfg(test)]
fn provider_journal_boundary_hook(boundary: ProviderJournalBoundary) -> Result<(), ScenarioError> {
    let fail = FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| {
        if hook.borrow().as_ref() == Some(&boundary) {
            hook.borrow_mut().take();
            true
        } else {
            false
        }
    });
    if fail {
        Err(ScenarioError::Io(format!(
            "injected provider journal crash at {boundary:?}"
        )))
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn provider_journal_boundary_hook(_boundary: ProviderJournalBoundary) -> Result<(), ScenarioError> {
    Ok(())
}

fn provider_scan_entry_visit() {
    #[cfg(test)]
    PROVIDER_SCAN_ENTRY_VISITS.with(|visits| visits.set(visits.get().saturating_add(1)));
}

#[cfg(test)]
fn provider_publication_after_publish_hook() -> Result<(), ScenarioError> {
    provider_publication_durability_hook(ProviderPublicationDurabilityStep::Published);
    if FAIL_PROVIDER_PUBLICATION_AFTER_PHYSICAL_WRITE.with(|hook| hook.replace(false)) {
        return Err(ScenarioError::Io(
            "injected provider publication validation failure".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn provider_rename_after_move_hook() -> Result<(), ScenarioError> {
    provider_publication_durability_hook(ProviderPublicationDurabilityStep::Published);
    if FAIL_PROVIDER_RENAME_AFTER_PHYSICAL_MOVE.with(|hook| hook.replace(false)) {
        return Err(ScenarioError::Io(
            "injected provider rename validation failure".into(),
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn provider_rename_after_move_hook() -> Result<(), ScenarioError> {
    provider_publication_durability_hook(ProviderPublicationDurabilityStep::Published);
    Ok(())
}

#[cfg(not(test))]
fn provider_publication_after_publish_hook() -> Result<(), ScenarioError> {
    provider_publication_durability_hook(ProviderPublicationDurabilityStep::Published);
    Ok(())
}

#[cfg(test)]
fn provider_publication_durability_hook(step: ProviderPublicationDurabilityStep) {
    PROVIDER_PUBLICATION_DURABILITY_STEPS.with(|steps| steps.borrow_mut().push(step));
}

#[cfg(not(test))]
fn provider_publication_durability_hook(_step: ProviderPublicationDurabilityStep) {}

#[cfg(test)]
fn provider_removal_durability_hook(step: ProviderRemovalDurabilityStep) {
    PROVIDER_REMOVAL_DURABILITY_STEPS.with(|steps| steps.borrow_mut().push(step));
}

#[cfg(not(test))]
#[allow(dead_code)]
fn provider_removal_durability_hook(_step: ProviderRemovalDurabilityStep) {}

#[cfg_attr(not(test), allow(dead_code))]
fn close_provider_delete_pending_file(file: fs::File) {
    provider_removal_durability_hook(ProviderRemovalDurabilityStep::DeletePending);
    drop(file);
    provider_removal_durability_hook(ProviderRemovalDurabilityStep::HandleDropped);
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn create_provider_file_exclusive(
    parent: &Dir,
    _name: &str,
    path: &str,
) -> Result<ProviderStagingFile, ScenarioError> {
    // Linux and Android can stage anonymously.  The later /proc descriptor
    // link is handle-bound, so there is no staging pathname to replace or
    // clean up after publication.
    #[cfg(test)]
    let staging_mode = PROVIDER_STAGING_MODE.with(std::cell::Cell::get);
    #[cfg(test)]
    if staging_mode == ProviderStagingMode::NamedFallback {
        return create_provider_file_named_exclusive(parent, _name, path);
    }
    let fd = unsafe {
        libc::openat(
            parent.as_fd().as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_TMPFILE,
            0o600,
        )
    };
    if fd < 0 {
        // Some sandbox and network filesystems do not implement O_TMPFILE.
        // The named fallback is consumed by an atomic no-replace rename and
        // then tied back to this retained handle before success is reported.
        return create_provider_file_named_exclusive(parent, _name, path);
    }
    // SAFETY: openat returned a new owned regular-file descriptor.
    let file = unsafe { fs::File::from_raw_fd(fd) };
    // O_TMPFILE deliberately has no directory entry yet, so Linux reports a
    // zero link count. It is still a regular, owned descriptor and becomes
    // publishable only through the retained handle below.
    let _ = validate_provider_regular_file_with_link_count(&file, path, false)?;
    Ok(ProviderStagingFile { file, name: None })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn create_provider_file_named_exclusive(
    parent: &Dir,
    name: &str,
    path: &str,
) -> Result<ProviderStagingFile, ScenarioError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderEntry(format!("{path}: {error}")))?
        .into_std();
    let _ = validate_provider_regular_file(&file, path)?;
    Ok(ProviderStagingFile {
        file,
        name: Some(name.into()),
    })
}

#[cfg(windows)]
fn create_provider_file_exclusive(
    parent: &Dir,
    name: &str,
    path: &str,
) -> Result<ProviderStagingFile, ScenarioError> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    // FileRenameInfo requires DELETE access on the file being renamed. Share
    // delete as well so receiver-style readers never turn a publish into a
    // transient sharing violation.
    options
        .read(true)
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderEntry(format!("{path}: {error}")))?
        .into_std();
    let _ = validate_provider_regular_file(&file, path)?;
    Ok(ProviderStagingFile {
        file,
        name: Some(name.into()),
    })
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn create_provider_file_exclusive(
    parent: &Dir,
    name: &str,
    path: &str,
) -> Result<ProviderStagingFile, ScenarioError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = parent
        .open_with(name, &options)
        .map_err(|error| ScenarioError::UnsafeProviderEntry(format!("{path}: {error}")))?
        .into_std();
    let _ = validate_provider_regular_file(&file, path)?;
    Ok(ProviderStagingFile {
        file,
        name: Some(name.into()),
    })
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
fn provider_publish_staged_file_noreplace(
    staged: &ProviderStagingFile,
    source_dir: Option<&Dir>,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    if let Some(source_name) = staged.name.as_deref() {
        let source_dir = source_dir.ok_or_else(|| {
            std::io::Error::new(ErrorKind::InvalidInput, "missing provider staging directory")
        })?;
        return provider_rename_named_noreplace(
            source_dir,
            source_name,
            destination_dir,
            destination_name,
        );
    }
    let file = &staged.file;
    let source = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider handle"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider target"))?;
    // SAFETY: `source` is this process's immutable descriptor indirection and
    // `destination` is resolved beneath the retained destination capability.
    // AT_SYMLINK_FOLLOW makes linkat bind that opened descriptor's object,
    // rather than the replaceable `.part` pathname checked above.
    let result = unsafe {
        libc::linkat(
            libc::AT_FDCWD,
            source.as_ptr(),
            destination_dir.as_fd().as_raw_fd(),
            destination.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn provider_rename_handle_noreplace(
    file: &fs::File,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{FileRenameInfo, SetFileInformationByHandle};

    #[repr(C)]
    struct RenameInformation {
        replace_if_exists: u8,
        root_directory: HANDLE,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    let destination: Vec<u16> = OsStr::new(destination_name).encode_wide().collect();
    if destination.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "empty provider target",
        ));
    }
    let destination_bytes = destination
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "provider target too long"))?;
    let length = std::mem::size_of::<RenameInformation>()
        .checked_add(destination_bytes)
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "provider target too long"))?;
    // `FILE_RENAME_INFO` contains a HANDLE, so a Vec<u8> does not provide the
    // alignment required to cast its storage to the C layout. usize storage is
    // aligned for every field in RenameInformation and is rounded up to cover
    // the variable UTF-16 tail.
    let words = length.div_ceil(std::mem::size_of::<usize>());
    let mut information = vec![0_usize; words];
    let root = destination_dir.try_clone()?.into_std_file();
    let rename = information.as_mut_ptr().cast::<RenameInformation>();
    // SAFETY: `information` is aligned for RenameInformation and has at least
    // `length` initialized bytes for FILE_RENAME_INFO plus the UTF-16 tail.
    // Both handles remain live for the call, which atomically renames the
    // object selected by `file` itself.
    unsafe {
        (*rename).replace_if_exists = 0;
        (*rename).root_directory = root.as_raw_handle();
        (*rename).file_name_length = u32::try_from(destination_bytes).map_err(|_| {
            std::io::Error::new(ErrorKind::InvalidInput, "provider target too long")
        })?;
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*rename).file_name.as_mut_ptr(),
            destination.len(),
        );
        if SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            rename.cast(),
            u32::try_from(length).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidInput, "provider target too long")
            })?,
        ) != 0
        {
            return Ok(());
        }
    }
    Err(std::io::Error::last_os_error())
}

#[cfg(all(test, not(any(target_os = "linux", target_os = "android"))))]
fn provider_publish_staged_file_noreplace(
    _staged: &ProviderStagingFile,
    _source_dir: Option<&Dir>,
    _destination_dir: &Dir,
    _destination_name: &str,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "handle-bound provider publication is unsupported",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn provider_rename_named_noreplace(
    source_dir: &Dir,
    source_name: &str,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    let source = CString::new(source_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider source"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider target"))?;
    // SAFETY: both names are live C strings and are resolved relative to
    // retained directory capabilities. RENAME_NOREPLACE atomically consumes
    // the exact source name without overwriting a destination.
    let result = unsafe {
        libc::renameat2(
            source_dir.as_fd().as_raw_fd(),
            source.as_ptr(),
            destination_dir.as_fd().as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_NOREPLACE as _,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn provider_rename_named_noreplace(
    source_dir: &Dir,
    source_name: &str,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    let file = open_provider_file_nofollow(source_dir, source_name)?;
    validate_provider_regular_file(&file, source_name)
        .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error.to_string()))?;
    provider_rename_handle_noreplace(&file, destination_dir, destination_name)?;
    match open_provider_file_nofollow(source_dir, source_name) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(std::io::Error::new(
            ErrorKind::Other,
            "provider source was replaced during diagnostic move",
        )),
        Err(error) => Err(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn provider_exchange_names(
    source_dir: &Dir,
    source_name: &str,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    let source = CString::new(source_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider source"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider target"))?;
    // SAFETY: both names are live and capability-relative. RENAME_EXCHANGE
    // ensures that any racing replacement is preserved in diagnostic storage.
    let result = unsafe {
        libc::renameat2(
            source_dir.as_fd().as_raw_fd(),
            source.as_ptr(),
            destination_dir.as_fd().as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCHANGE as _,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn provider_rename_named_noreplace(
    source_dir: &Dir,
    source_name: &str,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    let source = CString::new(source_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider source"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider target"))?;
    // SAFETY: both names are live C strings and both directory descriptors
    // remain live for the atomic exclusive rename.
    let result = unsafe {
        libc::renameatx_np(
            source_dir.as_fd().as_raw_fd(),
            source.as_ptr(),
            destination_dir.as_fd().as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn provider_exchange_names(
    source_dir: &Dir,
    source_name: &str,
    destination_dir: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    let source = CString::new(source_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider source"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "invalid provider target"))?;
    // SAFETY: both names are live and capability-relative; RENAME_SWAP
    // atomically preserves either the validated source or a racing replacement.
    let result = unsafe {
        libc::renameatx_np(
            source_dir.as_fd().as_raw_fd(),
            source.as_ptr(),
            destination_dir.as_fd().as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
#[allow(dead_code)]
fn provider_remove_open_file(file: fs::File) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };

    let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the retained handle selects the validated file object, the
    // disposition structure is initialized for the exact call size, and the
    // kernel retains neither pointer after the call.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&mut disposition as *mut FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if result != 0 {
        close_provider_delete_pending_file(file);
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

struct ProviderDiskFile {
    path: String,
    bytes: Vec<u8>,
    temporary: bool,
}

fn bounded_provider_files(
    root: &Dir,
    include_temporary: bool,
    entry_limit: usize,
    byte_limit: usize,
) -> Result<Vec<ProviderDiskFile>, ScenarioError> {
    fn walk(
        directory: &Dir,
        prefix: &str,
        depth: usize,
        include_temporary: bool,
        entry_limit: usize,
        byte_limit: usize,
        entries: &mut usize,
        bytes: &mut usize,
        files: &mut Vec<ProviderDiskFile>,
    ) -> Result<(), ScenarioError> {
        if depth > MAX_PROVIDER_RESCAN_DEPTH {
            return Err(ScenarioError::ProviderRescanLimit);
        }
        for entry in directory
            .entries()
            .map_err(|error| ScenarioError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| ScenarioError::Io(error.to_string()))?;
            provider_scan_entry_visit();
            *entries = entries
                .checked_add(1)
                .ok_or(ScenarioError::ProviderRescanLimit)?;
            if *entries > entry_limit {
                return Err(ScenarioError::ProviderRescanLimit);
            }
            let name = entry.file_name().into_string().map_err(|_| {
                ScenarioError::UnsafeProviderEntry("non-UTF-8 provider entry".into())
            })?;
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if relative.len() > MAX_PROVIDER_PATH_BYTES || !valid_provider_path(&relative) {
                return Err(ScenarioError::ProviderRescanLimit);
            }
            let file_type = entry
                .file_type()
                .map_err(|error| ScenarioError::Io(error.to_string()))?;
            if file_type.is_symlink() {
                return Err(ScenarioError::UnsafeProviderEntry(relative));
            }
            if file_type.is_dir() {
                if prefix.is_empty() && name == PROVIDER_TEMP_NAMESPACE && !include_temporary {
                    let _ = open_provider_directory(directory, &name)?;
                    continue;
                }
                if depth >= MAX_PROVIDER_RESCAN_DEPTH {
                    return Err(ScenarioError::ProviderRescanLimit);
                }
                let child = open_provider_directory(directory, &name)?;
                walk(
                    &child,
                    &relative,
                    depth + 1,
                    include_temporary,
                    entry_limit,
                    byte_limit,
                    entries,
                    bytes,
                    files,
                )?;
            } else if file_type.is_file() {
                if prefix.is_empty()
                    && [
                        PROVIDER_OBJECTS_NAMESPACE,
                        PROVIDER_MANIFESTS_NAMESPACE,
                        PROVIDER_TEMP_NAMESPACE,
                        PROVIDER_REMOVED_NAMESPACE,
                        PROVIDER_RENAME_EVIDENCE_NAMESPACE,
                    ]
                    .contains(&name.as_str())
                {
                    return Err(ScenarioError::UnsafeProviderEntry(relative));
                }
                let temporary = relative
                    .split('/')
                    .next()
                    .is_some_and(|namespace| namespace == PROVIDER_TEMP_NAMESPACE);
                let remaining = byte_limit
                    .checked_sub(*bytes)
                    .ok_or(ScenarioError::ProviderRescanLimit)?;
                let opened = open_provider_regular_optional(
                    directory,
                    &name,
                    remaining,
                    &relative,
                )?
                .ok_or_else(|| ScenarioError::UnknownProviderPath(relative.clone()))?;
                let file_bytes = opened.bytes;
                *bytes = bytes
                    .checked_add(file_bytes.len())
                    .ok_or(ScenarioError::ProviderRescanLimit)?;
                files.push(ProviderDiskFile {
                    path: relative,
                    bytes: file_bytes,
                    temporary,
                });
            } else {
                return Err(ScenarioError::UnsafeProviderEntry(relative));
            }
        }
        Ok(())
    }
    let mut entries = 0;
    let mut bytes = 0;
    let mut files = Vec::new();
    walk(
        root,
        "",
        0,
        include_temporary,
        entry_limit,
        byte_limit,
        &mut entries,
        &mut bytes,
        &mut files,
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(event_id: u64, action: ScheduledActionKind) -> ScheduledAction {
        ScheduledAction {
            event_id,
            tick: event_id,
            action,
        }
    }

    fn location(path: &str) -> ProviderLocation {
        ProviderLocation {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            path: path.into(),
        }
    }

    fn simulator_with_provider_item(bytes: &[u8]) -> DeterministicSimulator {
        let workspace = ScenarioWorkspace {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(1)),
            lineage_digest: LineageDigest::of(b"provider-publication-identity"),
            catalog_document_id: DocumentId::from_uuid(Uuid::from_u128(2)),
        };
        let scenario = Scenario::from_schedule(
            "provider-publication-identity",
            1,
            workspace,
            vec![ScenarioDevice {
                name: "beta".into(),
                device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                crdt_peer_id: CrdtPeerId::from_u64(1),
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        simulator
            .mailbox
            .insert(
                "fixture-object".into(),
                ProviderItem {
                    batch_id: None,
                    kind: ProviderItemKind::Object,
                    bytes: Arc::from(bytes.to_vec()),
                },
            )
            .unwrap();
        simulator
    }

    fn simulator_with_cross_device_provider_item(bytes: &[u8]) -> DeterministicSimulator {
        let workspace = ScenarioWorkspace {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(11)),
            lineage_digest: LineageDigest::of(b"provider-cross-device-gates"),
            catalog_document_id: DocumentId::from_uuid(Uuid::from_u128(12)),
        };
        let scenario = Scenario::from_schedule(
            "provider-cross-device-gates",
            1,
            workspace,
            vec![
                ScenarioDevice {
                    name: "alpha".into(),
                    device_id: DeviceId::from_uuid(Uuid::from_u128(13)),
                    crdt_peer_id: CrdtPeerId::from_u64(1),
                },
                ScenarioDevice {
                    name: "beta".into(),
                    device_id: DeviceId::from_uuid(Uuid::from_u128(14)),
                    crdt_peer_id: CrdtPeerId::from_u64(2),
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        simulator
            .mailbox
            .insert(
                "fixture-object".into(),
                ProviderItem {
                    batch_id: None,
                    kind: ProviderItemKind::Object,
                    bytes: Arc::from(bytes.to_vec()),
                },
            )
            .unwrap();
        simulator
    }

    fn cross_device_source() -> ProviderLocation {
        ProviderLocation {
            device: "alpha".into(),
            tree: ProviderTree::Outbox,
            path: "objects/source".into(),
        }
    }

    fn seed_cross_device_source(
        simulator: &mut DeterministicSimulator,
    ) -> ProviderLocation {
        let source = cross_device_source();
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: source.clone(),
                },
            ))
            .unwrap();
        source
    }

    fn assert_no_destination_transaction_residue(
        simulator: &DeterministicSimulator,
        destination_path: &str,
    ) {
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert!(!inbox.join(destination_path).exists());
        assert_eq!(
            std::fs::read_dir(inbox.join(PROVIDER_TEMP_NAMESPACE))
                .unwrap()
                .count(),
            0
        );
        let journal = simulator.provider_journal_path("beta").unwrap();
        for namespace in ["records", "blobs", "quarantine", "completed"] {
            assert_eq!(
                std::fs::read_dir(journal.join(namespace)).unwrap().count(),
                0,
                "{namespace}"
            );
        }
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .is_empty());
    }

    fn begin_provider_write(
        simulator: &mut DeterministicSimulator,
        transfer_id: &str,
        destination: ProviderLocation,
    ) {
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination,
                    transfer_id: transfer_id.into(),
                },
            ))
            .unwrap();
    }

    fn append_provider_write(
        simulator: &mut DeterministicSimulator,
        transfer_id: &str,
        len: usize,
    ) {
        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: transfer_id.into(),
                    len,
                },
            ))
            .unwrap();
    }

    fn finish_provider_write(
        simulator: &mut DeterministicSimulator,
        transfer_id: &str,
    ) -> Result<(), ScenarioError> {
        simulator.run_action(&action(
            3,
            ScheduledActionKind::FinishProviderWrite {
                device: "beta".into(),
                transfer_id: transfer_id.into(),
            },
        ))
    }

    #[test]
    fn provider_finish_uses_retained_handle_and_leaves_replacement_untouched() {
        let bytes = b"honest provider bytes";
        let destination = location("objects/final");
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(&mut simulator, "write", destination.clone());
        append_provider_write(&mut simulator, "write", bytes.len());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let temporary = inbox.join(".part/write.part");
        let named = temporary.exists();
        if named {
            let retained_name = inbox.join(".part/retained-original");
            std::fs::rename(&temporary, &retained_name).unwrap();
            std::fs::write(&temporary, b"attacker replacement").unwrap();
        }
        let result = finish_provider_write(&mut simulator, "write");
        if named {
            result.unwrap();
            assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
            assert_eq!(
                std::fs::read(inbox.join(".part/retained-original")).unwrap(),
                bytes
            );
            assert!(std::fs::read_dir(inbox.join(PROVIDER_TEMP_NAMESPACE))
                .unwrap()
                .any(|entry| std::fs::read(entry.unwrap().path()).unwrap()
                    == b"attacker replacement"));
            assert!(!simulator
                .device("beta")
                .unwrap()
                .provider
                .writes
                .contains_key("write"));
        } else {
            result.unwrap();
            assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
            assert!(!simulator
                .device("beta")
                .unwrap()
                .provider
                .writes
                .contains_key("write"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn provider_finish_never_reads_symlink_or_special_temp_replacements() {
        use std::os::unix::fs::symlink;

        for replacement in ["symlink", "fifo"] {
            let bytes = b"honest provider bytes";
            let mut simulator = simulator_with_provider_item(bytes);
            begin_provider_write(&mut simulator, "write", location("objects/final"));
            append_provider_write(&mut simulator, "write", bytes.len());
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            let temporary = inbox.join(".part/write.part");
            if !temporary.exists() {
                // O_TMPFILE staging has no pathname to swap, which is the
                // stronger form of this guarantee.
                finish_provider_write(&mut simulator, "write").unwrap();
                continue;
            }
            std::fs::remove_file(&temporary).unwrap();
            match replacement {
                "symlink" => {
                    let target = inbox.join(".part/symlink-target");
                    std::fs::write(&target, b"attacker replacement").unwrap();
                    symlink(&target, &temporary).unwrap();
                }
                "fifo" => {
                    let name = CString::new(temporary.as_os_str().as_encoded_bytes()).unwrap();
                    // SAFETY: `name` is a live NUL-terminated pathname and
                    // mkfifo does not retain it.  If the host filesystem does
                    // not offer FIFOs, this is an unavailable special-file
                    // case rather than a simulator failure.
                    if unsafe { libc::mkfifo(name.as_ptr(), 0o600) } != 0 {
                        let error = std::io::Error::last_os_error();
                        if matches!(error.raw_os_error(), Some(libc::EPERM | libc::EOPNOTSUPP)) {
                            continue;
                        }
                        panic!("create FIFO replacement: {error}");
                    }
                }
                _ => unreachable!(),
            }

            assert!(matches!(
                finish_provider_write(&mut simulator, "write"),
                Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/final"
            ));
            assert!(!inbox.join("objects/final").exists(), "{replacement}");
            assert!(std::fs::symlink_metadata(&temporary).is_ok(), "{replacement}");
            assert!(simulator
                .device("beta")
                .unwrap()
                .provider
                .writes
                .contains_key("write"));
        }
    }

    #[test]
    fn partial_provider_finish_preserves_handle_for_append_and_retry() {
        let bytes = b"partial provider bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(&mut simulator, "write", location("objects/final"));
        let split = bytes.len() / 2;
        append_provider_write(&mut simulator, "write", split);

        assert!(matches!(
            finish_provider_write(&mut simulator, "write"),
            Err(ScenarioError::PartialProviderWrite(id)) if id == "write"
        ));
        assert_eq!(
            simulator.device("beta").unwrap().provider.writes["write"].next,
            split
        );
        append_provider_write(&mut simulator, "write", bytes.len() - split);
        finish_provider_write(&mut simulator, "write").unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
    }

    #[test]
    fn provider_finish_conflict_keeps_honest_temp_and_never_claims_success() {
        let bytes = b"honest provider bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(&mut simulator, "write", location("objects/final"));
        append_provider_write(&mut simulator, "write", bytes.len());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let destination = inbox.join("objects/final");
        std::fs::write(&destination, b"conflicting destination").unwrap();

        assert!(matches!(
            finish_provider_write(&mut simulator, "write"),
            Err(ScenarioError::ProviderConflictingBytes(path)) if path == "objects/final"
        ));
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"conflicting destination"
        );
        if let Ok(staged) = std::fs::read(inbox.join(".part/write.part")) {
            assert_eq!(staged, bytes);
        }
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("write"));

        std::fs::remove_file(&destination).unwrap();
        finish_provider_write(&mut simulator, "write").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    }

    #[test]
    fn provider_finish_rejects_an_unrelated_identical_destination() {
        let bytes = b"idempotent provider bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(&mut simulator, "write", location("objects/final"));
        append_provider_write(&mut simulator, "write", bytes.len());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::write(inbox.join("objects/final"), bytes).unwrap();

        assert!(matches!(
            finish_provider_write(&mut simulator, "write"),
            Err(ScenarioError::ProviderConflictingBytes(path)) if path == "objects/final"
        ));
        assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("write"));
    }

    #[test]
    fn provider_copy_rejects_an_unrelated_identical_destination() {
        let bytes = b"idempotent provider bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::write(inbox.join("objects/final"), bytes).unwrap();
        assert!(matches!(
            simulator.run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/final"),
                },
            )),
            Err(ScenarioError::ProviderConflictingBytes(path)) if path == "objects/final"
        ));
        assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
    }

    #[test]
    fn adversarial_provider_walk_stops_exactly_at_entry_cap_plus_one() {
        let simulator = simulator_with_provider_item(b"scan cap");
        PROVIDER_SCAN_ENTRY_VISITS.with(|visits| visits.set(0));
        assert!(matches!(
            bounded_provider_files(
                simulator
                    .device("beta")
                    .unwrap()
                    .provider
                    .tree(ProviderTree::Inbox),
                true,
                3,
                MAX_PROVIDER_RESCAN_BYTES,
            ),
            Err(ScenarioError::ProviderRescanLimit)
        ));
        assert_eq!(PROVIDER_SCAN_ENTRY_VISITS.with(std::cell::Cell::get), 4);
    }

    #[test]
    fn provider_finish_retries_after_physical_publication_validation_error() {
        let bytes = b"retry after physical publication";
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(&mut simulator, "write", location("objects/final"));
        append_provider_write(&mut simulator, "write", bytes.len());
        let named = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap()
            .join(".part/write.part")
            .exists();
        FAIL_PROVIDER_PUBLICATION_AFTER_PHYSICAL_WRITE.with(|hook| hook.set(true));

        assert!(matches!(
            finish_provider_write(&mut simulator, "write"),
            Err(ScenarioError::Io(message)) if message.contains("injected provider publication")
        ));
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("write"));

        let _ = named;
        finish_provider_write(&mut simulator, "write").unwrap();
        assert!(!simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("write"));
    }

    #[test]
    fn complete_copy_publication_rejects_a_replaced_named_staging_source() {
        let bytes = b"complete copy bytes";
        let simulator = simulator_with_provider_item(bytes);
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let runtime = simulator.device("beta").unwrap();
        let temporary_dir = open_provider_directory(
            runtime.provider.tree(ProviderTree::Inbox),
            PROVIDER_TEMP_NAMESPACE,
        )
        .unwrap();
        let temporary_name = "complete-copy.part";
        let mut temporary =
            create_provider_file_exclusive(&temporary_dir, temporary_name, "objects/final")
                .unwrap();
        temporary.write_all(bytes).unwrap();
        temporary.sync_all().unwrap();
        let (destination_dir, destination_name) = runtime
            .provider
            .parent_and_name(ProviderTree::Inbox, "objects/final", true)
            .unwrap();
        let temporary_path = inbox.join(format!("{PROVIDER_TEMP_NAMESPACE}/{temporary_name}"));
        let named = temporary_path.exists();
        if named {
            let retained_name = inbox.join(format!("{PROVIDER_TEMP_NAMESPACE}/retained-complete"));
            std::fs::rename(&temporary_path, &retained_name).unwrap();
            std::fs::write(&temporary_path, b"replacement complete copy").unwrap();
        }

        let result = publish_provider_file_noreplace(
            &temporary,
            &destination_dir,
            &destination_name,
            bytes,
            "objects/final",
            Some(&temporary_dir),
        );
        if named {
            result.unwrap();
            assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
            assert_eq!(
                std::fs::read(inbox.join(".part/retained-complete")).unwrap(),
                bytes
            );
            assert!(std::fs::read_dir(inbox.join(PROVIDER_TEMP_NAMESPACE))
                .unwrap()
                .any(|entry| std::fs::read(entry.unwrap().path()).unwrap()
                    == b"replacement complete copy"));
        } else {
            result.unwrap();
            assert_eq!(std::fs::read(inbox.join("objects/final")).unwrap(), bytes);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn forced_anonymous_staging_validation_accepts_zero_link_count() {
        use std::os::unix::fs::MetadataExt as _;

        // Some test filesystems do not implement O_TMPFILE. Unlinking a named
        // test file while retaining its descriptor deterministically produces
        // the same zero-link validation boundary without depending on temp
        // residue or host O_TMPFILE support.
        let bytes = b"anonymous staging bytes";
        let simulator = simulator_with_provider_item(bytes);
        let runtime = simulator.device("beta").unwrap();
        let staging = open_provider_directory(
            runtime.provider.tree(ProviderTree::Inbox),
            PROVIDER_TEMP_NAMESPACE,
        )
        .unwrap();
        let temporary =
            create_provider_file_named_exclusive(&staging, "anonymous.part", "objects/anonymous")
                .unwrap();
        staging.remove_file("anonymous.part").unwrap();
        assert_eq!(temporary.metadata().unwrap().nlink(), 0);
        validate_provider_regular_file_with_link_count(&temporary, "objects/anonymous", false)
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn forced_named_fallback_staging_publishes_without_residue_assumptions() {
        use std::os::unix::fs::MetadataExt as _;

        let bytes = b"named fallback staging bytes";
        let simulator = simulator_with_provider_item(bytes);
        let runtime = simulator.device("beta").unwrap();
        let staging = open_provider_directory(
            runtime.provider.tree(ProviderTree::Inbox),
            PROVIDER_TEMP_NAMESPACE,
        )
        .unwrap();
        let previous = PROVIDER_STAGING_MODE.with(|mode| mode.replace(ProviderStagingMode::NamedFallback));
        let result = (|| {
            let mut temporary =
                create_provider_file_exclusive(&staging, "named.part", "objects/named")?;
            assert_eq!(temporary.metadata().unwrap().nlink(), 1);
            temporary.write_all(bytes).unwrap();
            temporary.sync_all().unwrap();
            let (destination_dir, destination_name) = runtime
                .provider
                .parent_and_name(ProviderTree::Inbox, "objects/named", true)?;
            PROVIDER_PUBLICATION_DURABILITY_STEPS.with(|steps| steps.borrow_mut().clear());
            publish_provider_file_noreplace(
                &temporary,
                &destination_dir,
                &destination_name,
                bytes,
                "objects/named",
                Some(&staging),
            )?;
            let inbox = runtime.provider.tree_path(ProviderTree::Inbox);
            assert!(!inbox.join(".part/named.part").exists());
            assert!(std::fs::read_dir(inbox.join(PROVIDER_TEMP_NAMESPACE))
                .unwrap()
                .any(|entry| std::fs::read(entry.unwrap().path()).unwrap() == bytes));
            assert_eq!(std::fs::read(inbox.join("objects/named")).unwrap(), bytes);
            assert_eq!(
                std::fs::metadata(inbox.join("objects/named"))
                    .unwrap()
                    .nlink(),
                1
            );
            std::fs::write(inbox.join(".part/named.part"), b"later staging bytes").unwrap();
            assert_eq!(std::fs::read(inbox.join("objects/named")).unwrap(), bytes);
            assert_eq!(
                PROVIDER_PUBLICATION_DURABILITY_STEPS.with(|steps| steps.borrow().clone()),
                vec![
                    ProviderPublicationDurabilityStep::Published,
                    ProviderPublicationDurabilityStep::DestinationDirectorySynced,
                    ProviderPublicationDurabilityStep::SourceDirectorySynced,
                ]
            );
            Ok::<(), ScenarioError>(())
        })();
        PROVIDER_STAGING_MODE.with(|mode| mode.set(previous));
        result.unwrap();
    }

    #[test]
    fn provider_same_path_rename_is_a_validated_noop() {
        let bytes = b"same path rename bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/same"),
                },
            ))
            .unwrap();

        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/same".into(),
                    to_path: "objects/same".into(),
                },
            ))
            .unwrap();

        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(std::fs::read(inbox.join("objects/same")).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(inbox.join(PROVIDER_RENAME_EVIDENCE_NAMESPACE))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn provider_rename_retry_reconciles_destination_before_source_reopen() {
        let bytes = b"published rename retry bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        FAIL_PROVIDER_RENAME_AFTER_PHYSICAL_MOVE.with(|hook| hook.set(true));
        let rename = || ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };

        assert!(matches!(
            simulator.run_action(&action(2, rename())),
            Err(ScenarioError::Io(message)) if message.contains("injected provider rename")
        ));
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert!(!inbox.join("objects/source").exists());
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );

        simulator.run_action(&action(2, rename())).unwrap();
        assert_eq!(
            std::fs::read_dir(inbox.join(PROVIDER_RENAME_EVIDENCE_NAMESPACE))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(
                simulator
                    .provider_journal_path("beta")
                    .unwrap()
                    .join("records")
            )
            .unwrap()
            .count(),
            0
        );
    }

    #[test]
    fn provider_rename_retry_rejects_a_conflicting_published_destination() {
        let bytes = b"published rename conflict bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        FAIL_PROVIDER_RENAME_AFTER_PHYSICAL_MOVE.with(|hook| hook.set(true));
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        assert!(simulator.run_action(&action(2, rename.clone())).is_err());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::write(inbox.join("objects/destination"), b"attacker conflict").unwrap();

        assert!(matches!(
            simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnsafeProviderEntry(path))
                if path == "objects/destination"
        ));
        assert!(!inbox.join("objects/destination").exists());
        assert!(std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
            .unwrap()
            .any(|entry| std::fs::read(entry.unwrap().path()).unwrap()
                == b"attacker conflict"));
    }

    #[test]
    fn crash_restart_reconciles_rename_from_disk_after_process_state_loss() {
        let bytes = b"crash state loss bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        FAIL_PROVIDER_RENAME_AFTER_PHYSICAL_MOVE.with(|hook| hook.set(true));
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        assert!(simulator.run_action(&action(2, rename.clone())).is_err());
        begin_provider_write(
            &mut simulator,
            "volatile",
            location("objects/volatile"),
        );
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("volatile"));

        simulator.device_mut("beta").unwrap().crash();
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .is_empty());
        simulator
            .run_action(&action(
                4,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        simulator.run_action(&action(2, rename)).unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
    }

    #[test]
    fn provider_rename_recovers_from_every_durable_journal_phase() {
        let bytes = b"phase durable rename bytes";
        for phase in [
            ProviderJournalPhase::Prepared,
            ProviderJournalPhase::Staged,
            ProviderJournalPhase::PublishIntent,
            ProviderJournalPhase::Published,
            ProviderJournalPhase::RetireIntent,
            ProviderJournalPhase::Retired,
            ProviderJournalPhase::Cleanup,
        ] {
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            let rename = ScheduledActionKind::ProviderRename {
                device: "beta".into(),
                tree: ProviderTree::Inbox,
                from_path: "objects/source".into(),
                to_path: "objects/destination".into(),
            };
            let journal = simulator.provider_journal_path("beta").unwrap();
            FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| hook.replace(Some(phase)));
            assert!(matches!(
                simulator.run_action(&action(2, rename.clone())),
                Err(ScenarioError::Io(message)) if message.contains("journal crash")
            ));
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    3,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            simulator.run_action(&action(2, rename)).unwrap();
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{phase:?}"
            );
            assert!(!inbox.join("objects/source").exists(), "{phase:?}");
            assert_eq!(
                std::fs::read_dir(journal.join("records")).unwrap().count(),
                0,
                "{phase:?}"
            );
            assert_eq!(
                std::fs::read_dir(journal.join("blobs")).unwrap().count(),
                0,
                "{phase:?}"
            );
        }
    }

    #[test]
    fn provider_remove_recovers_from_every_durable_journal_phase() {
        let bytes = b"phase durable remove bytes";
        for phase in [
            ProviderJournalPhase::Prepared,
            ProviderJournalPhase::RetireIntent,
            ProviderJournalPhase::Retired,
            ProviderJournalPhase::Cleanup,
        ] {
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            let remove = ScheduledActionKind::ProviderRemove {
                location: location("objects/source"),
            };
            let journal = simulator.provider_journal_path("beta").unwrap();
            FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| hook.replace(Some(phase)));
            assert!(matches!(
                simulator.run_action(&action(2, remove.clone())),
                Err(ScenarioError::Io(message)) if message.contains("journal crash")
            ));
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    3,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            simulator.run_action(&action(2, remove)).unwrap();
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert!(!inbox.join("objects/source").exists(), "{phase:?}");
            assert_eq!(
                std::fs::read_dir(journal.join("records")).unwrap().count(),
                0,
                "{phase:?}"
            );
        }
    }

    #[test]
    fn provider_put_recovers_from_every_journal_file_boundary() {
        let bytes = b"journal boundary put bytes";
        for boundary in [
            ProviderJournalBoundary::BeforeBlobDurable,
            ProviderJournalBoundary::BlobDurable,
            ProviderJournalBoundary::CreationRecordDurable,
            ProviderJournalBoundary::BlobInstalled,
            ProviderJournalBoundary::RecordDurable,
            ProviderJournalBoundary::UpdateDurable,
            ProviderJournalBoundary::UpdateInstalled,
            ProviderJournalBoundary::BlobRemoved,
            ProviderJournalBoundary::CompletionDurable,
            ProviderJournalBoundary::RecordRemoved,
        ] {
            let mut simulator = simulator_with_provider_item(bytes);
            let copy = ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: "fixture-object".into(),
                },
                destination: location("objects/destination"),
            };
            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| hook.replace(Some(boundary)));
            assert!(
                matches!(
                    simulator.run_action(&action(1, copy.clone())),
                    Err(ScenarioError::Io(message)) if message.contains("journal crash")
                ),
                "{boundary:?}"
            );
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    2,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            simulator.run_action(&action(1, copy)).unwrap();
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{boundary:?}"
            );
        }
    }

    #[test]
    fn provider_put_creation_crashes_reopen_without_overwrite() {
        let bytes = b"journal construction put bytes";
        let unrelated = b"unrelated destination";
        for boundary in [
            ProviderJournalBoundary::BeforeBlobDurable,
            ProviderJournalBoundary::BlobDurable,
            ProviderJournalBoundary::CreationRecordDurable,
            ProviderJournalBoundary::BlobInstalled,
            ProviderJournalBoundary::RecordDurable,
        ] {
            let mut simulator = simulator_with_provider_item(bytes);
            let copy = ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: "fixture-object".into(),
                },
                destination: location("objects/destination"),
            };
            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| hook.replace(Some(boundary)));
            assert!(
                matches!(
                    simulator.run_action(&action(1, copy.clone())),
                    Err(ScenarioError::Io(message)) if message.contains("journal crash")
                ),
                "{boundary:?}"
            );
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert!(!inbox.join("objects/destination").exists(), "{boundary:?}");
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    2,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            std::fs::write(inbox.join("objects/destination"), unrelated).unwrap();
            assert!(
                matches!(
                    simulator.run_action(&action(1, copy.clone())),
                    Err(ScenarioError::ProviderConflictingBytes(path))
                        if path == "objects/destination"
                ),
                "{boundary:?}"
            );
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                unrelated,
                "{boundary:?}"
            );
            std::fs::remove_file(inbox.join("objects/destination")).unwrap();
            simulator.run_action(&action(1, copy)).unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{boundary:?}"
            );
        }
    }

    #[test]
    fn provider_rename_creation_crashes_reopen_without_overwrite() {
        let bytes = b"journal construction rename bytes";
        let unrelated = b"unrelated destination";
        for boundary in [
            ProviderJournalBoundary::BeforeBlobDurable,
            ProviderJournalBoundary::BlobDurable,
            ProviderJournalBoundary::CreationRecordDurable,
            ProviderJournalBoundary::BlobInstalled,
            ProviderJournalBoundary::RecordDurable,
        ] {
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            let rename = ScheduledActionKind::ProviderRename {
                device: "beta".into(),
                tree: ProviderTree::Inbox,
                from_path: "objects/source".into(),
                to_path: "objects/destination".into(),
            };
            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| hook.replace(Some(boundary)));
            assert!(
                matches!(
                    simulator.run_action(&action(2, rename.clone())),
                    Err(ScenarioError::Io(message)) if message.contains("journal crash")
                ),
                "{boundary:?}"
            );
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/source")).unwrap(),
                bytes,
                "{boundary:?}"
            );
            assert!(!inbox.join("objects/destination").exists(), "{boundary:?}");
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    3,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            std::fs::write(inbox.join("objects/destination"), unrelated).unwrap();
            assert!(
                matches!(
                    simulator.run_action(&action(2, rename.clone())),
                    Err(ScenarioError::ProviderConflictingBytes(path))
                        if path == "objects/destination"
                ),
                "{boundary:?}"
            );
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                unrelated,
                "{boundary:?}"
            );
            assert_eq!(
                std::fs::read(inbox.join("objects/source")).unwrap(),
                bytes,
                "{boundary:?}"
            );
            std::fs::remove_file(inbox.join("objects/destination")).unwrap();
            simulator.run_action(&action(2, rename)).unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{boundary:?}"
            );
            assert!(!inbox.join("objects/source").exists(), "{boundary:?}");
        }
    }

    #[test]
    fn provider_orphan_blob_retirement_is_crash_closed() {
        let bytes = b"journal orphan retirement bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let copy = ScheduledActionKind::ProviderCopy {
            source: ProviderSource::Mailbox {
                item_id: "fixture-object".into(),
            },
            destination: location("objects/destination"),
        };
        let journal = simulator.provider_journal_path("beta").unwrap();
        FAIL_PROVIDER_JOURNAL_BOUNDARY
            .with(|hook| hook.replace(Some(ProviderJournalBoundary::BlobDurable)));
        assert!(matches!(
            simulator.run_action(&action(1, copy.clone())),
            Err(ScenarioError::Io(message)) if message.contains("journal crash")
        ));
        assert_eq!(
            std::fs::read_dir(journal.join("records")).unwrap().count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(journal.join("blobs")).unwrap().count(),
            1
        );
        simulator.device_mut("beta").unwrap().crash();
        FAIL_PROVIDER_JOURNAL_BOUNDARY
            .with(|hook| hook.replace(Some(ProviderJournalBoundary::OrphanPrivateDeleted)));
        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            )),
            Err(ScenarioError::Io(message)) if message.contains("journal crash")
        ));
        assert_eq!(
            std::fs::read_dir(journal.join("blobs")).unwrap().count(),
            0
        );
        simulator.device_mut("beta").unwrap().crash();
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        simulator.run_action(&action(1, copy)).unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
    }

    #[test]
    fn orphan_quarantine_restores_bytes_when_authenticated_owner_arrives_at_race_boundary() {
        let bytes = b"authenticated orphan owner race";
        let mut simulator = simulator_with_provider_item(bytes);
        let copy = ScheduledActionKind::ProviderCopy {
            source: ProviderSource::Mailbox {
                item_id: "fixture-object".into(),
            },
            destination: location("objects/destination"),
        };
        let journal = simulator.provider_journal_path("beta").unwrap();
        let operation_binding = "event:1:mailbox:fixture-object";
        let source_provenance = "mailbox:fixture-object";
        let operation_id = ProviderRetryJournal::operation_id(
            ProviderJournalOperation::Put,
            operation_binding,
            source_provenance,
            ProviderTree::Inbox,
            "objects/destination",
            None,
            u64::try_from(bytes.len()).unwrap(),
            &provider_digest(bytes),
        );
        FAIL_PROVIDER_JOURNAL_BOUNDARY
            .with(|hook| hook.replace(Some(ProviderJournalBoundary::BlobDurable)));
        assert!(simulator.run_action(&action(1, copy.clone())).is_err());
        simulator.device_mut("beta").unwrap().crash();

        let hook_journal = journal.clone();
        let hook_operation_id = operation_id.clone();
        let hook_bytes = bytes.to_vec();
        PROVIDER_ORPHAN_AFTER_QUARANTINE_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                let key: [u8; 32] = std::fs::read(hook_journal.join("authority.key"))
                    .unwrap()
                    .try_into()
                    .unwrap();
                let mut record = ProviderJournalRecord {
                    journal_schema_version: PROVIDER_JOURNAL_SCHEMA_VERSION,
                    operation_id: hook_operation_id.clone(),
                    operation: ProviderJournalOperation::Put,
                    operation_binding: operation_binding.into(),
                    source_provenance: source_provenance.into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/destination".into(),
                    to_path: None,
                    source_identity: None,
                    source_len: u64::try_from(hook_bytes.len()).unwrap(),
                    source_digest: provider_digest(&hook_bytes),
                    blob_name: Some(ProviderRetryJournal::blob_name(&hook_operation_id)),
                    phase: ProviderJournalPhase::Prepared,
                    staging_identity: None,
                    destination_identity: None,
                    staging_name: Some(ProviderRetryJournal::staging_name(
                        &hook_operation_id,
                        0,
                    )),
                    staging_generation: 0,
                    diagnostic_path: None,
                    authentication_tag: String::new(),
                };
                let unsigned = serde_json::to_vec(&record).unwrap();
                record.authentication_tag = hmac_sha256_hex(&key, &unsigned);
                let update = serde_json::to_vec(&record).unwrap();
                let update_path = hook_journal
                    .join("records")
                    .join(format!("{}.update", hook_operation_id));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(update_path)
                    .unwrap();
                file.write_all(&update).unwrap();
                file.sync_all().unwrap();
                std::fs::File::open(hook_journal.join("records"))
                    .unwrap()
                    .sync_all()
                    .unwrap();
            })));
        });
        FAIL_PROVIDER_JOURNAL_BOUNDARY
            .with(|hook| hook.replace(Some(ProviderJournalBoundary::OrphanRestored)));
        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            )),
            Err(ScenarioError::Io(message)) if message.contains("journal crash")
        ));
        assert_eq!(
            std::fs::read(journal.join("blobs").join(format!("{operation_id}.creating")))
                .unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read_dir(journal.join("quarantine"))
                .unwrap()
                .count(),
            0
        );

        simulator.device_mut("beta").unwrap().crash();
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        simulator.run_action(&action(1, copy)).unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
    }

    #[test]
    fn orphan_retirement_private_boundaries_are_crash_closed() {
        for boundary in [
            ProviderJournalBoundary::OrphanQuarantined,
            ProviderJournalBoundary::OrphanOwnershipRechecked,
            ProviderJournalBoundary::OrphanPrivateDeleted,
        ] {
            let bytes = b"orphan private crash boundary";
            let mut simulator = simulator_with_provider_item(bytes);
            let copy = ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: "fixture-object".into(),
                },
                destination: location("objects/destination"),
            };
            FAIL_PROVIDER_JOURNAL_BOUNDARY
                .with(|hook| hook.replace(Some(ProviderJournalBoundary::BlobDurable)));
            assert!(simulator.run_action(&action(1, copy.clone())).is_err());
            simulator.device_mut("beta").unwrap().crash();
            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| hook.replace(Some(boundary)));
            assert!(
                matches!(
                    simulator.run_action(&action(
                        2,
                        ScheduledActionKind::Restart {
                            device: "beta".into(),
                        },
                    )),
                    Err(ScenarioError::Io(message)) if message.contains("journal crash")
                ),
                "{boundary:?}"
            );
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    3,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            simulator.run_action(&action(1, copy)).unwrap();
            let journal = simulator.provider_journal_path("beta").unwrap();
            assert_eq!(
                std::fs::read_dir(journal.join("quarantine"))
                    .unwrap()
                    .count(),
                0,
                "{boundary:?}"
            );
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{boundary:?}"
            );
        }
    }

    #[test]
    fn provider_transaction_gate_rejects_a_second_process_scope() {
        let simulator = simulator_with_provider_item(b"gate bytes");
        let runtime = simulator.device("beta").unwrap();
        let journal = runtime.provider_journal.as_ref().unwrap();
        let _gate = journal.acquire_transaction_gate().unwrap();
        assert!(matches!(
            ProviderRetryJournal::open(runtime.root.join("provider-local-journal")),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("gate")
        ));
    }

    #[test]
    fn provider_transaction_device_order_is_canonical_and_unique() {
        let alpha_source = ProviderSource::Tree {
            location: ProviderLocation {
                device: "alpha".into(),
                tree: ProviderTree::Outbox,
                path: "objects/source".into(),
            },
        };
        let beta_source = ProviderSource::Tree {
            location: ProviderLocation {
                device: "beta".into(),
                tree: ProviderTree::Outbox,
                path: "objects/source".into(),
            },
        };
        assert_eq!(
            provider_transaction_device_names(&alpha_source, "beta"),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
        assert_eq!(
            provider_transaction_device_names(&beta_source, "alpha"),
            vec!["alpha".to_owned(), "beta".to_owned()]
        );
        assert_eq!(
            provider_transaction_device_names(&beta_source, "beta"),
            vec!["beta".to_owned()]
        );
        assert_eq!(
            provider_transaction_device_names(
                &ProviderSource::Mailbox {
                    item_id: "item".into(),
                },
                "beta",
            ),
            vec!["beta".to_owned()]
        );
    }

    #[test]
    fn same_device_provider_source_uses_one_gate_and_copy_succeeds() {
        let bytes = b"same device gate bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let source = location("objects/source");
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: source.clone(),
                },
            ))
            .unwrap();
        let tree_source = ProviderSource::Tree {
            location: source,
        };
        assert_eq!(
            provider_transaction_device_names(&tree_source, "beta"),
            vec!["beta".to_owned()]
        );
        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::ProviderCopy {
                    source: tree_source,
                    destination: location("objects/destination"),
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
    }

    #[cfg(unix)]
    #[test]
    fn cross_device_copy_waits_for_source_gate_before_source_inspection() {
        let bytes = b"cross device copy bytes";
        let mut simulator = simulator_with_cross_device_provider_item(bytes);
        let source = seed_cross_device_source(&mut simulator);
        let destination = location("objects/copied");
        let competing_journal =
            ProviderRetryJournal::open(simulator.provider_journal_path("alpha").unwrap())
                .unwrap();
        let source_gate = competing_journal.acquire_transaction_gate().unwrap();
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| visits.set(0));

        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree {
                        location: source.clone(),
                    },
                    destination: destination.clone(),
                },
            )),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("gate")
        ));
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| assert_eq!(visits.get(), 0));
        assert_no_destination_transaction_residue(&simulator, &destination.path);

        drop(source_gate);
        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree { location: source },
                    destination: destination.clone(),
                },
            ))
            .unwrap();
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| assert_eq!(visits.get(), 1));
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(std::fs::read(inbox.join(destination.path)).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn cross_device_begin_write_waits_for_source_gate_before_source_inspection() {
        let bytes = b"cross device begin bytes";
        let mut simulator = simulator_with_cross_device_provider_item(bytes);
        let source = seed_cross_device_source(&mut simulator);
        let destination = location("objects/partial");
        let competing_journal =
            ProviderRetryJournal::open(simulator.provider_journal_path("alpha").unwrap())
                .unwrap();
        let source_gate = competing_journal.acquire_transaction_gate().unwrap();
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| visits.set(0));

        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Tree {
                        location: source.clone(),
                    },
                    destination: destination.clone(),
                    transfer_id: "cross-write".into(),
                },
            )),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("gate")
        ));
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| assert_eq!(visits.get(), 0));
        assert_no_destination_transaction_residue(&simulator, &destination.path);

        drop(source_gate);
        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Tree { location: source },
                    destination,
                    transfer_id: "cross-write".into(),
                },
            ))
            .unwrap();
        PROVIDER_SOURCE_INSPECTION_VISITS.with(|visits| assert_eq!(visits.get(), 1));
        assert!(simulator
            .device("beta")
            .unwrap()
            .provider
            .writes
            .contains_key("cross-write"));
    }

    #[cfg(unix)]
    #[test]
    fn provider_authority_key_and_journal_root_replacement_fail_closed() {
        let bytes = b"retained authority bytes";
        let mut key_simulator = simulator_with_provider_item(bytes);
        let key_journal = key_simulator.provider_journal_path("beta").unwrap();
        let authority_key = key_journal.join("authority.key");
        let original_key = std::fs::read(&authority_key).unwrap();
        std::fs::rename(&authority_key, key_journal.join("authority.original")).unwrap();
        std::fs::write(&authority_key, &original_key).unwrap();

        let runtime = key_simulator.device("beta").unwrap();
        assert!(matches!(
            ProviderRetryJournal::open(runtime.root.join("provider-local-journal")),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("authority")
        ));
        assert!(matches!(
            key_simulator.run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/destination"),
                },
            )),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("authority")
        ));
        let key_inbox = key_simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert!(!key_inbox.join("objects/destination").exists());

        let mut root_simulator = simulator_with_provider_item(bytes);
        let journal = root_simulator.provider_journal_path("beta").unwrap();
        let replacement = journal.with_file_name("provider-local-journal.original");
        std::fs::rename(&journal, &replacement).unwrap();
        std::fs::create_dir(&journal).unwrap();
        for child in ["records", "blobs", "quarantine", "completed"] {
            std::fs::create_dir(journal.join(child)).unwrap();
        }
        std::fs::write(
            journal.join("authority.key"),
            std::fs::read(replacement.join("authority.key")).unwrap(),
        )
        .unwrap();
        let runtime = root_simulator.device("beta").unwrap();
        assert!(matches!(
            ProviderRetryJournal::open(runtime.root.join("provider-local-journal")),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("root")
        ));
        assert!(matches!(
            root_simulator.run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/destination"),
                },
            )),
            Err(ScenarioError::UnsafeProviderJournal(message))
                if message.contains("journal")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn finish_provider_write_holds_gate_before_source_or_retry_inspection() {
        let bytes = b"finish entry gate bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        begin_provider_write(
            &mut simulator,
            "write",
            location("objects/destination"),
        );
        append_provider_write(&mut simulator, "write", bytes.len());
        let runtime_root = simulator.device("beta").unwrap().root.clone();
        let journal = simulator.provider_journal_path("beta").unwrap();
        PROVIDER_FINISH_AFTER_GATE_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                let pending_before = std::fs::read_dir(journal.join("records"))
                    .unwrap()
                    .count();
                assert!(matches!(
                    ProviderRetryJournal::open(
                        runtime_root.join("provider-local-journal")
                    ),
                    Err(ScenarioError::UnsafeProviderJournal(message))
                        if message.contains("gate")
                ));
                assert_eq!(
                    std::fs::read_dir(journal.join("records"))
                        .unwrap()
                        .count(),
                    pending_before
                );
            })));
        });
        finish_provider_write(&mut simulator, "write").unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
    }

    #[cfg(unix)]
    #[test]
    fn retirement_race_before_public_cleanup_preserves_foreign_bytes_and_retry_converges() {
        let bytes = b"rename retirement original";
        let foreign = b"rename retirement foreign";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let hook_source = inbox.join("objects/source");
        PROVIDER_RETIREMENT_BEFORE_PRIVATE_MOVE_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                std::fs::remove_file(&hook_source).unwrap();
                std::fs::write(&hook_source, foreign).unwrap();
            })));
        });
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        assert!(matches!(
            simulator.run_action(&action(2, rename.clone())),
            Err(ScenarioError::UnsafeProviderEntry(_))
        ));
        assert!(!inbox.join("objects/source").exists());
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
        let evidence = inbox.join(PROVIDER_RENAME_EVIDENCE_NAMESPACE);
        let retained: Vec<_> = std::fs::read_dir(&evidence)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(retained.len(), 1);
        assert_eq!(std::fs::read(&retained[0]).unwrap(), foreign);

        simulator.device_mut("beta").unwrap().crash();
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        simulator.run_action(&action(2, rename)).unwrap();
        assert_eq!(std::fs::read(&retained[0]).unwrap(), foreign);
        assert_eq!(std::fs::read_dir(evidence).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn rename_retirement_private_boundaries_are_crash_closed() {
        for boundary in [
            ProviderJournalBoundary::RetirementPlaceholderDurable,
            ProviderJournalBoundary::RetirementExchangeDurable,
            ProviderJournalBoundary::RetirementPlaceholderQuarantined,
            ProviderJournalBoundary::RetirementPlaceholderPrivateDeleted,
        ] {
            let bytes = b"rename private crash boundary";
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            let rename = ScheduledActionKind::ProviderRename {
                device: "beta".into(),
                tree: ProviderTree::Inbox,
                from_path: "objects/source".into(),
                to_path: "objects/destination".into(),
            };
            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| hook.replace(Some(boundary)));
            assert!(
                matches!(
                    simulator.run_action(&action(2, rename.clone())),
                    Err(ScenarioError::Io(message)) if message.contains("journal crash")
                ),
                "{boundary:?}"
            );
            simulator.device_mut("beta").unwrap().crash();
            simulator
                .run_action(&action(
                    3,
                    ScheduledActionKind::Restart {
                        device: "beta".into(),
                    },
                ))
                .unwrap();
            simulator.run_action(&action(2, rename)).unwrap();
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert!(!inbox.join("objects/source").exists(), "{boundary:?}");
            assert_eq!(
                std::fs::read(inbox.join("objects/destination")).unwrap(),
                bytes,
                "{boundary:?}"
            );
            assert_eq!(
                std::fs::read_dir(inbox.join(PROVIDER_RENAME_EVIDENCE_NAMESPACE))
                    .unwrap()
                    .count(),
                0,
                "{boundary:?}"
            );
        }
    }

    #[test]
    fn provider_reopen_rejects_missing_referenced_blob() {
        let bytes = b"missing referenced blob bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Prepared)));
        assert!(simulator.run_action(&action(2, rename)).is_err());
        let journal = simulator.provider_journal_path("beta").unwrap();
        let blob = std::fs::read_dir(journal.join("blobs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::remove_file(blob).unwrap();
        simulator.device_mut("beta").unwrap().crash();
        assert!(matches!(
            simulator.run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            )),
            Err(ScenarioError::UnsafeProviderJournal(_))
        ));
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/source")).unwrap(),
            bytes
        );
        assert!(!inbox.join("objects/destination").exists());
    }

    #[test]
    fn provider_destination_replacement_is_quarantined_without_overwrite() {
        let bytes = b"destination identity bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Published)));
        assert!(simulator.run_action(&action(2, rename.clone())).is_err());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::rename(
            inbox.join("objects/destination"),
            inbox.join("objects/replaced-destination"),
        )
        .unwrap();
        std::fs::write(
            inbox.join("objects/destination"),
            b"attacker destination bytes",
        )
        .unwrap();
        simulator.device_mut("beta").unwrap().crash();
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();

        assert!(matches!(
            simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/destination"
        ));
        assert!(!inbox.join("objects/destination").exists());
        assert_eq!(
            std::fs::read(inbox.join("objects/source")).unwrap(),
            bytes
        );
        assert!(std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
            .unwrap()
            .any(|entry| std::fs::read(entry.unwrap().path()).unwrap()
                == b"attacker destination bytes"));
    }

    #[test]
    fn forged_deterministic_quarantine_collision_fails_closed_without_overwrite() {
        let bytes = b"destination identity bytes";
        let attacker = b"attacker destination bytes";
        let forged = b"forged diagnostic collision";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Published)));
        assert!(simulator.run_action(&action(2, rename.clone())).is_err());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::remove_file(inbox.join("objects/destination")).unwrap();
        std::fs::write(inbox.join("objects/destination"), attacker).unwrap();
        let collision_name = provider_quarantine_diagnostic_name(
            "destination-mismatch",
            "destination",
            attacker,
        );
        let collision = inbox
            .join(PROVIDER_REMOVED_NAMESPACE)
            .join(collision_name);
        std::fs::write(&collision, forged).unwrap();

        assert!(matches!(
            simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnsafeProviderEntry(path))
                if path.starts_with(PROVIDER_REMOVED_NAMESPACE)
        ));
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            attacker
        );
        assert_eq!(std::fs::read(collision).unwrap(), forged);
    }

    #[test]
    fn provider_partial_destination_recovery_rejects_mutated_recorded_identity() {
        let bytes = b"partial destination recovery bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Published)));
        assert!(simulator
            .run_action(&action(2, rename.clone()))
            .is_err());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::write(
            inbox.join("objects/destination"),
            &bytes[..bytes.len() / 2],
        )
        .unwrap();
        simulator.device_mut("beta").unwrap().crash();
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        assert!(matches!(
            simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/destination"
        ));
        assert!(!inbox.join("objects/destination").exists());
        assert!(std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
            .unwrap()
            .any(|entry| std::fs::read(entry.unwrap().path()).unwrap()
                == &bytes[..bytes.len() / 2]));
    }

    #[test]
    fn forged_provider_residue_never_authorizes_rename_or_remove_retry() {
        let bytes = b"forged provider evidence bytes";
        let mut rename_simulator = simulator_with_provider_item(bytes);
        rename_simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Retired)));
        assert!(rename_simulator
            .run_action(&action(2, rename.clone()))
            .is_err());
        let inbox = rename_simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let journal = rename_simulator.provider_journal_path("beta").unwrap();
        for directory in ["records", "blobs"] {
            for entry in std::fs::read_dir(journal.join(directory)).unwrap() {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
        }
        std::fs::write(
            inbox.join(PROVIDER_RENAME_EVIDENCE_NAMESPACE).join("forged"),
            bytes,
        )
        .unwrap();
        rename_simulator.device_mut("beta").unwrap().crash();
        rename_simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        assert!(matches!(
            rename_simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnknownProviderPath(path)) if path == "objects/source"
        ));
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );

        let mut remove_simulator = simulator_with_provider_item(bytes);
        remove_simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Retired)));
        assert!(remove_simulator
            .run_action(&action(
                2,
                ScheduledActionKind::ProviderRemove {
                    location: location("objects/source"),
                },
            ))
            .is_err());
        let remove_journal = remove_simulator.provider_journal_path("beta").unwrap();
        for entry in std::fs::read_dir(remove_journal.join("records")).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        remove_simulator.device_mut("beta").unwrap().crash();
        remove_simulator
            .run_action(&action(
                3,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ))
            .unwrap();
        assert!(matches!(
            remove_simulator.run_action(&action(
                4,
                ScheduledActionKind::ProviderRemove {
                    location: location("objects/source"),
                },
            )),
            Err(ScenarioError::UnknownProviderPath(path)) if path == "objects/source"
        ));
    }

    #[test]
    fn every_journal_record_class_rejects_truncated_auth_invalid_unknown_and_future_bytes_on_open() {
        for record_class in ["pending", "completed", "update"] {
            for corruption in ["truncated", "auth-invalid", "unknown", "future"] {
                let bytes = b"authenticated graph validation bytes";
                let mut simulator = simulator_with_provider_item(bytes);
                let rename = ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/source".into(),
                    to_path: "objects/destination".into(),
                };
                let target = match record_class {
                    "completed" => {
                        simulator
                            .run_action(&action(
                                1,
                                ScheduledActionKind::ProviderCopy {
                                    source: ProviderSource::Mailbox {
                                        item_id: "fixture-object".into(),
                                    },
                                    destination: location("objects/source"),
                                },
                            ))
                            .unwrap();
                        std::fs::read_dir(
                            simulator
                                .provider_journal_path("beta")
                                .unwrap()
                                .join("completed"),
                        )
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .path()
                    }
                    "pending" | "update" => {
                        simulator
                            .run_action(&action(
                                1,
                                ScheduledActionKind::ProviderCopy {
                                    source: ProviderSource::Mailbox {
                                        item_id: "fixture-object".into(),
                                    },
                                    destination: location("objects/source"),
                                },
                            ))
                            .unwrap();
                        if record_class == "pending" {
                            FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| {
                                hook.replace(Some(ProviderJournalPhase::Prepared));
                            });
                        } else {
                            FAIL_PROVIDER_JOURNAL_BOUNDARY.with(|hook| {
                                hook.replace(Some(ProviderJournalBoundary::UpdateDurable));
                            });
                        }
                        assert!(simulator.run_action(&action(2, rename.clone())).is_err());
                        let records = simulator
                            .provider_journal_path("beta")
                            .unwrap()
                            .join("records");
                        std::fs::read_dir(records)
                            .unwrap()
                            .map(|entry| entry.unwrap().path())
                            .find(|path| {
                                path.extension().and_then(|value| value.to_str())
                                    == Some(if record_class == "update" {
                                        "update"
                                    } else {
                                        "json"
                                    })
                            })
                            .unwrap()
                    }
                    _ => unreachable!(),
                };
                let original = std::fs::read(&target).unwrap();
                match corruption {
                    "truncated" => {
                        std::fs::write(&target, &original[..original.len() / 2]).unwrap();
                    }
                    "auth-invalid" => {
                        let mut value: serde_json::Value =
                            serde_json::from_slice(&original).unwrap();
                        value["authentication_tag"] = serde_json::json!("0".repeat(64));
                        std::fs::write(&target, serde_json::to_vec(&value).unwrap()).unwrap();
                    }
                    "unknown" => {
                        let mut value: serde_json::Value =
                            serde_json::from_slice(&original).unwrap();
                        value["unknown_future_field"] = serde_json::json!(true);
                        std::fs::write(&target, serde_json::to_vec(&value).unwrap()).unwrap();
                    }
                    "future" => {
                        let mut record: ProviderJournalRecord =
                            serde_json::from_slice(&original).unwrap();
                        record.journal_schema_version =
                            PROVIDER_JOURNAL_SCHEMA_VERSION + 1;
                        simulator
                            .device("beta")
                            .unwrap()
                            .provider_journal
                            .as_ref()
                            .unwrap()
                            .sign_record(&mut record)
                            .unwrap();
                        std::fs::write(&target, serde_json::to_vec(&record).unwrap()).unwrap();
                    }
                    _ => unreachable!(),
                }
                let corrupted = std::fs::read(&target).unwrap();
                simulator.device_mut("beta").unwrap().crash();
                assert!(
                    matches!(
                        simulator.run_action(&action(
                            9,
                            ScheduledActionKind::Restart {
                                device: "beta".into(),
                            },
                        )),
                        Err(ScenarioError::UnsafeProviderJournal(_))
                    ),
                    "{record_class}/{corruption}"
                );
                assert_eq!(std::fs::read(&target).unwrap(), corrupted);
            }
        }
    }

    #[test]
    fn orphan_wrong_and_shared_blob_ownership_fail_before_reconciliation() {
        for corruption in ["orphan", "wrong-name", "shared-link"] {
            let bytes = b"unique blob ownership bytes";
            let mut simulator = simulator_with_provider_item(bytes);
            let journal = simulator.provider_journal_path("beta").unwrap();
            if corruption == "orphan" {
                std::fs::write(
                    journal.join("blobs").join(format!("{}.blob", "a".repeat(64))),
                    bytes,
                )
                .unwrap();
            } else {
                simulator
                    .run_action(&action(
                        1,
                        ScheduledActionKind::ProviderCopy {
                            source: ProviderSource::Mailbox {
                                item_id: "fixture-object".into(),
                            },
                            destination: location("objects/source"),
                        },
                    ))
                    .unwrap();
                FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| {
                    hook.replace(Some(ProviderJournalPhase::Prepared));
                });
                assert!(simulator
                    .run_action(&action(
                        2,
                        ScheduledActionKind::ProviderRename {
                            device: "beta".into(),
                            tree: ProviderTree::Inbox,
                            from_path: "objects/source".into(),
                            to_path: "objects/destination".into(),
                        },
                    ))
                    .is_err());
                let blob = std::fs::read_dir(journal.join("blobs"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                let wrong = journal.join("blobs").join(format!("{}.blob", "b".repeat(64)));
                if corruption == "wrong-name" {
                    std::fs::rename(blob, wrong).unwrap();
                } else {
                    std::fs::hard_link(blob, wrong).unwrap();
                }
            }
            simulator.device_mut("beta").unwrap().crash();
            assert!(
                matches!(
                    simulator.run_action(&action(
                        9,
                        ScheduledActionKind::Restart {
                            device: "beta".into(),
                        },
                    )),
                    Err(ScenarioError::UnsafeProviderJournal(_))
                ),
                "{corruption}"
            );
        }
    }

    #[test]
    fn operation_identity_binds_provenance_length_and_digest_and_exact_retry_keeps_one_receipt() {
        let base = ProviderRetryJournal::operation_id(
            ProviderJournalOperation::Put,
            "event:1",
            "mailbox:item",
            ProviderTree::Inbox,
            "objects/destination",
            None,
            3,
            &provider_digest(b"one"),
        );
        for changed in [
            ProviderRetryJournal::operation_id(
                ProviderJournalOperation::Put,
                "event:1",
                "mailbox:other",
                ProviderTree::Inbox,
                "objects/destination",
                None,
                3,
                &provider_digest(b"one"),
            ),
            ProviderRetryJournal::operation_id(
                ProviderJournalOperation::Put,
                "event:1",
                "mailbox:item",
                ProviderTree::Inbox,
                "objects/destination",
                None,
                4,
                &provider_digest(b"one"),
            ),
            ProviderRetryJournal::operation_id(
                ProviderJournalOperation::Put,
                "event:1",
                "mailbox:item",
                ProviderTree::Inbox,
                "objects/destination",
                None,
                3,
                &provider_digest(b"two"),
            ),
        ] {
            assert_ne!(base, changed);
        }
        let bytes = b"exact completed retry";
        let mut simulator = simulator_with_provider_item(bytes);
        let copy = ScheduledActionKind::ProviderCopy {
            source: ProviderSource::Mailbox {
                item_id: "fixture-object".into(),
            },
            destination: location("objects/destination"),
        };
        simulator.run_action(&action(1, copy.clone())).unwrap();
        let completed = simulator
            .provider_journal_path("beta")
            .unwrap()
            .join("completed");
        let first_name = std::fs::read_dir(&completed)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name();
        simulator.run_action(&action(1, copy)).unwrap();
        let names = std::fs::read_dir(completed)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![first_name]);
    }

    #[test]
    fn authenticated_record_load_rederives_source_provenance_length_and_digest_identity() {
        for field in ["provenance", "length", "digest"] {
            let bytes = b"identity rederivation bytes";
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| {
                hook.replace(Some(ProviderJournalPhase::Prepared));
            });
            assert!(simulator
                .run_action(&action(
                    2,
                    ScheduledActionKind::ProviderRename {
                        device: "beta".into(),
                        tree: ProviderTree::Inbox,
                        from_path: "objects/source".into(),
                        to_path: "objects/destination".into(),
                    },
                ))
                .is_err());
            let record_path = std::fs::read_dir(
                simulator
                    .provider_journal_path("beta")
                    .unwrap()
                    .join("records"),
            )
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
            let mut record: ProviderJournalRecord =
                serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
            match field {
                "provenance" => record.source_provenance.push_str(":changed"),
                "length" => record.source_len += 1,
                "digest" => record.source_digest = provider_digest(b"changed"),
                _ => unreachable!(),
            }
            simulator
                .device("beta")
                .unwrap()
                .provider_journal
                .as_ref()
                .unwrap()
                .sign_record(&mut record)
                .unwrap();
            std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
            simulator.device_mut("beta").unwrap().crash();
            assert!(
                matches!(
                    simulator.run_action(&action(
                        9,
                        ScheduledActionKind::Restart {
                            device: "beta".into(),
                        },
                    )),
                    Err(ScenarioError::UnsafeProviderJournal(_))
                ),
                "{field}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn production_publication_and_retirement_reject_source_name_substitution() {
        let bytes = b"retained construction bytes";
        let mut publication = simulator_with_provider_item(bytes);
        let inbox = publication
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let hook_root = inbox.clone();
        PROVIDER_PUBLICATION_SOURCE_VALIDATION_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                let staging = std::fs::read_dir(hook_root.join(PROVIDER_TEMP_NAMESPACE))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| std::fs::read(path).unwrap() == bytes)
                    .unwrap();
                std::fs::rename(
                    &staging,
                    hook_root.join(PROVIDER_TEMP_NAMESPACE).join("retained-original"),
                )
                .unwrap();
                std::fs::write(staging, b"attacker publication bytes").unwrap();
            })));
        });
        publication
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/destination"),
                },
            ))
            .unwrap();
        assert_eq!(std::fs::read(inbox.join("objects/destination")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(inbox.join(".part/retained-original")).unwrap(),
            bytes
        );

        let mut retirement = simulator_with_provider_item(bytes);
        retirement
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let retire_root = retirement
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let hook_root = retire_root.clone();
        PROVIDER_RETIREMENT_VALIDATION_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move || {
                std::fs::rename(
                    hook_root.join("objects/source"),
                    hook_root.join("objects/retained-original"),
                )
                .unwrap();
                std::fs::write(
                    hook_root.join("objects/source"),
                    b"attacker retirement bytes",
                )
                .unwrap();
            })));
        });
        assert!(matches!(
            retirement.run_action(&action(
                2,
                ScheduledActionKind::ProviderRemove {
                    location: location("objects/source"),
                },
            )),
            Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/source"
        ));
        assert_eq!(
            std::fs::read(retire_root.join("objects/source")).unwrap(),
            b"attacker retirement bytes"
        );
        assert_eq!(
            std::fs::read(retire_root.join("objects/retained-original")).unwrap(),
            bytes
        );
    }

    #[test]
    fn corrupt_substituted_or_multilink_local_journal_fails_closed() {
        for attack in [
            "record-corrupt",
            "record-substitute",
            "record-link",
            "blob-corrupt",
            "blob-link",
        ] {
            let bytes = b"journal corruption bytes";
            let mut simulator = simulator_with_provider_item(bytes);
            simulator
                .run_action(&action(
                    1,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location("objects/source"),
                    },
                ))
                .unwrap();
            let rename = ScheduledActionKind::ProviderRename {
                device: "beta".into(),
                tree: ProviderTree::Inbox,
                from_path: "objects/source".into(),
                to_path: "objects/destination".into(),
            };
            FAIL_PROVIDER_JOURNAL_AFTER_PHASE
                .with(|hook| hook.replace(Some(ProviderJournalPhase::Prepared)));
            assert!(simulator
                .run_action(&action(2, rename.clone()))
                .is_err());
            let journal = simulator.provider_journal_path("beta").unwrap();
            let record_path = std::fs::read_dir(journal.join("records"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let blob_path = std::fs::read_dir(journal.join("blobs"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            match attack {
                "record-corrupt" => std::fs::write(&record_path, b"{}").unwrap(),
                "record-substitute" => {
                    let mut record: ProviderJournalRecord =
                        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
                    record.from_path = "objects/substituted".into();
                    std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
                }
                "record-link" => {
                    std::fs::hard_link(&record_path, journal.join("record-alias")).unwrap()
                }
                "blob-corrupt" => std::fs::write(&blob_path, b"corrupt").unwrap(),
                "blob-link" => {
                    std::fs::hard_link(&blob_path, journal.join("blob-alias")).unwrap()
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    simulator.run_action(&action(2, rename)),
                    Err(ScenarioError::UnsafeProviderJournal(_))
                ),
                "{attack}"
            );
            let inbox = simulator
                .provider_tree_path("beta", ProviderTree::Inbox)
                .unwrap();
            assert!(!inbox.join("objects/destination").exists(), "{attack}");
            assert_eq!(
                std::fs::read(inbox.join("objects/source")).unwrap(),
                bytes,
                "{attack}"
            );
        }
    }

    #[test]
    fn provider_journal_enforces_numeric_entry_and_byte_bounds() {
        assert_eq!(MAX_PROVIDER_JOURNAL_PENDING, 4);
        assert_eq!(MAX_PROVIDER_JOURNAL_BLOB_BYTES, 8 * 1024 * 1024);
        assert_eq!(
            MAX_PROVIDER_JOURNAL_BYTES,
            MAX_PROVIDER_JOURNAL_BLOB_BYTES
                + (MAX_PROVIDER_JOURNAL_PENDING + 1) * MAX_PROVIDER_JOURNAL_RECORD_BYTES
        );
        assert_eq!(MAX_PROVIDER_JOURNAL_FILES, 9);
        let bytes = b"bounded journal";
        let mut simulator = simulator_with_provider_item(bytes);
        for index in 0..=MAX_PROVIDER_JOURNAL_PENDING {
            let source = format!("objects/source-{index}");
            simulator
                .run_action(&action(
                    10 + index as u64,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination: location(&source),
                    },
                ))
                .unwrap();
        }
        for index in 0..=MAX_PROVIDER_JOURNAL_PENDING {
            let source = format!("objects/source-{index}");
            FAIL_PROVIDER_JOURNAL_AFTER_PHASE
                .with(|hook| hook.replace(Some(ProviderJournalPhase::Prepared)));
            let result = simulator.run_action(&action(
                100 + index as u64,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: source,
                    to_path: format!("objects/destination-{index}"),
                },
            ));
            if index < MAX_PROVIDER_JOURNAL_PENDING {
                assert!(matches!(result, Err(ScenarioError::Io(_))));
            } else {
                assert!(matches!(result, Err(ScenarioError::ProviderJournalLimit)));
                FAIL_PROVIDER_JOURNAL_AFTER_PHASE.with(|hook| hook.borrow_mut().take());
            }
        }
        let journal = simulator.provider_journal_path("beta").unwrap();
        assert_eq!(
            std::fs::read_dir(journal.join("records")).unwrap().count(),
            MAX_PROVIDER_JOURNAL_PENDING
        );
        assert_eq!(
            std::fs::read_dir(journal.join("blobs")).unwrap().count(),
            MAX_PROVIDER_JOURNAL_PENDING
        );
        assert_eq!(std::fs::read_dir(&journal).unwrap().count(), 5);
        let authority = simulator
            .device("beta")
            .unwrap()
            .root
            .join(PROVIDER_DEVICE_AUTHORITY_NAME);
        let authority_metadata = std::fs::symlink_metadata(&authority).unwrap();
        assert!(authority_metadata.is_file());
        assert!(
            usize::try_from(authority_metadata.len()).unwrap()
                <= MAX_PROVIDER_AUTHORITY_BYTES
        );
        assert_eq!(
            std::fs::read_dir(simulator.device("beta").unwrap().root.clone())
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .is_ok_and(|entry| entry.file_name() == PROVIDER_DEVICE_AUTHORITY_NAME)
                })
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn provider_rename_rejects_multilink_source_destination_and_journal_blob() {
        let bytes = b"single link authority";
        let mut source_simulator = simulator_with_provider_item(bytes);
        source_simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let source_inbox = source_simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::hard_link(
            source_inbox.join("objects/source"),
            source_inbox.join("objects/source-alias"),
        )
        .unwrap();
        assert!(matches!(
            source_simulator.run_action(&action(
                2,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/source".into(),
                    to_path: "objects/destination".into(),
                },
            )),
            Err(ScenarioError::UnsafeProviderEntry(_))
        ));
        assert!(!source_inbox.join("objects/destination").exists());

        let mut destination_simulator = simulator_with_provider_item(bytes);
        destination_simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        let rename = ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: "objects/source".into(),
            to_path: "objects/destination".into(),
        };
        FAIL_PROVIDER_JOURNAL_AFTER_PHASE
            .with(|hook| hook.replace(Some(ProviderJournalPhase::Staged)));
        assert!(destination_simulator
            .run_action(&action(2, rename.clone()))
            .is_err());
        let destination_inbox = destination_simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let staging = std::fs::read_dir(destination_inbox.join(PROVIDER_TEMP_NAMESPACE))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| std::fs::read(path).unwrap() == bytes)
            .unwrap();
        std::fs::hard_link(
            &staging,
            destination_inbox
                .join(PROVIDER_TEMP_NAMESPACE)
                .join("destination-alias"),
        )
        .unwrap();
        assert!(matches!(
            destination_simulator.run_action(&action(2, rename)),
            Err(ScenarioError::UnsafeProviderEntry(_))
        ));
        assert!(!destination_inbox.join("objects/destination").exists());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn provider_named_fallback_rejects_multilink_staging_before_publication() {
        let bytes = b"named fallback hardlink bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let previous =
            PROVIDER_STAGING_MODE.with(|mode| mode.replace(ProviderStagingMode::NamedFallback));
        begin_provider_write(
            &mut simulator,
            "write",
            location("objects/destination"),
        );
        append_provider_write(&mut simulator, "write", bytes.len());
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        std::fs::hard_link(
            inbox.join(".part/write.part"),
            inbox.join(".part/write-alias.part"),
        )
        .unwrap();
        let result = finish_provider_write(&mut simulator, "write");
        PROVIDER_STAGING_MODE.with(|mode| mode.set(previous));
        assert!(matches!(result, Err(ScenarioError::UnsafeProviderEntry(_))));
        assert!(!inbox.join("objects/destination").exists());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn anonymous_and_named_staging_produce_identical_canonical_provider_snapshots() {
        fn run(mode: ProviderStagingMode) -> ProviderTreeSnapshot {
            let previous = PROVIDER_STAGING_MODE.with(|current| current.replace(mode));
            let bytes = b"staging-independent snapshot";
            let mut simulator = simulator_with_provider_item(bytes);
            begin_provider_write(
                &mut simulator,
                "write",
                location("objects/nested/destination"),
            );
            append_provider_write(&mut simulator, "write", bytes.len());
            finish_provider_write(&mut simulator, "write").unwrap();
            PROVIDER_STAGING_MODE.with(|current| current.set(previous));
            simulator.provider_snapshots().unwrap().remove(0)
        }

        let anonymous = run(ProviderStagingMode::Automatic);
        let named = run(ProviderStagingMode::NamedFallback);
        assert_eq!(anonymous, named);
        assert!(anonymous.entries.iter().all(|entry| !entry.temporary));
    }

    #[cfg(unix)]
    #[test]
    fn provider_rename_destination_is_an_independent_inode_after_source_mutation() {
        use std::os::unix::fs::MetadataExt as _;

        let bytes = b"independent destination bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: location("objects/source"),
                },
            ))
            .unwrap();
        simulator
            .run_action(&action(
                2,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/source".into(),
                    to_path: "objects/destination".into(),
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let destination = inbox.join("objects/destination");
        let retired = std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| std::fs::read(path).unwrap() == bytes)
            .unwrap();
        assert_ne!(
            std::fs::metadata(&destination).unwrap().ino(),
            std::fs::metadata(&retired).unwrap().ino()
        );
        std::fs::write(&retired, b"mutated retired source").unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), bytes);
    }

    #[test]
    fn delete_pending_handle_is_dropped_before_parent_sync_step() {
        let simulator = simulator_with_provider_item(b"ordering");
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let file = std::fs::File::open(inbox.join("objects")).unwrap();
        PROVIDER_REMOVAL_DURABILITY_STEPS.with(|steps| steps.borrow_mut().clear());
        close_provider_delete_pending_file(file);
        provider_removal_durability_hook(ProviderRemovalDurabilityStep::DirectorySyncing);
        assert_eq!(
            PROVIDER_REMOVAL_DURABILITY_STEPS.with(|steps| steps.borrow().clone()),
            vec![
                ProviderRemovalDurabilityStep::DeletePending,
                ProviderRemovalDurabilityStep::HandleDropped,
                ProviderRemovalDurabilityStep::DirectorySyncing,
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn provider_rename_post_validation_replacement_cannot_publish_attacker_bytes() {
        let bytes = b"validated rename bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let source = location("objects/source");
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: source,
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let original = inbox.join("objects/source");
        let retained = inbox.join("objects/validated-original");
        let replacement = original.clone();
        PROVIDER_POST_VALIDATION_HOOK.with(|hook| {
            hook.replace(Some((
                ProviderPostValidationOperation::Rename,
                Box::new(move || {
                    std::fs::rename(&original, &retained).unwrap();
                    std::fs::write(replacement, b"attacker rename bytes").unwrap();
                }),
            )));
        });

        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/source".into(),
                    to_path: "objects/destination".into(),
                },
            )),
            Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/source"
        ));
        assert_eq!(std::fs::read(inbox.join("objects/validated-original")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read(inbox.join("objects/source")).unwrap(),
            b"attacker rename bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn provider_remove_post_validation_replacement_cannot_delete_attacker_bytes() {
        let bytes = b"validated remove bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        let source = location("objects/source");
        simulator
            .run_action(&action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "fixture-object".into(),
                    },
                    destination: source,
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        let original = inbox.join("objects/source");
        let retained = inbox.join("objects/validated-original");
        let replacement = original.clone();
        PROVIDER_POST_VALIDATION_HOOK.with(|hook| {
            hook.replace(Some((
                ProviderPostValidationOperation::Remove,
                Box::new(move || {
                    std::fs::rename(&original, &retained).unwrap();
                    std::fs::write(replacement, b"attacker remove bytes").unwrap();
                }),
            )));
        });

        assert!(matches!(
            simulator.run_action(&action(
                2,
                ScheduledActionKind::ProviderRemove {
                    location: location("objects/source"),
                },
            )),
            Err(ScenarioError::UnsafeProviderEntry(path)) if path == "objects/source"
        ));
        assert_eq!(
            std::fs::read(inbox.join("objects/source")).unwrap(),
            b"attacker remove bytes"
        );
        assert_eq!(
            std::fs::read(inbox.join("objects/validated-original")).unwrap(),
            bytes
        );
        assert_eq!(
            std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
                .unwrap()
                .count(),
            0
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn provider_rename_consumes_source_and_remove_leaves_visible_evidence() {
        let bytes = b"rename source bytes";
        let mut simulator = simulator_with_provider_item(bytes);
        for (event_id, destination) in [
            (1, location("objects/source")),
            (2, location("objects/remove")),
        ] {
            simulator
                .run_action(&action(
                    event_id,
                    ScheduledActionKind::ProviderCopy {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination,
                    },
                ))
                .unwrap();
        }
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/source".into(),
                    to_path: "objects/destination".into(),
                },
            ))
            .unwrap();
        simulator
            .run_action(&action(
                4,
                ScheduledActionKind::ProviderRemove {
                    location: location("objects/remove"),
                },
            ))
            .unwrap();
        let inbox = simulator
            .provider_tree_path("beta", ProviderTree::Inbox)
            .unwrap();
        assert!(!inbox.join("objects/source").exists());
        std::fs::write(inbox.join("objects/source"), b"new source bytes").unwrap();
        assert_eq!(
            std::fs::read(inbox.join("objects/destination")).unwrap(),
            bytes
        );
        assert!(!inbox.join("objects/remove").exists());
        let removed = std::fs::read_dir(inbox.join(PROVIDER_REMOVED_NAMESPACE))
            .unwrap()
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            removed
                .iter()
                .filter(|entry| entry.as_slice() == bytes)
                .count(),
            2
        );
        assert_eq!(removed.iter().filter(|entry| entry.is_empty()).count(), 0);
        simulator
            .run_action(&action(
                5,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree {
                        location: location("objects/destination"),
                    },
                    destination: location("objects/copied"),
                },
            ))
            .unwrap();

        assert_eq!(std::fs::read(inbox.join("objects/copied")).unwrap(), bytes);
    }

    #[test]
    fn trace_repair_removes_orphan_provider_handles_and_tree_sources() {
        let source = location("objects/source");
        let finished = location("objects/finished");
        let copied = location("objects/copied");
        let mut actions = vec![
            action(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: "wire-static-object".into(),
                    },
                    destination: source.clone(),
                },
            ),
            action(
                2,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Tree {
                        location: source.clone(),
                    },
                    destination: finished.clone(),
                    transfer_id: "write".into(),
                },
            ),
            action(
                3,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                    len: 1,
                },
            ),
            action(
                4,
                ScheduledActionKind::FinishProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                },
            ),
            action(
                5,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree { location: finished },
                    destination: copied,
                },
            ),
            action(
                6,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "missing".into(),
                    len: 1,
                },
            ),
            action(
                7,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree {
                        location: location("objects/missing"),
                    },
                    destination: location("objects/orphan"),
                },
            ),
        ];

        repair_trace(&mut actions);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.event_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn trace_repair_treats_crash_as_dropping_provider_write_handles() {
        let mut actions = vec![
            action(
                1,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Mailbox {
                        item_id: "wire-static-object".into(),
                    },
                    destination: location("objects/destination"),
                    transfer_id: "write".into(),
                },
            ),
            action(
                2,
                ScheduledActionKind::Crash {
                    device: "beta".into(),
                },
            ),
            action(
                3,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                    len: 1,
                },
            ),
            action(
                4,
                ScheduledActionKind::FinishProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                },
            ),
        ];

        repair_trace(&mut actions);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.event_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn partition_freezes_provider_writes_until_rejoin_without_publishing() {
        let workspace = ScenarioWorkspace {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(1)),
            lineage_digest: LineageDigest::of(b"provider-write-freeze"),
            catalog_document_id: DocumentId::from_uuid(Uuid::from_u128(2)),
        };
        let scenario = Scenario::from_schedule(
            "provider-write-freeze",
            1,
            workspace,
            vec![ScenarioDevice {
                name: "beta".into(),
                device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                crdt_peer_id: CrdtPeerId::from_u64(1),
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        let bytes: Arc<[u8]> = Arc::from(&b"frozen-provider-write"[..]);
        simulator
            .mailbox
            .insert(
                "fixture-object".into(),
                ProviderItem {
                    batch_id: None,
                    kind: ProviderItemKind::Object,
                    bytes: Arc::clone(&bytes),
                },
            )
            .unwrap();

        for (event_id, transfer_id, destination) in [
            (1, "append", location("objects/append")),
            (2, "finish", location("objects/finish")),
        ] {
            simulator
                .run_action(&action(
                    event_id,
                    ScheduledActionKind::BeginProviderWrite {
                        source: ProviderSource::Mailbox {
                            item_id: "fixture-object".into(),
                        },
                        destination,
                        transfer_id: transfer_id.into(),
                    },
                ))
                .unwrap();
        }
        simulator
            .run_action(&action(
                3,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "finish".into(),
                    len: bytes.len(),
                },
            ))
            .unwrap();
        simulator
            .run_action(&action(
                4,
                ScheduledActionKind::SetProviderPartition {
                    device: "beta".into(),
                    partitioned: true,
                },
            ))
            .unwrap();

        assert!(matches!(
            simulator.run_action(&action(
                5,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "append".into(),
                    len: bytes.len(),
                },
            )),
            Err(ScenarioError::ProviderPartitioned(device)) if device == "beta"
        ));
        assert!(matches!(
            simulator.run_action(&action(
                6,
                ScheduledActionKind::FinishProviderWrite {
                    device: "beta".into(),
                    transfer_id: "finish".into(),
                },
            )),
            Err(ScenarioError::ProviderPartitioned(device)) if device == "beta"
        ));
        assert_eq!(
            simulator.device("beta").unwrap().provider.writes["append"].next,
            0
        );
        assert_eq!(
            simulator.device("beta").unwrap().provider.writes["finish"].next,
            bytes.len()
        );
        let blocked = simulator.provider_snapshots().unwrap();
        assert_eq!(
            blocked[0]
                .entries
                .iter()
                .filter(|entry| entry.temporary)
                .count(),
            2
        );
        assert!(blocked[0].entries.iter().all(|entry| entry.temporary));
        assert!(simulator.provider_ingress_receipts().is_empty());

        simulator
            .run_action(&action(
                7,
                ScheduledActionKind::SetProviderPartition {
                    device: "beta".into(),
                    partitioned: false,
                },
            ))
            .unwrap();
        simulator
            .run_action(&action(
                8,
                ScheduledActionKind::AppendProviderWrite {
                    device: "beta".into(),
                    transfer_id: "append".into(),
                    len: bytes.len(),
                },
            ))
            .unwrap();
        for (event_id, transfer_id) in [(9, "append"), (10, "finish")] {
            simulator
                .run_action(&action(
                    event_id,
                    ScheduledActionKind::FinishProviderWrite {
                        device: "beta".into(),
                        transfer_id: transfer_id.into(),
                    },
                ))
                .unwrap();
        }
        let resumed = simulator.provider_snapshots().unwrap();
        assert_eq!(
            resumed[0]
                .entries
                .iter()
                .filter(|entry| !entry.temporary)
                .count(),
            2
        );
        assert!(
            resumed[0]
                .entries
                .iter()
                .filter(|entry| entry.temporary)
                .count()
                <= 2
        );
    }

    #[test]
    fn trace_repair_keeps_partitioned_provider_write_until_rejoin() {
        let finished = location("objects/finished");
        let copied = location("objects/copied");
        let mut actions = vec![
            action(
                1,
                ScheduledActionKind::BeginProviderWrite {
                    source: ProviderSource::Mailbox {
                        item_id: "wire-static-object".into(),
                    },
                    destination: finished.clone(),
                    transfer_id: "write".into(),
                },
            ),
            action(
                2,
                ScheduledActionKind::SetProviderPartition {
                    device: "beta".into(),
                    partitioned: true,
                },
            ),
            action(
                3,
                ScheduledActionKind::FinishProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                },
            ),
            action(
                4,
                ScheduledActionKind::SetProviderPartition {
                    device: "beta".into(),
                    partitioned: false,
                },
            ),
            action(
                5,
                ScheduledActionKind::FinishProviderWrite {
                    device: "beta".into(),
                    transfer_id: "write".into(),
                },
            ),
            action(
                6,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Tree { location: finished },
                    destination: copied,
                },
            ),
        ];

        repair_trace(&mut actions);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.event_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
    }
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
    InvalidProviderPath(String),
    UnknownProviderPath(String),
    UnsafeProviderEntry(String),
    ProviderConflictingBytes(String),
    ProviderPartitioned(String),
    PartialProviderWrite(String),
    ProviderRescanLimit,
    ProviderResidueLimit(String),
    ProviderJournalLimit,
    UnsafeProviderJournal(String),
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
            Self::InvalidProviderPath(path) => write!(f, "provider path is invalid: {path}"),
            Self::UnknownProviderPath(path) => write!(f, "unknown provider path: {path}"),
            Self::UnsafeProviderEntry(path) => write!(f, "unsafe provider entry: {path}"),
            Self::ProviderConflictingBytes(path) => {
                write!(f, "conflicting provider bytes at {path}")
            }
            Self::ProviderPartitioned(device) => {
                write!(f, "provider device is partitioned: {device}")
            }
            Self::PartialProviderWrite(transfer) => {
                write!(f, "partial provider write cannot be published: {transfer}")
            }
            Self::ProviderRescanLimit => f.write_str("provider rescan exceeded explicit bound"),
            Self::ProviderResidueLimit(device) => {
                write!(f, "provider residue exceeded explicit bound for {device}")
            }
            Self::ProviderJournalLimit => {
                f.write_str("provider retry journal exceeded explicit bound")
            }
            Self::UnsafeProviderJournal(entry) => {
                write!(f, "unsafe provider retry journal: {entry}")
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
