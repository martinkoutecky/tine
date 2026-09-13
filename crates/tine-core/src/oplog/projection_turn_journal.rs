//! The projection-turn journal — the second local journal segment.
//!
//! Journal-universal durability design v4 §3.6. Projection-only turns (ingress,
//! terminal repair, superseded repair) cannot share the foreground managed-local
//! journal: managed-local append requires its physical sequence to equal the
//! semantic overlay's next sequence, and a projection-only record consumes a
//! physical sequence with no semantic transition. So it would never drain, and
//! the next ordinary save would fail the equality check.
//!
//! The fix is a second physical segment with its own monotonic counter, its own
//! generation anchor and its own checkpoint. The segment type is already generic
//! over the payload kind, so this is a new instantiation, not a new mechanism —
//! and it inherits §3.4's WAL rule (durable frontier as the torn/corrupt
//! discriminator) unchanged.
//!
//! Two deliberate differences from the foreground journal:
//!
//! * **Anchors are keyed by ENDPOINT, from store creation** (sub-design (c) §7
//!   amendment (vi)). One format, no dual grammar: a pre-(c) store never reaches
//!   journal selection, because the receipt-store claim precheck refuses it at
//!   the head of the cold open.
//! * **The drain gate is different** (§3.6). There is no semantic-prefix check,
//!   because a projection turn carries no semantic transition to be ahead of.
//!   That gate lands with the producers in packet 2a.
//!
//! Production opens this journal. The cold open selects and drains it
//! (`sync_runtime.rs`: `open_projection_turn_journal`,
//! `drain_open_projection_turn_journal`), the coordinator appends one turn per
//! projection-only origin, and `projection.rs` replays it. The architectural
//! fact is `every_projection_only_producer_reaches_the_projection_turn_journal`
//! at the end of this file's tests, which fails if a producer is unwired.
//!
//! This paragraph said the exact opposite until 2026-09 — "NOTHING IN
//! PRODUCTION OPENS THIS JOURNAL YET", citing a guard that had never been
//! written. The producers landed; the sentence did not move.

use std::collections::VecDeque;

use cap_std::fs::Dir;
use tine_storage::formats::LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION;
use tine_storage::{
    ContentDigest, DurableDirectoryPublication, LineageDigest, LocalJournalError,
    LocalJournalSegmentV2, LocalJournalSegmentV2Selection,
};
use uuid::Uuid;

use super::local_journal_drain::{
    classify_local_journal_open_error, decode_projection_turn_frame, LocalJournalOpenRefusal,
};
use super::object_store::{
    ensure_directory_nofollow, open_dir_nofollow, read_optional_regular, sync_dir_required,
};
use super::sync_layout::MANAGED_LOCAL_JOURNAL_DIR;
use super::{
    ProjectionEndpointId, ProjectionTurn, ProjectionTurnPayloadKind, SequenceDomain, TurnOrigin,
    TurnPage, WorkspaceId, PROJECTION_TURN_DERIVATION_SCHEME_V1, PROJECTION_TURN_SCHEMA_VERSION,
};

/// Sibling of the foreground journal's `clean-workspace-…` directory, under the
/// same namespace.
pub(crate) const PROJECTION_TURN_JOURNAL_WORKSPACE_PREFIX: &str = "projection-turns";

const PROJECTION_TURN_ANCHOR_MAGIC: &[u8; 8] = b"TINEPTA1";
const PROJECTION_TURN_ANCHOR_SCHEMA: u32 = 1;
pub(crate) const PROJECTION_TURN_ANCHOR_BYTES: usize = 1024;
const PROJECTION_TURN_CHECKPOINT_BYTES: u64 = 16 * 1024;
const PROJECTION_TURN_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

const CHECKSUM_BYTES: usize = 32;
const CHECKSUM_OFFSET: usize = PROJECTION_TURN_ANCHOR_BYTES - CHECKSUM_BYTES;
const CHECKPOINT_CAPACITY: usize = 256;
const SEGMENT_NAME_CAPACITY: usize = 160;

#[cfg(not(test))]
const PROJECTION_TURN_COMPACTION_FRAME_THRESHOLD: u64 = 64;
#[cfg(test)]
const PROJECTION_TURN_COMPACTION_FRAME_THRESHOLD: u64 = 4;

const SCHEMA_OFFSET: usize = 8;
const PROTOCOL_OFFSET: usize = 12;
const SELECTOR_GENERATION_OFFSET: usize = 16;
const WORKSPACE_OFFSET: usize = 24;
const LINEAGE_OFFSET: usize = 40;
const ENDPOINT_OFFSET: usize = 72;
const DEVICE_OFFSET: usize = 88;
const BASE_SEQUENCE_OFFSET: usize = 104;
const CHECKPOINT_LENGTH_OFFSET: usize = 112;
const CHECKPOINT_OFFSET: usize = 114;
const SEGMENT_ID_OFFSET: usize = CHECKPOINT_OFFSET + CHECKPOINT_CAPACITY;
const SEGMENT_NAME_DIGEST_OFFSET: usize = SEGMENT_ID_OFFSET + 16;
const SEGMENT_NAME_LENGTH_OFFSET: usize = SEGMENT_NAME_DIGEST_OFFSET + 32;
const SEGMENT_NAME_OFFSET: usize = SEGMENT_NAME_LENGTH_OFFSET + 2;
const RESERVED_OFFSET: usize = SEGMENT_NAME_OFFSET + SEGMENT_NAME_CAPACITY;

/// Canonical, checkpointable drained prefix of one endpoint's turn queue.
///
/// The projection-turn domain checkpoints independently of the managed-local
/// domain (§3.6); nothing here is comparable to `ManagedLocalDrainCheckpoint`'s
/// sequence, and neither counter ever meets the other.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionTurnCheckpoint {
    schema_version: u32,
    endpoint_id: ProjectionEndpointId,
    device_id: Uuid,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    next_sequence: u64,
}

impl ProjectionTurnCheckpoint {
    pub(crate) const fn initial(
        endpoint_id: ProjectionEndpointId,
        device_id: Uuid,
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
    ) -> Self {
        Self {
            schema_version: PROJECTION_TURN_CHECKPOINT_SCHEMA_VERSION,
            endpoint_id,
            device_id,
            workspace_id,
            lineage_digest,
            next_sequence: 0,
        }
    }

    pub(crate) const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub(crate) const fn endpoint_id(&self) -> ProjectionEndpointId {
        self.endpoint_id
    }

    pub(crate) const fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub(crate) const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub(crate) const fn lineage_digest(&self) -> LineageDigest {
        self.lineage_digest
    }

    pub(crate) fn advanced_to(&self, next_sequence: u64) -> Self {
        let mut advanced = self.clone();
        advanced.next_sequence = next_sequence;
        advanced
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        postcard::to_allocvec(self).map_err(|error| error.to_string())
    }

    pub(crate) fn decode(
        bytes: &[u8],
        endpoint_id: ProjectionEndpointId,
        device_id: Uuid,
        workspace_id: WorkspaceId,
        lineage_digest: LineageDigest,
    ) -> Result<Self, String> {
        let checkpoint: Self = postcard::from_bytes(bytes).map_err(|error| error.to_string())?;
        if checkpoint.schema_version != PROJECTION_TURN_CHECKPOINT_SCHEMA_VERSION
            || checkpoint.endpoint_id != endpoint_id
            || checkpoint.device_id != device_id
            || checkpoint.workspace_id != workspace_id
            || checkpoint.lineage_digest != lineage_digest
            || postcard::to_allocvec(&checkpoint).map_err(|error| error.to_string())? != bytes
        {
            return Err("projection turn checkpoint binding is invalid".into());
        }
        Ok(checkpoint)
    }
}

/// The endpoint-keyed generation anchor. It is the bootstrap locator for one
/// selector generation's segment and its drained-prefix checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionTurnGenerationAnchor {
    selector_generation: u64,
    checkpoint: ProjectionTurnCheckpoint,
    selection: LocalJournalSegmentV2Selection,
}

impl ProjectionTurnGenerationAnchor {
    pub(crate) fn new(
        selector_generation: u64,
        checkpoint: ProjectionTurnCheckpoint,
        segment_id: Uuid,
    ) -> Result<Self, String> {
        if selector_generation == 0 {
            return Err("projection turn selector generation must be positive".into());
        }
        if segment_id.is_nil() {
            return Err("projection turn segment identity must be random and nonzero".into());
        }
        let segment_name =
            projection_turn_segment_name(checkpoint.endpoint_id(), selector_generation, segment_id);
        let selection = LocalJournalSegmentV2Selection::new(
            segment_name,
            segment_id,
            checkpoint.device_id(),
            checkpoint.next_sequence(),
        )
        .map_err(|error| format!("cannot construct projection turn selection: {error}"))?;
        Ok(Self {
            selector_generation,
            checkpoint,
            selection,
        })
    }

    pub(crate) const fn selector_generation(&self) -> u64 {
        self.selector_generation
    }

    pub(crate) const fn checkpoint(&self) -> &ProjectionTurnCheckpoint {
        &self.checkpoint
    }

    pub(crate) const fn selection(&self) -> &LocalJournalSegmentV2Selection {
        &self.selection
    }

    pub(crate) fn encode(&self) -> Result<[u8; PROJECTION_TURN_ANCHOR_BYTES], String> {
        if self.selection.base_sequence() != self.checkpoint.next_sequence() {
            return Err("projection turn anchor checkpoint is not its segment base".into());
        }
        let checkpoint = self.checkpoint.encode()?;
        if checkpoint.is_empty() || checkpoint.len() > CHECKPOINT_CAPACITY {
            return Err("projection turn checkpoint is too large for its anchor".into());
        }
        let checkpoint_length = u16::try_from(checkpoint.len())
            .map_err(|_| "projection turn checkpoint is too large for its anchor")?;
        let segment_name = self.selection.segment_name().as_bytes();
        if segment_name.is_empty() || segment_name.len() > SEGMENT_NAME_CAPACITY {
            return Err("projection turn segment name is not canonically bounded".into());
        }
        let segment_name_length = u16::try_from(segment_name.len())
            .map_err(|_| "projection turn segment name is too long")?;

        let mut bytes = [0_u8; PROJECTION_TURN_ANCHOR_BYTES];
        bytes[..8].copy_from_slice(PROJECTION_TURN_ANCHOR_MAGIC);
        bytes[SCHEMA_OFFSET..PROTOCOL_OFFSET]
            .copy_from_slice(&PROJECTION_TURN_ANCHOR_SCHEMA.to_be_bytes());
        bytes[PROTOCOL_OFFSET..SELECTOR_GENERATION_OFFSET]
            .copy_from_slice(&LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION.to_be_bytes());
        bytes[SELECTOR_GENERATION_OFFSET..WORKSPACE_OFFSET]
            .copy_from_slice(&self.selector_generation.to_be_bytes());
        bytes[WORKSPACE_OFFSET..LINEAGE_OFFSET]
            .copy_from_slice(self.checkpoint.workspace_id().as_uuid().as_bytes());
        bytes[LINEAGE_OFFSET..ENDPOINT_OFFSET]
            .copy_from_slice(self.checkpoint.lineage_digest().as_bytes());
        bytes[ENDPOINT_OFFSET..DEVICE_OFFSET]
            .copy_from_slice(self.checkpoint.endpoint_id().as_uuid().as_bytes());
        bytes[DEVICE_OFFSET..BASE_SEQUENCE_OFFSET]
            .copy_from_slice(self.checkpoint.device_id().as_bytes());
        bytes[BASE_SEQUENCE_OFFSET..CHECKPOINT_LENGTH_OFFSET]
            .copy_from_slice(&self.selection.base_sequence().to_be_bytes());
        bytes[CHECKPOINT_LENGTH_OFFSET..CHECKPOINT_OFFSET]
            .copy_from_slice(&checkpoint_length.to_be_bytes());
        bytes[CHECKPOINT_OFFSET..CHECKPOINT_OFFSET + checkpoint.len()].copy_from_slice(&checkpoint);
        bytes[SEGMENT_ID_OFFSET..SEGMENT_NAME_DIGEST_OFFSET]
            .copy_from_slice(self.selection.segment_id().as_bytes());
        bytes[SEGMENT_NAME_DIGEST_OFFSET..SEGMENT_NAME_LENGTH_OFFSET]
            .copy_from_slice(self.selection.segment_name_digest().as_bytes());
        bytes[SEGMENT_NAME_LENGTH_OFFSET..SEGMENT_NAME_OFFSET]
            .copy_from_slice(&segment_name_length.to_be_bytes());
        bytes[SEGMENT_NAME_OFFSET..SEGMENT_NAME_OFFSET + segment_name.len()]
            .copy_from_slice(segment_name);
        let checksum = ContentDigest::of(&bytes[..CHECKSUM_OFFSET]);
        bytes[CHECKSUM_OFFSET..].copy_from_slice(checksum.as_bytes());
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
        expected_selector_generation: u64,
        expected_endpoint_id: ProjectionEndpointId,
        expected_device_id: Uuid,
        expected_workspace_id: WorkspaceId,
        expected_lineage_digest: LineageDigest,
    ) -> Result<Self, String> {
        if !bytes.starts_with(PROJECTION_TURN_ANCHOR_MAGIC) {
            return Err("projection turn anchor magic is invalid".into());
        }
        if bytes.len() != PROJECTION_TURN_ANCHOR_BYTES {
            return Err("projection turn anchor has a noncanonical length".into());
        }
        if read_u32(bytes, SCHEMA_OFFSET)? != PROJECTION_TURN_ANCHOR_SCHEMA
            || read_u32(bytes, PROTOCOL_OFFSET)? != LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION
        {
            return Err("projection turn anchor version is unsupported".into());
        }
        if bytes[CHECKSUM_OFFSET..] != ContentDigest::of(&bytes[..CHECKSUM_OFFSET]).as_bytes()[..] {
            return Err("projection turn anchor checksum mismatch".into());
        }
        let selector_generation = read_u64(bytes, SELECTOR_GENERATION_OFFSET)?;
        if selector_generation == 0 || selector_generation != expected_selector_generation {
            return Err("projection turn selector generation is mismatched".into());
        }
        let workspace_id = WorkspaceId::from_uuid(read_uuid(bytes, WORKSPACE_OFFSET)?);
        let lineage_digest = read_lineage_digest(bytes, LINEAGE_OFFSET)?;
        let endpoint_id = ProjectionEndpointId::from_uuid(read_uuid(bytes, ENDPOINT_OFFSET)?);
        let device_id = read_uuid(bytes, DEVICE_OFFSET)?;
        if workspace_id != expected_workspace_id
            || lineage_digest != expected_lineage_digest
            || endpoint_id != expected_endpoint_id
            || device_id != expected_device_id
        {
            return Err("projection turn anchor binding is invalid".into());
        }
        let base_sequence = read_u64(bytes, BASE_SEQUENCE_OFFSET)?;
        let checkpoint_length = read_u16(bytes, CHECKPOINT_LENGTH_OFFSET)? as usize;
        if checkpoint_length == 0 || checkpoint_length > CHECKPOINT_CAPACITY {
            return Err("projection turn checkpoint length is invalid".into());
        }
        if bytes[CHECKPOINT_OFFSET + checkpoint_length..CHECKPOINT_OFFSET + CHECKPOINT_CAPACITY]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err("projection turn checkpoint padding is nonzero".into());
        }
        let checkpoint = ProjectionTurnCheckpoint::decode(
            &bytes[CHECKPOINT_OFFSET..CHECKPOINT_OFFSET + checkpoint_length],
            endpoint_id,
            device_id,
            workspace_id,
            lineage_digest,
        )?;
        if checkpoint.next_sequence() != base_sequence {
            return Err("projection turn anchor checkpoint is not its segment base".into());
        }
        let segment_id = read_uuid(bytes, SEGMENT_ID_OFFSET)?;
        if segment_id.is_nil() {
            return Err("projection turn segment identity is invalid".into());
        }
        let stored_name_digest = read_digest(bytes, SEGMENT_NAME_DIGEST_OFFSET)?;
        let segment_name_length = read_u16(bytes, SEGMENT_NAME_LENGTH_OFFSET)? as usize;
        if segment_name_length == 0 || segment_name_length > SEGMENT_NAME_CAPACITY {
            return Err("projection turn segment name length is invalid".into());
        }
        if bytes
            [SEGMENT_NAME_OFFSET + segment_name_length..SEGMENT_NAME_OFFSET + SEGMENT_NAME_CAPACITY]
            .iter()
            .any(|byte| *byte != 0)
            || bytes[RESERVED_OFFSET..CHECKSUM_OFFSET]
                .iter()
                .any(|byte| *byte != 0)
        {
            return Err("projection turn anchor reserved bytes are nonzero".into());
        }
        let segment_name = std::str::from_utf8(
            &bytes[SEGMENT_NAME_OFFSET..SEGMENT_NAME_OFFSET + segment_name_length],
        )
        .map_err(|_| "projection turn segment name is not UTF-8")?;
        if parse_projection_turn_segment_name(segment_name, endpoint_id)
            != Some((selector_generation, segment_id))
        {
            return Err("projection turn segment name binding is invalid".into());
        }
        let selection =
            LocalJournalSegmentV2Selection::new(segment_name, segment_id, device_id, base_sequence)
                .map_err(|error| {
                    format!("cannot reconstruct projection turn selection: {error}")
                })?;
        if selection.segment_name_digest() != stored_name_digest {
            return Err("projection turn segment name digest is invalid".into());
        }
        Ok(Self {
            selector_generation,
            checkpoint,
            selection,
        })
    }
}

pub(crate) fn projection_turn_journal_workspace_name(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
) -> String {
    format!("{PROJECTION_TURN_JOURNAL_WORKSPACE_PREFIX}-{workspace_id}-{lineage_digest}")
}

pub(crate) fn projection_turn_anchor_name(
    endpoint_id: ProjectionEndpointId,
    selector_generation: u64,
) -> String {
    format!(
        "endpoint-{}-selector-{selector_generation:020}.anchor-v2",
        endpoint_id.as_uuid().simple()
    )
}

pub(crate) fn projection_turn_segment_name(
    endpoint_id: ProjectionEndpointId,
    selector_generation: u64,
    segment_id: Uuid,
) -> String {
    format!(
        "endpoint-{}-selector-{selector_generation:020}-segment-{}.journal-v2",
        endpoint_id.as_uuid().simple(),
        segment_id.simple()
    )
}

pub(crate) fn parse_projection_turn_anchor_name(
    name: &str,
    expected_endpoint_id: ProjectionEndpointId,
) -> Option<u64> {
    let prefix = format!(
        "endpoint-{}-selector-",
        expected_endpoint_id.as_uuid().simple()
    );
    let generation = name.strip_prefix(&prefix)?.strip_suffix(".anchor-v2")?;
    if generation.len() != 20 || !generation.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let selector_generation = generation.parse().ok()?;
    (selector_generation != 0
        && projection_turn_anchor_name(expected_endpoint_id, selector_generation) == name)
        .then_some(selector_generation)
}

pub(crate) fn parse_projection_turn_segment_name(
    name: &str,
    expected_endpoint_id: ProjectionEndpointId,
) -> Option<(u64, Uuid)> {
    let prefix = format!(
        "endpoint-{}-selector-",
        expected_endpoint_id.as_uuid().simple()
    );
    let rest = name.strip_prefix(&prefix)?.strip_suffix(".journal-v2")?;
    let (generation, segment_id) = rest.split_once("-segment-")?;
    if generation.len() != 20 || !generation.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let selector_generation = generation.parse().ok()?;
    let segment_id = Uuid::parse_str(segment_id).ok()?;
    (projection_turn_segment_name(expected_endpoint_id, selector_generation, segment_id) == name)
        .then_some((selector_generation, segment_id))
}

pub(crate) fn projection_turn_checkpoint_filename(next_sequence: u64) -> String {
    format!("checkpoint-{next_sequence:020}.bin")
}

fn projection_turn_checkpoint_sequence(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("checkpoint-")?.strip_suffix(".bin")?;
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let sequence = digits.parse().ok()?;
    (format!("{sequence:020}") == digits).then_some(sequence)
}

/// Why the projection-turn journal could not be opened.
///
/// `Refused` carries §3.4's per-variant classification so the caller surfaces
/// the right refusal: `MS-REF-DISK-CORRUPT` retains evidence and never
/// truncates, while a concurrent instance or an unsafe filesystem keeps its own
/// existing refusal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionTurnJournalError {
    Refused(LocalJournalOpenRefusal),
    Invalid(String),
}

impl std::fmt::Display for ProjectionTurnJournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(formatter),
            Self::Invalid(detail) => {
                write!(formatter, "projection turn journal is invalid: {detail}")
            }
        }
    }
}

impl std::error::Error for ProjectionTurnJournalError {}

impl From<LocalJournalError> for ProjectionTurnJournalError {
    fn from(error: LocalJournalError) -> Self {
        Self::Refused(classify_local_journal_open_error(&error))
    }
}

/// One open projection-turn journal plus the undrained turns it retains.
pub(crate) struct ProjectionTurnJournalState {
    pub(crate) directory: Dir,
    pub(crate) selector_generation: u64,
    pub(crate) journal: LocalJournalSegmentV2<ProjectionTurnPayloadKind>,
    pub(crate) checkpoint: ProjectionTurnCheckpoint,
    /// Turns strictly after the checkpoint: everything still owed. Open-time
    /// frames are decoded once, while live appends enter the identical queue.
    pending: VecDeque<ProjectionTurn>,
    /// Whether an older selector generation's anchor is still present.
    pub(crate) cleanup_pending: bool,
}

impl std::fmt::Debug for ProjectionTurnJournalState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectionTurnJournalState")
            .field("selector_generation", &self.selector_generation)
            .field("next_sequence", &self.journal.next_sequence())
            .field("checkpoint", &self.checkpoint)
            .field("undrained_turns", &self.pending.len())
            .field("cleanup_pending", &self.cleanup_pending)
            .finish()
    }
}

impl ProjectionTurnJournalState {
    /// The authenticated generation and durable append prefix selected when a
    /// full-history reconstruction fences local authority. Checkpoints may
    /// advance while the fence is held, but this tuple must not move to a new
    /// generation until reinstall succeeds.
    pub(crate) fn recovery_fence(&self) -> (u64, &str, u64, u64) {
        (
            self.selector_generation,
            self.journal.selection().segment_name(),
            self.journal.selection().base_sequence(),
            self.journal.next_sequence(),
        )
    }

    /// Decode every undrained frame into the single [`ProjectionTurn`] view.
    pub(crate) fn undrained_turns(
        &self,
    ) -> Result<Vec<ProjectionTurn>, ProjectionTurnJournalError> {
        Ok(self.pending.iter().cloned().collect())
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn front(&self) -> Option<&ProjectionTurn> {
        self.pending.front()
    }

    pub(crate) fn retains_batch(&self, batch_id: super::BatchId) -> bool {
        self.pending
            .iter()
            .any(|turn| turn.origin.batch_id() == Some(batch_id))
    }

    /// Append one projection-domain turn before any graph mutation it
    /// authorizes. The segment supplies the sequence; all remaining bindings
    /// come from the authenticated checkpoint opened for this endpoint.
    pub(crate) fn append(
        &mut self,
        origin: TurnOrigin,
        pages: Vec<TurnPage>,
    ) -> Result<ProjectionTurn, ProjectionTurnJournalError> {
        let turn = ProjectionTurn {
            schema_version: PROJECTION_TURN_SCHEMA_VERSION,
            derivation_scheme: PROJECTION_TURN_DERIVATION_SCHEME_V1,
            workspace_id: self.checkpoint.workspace_id(),
            lineage_digest: self.checkpoint.lineage_digest(),
            device_id: self.checkpoint.device_id(),
            endpoint_id: self.checkpoint.endpoint_id(),
            sequence: self.journal.next_sequence(),
            domain: SequenceDomain::ProjectionTurn,
            origin,
            pages,
        };
        let payload = turn
            .encode()
            .map_err(|error| ProjectionTurnJournalError::Invalid(error.to_string()))?;
        self.journal
            .append(ProjectionTurnPayloadKind::TurnV1, &payload)
            .map_err(|error| {
                ProjectionTurnJournalError::Invalid(format!(
                    "projection turn append outcome is unknown: {error}"
                ))
            })?;
        self.pending.push_back(turn.clone());
        Ok(turn)
    }

    /// Advance exactly one independent projection-turn prefix. The durable
    /// checkpoint is published before the in-memory queue forgets the turn.
    pub(crate) fn checkpoint_front(&mut self) -> Result<ProjectionTurn, String> {
        let turn = self
            .pending
            .front()
            .ok_or_else(|| "projection turn checkpoint has no pending turn".to_owned())?;
        if turn.sequence != self.checkpoint.next_sequence() {
            return Err(format!(
                "projection turn queue is out of order: expected {}, found {}",
                self.checkpoint.next_sequence(),
                turn.sequence
            ));
        }
        let checkpoint = self.checkpoint.advanced_to(turn.sequence.saturating_add(1));
        persist_projection_turn_checkpoint(&self.directory, &checkpoint)?;
        self.checkpoint = checkpoint;
        let completed = self
            .pending
            .pop_front()
            .expect("checked projection turn front");
        self.compact_if_needed()
            .map_err(|error| error.to_string())?;
        Ok(completed)
    }

    /// Replace a completely drained suffix with a new anchor whose embedded
    /// checkpoint names the same next sequence. The new segment and its
    /// marker-last anchor are durable before any predecessor evidence is
    /// retired, so every crash prefix selects either the old complete chain or
    /// the new complete chain. Checkpoint files covered by the new anchor are
    /// then redundant rather than lifetime open inputs.
    fn compact_if_needed(&mut self) -> Result<bool, ProjectionTurnJournalError> {
        if self.cleanup_pending {
            self.retry_history_cleanup()?;
        }
        if !self.pending.is_empty() {
            return Ok(false);
        }
        if self.checkpoint.next_sequence() != self.journal.next_sequence() {
            return Err(ProjectionTurnJournalError::Invalid(
                "projection turn compaction checkpoint is not current".into(),
            ));
        }
        let suffix_frames = self
            .journal
            .next_sequence()
            .checked_sub(self.journal.selection().base_sequence())
            .ok_or_else(|| {
                ProjectionTurnJournalError::Invalid(
                    "projection turn journal sequence is before its base".into(),
                )
            })?;
        if suffix_frames < PROJECTION_TURN_COMPACTION_FRAME_THRESHOLD {
            return Ok(false);
        }
        let successor_generation = self.selector_generation.checked_add(1).ok_or_else(|| {
            ProjectionTurnJournalError::Invalid(
                "projection turn selector generation overflow".into(),
            )
        })?;
        let (successor, _) = prepare_projection_turn_journal(
            &self.directory,
            self.checkpoint.clone(),
            successor_generation,
        )?;
        let predecessor = std::mem::replace(&mut self.journal, successor);
        self.selector_generation = successor_generation;
        drop(predecessor);
        self.cleanup_pending = true;
        self.retry_history_cleanup()?;
        Ok(true)
    }

    fn retry_history_cleanup(&mut self) -> Result<(), ProjectionTurnJournalError> {
        if !self.cleanup_pending {
            return Ok(());
        }
        self.cleanup_history(self.selector_generation)?;
        self.cleanup_pending = false;
        Ok(())
    }

    fn cleanup_history(&self, retained_generation: u64) -> Result<(), ProjectionTurnJournalError> {
        let invalid = ProjectionTurnJournalError::Invalid;
        let names = directory_names(&self.directory).map_err(invalid)?;
        for (generation, anchor_name) in names.iter().filter_map(|name| {
            parse_projection_turn_anchor_name(name, self.checkpoint.endpoint_id())
                .map(|generation| (generation, name.clone()))
        }) {
            if generation >= retained_generation {
                continue;
            }
            let bytes = read_optional_regular(
                &self.directory,
                &anchor_name,
                PROJECTION_TURN_ANCHOR_BYTES as u64,
                None,
            )
            .map_err(|error| {
                invalid(format!(
                    "cannot read retired projection turn anchor {anchor_name}: {error}"
                ))
            })?
            .ok_or_else(|| {
                invalid(format!(
                    "retired projection turn anchor {anchor_name} disappeared"
                ))
            })?;
            let anchor = ProjectionTurnGenerationAnchor::decode(
                &bytes,
                generation,
                self.checkpoint.endpoint_id(),
                self.checkpoint.device_id(),
                self.checkpoint.workspace_id(),
                self.checkpoint.lineage_digest(),
            )
            .map_err(|error| {
                invalid(format!(
                    "retired projection turn anchor {anchor_name} is invalid: {error}"
                ))
            })?;
            // Remove the tuple first. If cleanup is interrupted, the old
            // anchor remains a discoverable retry root while the greater new
            // anchor stays authoritative.
            for tuple_name in [
                anchor.selection().segment_name(),
                anchor.selection().frontier_name(),
            ] {
                if let Err(error) = self.directory.remove_file(tuple_name) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(invalid(format!(
                            "cannot retire projection turn tuple {tuple_name}: {error}"
                        )));
                    }
                }
            }
            if let Err(error) = self.directory.remove_file(&anchor_name) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(invalid(format!(
                        "cannot retire projection turn anchor {anchor_name}: {error}"
                    )));
                }
            }
        }
        for name in names {
            if projection_turn_checkpoint_sequence(&name)
                .is_some_and(|sequence| sequence <= self.checkpoint.next_sequence())
            {
                if let Err(error) = self.directory.remove_file(&name) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(invalid(format!(
                            "cannot retire projection turn checkpoint {name}: {error}"
                        )));
                    }
                }
            }
        }
        sync_dir_required(&self.directory)
            .map_err(|error| invalid(format!("cannot sync projection turn cleanup: {error}")))
    }
}

/// Open (or create) one endpoint's projection-turn journal under the managed
/// local-journal namespace.
///
/// The open protocol mirrors the foreground journal's: the greatest
/// authenticated selector generation is authoritative, lower selectors are
/// redundant recovery history, and contiguous checkpoint files above the
/// anchor's own checkpoint advance the drained prefix.
pub(crate) fn open_projection_turn_journal(
    application_runtime_root: &std::path::Path,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    endpoint_id: ProjectionEndpointId,
    device_id: Uuid,
) -> Result<ProjectionTurnJournalState, ProjectionTurnJournalError> {
    let invalid = |error: String| ProjectionTurnJournalError::Invalid(error);
    let root = Dir::open_ambient_dir(application_runtime_root, cap_std::ambient_authority())
        .map_err(|error| {
            invalid(format!(
                "cannot retain projection turn journal root: {error}"
            ))
        })?;
    ensure_directory_nofollow(&root, MANAGED_LOCAL_JOURNAL_DIR)
        .map_err(|error| invalid(error.to_string()))?;
    let namespace = open_dir_nofollow(&root, MANAGED_LOCAL_JOURNAL_DIR)
        .map_err(|error| invalid(error.to_string()))?;
    let workspace_name = projection_turn_journal_workspace_name(workspace_id, lineage_digest);
    ensure_directory_nofollow(&namespace, &workspace_name)
        .map_err(|error| invalid(error.to_string()))?;
    let directory = open_dir_nofollow(&namespace, &workspace_name)
        .map_err(|error| invalid(error.to_string()))?;

    let names = directory_names(&directory).map_err(invalid)?;
    let mut anchors = names
        .iter()
        .filter_map(|name| {
            parse_projection_turn_anchor_name(name, endpoint_id)
                .map(|generation| (generation, name.clone()))
        })
        .collect::<Vec<_>>();
    anchors.sort_unstable_by_key(|(generation, _)| *generation);
    let (selector_generation, journal, mut checkpoint) = match anchors.pop() {
        Some((selector_generation, anchor_name)) => {
            let bytes = read_optional_regular(
                &directory,
                &anchor_name,
                PROJECTION_TURN_ANCHOR_BYTES as u64,
                None,
            )
            .map_err(|error| {
                invalid(format!(
                    "cannot read projection turn anchor {anchor_name}: {error}"
                ))
            })?
            .ok_or_else(|| invalid(format!("projection turn anchor {anchor_name} disappeared")))?;
            let anchor = ProjectionTurnGenerationAnchor::decode(
                &bytes,
                selector_generation,
                endpoint_id,
                device_id,
                workspace_id,
                lineage_digest,
            )
            .map_err(|error| {
                invalid(format!(
                    "projection turn anchor {anchor_name} is invalid: {error}"
                ))
            })?;
            let (segment, _) =
                LocalJournalSegmentV2::open_selected(&directory, anchor.selection())?;
            (selector_generation, segment, anchor.checkpoint().clone())
        }
        None => {
            let checkpoint = ProjectionTurnCheckpoint::initial(
                endpoint_id,
                device_id,
                workspace_id,
                lineage_digest,
            );
            let (segment, anchor) = prepare_projection_turn_journal(&directory, checkpoint, 1)?;
            (1, segment, anchor.checkpoint().clone())
        }
    };

    let mut recovered_frames = Vec::new();
    journal.replay(|frame| recovered_frames.push(frame))?;
    if journal.selection().base_sequence() != checkpoint.next_sequence()
        || journal
            .selection()
            .base_sequence()
            .checked_add(recovered_frames.len() as u64)
            != Some(journal.next_sequence())
    {
        return Err(invalid(
            "projection turn anchor/sequence binding is inconsistent".into(),
        ));
    }

    let mut checkpoint_sequences = names
        .iter()
        .filter_map(|name| projection_turn_checkpoint_sequence(name))
        .filter(|sequence| *sequence > checkpoint.next_sequence())
        .collect::<Vec<_>>();
    checkpoint_sequences.sort_unstable();
    checkpoint_sequences.dedup();
    let mut expected_checkpoint = checkpoint.next_sequence().saturating_add(1);
    for next_sequence in checkpoint_sequences {
        if next_sequence != expected_checkpoint || next_sequence > journal.next_sequence() {
            return Err(invalid(format!(
                "projection turn checkpoint sequence is not a contiguous journal prefix: \
                 expected {expected_checkpoint}, found {next_sequence}"
            )));
        }
        let filename = projection_turn_checkpoint_filename(next_sequence);
        let bytes = read_optional_regular(
            &directory,
            &filename,
            PROJECTION_TURN_CHECKPOINT_BYTES,
            None,
        )
        .map_err(|error| {
            invalid(format!(
                "cannot read projection turn checkpoint {filename}: {error}"
            ))
        })?
        .ok_or_else(|| invalid(format!("projection turn checkpoint {filename} disappeared")))?;
        checkpoint = ProjectionTurnCheckpoint::decode(
            &bytes,
            endpoint_id,
            device_id,
            workspace_id,
            lineage_digest,
        )
        .map_err(|error| {
            invalid(format!(
                "projection turn checkpoint {filename} is invalid: {error}"
            ))
        })?;
        if checkpoint.next_sequence() != next_sequence {
            return Err(invalid(format!(
                "projection turn checkpoint {filename} names sequence {}",
                checkpoint.next_sequence()
            )));
        }
        expected_checkpoint = next_sequence.saturating_add(1);
    }

    let checkpointed = checkpoint
        .next_sequence()
        .checked_sub(journal.selection().base_sequence())
        .ok_or_else(|| {
            invalid("projection turn checkpoint is behind its journal generation".into())
        })?;
    let checkpointed = usize::try_from(checkpointed)
        .map_err(|_| invalid("projection turn checkpoint exceeds addressable memory".into()))?;
    if checkpointed > recovered_frames.len() {
        return Err(invalid(
            "projection turn checkpoint is ahead of its journal generation".into(),
        ));
    }
    let frames = recovered_frames.split_off(checkpointed);
    let pending = frames
        .iter()
        .map(|frame| {
            decode_projection_turn_frame(frame)
                .map_err(|error| ProjectionTurnJournalError::Invalid(error.to_string()))
        })
        .collect::<Result<VecDeque<_>, _>>()?;

    let mut state = ProjectionTurnJournalState {
        directory,
        selector_generation,
        journal,
        checkpoint,
        pending,
        cleanup_pending: !anchors.is_empty(),
    };
    // Upgrade/self-heal: an installation created before bounded generations
    // may already have a fully drained lifetime suffix. Its first new-code open
    // authenticates that suffix once, then rotates it so every later open is
    // bounded by the live generation.
    state.compact_if_needed()?;
    Ok(state)
}

fn prepare_projection_turn_journal(
    directory: &Dir,
    checkpoint: ProjectionTurnCheckpoint,
    selector_generation: u64,
) -> Result<
    (
        LocalJournalSegmentV2<ProjectionTurnPayloadKind>,
        ProjectionTurnGenerationAnchor,
    ),
    ProjectionTurnJournalError,
> {
    let invalid = ProjectionTurnJournalError::Invalid;
    let publication = DurableDirectoryPublication::open(directory).map_err(|error| {
        invalid(format!(
            "projection turn journal publication is unavailable: {error}"
        ))
    })?;
    let anchor =
        ProjectionTurnGenerationAnchor::new(selector_generation, checkpoint, Uuid::new_v4())
            .map_err(|error| {
                invalid(format!("cannot construct projection turn journal: {error}"))
            })?;
    let anchor_name = projection_turn_anchor_name(
        anchor.checkpoint().endpoint_id(),
        anchor.selector_generation(),
    );
    let anchor_bytes = anchor
        .encode()
        .map_err(|error| invalid(format!("cannot encode projection turn anchor: {error}")))?;
    LocalJournalSegmentV2::<ProjectionTurnPayloadKind>::prepare_single_writer(
        directory,
        anchor.selection(),
    )?;
    let (journal, recovery) = LocalJournalSegmentV2::open_selected(directory, anchor.selection())?;
    if recovery.frames_recovered != 0
        || recovery.discarded_tail_bytes != 0
        || journal.next_sequence() != anchor.checkpoint().next_sequence()
    {
        return Err(invalid(
            "a fresh projection turn journal is not empty".into(),
        ));
    }
    publication
        .publish_new_exact_single_writer(&anchor_name, &anchor_bytes)
        .map_err(|error| {
            invalid(format!(
                "cannot publish projection turn anchor {anchor_name}: {error}"
            ))
        })?;
    Ok((journal, anchor))
}

/// Publish one drained-prefix checkpoint. Idempotent for identical evidence;
/// a differing byte sequence at the same name is a hard refusal.
pub(crate) fn persist_projection_turn_checkpoint(
    directory: &Dir,
    checkpoint: &ProjectionTurnCheckpoint,
) -> Result<(), String> {
    let filename = projection_turn_checkpoint_filename(checkpoint.next_sequence());
    let bytes = checkpoint.encode()?;
    match read_optional_regular(directory, &filename, PROJECTION_TURN_CHECKPOINT_BYTES, None)
        .map_err(|error| format!("cannot read projection turn checkpoint {filename}: {error}"))?
    {
        Some(existing) if existing == bytes => Ok(()),
        Some(_) => Err(format!(
            "projection turn checkpoint {filename} collides with different durable evidence"
        )),
        // This directory is private to one local runtime actor, so the
        // Android-safe proven-absent atomic-rename fallback is available.
        None => DurableDirectoryPublication::open(directory)
            .map_err(|error| {
                format!("projection turn checkpoint publication is unavailable: {error}")
            })?
            .publish_new_exact_single_writer(&filename, &bytes)
            .map_err(|error| {
                format!("cannot publish projection turn checkpoint {filename}: {error}")
            }),
    }
}

fn directory_names(directory: &Dir) -> Result<Vec<String>, String> {
    let entries = directory
        .entries()
        .map_err(|error| format!("cannot enumerate projection turn journal: {error}"))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("cannot enumerate projection turn journal entry: {error}"))?;
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|slice| slice.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or_else(|| "projection turn anchor is truncated".into())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| "projection turn anchor is truncated".into())
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    bytes
        .get(offset..offset + 8)
        .and_then(|slice| slice.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| "projection turn anchor is truncated".into())
}

fn read_uuid(bytes: &[u8], offset: usize) -> Result<Uuid, String> {
    bytes
        .get(offset..offset + 16)
        .and_then(|slice| Uuid::from_slice(slice).ok())
        .ok_or_else(|| "projection turn anchor is truncated".into())
}

fn read_digest(bytes: &[u8], offset: usize) -> Result<ContentDigest, String> {
    let slice: [u8; 32] = bytes
        .get(offset..offset + 32)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| "projection turn anchor is truncated".to_owned())?;
    Ok(ContentDigest::from_bytes(slice))
}

fn read_lineage_digest(bytes: &[u8], offset: usize) -> Result<LineageDigest, String> {
    let slice: [u8; 32] = bytes
        .get(offset..offset + 32)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| "projection turn anchor is truncated".to_owned())?;
    Ok(LineageDigest::from_bytes(slice))
}

/// Deterministic-identity journal for unit tests that drive the clean
/// coordinator/actor below the sync runtime. Production opens the journal
/// with the activation binding identities; these tests only need a working
/// journal instance in an isolated root.
#[cfg(test)]
pub(crate) fn open_test_projection_turn_journal(
    application_runtime_root: &std::path::Path,
) -> ProjectionTurnJournalState {
    open_projection_turn_journal(
        application_runtime_root,
        WorkspaceId::from_uuid(Uuid::from_u128(0x7e57_0000_0001)),
        crate::oplog::LineageDigest::of(b"test projection turn journal"),
        ProjectionEndpointId::from_uuid(Uuid::from_u128(0x7e57_0000_0002)),
        Uuid::from_u128(0x7e57_0000_0003),
    )
    .expect("test projection turn journal opens")
}

/// One-shot scratch journal in a fresh isolated root, stamped with the
/// engine's real workspace/endpoint/device identity so replay's endpoint
/// binding check accepts its turns. Resume self-heals by appending any
/// un-retained batch turn, so unit tests below the runtime may use a fresh
/// journal per coordinator call.
#[cfg(test)]
pub(crate) fn open_scratch_projection_turn_journal_for(
    engine: &super::hot_engine::ShardedHotEngine,
) -> ProjectionTurnJournalState {
    let binding = engine
        .projection_endpoint_binding()
        .expect("test engine has a projection endpoint binding");
    let root = std::env::temp_dir().join(format!("tine-scratch-turns-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("scratch projection turn root");
    open_projection_turn_journal(
        &root,
        engine.workspace_id(),
        crate::oplog::LineageDigest::of(b"test projection turn journal"),
        binding.endpoint_id(),
        binding.device_id().as_uuid(),
    )
    .expect("scratch projection turn journal opens")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::oplog::{
        projection_turn_attempt_id, projection_turn_recovery_filename,
        projection_turn_staged_filename, projection_turn_withdrawn_filename, BlobDescription,
        FrontierV2, ManagedPath, PageId, ProjectionTurn, SequenceDomain, TurnOrigin, TurnPage,
        TurnPrecondition, TurnTarget, LIVE_PROJECTION_TURN_DERIVATION_SCHEMES,
        PROJECTION_TURN_DERIVATION_SCHEME_V1, PROJECTION_TURN_SCHEMA_VERSION,
    };

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-projection-turn-{label}-{}", Uuid::new_v4()));
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

    fn workspace() -> WorkspaceId {
        WorkspaceId::from_uuid(Uuid::from_u128(0x7017_0001))
    }

    fn lineage() -> LineageDigest {
        LineageDigest::from_bytes([0x5a; 32])
    }

    fn endpoint() -> ProjectionEndpointId {
        ProjectionEndpointId::from_uuid(Uuid::from_u128(0x7017_0002))
    }

    fn device() -> Uuid {
        Uuid::from_u128(0x7017_0003)
    }

    fn turn(sequence: u64, domain: SequenceDomain) -> ProjectionTurn {
        ProjectionTurn {
            schema_version: PROJECTION_TURN_SCHEMA_VERSION,
            derivation_scheme: PROJECTION_TURN_DERIVATION_SCHEME_V1,
            workspace_id: workspace(),
            lineage_digest: lineage(),
            device_id: device(),
            endpoint_id: endpoint(),
            sequence,
            domain,
            origin: TurnOrigin::IngressForeign {
                batch_id: crate::oplog::BatchId::from_uuid(Uuid::from_u128(0x7017_0004)),
                source_endpoint_id: ProjectionEndpointId::from_uuid(Uuid::from_u128(0x7017_0005)),
            },
            pages: vec![TurnPage {
                page_id: PageId::from_uuid(Uuid::from_u128(0x7017_0006)),
                path: ManagedPath::parse("pages/Turn.md").unwrap(),
                precondition: TurnPrecondition::Base {
                    description: BlobDescription::of(b"- base\n"),
                    bytes: None,
                    annotations: Vec::new(),
                },
                target: TurnTarget::Present {
                    description: BlobDescription::of(b"- target\n"),
                    bytes: None,
                    annotations: Vec::new(),
                },
                frontier: FrontierV2::default(),
                claim_evidence: Vec::new(),
            }],
        }
    }

    fn open(root: &Path) -> Result<ProjectionTurnJournalState, ProjectionTurnJournalError> {
        open_projection_turn_journal(root, workspace(), lineage(), endpoint(), device())
    }

    fn journal_directory(root: &Path) -> PathBuf {
        root.join(MANAGED_LOCAL_JOURNAL_DIR)
            .join(projection_turn_journal_workspace_name(
                workspace(),
                lineage(),
            ))
    }

    fn segment_path(root: &Path) -> PathBuf {
        let directory = journal_directory(root);
        let name = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .find(|name| name.ends_with(".journal-v2"))
            .expect("one projection turn segment");
        directory.join(name)
    }

    // ---- record and derivation (§3.2, §3.5) --------------------------------

    #[test]
    fn a_projection_turn_round_trips_through_its_canonical_encoding() {
        let original = turn(0, SequenceDomain::ProjectionTurn);
        let bytes = original.encode().unwrap();
        let decoded = ProjectionTurn::decode(&bytes, device(), 0).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn a_turn_whose_frame_and_payload_sequences_disagree_is_corruption_not_absence() {
        let bytes = turn(3, SequenceDomain::ProjectionTurn).encode().unwrap();
        let error = ProjectionTurn::decode(&bytes, device(), 4).unwrap_err();
        assert!(
            matches!(&error, crate::oplog::ProjectionTurnError::CorruptPayload(detail)
                if detail.contains("sequences differ")),
            "unexpected error: {error}"
        );
        let error = ProjectionTurn::decode(&bytes, Uuid::from_u128(9), 3).unwrap_err();
        assert!(
            matches!(&error, crate::oplog::ProjectionTurnError::CorruptPayload(detail)
                if detail.contains("device")),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_noncanonical_turn_payload_is_refused() {
        let mut bytes = turn(0, SequenceDomain::ProjectionTurn).encode().unwrap();
        bytes.push(0);
        let error = ProjectionTurn::decode(&bytes, device(), 0).unwrap_err();
        assert!(
            matches!(&error, crate::oplog::ProjectionTurnError::CorruptPayload(_)),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_projection_domain_turn_may_not_carry_bytes() {
        let mut carrying = turn(0, SequenceDomain::ProjectionTurn);
        carrying.pages[0].target = TurnTarget::Present {
            description: BlobDescription::of(b"- target\n"),
            bytes: Some(b"- target\n".to_vec()),
            annotations: Vec::new(),
        };
        let error = carrying.encode().unwrap_err();
        assert!(
            matches!(&error, crate::oplog::ProjectionTurnError::CorruptPayload(detail)
                if detail.contains("never bytes")),
            "unexpected error: {error}"
        );
        // The same page in the managed-local domain is the shape the foreground
        // frame already carries today, and is accepted.
        let mut managed = carrying.clone();
        managed.domain = SequenceDomain::ManagedLocal;
        managed.encode().unwrap();
    }

    #[test]
    fn an_unknown_derivation_scheme_is_preserved_never_treated_as_absent() {
        let mut future = turn(0, SequenceDomain::ProjectionTurn);
        future.derivation_scheme = 999;
        // The record still encodes as bytes on disk; only this build refuses to
        // evaluate it, and it says so by name.
        let bytes = postcard::to_allocvec(&future).unwrap();
        let error = ProjectionTurn::decode(&bytes, device(), 0).unwrap_err();
        assert_eq!(
            error,
            crate::oplog::ProjectionTurnError::UnknownDerivationScheme(999)
        );
    }

    /// The derivation scheme is a hash input, so two domains at the same
    /// sequence can never derive the same names.
    #[test]
    fn the_sequence_domain_separates_two_identical_sequences() {
        let managed = turn(7, SequenceDomain::ManagedLocal);
        let projection = turn(7, SequenceDomain::ProjectionTurn);
        assert_ne!(managed.turn_id(), projection.turn_id());
        assert_ne!(managed.attempt_id(0), projection.attempt_id(0));
    }

    /// §3.5 is byte-level normative. This pins the exact bytes so a later
    /// refactor cannot silently re-derive different names for existing records.
    #[test]
    fn the_turn_derivation_matches_its_specified_byte_layout() {
        use sha2::{Digest as _, Sha256};

        let turn = turn(11, SequenceDomain::ProjectionTurn);
        let mut hasher = Sha256::new();
        hasher.update(b"tine/projection-turn/v1\0");
        hasher.update(PROJECTION_TURN_DERIVATION_SCHEME_V1.to_be_bytes());
        hasher.update(workspace().as_uuid().as_bytes());
        hasher.update(lineage().as_bytes());
        hasher.update(device().as_bytes());
        hasher.update(endpoint().as_uuid().as_bytes());
        hasher.update([1_u8]);
        hasher.update(11_u64.to_be_bytes());
        let expected_turn_id: [u8; 32] = hasher.finalize().into();
        assert_eq!(turn.turn_id(), expected_turn_id);

        let page_id = turn.pages[0].page_id;
        let mut hasher = Sha256::new();
        hasher.update(b"tine/projection-attempt/v2\0");
        hasher.update(expected_turn_id);
        hasher.update(0_u32.to_be_bytes());
        hasher.update(page_id.as_uuid().as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let mut expected = [0_u8; 16];
        expected.copy_from_slice(&digest[..16]);
        expected[6] = (expected[6] & 0x0f) | 0x80;
        expected[8] = (expected[8] & 0x3f) | 0x80;
        let attempt_id = turn.attempt_id(0).unwrap();
        assert_eq!(attempt_id, Uuid::from_bytes(expected));
        assert_eq!(attempt_id.get_version_num(), 8);
        assert_eq!(
            attempt_id,
            projection_turn_attempt_id(expected_turn_id, 0, page_id)
        );

        // The three W2 names, replacing today's PID-scoped and random-UUID ones.
        let simple = attempt_id.simple().to_string();
        assert_eq!(
            projection_turn_recovery_filename("Turn.md", attempt_id),
            format!(".Turn.md.{simple}.projection.recovery")
        );
        assert_eq!(
            projection_turn_staged_filename("Turn.md", attempt_id),
            format!(".Turn.md.{simple}.projection.staged")
        );
        assert_eq!(
            projection_turn_withdrawn_filename("Turn.md", attempt_id),
            format!(".Turn.md.{simple}.projection.withdrawn")
        );
    }

    /// The identity is a function of the record alone: the receipt-store
    /// resource id deliberately does not participate (§3.5 difference 1).
    #[test]
    fn the_turn_identity_is_a_function_of_the_record_alone() {
        assert_eq!(
            turn(4, SequenceDomain::ProjectionTurn).turn_id(),
            turn(4, SequenceDomain::ProjectionTurn).turn_id()
        );
        assert_ne!(
            turn(4, SequenceDomain::ProjectionTurn).turn_id(),
            turn(5, SequenceDomain::ProjectionTurn).turn_id()
        );
    }

    /// A scheme implementation is never deleted while any on-disk record may
    /// reference it, so the live set is contract-visible.
    #[test]
    fn the_contract_lists_every_live_derivation_scheme() {
        let contract = include_str!("../../../../docs/storage-sync-contract.md");
        assert!(
            contract.contains("Projection turn derivation schemes"),
            "the storage contract must carry the derivation-scheme section"
        );
        for scheme in LIVE_PROJECTION_TURN_DERIVATION_SCHEMES {
            assert!(
                contract.contains(&format!("`derivation_scheme` = **{scheme}**")),
                "the storage contract must list live projection-turn derivation scheme {scheme}"
            );
        }
        assert!(
            contract.contains(&format!(
                "live projection-turn derivation schemes: **{}**",
                LIVE_PROJECTION_TURN_DERIVATION_SCHEMES.len()
            )),
            "the storage contract must state how many derivation schemes are live"
        );
    }

    // ---- the second segment (§3.6) ----------------------------------------

    #[test]
    fn the_anchor_round_trips_and_refuses_a_foreign_binding() {
        let checkpoint =
            ProjectionTurnCheckpoint::initial(endpoint(), device(), workspace(), lineage());
        let anchor =
            ProjectionTurnGenerationAnchor::new(1, checkpoint, Uuid::from_u128(0x99)).unwrap();
        let bytes = anchor.encode().unwrap();
        assert_eq!(bytes.len(), PROJECTION_TURN_ANCHOR_BYTES);
        let decoded = ProjectionTurnGenerationAnchor::decode(
            &bytes,
            1,
            endpoint(),
            device(),
            workspace(),
            lineage(),
        )
        .unwrap();
        assert_eq!(decoded, anchor);

        let foreign_endpoint = ProjectionEndpointId::from_uuid(Uuid::from_u128(0xdead));
        assert!(ProjectionTurnGenerationAnchor::decode(
            &bytes,
            1,
            foreign_endpoint,
            device(),
            workspace(),
            lineage(),
        )
        .is_err());
        let mut tampered = bytes;
        tampered[SELECTOR_GENERATION_OFFSET + 7] ^= 0x01;
        assert!(ProjectionTurnGenerationAnchor::decode(
            &tampered,
            1,
            endpoint(),
            device(),
            workspace(),
            lineage(),
        )
        .is_err());
    }

    /// (c) §7 amendment (vi): one grammar, keyed by endpoint from store
    /// creation. A device-keyed name is not a turn-journal anchor.
    #[test]
    fn turn_journal_anchors_are_keyed_by_endpoint() {
        let name = projection_turn_anchor_name(endpoint(), 3);
        assert!(name.starts_with(&format!("endpoint-{}", endpoint().as_uuid().simple())));
        assert_eq!(
            parse_projection_turn_anchor_name(&name, endpoint()),
            Some(3)
        );
        let other = ProjectionEndpointId::from_uuid(Uuid::from_u128(0xbeef));
        assert_eq!(parse_projection_turn_anchor_name(&name, other), None);
        assert_eq!(
            parse_projection_turn_anchor_name(
                &format!("device-{}-selector-{:020}.anchor-v2", device().simple(), 3),
                endpoint()
            ),
            None
        );
    }

    #[test]
    fn a_fresh_turn_journal_opens_empty_and_reopens_with_its_own_counter() {
        let root = TestDir::new("fresh");
        let state = open(root.path()).unwrap();
        assert_eq!(state.checkpoint.next_sequence(), 0);
        assert_eq!(state.journal.next_sequence(), 0);
        assert!(state.pending.is_empty());
        assert!(!state.cleanup_pending);
        drop(state);

        let reopened = open(root.path()).unwrap();
        assert_eq!(reopened.selector_generation, 1);
        assert_eq!(reopened.checkpoint.next_sequence(), 0);
        assert!(reopened.undrained_turns().unwrap().is_empty());
    }

    #[test]
    fn an_appended_turn_survives_reopen_and_retires_at_its_checkpoint() {
        let root = TestDir::new("append");
        let mut state = open(root.path()).unwrap();
        let first = turn(0, SequenceDomain::ProjectionTurn);
        let second = turn(1, SequenceDomain::ProjectionTurn);
        state
            .journal
            .append(ProjectionTurnPayloadKind::TurnV1, &first.encode().unwrap())
            .unwrap();
        state
            .journal
            .append(ProjectionTurnPayloadKind::TurnV1, &second.encode().unwrap())
            .unwrap();
        drop(state);

        let state = open(root.path()).unwrap();
        assert_eq!(state.journal.next_sequence(), 2);
        assert_eq!(
            state.undrained_turns().unwrap(),
            vec![first, second.clone()]
        );
        let advanced = state.checkpoint.advanced_to(1);
        persist_projection_turn_checkpoint(&state.directory, &advanced).unwrap();
        drop(state);

        let state = open(root.path()).unwrap();
        assert_eq!(state.checkpoint.next_sequence(), 1);
        assert_eq!(state.undrained_turns().unwrap(), vec![second]);
    }

    #[test]
    fn drained_turn_journal_rotates_and_retires_covered_checkpoint_names() {
        let root = TestDir::new("drained-compaction");
        let mut state = open(root.path()).unwrap();
        for sequence in 0..5 {
            let expected = turn(sequence, SequenceDomain::ProjectionTurn);
            state
                .append(expected.origin.clone(), expected.pages.clone())
                .unwrap();
            assert_eq!(state.checkpoint_front().unwrap(), expected);
        }
        assert_eq!(state.selector_generation, 2);
        assert_eq!(state.journal.selection().base_sequence(), 4);
        assert_eq!(state.journal.next_sequence(), 5);
        assert_eq!(state.checkpoint.next_sequence(), 5);
        assert!(state.pending.is_empty());
        drop(state);

        let names = fs::read_dir(journal_directory(root.path()))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            names.len(),
            4,
            "the live anchor, segment, frontier, and one post-rotation checkpoint are sufficient"
        );
        let reopened = open(root.path()).unwrap();
        assert_eq!(reopened.selector_generation, 2);
        assert_eq!(reopened.journal.selection().base_sequence(), 4);
        assert_eq!(reopened.journal.next_sequence(), 5);
        assert_eq!(reopened.checkpoint.next_sequence(), 5);
        assert!(reopened.pending.is_empty());
    }

    #[test]
    fn turn_journal_rotation_crash_prefixes_select_a_complete_generation() {
        let seed_drained = |root: &Path| {
            let mut state = open(root).unwrap();
            for sequence in 0..3 {
                let expected = turn(sequence, SequenceDomain::ProjectionTurn);
                state
                    .append(expected.origin.clone(), expected.pages.clone())
                    .unwrap();
                assert_eq!(state.checkpoint_front().unwrap(), expected);
            }
            state
        };

        // Before the marker, a prepared successor tuple is not authority.
        let before_marker = TestDir::new("rotation-before-marker");
        let state = seed_drained(before_marker.path());
        let uncommitted = ProjectionTurnGenerationAnchor::new(
            2,
            state.checkpoint.clone(),
            Uuid::from_u128(0x7017_1001),
        )
        .unwrap();
        LocalJournalSegmentV2::<ProjectionTurnPayloadKind>::prepare_single_writer(
            &state.directory,
            uncommitted.selection(),
        )
        .unwrap();
        drop(state);
        let reopened = open(before_marker.path()).unwrap();
        assert_eq!(reopened.selector_generation, 1);
        assert_eq!(reopened.checkpoint.next_sequence(), 3);
        assert!(reopened.pending.is_empty());

        // Once the new anchor is durable, it is authoritative even while the
        // predecessor tuple and covered checkpoints are still present.
        let after_marker = TestDir::new("rotation-after-marker");
        let state = seed_drained(after_marker.path());
        let (successor, _) =
            prepare_projection_turn_journal(&state.directory, state.checkpoint.clone(), 2).unwrap();
        drop(successor);
        drop(state);
        let reopened = open(after_marker.path()).unwrap();
        assert_eq!(reopened.selector_generation, 2);
        assert_eq!(reopened.journal.selection().base_sequence(), 3);
        assert_eq!(reopened.checkpoint.next_sequence(), 3);
        assert!(reopened.pending.is_empty());
        assert_eq!(
            fs::read_dir(journal_directory(after_marker.path()))
                .unwrap()
                .count(),
            3
        );

        // Cleanup is resumable when the obsolete tuple disappeared but its
        // anchor still names the retry root.
        let during_cleanup = TestDir::new("rotation-during-cleanup");
        let state = seed_drained(during_cleanup.path());
        let predecessor_segment = state.journal.selection().segment_name().to_owned();
        let predecessor_frontier = state.journal.selection().frontier_name().to_owned();
        let (successor, _) =
            prepare_projection_turn_journal(&state.directory, state.checkpoint.clone(), 2).unwrap();
        drop(successor);
        drop(state);
        let directory = journal_directory(during_cleanup.path());
        fs::remove_file(directory.join(predecessor_segment)).unwrap();
        fs::remove_file(directory.join(predecessor_frontier)).unwrap();
        let reopened = open(during_cleanup.path()).unwrap();
        assert_eq!(reopened.selector_generation, 2);
        assert_eq!(reopened.checkpoint.next_sequence(), 3);
        assert!(reopened.pending.is_empty());
        assert_eq!(fs::read_dir(directory).unwrap().count(), 3);
    }

    /// §3.6: the two counters never meet. The turn journal's own sequence is
    /// independent of the foreground journal's.
    #[test]
    fn the_turn_journal_directory_is_a_sibling_of_the_foreground_journal() {
        let root = TestDir::new("sibling");
        let state = open(root.path()).unwrap();
        drop(state);
        let namespace = root.path().join(MANAGED_LOCAL_JOURNAL_DIR);
        assert!(namespace.is_dir());
        let workspace_directory = journal_directory(root.path());
        assert!(workspace_directory.is_dir());
        assert!(workspace_directory
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(PROJECTION_TURN_JOURNAL_WORKSPACE_PREFIX));
    }

    // ---- §3.4 the WAL rule: torn versus corrupt ---------------------------
    //
    // NECESSITY GATE (design §8.3): this pair must FAIL against a decoder that
    // treats the two cases alike. Truncating both loses an authoritative record
    // whose graph effects may exist; refusing both wedges activation on a state
    // that provably never mutated the graph.

    #[test]
    fn bytes_beyond_the_turn_journal_frontier_are_truncated_and_owe_nothing() {
        let root = TestDir::new("torn-tail");
        let mut state = open(root.path()).unwrap();
        let durable = turn(0, SequenceDomain::ProjectionTurn);
        state
            .journal
            .append(
                ProjectionTurnPayloadKind::TurnV1,
                &durable.encode().unwrap(),
            )
            .unwrap();
        drop(state);

        // A partial append that never returned: bytes past the durable
        // frontier. By turn-before-mutation, no graph mutation for them began.
        let segment = segment_path(root.path());
        let committed = fs::metadata(&segment).unwrap().len();
        let mut bytes = fs::read(&segment).unwrap();
        bytes.extend_from_slice(&[0x5a; 512]);
        fs::write(&segment, &bytes).unwrap();

        let state = open(root.path()).expect("a torn tail must not block activation");
        assert_eq!(state.journal.next_sequence(), 1);
        assert_eq!(state.undrained_turns().unwrap(), vec![durable]);
        drop(state);
        assert_eq!(
            fs::metadata(&segment).unwrap().len(),
            committed,
            "the segment must be truncated back to its durable frontier"
        );
    }

    #[test]
    fn a_corrupt_frame_inside_the_turn_journal_frontier_refuses_and_retains_evidence() {
        let root = TestDir::new("corrupt-interior");
        let mut state = open(root.path()).unwrap();
        for sequence in 0..2 {
            state
                .journal
                .append(
                    ProjectionTurnPayloadKind::TurnV1,
                    &turn(sequence, SequenceDomain::ProjectionTurn)
                        .encode()
                        .unwrap(),
                )
                .unwrap();
        }
        drop(state);

        // A disk/media error inside the committed frontier. The record is
        // authoritative and its graph effects may already exist.
        let segment = segment_path(root.path());
        let mut bytes = fs::read(&segment).unwrap();
        let committed = bytes.len();
        let midpoint = committed / 2;
        bytes[midpoint] ^= 0xff;
        fs::write(&segment, &bytes).unwrap();

        let error = open(root.path()).expect_err("a damaged authoritative record must refuse");
        let ProjectionTurnJournalError::Refused(refusal) = &error else {
            panic!("unexpected error: {error}");
        };
        assert!(
            matches!(refusal, LocalJournalOpenRefusal::DiskCorrupt(_)),
            "unexpected refusal: {refusal}"
        );
        assert_eq!(refusal.refusal_code(), Some("MS-REF-DISK-CORRUPT"));
        assert!(refusal.retains_evidence());
        assert_eq!(
            fs::read(&segment).unwrap(),
            bytes,
            "a refused open must never truncate or rewrite the evidence"
        );
    }

    #[test]
    fn an_honest_concurrent_instance_is_not_reported_as_disk_corruption() {
        let root = TestDir::new("concurrent");
        let held = open(root.path()).unwrap();
        let error = open(root.path()).expect_err("a second holder must be refused");
        let ProjectionTurnJournalError::Refused(refusal) = &error else {
            panic!("unexpected error: {error}");
        };
        assert!(
            matches!(refusal, LocalJournalOpenRefusal::ConcurrentInstance(_)),
            "unexpected refusal: {refusal}"
        );
        assert_eq!(refusal.refusal_code(), None);
        assert!(!refusal.retains_evidence());
        drop(held);
    }

    // ---- supplemental receipt/turn state-machine model -------------------
    //
    // This cheap model checks schedule generation and expected state-machine
    // agreement only. The §8.2 license is the real-store production-protocol
    // oracle in `sync_runtime::tests`; a BTreeMap model cannot provide it.

    #[derive(Clone, Debug)]
    struct OracleCase {
        feature: &'static str,
        base: BTreeMap<&'static str, Option<&'static str>>,
        target: BTreeMap<&'static str, Option<&'static str>>,
        valid: bool,
    }

    #[derive(Clone, Copy, Debug)]
    struct OracleSchedule {
        written_prefix: usize,
        completed_receipt_prefix: usize,
        checkpointed: bool,
        external_winner: bool,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum OracleOutcome {
        Converged(BTreeMap<&'static str, Option<&'static str>>),
        GuardedConflict(BTreeMap<&'static str, Option<&'static str>>),
        InvalidSemanticInput(BTreeMap<&'static str, Option<&'static str>>),
    }

    fn map(
        entries: &[(&'static str, Option<&'static str>)],
    ) -> BTreeMap<&'static str, Option<&'static str>> {
        entries.iter().copied().collect()
    }

    fn oracle_corpus() -> Vec<OracleCase> {
        vec![
            OracleCase {
                feature: "create",
                base: map(&[("pages/Create.md", None)]),
                target: map(&[("pages/Create.md", Some("- created\n"))]),
                valid: true,
            },
            OracleCase {
                feature: "edit",
                base: map(&[("pages/Edit.md", Some("- before\n"))]),
                target: map(&[("pages/Edit.md", Some("- after\n"))]),
                valid: true,
            },
            OracleCase {
                feature: "delete",
                base: map(&[("pages/Delete.md", Some("- remove me\n"))]),
                target: map(&[("pages/Delete.md", None)]),
                valid: true,
            },
            OracleCase {
                feature: "cross-page move",
                base: map(&[
                    ("pages/Move Source.md", Some("- movable\n")),
                    ("pages/Move Target.md", Some("- target\n")),
                ]),
                target: map(&[
                    ("pages/Move Source.md", Some("- source\n")),
                    ("pages/Move Target.md", Some("- target\n- movable\n")),
                ]),
                valid: true,
            },
            OracleCase {
                feature: "namespaced path",
                base: map(&[("pages/team___plans.md", Some("- v1\n"))]),
                target: map(&[("pages/team___plans.md", Some("- v2\n"))]),
                valid: true,
            },
            OracleCase {
                feature: "NFC-NFD twin",
                base: map(&[
                    ("pages/Café.md", Some("- composed\n")),
                    ("pages/Cafe\u{301}.md", Some("- decomposed\n")),
                ]),
                target: map(&[
                    ("pages/Café.md", Some("- composed edited\n")),
                    ("pages/Cafe\u{301}.md", Some("- decomposed edited\n")),
                ]),
                valid: true,
            },
            OracleCase {
                feature: "title property page",
                base: map(&[("pages/slug.md", Some("title:: Display\n- old\n"))]),
                target: map(&[("pages/slug.md", Some("title:: Display\n- new\n"))]),
                valid: true,
            },
            OracleCase {
                feature: "missing parent",
                base: map(&[("pages/Missing Parent.md", Some("- child\n"))]),
                target: map(&[("pages/Missing Parent.md", Some("- orphaned child\n"))]),
                valid: false,
            },
        ]
    }

    fn schedule(seed: &mut u64, pages: usize) -> OracleSchedule {
        let mut next = || {
            *seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *seed
        };
        let written_prefix = (next() as usize) % (pages + 1);
        let completed_receipt_prefix = (next() as usize) % (written_prefix + 1);
        let checkpointed = written_prefix == pages && next() & 3 == 0;
        OracleSchedule {
            written_prefix,
            completed_receipt_prefix,
            checkpointed,
            external_winner: !checkpointed && next() % 7 == 0,
        }
    }

    fn crashed_graph(
        case: &OracleCase,
        schedule: OracleSchedule,
    ) -> BTreeMap<&'static str, Option<&'static str>> {
        let mut graph = case.base.clone();
        for (path, target) in case.target.iter().take(schedule.written_prefix) {
            graph.insert(path, *target);
        }
        if schedule.external_winner {
            let path = *case.target.keys().next().unwrap();
            graph.insert(path, Some("- external winner\n"));
        }
        graph
    }

    fn receipt_recovery(case: &OracleCase, schedule: OracleSchedule) -> OracleOutcome {
        let mut graph = crashed_graph(case, schedule);
        if !case.valid {
            return OracleOutcome::InvalidSemanticInput(graph);
        }
        if schedule.checkpointed {
            return OracleOutcome::Converged(graph);
        }
        for (index, (path, target)) in case.target.iter().enumerate() {
            if index < schedule.completed_receipt_prefix && graph.get(path) == Some(target) {
                continue;
            }
            let base = case.base.get(path).copied().flatten();
            let current = graph.get(path).copied().flatten();
            if current != base && current != *target {
                return OracleOutcome::GuardedConflict(graph);
            }
            graph.insert(path, *target);
        }
        OracleOutcome::Converged(graph)
    }

    fn turn_recovery(case: &OracleCase, schedule: OracleSchedule) -> OracleOutcome {
        let mut graph = crashed_graph(case, schedule);
        if !case.valid {
            return OracleOutcome::InvalidSemanticInput(graph);
        }
        if schedule.checkpointed {
            return OracleOutcome::Converged(graph);
        }
        // Turn replay deliberately ignores receipt completion. It walks every
        // page in record order, re-proves already-exact pages, and derives the
        // desired bytes from the current accepted semantic target.
        for (path, target) in &case.target {
            let base = case.base.get(path).copied().flatten();
            let current = graph.get(path).copied().flatten();
            if current != base && current != *target {
                return OracleOutcome::GuardedConflict(graph);
            }
            graph.insert(path, *target);
        }
        OracleOutcome::Converged(graph)
    }

    fn run_equivalence_oracle(schedules_per_case: usize) {
        let mut seed = 0x2a20_2608_26d0_0d5eu64;
        let corpus = oracle_corpus();
        for case in &corpus {
            for ordinal in 0..schedules_per_case {
                let schedule = schedule(&mut seed, case.target.len());
                assert_eq!(
                    receipt_recovery(case, schedule),
                    turn_recovery(case, schedule),
                    "agreement failure for {} schedule {ordinal}: {schedule:?}",
                    case.feature
                );
            }
        }
    }

    #[test]
    fn projection_recovery_state_machine_model_agreement_subset() {
        run_equivalence_oracle(3);
    }

    #[test]
    #[ignore = "full seeded 8-feature × 25-schedule agreement oracle"]
    fn projection_recovery_state_machine_model_agreement_200_cases() {
        run_equivalence_oracle(25);
    }

    // ---- architectural facts ----------------------------------------------

    #[test]
    fn every_projection_only_producer_reaches_the_projection_turn_journal() {
        let runtime = include_str!("../sync_runtime.rs");
        let coordinator = include_str!("operational_coordinator.rs");
        let projection = include_str!("projection.rs");
        for origin in [
            "IngressLocal",
            "IngressForeign",
            "TerminalLocal",
            "TerminalForeign",
            "SupersededRepair",
        ] {
            assert!(
                runtime.contains(origin) || coordinator.contains(origin),
                "projection-only producer {origin} is not wired to production"
            );
        }
        assert!(runtime.contains("open_projection_turn_journal"));
        assert!(runtime.contains("drain_open_managed_local_journal"));
        assert!(runtime.contains("drain_open_projection_turn_journal"));
        assert!(projection.contains("replay_projection_turn"));
        assert!(coordinator.contains("turns.append(origin, pages)"));
    }
}
