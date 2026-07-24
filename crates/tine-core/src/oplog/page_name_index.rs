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
    BatchCausalDot, BatchId, ContentDigest, DocumentCausalDigest, DocumentId, LogicalPageName,
    PageId, PageNameKeyDigest, PageState, PAGE_NAME_KEY_VERSION,
};

pub const EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION: u32 = 1;
pub const EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_STORE_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_RECORD_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_OWNERSHIP_ROOT_SCHEMA_VERSION: u32 = 1;
pub const PAGE_NAME_CATALOG_FRONTIER_SCHEMA_VERSION: u32 = 1;
pub const MAX_PAGE_NAME_POINT_BATCH: usize = 100_000;

const EXACT_NAME_BLOB_SUFFIX: &str = ".exact-page-name";
const MAX_EXACT_NAME_BLOB_BYTES: u64 = 4 * 1024 * 1024 + 1024;
const PAGE_NAME_INDEX_DOMAIN: &[u8] = b"tine/page-name-ownership-index/v1";
const STORE_CLAIM_FILE: &str = "page-name-index.claim";
const NODES_DIR: &str = "nodes";
const EXACT_NAMES_DIR: &str = "exact-names";

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
        let blob = ExactLogicalPageNameBlobV1 {
            schema_version: EXACT_LOGICAL_PAGE_NAME_BLOB_SCHEMA_VERSION,
            exact_name: name.clone(),
        };
        let bytes = encode_canonical(&blob)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_EXACT_NAME_BLOB_BYTES {
            return Err(StoreError::MalformedPageNameIndex);
        }
        let content_digest = ContentDigest::of(&bytes);
        publish_immutable_exact(
            &self.exact_names,
            &exact_name_blob_filename(content_digest),
            &bytes,
            "exact logical page-name blob",
        )?;
        Ok(ExactLogicalPageNameRefV1 {
            schema_version: EXACT_LOGICAL_PAGE_NAME_REF_SCHEMA_VERSION,
            encoded_len: bytes.len() as u64,
            content_digest,
            exact_name_digest: ExactLogicalPageNameDigest::of(name),
        })
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
        if ExactLogicalPageNameDigest::of(&blob.exact_name) != name_ref.exact_name_digest
            || blob.exact_name.key_digest() != expected_key
        {
            return Err(StoreError::MalformedPageNameIndex);
        }
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
}
