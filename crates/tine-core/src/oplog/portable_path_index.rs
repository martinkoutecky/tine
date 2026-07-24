#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::authenticated_patricia::{PatriciaIndexRoot, PatriciaIndexStats, PatriciaIndexStore};
use super::object_store::StoreError;
use super::{
    BatchCausalDot, BatchId, ContentDigest, ManagedPath, PageId, PortablePathKeyDigest,
    PORTABLE_PATH_KEY_VERSION,
};

const PORTABLE_PATH_RECORD_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PortablePathIndexRoot(PatriciaIndexRoot);

impl PortablePathIndexRoot {
    pub fn empty() -> Self {
        Self(PatriciaIndexRoot::empty())
    }

    pub const fn digest(self) -> ContentDigest {
        self.0.digest()
    }

    pub(crate) const fn from_digest(digest: ContentDigest) -> Self {
        Self(PatriciaIndexRoot::from_digest(digest))
    }
}

impl Default for PortablePathIndexRoot {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePathOccupied {
    page_id: PageId,
    exact_path: ManagedPath,
    exact_path_digest: ContentDigest,
    acquisition_batch: BatchId,
    causal_dot: BatchCausalDot,
}

impl PortablePathOccupied {
    pub fn new(
        page_id: PageId,
        exact_path: ManagedPath,
        acquisition_batch: BatchId,
        causal_dot: BatchCausalDot,
    ) -> Self {
        let exact_path_digest = exact_path_digest(&exact_path);
        Self {
            page_id,
            exact_path,
            exact_path_digest,
            acquisition_batch,
            causal_dot,
        }
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn exact_path(&self) -> &ManagedPath {
        &self.exact_path
    }

    pub const fn exact_path_digest(&self) -> ContentDigest {
        self.exact_path_digest
    }

    pub const fn acquisition_batch(&self) -> BatchId {
        self.acquisition_batch
    }

    pub const fn causal_dot(&self) -> BatchCausalDot {
        self.causal_dot
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePathReleased {
    prior_page_id: PageId,
    prior_exact_path: ManagedPath,
    prior_acquisition_batch: BatchId,
    release_batch: BatchId,
    causal_dot: BatchCausalDot,
}

impl PortablePathReleased {
    pub const fn new(
        prior_page_id: PageId,
        prior_exact_path: ManagedPath,
        prior_acquisition_batch: BatchId,
        release_batch: BatchId,
        causal_dot: BatchCausalDot,
    ) -> Self {
        Self {
            prior_page_id,
            prior_exact_path,
            prior_acquisition_batch,
            release_batch,
            causal_dot,
        }
    }

    pub const fn prior_page_id(&self) -> PageId {
        self.prior_page_id
    }

    pub const fn release_batch(&self) -> BatchId {
        self.release_batch
    }

    pub fn prior_exact_path(&self) -> &ManagedPath {
        &self.prior_exact_path
    }

    pub const fn prior_acquisition_batch(&self) -> BatchId {
        self.prior_acquisition_batch
    }

    pub const fn causal_dot(&self) -> BatchCausalDot {
        self.causal_dot
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePathRecord {
    schema_version: u32,
    key_version: u32,
    key_digest: PortablePathKeyDigest,
    occupied: Option<PortablePathOccupied>,
    latest_release: Option<PortablePathReleased>,
}

impl PortablePathRecord {
    pub fn new(
        key_digest: PortablePathKeyDigest,
        occupied: Option<PortablePathOccupied>,
        latest_release: Option<PortablePathReleased>,
    ) -> Result<Self, StoreError> {
        let record = Self {
            schema_version: PORTABLE_PATH_RECORD_SCHEMA_VERSION,
            key_version: PORTABLE_PATH_KEY_VERSION,
            key_digest,
            occupied,
            latest_release,
        };
        record.validate(key_digest)?;
        Ok(record)
    }

    pub const fn key_digest(&self) -> PortablePathKeyDigest {
        self.key_digest
    }

    pub const fn occupied(&self) -> Option<&PortablePathOccupied> {
        self.occupied.as_ref()
    }

    pub const fn latest_release(&self) -> Option<&PortablePathReleased> {
        self.latest_release.as_ref()
    }

    fn validate(&self, expected: PortablePathKeyDigest) -> Result<(), StoreError> {
        if self.schema_version != PORTABLE_PATH_RECORD_SCHEMA_VERSION
            || self.key_version != PORTABLE_PATH_KEY_VERSION
            || self.key_digest != expected
            || self.occupied.as_ref().is_some_and(|occupied| {
                occupied.exact_path.portable_key().digest() != expected
                    || exact_path_digest(&occupied.exact_path) != occupied.exact_path_digest
            })
            || self
                .latest_release
                .as_ref()
                .is_some_and(|release| release.prior_exact_path.portable_key().digest() != expected)
        {
            return Err(StoreError::MalformedLogseqClaimIndex);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PortablePathIndexStore {
    patricia: PatriciaIndexStore,
}

impl PortablePathIndexStore {
    pub(crate) fn new(patricia: PatriciaIndexStore) -> Self {
        Self { patricia }
    }

    pub(crate) fn stats(&self) -> PatriciaIndexStats {
        self.patricia.stats()
    }

    pub(crate) fn validate_root(&self, root: PortablePathIndexRoot) -> Result<(), StoreError> {
        self.patricia.validate_root(root.0)
    }

    pub(crate) fn lookup(
        &self,
        root: PortablePathIndexRoot,
        key: PortablePathKeyDigest,
    ) -> Result<Option<PortablePathRecord>, StoreError> {
        self.patricia
            .lookup(root.0, key.as_bytes())?
            .map(|bytes| decode_record(key, &bytes))
            .transpose()
    }

    pub(crate) fn lookup_many(
        &self,
        root: PortablePathIndexRoot,
        keys: &[PortablePathKeyDigest],
    ) -> Result<BTreeMap<PortablePathKeyDigest, PortablePathRecord>, StoreError> {
        keys.iter()
            .filter_map(|key| {
                self.lookup(root, *key)
                    .transpose()
                    .map(|result| result.map(|record| (*key, record)))
            })
            .collect()
    }

    pub(crate) fn insert_many(
        &self,
        root: PortablePathIndexRoot,
        records: &BTreeMap<PortablePathKeyDigest, PortablePathRecord>,
    ) -> Result<PortablePathIndexRoot, StoreError> {
        let encoded = records
            .iter()
            .map(|(key, record)| {
                record.validate(*key)?;
                Ok((key.as_bytes().to_vec(), encode_record(record)?))
            })
            .collect::<Result<BTreeMap<_, _>, StoreError>>()?;
        self.patricia
            .insert_many(root.0, &encoded)
            .map(PortablePathIndexRoot)
    }
}

fn exact_path_digest(path: &ManagedPath) -> ContentDigest {
    let mut bytes = b"tine/exact-managed-path/v1\0".to_vec();
    bytes.extend_from_slice(&(path.as_str().len() as u64).to_be_bytes());
    bytes.extend_from_slice(path.as_str().as_bytes());
    ContentDigest::of(&bytes)
}

fn encode_record(record: &PortablePathRecord) -> Result<Vec<u8>, StoreError> {
    postcard::to_allocvec(record).map_err(|_| StoreError::MalformedLogseqClaimIndex)
}

fn decode_record(
    expected: PortablePathKeyDigest,
    bytes: &[u8],
) -> Result<PortablePathRecord, StoreError> {
    let record: PortablePathRecord =
        postcard::from_bytes(bytes).map_err(|_| StoreError::MalformedLogseqClaimIndex)?;
    record.validate(expected)?;
    if encode_record(&record)? != bytes {
        return Err(StoreError::MalformedLogseqClaimIndex);
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cap_std::{ambient_authority, fs::Dir};
    use uuid::Uuid;

    use super::*;
    use crate::oplog::object_store::{ensure_directory_nofollow, open_dir_nofollow};
    use crate::oplog::{CausalPeerId, DeviceId};

    fn store(name: &str) -> (std::path::PathBuf, PortablePathIndexStore) {
        let path =
            std::env::temp_dir().join(format!("tine-portable-index-{name}-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        let root = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();
        ensure_directory_nofollow(&root, "nodes").unwrap();
        let nodes = open_dir_nofollow(&root, "nodes").unwrap();
        (
            path,
            PortablePathIndexStore::new(PatriciaIndexStore::new(nodes)),
        )
    }

    fn dot() -> BatchCausalDot {
        BatchCausalDot::new(
            CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(1))),
            1,
        )
        .unwrap()
    }

    fn dot_at(counter: u64) -> BatchCausalDot {
        BatchCausalDot::new(
            CausalPeerId::from_device_id(DeviceId::from_uuid(Uuid::from_u128(1))),
            counter,
        )
        .unwrap()
    }

    #[test]
    fn authenticated_node_tamper_fails_closed() {
        let (path, store) = store("tamper");
        let exact_path = ManagedPath::parse("pages/Foo.md").unwrap();
        let key = exact_path.portable_key().digest();
        let record = PortablePathRecord::new(
            key,
            Some(PortablePathOccupied::new(
                PageId::from_uuid(Uuid::from_u128(2)),
                exact_path,
                BatchId::from_uuid(Uuid::from_u128(3)),
                dot(),
            )),
            None,
        )
        .unwrap();
        let root = store
            .insert_many(
                PortablePathIndexRoot::empty(),
                &BTreeMap::from([(key, record)]),
            )
            .unwrap();
        let node_path = path
            .join("nodes")
            .join(format!("{}.patricia-node", root.digest()));
        let mut bytes = fs::read(&node_path).unwrap();
        bytes[0] ^= 0x01;
        fs::write(node_path, bytes).unwrap();

        assert!(store.lookup(root, key).is_err());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn authenticated_but_semantically_misbound_record_fails_closed() {
        let (path, store) = store("misbound");
        let indexed_path = ManagedPath::parse("pages/Foo.md").unwrap();
        let wrong_path = ManagedPath::parse("pages/Bar.md").unwrap();
        let key = indexed_path.portable_key().digest();
        let invalid = PortablePathRecord {
            schema_version: PORTABLE_PATH_RECORD_SCHEMA_VERSION,
            key_version: PORTABLE_PATH_KEY_VERSION,
            key_digest: key,
            occupied: Some(PortablePathOccupied {
                page_id: PageId::from_uuid(Uuid::from_u128(4)),
                exact_path_digest: exact_path_digest(&wrong_path),
                exact_path: wrong_path,
                acquisition_batch: BatchId::from_uuid(Uuid::from_u128(5)),
                causal_dot: dot(),
            }),
            latest_release: None,
        };
        let bytes = postcard::to_allocvec(&invalid).unwrap();
        let raw_root = store
            .patricia
            .insert_many(
                PatriciaIndexRoot::empty(),
                &BTreeMap::from([(key.as_bytes().to_vec(), bytes)]),
            )
            .unwrap();
        let root = PortablePathIndexRoot(raw_root);

        assert!(store.lookup(root, key).is_err());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn four_hundred_reuses_keep_values_and_structural_work_bounded() {
        let (path, store) = store("long-reuse");
        let exact_path = ManagedPath::parse("pages/Reused.md").unwrap();
        let key = exact_path.portable_key().digest();
        let pages = [
            PageId::from_uuid(Uuid::from_u128(20)),
            PageId::from_uuid(Uuid::from_u128(21)),
        ];
        let mut root = PortablePathIndexRoot::empty();
        let mut prior_page = pages[0];
        let mut prior_acquisition = BatchId::from_uuid(Uuid::from_u128(30));
        let mut samples = Vec::new();

        for iteration in 1_u128..=400 {
            let page = pages[iteration as usize % pages.len()];
            let acquisition = BatchId::from_uuid(Uuid::from_u128(1_000 + iteration));
            let release = BatchId::from_uuid(Uuid::from_u128(2_000 + iteration));
            let record = PortablePathRecord::new(
                key,
                Some(PortablePathOccupied::new(
                    page,
                    exact_path.clone(),
                    acquisition,
                    dot_at(iteration as u64),
                )),
                Some(PortablePathReleased::new(
                    prior_page,
                    exact_path.clone(),
                    prior_acquisition,
                    release,
                    dot_at(iteration as u64),
                )),
            )
            .unwrap();
            assert!(
                encode_record(&record).unwrap().len() < 4 * 1024,
                "portable value grew with reuse history at generation {iteration}"
            );
            root = store
                .insert_many(root, &BTreeMap::from([(key, record)]))
                .unwrap();
            prior_page = page;
            prior_acquisition = acquisition;
            if matches!(iteration, 100 | 200 | 400) {
                samples.push((iteration, store.stats()));
            }
        }

        assert_eq!(
            samples.iter().map(|sample| sample.0).collect::<Vec<_>>(),
            [100, 200, 400]
        );
        let first_bytes = samples[0].1.bytes_written;
        let second_bytes = samples[1]
            .1
            .bytes_written
            .saturating_sub(samples[0].1.bytes_written);
        let next_two_hundred_bytes = samples[2]
            .1
            .bytes_written
            .saturating_sub(samples[1].1.bytes_written);
        assert!(
            second_bytes <= first_bytes.saturating_add(first_bytes / 8),
            "the second 100 point updates copied history: {first_bytes} then {second_bytes}"
        );
        assert!(
            next_two_hundred_bytes
                <= second_bytes
                    .saturating_mul(2)
                    .saturating_add(second_bytes / 4),
            "point-update bytes grew with path lifetime: {second_bytes} then {next_two_hundred_bytes}"
        );
        assert_eq!(samples[0].1.writes, 100);
        assert_eq!(samples[1].1.writes - samples[0].1.writes, 100);
        assert_eq!(samples[2].1.writes - samples[1].1.writes, 200);
        let current = store.lookup(root, key).unwrap().unwrap();
        assert_eq!(
            current.latest_release().unwrap().release_batch(),
            BatchId::from_uuid(Uuid::from_u128(2_400))
        );
        fs::remove_dir_all(path).unwrap();
    }
}
