#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::authenticated_patricia::{PatriciaIndexRoot, PatriciaIndexStats, PatriciaIndexStore};
use super::object_store::{
    ensure_directory_nofollow, open_dir_nofollow, publish_immutable_exact, read_optional_regular,
    StoreError,
};
use super::scratch_store::{ScratchLsmRoot, ScratchPageKind, ScratchStore};
use super::{
    BatchCausalDot, BatchId, ContentDigest, DocumentCausalDigest, DocumentDependencies, DocumentId,
    FrontierV2, LogicalPageName, PageDelta, PageId, PageNameKeyDigest, PageState,
    PAGE_NAME_KEY_VERSION,
};

pub const EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION: u32 = 1;
pub const EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_STORE_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_CONFLICT_EVIDENCE_SCHEMA_VERSION: u32 = 1;
pub const MAX_PAGE_NAME_POINT_BATCH: usize = 100_000;
pub const MAX_PAGE_NAME_CONFLICT_PARTICIPANTS: usize = 100_000;
pub const MAX_PAGE_NAME_CONFLICT_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EPHEMERAL_PAGE_NAME_RECORDS: usize = 4_096;

const EXACT_NAME_BLOB_SUFFIX: &str = ".exact-page-name";
const MAX_EXACT_NAME_BLOB_BYTES: u64 = 4 * 1024 * 1024 + 1024;
const PAGE_NAME_INDEX_DOMAIN: &[u8] = b"tine/page-name-ownership-index/v1";
const STORE_CLAIM_FILE: &str = "page-name-index.claim";
const NODES_DIR: &str = "nodes";
const EXACT_NAMES_DIR: &str = "exact-names";

/// Opaque bounded page-name view extracted from one authenticated exact
/// catalog checkpoint.
///
/// Callers can request affected PageIds, but cannot supply document identity,
/// causal digests, checkpoint bindings, content digests, or decoded states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedCatalogPageNameCheckpointV1 {
    catalog_document_id: DocumentId,
    catalog_causal_digest: DocumentCausalDigest,
    catalog_checkpoint_binding: ContentDigest,
    catalog_checkpoint_content_digest: ContentDigest,
    entries: BTreeMap<PageId, Option<PageState>>,
}

impl AuthenticatedCatalogPageNameCheckpointV1 {
    pub(crate) const fn catalog_document_id(&self) -> DocumentId {
        self.catalog_document_id
    }

    pub(crate) const fn catalog_causal_digest(&self) -> DocumentCausalDigest {
        self.catalog_causal_digest
    }

    pub(crate) const fn catalog_checkpoint_binding(&self) -> ContentDigest {
        self.catalog_checkpoint_binding
    }

    pub(crate) const fn catalog_checkpoint_content_digest(&self) -> ContentDigest {
        self.catalog_checkpoint_content_digest
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AuthoritativeCatalogPageNameObservationsV1 {
    entries: BTreeMap<PageId, Option<PageState>>,
}

pub(crate) fn extract_authoritative_catalog_page_names(
    catalog_document_id: DocumentId,
    document: &loro::LoroDoc,
    requested_page_ids: &[PageId],
) -> Result<AuthoritativeCatalogPageNameObservationsV1, StoreError> {
    if requested_page_ids.len() > MAX_PAGE_NAME_POINT_BATCH {
        return Err(StoreError::PageNamePointBatchTooLarge {
            actual: requested_page_ids.len(),
            limit: MAX_PAGE_NAME_POINT_BATCH,
        });
    }
    if requested_page_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(StoreError::NonCanonicalPageNamePointKeys);
    }
    let entries = requested_page_ids
        .iter()
        .map(|page_id| {
            super::hot_engine::validate_catalog_page(catalog_document_id, document, *page_id)
                .map(|state| (*page_id, state))
                .map_err(|_| StoreError::MalformedPageNameIndex)
        })
        .collect::<Result<_, _>>()?;
    Ok(AuthoritativeCatalogPageNameObservationsV1 { entries })
}

pub(crate) fn extract_authenticated_catalog_page_names(
    checkpoint: &super::document_state::AuthenticatedExternalExactCheckpoint,
    archive_proof: &super::hot_engine::AuthenticatedCatalogCheckpointArchiveProof,
    expected_catalog_document_id: DocumentId,
    expected_dependencies: Option<&DocumentDependencies>,
    requested_page_ids: &[PageId],
) -> Result<AuthenticatedCatalogPageNameCheckpointV1, StoreError> {
    if requested_page_ids.len() > MAX_PAGE_NAME_POINT_BATCH {
        return Err(StoreError::PageNamePointBatchTooLarge {
            actual: requested_page_ids.len(),
            limit: MAX_PAGE_NAME_POINT_BATCH,
        });
    }
    if requested_page_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(StoreError::NonCanonicalPageNamePointKeys);
    }
    let expected_causal_digest = expected_dependencies
        .map(DocumentDependencies::causal_state_digest)
        .unwrap_or_else(|| DocumentCausalDigest::of(expected_catalog_document_id, &[], &[]));
    if checkpoint.document_id() != expected_catalog_document_id
        || expected_dependencies
            .is_some_and(|dependencies| checkpoint.document_id() != dependencies.document_id())
        || checkpoint.causal_digest() != expected_causal_digest
        || checkpoint.peer_counters()
            != expected_dependencies
                .map(DocumentDependencies::peer_counters)
                .unwrap_or_default()
        || checkpoint.exact_direct_heads()
            != expected_dependencies
                .map(DocumentDependencies::direct_dependency_heads)
                .unwrap_or_default()
        || archive_proof.catalog_document_id() != checkpoint.document_id()
        || archive_proof.catalog_causal_digest() != checkpoint.causal_digest()
        || archive_proof.checkpoint_binding() != checkpoint.checkpoint_binding()
        || archive_proof.checkpoint_content_digest() != checkpoint.checkpoint_content_digest()
    {
        return Err(StoreError::MisboundPageNameCatalogFrontier);
    }
    let mut entries = BTreeMap::new();
    for page_id in requested_page_ids {
        let state = super::hot_engine::validate_catalog_page(
            expected_catalog_document_id,
            checkpoint.document(),
            *page_id,
        )
        .map_err(|_| StoreError::MalformedPageNameIndex)?;
        if entries.insert(*page_id, state).is_some() {
            return Err(StoreError::MalformedPageNameIndex);
        }
    }
    let entry_bytes = encode_canonical(&entries)?;
    let catalog_checkpoint_binding = ContentDigest::of(
        &[
            b"tine/authenticated-catalog-page-names/v1\0".as_slice(),
            checkpoint.checkpoint_binding().as_bytes(),
            checkpoint.checkpoint_content_digest().as_bytes(),
            ContentDigest::of(&entry_bytes).as_bytes(),
        ]
        .concat(),
    );
    Ok(AuthenticatedCatalogPageNameCheckpointV1 {
        catalog_document_id: expected_catalog_document_id,
        catalog_causal_digest: expected_causal_digest,
        catalog_checkpoint_binding,
        catalog_checkpoint_content_digest: checkpoint.checkpoint_content_digest(),
        entries,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageNameCollisionClassV1 {
    DifferentPagesSameCanonicalKey,
    DivergentCanonicalRename,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameReleaseFenceV1 {
    release_batch: BatchId,
    release_dot: BatchCausalDot,
}

impl PageNameReleaseFenceV1 {
    pub const fn release_batch(&self) -> BatchId {
        self.release_batch
    }

    pub const fn release_dot(&self) -> BatchCausalDot {
        self.release_dot
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameConflictParticipantV1 {
    page_id: PageId,
    exact_name: LogicalPageName,
    canonical_key: PageNameKeyDigest,
    acquisition_batch: BatchId,
    acquisition_dot: BatchCausalDot,
    exact_state_batch: BatchId,
    exact_state_dot: BatchCausalDot,
    release_fence: Option<PageNameReleaseFenceV1>,
    declared_frontier: FrontierV2,
}

impl PageNameConflictParticipantV1 {
    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub const fn exact_name(&self) -> &LogicalPageName {
        &self.exact_name
    }

    pub const fn canonical_key(&self) -> PageNameKeyDigest {
        self.canonical_key
    }

    pub const fn acquisition_batch(&self) -> BatchId {
        self.acquisition_batch
    }

    pub const fn acquisition_dot(&self) -> BatchCausalDot {
        self.acquisition_dot
    }

    pub const fn exact_state_batch(&self) -> BatchId {
        self.exact_state_batch
    }

    pub const fn exact_state_dot(&self) -> BatchCausalDot {
        self.exact_state_dot
    }

    pub const fn release_fence(&self) -> Option<&PageNameReleaseFenceV1> {
        self.release_fence.as_ref()
    }

    pub const fn declared_frontier(&self) -> &FrontierV2 {
        &self.declared_frontier
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameConflictEvidenceV1 {
    schema_version: u32,
    key_version: u32,
    collision_class: PageNameCollisionClassV1,
    canonical_keys: Vec<PageNameKeyDigest>,
    participants: Vec<PageNameConflictParticipantV1>,
}

impl PageNameConflictEvidenceV1 {
    fn new(
        collision_class: PageNameCollisionClassV1,
        mut participants: Vec<PageNameConflictParticipantV1>,
    ) -> Result<Self, StoreError> {
        if !(2..=MAX_PAGE_NAME_CONFLICT_PARTICIPANTS).contains(&participants.len()) {
            return Err(StoreError::MalformedPageNameIndex);
        }
        let mut ordered = participants
            .drain(..)
            .map(|participant| Ok((encode_canonical(&participant)?, participant)))
            .collect::<Result<Vec<_>, StoreError>>()?;
        ordered.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        ordered.dedup_by(|left, right| left.0 == right.0);
        let participants = ordered
            .into_iter()
            .map(|(_, participant)| participant)
            .collect::<Vec<_>>();
        let mut canonical_keys = participants
            .iter()
            .map(PageNameConflictParticipantV1::canonical_key)
            .collect::<Vec<_>>();
        canonical_keys.sort_unstable();
        canonical_keys.dedup();
        let evidence = Self {
            schema_version: PAGE_NAME_CONFLICT_EVIDENCE_SCHEMA_VERSION,
            key_version: PAGE_NAME_KEY_VERSION,
            collision_class,
            canonical_keys,
            participants,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    pub const fn collision_class(&self) -> PageNameCollisionClassV1 {
        self.collision_class
    }

    pub fn canonical_keys(&self) -> &[PageNameKeyDigest] {
        &self.canonical_keys
    }

    pub fn participants(&self) -> &[PageNameConflictParticipantV1] {
        &self.participants
    }

    pub fn encode(&self) -> Result<Vec<u8>, StoreError> {
        self.validate()?;
        let bytes = encode_canonical(self)?;
        if bytes.len() > MAX_PAGE_NAME_CONFLICT_EVIDENCE_BYTES {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() > MAX_PAGE_NAME_CONFLICT_EVIDENCE_BYTES {
            return Err(StoreError::MalformedPageNameIndex);
        }
        let evidence: Self = decode_canonical(bytes)?;
        evidence.validate()?;
        Ok(evidence)
    }

    pub fn digest(&self) -> Result<ContentDigest, StoreError> {
        Ok(ContentDigest::of(&self.encode()?))
    }

    fn validate(&self) -> Result<(), StoreError> {
        require_version(
            "page-name conflict evidence",
            self.schema_version,
            PAGE_NAME_CONFLICT_EVIDENCE_SCHEMA_VERSION,
        )?;
        require_version("page-name key", self.key_version, PAGE_NAME_KEY_VERSION)?;
        if !(2..=MAX_PAGE_NAME_CONFLICT_PARTICIPANTS).contains(&self.participants.len())
            || self.canonical_keys.is_empty()
            || self.canonical_keys.len() > self.participants.len()
            || self
                .canonical_keys
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(StoreError::MalformedPageNameIndex);
        }
        let participant_bytes = self
            .participants
            .iter()
            .map(encode_canonical)
            .collect::<Result<Vec<_>, StoreError>>()?;
        if participant_bytes.windows(2).any(|pair| pair[0] >= pair[1])
            || self.participants.iter().any(|participant| {
                participant.exact_name.key_digest() != participant.canonical_key
                    || self
                        .canonical_keys
                        .binary_search(&participant.canonical_key)
                        .is_err()
            })
        {
            return Err(StoreError::MalformedPageNameIndex);
        }
        match self.collision_class {
            PageNameCollisionClassV1::DifferentPagesSameCanonicalKey => {
                if self.canonical_keys.len() != 1
                    || self
                        .participants
                        .iter()
                        .map(PageNameConflictParticipantV1::page_id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        < 2
                {
                    return Err(StoreError::MalformedPageNameIndex);
                }
            }
            PageNameCollisionClassV1::DivergentCanonicalRename => {
                if self.canonical_keys.len() < 2
                    || self
                        .participants
                        .iter()
                        .map(PageNameConflictParticipantV1::page_id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        != 1
                {
                    return Err(StoreError::MalformedPageNameIndex);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum PageNameTransitionError {
    Store(StoreError),
    MalformedBatch(&'static str),
}

impl From<StoreError> for PageNameTransitionError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct EphemeralPageNameOwnershipStateV1 {
    records: BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>,
    exact_names: BTreeMap<(PageNameKeyDigest, ExactLogicalPageNameRefV1), LogicalPageName>,
}

#[derive(Debug)]
struct EphemeralPageNameOwnershipCandidateV1 {
    records: BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>,
    exact_names: BTreeMap<(PageNameKeyDigest, ExactLogicalPageNameRefV1), LogicalPageName>,
}

pub(crate) struct PageNamePublicationCandidateV1 {
    pub(crate) root: PageNameOwnershipRootV1,
    pub(crate) conflicts: Vec<PageNameConflictEvidenceV1>,
    ephemeral: Option<EphemeralPageNameOwnershipCandidateV1>,
}

impl EphemeralPageNameOwnershipStateV1 {
    pub(crate) fn commit(&mut self, candidate: PageNamePublicationCandidateV1) {
        let Some(candidate) = candidate.ephemeral else {
            return;
        };
        debug_assert!(candidate.records.len() <= MAX_PAGE_NAME_POINT_BATCH);
        for (key, record) in candidate.records {
            if let Some(prior) = self.records.insert(key, record) {
                if let Some(occupied) = prior.occupied {
                    self.exact_names.remove(&(key, occupied.exact_name));
                }
                if let Some(released) = prior.latest_release {
                    self.exact_names.remove(&(key, released.prior_exact_name));
                }
            }
        }
        self.exact_names.extend(candidate.exact_names);
    }

    #[cfg(test)]
    pub(crate) fn record_count(&self) -> usize {
        self.records.len()
    }
}

trait PageNameTransitionAccess {
    fn lookup_many(
        &self,
        keys: &[PageNameKeyDigest],
    ) -> Result<BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>, StoreError>;

    fn read_exact_name(
        &self,
        expected_key: PageNameKeyDigest,
        name_ref: &ExactLogicalPageNameRefV1,
    ) -> Result<LogicalPageName, StoreError>;

    fn put_exact_name(
        &self,
        name: &LogicalPageName,
    ) -> Result<ExactLogicalPageNameRefV1, StoreError>;
}

struct PersistentPageNameTransitionAccess<'a> {
    store: &'a PageNameOwnershipStore,
    root: &'a PageNameOwnershipRootV1,
}

impl PageNameTransitionAccess for PersistentPageNameTransitionAccess<'_> {
    fn lookup_many(
        &self,
        keys: &[PageNameKeyDigest],
    ) -> Result<BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>, StoreError> {
        self.store.lookup_many(self.root, keys)
    }

    fn read_exact_name(
        &self,
        expected_key: PageNameKeyDigest,
        name_ref: &ExactLogicalPageNameRefV1,
    ) -> Result<LogicalPageName, StoreError> {
        self.store.read_exact_name(expected_key, name_ref)
    }

    fn put_exact_name(
        &self,
        name: &LogicalPageName,
    ) -> Result<ExactLogicalPageNameRefV1, StoreError> {
        self.store.put_exact_name(name)
    }
}

struct EphemeralPageNameTransitionAccess<'a> {
    state: &'a EphemeralPageNameOwnershipStateV1,
    staged_exact_names: std::cell::RefCell<
        BTreeMap<(PageNameKeyDigest, ExactLogicalPageNameRefV1), LogicalPageName>,
    >,
}

impl PageNameTransitionAccess for EphemeralPageNameTransitionAccess<'_> {
    fn lookup_many(
        &self,
        keys: &[PageNameKeyDigest],
    ) -> Result<BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>, StoreError> {
        if keys.len() > MAX_PAGE_NAME_POINT_BATCH {
            return Err(StoreError::PageNamePointBatchTooLarge {
                actual: keys.len(),
                limit: MAX_PAGE_NAME_POINT_BATCH,
            });
        }
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(StoreError::NonCanonicalPageNamePointKeys);
        }
        Ok(keys
            .iter()
            .filter_map(|key| {
                self.state
                    .records
                    .get(key)
                    .cloned()
                    .map(|record| (*key, record))
            })
            .collect())
    }

    fn read_exact_name(
        &self,
        expected_key: PageNameKeyDigest,
        name_ref: &ExactLogicalPageNameRefV1,
    ) -> Result<LogicalPageName, StoreError> {
        let lookup_key = (expected_key, name_ref.clone());
        let name = self
            .staged_exact_names
            .borrow()
            .get(&lookup_key)
            .cloned()
            .or_else(|| self.state.exact_names.get(&lookup_key).cloned())
            .ok_or(StoreError::MissingExactLogicalPageNameBlob(
                name_ref.content_digest,
            ))?;
        validate_exact_name_ref(expected_key, name_ref, &name)?;
        Ok(name)
    }

    fn put_exact_name(
        &self,
        name: &LogicalPageName,
    ) -> Result<ExactLogicalPageNameRefV1, StoreError> {
        let (_, name_ref) = encode_exact_name_blob(name)?;
        self.staged_exact_names
            .borrow_mut()
            .insert((name.key_digest(), name_ref.clone()), name.clone());
        Ok(name_ref)
    }
}

struct PageNameTransitionCoreCandidateV1 {
    changed: BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>,
    conflicts: Vec<PageNameConflictEvidenceV1>,
}

#[allow(clippy::too_many_arguments)]
fn prepare_page_name_transition_core(
    access: &impl PageNameTransitionAccess,
    batch_id: BatchId,
    causal_dot: BatchCausalDot,
    declared_frontier: &FrontierV2,
    exact_before_pages: &BTreeMap<PageId, Option<PageState>>,
    deltas: &[PageDelta],
    current_pages: &BTreeMap<PageId, Option<PageState>>,
    prospective_pages: &BTreeMap<PageId, Option<PageState>>,
    contains: impl Fn(BatchCausalDot, BatchId) -> bool,
    frontier_for_batch: impl Fn(BatchId) -> Option<FrontierV2>,
) -> Result<PageNameTransitionCoreCandidateV1, PageNameTransitionError> {
    if deltas.len() > MAX_PAGE_NAME_POINT_BATCH {
        return Err(StoreError::PageNamePointBatchTooLarge {
            actual: deltas.len(),
            limit: MAX_PAGE_NAME_POINT_BATCH,
        }
        .into());
    }
    let affected = deltas
        .iter()
        .map(|delta| delta.page_id)
        .collect::<BTreeSet<_>>();
    if affected.len() != deltas.len()
        || affected.iter().any(|page_id| {
            !exact_before_pages.contains_key(page_id)
                || !current_pages.contains_key(page_id)
                || !prospective_pages.contains_key(page_id)
        })
    {
        return Err(PageNameTransitionError::MalformedBatch(
            "page-name transition observations are incomplete or non-unique",
        ));
    }
    for delta in deltas {
        if exact_before_pages[&delta.page_id].as_ref() != delta.before.as_ref() {
            return Err(PageNameTransitionError::MalformedBatch(
                "page-name transition disagrees with the authenticated dependency catalog",
            ));
        }
    }

    let mut keys = BTreeSet::new();
    for delta in deltas {
        for state in [
            delta.before.as_ref(),
            delta.after.as_ref(),
            current_pages[&delta.page_id].as_ref(),
            prospective_pages[&delta.page_id].as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let key = state.name().key_digest();
            if keys.len() == MAX_PAGE_NAME_POINT_BATCH && !keys.contains(&key) {
                return Err(StoreError::PageNamePointBatchTooLarge {
                    actual: MAX_PAGE_NAME_POINT_BATCH + 1,
                    limit: MAX_PAGE_NAME_POINT_BATCH,
                }
                .into());
            }
            keys.insert(key);
        }
    }
    let requested = keys.into_iter().collect::<Vec<_>>();
    let mut records = access.lookup_many(&requested)?;

    let participant_for_occupied = |key: PageNameKeyDigest,
                                    occupied: &PageNameOwnershipOccupiedV1|
     -> Result<PageNameConflictParticipantV1, StoreError> {
        Ok(PageNameConflictParticipantV1 {
            page_id: occupied.page_id,
            exact_name: access.read_exact_name(key, &occupied.exact_name)?,
            canonical_key: key,
            acquisition_batch: occupied.acquisition_batch,
            acquisition_dot: occupied.acquisition_dot,
            exact_state_batch: occupied.exact_state_batch,
            exact_state_dot: occupied.exact_state_dot,
            release_fence: None,
            declared_frontier: frontier_for_batch(occupied.acquisition_batch)
                .ok_or(StoreError::MalformedPageNameIndex)?,
        })
    };
    let participant_for_release = |key: PageNameKeyDigest,
                                   released: &PageNameOwnershipReleasedV1|
     -> Result<PageNameConflictParticipantV1, StoreError> {
        Ok(PageNameConflictParticipantV1 {
            page_id: released.prior_page_id,
            exact_name: access.read_exact_name(key, &released.prior_exact_name)?,
            canonical_key: key,
            acquisition_batch: released.prior_acquisition_batch,
            acquisition_dot: released.prior_acquisition_dot,
            exact_state_batch: released.prior_exact_state_batch,
            exact_state_dot: released.prior_exact_state_dot,
            release_fence: Some(PageNameReleaseFenceV1 {
                release_batch: released.release_batch,
                release_dot: released.release_dot,
            }),
            declared_frontier: frontier_for_batch(released.prior_acquisition_batch)
                .ok_or(StoreError::MalformedPageNameIndex)?,
        })
    };
    let proposed_participant =
        |page_id: PageId, name: LogicalPageName| -> PageNameConflictParticipantV1 {
            PageNameConflictParticipantV1 {
                page_id,
                canonical_key: name.key_digest(),
                exact_name: name,
                acquisition_batch: batch_id,
                acquisition_dot: causal_dot,
                exact_state_batch: batch_id,
                exact_state_dot: causal_dot,
                release_fence: None,
                declared_frontier: declared_frontier.clone(),
            }
        };

    let mut conflicts = Vec::new();
    for delta in deltas {
        let (Some(before_name), Some(proposed_name), Some(current_name)) = (
            delta.before.as_ref().and_then(PageState::live_name),
            delta.after.as_ref().and_then(PageState::live_name),
            current_pages[&delta.page_id]
                .as_ref()
                .and_then(PageState::live_name),
        ) else {
            continue;
        };
        let before_key = before_name.key_digest();
        let proposed_key = proposed_name.key_digest();
        let current_key = current_name.key_digest();
        if proposed_key == before_key || current_key == before_key || current_key == proposed_key {
            continue;
        }
        let existing = records
            .get(&current_key)
            .and_then(PageNameOwnershipRecordV1::occupied)
            .filter(|occupied| occupied.page_id == delta.page_id)
            .ok_or(StoreError::MalformedPageNameIndex)?;
        if !contains(existing.acquisition_dot, existing.acquisition_batch) {
            conflicts.push(PageNameConflictEvidenceV1::new(
                PageNameCollisionClassV1::DivergentCanonicalRename,
                vec![
                    participant_for_occupied(current_key, existing)?,
                    proposed_participant(delta.page_id, proposed_name.clone()),
                ],
            )?);
        }
    }
    if !conflicts.is_empty() {
        conflicts.sort_unstable_by(|left, right| {
            left.encode()
                .expect("constructed page-name evidence remains canonical")
                .cmp(
                    &right
                        .encode()
                        .expect("constructed page-name evidence remains canonical"),
                )
        });
        conflicts.dedup();
        return Ok(PageNameTransitionCoreCandidateV1 {
            changed: BTreeMap::new(),
            conflicts,
        });
    }

    let mut changed = BTreeMap::new();
    for key in &requested {
        let Some(record) = records.get_mut(key) else {
            continue;
        };
        let Some(occupied) = record.occupied().cloned() else {
            continue;
        };
        if !affected.contains(&occupied.page_id) {
            continue;
        }
        let desired = prospective_pages[&occupied.page_id]
            .as_ref()
            .and_then(PageState::live_name);
        if desired.is_some_and(|name| name.key_digest() == *key) {
            continue;
        }
        let latest_release = PageNameOwnershipReleasedV1::new(
            occupied.page_id,
            occupied.exact_name,
            occupied.acquisition_batch,
            occupied.acquisition_dot,
            occupied.exact_state_batch,
            occupied.exact_state_dot,
            batch_id,
            causal_dot,
        );
        let replacement = PageNameOwnershipRecordV1::new(*key, None, Some(latest_release))?;
        *record = replacement.clone();
        changed.insert(*key, replacement);
    }

    let mut acquisitions = deltas
        .iter()
        .filter_map(|delta| {
            prospective_pages[&delta.page_id]
                .as_ref()
                .and_then(PageState::live_name)
                .map(|name| (name.key_digest(), delta.page_id, name.clone(), delta))
        })
        .collect::<Vec<_>>();
    acquisitions.sort_unstable_by(|left, right| {
        (left.0, left.1, left.2.as_str()).cmp(&(right.0, right.1, right.2.as_str()))
    });
    for pair in acquisitions.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1 {
            return Err(PageNameTransitionError::MalformedBatch(
                "two PageIds acquire one canonical page-name key in the same batch",
            ));
        }
    }

    for (key, page_id, exact_name, delta) in acquisitions {
        if let Some(existing) = records
            .get(&key)
            .and_then(PageNameOwnershipRecordV1::occupied)
        {
            if existing.page_id == page_id {
                let existing_name = access.read_exact_name(key, &existing.exact_name)?;
                if existing_name == exact_name {
                    continue;
                }
                let proposed_wins = delta
                    .after
                    .as_ref()
                    .and_then(PageState::live_name)
                    .is_some_and(|name| name == &exact_name);
                let current_wins = current_pages[&page_id]
                    .as_ref()
                    .and_then(PageState::live_name)
                    .is_some_and(|name| name == &exact_name);
                let (state_batch, state_dot) = if proposed_wins {
                    (batch_id, causal_dot)
                } else if current_wins {
                    (existing.exact_state_batch, existing.exact_state_dot)
                } else {
                    return Err(StoreError::MalformedPageNameIndex.into());
                };
                let replacement = PageNameOwnershipRecordV1::new(
                    key,
                    Some(PageNameOwnershipOccupiedV1::new(
                        page_id,
                        access.put_exact_name(&exact_name)?,
                        existing.acquisition_batch,
                        existing.acquisition_dot,
                        state_batch,
                        state_dot,
                    )),
                    records
                        .get(&key)
                        .and_then(PageNameOwnershipRecordV1::latest_release)
                        .cloned(),
                )?;
                records.insert(key, replacement.clone());
                changed.insert(key, replacement);
                continue;
            }
            if contains(existing.acquisition_dot, existing.acquisition_batch) {
                return Err(PageNameTransitionError::MalformedBatch(
                    "canonical page-name key is occupied at the declared dependency frontier",
                ));
            }
            conflicts.push(PageNameConflictEvidenceV1::new(
                PageNameCollisionClassV1::DifferentPagesSameCanonicalKey,
                vec![
                    participant_for_occupied(key, existing)?,
                    proposed_participant(page_id, exact_name),
                ],
            )?);
            continue;
        }
        if let Some(released) = records
            .get(&key)
            .and_then(PageNameOwnershipRecordV1::latest_release)
            .filter(|release| {
                release.release_batch != batch_id
                    && !contains(release.release_dot, release.release_batch)
            })
        {
            conflicts.push(PageNameConflictEvidenceV1::new(
                PageNameCollisionClassV1::DifferentPagesSameCanonicalKey,
                vec![
                    participant_for_release(key, released)?,
                    proposed_participant(page_id, exact_name),
                ],
            )?);
            continue;
        }
        let latest_release = records
            .get(&key)
            .and_then(PageNameOwnershipRecordV1::latest_release)
            .cloned();
        let replacement = PageNameOwnershipRecordV1::new(
            key,
            Some(PageNameOwnershipOccupiedV1::new(
                page_id,
                access.put_exact_name(&exact_name)?,
                batch_id,
                causal_dot,
                batch_id,
                causal_dot,
            )),
            latest_release,
        )?;
        records.insert(key, replacement.clone());
        changed.insert(key, replacement);
    }

    if !conflicts.is_empty() {
        conflicts.sort_unstable_by(|left, right| {
            left.encode()
                .expect("constructed page-name evidence remains canonical")
                .cmp(
                    &right
                        .encode()
                        .expect("constructed page-name evidence remains canonical"),
                )
        });
        conflicts.dedup();
        return Ok(PageNameTransitionCoreCandidateV1 {
            changed: BTreeMap::new(),
            conflicts,
        });
    }
    Ok(PageNameTransitionCoreCandidateV1 { changed, conflicts })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_page_name_transition(
    store: &PageNameOwnershipStore,
    root: &PageNameOwnershipRootV1,
    batch_id: BatchId,
    causal_dot: BatchCausalDot,
    declared_frontier: &FrontierV2,
    exact_checkpoint: &AuthenticatedCatalogPageNameCheckpointV1,
    deltas: &[PageDelta],
    current_pages: &BTreeMap<PageId, Option<PageState>>,
    prospective_pages: &BTreeMap<PageId, Option<PageState>>,
    contains: impl Fn(BatchCausalDot, BatchId) -> bool,
    frontier_for_batch: impl Fn(BatchId) -> Option<FrontierV2>,
) -> Result<PageNamePublicationCandidateV1, PageNameTransitionError> {
    let access = PersistentPageNameTransitionAccess { store, root };
    let candidate = prepare_page_name_transition_core(
        &access,
        batch_id,
        causal_dot,
        declared_frontier,
        &exact_checkpoint.entries,
        deltas,
        current_pages,
        prospective_pages,
        contains,
        frontier_for_batch,
    )?;
    let next_root = if candidate.conflicts.is_empty() {
        store.insert_many(root, &candidate.changed)?
    } else {
        root.clone()
    };
    Ok(PageNamePublicationCandidateV1 {
        root: next_root,
        conflicts: candidate.conflicts,
        ephemeral: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_ephemeral_page_name_transition(
    state: &EphemeralPageNameOwnershipStateV1,
    batch_id: BatchId,
    causal_dot: BatchCausalDot,
    declared_frontier: &FrontierV2,
    exact_before: &AuthoritativeCatalogPageNameObservationsV1,
    deltas: &[PageDelta],
    current_pages: &BTreeMap<PageId, Option<PageState>>,
    prospective_pages: &BTreeMap<PageId, Option<PageState>>,
    contains: impl Fn(BatchCausalDot, BatchId) -> bool,
    frontier_for_batch: impl Fn(BatchId) -> Option<FrontierV2>,
) -> Result<PageNamePublicationCandidateV1, PageNameTransitionError> {
    let access = EphemeralPageNameTransitionAccess {
        state,
        staged_exact_names: std::cell::RefCell::new(BTreeMap::new()),
    };
    let candidate = prepare_page_name_transition_core(
        &access,
        batch_id,
        causal_dot,
        declared_frontier,
        &exact_before.entries,
        deltas,
        current_pages,
        prospective_pages,
        contains,
        frontier_for_batch,
    )?;
    let additions = candidate
        .changed
        .keys()
        .filter(|key| !state.records.contains_key(key))
        .count();
    if state.records.len().saturating_add(additions) > MAX_EPHEMERAL_PAGE_NAME_RECORDS {
        return Err(PageNameTransitionError::MalformedBatch(
            "no-store page-name test index reached its fixed capacity",
        ));
    }
    let ephemeral = if candidate.conflicts.is_empty() {
        let staged = access.staged_exact_names.into_inner();
        let required_exact_names = candidate
            .changed
            .iter()
            .flat_map(|(key, record)| {
                record
                    .occupied()
                    .map(|occupied| (*key, occupied.exact_name().clone()))
                    .into_iter()
                    .chain(
                        record
                            .latest_release()
                            .map(|released| (*key, released.prior_exact_name().clone())),
                    )
            })
            .collect::<BTreeSet<_>>();
        let exact_names = required_exact_names
            .into_iter()
            .map(|lookup_key| {
                let name = staged
                    .get(&lookup_key)
                    .cloned()
                    .or_else(|| state.exact_names.get(&lookup_key).cloned())
                    .ok_or(StoreError::MissingExactLogicalPageNameBlob(
                        lookup_key.1.content_digest,
                    ))?;
                Ok((lookup_key, name))
            })
            .collect::<Result<BTreeMap<_, _>, StoreError>>()?;
        Some(EphemeralPageNameOwnershipCandidateV1 {
            records: candidate.changed,
            exact_names,
        })
    } else {
        None
    };
    Ok(PageNamePublicationCandidateV1 {
        root: PageNameOwnershipRootV1::empty(),
        conflicts: candidate.conflicts,
        ephemeral,
    })
}

/// Digest of an exact, pre-canonicalization logical page name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExactLogicalPageNameDigest([u8; 32]);

impl ExactLogicalPageNameDigest {
    pub fn of(name: &LogicalPageName) -> Self {
        let exact = name.as_str().as_bytes();
        let mut hasher = Sha256::new();
        hasher.update(b"tine/exact-logical-page-name/v1\0");
        hasher.update((exact.len() as u64).to_be_bytes());
        hasher.update(exact);
        Self(hasher.finalize().into())
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ExactLogicalPageNameDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactLogicalPageNameBlobV1 {
    schema_version: u32,
    exact_name: LogicalPageName,
}

impl ExactLogicalPageNameBlobV1 {
    pub const fn exact_name(&self) -> &LogicalPageName {
        &self.exact_name
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactLogicalPageNameRefV1 {
    schema_version: u32,
    encoded_len: u64,
    content_digest: ContentDigest,
    exact_name_digest: ExactLogicalPageNameDigest,
}

impl ExactLogicalPageNameRefV1 {
    pub const fn encoded_len(&self) -> u64 {
        self.encoded_len
    }

    pub const fn content_digest(&self) -> ContentDigest {
        self.content_digest
    }

    pub const fn exact_name_digest(&self) -> ExactLogicalPageNameDigest {
        self.exact_name_digest
    }

    fn validate_version_and_length(&self) -> Result<(), StoreError> {
        require_version(
            "exact logical page-name reference",
            self.schema_version,
            EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION,
        )?;
        if self.encoded_len == 0 || self.encoded_len > MAX_EXACT_NAME_BLOB_BYTES {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(())
    }
}

fn encode_exact_name_blob(
    name: &LogicalPageName,
) -> Result<(Vec<u8>, ExactLogicalPageNameRefV1), StoreError> {
    let blob = ExactLogicalPageNameBlobV1 {
        schema_version: EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION,
        exact_name: name.clone(),
    };
    let bytes = encode_canonical(&blob)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_EXACT_NAME_BLOB_BYTES {
        return Err(StoreError::MalformedPageNameIndex);
    }
    Ok((
        bytes.clone(),
        ExactLogicalPageNameRefV1 {
            schema_version: EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION,
            encoded_len: bytes.len() as u64,
            content_digest: ContentDigest::of(&bytes),
            exact_name_digest: ExactLogicalPageNameDigest::of(name),
        },
    ))
}

fn validate_exact_name_ref(
    expected_key: PageNameKeyDigest,
    name_ref: &ExactLogicalPageNameRefV1,
    name: &LogicalPageName,
) -> Result<(), StoreError> {
    name_ref.validate_version_and_length()?;
    let (bytes, expected_ref) = encode_exact_name_blob(name)?;
    if bytes.len() as u64 != name_ref.encoded_len
        || expected_ref.content_digest != name_ref.content_digest
        || expected_ref.exact_name_digest != name_ref.exact_name_digest
        || name.key_digest() != expected_key
    {
        return Err(StoreError::MalformedPageNameIndex);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameOwnershipOccupiedV1 {
    page_id: PageId,
    exact_name: ExactLogicalPageNameRefV1,
    acquisition_batch: BatchId,
    acquisition_dot: BatchCausalDot,
    exact_state_batch: BatchId,
    exact_state_dot: BatchCausalDot,
}

impl PageNameOwnershipOccupiedV1 {
    pub const fn new(
        page_id: PageId,
        exact_name: ExactLogicalPageNameRefV1,
        acquisition_batch: BatchId,
        acquisition_dot: BatchCausalDot,
        exact_state_batch: BatchId,
        exact_state_dot: BatchCausalDot,
    ) -> Self {
        Self {
            page_id,
            exact_name,
            acquisition_batch,
            acquisition_dot,
            exact_state_batch,
            exact_state_dot,
        }
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub const fn exact_name(&self) -> &ExactLogicalPageNameRefV1 {
        &self.exact_name
    }

    pub const fn acquisition_batch(&self) -> BatchId {
        self.acquisition_batch
    }

    pub const fn acquisition_dot(&self) -> BatchCausalDot {
        self.acquisition_dot
    }

    pub const fn exact_state_batch(&self) -> BatchId {
        self.exact_state_batch
    }

    pub const fn exact_state_dot(&self) -> BatchCausalDot {
        self.exact_state_dot
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameOwnershipReleasedV1 {
    prior_page_id: PageId,
    prior_exact_name: ExactLogicalPageNameRefV1,
    prior_acquisition_batch: BatchId,
    prior_acquisition_dot: BatchCausalDot,
    prior_exact_state_batch: BatchId,
    prior_exact_state_dot: BatchCausalDot,
    release_batch: BatchId,
    release_dot: BatchCausalDot,
}

impl PageNameOwnershipReleasedV1 {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        prior_page_id: PageId,
        prior_exact_name: ExactLogicalPageNameRefV1,
        prior_acquisition_batch: BatchId,
        prior_acquisition_dot: BatchCausalDot,
        prior_exact_state_batch: BatchId,
        prior_exact_state_dot: BatchCausalDot,
        release_batch: BatchId,
        release_dot: BatchCausalDot,
    ) -> Self {
        Self {
            prior_page_id,
            prior_exact_name,
            prior_acquisition_batch,
            prior_acquisition_dot,
            prior_exact_state_batch,
            prior_exact_state_dot,
            release_batch,
            release_dot,
        }
    }

    pub const fn prior_page_id(&self) -> PageId {
        self.prior_page_id
    }

    pub const fn prior_exact_name(&self) -> &ExactLogicalPageNameRefV1 {
        &self.prior_exact_name
    }

    pub const fn prior_acquisition_batch(&self) -> BatchId {
        self.prior_acquisition_batch
    }

    pub const fn prior_acquisition_dot(&self) -> BatchCausalDot {
        self.prior_acquisition_dot
    }

    pub const fn prior_exact_state_batch(&self) -> BatchId {
        self.prior_exact_state_batch
    }

    pub const fn prior_exact_state_dot(&self) -> BatchCausalDot {
        self.prior_exact_state_dot
    }

    pub const fn release_batch(&self) -> BatchId {
        self.release_batch
    }

    pub const fn release_dot(&self) -> BatchCausalDot {
        self.release_dot
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameOwnershipRecordV1 {
    schema_version: u32,
    key_version: u32,
    key_digest: PageNameKeyDigest,
    occupied: Option<PageNameOwnershipOccupiedV1>,
    latest_release: Option<PageNameOwnershipReleasedV1>,
}

impl PageNameOwnershipRecordV1 {
    pub fn new(
        key_digest: PageNameKeyDigest,
        occupied: Option<PageNameOwnershipOccupiedV1>,
        latest_release: Option<PageNameOwnershipReleasedV1>,
    ) -> Result<Self, StoreError> {
        let record = Self {
            schema_version: PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION,
            key_version: PAGE_NAME_KEY_VERSION,
            key_digest,
            occupied,
            latest_release,
        };
        record.validate_shape(key_digest)?;
        Ok(record)
    }

    pub const fn key_digest(&self) -> PageNameKeyDigest {
        self.key_digest
    }

    pub const fn occupied(&self) -> Option<&PageNameOwnershipOccupiedV1> {
        self.occupied.as_ref()
    }

    pub const fn latest_release(&self) -> Option<&PageNameOwnershipReleasedV1> {
        self.latest_release.as_ref()
    }

    fn validate_shape(&self, expected_key: PageNameKeyDigest) -> Result<(), StoreError> {
        require_version(
            "page-name ownership record",
            self.schema_version,
            PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION,
        )?;
        require_version("page-name key", self.key_version, PAGE_NAME_KEY_VERSION)?;
        if self.key_digest != expected_key
            || (self.occupied.is_none() && self.latest_release.is_none())
        {
            return Err(StoreError::MalformedPageNameIndex);
        }
        if let Some(occupied) = &self.occupied {
            occupied.exact_name.validate_version_and_length()?;
        }
        if let Some(released) = &self.latest_release {
            released.prior_exact_name.validate_version_and_length()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageNameOwnershipRootV1 {
    schema_version: u32,
    key_version: u32,
    patricia_root: PatriciaIndexRoot,
    entry_count: u64,
}

impl PageNameOwnershipRootV1 {
    pub fn empty() -> Self {
        Self {
            schema_version: PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION,
            key_version: PAGE_NAME_KEY_VERSION,
            patricia_root: PatriciaIndexRoot::empty(),
            entry_count: 0,
        }
    }

    pub const fn entry_count(&self) -> u64 {
        self.entry_count
    }

    pub const fn patricia_digest(&self) -> ContentDigest {
        self.patricia_root.digest()
    }

    pub fn external_digest(&self) -> Result<ContentDigest, StoreError> {
        self.validate_version_and_shape()?;
        let encoded = encode_canonical(self)?;
        let mut bytes = b"tine/page-name-ownership-root/v1\0".to_vec();
        bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&encoded);
        Ok(ContentDigest::of(&bytes))
    }

    pub fn encode(&self) -> Result<Vec<u8>, StoreError> {
        self.validate_version_and_shape()?;
        encode_canonical(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        let root: Self = decode_canonical(bytes)?;
        root.validate_version_and_shape()?;
        Ok(root)
    }

    fn validate_version_and_shape(&self) -> Result<(), StoreError> {
        require_version(
            "page-name ownership root",
            self.schema_version,
            PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION,
        )?;
        require_version("page-name key", self.key_version, PAGE_NAME_KEY_VERSION)?;
        if (self.entry_count == 0) != (self.patricia_root == PatriciaIndexRoot::empty()) {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(())
    }
}

impl Default for PageNameOwnershipRootV1 {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug)]
pub(crate) struct PageNameOwnershipStore {
    patricia: PatriciaIndexStore,
    exact_names: Dir,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageNameOwnershipStoreClaimV1 {
    schema_version: u32,
    key_version: u32,
}

impl PageNameOwnershipStore {
    pub(crate) fn open(index: Dir) -> Result<Self, StoreError> {
        let expected = PageNameOwnershipStoreClaimV1 {
            schema_version: PAGE_NAME_OWNERSHIP_STORE_SCHEMA_VERSION,
            key_version: PAGE_NAME_KEY_VERSION,
        };
        match read_optional_regular(&index, STORE_CLAIM_FILE, 64, None)? {
            Some(bytes) => {
                let claim: PageNameOwnershipStoreClaimV1 = decode_canonical(&bytes)?;
                require_version(
                    "page-name ownership store",
                    claim.schema_version,
                    PAGE_NAME_OWNERSHIP_STORE_SCHEMA_VERSION,
                )?;
                require_version("page-name key", claim.key_version, PAGE_NAME_KEY_VERSION)?;
            }
            None => {
                if index.entries()?.next().is_some() {
                    return Err(StoreError::MalformedPageNameIndex);
                }
                publish_immutable_exact(
                    &index,
                    STORE_CLAIM_FILE,
                    &encode_canonical(&expected)?,
                    "page-name ownership store claim",
                )?;
            }
        }
        ensure_directory_nofollow(&index, NODES_DIR)?;
        ensure_directory_nofollow(&index, EXACT_NAMES_DIR)?;
        Ok(Self {
            patricia: PatriciaIndexStore::new(open_dir_nofollow(&index, NODES_DIR)?),
            exact_names: open_dir_nofollow(&index, EXACT_NAMES_DIR)?,
        })
    }

    pub(crate) fn stats(&self) -> PatriciaIndexStats {
        self.patricia.stats()
    }

    pub(crate) fn put_exact_name(
        &self,
        name: &LogicalPageName,
    ) -> Result<ExactLogicalPageNameRefV1, StoreError> {
        let (bytes, name_ref) = encode_exact_name_blob(name)?;
        publish_immutable_exact(
            &self.exact_names,
            &exact_name_blob_filename(name_ref.content_digest),
            &bytes,
            "exact logical page-name blob",
        )?;
        Ok(name_ref)
    }

    pub(crate) fn read_exact_name(
        &self,
        expected_key: PageNameKeyDigest,
        name_ref: &ExactLogicalPageNameRefV1,
    ) -> Result<LogicalPageName, StoreError> {
        name_ref.validate_version_and_length()?;
        let filename = exact_name_blob_filename(name_ref.content_digest);
        let bytes = read_optional_regular(
            &self.exact_names,
            &filename,
            MAX_EXACT_NAME_BLOB_BYTES,
            Some(name_ref.encoded_len),
        )?
        .ok_or(StoreError::MissingExactLogicalPageNameBlob(
            name_ref.content_digest,
        ))?;
        if ContentDigest::of(&bytes) != name_ref.content_digest {
            return Err(StoreError::ExactLogicalPageNameBlobPathMismatch(
                name_ref.content_digest,
            ));
        }
        let blob: ExactLogicalPageNameBlobV1 = decode_canonical(&bytes)?;
        require_version(
            "exact logical page-name blob",
            blob.schema_version,
            EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION,
        )?;
        validate_exact_name_ref(expected_key, name_ref, &blob.exact_name)?;
        Ok(blob.exact_name)
    }

    pub(crate) fn validate_root(&self, root: &PageNameOwnershipRootV1) -> Result<(), StoreError> {
        root.validate_version_and_shape()?;
        self.patricia.validate_root(root.patricia_root)
    }

    pub(crate) fn lookup(
        &self,
        root: &PageNameOwnershipRootV1,
        key: PageNameKeyDigest,
    ) -> Result<Option<PageNameOwnershipRecordV1>, StoreError> {
        self.validate_root(root)?;
        self.patricia
            .lookup(root.patricia_root, key.as_bytes())?
            .map(|bytes| {
                let record = decode_record(key, &bytes)?;
                self.validate_record_names(key, &record)?;
                Ok(record)
            })
            .transpose()
    }

    pub(crate) fn lookup_many(
        &self,
        root: &PageNameOwnershipRootV1,
        keys: &[PageNameKeyDigest],
    ) -> Result<BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>, StoreError> {
        if keys.len() > MAX_PAGE_NAME_POINT_BATCH {
            return Err(StoreError::PageNamePointBatchTooLarge {
                actual: keys.len(),
                limit: MAX_PAGE_NAME_POINT_BATCH,
            });
        }
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(StoreError::NonCanonicalPageNamePointKeys);
        }
        self.validate_root(root)?;
        let raw_keys = keys
            .iter()
            .map(|key| key.as_bytes().to_vec())
            .collect::<Vec<_>>();
        let raw = self.patricia.lookup_many(root.patricia_root, &raw_keys)?;
        keys.iter()
            .filter_map(|key| {
                raw.get(key.as_bytes().as_slice()).map(|bytes| {
                    let record = decode_record(*key, bytes)?;
                    self.validate_record_names(*key, &record)?;
                    Ok((*key, record))
                })
            })
            .collect()
    }

    pub(crate) fn insert_many(
        &self,
        root: &PageNameOwnershipRootV1,
        records: &BTreeMap<PageNameKeyDigest, PageNameOwnershipRecordV1>,
    ) -> Result<PageNameOwnershipRootV1, StoreError> {
        if records.len() > MAX_PAGE_NAME_POINT_BATCH {
            return Err(StoreError::PageNamePointBatchTooLarge {
                actual: records.len(),
                limit: MAX_PAGE_NAME_POINT_BATCH,
            });
        }
        self.validate_root(root)?;
        let mut additions = 0_u64;
        let encoded = records
            .iter()
            .map(|(key, record)| {
                record.validate_shape(*key)?;
                self.validate_record_names(*key, record)?;
                if self
                    .patricia
                    .lookup(root.patricia_root, key.as_bytes())?
                    .is_none()
                {
                    additions = additions
                        .checked_add(1)
                        .ok_or(StoreError::MalformedPageNameIndex)?;
                }
                Ok((key.as_bytes().to_vec(), encode_canonical(record)?))
            })
            .collect::<Result<BTreeMap<_, _>, StoreError>>()?;
        let patricia_root = self.patricia.insert_many(root.patricia_root, &encoded)?;
        let entry_count = root
            .entry_count
            .checked_add(additions)
            .ok_or(StoreError::MalformedPageNameIndex)?;
        let next = PageNameOwnershipRootV1 {
            schema_version: PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION,
            key_version: PAGE_NAME_KEY_VERSION,
            patricia_root,
            entry_count,
        };
        next.validate_version_and_shape()?;
        Ok(next)
    }

    fn validate_record_names(
        &self,
        key: PageNameKeyDigest,
        record: &PageNameOwnershipRecordV1,
    ) -> Result<(), StoreError> {
        if let Some(occupied) = &record.occupied {
            self.read_exact_name(key, &occupied.exact_name)?;
        }
        if let Some(released) = &record.latest_release {
            self.read_exact_name(key, &released.prior_exact_name)?;
        }
        Ok(())
    }

    // Deliberately module-private until authenticated exact-catalog decoding
    // produces `ExactCatalogPageNameCheckpointV1`.
    fn reconstruct_from_exact_catalog_checkpoint(
        &self,
        scratch: &ScratchStore,
        frontier_root: &PageNameCatalogFrontierRootV1,
        checkpoint: &ExactCatalogPageNameCheckpointV1,
    ) -> Result<ColdPageNameReconstructionV1, StoreError> {
        checkpoint.validate()?;
        let mut records = BTreeMap::new();
        for (key, snapshot) in &checkpoint.entries {
            let occupied = snapshot
                .occupied
                .as_ref()
                .map(|occupied| {
                    let name = occupied
                        .winning_state
                        .live_name()
                        .ok_or(StoreError::MalformedPageNameIndex)?;
                    Ok::<_, StoreError>(PageNameOwnershipOccupiedV1::new(
                        occupied.page_id,
                        self.put_exact_name(name)?,
                        occupied.acquisition_batch,
                        occupied.acquisition_dot,
                        occupied.exact_state_batch,
                        occupied.exact_state_dot,
                    ))
                })
                .transpose()?;
            let latest_release = snapshot
                .latest_release
                .as_ref()
                .map(|released| {
                    Ok::<_, StoreError>(PageNameOwnershipReleasedV1::new(
                        released.prior_page_id,
                        self.put_exact_name(&released.prior_exact_name)?,
                        released.prior_acquisition_batch,
                        released.prior_acquisition_dot,
                        released.prior_exact_state_batch,
                        released.prior_exact_state_dot,
                        released.release_batch,
                        released.release_dot,
                    ))
                })
                .transpose()?;
            records.insert(
                *key,
                PageNameOwnershipRecordV1::new(*key, occupied, latest_release)?,
            );
        }
        let ownership_root = self.insert_many(&PageNameOwnershipRootV1::empty(), &records)?;
        let frontier_root = publish_catalog_frontier_binding(
            scratch,
            frontier_root,
            PageNameCatalogFrontierBindingV1::new(
                checkpoint.catalog_document_id,
                checkpoint.catalog_causal_digest,
                checkpoint.catalog_checkpoint_binding,
                ownership_root.external_digest()?,
            ),
        )?;
        Ok(ColdPageNameReconstructionV1 {
            ownership_root,
            frontier_root,
        })
    }
}

trait LivePageState {
    fn live_name(&self) -> Option<&LogicalPageName>;
}

impl LivePageState for PageState {
    fn live_name(&self) -> Option<&LogicalPageName> {
        match self {
            Self::Live { name, .. } => Some(name),
            Self::Tombstone { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactCatalogOccupiedSnapshotV1 {
    page_id: PageId,
    winning_state: PageState,
    acquisition_batch: BatchId,
    acquisition_dot: BatchCausalDot,
    exact_state_batch: BatchId,
    exact_state_dot: BatchCausalDot,
}

impl ExactCatalogOccupiedSnapshotV1 {
    fn new(
        page_id: PageId,
        winning_state: PageState,
        acquisition_batch: BatchId,
        acquisition_dot: BatchCausalDot,
        exact_state_batch: BatchId,
        exact_state_dot: BatchCausalDot,
    ) -> Result<Self, StoreError> {
        if winning_state.live_name().is_none() {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(Self {
            page_id,
            winning_state,
            acquisition_batch,
            acquisition_dot,
            exact_state_batch,
            exact_state_dot,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactCatalogReleasedSnapshotV1 {
    prior_page_id: PageId,
    prior_exact_name: LogicalPageName,
    prior_acquisition_batch: BatchId,
    prior_acquisition_dot: BatchCausalDot,
    prior_exact_state_batch: BatchId,
    prior_exact_state_dot: BatchCausalDot,
    release_batch: BatchId,
    release_dot: BatchCausalDot,
}

impl ExactCatalogReleasedSnapshotV1 {
    #[allow(clippy::too_many_arguments)]
    const fn new(
        prior_page_id: PageId,
        prior_exact_name: LogicalPageName,
        prior_acquisition_batch: BatchId,
        prior_acquisition_dot: BatchCausalDot,
        prior_exact_state_batch: BatchId,
        prior_exact_state_dot: BatchCausalDot,
        release_batch: BatchId,
        release_dot: BatchCausalDot,
    ) -> Self {
        Self {
            prior_page_id,
            prior_exact_name,
            prior_acquisition_batch,
            prior_acquisition_dot,
            prior_exact_state_batch,
            prior_exact_state_dot,
            release_batch,
            release_dot,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactCatalogOwnershipSnapshotV1 {
    occupied: Option<ExactCatalogOccupiedSnapshotV1>,
    latest_release: Option<ExactCatalogReleasedSnapshotV1>,
}

impl ExactCatalogOwnershipSnapshotV1 {
    fn new(
        occupied: Option<ExactCatalogOccupiedSnapshotV1>,
        latest_release: Option<ExactCatalogReleasedSnapshotV1>,
    ) -> Result<Self, StoreError> {
        if occupied.is_none() && latest_release.is_none() {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(Self {
            occupied,
            latest_release,
        })
    }

    fn validate(&self, expected_key: PageNameKeyDigest) -> Result<(), StoreError> {
        if self.occupied.is_none() && self.latest_release.is_none() {
            return Err(StoreError::MalformedPageNameIndex);
        }
        if self.occupied.as_ref().is_some_and(|occupied| {
            occupied
                .winning_state
                .live_name()
                .is_none_or(|name| name.key_digest() != expected_key)
        }) || self
            .latest_release
            .as_ref()
            .is_some_and(|released| released.prior_exact_name.key_digest() != expected_key)
        {
            return Err(StoreError::MalformedPageNameIndex);
        }
        Ok(())
    }
}

/// Opaque values extracted from one authenticated exact catalog checkpoint.
///
/// P2N2 I1-I3 has no authenticated extractor yet, so production code has no
/// constructor and the reconstruction seam remains inside this module. I4+
/// may expose it only when exact-catalog decoding and validation can mint this
/// value without accepting caller-supplied evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ExactCatalogPageNameCheckpointV1 {
    catalog_document_id: DocumentId,
    catalog_causal_digest: DocumentCausalDigest,
    catalog_checkpoint_binding: ContentDigest,
    entries: BTreeMap<PageNameKeyDigest, ExactCatalogOwnershipSnapshotV1>,
}

impl ExactCatalogPageNameCheckpointV1 {
    #[cfg(test)]
    fn from_authenticated_exact_checkpoint_for_test(
        catalog_document_id: DocumentId,
        catalog_causal_digest: DocumentCausalDigest,
        catalog_checkpoint_binding: ContentDigest,
        entries: BTreeMap<PageNameKeyDigest, ExactCatalogOwnershipSnapshotV1>,
    ) -> Result<Self, StoreError> {
        let checkpoint = Self {
            catalog_document_id,
            catalog_causal_digest,
            catalog_checkpoint_binding,
            entries,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    fn validate(&self) -> Result<(), StoreError> {
        if self.entries.len() > MAX_PAGE_NAME_POINT_BATCH {
            return Err(StoreError::PageNamePointBatchTooLarge {
                actual: self.entries.len(),
                limit: MAX_PAGE_NAME_POINT_BATCH,
            });
        }
        let mut occupied_pages = BTreeSet::new();
        for (key, snapshot) in &self.entries {
            snapshot.validate(*key)?;
            if let Some(occupied) = &snapshot.occupied {
                if !occupied_pages.insert(occupied.page_id) {
                    return Err(StoreError::MalformedPageNameIndex);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ColdPageNameReconstructionV1 {
    ownership_root: PageNameOwnershipRootV1,
    frontier_root: PageNameCatalogFrontierRootV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageNameCatalogFrontierRootV1 {
    schema_version: u32,
    bindings: ScratchLsmRoot,
}

impl PageNameCatalogFrontierRootV1 {
    fn empty() -> Self {
        Self {
            schema_version: PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
            bindings: ScratchLsmRoot::default(),
        }
    }

    fn validate(&self) -> Result<(), StoreError> {
        require_version(
            "page-name catalog-frontier root",
            self.schema_version,
            PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
        )
    }
}

impl Default for PageNameCatalogFrontierRootV1 {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageNameCatalogFrontierBindingV1 {
    schema_version: u32,
    index_domain: ContentDigest,
    catalog_document_id: DocumentId,
    catalog_causal_digest: DocumentCausalDigest,
    key_version: u32,
    catalog_checkpoint_binding: ContentDigest,
    ownership_root_digest: ContentDigest,
}

impl PageNameCatalogFrontierBindingV1 {
    fn new(
        catalog_document_id: DocumentId,
        catalog_causal_digest: DocumentCausalDigest,
        catalog_checkpoint_binding: ContentDigest,
        ownership_root_digest: ContentDigest,
    ) -> Self {
        Self {
            schema_version: PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
            index_domain: page_name_index_domain_digest(),
            catalog_document_id,
            catalog_causal_digest,
            key_version: PAGE_NAME_KEY_VERSION,
            catalog_checkpoint_binding,
            ownership_root_digest,
        }
    }

    fn validate(
        &self,
        catalog_document_id: DocumentId,
        catalog_causal_digest: DocumentCausalDigest,
    ) -> Result<(), StoreError> {
        require_version(
            "page-name catalog-frontier binding",
            self.schema_version,
            PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
        )?;
        require_version("page-name key", self.key_version, PAGE_NAME_KEY_VERSION)?;
        if self.index_domain != page_name_index_domain_digest()
            || self.catalog_document_id != catalog_document_id
            || self.catalog_causal_digest != catalog_causal_digest
        {
            return Err(StoreError::MisboundPageNameCatalogFrontier);
        }
        Ok(())
    }
}

fn publish_catalog_frontier_binding(
    scratch: &ScratchStore,
    root: &PageNameCatalogFrontierRootV1,
    binding: PageNameCatalogFrontierBindingV1,
) -> Result<PageNameCatalogFrontierRootV1, StoreError> {
    root.validate()?;
    binding.validate(binding.catalog_document_id, binding.catalog_causal_digest)?;
    let key = catalog_frontier_key(binding.catalog_document_id, binding.catalog_causal_digest);
    let bindings = scratch
        .insert_many(
            &root.bindings,
            ScratchPageKind::PageNameCatalogFrontier,
            &BTreeMap::from([(key, Some(encode_canonical(&binding)?))]),
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))?;
    Ok(PageNameCatalogFrontierRootV1 {
        schema_version: PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
        bindings,
    })
}

fn require_catalog_frontier_binding(
    scratch: &ScratchStore,
    root: &PageNameCatalogFrontierRootV1,
    catalog_document_id: DocumentId,
    catalog_causal_digest: DocumentCausalDigest,
    expected_checkpoint_binding: ContentDigest,
) -> Result<ContentDigest, StoreError> {
    root.validate()?;
    let key = catalog_frontier_key(catalog_document_id, catalog_causal_digest);
    let bytes = scratch
        .lookup(
            &root.bindings,
            ScratchPageKind::PageNameCatalogFrontier,
            &key,
        )
        .map_err(|error| StoreError::Scratch(error.to_string()))?
        .ok_or(StoreError::MissingPageNameCatalogFrontier)?;
    let binding: PageNameCatalogFrontierBindingV1 = decode_canonical(&bytes)?;
    binding.validate(catalog_document_id, catalog_causal_digest)?;
    if binding.catalog_checkpoint_binding != expected_checkpoint_binding {
        return Err(StoreError::MisboundPageNameCatalogFrontier);
    }
    Ok(binding.ownership_root_digest)
}

fn catalog_frontier_key(
    catalog_document_id: DocumentId,
    catalog_causal_digest: DocumentCausalDigest,
) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(b"tine/page-name-ownership/catalog-frontier-key/v1\0");
    hasher.update(catalog_document_id.as_uuid().as_bytes());
    hasher.update(catalog_causal_digest.as_bytes());
    hasher.finalize().to_vec()
}

fn page_name_index_domain_digest() -> ContentDigest {
    ContentDigest::of(PAGE_NAME_INDEX_DOMAIN)
}

fn exact_name_blob_filename(digest: ContentDigest) -> String {
    format!("{digest}{EXACT_NAME_BLOB_SUFFIX}")
}

fn decode_record(
    expected_key: PageNameKeyDigest,
    bytes: &[u8],
) -> Result<PageNameOwnershipRecordV1, StoreError> {
    let record: PageNameOwnershipRecordV1 = decode_canonical(bytes)?;
    record.validate_shape(expected_key)?;
    Ok(record)
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    postcard::to_allocvec(value).map_err(|_| StoreError::MalformedPageNameIndex)
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(
    bytes: &[u8],
) -> Result<T, StoreError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| StoreError::MalformedPageNameIndex)?;
    if encode_canonical(&value)? != bytes {
        return Err(StoreError::MalformedPageNameIndex);
    }
    Ok(value)
}

fn require_version(store: &'static str, found: u32, current: u32) -> Result<(), StoreError> {
    if found < current {
        return Err(StoreError::UpgradeRequired {
            store,
            found,
            current,
        });
    }
    if found > current {
        return Err(StoreError::UnsupportedStoreVersion {
            store,
            version: found,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        CausalPeerId, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies, ManagedPath,
        ManagedTextKind, ObjectStore, WorkspaceId, MAX_LOGICAL_PAGE_NAME_BYTES,
    };

    fn store(name: &str) -> (PathBuf, ObjectStore, PageNameOwnershipStore) {
        let path =
            std::env::temp_dir().join(format!("tine-page-name-index-{name}-{}", Uuid::new_v4()));
        let archive =
            ObjectStore::open(&path, WorkspaceId::from_uuid(Uuid::from_u128(0x100))).unwrap();
        let index = archive.open_page_name_ownership_index().unwrap();
        (path, archive, index)
    }

    fn dot(counter: u64) -> BatchCausalDot {
        BatchCausalDot::new(
            CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(0x200))),
            counter,
        )
        .unwrap()
    }

    fn page(value: u128) -> PageId {
        PageId::from_uuid(Uuid::from_u128(value))
    }

    fn batch(value: u128) -> BatchId {
        BatchId::from_uuid(Uuid::from_u128(value))
    }

    fn document(value: u128) -> DocumentId {
        DocumentId::from_uuid(Uuid::from_u128(value))
    }

    fn live_state(name: &str) -> PageState {
        PageState::Live {
            name: LogicalPageName::parse(name).unwrap(),
            path: ManagedPath::parse(format!("pages/{name}.md")).unwrap(),
            home_document_id: document(0x300),
            kind: ManagedTextKind::Page,
        }
    }

    fn occupied(
        index: &PageNameOwnershipStore,
        name: &LogicalPageName,
        page_id: PageId,
    ) -> PageNameOwnershipOccupiedV1 {
        PageNameOwnershipOccupiedV1::new(
            page_id,
            index.put_exact_name(name).unwrap(),
            batch(0x400),
            dot(1),
            batch(0x401),
            dot(2),
        )
    }

    fn causal_digest(document_id: DocumentId, counter: u64) -> DocumentCausalDigest {
        DocumentDependencies::new(
            document_id,
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(7), counter)],
            vec![batch(0x500 + counter as u128)],
        )
        .unwrap()
        .causal_state_digest()
    }

    fn seed_blob(
        path: &std::path::Path,
        name: &LogicalPageName,
        schema_version: u32,
    ) -> ExactLogicalPageNameRefV1 {
        let blob = ExactLogicalPageNameBlobV1 {
            schema_version,
            exact_name: name.clone(),
        };
        let bytes = encode_canonical(&blob).unwrap();
        let content_digest = ContentDigest::of(&bytes);
        fs::write(
            path.join(PAGE_NAME_OWNERSHIP_INDEX_DIR_FOR_TEST)
                .join("exact-names")
                .join(exact_name_blob_filename(content_digest)),
            &bytes,
        )
        .unwrap();
        ExactLogicalPageNameRefV1 {
            schema_version: EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION,
            encoded_len: bytes.len() as u64,
            content_digest,
            exact_name_digest: ExactLogicalPageNameDigest::of(name),
        }
    }

    const PAGE_NAME_OWNERSHIP_INDEX_DIR_FOR_TEST: &str = "page-name-ownership-index-v1";

    #[test]
    fn exact_name_digest_blob_and_maximum_name_are_strict_and_leaf_bounded() {
        let (path, archive, index) = store("exact-blob");
        let name = LogicalPageName::parse("Foo").unwrap();
        assert_eq!(
            ExactLogicalPageNameDigest::of(&name).to_string(),
            "03f1e9d8a353e71351a76e63545833ea6bead1d35556b71655ca7787d164460f"
        );

        let name_ref = index.put_exact_name(&name).unwrap();
        assert_eq!(
            index.read_exact_name(name.key_digest(), &name_ref).unwrap(),
            name
        );
        let prior = LogicalPageName::parse("FOO").unwrap();
        let record = PageNameOwnershipRecordV1::new(
            name.key_digest(),
            Some(PageNameOwnershipOccupiedV1::new(
                page(1),
                name_ref,
                batch(2),
                dot(1),
                batch(3),
                dot(2),
            )),
            Some(PageNameOwnershipReleasedV1::new(
                page(9),
                index.put_exact_name(&prior).unwrap(),
                batch(10),
                dot(3),
                batch(11),
                dot(4),
                batch(12),
                dot(5),
            )),
        )
        .unwrap();
        assert!(encode_canonical(&record).unwrap().len() < 4 * 1024);
        assert!(PageNameOwnershipRecordV1::new(name.key_digest(), None, None).is_err());
        let root = index
            .insert_many(
                &PageNameOwnershipRootV1::empty(),
                &BTreeMap::from([(name.key_digest(), record)]),
            )
            .unwrap();

        let maximum = LogicalPageName::parse("x".repeat(MAX_LOGICAL_PAGE_NAME_BYTES)).unwrap();
        let maximum_ref = index.put_exact_name(&maximum).unwrap();
        assert_eq!(
            index
                .read_exact_name(maximum.key_digest(), &maximum_ref)
                .unwrap(),
            maximum
        );

        drop(index);
        drop(archive);
        let reopened =
            ObjectStore::open(&path, WorkspaceId::from_uuid(Uuid::from_u128(0x100))).unwrap();
        let reopened_index = reopened.open_page_name_ownership_index().unwrap();
        let found = reopened_index
            .lookup(&root, name.key_digest())
            .unwrap()
            .unwrap();
        assert_eq!(found.occupied().unwrap().page_id(), page(1));
        assert_eq!(found.latest_release().unwrap().prior_page_id(), page(9));
        drop(reopened_index);
        drop(reopened);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn blob_tamper_and_cross_key_substitution_fail_closed() {
        let (path, archive, index) = store("blob-tamper");
        let foo = LogicalPageName::parse("Foo").unwrap();
        let foo_ref = index.put_exact_name(&foo).unwrap();
        let blob_path = path
            .join(PAGE_NAME_OWNERSHIP_INDEX_DIR_FOR_TEST)
            .join("exact-names")
            .join(exact_name_blob_filename(foo_ref.content_digest()));
        let mut bytes = fs::read(&blob_path).unwrap();
        bytes[0] ^= 0x80;
        fs::write(&blob_path, bytes).unwrap();
        assert!(matches!(
            index.read_exact_name(foo.key_digest(), &foo_ref),
            Err(StoreError::ExactLogicalPageNameBlobPathMismatch(_))
        ));
        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();

        let (path, archive, index) = store("blob-substitution");
        let foo_ref = index.put_exact_name(&foo).unwrap();
        let bar = LogicalPageName::parse("Bar").unwrap();
        let invalid = PageNameOwnershipRecordV1::new(
            bar.key_digest(),
            Some(PageNameOwnershipOccupiedV1::new(
                page(1),
                foo_ref,
                batch(2),
                dot(1),
                batch(3),
                dot(2),
            )),
            None,
        )
        .unwrap();
        assert!(matches!(
            index.insert_many(
                &PageNameOwnershipRootV1::empty(),
                &BTreeMap::from([(bar.key_digest(), invalid)])
            ),
            Err(StoreError::MalformedPageNameIndex)
        ));
        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn root_record_reference_and_blob_prior_future_versions_are_classified() {
        for found in [0, 2] {
            let (path, archive, index) = store(&format!("store-version-{found}"));
            drop(index);
            let claim = PageNameOwnershipStoreClaimV1 {
                schema_version: found,
                key_version: PAGE_NAME_KEY_VERSION,
            };
            fs::write(
                path.join(PAGE_NAME_OWNERSHIP_INDEX_DIR_FOR_TEST)
                    .join(STORE_CLAIM_FILE),
                encode_canonical(&claim).unwrap(),
            )
            .unwrap();
            match (found, archive.open_page_name_ownership_index()) {
                (0, Err(StoreError::UpgradeRequired { store, .. })) => {
                    assert_eq!(store, "page-name ownership store")
                }
                (2, Err(StoreError::UnsupportedStoreVersion { store, .. })) => {
                    assert_eq!(store, "page-name ownership store")
                }
                (_, result) => panic!("unexpected store version result: {result:?}"),
            }
            drop(archive);
            fs::remove_dir_all(path).unwrap();
        }

        for found in [0, 2] {
            let root = PageNameOwnershipRootV1 {
                schema_version: found,
                key_version: PAGE_NAME_KEY_VERSION,
                patricia_root: PatriciaIndexRoot::empty(),
                entry_count: 0,
            };
            let bytes = encode_canonical(&root).unwrap();
            match (found, PageNameOwnershipRootV1::decode(&bytes)) {
                (0, Err(StoreError::UpgradeRequired { store, .. })) => {
                    assert_eq!(store, "page-name ownership root")
                }
                (2, Err(StoreError::UnsupportedStoreVersion { store, .. })) => {
                    assert_eq!(store, "page-name ownership root")
                }
                (_, result) => panic!("unexpected root version result: {result:?}"),
            }
        }

        for found in [0, 2] {
            let (path, archive, index) = store(&format!("record-version-{found}"));
            let name = LogicalPageName::parse("Versioned").unwrap();
            let key = name.key_digest();
            let invalid = PageNameOwnershipRecordV1 {
                schema_version: found,
                key_version: PAGE_NAME_KEY_VERSION,
                key_digest: key,
                occupied: Some(occupied(&index, &name, page(1))),
                latest_release: None,
            };
            let raw = index
                .patricia
                .insert_many(
                    PatriciaIndexRoot::empty(),
                    &BTreeMap::from([(
                        key.as_bytes().to_vec(),
                        encode_canonical(&invalid).unwrap(),
                    )]),
                )
                .unwrap();
            let root = PageNameOwnershipRootV1 {
                schema_version: PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION,
                key_version: PAGE_NAME_KEY_VERSION,
                patricia_root: raw,
                entry_count: 1,
            };
            match (found, index.lookup(&root, key)) {
                (0, Err(StoreError::UpgradeRequired { store, .. })) => {
                    assert_eq!(store, "page-name ownership record")
                }
                (2, Err(StoreError::UnsupportedStoreVersion { store, .. })) => {
                    assert_eq!(store, "page-name ownership record")
                }
                (_, result) => panic!("unexpected record version result: {result:?}"),
            }
            drop(index);
            drop(archive);
            fs::remove_dir_all(path).unwrap();
        }

        for found in [0, 2] {
            let (path, archive, index) = store(&format!("ref-version-{found}"));
            let name = LogicalPageName::parse("Nested Ref").unwrap();
            let key = name.key_digest();
            let mut occupied = occupied(&index, &name, page(1));
            occupied.exact_name.schema_version = found;
            let invalid = PageNameOwnershipRecordV1 {
                schema_version: PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION,
                key_version: PAGE_NAME_KEY_VERSION,
                key_digest: key,
                occupied: Some(occupied),
                latest_release: None,
            };
            let raw = index
                .patricia
                .insert_many(
                    PatriciaIndexRoot::empty(),
                    &BTreeMap::from([(
                        key.as_bytes().to_vec(),
                        encode_canonical(&invalid).unwrap(),
                    )]),
                )
                .unwrap();
            let root = PageNameOwnershipRootV1 {
                schema_version: PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION,
                key_version: PAGE_NAME_KEY_VERSION,
                patricia_root: raw,
                entry_count: 1,
            };
            match (found, index.lookup(&root, key)) {
                (0, Err(StoreError::UpgradeRequired { store, .. })) => {
                    assert_eq!(store, "exact logical page-name reference")
                }
                (2, Err(StoreError::UnsupportedStoreVersion { store, .. })) => {
                    assert_eq!(store, "exact logical page-name reference")
                }
                (_, result) => panic!("unexpected reference version result: {result:?}"),
            }
            drop(index);
            drop(archive);
            fs::remove_dir_all(path).unwrap();
        }

        for found in [0, 2] {
            let (path, archive, index) = store(&format!("blob-version-{found}"));
            let name = LogicalPageName::parse("Nested Blob").unwrap();
            let name_ref = seed_blob(&path, &name, found);
            match (found, index.read_exact_name(name.key_digest(), &name_ref)) {
                (0, Err(StoreError::UpgradeRequired { store, .. })) => {
                    assert_eq!(store, "exact logical page-name blob")
                }
                (2, Err(StoreError::UnsupportedStoreVersion { store, .. })) => {
                    assert_eq!(store, "exact logical page-name blob")
                }
                (_, result) => panic!("unexpected blob version result: {result:?}"),
            }
            drop(index);
            drop(archive);
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn authenticated_node_tamper_and_noncanonical_lookup_many_fail_closed() {
        let (path, archive, index) = store("point-refusal");
        let name = LogicalPageName::parse("Point").unwrap();
        let key = name.key_digest();
        let record =
            PageNameOwnershipRecordV1::new(key, Some(occupied(&index, &name, page(1))), None)
                .unwrap();
        let root = index
            .insert_many(
                &PageNameOwnershipRootV1::empty(),
                &BTreeMap::from([(key, record)]),
            )
            .unwrap();
        assert!(matches!(
            index.lookup_many(&root, &[key, key]),
            Err(StoreError::NonCanonicalPageNamePointKeys)
        ));
        assert!(matches!(
            index.lookup_many(&root, &vec![key; MAX_PAGE_NAME_POINT_BATCH + 1]),
            Err(StoreError::PageNamePointBatchTooLarge { .. })
        ));

        let node_path = path
            .join(PAGE_NAME_OWNERSHIP_INDEX_DIR_FOR_TEST)
            .join("nodes")
            .join(format!("{}.patricia-node", root.patricia_digest()));
        let mut bytes = fs::read(&node_path).unwrap();
        bytes[0] ^= 0x40;
        fs::write(node_path, bytes).unwrap();
        assert!(index.lookup(&root, key).is_err());

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn early_and_late_one_key_lookup_remain_point_local_in_large_index() {
        const ENTRIES: usize = 1_024;

        let (path, archive, index) = store("point-cost");
        let mut records = BTreeMap::new();
        let mut insertion_order = Vec::new();
        for value in 0..ENTRIES {
            let name = LogicalPageName::parse(format!("Page {value:04}")).unwrap();
            let key = name.key_digest();
            insertion_order.push(key);
            let name_ref = seed_blob(&path, &name, EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION);
            records.insert(
                key,
                PageNameOwnershipRecordV1::new(
                    key,
                    Some(PageNameOwnershipOccupiedV1::new(
                        page(0x1_000 + value as u128),
                        name_ref,
                        batch(0x2_000 + value as u128),
                        dot(value as u64 + 1),
                        batch(0x3_000 + value as u128),
                        dot(value as u64 + 1),
                    )),
                    None,
                )
                .unwrap(),
            );
        }
        let root = index
            .insert_many(&PageNameOwnershipRootV1::empty(), &records)
            .unwrap();
        assert_eq!(root.entry_count(), ENTRIES as u64);

        let before_early = index.stats();
        assert!(index.lookup(&root, insertion_order[0]).unwrap().is_some());
        let after_early = index.stats();
        assert!(index
            .lookup(&root, insertion_order[ENTRIES - 1])
            .unwrap()
            .is_some());
        let after_late = index.stats();
        let early_reads = after_early.reads - before_early.reads;
        let late_reads = after_late.reads - after_early.reads;
        let early_bytes = after_early.bytes_read - before_early.bytes_read;
        let late_bytes = after_late.bytes_read - after_early.bytes_read;
        eprintln!(
            "page-name point lookup counters: entries={ENTRIES} early_reads={early_reads} \
             late_reads={late_reads} early_bytes={early_bytes} late_bytes={late_bytes}"
        );
        assert!(
            early_reads <= 64,
            "early point lookup read {early_reads} nodes"
        );
        assert!(
            late_reads <= 64,
            "late point lookup read {late_reads} nodes"
        );
        assert!(
            early_reads.abs_diff(late_reads) <= 16,
            "lookup depth depended on insertion position: {early_reads} vs {late_reads}"
        );
        assert!(early_bytes <= 64 * 1024);
        assert!(late_bytes <= 64 * 1024);

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn exact_catalog_reconstruction_is_deterministic_and_binds_exact_frontier() {
        let (path, archive, index) = store("cold-reconstruction");
        let (scratch, claim_index) = archive.start_engine_scratch().unwrap();
        let catalog_document_id = document(0x600);
        let catalog_causal_digest = causal_digest(catalog_document_id, 1);
        let checkpoint_binding = ContentDigest::of(b"authenticated exact catalog checkpoint");
        let state = live_state("Owned");
        let key = state.name().key_digest();
        let occupied = ExactCatalogOccupiedSnapshotV1::new(
            page(0x601),
            state,
            batch(0x602),
            dot(1),
            batch(0x603),
            dot(2),
        )
        .unwrap();
        let released = ExactCatalogReleasedSnapshotV1::new(
            page(0x610),
            LogicalPageName::parse("OWNED").unwrap(),
            batch(0x611),
            dot(3),
            batch(0x612),
            dot(4),
            batch(0x613),
            dot(5),
        );
        let checkpoint =
            ExactCatalogPageNameCheckpointV1::from_authenticated_exact_checkpoint_for_test(
                catalog_document_id,
                catalog_causal_digest,
                checkpoint_binding,
                BTreeMap::from([(
                    key,
                    ExactCatalogOwnershipSnapshotV1::new(Some(occupied), Some(released)).unwrap(),
                )]),
            )
            .unwrap();

        let first = index
            .reconstruct_from_exact_catalog_checkpoint(
                &scratch,
                &PageNameCatalogFrontierRootV1::empty(),
                &checkpoint,
            )
            .unwrap();
        let second = index
            .reconstruct_from_exact_catalog_checkpoint(
                &scratch,
                &PageNameCatalogFrontierRootV1::empty(),
                &checkpoint,
            )
            .unwrap();
        assert_eq!(first.ownership_root, second.ownership_root);
        assert_eq!(
            require_catalog_frontier_binding(
                &scratch,
                &first.frontier_root,
                catalog_document_id,
                catalog_causal_digest,
                checkpoint_binding,
            )
            .unwrap(),
            first.ownership_root.external_digest().unwrap()
        );
        let found = index.lookup(&first.ownership_root, key).unwrap().unwrap();
        assert_eq!(found.occupied().unwrap().page_id(), page(0x601));
        assert_eq!(found.latest_release().unwrap().prior_page_id(), page(0x610));

        assert!(ExactCatalogOccupiedSnapshotV1::new(
            page(0x604),
            PageState::Tombstone {
                name: LogicalPageName::parse("Owned").unwrap(),
                home_document_id: document(0x605),
                kind: ManagedTextKind::Page,
            },
            batch(0x606),
            dot(3),
            batch(0x607),
            dot(4),
        )
        .is_err());

        drop(first);
        drop(second);
        drop(claim_index);
        drop(scratch);
        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn authenticated_checkpoint_and_frontier_construction_are_not_crate_visible() {
        let source = include_str!("page_name_index.rs");
        for constructor in [
            "from_authenticated_exact_checkpoint",
            "publish_catalog_frontier_binding",
        ] {
            let exposed = ["pub(crate) fn ", constructor].concat();
            assert!(
                !source.contains(&exposed),
                "{constructor} must not be a production crate-visible capability"
            );
        }
        for evidence_type in [
            "ExactCatalogPageNameCheckpointV1",
            "PageNameCatalogFrontierBindingV1",
            "PageNameCatalogFrontierRootV1",
        ] {
            let exposed = ["pub(crate) struct ", evidence_type].concat();
            assert!(
                !source.contains(&exposed),
                "{evidence_type} must remain opaque outside page_name_index"
            );
        }
        assert!(
            source.contains("#[cfg(test)]\n    fn from_authenticated_exact_checkpoint_for_test")
        );
        assert!(source.contains(
            "archive_proof: &super::hot_engine::AuthenticatedCatalogCheckpointArchiveProof"
        ));
        let hot_engine_source = include_str!("hot_engine.rs");
        assert!(
            !hot_engine_source.contains("pub(crate) fn authenticate_catalog_checkpoint_archive")
        );
        assert!(!hot_engine_source.contains(
            "pub(crate) fn new(\n        catalog_document_id: DocumentId,\n        catalog_causal_digest"
        ));
    }

    #[test]
    fn missing_misbound_and_cross_index_frontier_bindings_refuse() {
        let (path, archive, index) = store("frontier-refusal");
        let (scratch, claim_index) = archive.start_engine_scratch().unwrap();
        let catalog_document_id = document(0x700);
        let causal = causal_digest(catalog_document_id, 1);
        let checkpoint = ContentDigest::of(b"checkpoint-a");
        let ownership = ContentDigest::of(b"ownership-a");

        for found in [0, 2] {
            let invalid_root = PageNameCatalogFrontierRootV1 {
                schema_version: found,
                bindings: ScratchLsmRoot::default(),
            };
            match require_catalog_frontier_binding(
                &scratch,
                &invalid_root,
                catalog_document_id,
                causal,
                checkpoint,
            ) {
                Err(StoreError::UpgradeRequired { store, .. }) if found == 0 => {
                    assert_eq!(store, "page-name catalog-frontier root")
                }
                Err(StoreError::UnsupportedStoreVersion { store, .. }) if found == 2 => {
                    assert_eq!(store, "page-name catalog-frontier root")
                }
                result => panic!("unexpected frontier-root version result: {result:?}"),
            }
        }

        assert!(matches!(
            require_catalog_frontier_binding(
                &scratch,
                &PageNameCatalogFrontierRootV1::empty(),
                catalog_document_id,
                causal,
                checkpoint,
            ),
            Err(StoreError::MissingPageNameCatalogFrontier)
        ));

        let root = publish_catalog_frontier_binding(
            &scratch,
            &PageNameCatalogFrontierRootV1::empty(),
            PageNameCatalogFrontierBindingV1::new(
                catalog_document_id,
                causal,
                checkpoint,
                ownership,
            ),
        )
        .unwrap();
        assert!(matches!(
            require_catalog_frontier_binding(
                &scratch,
                &root,
                catalog_document_id,
                causal,
                ContentDigest::of(b"checkpoint-b"),
            ),
            Err(StoreError::MisboundPageNameCatalogFrontier)
        ));

        for found in [0, 2] {
            let mut invalid = PageNameCatalogFrontierBindingV1::new(
                catalog_document_id,
                causal,
                checkpoint,
                ownership,
            );
            invalid.schema_version = found;
            let key = catalog_frontier_key(catalog_document_id, causal);
            let bindings = scratch
                .insert_many(
                    &ScratchLsmRoot::default(),
                    ScratchPageKind::PageNameCatalogFrontier,
                    &BTreeMap::from([(key, Some(encode_canonical(&invalid).unwrap()))]),
                )
                .unwrap();
            let invalid_root = PageNameCatalogFrontierRootV1 {
                schema_version: PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
                bindings,
            };
            match require_catalog_frontier_binding(
                &scratch,
                &invalid_root,
                catalog_document_id,
                causal,
                checkpoint,
            ) {
                Err(StoreError::UpgradeRequired { store, .. }) if found == 0 => {
                    assert_eq!(store, "page-name catalog-frontier binding")
                }
                Err(StoreError::UnsupportedStoreVersion { store, .. }) if found == 2 => {
                    assert_eq!(store, "page-name catalog-frontier binding")
                }
                result => panic!("unexpected frontier-binding version result: {result:?}"),
            }
        }

        let mut foreign = PageNameCatalogFrontierBindingV1::new(
            catalog_document_id,
            causal,
            checkpoint,
            ownership,
        );
        foreign.index_domain = ContentDigest::of(b"tine/foreign-index/v1");
        let key = catalog_frontier_key(catalog_document_id, causal);
        let bindings = scratch
            .insert_many(
                &ScratchLsmRoot::default(),
                ScratchPageKind::PageNameCatalogFrontier,
                &BTreeMap::from([(key, Some(encode_canonical(&foreign).unwrap()))]),
            )
            .unwrap();
        let cross_index_root = PageNameCatalogFrontierRootV1 {
            schema_version: PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION,
            bindings,
        };
        assert!(matches!(
            require_catalog_frontier_binding(
                &scratch,
                &cross_index_root,
                catalog_document_id,
                causal,
                checkpoint,
            ),
            Err(StoreError::MisboundPageNameCatalogFrontier)
        ));

        drop(claim_index);
        drop(scratch);
        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    fn authenticated_page_points_for_test(
        states: Vec<(PageId, Option<PageState>)>,
    ) -> AuthenticatedCatalogPageNameCheckpointV1 {
        AuthenticatedCatalogPageNameCheckpointV1 {
            catalog_document_id: document(0x900),
            catalog_causal_digest: causal_digest(document(0x900), 1),
            catalog_checkpoint_binding: ContentDigest::of(b"authenticated catalog points"),
            catalog_checkpoint_content_digest: ContentDigest::of(
                b"authenticated catalog checkpoint bytes",
            ),
            entries: states.into_iter().collect(),
        }
    }

    fn empty_frontier() -> FrontierV2 {
        FrontierV2::new(Vec::new()).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn transition(
        index: &PageNameOwnershipStore,
        root: &PageNameOwnershipRootV1,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        checkpoint: &AuthenticatedCatalogPageNameCheckpointV1,
        deltas: &[PageDelta],
        current: Vec<(PageId, Option<PageState>)>,
        prospective: Vec<(PageId, Option<PageState>)>,
        contained: &[BatchId],
    ) -> Result<PageNamePublicationCandidateV1, PageNameTransitionError> {
        let contained = contained.iter().copied().collect::<BTreeSet<_>>();
        prepare_page_name_transition(
            index,
            root,
            batch_id,
            causal_dot,
            &empty_frontier(),
            checkpoint,
            deltas,
            &current.into_iter().collect(),
            &prospective.into_iter().collect(),
            |_, introducing_batch| contained.contains(&introducing_batch),
            |_| Some(empty_frontier()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn ephemeral_transition(
        state: &EphemeralPageNameOwnershipStateV1,
        batch_id: BatchId,
        causal_dot: BatchCausalDot,
        deltas: &[PageDelta],
        exact_before: Vec<(PageId, Option<PageState>)>,
        current: Vec<(PageId, Option<PageState>)>,
        prospective: Vec<(PageId, Option<PageState>)>,
        contained: &[BatchId],
    ) -> Result<PageNamePublicationCandidateV1, PageNameTransitionError> {
        let contained = contained.iter().copied().collect::<BTreeSet<_>>();
        let exact_before = AuthoritativeCatalogPageNameObservationsV1 {
            entries: exact_before.into_iter().collect(),
        };
        prepare_ephemeral_page_name_transition(
            state,
            batch_id,
            causal_dot,
            &empty_frontier(),
            &exact_before,
            deltas,
            &current.into_iter().collect(),
            &prospective.into_iter().collect(),
            |_, introducing_batch| contained.contains(&introducing_batch),
            |_| Some(empty_frontier()),
        )
    }

    fn page_delta(
        page_id: PageId,
        before: Option<PageState>,
        after: Option<PageState>,
    ) -> PageDelta {
        PageDelta {
            page_id,
            before,
            after,
        }
    }

    #[test]
    fn transition_create_delete_causal_reuse_swap_replay_and_path_only_move() {
        let (path, archive, index) = store("transition-basics");
        let page_a = page(0xa01);
        let page_b = page(0xa02);
        let alpha = live_state("Alpha");
        let beta = live_state("Beta");
        let alpha_tombstone = PageState::Tombstone {
            name: LogicalPageName::parse("Alpha").unwrap(),
            home_document_id: alpha.home_document_id(),
            kind: ManagedTextKind::Page,
        };

        let create = transition(
            &index,
            &PageNameOwnershipRootV1::empty(),
            batch(0xa10),
            dot(10),
            &authenticated_page_points_for_test(vec![(page_a, None)]),
            &[page_delta(page_a, None, Some(alpha.clone()))],
            vec![(page_a, None)],
            vec![(page_a, Some(alpha.clone()))],
            &[],
        )
        .unwrap();
        assert!(create.conflicts.is_empty());
        let acquisition = index
            .lookup(&create.root, alpha.name().key_digest())
            .unwrap()
            .unwrap()
            .occupied()
            .unwrap()
            .clone();

        let replay = transition(
            &index,
            &create.root,
            batch(0xa11),
            dot(11),
            &authenticated_page_points_for_test(vec![(page_a, Some(alpha.clone()))]),
            &[],
            Vec::new(),
            Vec::new(),
            &[batch(0xa10)],
        )
        .unwrap();
        assert_eq!(replay.root, create.root);

        let moved = PageState::Live {
            name: LogicalPageName::parse("Alpha").unwrap(),
            path: ManagedPath::parse("journals/unrelated/physical.md").unwrap(),
            home_document_id: alpha.home_document_id(),
            kind: ManagedTextKind::Page,
        };
        let path_only = transition(
            &index,
            &create.root,
            batch(0xa12),
            dot(12),
            &authenticated_page_points_for_test(vec![(page_a, Some(alpha.clone()))]),
            &[page_delta(page_a, Some(alpha.clone()), Some(moved.clone()))],
            vec![(page_a, Some(alpha.clone()))],
            vec![(page_a, Some(moved))],
            &[batch(0xa10)],
        )
        .unwrap();
        assert_eq!(path_only.root, create.root);

        let deleted = transition(
            &index,
            &create.root,
            batch(0xa13),
            dot(13),
            &authenticated_page_points_for_test(vec![(page_a, Some(alpha.clone()))]),
            &[page_delta(
                page_a,
                Some(alpha.clone()),
                Some(alpha_tombstone.clone()),
            )],
            vec![(page_a, Some(alpha.clone()))],
            vec![(page_a, Some(alpha_tombstone.clone()))],
            &[batch(0xa10)],
        )
        .unwrap();
        let released = index
            .lookup(&deleted.root, alpha.name().key_digest())
            .unwrap()
            .unwrap();
        assert!(released.occupied().is_none());
        assert_eq!(
            released.latest_release().unwrap().release_batch(),
            batch(0xa13)
        );

        let reused = transition(
            &index,
            &deleted.root,
            batch(0xa14),
            dot(14),
            &authenticated_page_points_for_test(vec![(page_b, None)]),
            &[page_delta(page_b, None, Some(alpha.clone()))],
            vec![(page_b, None)],
            vec![(page_b, Some(alpha.clone()))],
            &[batch(0xa13)],
        )
        .unwrap();
        assert_eq!(
            index
                .lookup(&reused.root, alpha.name().key_digest())
                .unwrap()
                .unwrap()
                .occupied()
                .unwrap()
                .page_id(),
            page_b
        );

        let same_batch_reused = transition(
            &index,
            &create.root,
            batch(0xa15),
            dot(15),
            &authenticated_page_points_for_test(vec![
                (page_a, Some(alpha.clone())),
                (page_b, None),
            ]),
            &[
                page_delta(page_a, Some(alpha.clone()), Some(alpha_tombstone.clone())),
                page_delta(page_b, None, Some(alpha.clone())),
            ],
            vec![(page_a, Some(alpha.clone())), (page_b, None)],
            vec![
                (page_a, Some(alpha_tombstone)),
                (page_b, Some(alpha.clone())),
            ],
            &[batch(0xa10)],
        )
        .unwrap();
        assert_eq!(
            index
                .lookup(&same_batch_reused.root, alpha.name().key_digest())
                .unwrap()
                .unwrap()
                .occupied()
                .unwrap()
                .page_id(),
            page_b
        );

        let root = transition(
            &index,
            &PageNameOwnershipRootV1::empty(),
            batch(0xa20),
            dot(20),
            &authenticated_page_points_for_test(vec![(page_a, None), (page_b, None)]),
            &[
                page_delta(page_a, None, Some(alpha.clone())),
                page_delta(page_b, None, Some(beta.clone())),
            ],
            vec![(page_a, None), (page_b, None)],
            vec![(page_a, Some(alpha.clone())), (page_b, Some(beta.clone()))],
            &[],
        )
        .unwrap()
        .root;
        let swapped = transition(
            &index,
            &root,
            batch(0xa21),
            dot(21),
            &authenticated_page_points_for_test(vec![
                (page_a, Some(alpha.clone())),
                (page_b, Some(beta.clone())),
            ]),
            &[
                page_delta(page_a, Some(alpha.clone()), Some(beta.clone())),
                page_delta(page_b, Some(beta.clone()), Some(alpha.clone())),
            ],
            vec![(page_a, Some(alpha.clone())), (page_b, Some(beta.clone()))],
            vec![(page_a, Some(beta.clone())), (page_b, Some(alpha.clone()))],
            &[batch(0xa20)],
        )
        .unwrap();
        assert_eq!(
            index
                .lookup(&swapped.root, alpha.name().key_digest())
                .unwrap()
                .unwrap()
                .occupied()
                .unwrap()
                .page_id(),
            page_b
        );
        assert_eq!(acquisition.acquisition_batch(), batch(0xa10));

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn dependency_duplicate_and_intra_batch_duplicate_reject_without_root_change() {
        let (path, archive, index) = store("transition-duplicate-rejection");
        let alpha = live_state("Duplicate");
        let first_page = page(0xb01);
        let second_page = page(0xb02);
        let root = transition(
            &index,
            &PageNameOwnershipRootV1::empty(),
            batch(0xb10),
            dot(1),
            &authenticated_page_points_for_test(vec![(first_page, None)]),
            &[page_delta(first_page, None, Some(alpha.clone()))],
            vec![(first_page, None)],
            vec![(first_page, Some(alpha.clone()))],
            &[],
        )
        .unwrap()
        .root;
        let digest = root.external_digest().unwrap();
        assert!(matches!(
            transition(
                &index,
                &root,
                batch(0xb11),
                dot(2),
                &authenticated_page_points_for_test(vec![(second_page, None)]),
                &[page_delta(second_page, None, Some(alpha.clone()))],
                vec![(second_page, None)],
                vec![(second_page, Some(alpha.clone()))],
                &[batch(0xb10)],
            ),
            Err(PageNameTransitionError::MalformedBatch(_))
        ));
        assert_eq!(root.external_digest().unwrap(), digest);

        assert!(matches!(
            transition(
                &index,
                &PageNameOwnershipRootV1::empty(),
                batch(0xb12),
                dot(3),
                &authenticated_page_points_for_test(vec![(first_page, None), (second_page, None),]),
                &[
                    page_delta(first_page, None, Some(alpha.clone())),
                    page_delta(second_page, None, Some(alpha.clone())),
                ],
                vec![(first_page, None), (second_page, None)],
                vec![
                    (first_page, Some(alpha.clone())),
                    (second_page, Some(alpha)),
                ],
                &[],
            ),
            Err(PageNameTransitionError::MalformedBatch(_))
        ));

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn ephemeral_transition_preserves_frozen_reuse_swap_spelling_replay_path_and_tombstone_table() {
        let page_a = page(0xb21);
        let page_b = page(0xb22);
        let alpha = live_state("Alpha");
        let alpha_key = alpha.name().key_digest();
        let alpha_upper = live_state("ALPHA");
        let beta = live_state("Beta");
        let beta_key = beta.name().key_digest();
        let moved_alpha = PageState::Live {
            name: LogicalPageName::parse("Alpha").unwrap(),
            path: ManagedPath::parse("deep/physical/alpha.md").unwrap(),
            home_document_id: alpha.home_document_id(),
            kind: ManagedTextKind::Page,
        };
        let alpha_tombstone = PageState::Tombstone {
            name: LogicalPageName::parse("ALPHA").unwrap(),
            home_document_id: alpha.home_document_id(),
            kind: ManagedTextKind::Page,
        };
        let mut state = EphemeralPageNameOwnershipStateV1::default();

        let created = ephemeral_transition(
            &state,
            batch(0xb30),
            dot(30),
            &[page_delta(page_a, None, Some(alpha.clone()))],
            vec![(page_a, None)],
            vec![(page_a, None)],
            vec![(page_a, Some(alpha.clone()))],
            &[],
        )
        .unwrap();
        state.commit(created);
        assert_eq!(
            state.records[&alpha_key].occupied().unwrap().page_id(),
            page_a
        );

        let before_path_only = state.clone();
        let path_only = ephemeral_transition(
            &state,
            batch(0xb31),
            dot(31),
            &[page_delta(
                page_a,
                Some(alpha.clone()),
                Some(moved_alpha.clone()),
            )],
            vec![(page_a, Some(alpha.clone()))],
            vec![(page_a, Some(alpha.clone()))],
            vec![(page_a, Some(moved_alpha.clone()))],
            &[batch(0xb30)],
        )
        .unwrap();
        state.commit(path_only);
        assert_eq!(state, before_path_only);

        let spelling = ephemeral_transition(
            &state,
            batch(0xb32),
            dot(32),
            &[page_delta(
                page_a,
                Some(moved_alpha.clone()),
                Some(alpha_upper.clone()),
            )],
            vec![(page_a, Some(moved_alpha.clone()))],
            vec![(page_a, Some(moved_alpha.clone()))],
            vec![(page_a, Some(alpha_upper.clone()))],
            &[batch(0xb30), batch(0xb31)],
        )
        .unwrap();
        state.commit(spelling);
        let occupied = state.records[&alpha_key].occupied().unwrap();
        assert_eq!(
            state.exact_names[&(alpha_key, occupied.exact_name().clone())],
            LogicalPageName::parse("ALPHA").unwrap()
        );

        let before_replay = state.clone();
        let replay = ephemeral_transition(
            &state,
            batch(0xb32),
            dot(32),
            &[page_delta(
                page_a,
                Some(moved_alpha.clone()),
                Some(alpha_upper.clone()),
            )],
            vec![(page_a, Some(moved_alpha))],
            vec![(page_a, Some(alpha_upper.clone()))],
            vec![(page_a, Some(alpha_upper.clone()))],
            &[batch(0xb30), batch(0xb31), batch(0xb32)],
        )
        .unwrap();
        state.commit(replay);
        assert_eq!(state, before_replay);

        let deleted = ephemeral_transition(
            &state,
            batch(0xb33),
            dot(33),
            &[page_delta(
                page_a,
                Some(alpha_upper.clone()),
                Some(alpha_tombstone.clone()),
            )],
            vec![(page_a, Some(alpha_upper.clone()))],
            vec![(page_a, Some(alpha_upper.clone()))],
            vec![(page_a, Some(alpha_tombstone.clone()))],
            &[batch(0xb30), batch(0xb32)],
        )
        .unwrap();
        state.commit(deleted);
        assert!(state.records[&alpha_key].occupied().is_none());

        let reused = ephemeral_transition(
            &state,
            batch(0xb34),
            dot(34),
            &[page_delta(page_b, None, Some(alpha.clone()))],
            vec![(page_b, None)],
            vec![(page_b, None)],
            vec![(page_b, Some(alpha.clone()))],
            &[batch(0xb33)],
        )
        .unwrap();
        state.commit(reused);
        assert_eq!(
            state.records[&alpha_key].occupied().unwrap().page_id(),
            page_b
        );

        let before_losing_tombstone = state.clone();
        let losing_tombstone = ephemeral_transition(
            &state,
            batch(0xb35),
            dot(35),
            &[page_delta(
                page_a,
                Some(alpha_upper.clone()),
                Some(alpha_tombstone.clone()),
            )],
            vec![(page_a, Some(alpha_upper))],
            vec![(page_a, Some(alpha_tombstone.clone()))],
            vec![(page_a, Some(alpha_tombstone.clone()))],
            &[batch(0xb30), batch(0xb32)],
        )
        .unwrap();
        state.commit(losing_tombstone);
        assert_eq!(state, before_losing_tombstone);

        let created_beta = ephemeral_transition(
            &state,
            batch(0xb36),
            dot(36),
            &[page_delta(
                page_a,
                Some(alpha_tombstone.clone()),
                Some(beta.clone()),
            )],
            vec![(page_a, Some(alpha_tombstone.clone()))],
            vec![(page_a, Some(alpha_tombstone))],
            vec![(page_a, Some(beta.clone()))],
            &[batch(0xb33), batch(0xb34)],
        )
        .unwrap();
        state.commit(created_beta);

        let swapped = ephemeral_transition(
            &state,
            batch(0xb37),
            dot(37),
            &[
                page_delta(page_a, Some(beta.clone()), Some(alpha.clone())),
                page_delta(page_b, Some(alpha.clone()), Some(beta.clone())),
            ],
            vec![(page_a, Some(beta.clone())), (page_b, Some(alpha.clone()))],
            vec![(page_a, Some(beta.clone())), (page_b, Some(alpha.clone()))],
            vec![(page_a, Some(alpha)), (page_b, Some(beta))],
            &[batch(0xb34), batch(0xb36)],
        )
        .unwrap();
        assert!(swapped.conflicts.is_empty());
        state.commit(swapped);
        assert_eq!(
            state.records[&alpha_key].occupied().unwrap().page_id(),
            page_a
        );
        assert_eq!(
            state.records[&beta_key].occupied().unwrap().page_id(),
            page_b
        );
        assert_eq!(state.record_count(), 2);
    }

    #[test]
    fn concurrent_claims_and_divergent_renames_have_delivery_independent_evidence() {
        fn concurrent_claim_order(
            first_batch: BatchId,
            first_page: PageId,
            second_batch: BatchId,
            second_page: PageId,
        ) -> Vec<u8> {
            let (path, archive, index) = store("concurrent-claim-order");
            let name = live_state("Shared");
            let first = transition(
                &index,
                &PageNameOwnershipRootV1::empty(),
                first_batch,
                dot(first_batch.as_uuid().as_u128() as u64),
                &authenticated_page_points_for_test(vec![(first_page, None)]),
                &[page_delta(first_page, None, Some(name.clone()))],
                vec![(first_page, None)],
                vec![(first_page, Some(name.clone()))],
                &[],
            )
            .unwrap();
            let prior = first.root.external_digest().unwrap();
            let second = transition(
                &index,
                &first.root,
                second_batch,
                dot(second_batch.as_uuid().as_u128() as u64),
                &authenticated_page_points_for_test(vec![(second_page, None)]),
                &[page_delta(second_page, None, Some(name.clone()))],
                vec![(second_page, None)],
                vec![(second_page, Some(name))],
                &[],
            )
            .unwrap();
            assert_eq!(second.root.external_digest().unwrap(), prior);
            let encoded = second.conflicts[0].encode().unwrap();
            drop(index);
            drop(archive);
            fs::remove_dir_all(path).unwrap();
            encoded
        }

        let left = concurrent_claim_order(batch(0xc10), page(0xc01), batch(0xc20), page(0xc02));
        let right = concurrent_claim_order(batch(0xc20), page(0xc02), batch(0xc10), page(0xc01));
        assert_eq!(left, right);

        fn divergent_order(
            first_batch: BatchId,
            first_name: &str,
            second_batch: BatchId,
            second_name: &str,
            winner: &str,
        ) -> Vec<u8> {
            let (path, archive, index) = store("divergent-rename-order");
            let page_id = page(0xc30);
            let base = live_state("Base");
            let base_batch = batch(0xc31);
            let base_root = transition(
                &index,
                &PageNameOwnershipRootV1::empty(),
                base_batch,
                dot(31),
                &authenticated_page_points_for_test(vec![(page_id, None)]),
                &[page_delta(page_id, None, Some(base.clone()))],
                vec![(page_id, None)],
                vec![(page_id, Some(base.clone()))],
                &[],
            )
            .unwrap()
            .root;
            let first = live_state(first_name);
            let first_root = transition(
                &index,
                &base_root,
                first_batch,
                dot(first_batch.as_uuid().as_u128() as u64),
                &authenticated_page_points_for_test(vec![(page_id, Some(base.clone()))]),
                &[page_delta(page_id, Some(base.clone()), Some(first.clone()))],
                vec![(page_id, Some(base.clone()))],
                vec![(page_id, Some(first.clone()))],
                &[base_batch],
            )
            .unwrap()
            .root;
            let second = live_state(second_name);
            let conflict = transition(
                &index,
                &first_root,
                second_batch,
                dot(second_batch.as_uuid().as_u128() as u64),
                &authenticated_page_points_for_test(vec![(page_id, Some(base.clone()))]),
                &[page_delta(page_id, Some(base), Some(second))],
                vec![(page_id, Some(first))],
                vec![(page_id, Some(live_state(winner)))],
                &[base_batch],
            )
            .unwrap();
            let encoded = conflict.conflicts[0].encode().unwrap();
            drop(index);
            drop(archive);
            fs::remove_dir_all(path).unwrap();
            encoded
        }

        let left = divergent_order(
            batch(0xc40),
            "Branch A",
            batch(0xc50),
            "Branch B",
            "Branch A",
        );
        let right = divergent_order(
            batch(0xc50),
            "Branch B",
            batch(0xc40),
            "Branch A",
            "Branch A",
        );
        assert_eq!(left, right);
    }

    #[test]
    fn spelling_races_converge_losing_tombstone_does_not_release_and_names_ignore_paths() {
        let (path, archive, index) = store("spelling-and-paths");
        let page_id = page(0xd01);
        let original = live_state("Foo");
        let created = transition(
            &index,
            &PageNameOwnershipRootV1::empty(),
            batch(0xd10),
            dot(1),
            &authenticated_page_points_for_test(vec![(page_id, None)]),
            &[page_delta(page_id, None, Some(original.clone()))],
            vec![(page_id, None)],
            vec![(page_id, Some(original.clone()))],
            &[],
        )
        .unwrap()
        .root;
        let upper = live_state("FOO");
        let first = transition(
            &index,
            &created,
            batch(0xd11),
            dot(2),
            &authenticated_page_points_for_test(vec![(page_id, Some(original.clone()))]),
            &[page_delta(
                page_id,
                Some(original.clone()),
                Some(upper.clone()),
            )],
            vec![(page_id, Some(original.clone()))],
            vec![(page_id, Some(upper.clone()))],
            &[batch(0xd10)],
        )
        .unwrap();
        let lower = live_state("foo");
        let converged = transition(
            &index,
            &first.root,
            batch(0xd12),
            dot(3),
            &authenticated_page_points_for_test(vec![(page_id, Some(original.clone()))]),
            &[page_delta(
                page_id,
                Some(original.clone()),
                Some(lower.clone()),
            )],
            vec![(page_id, Some(upper))],
            vec![(page_id, Some(lower.clone()))],
            &[batch(0xd10)],
        )
        .unwrap();
        assert!(converged.conflicts.is_empty());
        let occupied = index
            .lookup(&converged.root, lower.name().key_digest())
            .unwrap()
            .unwrap();
        assert_eq!(
            occupied.occupied().unwrap().acquisition_batch(),
            batch(0xd10)
        );
        assert_eq!(
            index
                .read_exact_name(
                    lower.name().key_digest(),
                    occupied.occupied().unwrap().exact_name()
                )
                .unwrap(),
            LogicalPageName::parse("foo").unwrap()
        );
        let lower_first = transition(
            &index,
            &created,
            batch(0xd12),
            dot(3),
            &authenticated_page_points_for_test(vec![(page_id, Some(original.clone()))]),
            &[page_delta(
                page_id,
                Some(original.clone()),
                Some(lower.clone()),
            )],
            vec![(page_id, Some(original.clone()))],
            vec![(page_id, Some(lower.clone()))],
            &[batch(0xd10)],
        )
        .unwrap();
        let reverse = transition(
            &index,
            &lower_first.root,
            batch(0xd11),
            dot(2),
            &authenticated_page_points_for_test(vec![(page_id, Some(original.clone()))]),
            &[page_delta(
                page_id,
                Some(original.clone()),
                Some(live_state("FOO")),
            )],
            vec![(page_id, Some(lower.clone()))],
            vec![(page_id, Some(lower.clone()))],
            &[batch(0xd10)],
        )
        .unwrap();
        assert_eq!(reverse.root, converged.root);

        let tombstone = PageState::Tombstone {
            name: LogicalPageName::parse("Foo").unwrap(),
            home_document_id: original.home_document_id(),
            kind: ManagedTextKind::Page,
        };
        let losing_delete = transition(
            &index,
            &created,
            batch(0xd13),
            dot(4),
            &authenticated_page_points_for_test(vec![(page_id, Some(original.clone()))]),
            &[page_delta(page_id, Some(original.clone()), Some(tombstone))],
            vec![(page_id, Some(original.clone()))],
            vec![(page_id, Some(original))],
            &[batch(0xd10)],
        )
        .unwrap();
        assert_eq!(losing_delete.root, created);

        let nested_a = PageState::Live {
            name: LogicalPageName::parse("Area/One").unwrap(),
            path: ManagedPath::parse("pages/flat-one.md").unwrap(),
            home_document_id: document(0xd20),
            kind: ManagedTextKind::Page,
        };
        let nested_b = PageState::Live {
            name: LogicalPageName::parse("Area/Two").unwrap(),
            path: ManagedPath::parse("pages/deep/physical/two.md").unwrap(),
            home_document_id: document(0xd21),
            kind: ManagedTextKind::Page,
        };
        let nested = transition(
            &index,
            &PageNameOwnershipRootV1::empty(),
            batch(0xd22),
            dot(5),
            &authenticated_page_points_for_test(vec![(page(0xd20), None), (page(0xd21), None)]),
            &[
                page_delta(page(0xd20), None, Some(nested_a.clone())),
                page_delta(page(0xd21), None, Some(nested_b.clone())),
            ],
            vec![(page(0xd20), None), (page(0xd21), None)],
            vec![
                (page(0xd20), Some(nested_a.clone())),
                (page(0xd21), Some(nested_b.clone())),
            ],
            &[],
        )
        .unwrap();
        assert_ne!(nested_a.name().key_digest(), nested_b.name().key_digest());
        assert_eq!(nested.root.entry_count(), 2);

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn four_hundred_reuse_cycles_are_point_bounded_and_evidence_is_strict() {
        let (path, archive, index) = store("bounded-reuse-sequence");
        let name = live_state("Bounded");
        let mut root = PageNameOwnershipRootV1::empty();
        let before = index.stats();
        for sequence in 0..420_u128 {
            let page_id = page(0xe00 + sequence);
            let create_batch = batch(0x10_000 + sequence * 2);
            let delete_batch = batch(0x10_001 + sequence * 2);
            let prior_release = (sequence != 0)
                .then(|| batch(0x10_001 + (sequence - 1) * 2))
                .into_iter()
                .collect::<Vec<_>>();
            root = transition(
                &index,
                &root,
                create_batch,
                dot(sequence as u64 * 2 + 1),
                &authenticated_page_points_for_test(vec![(page_id, None)]),
                &[page_delta(page_id, None, Some(name.clone()))],
                vec![(page_id, None)],
                vec![(page_id, Some(name.clone()))],
                &prior_release,
            )
            .unwrap()
            .root;
            let tombstone = PageState::Tombstone {
                name: name.name().clone(),
                home_document_id: name.home_document_id(),
                kind: ManagedTextKind::Page,
            };
            root = transition(
                &index,
                &root,
                delete_batch,
                dot(sequence as u64 * 2 + 2),
                &authenticated_page_points_for_test(vec![(page_id, Some(name.clone()))]),
                &[page_delta(
                    page_id,
                    Some(name.clone()),
                    Some(tombstone.clone()),
                )],
                vec![(page_id, Some(name.clone()))],
                vec![(page_id, Some(tombstone))],
                &[create_batch],
            )
            .unwrap()
            .root;
        }
        let after = index.stats();
        let reads = after.reads - before.reads;
        let bytes = after.bytes_read - before.bytes_read;
        eprintln!("page-name transition point costs: cycles=420 reads={reads} bytes={bytes}");
        assert!(reads <= 420 * 2 * 64);
        assert!(bytes <= 420 * 2 * 64 * 1024);
        assert_eq!(root.entry_count(), 1);

        let evidence = PageNameConflictEvidenceV1::new(
            PageNameCollisionClassV1::DifferentPagesSameCanonicalKey,
            vec![
                PageNameConflictParticipantV1 {
                    page_id: page(1),
                    exact_name: LogicalPageName::parse("Evidence").unwrap(),
                    canonical_key: LogicalPageName::parse("Evidence").unwrap().key_digest(),
                    acquisition_batch: batch(1),
                    acquisition_dot: dot(1),
                    exact_state_batch: batch(1),
                    exact_state_dot: dot(1),
                    release_fence: None,
                    declared_frontier: empty_frontier(),
                },
                PageNameConflictParticipantV1 {
                    page_id: page(2),
                    exact_name: LogicalPageName::parse("evidence").unwrap(),
                    canonical_key: LogicalPageName::parse("evidence").unwrap().key_digest(),
                    acquisition_batch: batch(2),
                    acquisition_dot: dot(2),
                    exact_state_batch: batch(2),
                    exact_state_dot: dot(2),
                    release_fence: None,
                    declared_frontier: empty_frontier(),
                },
            ],
        )
        .unwrap();
        let encoded = evidence.encode().unwrap();
        assert_eq!(
            PageNameConflictEvidenceV1::decode(&encoded).unwrap(),
            evidence
        );
        let mut corrupt = encoded.clone();
        *corrupt.last_mut().unwrap() ^= 0x80;
        assert!(PageNameConflictEvidenceV1::decode(&corrupt).is_err());
        let mut prior = evidence.clone();
        prior.schema_version = 0;
        assert!(matches!(
            PageNameConflictEvidenceV1::decode(&encode_canonical(&prior).unwrap()),
            Err(StoreError::UpgradeRequired { .. })
        ));
        let mut future = evidence;
        future.schema_version = 2;
        assert!(matches!(
            PageNameConflictEvidenceV1::decode(&encode_canonical(&future).unwrap()),
            Err(StoreError::UnsupportedStoreVersion { .. })
        ));

        drop(index);
        drop(archive);
        fs::remove_dir_all(path).unwrap();
    }
}
