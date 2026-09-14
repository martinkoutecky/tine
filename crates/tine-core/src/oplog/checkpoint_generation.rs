//! Adapter between Tine's one current accepted-evidence format and the shared
//! sealed accepted-history index.
//!
//! A5 completes the former R1a reader-only boundary with one disposable
//! generation publisher. Managed Storage is pre-0.7, so this module still
//! contains no legacy decoder, version dispatch, or migration bridge.

#[path = "sealed_document_map.rs"]
mod sealed_document_map;

use sealed_document_map::SealedDocumentMap;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::hot_engine::{
    AcceptedBatchEvidence, AcceptedFrontierRoot, CleanCheckpointAcceptedRow,
    CleanCheckpointCapture, CompactAcceptedDocument, PolicyCompactAcceptedDocument,
    ACCEPTED_EVIDENCE_SCHEMA_VERSION,
};
use super::object_store::ObjectStore;
use super::{
    BatchCausalDot, BatchId, BlobDescription, CausalPeerId, ContentDigest, CrdtPeerCounter,
    DocumentDependencies, DocumentId, LineageDigest, WorkspaceId, WriterIncarnationId,
};
use crate::sync_runtime::{
    SyncCheckpointDocumentDiagnostics, SyncCheckpointLimitingCause,
    SyncCheckpointPublicationDiagnostics, SyncCheckpointPublicationEdge,
};
use tine_storage::sealed_accepted_index::AuthenticatedMapKey;

const CHECKPOINT_SCHEMA_VERSION: u32 = 5;
/// The checkpoint slot pointer and the sealed tables share ONE directory.
///
/// P4c2 §4.4: `clean-open-checkpoint-v2/` and `cold-history-v1/` merge into a
/// single `sealed-v3/` with one marker, one root and one pack set, so a
/// generation is made durable by one marker install rather than by two
/// directories that can disagree after a crash (I-2).
use super::cold_object_store::SEALED_DIRECTORY as CHECKPOINT_DIRECTORY;
/// The checkpoint slot pointer, inside the merged `sealed-v3/` directory.
///
/// It is NOT `current`: `cold_object_store::SEALED_ROOT_MARKER` owns that name
/// for the sealed marker, and P4c2 §4.4 merges the two directories into one.
/// Two files called `current` in one directory would have collided silently,
/// each publication clobbering the other's pointer.
const CHECKPOINT_POINTER: &str = "checkpoint";
const CHECKPOINT_PAYLOAD_NAMES: [&str; 2] = ["payload-a", "payload-b"];
const CHECKPOINT_GENERATION_NAMES: [&str; 2] = ["generation-a", "generation-b"];
const MAX_CHECKPOINT_BYTES: u64 = 512 * 1024 * 1024;
const CHECKPOINT_CLEANUP_ENTRY_HEADROOM: usize = 1024;

pub(crate) const CLEAN_CHECKPOINT_LAG_MAX: u64 = 64;

#[cfg(test)]
static FAIL_CHECKPOINT_WRITE_ROOTS: Mutex<BTreeSet<std::path::PathBuf>> =
    Mutex::new(BTreeSet::new());

#[cfg(test)]
pub(crate) fn fail_checkpoint_writes_for_test(store_root: &std::path::Path, fail: bool) {
    let root = std::fs::canonicalize(store_root)
        .expect("the checkpoint failure fixture has opened its archive root");
    let mut roots = FAIL_CHECKPOINT_WRITE_ROOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if fail {
        roots.insert(root);
    } else {
        roots.remove(&root);
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum CheckpointDamageForTest {
    PointerTorn,
    PointerWrongFormat,
    GenerationTorn,
    PayloadTorn,
    FloorMetadataInconsistent,
    DocumentImageTorn,
}

#[cfg(test)]
pub(crate) fn damage_checkpoint_for_test(
    store: &ObjectStore,
    damage: CheckpointDamageForTest,
) -> Result<(), String> {
    fn overwrite(path: &std::path::Path, mut bytes: &[u8]) -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        std::io::copy(&mut bytes, &mut file)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    let path = store.root_path().join(CHECKPOINT_DIRECTORY);
    let pointer_path = path.join(CHECKPOINT_POINTER);
    let pointer_bytes = std::fs::read(&pointer_path).map_err(|error| error.to_string())?;
    if matches!(damage, CheckpointDamageForTest::PointerTorn) {
        overwrite(&pointer_path, &[0x01])?;
        return Ok(());
    }
    let mut pointer: CheckpointPointerV2 = decode_canonical(&pointer_bytes)?;
    if matches!(damage, CheckpointDamageForTest::PointerWrongFormat) {
        pointer.schema_version = pointer.schema_version.saturating_add(1);
        overwrite(&pointer_path, &encode_canonical(&pointer)?)?;
        return Ok(());
    }
    let slot = pointer.slot as usize;
    let generation_path = path.join(CHECKPOINT_GENERATION_NAMES[slot]);
    if matches!(damage, CheckpointDamageForTest::GenerationTorn) {
        overwrite(&generation_path, &[0x01])?;
        return Ok(());
    }
    let generation_bytes = std::fs::read(&generation_path).map_err(|error| error.to_string())?;
    let mut generation: CheckpointGenerationV2 = decode_canonical(&generation_bytes)?;
    let payload_path = path.join(CHECKPOINT_PAYLOAD_NAMES[slot]);
    if matches!(damage, CheckpointDamageForTest::PayloadTorn) {
        overwrite(&payload_path, &[0x01])?;
        return Ok(());
    }
    let payload_bytes = std::fs::read(&payload_path).map_err(|error| error.to_string())?;
    let mut payload: CheckpointPayloadV2 = decode_canonical(&payload_bytes)?;
    if matches!(damage, CheckpointDamageForTest::FloorMetadataInconsistent) {
        payload.recovery_fence.eligible_through = generation.sequence.saturating_add(1);
        let payload_bytes = encode_canonical(&payload)?;
        generation.payload_len = payload_bytes.len() as u64;
        generation.payload_digest = ContentDigest::of(&payload_bytes);
        let generation_bytes = encode_canonical(&generation)?;
        pointer.generation_digest = ContentDigest::of(&generation_bytes);
        overwrite(&payload_path, &payload_bytes)?;
        overwrite(&generation_path, &generation_bytes)?;
        overwrite(&pointer_path, &encode_canonical(&pointer)?)?;
        return Ok(());
    }
    if matches!(damage, CheckpointDamageForTest::DocumentImageTorn) {
        let document = payload
            .document_dependencies
            .first()
            .ok_or_else(|| "checkpoint damage fixture has no document image".to_owned())?
            .document_id();
        let directory = checkpoint_directory(store)?;
        let roster = SealedDocumentRoster::with_count(payload.document_count);
        let reader = SealedGenerationDirectory::open_generation(
            &directory,
            payload.sealed_root.locator(),
            payload.sealed_root_digest,
        )?;
        let record = roster
            .document_record(&reader, document)?
            .ok_or_else(|| "checkpoint damage fixture omits its document".to_owned())?;
        // The image is a packed record, not a file: tear exactly its payload
        // range inside the pack that holds it.
        let digest = ContentDigest::from_bytes(*record.checkpoint.sha256());
        let value = reader
            .table_value(DOMAIN_CAPSULE_BLOB, digest.as_bytes())?
            .ok_or_else(|| "checkpoint damage fixture omits its image locator".to_owned())?;
        reader.tear_packed_record(locator_from_value(&value)?)?;
        return Ok(());
    }
    Err("unsupported checkpoint damage fixture".into())
}

pub(crate) struct TineAcceptedEvidenceDecoder;

impl tine_storage::sealed_accepted_index::SealedAcceptedEvidenceDecoder
    for TineAcceptedEvidenceDecoder
{
    fn decode_accepted_evidence(
        &self,
        evidence_schema: u32,
        exact_evidence_bytes: &[u8],
    ) -> Result<
        tine_storage::sealed_accepted_index::AcceptedEvidenceBindingV2,
        tine_storage::sealed_accepted_index::SealedAcceptedIndexError,
    > {
        use tine_storage::sealed_accepted_index::SealedAcceptedIndexError;

        if evidence_schema != ACCEPTED_EVIDENCE_SCHEMA_VERSION {
            return Err(SealedAcceptedIndexError::Corrupt(format!(
                "accepted-status evidence schema {evidence_schema} != current schema {ACCEPTED_EVIDENCE_SCHEMA_VERSION}"
            )));
        }
        let evidence = AcceptedBatchEvidence::decode_canonical(exact_evidence_bytes)
            .map_err(|error| SealedAcceptedIndexError::Corrupt(error.to_string()))?;
        Ok(
            tine_storage::sealed_accepted_index::AcceptedEvidenceBindingV2 {
                batch_id: evidence.batch_id().as_uuid().into_bytes(),
                manifest_fingerprint: evidence.manifest_fingerprint(),
                event_binding_digest: evidence.event_binding_digest(),
                acceptance_sequence: evidence.acceptance_sequence(),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// The sealed store seam, over one sealed-v3 archive
// ---------------------------------------------------------------------------
//
// There is exactly one sealed index format (D-1): immutable sorted tables
// inside packs, under one root record and one `current` marker, all in
// `sealed-v3/`. This module no longer owns an object store at all — it owns
// the *questions* it asks of one, and `cold_object_store` owns the container.
//
// Two access shapes exist and no more:
//
//   * [`SealedGenerationDirectory`] — read-only point access at one exact
//     root. A completed generation, or an older generation's rollback slot.
//   * [`SealedGenerationStagingStore`] — one open [`SealedCut`]: staged domain
//     entries and appended records that become durable, marker last, when
//     `finish` publishes them (I-2).
//
// Both answer through [`SealedTableAccess`], which is why no generic bound in
// this file names a node seam any more.

use super::cold_object_store::{
    framed_document_key, framed_identity_key, identity_complete_domain, identity_current_domain,
    unframed_identity_key, ColdLocatorV1, SealedArchiveReader, SealedCut, SealedCutKill,
    SealedCutWork, DOMAIN_BATCH, DOMAIN_CAPSULE_BLOB, DOMAIN_CAUSAL_TIP, DOMAIN_COVERED_OBJECT,
    DOMAIN_DOCUMENT_CHANGE, DOMAIN_DOCUMENT_ROSTER, DOMAIN_SEQUENCE,
};
use tine_storage::sealed_tables::TableDomain;

/// The two point questions and the two enumerations every sealed consumer in
/// this file asks. Reading is identical whether a cut is open or not; only a
/// staging store may also write.
pub(crate) trait SealedTableAccess {
    fn table_value(&self, domain: TableDomain, key: &[u8]) -> Result<Option<Vec<u8>>, String>;

    fn table_predecessor(
        &self,
        domain: TableDomain,
        key: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, String>;

    /// One variable-length record's exact bytes.
    fn sealed_record(&self, locator: ColdLocatorV1, limit: u64) -> Result<Vec<u8>, String>;

    /// Every live entry of one domain. Enumeration consumers only.
    fn table_entries(&self, domain: TableDomain) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String>;

    /// Table records read so far. The unit the index-work counters are in.
    fn index_reads(&self) -> u64;

    /// Read back one content-addressed capsule blob and prove the bytes are
    /// the ones the descriptor names.
    ///
    /// Blobs are content-addressed in `DOMAIN_CAPSULE_BLOB`, so this is one
    /// table lookup plus one packed record read (I-14: O(1) file opens per
    /// point read), never a directory scan.
    fn read_capsule_blob(&self, blob: BlobDescription) -> Result<Vec<u8>, String> {
        let digest = ContentDigest::from_bytes(*blob.sha256());
        let Some(value) = self.table_value(DOMAIN_CAPSULE_BLOB, digest.as_bytes())? else {
            return Err("sealed capsule blob is absent from its index".into());
        };
        let bytes = self.sealed_record(locator_from_value(&value)?, MAX_CHECKPOINT_BYTES)?;
        if BlobDescription::of(&bytes) != blob {
            return Err("sealed capsule blob does not match its description".into());
        }
        Ok(bytes)
    }
}

/// Domains whose live entry count is bounded by the DOCUMENT count, and whose
/// entries this reader therefore materializes once instead of re-reading a
/// table per point read.
///
/// WHY this list exists: `tine_storage` verifies a table's digest exactly once
/// per `TableSetReader`, and Tine builds a fresh reader for every point read.
/// That is free for a handful of lookups and quadratic for a loop -- and both
/// checkpoint qualification and checkpoint restore iterate EVERY document,
/// doing two point reads each. On the anonymized graph (1,077 documents,
/// H=1000, 2026-09-14) that hashed the roster and blob tables ~3,000 times and
/// made a reopen cost 28.4 s where the treap cost 0.78 s.
///
/// WHY it is these two and no others: their live entry count is the document
/// count -- the same count the caller has already materialized in
/// `payload.document_dependencies`, so the memory is proportionate to work it
/// is already doing. Batch-scale domains (`DOMAIN_BATCH`, `DOMAIN_SEQUENCE`,
/// `DOMAIN_COVERED_OBJECT`, `DOMAIN_CAUSAL_TIP`) are deliberately absent:
/// caching those would hold O(accepted batches) of index in RAM, which is the
/// cost sorted tables exist to avoid. A loop over batch-scale keys is a
/// different defect and must be fixed by not looping, not by caching.
const DOCUMENT_SCALE_CACHED_DOMAINS: [u8; 2] = [DOMAIN_DOCUMENT_ROSTER.id, DOMAIN_CAPSULE_BLOB.id];

/// Read-only point access to exact sealed state. This carries no generation
/// authority: only a later qualified generation commit can name its root.
pub(crate) struct SealedGenerationDirectory {
    directory: cap_std::fs::Dir,
    archive: Option<SealedArchiveReader>,
    /// Lazily materialized live entries of `DOCUMENT_SCALE_CACHED_DOMAINS`.
    /// Immutable: this reader is open at one exact root record.
    document_scale_entries: std::sync::RwLock<BTreeMap<u8, Arc<BTreeMap<Vec<u8>, Vec<u8>>>>>,
}

impl SealedGenerationDirectory {
    fn cached_domain_entries(
        &self,
        domain: TableDomain,
    ) -> Result<Arc<BTreeMap<Vec<u8>, Vec<u8>>>, String> {
        if let Some(entries) = self
            .document_scale_entries
            .read()
            .map_err(|_| "sealed document-scale cache is poisoned".to_owned())?
            .get(&domain.id)
        {
            return Ok(Arc::clone(entries));
        }
        let entries = Arc::new(match self.archive.as_ref() {
            Some(archive) => archive.domain_entries(domain)?,
            None => BTreeMap::new(),
        });
        self.document_scale_entries
            .write()
            .map_err(|_| "sealed document-scale cache is poisoned".to_owned())?
            .insert(domain.id, Arc::clone(&entries));
        Ok(entries)
    }
}

impl SealedGenerationDirectory {
    /// Open at the archive's current marker.
    pub(crate) fn open(directory: &cap_std::fs::Dir) -> Result<Self, String> {
        Ok(Self {
            directory: directory.try_clone().map_err(|error| error.to_string())?,
            archive: SealedArchiveReader::open_directory(directory)
                .map_err(|error| error.to_string())?,
            document_scale_entries: std::sync::RwLock::new(BTreeMap::new()),
        })
    }

    /// Open at one exact historical root record.
    ///
    /// The marker's pack table resolves any locator ever published, so an
    /// earlier generation keeps resolving after the marker advances. That is
    /// what preserves the two-slot checkpoint protocol's rollback slot.
    fn open_at_unchecked(
        directory: &cap_std::fs::Dir,
        root: ColdLocatorV1,
    ) -> Result<Self, String> {
        Ok(Self {
            directory: directory.try_clone().map_err(|error| error.to_string())?,
            archive: SealedArchiveReader::open_directory_at(directory, root)
                .map_err(|error| error.to_string())?,
            document_scale_entries: std::sync::RwLock::new(BTreeMap::new()),
        })
    }

    /// Reopen at the exact sealed root a generation binding names, and prove
    /// the root record found there IS that root.
    ///
    /// This is the ONLY way to open a reader on a generation's behalf. The
    /// check lives inside the constructor, not at its call sites, because it
    /// replaces the authenticated cross-check the treap used to give for free
    /// (D-2 removed the proof walk, not the binding): the generation's
    /// `sealed_root_digest` is the one value naming its covered half, and a
    /// caller that forgot to compare it would consume a DIFFERENT history
    /// while believing it had the generation's own.
    ///
    /// Refusal scenario: a torn or partially delivered sealed index -- the
    /// marker, its root record and its packs arriving out of step through a
    /// crash, a truncated write, or a sync service. The remedy is D-3 rebuild,
    /// never a graph-open refusal: the caller discards this generation and the
    /// store rebuilds from the untouched Markdown/Org originals and repairs
    /// the tables from the pack footers.
    pub(crate) fn open_generation(
        directory: &cap_std::fs::Dir,
        root: ColdLocatorV1,
        expected: ContentDigest,
    ) -> Result<Self, String> {
        let opened = Self::open_at_unchecked(directory, root)?;
        let actual = opened.sealed_root_digest()?;
        if actual != expected {
            return Err(format!(
                "sealed root record at offset {} does not match the generation binding \
                 (found {actual}, bound {expected}); rebuild this generation",
                root.offset
            ));
        }
        Ok(opened)
    }

    /// Tear one packed record's payload in place. Damage fixtures only.
    #[cfg(test)]
    pub(crate) fn tear_packed_record(&self, locator: ColdLocatorV1) -> Result<(), String> {
        super::cold_object_store::tear_packed_record_for_test(&self.directory, locator)
    }

    /// The digest of the sealed root record this reader is open at.
    ///
    /// A reader that has never published sealed state is at the empty root;
    /// a root record that cannot be digested is an error, never silently the
    /// empty root -- that would let a damaged root pass as a legitimate empty
    /// history.
    pub(crate) fn sealed_root_digest(&self) -> Result<ContentDigest, String> {
        match self.archive.as_ref() {
            None => Ok(tine_storage::sealed_tables::sealed_empty_root_digest()),
            Some(archive) => archive
                .root()
                .table_root_digest()
                .map_err(|error| error.to_string()),
        }
    }

    pub(crate) fn directory(&self) -> &cap_std::fs::Dir {
        &self.directory
    }
}

impl SealedTableAccess for SealedGenerationDirectory {
    fn table_value(&self, domain: TableDomain, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if DOCUMENT_SCALE_CACHED_DOMAINS.contains(&domain.id) {
            return Ok(self.cached_domain_entries(domain)?.get(key).cloned());
        }
        let Some(archive) = self.archive.as_ref() else {
            return Ok(None);
        };
        archive
            .tables(domain)?
            .get(key)
            .map_err(|error| error.to_string())
    }

    fn table_predecessor(
        &self,
        domain: TableDomain,
        key: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
        if DOCUMENT_SCALE_CACHED_DOMAINS.contains(&domain.id) {
            // The cached path must answer exactly what the table path answers,
            // or a cached domain would quietly have two semantics.
            return Ok(self
                .cached_domain_entries(domain)?
                .range::<[u8], _>((std::ops::Bound::Unbounded, std::ops::Bound::Excluded(key)))
                .next_back()
                .map(|(key, value)| (key.clone(), value.clone())));
        }
        let Some(archive) = self.archive.as_ref() else {
            return Ok(None);
        };
        archive
            .tables(domain)?
            .predecessor(key)
            .map_err(|error| error.to_string())
    }

    fn sealed_record(&self, locator: ColdLocatorV1, limit: u64) -> Result<Vec<u8>, String> {
        self.archive
            .as_ref()
            .ok_or("sealed archive has published no record")?
            .record_bytes(locator, limit)
    }

    fn table_entries(&self, domain: TableDomain) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        if DOCUMENT_SCALE_CACHED_DOMAINS.contains(&domain.id) {
            return Ok((*self.cached_domain_entries(domain)?).clone());
        }
        match self.archive.as_ref() {
            Some(archive) => archive.domain_entries(domain),
            None => Ok(BTreeMap::new()),
        }
    }

    fn index_reads(&self) -> u64 {
        self.archive
            .as_ref()
            .map_or(0, |archive| archive.work().index_nodes as u64)
    }
}

/// One open sealed cut.
///
/// Staged entries and appended records are unreferenced residue until
/// `finish` installs the marker; a crash before that leaves the predecessor
/// root exactly as authoritative as it was (I-2, P4c2 T5). Drop abandons the
/// cut without moving anything.
pub(crate) struct SealedGenerationStagingStore {
    directory: cap_std::fs::Dir,
    cut: SealedCut,
    /// Domain entries written by this cut. Counted for I-25, not for policy.
    entry_writes: u64,
    record_writes: u64,
}

impl SealedGenerationStagingStore {
    pub(crate) fn open(directory: &cap_std::fs::Dir) -> Result<Self, String> {
        Ok(Self {
            directory: directory.try_clone().map_err(|error| error.to_string())?,
            cut: SealedCut::open(directory).map_err(|error| error.to_string())?,
            entry_writes: 0,
            record_writes: 0,
        })
    }

    pub(crate) fn put(
        &mut self,
        domain: TableDomain,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), String> {
        self.entry_writes = self.entry_writes.saturating_add(1);
        self.cut.put(domain, key, value)
    }

    pub(crate) fn remove(&mut self, domain: TableDomain, key: &[u8]) -> Result<(), String> {
        self.entry_writes = self.entry_writes.saturating_add(1);
        self.cut.remove(domain, key)
    }

    /// Append one content-addressed variable-length record and return where it
    /// landed. Callers store the locator as a table value.
    pub(crate) fn append_record(&mut self, bytes: &[u8]) -> Result<ColdLocatorV1, String> {
        self.record_writes = self.record_writes.saturating_add(1);
        self.cut
            .add_sealed_record(bytes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn entry_writes(&self) -> u64 {
        self.entry_writes
    }

    pub(crate) fn record_writes(&self) -> u64 {
        self.record_writes
    }

    /// Publish up to one exact protocol boundary and stop. Crash fixtures only.
    #[cfg(test)]
    pub(crate) fn finish_killed_at(self, kill: SealedCutKill) -> Result<SealedCutWork, String> {
        self.cut
            .publish_with_kill(Some(kill))
            .map_err(|error| error.to_string())
    }

    /// Publish this cut and reopen read-only at the root it installed.
    pub(crate) fn finish(self) -> Result<(SealedCutWork, SealedGenerationDirectory), String> {
        let directory = self.directory;
        let work = self.cut.publish().map_err(|error| error.to_string())?;
        let reader = SealedGenerationDirectory::open_generation(
            &directory,
            work.root_locator,
            work.root_digest,
        )?;
        Ok((work, reader))
    }

    /// Publish one document image and return its content description. The
    /// same blob published by an earlier cut is reused, not re-appended.
    fn stage_capsule_blob(
        &mut self,
        bytes: &[u8],
    ) -> Result<(BlobDescription, ColdLocatorV1), String> {
        let blob = BlobDescription::of(bytes);
        if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
            return Err(
                "sealed construction record exceeds the current checkpoint record limit".into(),
            );
        }
        let digest = ContentDigest::from_bytes(*blob.sha256());
        if let Some(value) = self.table_value(DOMAIN_CAPSULE_BLOB, digest.as_bytes())? {
            return Ok((blob, locator_from_value(&value)?));
        }
        let locator = self.append_record(bytes)?;
        self.put(DOMAIN_CAPSULE_BLOB, digest.as_bytes(), &locator.to_bytes())?;
        Ok((blob, locator))
    }

    /// Re-append a blob whose existing record has been damaged in place.
    ///
    /// `stage_capsule_blob` deduplicates on the blob digest, which is right for
    /// authoring (I-25) and wrong for repair: the digest row still names the
    /// TORN record, so re-staging identical bytes hands the same broken locator
    /// back. Recovery appends fresh bytes and repoints the row, which is what
    /// `repack_cold_history` does for the whole archive.
    #[cfg(test)]
    fn restage_capsule_blob(
        &mut self,
        bytes: &[u8],
    ) -> Result<(BlobDescription, ColdLocatorV1), String> {
        let blob = BlobDescription::of(bytes);
        let digest = ContentDigest::from_bytes(*blob.sha256());
        let locator = self.append_record(bytes)?;
        self.put(DOMAIN_CAPSULE_BLOB, digest.as_bytes(), &locator.to_bytes())?;
        Ok((blob, locator))
    }
}

impl SealedTableAccess for SealedGenerationStagingStore {
    fn table_value(&self, domain: TableDomain, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        self.cut.get(domain, key)
    }

    fn table_predecessor(
        &self,
        domain: TableDomain,
        key: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
        self.cut.predecessor(domain, key)
    }

    fn sealed_record(&self, locator: ColdLocatorV1, _limit: u64) -> Result<Vec<u8>, String> {
        self.cut
            .read_staged_or_packed(locator)
            .map_err(|error| error.to_string())
    }

    fn table_entries(&self, domain: TableDomain) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        self.cut.domain_entries(domain)
    }

    fn index_reads(&self) -> u64 {
        self.cut.index_reads()
    }
}

/// Resolve one covered acceptance sequence to its batch id. Test accessor.
#[cfg(test)]
fn sequence_batch_id<Store: SealedTableAccess>(
    store: &Store,
    sequence: u64,
) -> Result<Option<[u8; 16]>, String> {
    let Some(value) = store.table_value(DOMAIN_SEQUENCE, &sequence.to_be_bytes())? else {
        return Ok(None);
    };
    value
        .try_into()
        .map_err(|_| "sealed sequence row is not a batch id".to_owned())
        .map(Some)
}

/// Read one 32-byte table value back as a record locator.
fn locator_from_value(value: &[u8]) -> Result<ColdLocatorV1, String> {
    ColdLocatorV1::from_value(value)
}

const DOCUMENT_CAPSULE_SCHEMA: u32 = 1;

fn verify_capsule_blob(expected: BlobDescription, bytes: &[u8]) -> Result<(), String> {
    if BlobDescription::of(bytes) != expected {
        return Err("generation capsule blob differs from its exact description".into());
    }
    Ok(())
}

/// Resolve one document image from the sealed packs.
///
/// Images are pack records like everything else, found through the capsule
/// blob domain; nothing writes a per-image file (I-14).
fn read_capsule_blob<Store: SealedTableAccess>(
    store: &Store,
    blob: BlobDescription,
) -> Result<Vec<u8>, String> {
    if blob.byte_length() > MAX_CHECKPOINT_BYTES {
        return Err("generation capsule blob exceeds the current checkpoint record limit".into());
    }
    let digest = ContentDigest::from_bytes(*blob.sha256());
    let value = store
        .table_value(DOMAIN_CAPSULE_BLOB, digest.as_bytes())?
        .ok_or("generation capsule blob is missing")?;
    let bytes = store.sealed_record(locator_from_value(&value)?, blob.byte_length())?;
    verify_capsule_blob(blob, &bytes)?;
    Ok(bytes)
}

/// Per-document immutable roster value. The checkpoint digest binds actual CRDT
/// bytes; dependencies retain stable document identity and accepted direct heads.
/// No run-local cutoff digest is serialized, so unchanged values remain shared.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentCapsuleRecord {
    schema: u32,
    dependencies: DocumentDependencies,
    checkpoint: BlobDescription,
    policy: DocumentCheckpointPolicyV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentCheckpointPolicyV1 {
    eligible_through: u64,
    config: super::checkpoint_floor_policy::FloorPolicyConfig,
    requested_k: Option<u64>,
    actual_removed_through: u64,
    actual_floor: Vec<CrdtPeerCounter>,
    metrics: super::checkpoint_floor_policy::FloorMetrics,
}

impl DocumentCheckpointPolicyV1 {
    fn uncut(checkpoint_bytes: usize) -> Result<Self, String> {
        let image_bytes = u64::try_from(checkpoint_bytes)
            .map_err(|_| "checkpoint image size exceeds u64".to_owned())?;
        let config = super::checkpoint_floor_policy::FloorPolicyConfig::default();
        Ok(Self {
            eligible_through: 0,
            config,
            requested_k: None,
            actual_removed_through: 0,
            actual_floor: Vec::new(),
            metrics: super::checkpoint_floor_policy::FloorMetrics {
                image_bytes,
                latest_state_bytes: image_bytes,
                removable_bytes: 0,
                budget_bytes: config
                    .minimum_tail_bytes
                    .max(config.live_size_multiplier.saturating_mul(image_bytes)),
                post_cut_removable_bytes: 0,
                hysteresis_shortfall_bytes: 0,
                budget_overage_bytes: 0,
                limiting_cause: None,
            },
        })
    }

    fn from_compact(
        eligible_through: u64,
        config: super::checkpoint_floor_policy::FloorPolicyConfig,
        document_id: DocumentId,
        document: &loro::LoroDoc,
        compact: &PolicyCompactAcceptedDocument,
    ) -> Result<Self, String> {
        use super::checkpoint_floor_policy::LoroFloorDecision;
        let (requested_k, actual_removed_through, actual_floor, metrics) = match compact.decision()
        {
            LoroFloorDecision::Keep {
                retained, metrics, ..
            } => (None, 0, &retained.actual_floor, *metrics),
            LoroFloorDecision::Advance {
                chosen, metrics, ..
            } => (
                Some(chosen.requested_k),
                chosen.actual_removed_through,
                &chosen.actual_floor,
                *metrics,
            ),
        };
        Ok(Self {
            eligible_through,
            config,
            requested_k,
            actual_removed_through,
            actual_floor: super::hot_engine::shallow_frontier_counters(
                document_id,
                document,
                actual_floor,
            )
            .map_err(|error| error.to_string())?,
            metrics,
        })
    }

    fn validate(&self, dependencies: &DocumentDependencies, fence: u64) -> Result<(), String> {
        if self.config.revision == 0
            || self.config.minimum_tail_bytes == 0
            || self.config.live_size_multiplier == 0
            || self.eligible_through > fence
            || self
                .requested_k
                .is_some_and(|requested| requested > self.eligible_through)
            || self.actual_removed_through > self.requested_k.unwrap_or(0)
            || !self
                .actual_floor
                .windows(2)
                .all(|pair| pair[0].peer_id() < pair[1].peer_id())
            || self.actual_floor.iter().any(|floor| {
                dependencies
                    .peer_counters()
                    .binary_search_by_key(&floor.peer_id(), |counter| counter.peer_id())
                    .ok()
                    .is_none_or(|index| {
                        dependencies.peer_counters()[index].max_counter() < floor.max_counter()
                    })
            })
        {
            return Err("checkpoint document floor policy binding is invalid".into());
        }
        let expected_removable = self
            .metrics
            .image_bytes
            .saturating_sub(self.metrics.latest_state_bytes);
        let expected_budget = self.config.minimum_tail_bytes.max(
            self.config
                .live_size_multiplier
                .saturating_mul(self.metrics.latest_state_bytes),
        );
        if self.metrics.removable_bytes != expected_removable
            || self.metrics.budget_bytes != expected_budget
            || self.metrics.budget_overage_bytes
                != self
                    .metrics
                    .post_cut_removable_bytes
                    .saturating_sub(expected_budget)
        {
            return Err("checkpoint document floor metrics are inconsistent".into());
        }
        Ok(())
    }
}

impl DocumentCapsuleRecord {
    fn encode(&self) -> Result<Vec<u8>, String> {
        postcard::to_stdvec(self).map_err(|error| error.to_string())
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let (record, remaining): (Self, &[u8]) =
            postcard::take_from_bytes(bytes).map_err(|error| error.to_string())?;
        if record.schema != DOCUMENT_CAPSULE_SCHEMA
            || !remaining.is_empty()
            || record.encode()? != bytes
        {
            return Err("generation document capsule is not the current canonical record".into());
        }
        Ok(record)
    }
}

/// An immutable document map candidate. The enclosing generation must separately
/// prove the complete roster and bind workspace/catalog/cutoff/retention facts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SealedDocumentRoster {
    map: SealedDocumentMap,
}

impl SealedDocumentRoster {
    pub(crate) fn empty() -> Self {
        Self {
            map: SealedDocumentMap::empty(),
        }
    }

    pub(crate) fn with_document(
        self,
        store: &mut SealedGenerationStagingStore,
        cutoff: &SealedAcceptedCutoff,
        compact: &CompactAcceptedDocument,
    ) -> Result<Self, String> {
        if compact.cutoff_state_digest() != cutoff.frontier().state_digest() {
            return Err("generation capsule belongs to another accepted cutoff".into());
        }
        let (checkpoint, _) = store.stage_capsule_blob(compact.checkpoint())?;
        let record = DocumentCapsuleRecord {
            schema: DOCUMENT_CAPSULE_SCHEMA,
            dependencies: compact.dependencies().clone(),
            checkpoint,
            policy: DocumentCheckpointPolicyV1::uncut(compact.checkpoint().len())?,
        };
        let (_, record_locator) = store.stage_capsule_blob(&record.encode()?)?;
        let map = self.map.upsert(
            store,
            super::DocumentKey::Entity(record.dependencies.document_id()),
            record_locator,
        )?;
        Ok(Self { map })
    }

    fn with_policy_document(
        self,
        store: &mut SealedGenerationStagingStore,
        cutoff_state_digest: ContentDigest,
        eligible_through: u64,
        config: super::checkpoint_floor_policy::FloorPolicyConfig,
        document: &loro::LoroDoc,
        compact: &PolicyCompactAcceptedDocument,
    ) -> Result<(Self, DocumentCheckpointPolicyV1), String> {
        if compact.cutoff_state_digest() != cutoff_state_digest {
            return Err("generation image belongs to another accepted cutoff".into());
        }
        let (checkpoint, _) = store.stage_capsule_blob(compact.checkpoint())?;
        let policy = DocumentCheckpointPolicyV1::from_compact(
            eligible_through,
            config,
            compact.dependencies().document_id(),
            document,
            compact,
        )?;
        let record = DocumentCapsuleRecord {
            schema: DOCUMENT_CAPSULE_SCHEMA,
            dependencies: compact.dependencies().clone(),
            checkpoint,
            policy: policy.clone(),
        };
        let (_, record_locator) = store.stage_capsule_blob(&record.encode()?)?;
        let map = self.map.upsert(
            store,
            super::DocumentKey::Entity(record.dependencies.document_id()),
            record_locator,
        )?;
        Ok((Self { map }, policy))
    }

    fn without_document(
        self,
        store: &mut SealedGenerationStagingStore,
        document: DocumentId,
    ) -> Result<Self, String> {
        Ok(Self {
            map: self
                .map
                .remove(store, super::DocumentKey::Entity(document))?,
        })
    }

    pub(crate) fn load_document<Store: SealedTableAccess>(
        self,
        store: &Store,
        catalog: DocumentId,
        document: DocumentId,
    ) -> Result<Option<(DocumentDependencies, loro::LoroDoc)>, String> {
        let Some(record) = self.document_record(store, document)? else {
            return Ok(None);
        };
        let checkpoint = read_capsule_blob(store, record.checkpoint)?;
        let restored =
            super::hot_engine::qualify_compact_document(catalog, &record.dependencies, &checkpoint)
                .map_err(|error| error.to_string())?;
        Ok(Some((record.dependencies, restored)))
    }

    pub(crate) fn qualify_complete_keys<Store: SealedTableAccess>(
        self,
        store: &Store,
        documents: impl Iterator<Item = DocumentId>,
    ) -> Result<(), String> {
        self.map
            .qualify_complete_keys(store, documents.map(super::DocumentKey::Entity))
    }

    pub(crate) fn document_count(self) -> u64 {
        self.map.count()
    }

    pub(crate) fn with_count(documents: u64) -> Self {
        Self {
            map: SealedDocumentMap::with_count(documents),
        }
    }

    /// Resume a roster over the rows the staging store's base already holds.
    pub(crate) fn over_base<Store: SealedTableAccess>(store: &Store) -> Result<Self, String> {
        Ok(Self {
            map: SealedDocumentMap::over_base(store)?,
        })
    }

    /// Start a base-zero roster, dropping every inherited row first.
    pub(crate) fn cleared(store: &mut SealedGenerationStagingStore) -> Result<Self, String> {
        Ok(Self {
            map: SealedDocumentMap::clear(store)?,
        })
    }

    fn document_reference<Store: SealedTableAccess>(
        self,
        store: &Store,
        document: DocumentId,
    ) -> Result<Option<ColdLocatorV1>, String> {
        self.map.value(store, super::DocumentKey::Entity(document))
    }

    pub(crate) fn inherited_dependencies(
        self,
        store: &SealedGenerationStagingStore,
        document: DocumentId,
    ) -> Result<Option<DocumentDependencies>, String> {
        Ok(self
            .document_record(store, document)?
            .map(|record| record.dependencies))
    }

    fn load_staged_document(
        self,
        store: &SealedGenerationStagingStore,
        catalog: DocumentId,
        document: DocumentId,
    ) -> Result<Option<(DocumentDependencies, loro::LoroDoc)>, String> {
        self.load_document(store, catalog, document)
    }

    fn document_record<Store: SealedTableAccess>(
        self,
        store: &Store,
        document: DocumentId,
    ) -> Result<Option<DocumentCapsuleRecord>, String> {
        let Some(locator) = self.document_reference(store, document)? else {
            return Ok(None);
        };
        let bytes = store.sealed_record(locator, MAX_CHECKPOINT_BYTES)?;
        let record = DocumentCapsuleRecord::decode(&bytes)?;
        if record.dependencies.document_id() != document {
            return Err("generation document descriptor names another document".into());
        }
        Ok(Some(record))
    }
}

/// Where a published sealed root record landed, as the payload records it.
///
/// A generation names its OWN root, not the marker's. The marker's pack table
/// resolves any locator ever published, so the rollback slot keeps resolving
/// after a later cut advances the marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedRootWire {
    offset: u64,
    length: u64,
}

impl SealedRootWire {
    fn of(locator: ColdLocatorV1) -> Self {
        Self {
            offset: locator.offset,
            length: locator.length,
        }
    }

    fn locator(self) -> ColdLocatorV1 {
        ColdLocatorV1 {
            offset: self.offset,
            length: self.length,
        }
    }
}

/// The four existing identity-admission domains. The discriminants are part
/// of the one current checkpoint format and deliberately match no ownership
/// decision: values are the existing admission evidence encoded by
/// `hot_engine`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CheckpointIdentityKind {
    BlockHome,
    LogseqUuid,
    PortablePath,
    PageName,
}

impl CheckpointIdentityKind {
    const ALL: [Self; 4] = [
        Self::BlockHome,
        Self::LogseqUuid,
        Self::PortablePath,
        Self::PageName,
    ];

    /// This kind's index into the two identity domain families.
    fn domain_index(self) -> u8 {
        match self {
            Self::BlockHome => 0,
            Self::LogseqUuid => 1,
            Self::PortablePath => 2,
            Self::PageName => 3,
        }
    }

    fn complete_domain(self) -> TableDomain {
        identity_complete_domain(self.domain_index())
    }

    fn current_domain(self) -> TableDomain {
        identity_current_domain(self.domain_index())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CheckpointIdentityChange {
    pub(crate) sequence: u64,
    pub(crate) kind: CheckpointIdentityKind,
    pub(crate) key: Vec<u8>,
    pub(crate) value: super::hot_engine::CheckpointIdentityValue,
    pub(crate) current: bool,
}

/// `document uuid ‖ acceptance sequence`, the document-change domain's key.
fn document_change_key(document: DocumentId, sequence: u64) -> [u8; 24] {
    let mut key = [0_u8; 24];
    key[..16].copy_from_slice(document.as_uuid().as_bytes());
    key[16..].copy_from_slice(&sequence.to_be_bytes());
    key
}

/// The acceptance sequence a document-change key names.
fn document_change_key_parts(key: &[u8]) -> Result<(DocumentId, u64), String> {
    if key.len() != 24 {
        return Err("sealed document-change key has the wrong width".into());
    }
    let document = DocumentId::from_uuid(
        uuid::Uuid::from_slice(&key[..16]).map_err(|error| error.to_string())?,
    );
    let mut sequence = [0_u8; 8];
    sequence.copy_from_slice(&key[16..]);
    Ok((document, u64::from_be_bytes(sequence)))
}

/// `peer id -> counter ‖ batch id ‖ zero padding`.
///
/// The tip is stored in the row itself rather than behind a record locator:
/// twenty-four bytes fit the domain's value width, so a tip lookup costs no
/// record read at all.
fn causal_tip_value(record: tine_storage::sealed_accepted_index::CausalTipRecordV2) -> [u8; 32] {
    let mut value = [0_u8; 32];
    value[..8].copy_from_slice(&record.highest_accepted_counter.to_be_bytes());
    value[8..24].copy_from_slice(&record.batch_id);
    value
}

fn causal_tip_record(
    peer_id: [u8; 16],
    value: &[u8],
) -> Result<tine_storage::sealed_accepted_index::CausalTipRecordV2, String> {
    if value.len() != 32 || value[24..].iter().any(|byte| *byte != 0) {
        return Err("sealed causal tip value is not the current shape".into());
    }
    let mut counter = [0_u8; 8];
    counter.copy_from_slice(&value[..8]);
    let mut batch_id = [0_u8; 16];
    batch_id.copy_from_slice(&value[8..24]);
    Ok(tine_storage::sealed_accepted_index::CausalTipRecordV2 {
        peer_id,
        highest_accepted_counter: u64::from_be_bytes(counter),
        batch_id,
    })
}

/// A sealed accepted record is stored as `logical address (32) ‖ canonical bytes`.
///
/// `tine-storage`'s `SealedAcceptedCausalRecordV2::decode` and
/// `AcceptedStatusRecordV2::decode` take the record's LOGICAL address -- a
/// domain-folded digest over the record's fields -- and refuse any decode whose
/// recomputed address differs. That address is not the digest of the encoding,
/// and the fixed-width batch row has room for the two pack locators only, so the
/// address travels with the record. Integrity of the bytes is already the pack
/// record header's `sha256(payload)`; carrying the address is what lets
/// `SealedBatchRecords::verify` bind the causal and status records to one
/// identity, which is the cross-check D-2 keeps after the proof walk went away.
const SEALED_ADDRESSED_RECORD_PREFIX_BYTES: usize = 32;

fn addressed_record(address: ContentDigest, bytes: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(SEALED_ADDRESSED_RECORD_PREFIX_BYTES + bytes.len());
    blob.extend_from_slice(address.as_bytes());
    blob.extend_from_slice(bytes);
    blob
}

fn split_addressed_record(blob: &[u8]) -> Result<(ContentDigest, &[u8]), String> {
    if blob.len() <= SEALED_ADDRESSED_RECORD_PREFIX_BYTES {
        return Err("sealed accepted record is shorter than its address prefix".into());
    }
    let mut address = [0_u8; SEALED_ADDRESSED_RECORD_PREFIX_BYTES];
    address.copy_from_slice(&blob[..SEALED_ADDRESSED_RECORD_PREFIX_BYTES]);
    Ok((
        ContentDigest::from_bytes(address),
        &blob[SEALED_ADDRESSED_RECORD_PREFIX_BYTES..],
    ))
}

/// `batch id -> causal record locator ‖ status record locator`.
fn batch_row_value(causal: ColdLocatorV1, status: ColdLocatorV1) -> [u8; 64] {
    let mut value = [0_u8; 64];
    value[..32].copy_from_slice(&causal.to_bytes());
    value[32..].copy_from_slice(&status.to_bytes());
    value
}

fn batch_row_locators(value: &[u8]) -> Result<(ColdLocatorV1, ColdLocatorV1), String> {
    if value.len() != 64 {
        return Err("sealed batch row is not the current width".into());
    }
    Ok((
        locator_from_value(&value[..32])?,
        locator_from_value(&value[32..])?,
    ))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointBindingV1 {
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRecoveryFenceV1 {
    accepted_sequence: u64,
    accepted_state_digest: ContentDigest,
    eligible_through: u64,
    retained_history_ms: i64,
    floor_policy: super::checkpoint_floor_policy::FloorPolicyConfig,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointPayloadV2 {
    schema_version: u32,
    binding: CheckpointBindingV1,
    recovery_fence: CheckpointRecoveryFenceV1,
    state_bytes: Vec<u8>,
    /// The one sealed root this generation was built over. Every domain --
    /// batch, sequence, document change, covered object, causal tip, the four
    /// identity families and the document roster -- is a table list inside it.
    sealed_root: SealedRootWire,
    sealed_root_digest: ContentDigest,
    /// Live rows in the document roster domain at this root.
    document_count: u64,
    identity_publish_work: IdentityPublishWork,
    capture_work: u64,
    image_work: CheckpointImageWork,
    document_dependencies: Vec<DocumentDependencies>,
    sqlite_generation: SqliteGenerationBindingV1,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SqliteGenerationBindingV1 {
    generation_id: [u8; 16],
    predecessor_generation_id: Option<[u8; 16]>,
    full_anchor_generation_id: [u8; 16],
    covered_block_count: u64,
    covered_semantic_capsules_root_digest: ContentDigest,
    covered_head_facts_root_digest: ContentDigest,
    current_projection_payload_pins_root_digest: ContentDigest,
    nonlinear_state_root_digest: ContentDigest,
    retention_pins_root_digest: ContentDigest,
}

/// Exact storage-owned anchor input selected by one qualified generation.
/// SQLite remains disposable; this value is immutable generation evidence used
/// to rebuild or validate its generation-relative cache.
pub(crate) struct CleanCheckpointSqliteAnchor {
    pub(crate) root: tine_storage::sqlite::PhysicalCheckpointFrontierRoot,
    pub(crate) anchor: tine_storage::sqlite::PhysicalCheckpointGenerationAnchor,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityPublishWork {
    pub(crate) changed_records: u64,
    /// Sealed TABLE records read while publishing the identity delta. The
    /// field keeps its persisted name; with sorted tables the unit it counts
    /// is a table, not a treap node.
    pub(crate) map_node_reads: u64,
    /// Domain entries staged by the identity delta.
    pub(crate) map_node_writes: u64,
    /// Variable-length admission-value records appended.
    pub(crate) value_writes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckpointImageWork {
    pub(crate) changed_documents: u64,
    pub(crate) exported_documents: u64,
    pub(crate) reused_documents: u64,
    pub(crate) snapshot_handoff_imports: u64,
    pub(crate) changed_predecessor_image_imports: u64,
    pub(crate) changed_document_reconstructions: u64,
    pub(crate) unchanged_image_imports: u64,
    pub(crate) unchanged_image_reconstructions: u64,
    pub(crate) measurement_exports: u64,
    pub(crate) candidate_exports: u64,
    pub(crate) verification_imports: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointGenerationV2 {
    schema_version: u32,
    sequence: u64,
    slot: u8,
    payload_digest: ContentDigest,
    payload_len: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointPointerV2 {
    schema_version: u32,
    sequence: u64,
    slot: u8,
    generation_digest: ContentDigest,
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(value).map_err(|error| error.to_string())
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(bytes: &[u8]) -> Result<T, String> {
    let (value, trailing): (T, &[u8]) =
        postcard::take_from_bytes(bytes).map_err(|error| error.to_string())?;
    if !trailing.is_empty() || encode_canonical(&value)? != bytes {
        return Err("clean checkpoint value is noncanonical".into());
    }
    Ok(value)
}

/// An in-process accepted-history cutoff built from engine evidence. It is not
/// a durable generation, a portable frontier, or a decoded checkpoint payload.
/// The unchanged run-local frontier is only a qualification witness for this
/// construction; a later generation format must bind the canonical facts.
#[derive(Clone, Debug)]
pub(crate) struct SealedAcceptedCutoff {
    frontier: AcceptedFrontierRoot,
    /// Accepted rows staged so far. `sequence` is the highest one.
    sequence: u64,
    causal_tips: BTreeMap<[u8; 16], tine_storage::sealed_accepted_index::CausalTipRecordV2>,
}

impl SealedAcceptedCutoff {
    pub(crate) fn empty(frontier: AcceptedFrontierRoot) -> Result<Self, String> {
        frontier
            .encode_canonical()
            .map_err(|error| error.to_string())?;
        if frontier.acceptance_sequence() != 0 || frontier.batch_map_root_key().is_some() {
            return Err("sealed cutoff bootstrap requires a sequence-zero frontier".into());
        }
        Ok(Self {
            frontier,
            sequence: 0,
            causal_tips: BTreeMap::new(),
        })
    }

    pub(crate) fn frontier(&self) -> &AcceptedFrontierRoot {
        &self.frontier
    }

    /// This cutoff's identity as a single comparable value.
    ///
    /// It replaces the treap `roots()` tuple the tests used to compare. There
    /// is no authenticated root over the covered half any more (D-2), so what
    /// identifies a cutoff is what it actually asserts: the accepted frontier
    /// it ends at, how many rows it covers, and the exact causal tip per peer.
    pub(crate) fn identity_digest(&self) -> ContentDigest {
        let mut material = Vec::new();
        material.extend_from_slice(self.frontier.state_digest().as_bytes());
        material.extend_from_slice(&self.sequence.to_be_bytes());
        for (peer, tip) in &self.causal_tips {
            material.extend_from_slice(peer);
            material.extend_from_slice(&causal_tip_value(*tip));
        }
        ContentDigest::of(&material)
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// One comparable value over every peer's exact causal tip.
    ///
    /// It replaces the retired causal-tip treap root: the tips are the claim,
    /// the treap was only how they used to be addressed.
    pub(crate) fn causal_tip_digest(&self) -> ContentDigest {
        let mut material = Vec::with_capacity(self.causal_tips.len() * 48);
        for (peer, tip) in &self.causal_tips {
            material.extend_from_slice(peer);
            material.extend_from_slice(&causal_tip_value(*tip));
        }
        ContentDigest::of(&material)
    }

    pub(crate) fn causal_tips(
        &self,
    ) -> impl Iterator<Item = &tine_storage::sealed_accepted_index::CausalTipRecordV2> {
        self.causal_tips.values()
    }

    pub(crate) fn builder<'a>(
        &self,
        store: &'a mut SealedGenerationStagingStore,
    ) -> AcceptedCutoffBuilder<'a> {
        AcceptedCutoffBuilder {
            store,
            cutoff: self.clone(),
        }
    }
}

pub(crate) struct AcceptedCutoffBuilder<'a> {
    store: &'a mut SealedGenerationStagingStore,
    cutoff: SealedAcceptedCutoff,
}

impl AcceptedCutoffBuilder<'_> {
    pub(crate) fn append(&mut self, row: &CleanCheckpointAcceptedRow) -> Result<(), String> {
        if row.evidence.prior_frontier_root() != &self.cutoff.frontier {
            return Err("sealed cutoff row does not extend its exact predecessor".into());
        }
        let batch_id = row.evidence.batch_id().as_uuid().into_bytes();
        if self.store.table_value(DOMAIN_BATCH, &batch_id)?.is_some() {
            return Err("sealed cutoff repeats an accepted batch".into());
        }
        let peer_id = row.causal_dot.peer_id().key().as_uuid().into_bytes();
        let prior_tip = self.cutoff.causal_tips.get(&peer_id).copied();
        let sealed_tip = self
            .store
            .table_value(DOMAIN_CAUSAL_TIP, &peer_id)?
            .map(|value| causal_tip_record(peer_id, &value))
            .transpose()?;
        if sealed_tip != prior_tip {
            return Err("sealed cutoff causal-tip predecessor does not authenticate".into());
        }
        let tip = tine_storage::sealed_accepted_index::CausalTipRecordV2 {
            peer_id,
            highest_accepted_counter: row.causal_dot.counter(),
            batch_id,
        };
        if tip.highest_accepted_counter == 0 {
            return Err("sealed cutoff causal-tip counter is zero".into());
        }
        if prior_tip.is_some_and(|prior| {
            prior.highest_accepted_counter == tip.highest_accepted_counter
                && prior.batch_id != tip.batch_id
        }) {
            return Err("sealed cutoff has conflicting batches at one causal tip".into());
        }
        let advance_tip = prior_tip
            .is_none_or(|prior| prior.highest_accepted_counter < tip.highest_accepted_counter);
        // Stage the row's records and entries. Nothing is durable until the
        // cut publishes and the marker installs (I-2), so an interrupted or
        // erroring append leaves the predecessor root exactly as it was.
        append_accepted_row(self.store, self.cutoff.sequence, row)?;
        let frontier = row.evidence.post_frontier_root();
        if row.evidence.acceptance_sequence() != frontier.acceptance_sequence() {
            return Err("sealed cutoff row sequence differs from engine evidence".into());
        }
        // The two records the batch row names must now resolve, and they must
        // bind to the same identity the engine accepted. This is
        // `prove_membership`'s cross-check without the Merkle path (D-2).
        let records = sealed_batch_records(self.store, batch_id)?
            .ok_or("sealed cutoff membership is missing after publication")?;
        records
            .verify(
                row.evidence.acceptance_sequence(),
                batch_id,
                &TineAcceptedEvidenceDecoder,
            )
            .map_err(|error| error.to_string())?;
        if records.status.no_op != row.no_op
            || records.status.exact_evidence_bytes
                != row
                    .evidence
                    .encode_canonical()
                    .map_err(|error| error.to_string())?
        {
            return Err("sealed cutoff status differs from engine acceptance".into());
        }
        if advance_tip {
            self.store
                .put(DOMAIN_CAUSAL_TIP, &peer_id, &causal_tip_value(tip))?;
        }
        // Mutate only the builder's private token after every staged write and
        // point proof succeeds. The input predecessor remains unchanged.
        self.cutoff.frontier = frontier.clone();
        self.cutoff.sequence = row.evidence.acceptance_sequence();
        if advance_tip {
            self.cutoff.causal_tips.insert(peer_id, tip);
        }
        Ok(())
    }

    pub(crate) fn finish(
        self,
        expected: &AcceptedFrontierRoot,
    ) -> Result<SealedAcceptedCutoff, String> {
        if &self.cutoff.frontier != expected {
            return Err("sealed cutoff does not reach the requested engine frontier".into());
        }
        Ok(self.cutoff)
    }
}

/// The two records one sealed accepted batch resolves to, through the batch
/// domain's single row. Two record reads, no structure walk.
pub(crate) fn sealed_batch_records<Store: SealedTableAccess>(
    store: &Store,
    batch_id: [u8; 16],
) -> Result<Option<tine_storage::sealed_accepted_index::SealedBatchRecords>, String> {
    use tine_storage::sealed_accepted_index::{
        AcceptedStatusRecordV2, SealedAcceptedCausalRecordV2, SealedBatchRecords,
    };
    let Some(row) = store.table_value(DOMAIN_BATCH, &batch_id)? else {
        return Ok(None);
    };
    let (causal_locator, status_locator) = batch_row_locators(&row)?;
    let causal_blob = store.sealed_record(causal_locator, MAX_CHECKPOINT_BYTES)?;
    let status_blob = store.sealed_record(status_locator, MAX_CHECKPOINT_BYTES)?;
    let (causal_address, causal_bytes) = split_addressed_record(&causal_blob)?;
    let (status_address, status_bytes) = split_addressed_record(&status_blob)?;
    let causal = SealedAcceptedCausalRecordV2::decode(batch_id, causal_address, causal_bytes)
        .map_err(|error| error.to_string())?;
    let status = AcceptedStatusRecordV2::decode(batch_id, status_address, status_bytes)
        .map_err(|error| error.to_string())?;
    Ok(Some(SealedBatchRecords { causal, status }))
}

/// One conversion from engine acceptance evidence to the sealed domains.
///
/// Causal and status stay ordinary variable-length pack records; the batch
/// row points at both by locator, and the sequence row names the batch.
fn append_accepted_row(
    store: &mut SealedGenerationStagingStore,
    base_sequence: u64,
    row: &CleanCheckpointAcceptedRow,
) -> Result<(), String> {
    use tine_storage::sealed_accepted_index::{
        AcceptedStatusRecordV2, SealedAcceptedCausalClockEntryV2, SealedAcceptedCausalRecordV2,
    };
    if base_sequence.checked_add(1) != Some(row.evidence.acceptance_sequence()) {
        return Err("sealed accepted delta sequence is not contiguous".into());
    }
    let batch_id = row.evidence.batch_id().as_uuid().into_bytes();
    let causal = SealedAcceptedCausalRecordV2 {
        batch_id,
        manifest_fingerprint: row.evidence.manifest_fingerprint(),
        event_binding_digest: row.evidence.event_binding_digest(),
        causal_peer_id: row.causal_dot.peer_id().key().as_uuid().into_bytes(),
        causal_counter: row.causal_dot.counter(),
        canonical_causal_clock: row
            .canonical_causal_clock
            .iter()
            .map(|(peer, counter)| SealedAcceptedCausalClockEntryV2 {
                peer_id: peer.key().as_uuid().into_bytes(),
                counter: *counter,
            })
            .collect(),
    };
    let causal_bytes = causal.encode().map_err(|error| error.to_string())?;
    let causal_address = causal.address().map_err(|error| error.to_string())?;
    let causal_locator = store.append_record(&addressed_record(causal_address, &causal_bytes))?;
    let status = AcceptedStatusRecordV2 {
        batch_id,
        no_op: row.no_op,
        evidence_schema: ACCEPTED_EVIDENCE_SCHEMA_VERSION,
        exact_evidence_bytes: row
            .evidence
            .encode_canonical()
            .map_err(|error| error.to_string())?,
        accepted_causal_record_digest: causal_address,
    };
    let status_bytes = status.encode().map_err(|error| error.to_string())?;
    let status_address = status.value_digest();
    let status_locator = store.append_record(&addressed_record(status_address, &status_bytes))?;
    store.put(
        DOMAIN_BATCH,
        &batch_id,
        &batch_row_value(causal_locator, status_locator),
    )?;
    store.put(
        DOMAIN_SEQUENCE,
        &row.evidence.acceptance_sequence().to_be_bytes(),
        &batch_id,
    )?;
    Ok(())
}

/// The last cut published by this process, for the release-only curve's T1/T2
/// numbers. Test-only: index bytes per batch cannot be read off the filesystem.
#[cfg(test)]
pub(crate) static LAST_SEALED_CUT_WORK: std::sync::Mutex<Option<SealedCutWork>> =
    std::sync::Mutex::new(None);

fn build_payload_with_images(
    capture: CleanCheckpointCapture,
    predecessor: Option<(u64, CheckpointPayloadV2)>,
    document_roster: SealedDocumentRoster,
    image_work: CheckpointImageWork,
    store: &mut SealedGenerationStagingStore,
) -> Result<(u64, CheckpointPayloadV2), String> {
    let predecessor_sqlite_generation = predecessor
        .as_ref()
        .map(|(_, payload)| payload.sqlite_generation.clone());
    let document_dependencies = capture
        .documents
        .as_ref()
        .map(|documents| documents.dependencies.values().cloned().collect())
        .or_else(|| {
            predecessor
                .as_ref()
                .map(|(_, payload)| payload.document_dependencies.clone())
        })
        .unwrap_or_default();
    // Every domain of the predecessor is already staged in the open cut's base
    // root, so the delta below is genuinely a delta: nothing is recopied
    // forward and no root is rebuilt (I-25).
    let base_sequence = match predecessor {
        Some((sequence, payload)) => {
            if payload.schema_version != CHECKPOINT_SCHEMA_VERSION
                || payload.binding.workspace_id != capture.workspace_id
                || payload.binding.lineage_digest != capture.lineage_digest
                || payload.binding.catalog_document_id != capture.catalog_document_id
                || sequence < capture.base_sequence
                || sequence > capture.target_sequence
            {
                return Err("clean checkpoint predecessor frontier is incompatible".into());
            }
            if payload.recovery_fence.accepted_sequence != sequence {
                return Err("clean checkpoint predecessor roster frontier differs".into());
            }
            sequence
        }
        None => {
            if capture.base_sequence != 0 {
                return Err("clean checkpoint delta has no durable predecessor".into());
            }
            0
        }
    };
    let mut sequence_reached = base_sequence;
    for row in &capture.accepted_rows {
        if row.evidence.acceptance_sequence() <= sequence_reached {
            continue;
        }
        append_accepted_row(&mut *store, sequence_reached, row)?;
        sequence_reached = row.evidence.acceptance_sequence();
        for document in row.evidence.affected_documents() {
            store.put(
                DOMAIN_DOCUMENT_CHANGE,
                &document_change_key(document.document_id(), row.evidence.acceptance_sequence()),
                &row.evidence.acceptance_sequence().to_be_bytes(),
            )?;
        }
    }
    let sequence = capture.target_sequence;
    if sequence_reached != sequence {
        return Err("clean checkpoint delta does not reach its target frontier".into());
    }
    for digest in &capture.required_objects {
        store.put(DOMAIN_COVERED_OBJECT, digest.as_bytes(), &[1])?;
    }
    let changed_records = u64::try_from(capture.identity_changes.len())
        .map_err(|_| "identity change count exceeds u64")?;
    let index_reads_before = store.index_reads();
    let entry_writes_before = store.entry_writes();
    let record_writes_before = store.record_writes();
    let mut previous_change = None;
    for change in &capture.identity_changes {
        if (change.sequence <= capture.base_sequence
            && !(capture.base_sequence == 0 && change.sequence == 0))
            || change.sequence > sequence
            || previous_change.is_some_and(|previous| previous > change.sequence)
        {
            return Err("clean checkpoint identity delta is outside its accepted tail".into());
        }
        previous_change = Some(change.sequence);
        let key = framed_identity_key(&change.key)?;
        // Encoding belongs to the disposable publisher, not the actor that
        // accepted the change. Capture carries only a bounded typed delta.
        let value_bytes = change.value.encode_canonical()?;
        let locator = store.append_record(&value_bytes)?;
        store.put(change.kind.complete_domain(), &key, &locator.to_bytes())?;
        if change.current {
            store.put(change.kind.current_domain(), &key, &locator.to_bytes())?;
        } else {
            store.remove(change.kind.current_domain(), &key)?;
        }
    }
    let identity_publish_work = IdentityPublishWork {
        changed_records,
        map_node_reads: store.index_reads().saturating_sub(index_reads_before),
        map_node_writes: store.entry_writes().saturating_sub(entry_writes_before),
        value_writes: store.record_writes().saturating_sub(record_writes_before),
    };
    // Qualification is point-bounded at the new terminal entry. Predecessor
    // rows were already qualified when their marker became authoritative;
    // walking them again would make every later generation O(H).
    if sequence != 0 {
        let batch_id = store
            .table_value(DOMAIN_SEQUENCE, &sequence.to_be_bytes())?
            .ok_or_else(|| "final clean checkpoint sequence is incomplete".to_owned())?;
        let batch_id: [u8; 16] = batch_id
            .try_into()
            .map_err(|_| "sealed sequence row is not a batch id".to_owned())?;
        let records = sealed_batch_records(&*store, batch_id)?
            .ok_or_else(|| "final clean checkpoint roster membership is absent".to_owned())?;
        records
            .verify(sequence, batch_id, &TineAcceptedEvidenceDecoder)
            .map_err(|error| error.to_string())?;
        let evidence =
            AcceptedBatchEvidence::decode_canonical(&records.status.exact_evidence_bytes)
                .map_err(|error| error.to_string())?;
        if evidence.post_frontier_root().state_digest() != capture.cutoff_state_digest {
            return Err("final clean checkpoint roster frontier differs".into());
        }
    }
    let state_root_digest = ContentDigest::of(&capture.state_bytes);
    // The root record is not published yet -- `finish` seals this cut. The
    // payload's own identity therefore binds the FACTS the generation covers;
    // its exact sealed root locator and digest are stamped in by the caller
    // once the cut publishes.
    let generation_material = encode_canonical(&(
        capture.workspace_id,
        capture.lineage_digest,
        capture.target_sequence,
        capture.cutoff_state_digest,
        document_roster.document_count(),
        state_root_digest,
    ))?;
    let generation_digest = ContentDigest::of(&generation_material);
    let mut generation_id = [0_u8; 16];
    generation_id.copy_from_slice(&generation_digest.as_bytes()[..16]);
    let sqlite_generation = SqliteGenerationBindingV1 {
        generation_id,
        predecessor_generation_id: predecessor_sqlite_generation
            .as_ref()
            .map(|generation| generation.generation_id),
        full_anchor_generation_id: predecessor_sqlite_generation
            .as_ref()
            .map_or(generation_id, |generation| {
                generation.full_anchor_generation_id
            }),
        covered_block_count: capture.covered_block_count,
        covered_semantic_capsules_root_digest: generation_digest,
        covered_head_facts_root_digest: capture.cutoff_state_digest,
        current_projection_payload_pins_root_digest: state_root_digest,
        nonlinear_state_root_digest: state_root_digest,
        retention_pins_root_digest: state_root_digest,
    };
    let payload = CheckpointPayloadV2 {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        binding: CheckpointBindingV1 {
            workspace_id: capture.workspace_id,
            lineage_digest: capture.lineage_digest,
            catalog_document_id: capture.catalog_document_id,
        },
        recovery_fence: CheckpointRecoveryFenceV1 {
            accepted_sequence: sequence,
            accepted_state_digest: capture.cutoff_state_digest,
            eligible_through: capture.eligible_through,
            retained_history_ms: super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            floor_policy: capture.floor_policy,
        },
        state_bytes: capture.state_bytes,
        // Stamped by the caller after `SealedGenerationStagingStore::finish`.
        sealed_root: SealedRootWire {
            offset: 0,
            length: 0,
        },
        sealed_root_digest: ContentDigest::of(&[]),
        document_count: document_roster.document_count(),
        identity_publish_work,
        capture_work: capture.capture_work,
        image_work,
        document_dependencies,
        sqlite_generation,
    };
    Ok((sequence, payload))
}

/// Payload-only construction used by the publication primitive tests. Live
/// capture first publishes its immutable document images and calls the inner
/// constructor with their qualified roster.
fn build_payload(
    capture: CleanCheckpointCapture,
    predecessor: Option<(u64, CheckpointPayloadV2)>,
    store: &mut SealedGenerationStagingStore,
) -> Result<(u64, CheckpointPayloadV2), String> {
    if capture.documents.is_some() {
        return Err("live checkpoint payload requires published document images".into());
    }
    // The census is the store's live rows, not the payload's remembered count:
    // this cut opens over the predecessor's tables, and a base-zero cut
    // REPLACES that roster instead of extending it.
    let document_roster = match predecessor.as_ref() {
        Some(_) => SealedDocumentRoster::over_base(&*store)?,
        None => SealedDocumentRoster::cleared(store)?,
    };
    build_payload_with_images(
        capture,
        predecessor,
        document_roster,
        CheckpointImageWork::default(),
        store,
    )
}

fn checkpoint_directory(store: &ObjectStore) -> Result<cap_std::fs::Dir, String> {
    let root = store
        .private_derived_root_capability()
        .map_err(|error| error.to_string())?;
    tine_storage::ensure_directory_nofollow(&root, CHECKPOINT_DIRECTORY)
        .map_err(|error| error.to_string())?;
    tine_storage::open_dir_nofollow(&root, CHECKPOINT_DIRECTORY).map_err(|error| error.to_string())
}

fn read_current_payload_for_extension(
    store: &ObjectStore,
) -> Result<Option<(u64, CheckpointPayloadV2)>, String> {
    let directory = checkpoint_directory(store)?;
    let Some(pointer_bytes) =
        tine_storage::read_optional_regular(&directory, CHECKPOINT_POINTER, 4 * 1024, None)
            .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let pointer: CheckpointPointerV2 = decode_canonical(&pointer_bytes)?;
    if pointer.schema_version != CHECKPOINT_SCHEMA_VERSION || pointer.slot >= 2 {
        return Err("clean checkpoint predecessor pointer is invalid".into());
    }
    let slot = pointer.slot as usize;
    let generation_bytes = tine_storage::read_optional_regular(
        &directory,
        CHECKPOINT_GENERATION_NAMES[slot],
        16 * 1024,
        None,
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "clean checkpoint predecessor generation is missing".to_owned())?;
    if ContentDigest::of(&generation_bytes) != pointer.generation_digest {
        return Err("clean checkpoint predecessor generation digest differs".into());
    }
    let generation: CheckpointGenerationV2 = decode_canonical(&generation_bytes)?;
    if generation.schema_version != CHECKPOINT_SCHEMA_VERSION
        || generation.slot != pointer.slot
        || generation.sequence != pointer.sequence
    {
        return Err("clean checkpoint predecessor generation is invalid".into());
    }
    let payload_bytes = tine_storage::read_optional_regular(
        &directory,
        CHECKPOINT_PAYLOAD_NAMES[slot],
        MAX_CHECKPOINT_BYTES,
        Some(generation.payload_len),
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "clean checkpoint predecessor payload is missing".to_owned())?;
    if ContentDigest::of(&payload_bytes) != generation.payload_digest {
        return Err("clean checkpoint predecessor payload digest differs".into());
    }
    let payload: CheckpointPayloadV2 = decode_canonical(&payload_bytes)?;
    if payload.schema_version != CHECKPOINT_SCHEMA_VERSION {
        return Err("clean checkpoint predecessor payload schema differs".into());
    }
    validate_checkpoint_payload_metadata(store, &directory, generation, &payload, None)?;
    Ok(Some((generation.sequence, payload)))
}

fn validate_checkpoint_payload_metadata(
    store: &ObjectStore,
    directory: &cap_std::fs::Dir,
    generation: CheckpointGenerationV2,
    payload: &CheckpointPayloadV2,
    pinned_unchanged_images: Option<&BTreeSet<DocumentId>>,
) -> Result<(), String> {
    if payload.schema_version != CHECKPOINT_SCHEMA_VERSION
        || payload.binding.workspace_id != store.workspace_id()
        || payload.recovery_fence.accepted_sequence != generation.sequence
        || payload.recovery_fence.eligible_through > generation.sequence
        || payload.recovery_fence.retained_history_ms
            != super::checkpoint_floor_policy::RETAINED_HISTORY_MS
        || payload.recovery_fence.floor_policy.revision == 0
        || payload.recovery_fence.floor_policy.minimum_tail_bytes == 0
        || payload.recovery_fence.floor_policy.live_size_multiplier == 0
    {
        return Err("checkpoint payload publication binding is invalid".into());
    }
    if !payload
        .document_dependencies
        .windows(2)
        .all(|pair| pair[0].document_id() < pair[1].document_id())
    {
        return Err("clean checkpoint document dependencies are not strictly ordered".into());
    }
    let roster = SealedDocumentRoster::with_count(payload.document_count);
    let reader = SealedGenerationDirectory::open_generation(
        directory,
        payload.sealed_root.locator(),
        payload.sealed_root_digest,
    )?;
    roster.qualify_complete_keys(
        &reader,
        payload
            .document_dependencies
            .iter()
            .map(DocumentDependencies::document_id),
    )?;
    if roster.document_count() != payload.document_dependencies.len() as u64 {
        return Err("clean checkpoint document roster cardinality differs".into());
    }
    for expected in &payload.document_dependencies {
        let record = roster
            .document_record(&reader, expected.document_id())?
            .ok_or_else(|| "clean checkpoint document descriptor is missing".to_owned())?;
        if &record.dependencies != expected {
            return Err("clean checkpoint document descriptor binding differs".into());
        }
        if !pinned_unchanged_images
            .is_some_and(|documents| documents.contains(&expected.document_id()))
        {
            // Cold qualification hashes every named immutable image but does
            // not import it into a Loro document. Candidate publication may
            // reuse the staging/predecessor proof for pinned immutable bytes.
            reader.read_capsule_blob(record.checkpoint)?;
        }
        record
            .policy
            .validate(expected, payload.recovery_fence.accepted_sequence)?;
    }
    Ok(())
}

/// Qualify the exact bytes now present in the inactive slot before `current`
/// can name them. Document images were already verification-imported when
/// changed; unchanged immutable images reuse that proof and only their sealed
/// descriptor/dependency/floor bindings are checked here.
fn validate_published_candidate(
    store: &ObjectStore,
    directory: &cap_std::fs::Dir,
    slot: usize,
    expected_generation: &[u8],
    expected_payload: &[u8],
    validate_state_binding: bool,
    pinned_unchanged_images: &BTreeSet<DocumentId>,
) -> Result<(), String> {
    let payload_bytes = tine_storage::read_optional_regular(
        directory,
        CHECKPOINT_PAYLOAD_NAMES[slot],
        MAX_CHECKPOINT_BYTES,
        Some(expected_payload.len() as u64),
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "published checkpoint candidate payload is missing".to_owned())?;
    if payload_bytes != expected_payload {
        return Err("published checkpoint candidate payload differs".into());
    }
    let generation_bytes = tine_storage::read_optional_regular(
        directory,
        CHECKPOINT_GENERATION_NAMES[slot],
        16 * 1024,
        Some(expected_generation.len() as u64),
    )
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "published checkpoint candidate generation is missing".to_owned())?;
    if generation_bytes != expected_generation {
        return Err("published checkpoint candidate generation differs".into());
    }
    let generation: CheckpointGenerationV2 = decode_canonical(&generation_bytes)?;
    if generation.schema_version != CHECKPOINT_SCHEMA_VERSION
        || generation.slot as usize != slot
        || generation.payload_len != payload_bytes.len() as u64
        || generation.payload_digest != ContentDigest::of(&payload_bytes)
    {
        return Err("published checkpoint candidate generation binding is invalid".into());
    }
    let payload: CheckpointPayloadV2 = decode_canonical(&payload_bytes)?;
    validate_checkpoint_payload_metadata(
        store,
        directory,
        generation,
        &payload,
        Some(pinned_unchanged_images),
    )?;
    if validate_state_binding {
        let state = super::hot_engine::clean_checkpoint_state_binding(&payload.state_bytes)
            .map_err(|error| error.to_string())?;
        if state.workspace_id != payload.binding.workspace_id
            || state.lineage_digest != payload.binding.lineage_digest
            || state.catalog_document_id != payload.binding.catalog_document_id
            || state.accepted_sequence != payload.recovery_fence.accepted_sequence
            || state.accepted_state_digest != payload.recovery_fence.accepted_state_digest
            || state.eligible_through != payload.recovery_fence.eligible_through
        {
            return Err("published checkpoint state/recovery binding differs".into());
        }
    }
    Ok(())
}

fn install_replaceable_exact(
    directory: &cap_std::fs::Dir,
    publication: &tine_storage::DurableDirectoryPublication,
    name: &str,
    replacement: &[u8],
) -> Result<(), String> {
    match tine_storage::read_optional_regular(directory, name, MAX_CHECKPOINT_BYTES, None)
        .map_err(|error| error.to_string())?
    {
        Some(existing) if existing == replacement => Ok(()),
        Some(existing) => publication
            .replace_exact(name, &existing, replacement)
            .map_err(|error| error.to_string()),
        None => publication
            .publish_new_exact_single_writer(name, replacement)
            .map_err(|error| error.to_string()),
    }
}

fn floor_candidate_for_document(
    directory: &cap_std::fs::Dir,
    predecessor: Option<&(u64, CheckpointPayloadV2)>,
    accepted_rows: &[CleanCheckpointAcceptedRow],
    eligible_through: u64,
    document: DocumentId,
    lazy_genesis: &super::lazy_genesis::LazyGenesisCandidate,
) -> Result<Option<(u64, DocumentDependencies)>, String> {
    let predecessor_sequence = predecessor.map_or(0, |(sequence, _)| *sequence);
    if eligible_through
        > predecessor_sequence.saturating_add(
            u64::try_from(accepted_rows.len())
                .map_err(|_| "checkpoint accepted delta exceeds u64".to_owned())?,
        )
    {
        return Err("checkpoint age eligibility exceeds available accepted history".into());
    }
    if eligible_through == 0 {
        return Ok(None);
    }
    if let Some(candidate) = accepted_rows
        .iter()
        .rev()
        .filter(|row| row.evidence.acceptance_sequence() <= eligible_through)
        .find_map(|row| {
            row.evidence
                .affected_documents()
                .iter()
                .find(|candidate| candidate.document_id() == document)
                .cloned()
                .map(|dependencies| (row.evidence.acceptance_sequence(), dependencies))
        })
    {
        return Ok(Some(candidate));
    }
    if let Some((_, payload)) = predecessor {
        let history = SealedAcceptedHistory {
            directory: SealedGenerationDirectory::open_generation(
                directory,
                payload.sealed_root.locator(),
                payload.sealed_root_digest,
            )?,
            sequence: payload.recovery_fence.accepted_sequence,
            identity_history: Arc::new(SealedIdentityHistory::new(
                SealedGenerationDirectory::open_generation(
                    directory,
                    payload.sealed_root.locator(),
                    payload.sealed_root_digest,
                )?,
            )),
            sequence_enumerations: AtomicUsize::new(0),
            sequence_row_reads: AtomicUsize::new(0),
        };
        if let Some(candidate) = history.document_dependencies_at_or_before(
            document,
            eligible_through.min(predecessor_sequence),
        )? {
            return Ok(Some(candidate));
        }
    }
    Ok(lazy_genesis
        .frontier_document(document)
        .map(|dependencies| (eligible_through, dependencies)))
}

/// Publish this generation's document images into the OPEN cut.
///
/// Images and the payload share one cut, so one marker install makes the whole
/// generation durable (I-2). There is no separate image publication step and
/// no second directory-visible write path.
#[allow(clippy::too_many_arguments)]
fn publish_document_images(
    staging: &mut SealedGenerationStagingStore,
    directory: &cap_std::fs::Dir,
    store: &ObjectStore,
    capture: &super::hot_engine::CleanCheckpointDocumentCapture,
    eligible_through: u64,
    policy: super::checkpoint_floor_policy::FloorPolicyConfig,
    measurement_sequence: u64,
    accepted_rows: &[CleanCheckpointAcceptedRow],
    predecessor: Option<&(u64, CheckpointPayloadV2)>,
) -> Result<
    (
        SealedDocumentRoster,
        CheckpointImageWork,
        Vec<SyncCheckpointDocumentDiagnostics>,
    ),
    String,
> {
    let predecessor_sequence = predecessor.map(|(sequence, _)| *sequence);
    let predecessor_payload = predecessor.map(|(_, payload)| payload);
    let predecessor_roster = match predecessor_payload {
        Some(_) => SealedDocumentRoster::over_base(&*staging)?,
        None => SealedDocumentRoster::cleared(staging)?,
    };
    let mut roster = predecessor_roster;
    let mut work = CheckpointImageWork::default();
    let mut diagnostics = Vec::new();
    if let Some(predecessor_payload) = predecessor_payload {
        for dependencies in &predecessor_payload.document_dependencies {
            if !capture
                .dependencies
                .contains_key(&dependencies.document_id())
            {
                roster = roster.without_document(staging, dependencies.document_id())?;
            }
        }
    }
    for (document_id, dependencies) in &capture.dependencies {
        if predecessor_roster
            .inherited_dependencies(&*staging, *document_id)?
            .as_ref()
            == Some(dependencies)
        {
            work.reused_documents = work.reused_documents.saturating_add(1);
            continue;
        }
        if !capture.changed_snapshots.contains_key(document_id) {
            return Err(format!(
                "changed checkpoint document {document_id} has no fenced worker input"
            ));
        }
        work.changed_documents = work.changed_documents.saturating_add(1);
        let predecessor_document = capture
            .changed_snapshots
            .get(document_id)
            .is_some_and(Option::is_none)
            .then_some(predecessor_sequence)
            .flatten()
            .map(|sequence| {
                predecessor_roster
                    .load_staged_document(&*staging, capture.catalog_document_id, *document_id)
                    .map(|loaded| {
                        loaded.map(|(dependencies, document)| (sequence, dependencies, document))
                    })
            })
            .transpose()?
            .flatten();
        let materialized =
            super::hot_engine::ShardedHotEngine::materialize_checkpoint_worker_document(
                capture,
                store,
                *document_id,
                predecessor_document,
                accepted_rows,
            )
            .map_err(|error| error.to_string())?;
        work.snapshot_handoff_imports = work
            .snapshot_handoff_imports
            .saturating_add(u64::from(materialized.imported_handoff));
        work.changed_predecessor_image_imports = work
            .changed_predecessor_image_imports
            .saturating_add(u64::from(materialized.imported_predecessor_image));
        work.changed_document_reconstructions = work
            .changed_document_reconstructions
            .saturating_add(u64::from(materialized.reconstructed));
        let compact = super::hot_engine::ShardedHotEngine::build_policy_compact_worker_document(
            capture,
            *document_id,
            dependencies.clone(),
            &materialized.document,
            eligible_through,
            policy,
            floor_candidate_for_document(
                directory,
                predecessor,
                accepted_rows,
                eligible_through,
                *document_id,
                &capture.lazy_genesis,
            )?,
        )
        .map_err(|error| error.to_string())?;
        work.exported_documents = work.exported_documents.saturating_add(1);
        work.measurement_exports = work
            .measurement_exports
            .saturating_add(compact.work().measurement_exports);
        work.candidate_exports = work
            .candidate_exports
            .saturating_add(compact.work().candidate_exports);
        work.verification_imports = work
            .verification_imports
            .saturating_add(compact.work().verification_imports);
        let (next_roster, document_policy) = roster.with_policy_document(
            staging,
            capture.cutoff_state_digest,
            eligible_through,
            policy,
            &materialized.document,
            &compact,
        )?;
        roster = next_roster;
        let limiting_cause = document_policy
            .metrics
            .limiting_cause
            .map(|cause| match cause {
                super::checkpoint_floor_policy::LimitingCause::AgeLowerBound => {
                    SyncCheckpointLimitingCause::AgeLowerBound
                }
                super::checkpoint_floor_policy::LimitingCause::NativeNormalization => {
                    SyncCheckpointLimitingCause::NativeNormalization
                }
            });
        diagnostics.push(SyncCheckpointDocumentDiagnostics {
            document_id: *document_id,
            measurement_sequence,
            requested_floor: document_policy.requested_k,
            actual_floor: document_policy.actual_floor,
            image_bytes: document_policy.metrics.image_bytes,
            latest_state_bytes: document_policy.metrics.latest_state_bytes,
            removable_bytes: document_policy.metrics.removable_bytes,
            budget_bytes: document_policy.metrics.budget_bytes,
            post_cut_removable_bytes: document_policy.metrics.post_cut_removable_bytes,
            hysteresis_shortfall_bytes: document_policy.metrics.hysteresis_shortfall_bytes,
            budget_overage_bytes: document_policy.metrics.budget_overage_bytes,
            limiting_cause,
        });
    }
    if roster.document_count() != capture.dependencies.len() as u64 {
        return Err("checkpoint image roster has extra or missing documents".into());
    }
    roster.qualify_complete_keys(&*staging, capture.dependencies.keys().copied())?;
    Ok((roster, work, diagnostics))
}

struct PublishedCheckpoint {
    sequence: u64,
    documents: Option<Arc<CleanCheckpointDocuments>>,
    identities: Arc<SealedIdentityHistory>,
    identity_publish_work: IdentityPublishWork,
    diagnostics: SyncCheckpointPublicationDiagnostics,
}

fn publish_capture(
    store: &ObjectStore,
    capture: CleanCheckpointCapture,
) -> Result<PublishedCheckpoint, String> {
    publish_capture_with_predecessor(store, capture, true, None)
}

fn publish_capture_with_predecessor(
    store: &ObjectStore,
    capture: CleanCheckpointCapture,
    extend_predecessor: bool,
    publication_authority: Option<&CheckpointPublicationAuthority>,
) -> Result<PublishedCheckpoint, String> {
    let publication_started_at = Instant::now();
    let store_stats_before = store.instrumentation();
    let relocation_batches = capture
        .accepted_rows
        .iter()
        .map(|row| row.evidence.batch_id())
        .collect::<BTreeSet<_>>();
    let hot_pin_batches = if capture.documents.is_some() {
        super::hot_engine::clean_checkpoint_hot_pin_batches(&capture.state_bytes)
            .map_err(|error| error.to_string())?
    } else {
        BTreeSet::new()
    };
    let measurement_sequence = capture.target_sequence;
    let policy = capture.floor_policy;
    let latest_acceptance_utc_ms = capture.latest_acceptance_utc_ms;
    let eligible_through = capture.eligible_through;
    let age_cutoff_utc_ms = capture.age_cutoff_utc_ms;
    let clock_frozen = capture.clock_frozen;
    let last_clock_reset_utc_ms = capture.last_clock_reset_utc_ms;
    #[cfg(test)]
    if FAIL_CHECKPOINT_WRITE_ROOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(store.root_path())
    {
        return Err("deterministic checkpoint publication failure".into());
    }
    let has_document_epoch = capture.documents.is_some();
    let pinned_unchanged_images = capture
        .documents
        .as_ref()
        .map(|documents| {
            documents
                .dependencies
                .keys()
                .filter(|document| !documents.changed_snapshots.contains_key(document))
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let predecessor = if extend_predecessor {
        read_current_payload_for_extension(store)?
    } else {
        None
    };
    let mut retirement_batches = relocation_batches.clone();
    if has_document_epoch {
        if let Some((_, payload)) = predecessor.as_ref() {
            retirement_batches.extend(
                super::hot_engine::clean_checkpoint_hot_pin_batches(&payload.state_bytes)
                    .map_err(|error| error.to_string())?
                    .difference(&hot_pin_batches)
                    .copied(),
            );
        }
    }
    let directory = checkpoint_directory(store)?;
    // Images, the accepted delta and the root record are ONE cut: one marker
    // install makes the whole generation durable, and a crash before it leaves
    // the predecessor generation exactly as authoritative as it was (I-2).
    let mut sealed_staging = SealedGenerationStagingStore::open(&directory)?;
    let (document_roster, image_work, document_diagnostics) = match capture.documents.as_ref() {
        Some(documents) => publish_document_images(
            &mut sealed_staging,
            &directory,
            store,
            documents,
            capture.eligible_through,
            capture.floor_policy,
            capture.target_sequence,
            &capture.accepted_rows,
            predecessor.as_ref(),
        )?,
        None => (
            match predecessor.as_ref() {
                Some(_) => SealedDocumentRoster::over_base(&sealed_staging)?,
                None => SealedDocumentRoster::cleared(&mut sealed_staging)?,
            },
            CheckpointImageWork::default(),
            Vec::new(),
        ),
    };
    let image_phase_done_at = Instant::now();
    let (sequence, mut payload) = build_payload_with_images(
        capture,
        predecessor,
        document_roster,
        image_work,
        &mut sealed_staging,
    )?;
    let (cut_work, _published) = sealed_staging.finish()?;
    #[cfg(test)]
    {
        // I-25 is a number, and a directory census cannot answer it: index
        // records and payload records share a pack file. Only the cut itself
        // knows which bytes it wrote for the index.
        *LAST_SEALED_CUT_WORK.lock().unwrap() = Some(cut_work);
    }
    payload.sealed_root = SealedRootWire::of(cut_work.root_locator);
    payload.sealed_root_digest = cut_work.root_digest;
    let sealed_root_locator = cut_work.root_locator;
    let payload_bytes = encode_canonical(&payload)?;
    let payload_phase_done_at = Instant::now();
    let payload_len = u64::try_from(payload_bytes.len())
        .map_err(|_| "clean checkpoint payload length exceeds u64".to_owned())?;
    if payload_len > MAX_CHECKPOINT_BYTES {
        return Err("clean checkpoint payload exceeds its disposable-cache limit".into());
    }
    let publication = tine_storage::DurableDirectoryPublication::open(&directory)
        .map_err(|error| error.to_string())?;
    let prior_pointer_bytes =
        tine_storage::read_optional_regular(&directory, CHECKPOINT_POINTER, 4 * 1024, None)
            .map_err(|error| error.to_string())?;
    let prior_slot = prior_pointer_bytes
        .as_deref()
        .and_then(|bytes| decode_canonical::<CheckpointPointerV2>(bytes).ok())
        .filter(|pointer| pointer.schema_version == CHECKPOINT_SCHEMA_VERSION && pointer.slot < 2)
        .map(|pointer| pointer.slot as usize);
    let slot = prior_slot.map_or(0, |slot| 1 - slot);
    install_replaceable_exact(
        &directory,
        &publication,
        CHECKPOINT_PAYLOAD_NAMES[slot],
        &payload_bytes,
    )?;
    let generation = CheckpointGenerationV2 {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        sequence,
        slot: slot as u8,
        payload_digest: ContentDigest::of(&payload_bytes),
        payload_len,
    };
    let generation_bytes = encode_canonical(&generation)?;
    install_replaceable_exact(
        &directory,
        &publication,
        CHECKPOINT_GENERATION_NAMES[slot],
        &generation_bytes,
    )?;
    validate_published_candidate(
        store,
        &directory,
        slot,
        &generation_bytes,
        &payload_bytes,
        has_document_epoch,
        &pinned_unchanged_images,
    )?;
    // Cold publication is additive and exact-byte preserving.  Keep each
    // input turn bounded; only this generation's C+1..=new C inventory is
    // visited after bootstrap.
    for chunk in relocation_batches
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .chunks(64)
    {
        store
            .publish_cold_history_for_batches(&chunk.iter().copied().collect())
            .map_err(|error| error.to_string())?;
    }
    let pointer = CheckpointPointerV2 {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        sequence,
        slot: slot as u8,
        generation_digest: ContentDigest::of(&generation_bytes),
    };
    let pointer_bytes = encode_canonical(&pointer)?;
    if let Some(authority) = publication_authority {
        // This is intentionally after complete candidate validation and
        // immediately before the sole commit-point name operation.
        authority.revalidate()?;
    }
    match prior_pointer_bytes {
        Some(existing) if existing == pointer_bytes => {}
        Some(existing) => publication
            .replace_exact(CHECKPOINT_POINTER, &existing, &pointer_bytes)
            .map_err(|error| error.to_string())?,
        None => publication
            .publish_new_exact_single_writer(CHECKPOINT_POINTER, &pointer_bytes)
            .map_err(|error| error.to_string())?,
    }
    // Marker-last makes the new roots authoritative before any hot name can
    // disappear.  Each deletion is then safe to repeat after a crash.
    // The marker is already the authority. Retirement is a repeatable
    // placement optimization, so a failure leaves the safe hot+cold duplicate
    // in place and must not make the committed generation look unpublished to
    // the actor. Ordinary reopen resumes it from the marker-selected roots.
    let _ = store.retire_hot_history_for_batches(&retirement_batches, &hot_pin_batches);
    let publication_done_at = Instant::now();
    let store_stats_after = store.instrumentation();
    let documents = has_document_epoch
        .then(|| {
            SealedGenerationDirectory::open_generation(
                &directory,
                sealed_root_locator,
                payload.sealed_root_digest,
            )
            .map(|directory| {
                Arc::new(CleanCheckpointDocuments {
                    roster: document_roster,
                    directory,
                    sequence,
                })
            })
        })
        .transpose()?;
    let published_payload: CheckpointPayloadV2 = decode_canonical(&payload_bytes)?;
    let identities = Arc::new(SealedIdentityHistory::new(SealedGenerationDirectory::open(
        &directory,
    )?));
    Ok(PublishedCheckpoint {
        sequence,
        documents,
        identities,
        identity_publish_work: published_payload.identity_publish_work,
        diagnostics: SyncCheckpointPublicationDiagnostics {
            measurement_sequence,
            latest_acceptance_utc_ms,
            eligible_through,
            age_cutoff_utc_ms: Some(age_cutoff_utc_ms),
            clock_frozen: Some(clock_frozen),
            last_clock_reset_utc_ms,
            policy_revision: policy.revision,
            minimum_tail_bytes: policy.minimum_tail_bytes,
            live_size_multiplier: policy.live_size_multiplier,
            documents: document_diagnostics,
            changed_documents: image_work.changed_documents,
            exported_documents: image_work.exported_documents,
            reused_documents: image_work.reused_documents,
            measurement_exports: image_work.measurement_exports,
            candidate_exports: image_work.candidate_exports,
            verification_imports: image_work.verification_imports,
            hot_manifest_reads: usize_delta_u64(
                store_stats_after.accepted_manifest_reads,
                store_stats_before.accepted_manifest_reads,
            )
            .saturating_sub(usize_delta_u64(
                store_stats_after.cold_manifest_reads,
                store_stats_before.cold_manifest_reads,
            )),
            hot_manifest_bytes: usize_delta_u64(
                store_stats_after.hot_manifest_bytes,
                store_stats_before.hot_manifest_bytes,
            ),
            hot_object_reads: usize_delta_u64(
                store_stats_after.accepted_object_reads,
                store_stats_before.accepted_object_reads,
            )
            .saturating_sub(usize_delta_u64(
                store_stats_after.cold_object_reads,
                store_stats_before.cold_object_reads,
            )),
            hot_object_bytes: usize_delta_u64(
                store_stats_after.hot_object_bytes,
                store_stats_before.hot_object_bytes,
            ),
            cold_manifest_reads: usize_delta_u64(
                store_stats_after.cold_manifest_reads,
                store_stats_before.cold_manifest_reads,
            ),
            cold_manifest_bytes: usize_delta_u64(
                store_stats_after.cold_manifest_bytes,
                store_stats_before.cold_manifest_bytes,
            ),
            cold_object_reads: usize_delta_u64(
                store_stats_after.cold_object_reads,
                store_stats_before.cold_object_reads,
            ),
            cold_object_bytes: usize_delta_u64(
                store_stats_after.cold_object_bytes,
                store_stats_before.cold_object_bytes,
            ),
            relocation_batch_visits: usize_delta_u64(
                store_stats_after.relocation_batch_visits,
                store_stats_before.relocation_batch_visits,
            ),
            retired_hot_manifests: usize_delta_u64(
                store_stats_after.retired_hot_manifests,
                store_stats_before.retired_hot_manifests,
            ),
            retired_hot_objects: usize_delta_u64(
                store_stats_after.retired_hot_objects,
                store_stats_before.retired_hot_objects,
            ),
            image_phase_ms: elapsed_millis(publication_started_at, image_phase_done_at),
            payload_phase_ms: elapsed_millis(image_phase_done_at, payload_phase_done_at),
            publication_phase_ms: elapsed_millis(payload_phase_done_at, publication_done_at),
            peak_rss_bytes: None,
            checkpoint_bytes: payload_len,
            publication_edge: SyncCheckpointPublicationEdge::CurrentPointerDurable,
        },
    })
}

fn usize_delta_u64(after: usize, before: usize) -> u64 {
    u64::try_from(after.saturating_sub(before)).unwrap_or(u64::MAX)
}

fn elapsed_millis(start: Instant, end: Instant) -> u64 {
    u64::try_from(end.saturating_duration_since(start).as_millis()).unwrap_or(u64::MAX)
}

pub(crate) enum CleanCheckpointOpen {
    Absent,
    Invalid(String),
    Loaded(CleanCheckpointLoaded),
}

pub(crate) struct CleanCheckpointLoaded {
    pub(crate) state_bytes: Vec<u8>,
    pub(crate) accepted_history: Arc<SealedAcceptedHistory>,
    pub(crate) accepted_sequence: u64,
    pub(crate) tail: BTreeSet<BatchId>,
    pub(crate) capture_work: u64,
    pub(crate) payload_bytes: usize,
    pub(crate) image_work: CheckpointImageWork,
    pub(crate) documents: Arc<CleanCheckpointDocuments>,
    pub(crate) sqlite_anchor: Option<Arc<CleanCheckpointSqliteAnchor>>,
    pub(crate) open_work: GenerationOpenWork,
}

/// Lifetime-scaling work performed by one healthy generation open. Every field
/// here MUST be a number something actually increments: a counter that no code
/// path can raise reads zero whether or not the property holds, so asserting it
/// proves nothing while looking like proof.
///
/// That is why this struct carries one field and not four. The covered
/// namespace decode counters it used to carry were never incremented anywhere,
/// and `covered_roster_rows_loaded` was the literal `0` — so the regression
/// they named (a covered name being decoded during open) would have left all
/// three reading zero. What actually enforces that property is, in order of
/// strength: the `is_covered` early `continue` in `ObjectStore`'s namespace
/// walk, which makes a covered object decode unwritable; and
/// `generation_hot_retirement_bounded_open`'s assertions on the REAL
/// `ObjectStoreStats::namespace_{manifest,object}_decodes`, which do grow when
/// a covered name is read.
///
/// `covered_sequence_enumerations` is real: `note_sequence_enumeration` is
/// called by both paths that can walk `1..=sequence`
/// (`StatusHistorySource::materialize` and `accepted_batch_cursor`), and
/// reintroducing that walk on the open path fails the gate — verified.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GenerationOpenWork {
    pub(crate) covered_sequence_enumerations: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IdentityIndexWork {
    pub(crate) point_lookups: usize,
    pub(crate) map_node_reads: usize,
    pub(crate) value_reads: usize,
    pub(crate) bytes_read: usize,
    pub(crate) current_root_enumerations: usize,
    pub(crate) current_rows_enumerated: usize,
}

#[derive(Default)]
struct IdentityIndexCounters {
    point_lookups: AtomicUsize,
    map_node_reads: AtomicUsize,
    value_reads: AtomicUsize,
    bytes_read: AtomicUsize,
    current_root_enumerations: AtomicUsize,
    current_rows_enumerated: AtomicUsize,
}

/// Four typed domain pairs over the shared sorted tables. The complete
/// domains answer released/history points; the current domains alone may be
/// enumerated to rebuild O(G+O) resident claims.
pub(crate) struct SealedIdentityHistory {
    directory: SealedGenerationDirectory,
    counters: IdentityIndexCounters,
}

impl SealedIdentityHistory {
    fn new(directory: SealedGenerationDirectory) -> Self {
        Self {
            directory,
            counters: IdentityIndexCounters::default(),
        }
    }

    fn value_bytes(&self, locator: ColdLocatorV1) -> Result<Vec<u8>, String> {
        let bytes = self
            .directory
            .sealed_record(locator, MAX_CHECKPOINT_BYTES)?;
        self.counters
            .bytes_read
            .fetch_add(bytes.len(), Ordering::Relaxed);
        self.counters.value_reads.fetch_add(1, Ordering::Relaxed);
        Ok(bytes)
    }

    /// Table records this history has loaded. One table is one file open and
    /// one digest check, so this is the index-work unit the counters report.
    fn note_index_reads(&self, before: u64) {
        let after = self.directory.index_reads();
        self.counters.map_node_reads.fetch_add(
            usize::try_from(after.saturating_sub(before)).unwrap_or(usize::MAX),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn point(
        &self,
        kind: CheckpointIdentityKind,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, String> {
        self.counters.point_lookups.fetch_add(1, Ordering::Relaxed);
        let framed = framed_identity_key(key)?;
        let before = self.directory.index_reads();
        let value = self
            .directory
            .table_value(kind.complete_domain(), &framed)?;
        self.note_index_reads(before);
        value
            .map(|value| self.value_bytes(locator_from_value(&value)?))
            .transpose()
    }

    pub(crate) fn current_rows(
        &self,
        kind: CheckpointIdentityKind,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        self.counters
            .current_root_enumerations
            .fetch_add(1, Ordering::Relaxed);
        let before = self.directory.index_reads();
        let entries = self.directory.table_entries(kind.current_domain())?;
        self.note_index_reads(before);
        let mut rows = Vec::with_capacity(entries.len());
        for (framed, value) in entries {
            rows.push((
                unframed_identity_key(&framed)?,
                self.value_bytes(locator_from_value(&value)?)?,
            ));
        }
        rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        self.counters
            .current_rows_enumerated
            .fetch_add(rows.len(), Ordering::Relaxed);
        Ok(rows)
    }
    pub(crate) fn work(&self) -> IdentityIndexWork {
        IdentityIndexWork {
            point_lookups: self.counters.point_lookups.load(Ordering::Relaxed),
            map_node_reads: self.counters.map_node_reads.load(Ordering::Relaxed),
            value_reads: self.counters.value_reads.load(Ordering::Relaxed),
            bytes_read: self.counters.bytes_read.load(Ordering::Relaxed),
            current_root_enumerations: self
                .counters
                .current_root_enumerations
                .load(Ordering::Relaxed),
            current_rows_enumerated: self
                .counters
                .current_rows_enumerated
                .load(Ordering::Relaxed),
        }
    }
}

/// Point-addressable covered accepted history.  The root is qualified by the
/// marker-selected generation; ordinary consumers never enumerate the covered
/// sequence or retain one row per lifetime batch.
pub(crate) struct SealedAcceptedHistory {
    directory: SealedGenerationDirectory,
    /// The highest acceptance sequence this generation covers.
    sequence: u64,
    identity_history: Arc<SealedIdentityHistory>,
    sequence_enumerations: AtomicUsize,
    sequence_row_reads: AtomicUsize,
}

/// The accepted tail a live candidate has applied on top of the sealed root.
///
/// It is an ordinary in-memory map, not a second index: the sealed side is
/// immutable, so a tail cannot be expressed inside it, and a structure-shaped
/// overlay would be a second index format (D-1).
#[derive(Clone)]
pub(crate) struct SealedAcceptedBatchOverlay {
    history: Arc<SealedAcceptedHistory>,
    /// The TAIL treap, spanning `C+1..=N` only. Covered batches are in the
    /// sealed tables and are deliberately not expressible here: that is what
    /// makes the treap's size the tail's size rather than the history's.
    tail: super::hot_engine::RunLocalAuthenticatedMap,
    /// The same tail in acceptance-insensitive form, for the folded
    /// whole-history digest below.
    tail_rows: BTreeMap<[u8; 16], ContentDigest>,
}

impl SealedAcceptedBatchOverlay {
    pub(crate) fn new(history: Arc<SealedAcceptedHistory>) -> Self {
        Self {
            history,
            tail: super::hot_engine::RunLocalAuthenticatedMap::default(),
            tail_rows: BTreeMap::new(),
        }
    }

    /// The tail treap's root key, as tine-storage's
    /// `PhysicalFrontierRoot::batch_map_root_key` means it.
    pub(crate) fn tail_root_key(&self) -> Option<AuthenticatedMapKey> {
        self.tail.root_key()
    }

    pub(crate) fn tail_root_digest(&self) -> ContentDigest {
        self.tail.root_digest()
    }

    pub(crate) fn upsert(
        &mut self,
        batch_id: BatchId,
        causal_digest: ContentDigest,
    ) -> Result<(), String> {
        let key = batch_id.as_uuid().into_bytes();
        self.tail
            .upsert(AuthenticatedMapKey::from(key), causal_digest);
        self.tail_rows.insert(key, causal_digest);
        Ok(())
    }

    /// This candidate's frontier identity: the sealed root the generation
    /// published, folded with the tail it has accepted on top of it.
    pub(crate) fn root_digest(&self) -> Result<ContentDigest, String> {
        let mut material = Vec::with_capacity(32 + self.tail_rows.len() * 48);
        material.extend_from_slice(self.history.sealed_root_digest().as_bytes());
        for (batch_id, causal) in &self.tail_rows {
            material.extend_from_slice(batch_id);
            material.extend_from_slice(causal.as_bytes());
        }
        Ok(ContentDigest::of(&material))
    }

    pub(crate) fn covered_count(&self) -> u64 {
        self.history.sequence()
    }

    pub(crate) fn contains_batch(&self, batch_id: BatchId) -> Result<bool, String> {
        if self
            .tail_rows
            .contains_key(&batch_id.as_uuid().into_bytes())
        {
            return Ok(true);
        }
        self.history.contains_batch(batch_id)
    }
}

impl SealedAcceptedHistory {
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Infallible here on purpose: `SealedAcceptedHistory` is only ever built
    /// from `SealedGenerationDirectory::open_generation`, which has already
    /// decoded this root and proved it against the generation binding.
    pub(crate) fn sealed_root_digest(&self) -> ContentDigest {
        self.directory
            .sealed_root_digest()
            .unwrap_or_else(|_| tine_storage::sealed_tables::sealed_empty_root_digest())
    }

    pub(crate) fn note_sequence_enumeration(&self) {
        self.sequence_enumerations.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn sequence_enumerations(&self) -> usize {
        self.sequence_enumerations.load(Ordering::Relaxed)
    }

    /// Number of covered rows resolved by sequence. Point answers cost one;
    /// a caller that walks `1..=sequence` shows up here as the whole covered
    /// length per call, which is how the write-path I-14 regression is caught.
    pub(crate) fn sequence_row_reads(&self) -> usize {
        self.sequence_row_reads.load(Ordering::Relaxed)
    }

    fn batch_id_at(&self, sequence: u64) -> Result<Option<[u8; 16]>, String> {
        let Some(value) = self
            .directory
            .table_value(DOMAIN_SEQUENCE, &sequence.to_be_bytes())?
        else {
            return Ok(None);
        };
        value
            .try_into()
            .map_err(|_| "sealed sequence row is not a batch id".to_owned())
            .map(Some)
    }

    pub(crate) fn row_by_sequence(
        &self,
        sequence: u64,
    ) -> Result<Option<CleanCheckpointAcceptedRow>, String> {
        self.sequence_row_reads.fetch_add(1, Ordering::Relaxed);
        let Some(batch_id) = self.batch_id_at(sequence)? else {
            return Ok(None);
        };
        self.row_for_entry(sequence, batch_id)
    }

    pub(crate) fn row_by_batch(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<CleanCheckpointAcceptedRow>, String> {
        let key = batch_id.as_uuid().into_bytes();
        let Some(records) = sealed_batch_records(&self.directory, key)? else {
            return Ok(None);
        };
        let evidence =
            AcceptedBatchEvidence::decode_canonical(&records.status.exact_evidence_bytes)
                .map_err(|error| error.to_string())?;
        self.row_from_records(evidence.acceptance_sequence(), key, records)
    }

    pub(crate) fn contains_batch(&self, batch_id: BatchId) -> Result<bool, String> {
        Ok(self
            .directory
            .table_value(DOMAIN_BATCH, &batch_id.as_uuid().into_bytes())?
            .is_some())
    }

    pub(crate) fn contains_object(&self, digest: ContentDigest) -> Result<bool, String> {
        Ok(self
            .directory
            .table_value(DOMAIN_COVERED_OBJECT, digest.as_bytes())?
            .is_some())
    }

    /// The newest accepted change to `document` at or before `through`.
    ///
    /// One inclusive-floor lookup in the document-change domain, not a walk:
    /// the domain is keyed `document ‖ sequence`, so the floor of
    /// `document ‖ through` is exactly that row when one exists.
    pub(crate) fn document_dependencies_at_or_before(
        &self,
        document: DocumentId,
        through: u64,
    ) -> Result<Option<(u64, DocumentDependencies)>, String> {
        let target = document_change_key(document, through);
        let Some((key, _)) = self
            .directory
            .table_predecessor(DOMAIN_DOCUMENT_CHANGE, &target)?
        else {
            return Ok(None);
        };
        let (found_document, sequence) = document_change_key_parts(&key)?;
        if found_document != document {
            return Ok(None);
        }
        let row = self
            .row_by_sequence(sequence)?
            .ok_or_else(|| "sealed document-change sequence is absent".to_owned())?;
        let dependencies = row
            .evidence
            .affected_documents()
            .iter()
            .find(|candidate| candidate.document_id() == document)
            .cloned()
            .ok_or_else(|| "sealed document-change entry names no matching document".to_owned())?;
        Ok(Some((sequence, dependencies)))
    }

    fn row_for_entry(
        &self,
        sequence: u64,
        batch_id: [u8; 16],
    ) -> Result<Option<CleanCheckpointAcceptedRow>, String> {
        let Some(records) = sealed_batch_records(&self.directory, batch_id)? else {
            return Ok(None);
        };
        self.row_from_records(sequence, batch_id, records)
    }

    fn row_from_records(
        &self,
        sequence: u64,
        batch_id: [u8; 16],
        records: tine_storage::sealed_accepted_index::SealedBatchRecords,
    ) -> Result<Option<CleanCheckpointAcceptedRow>, String> {
        // The cross-check the retired Merkle proof also performed: bind the
        // two records and the evidence to one identity (D-2).
        records
            .verify(sequence, batch_id, &TineAcceptedEvidenceDecoder)
            .map_err(|error| error.to_string())?;
        let evidence =
            AcceptedBatchEvidence::decode_canonical(&records.status.exact_evidence_bytes)
                .map_err(|error| error.to_string())?;
        let causal = records.causal;
        let peer = CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
            causal.causal_peer_id,
        )));
        let causal_dot =
            BatchCausalDot::new(peer, causal.causal_counter).map_err(|error| error.to_string())?;
        let canonical_causal_clock = causal
            .canonical_causal_clock
            .iter()
            .map(|entry| {
                (
                    CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
                        entry.peer_id,
                    ))),
                    entry.counter,
                )
            })
            .collect();
        Ok(Some(CleanCheckpointAcceptedRow {
            no_op: records.status.no_op,
            evidence,
            causal_dot,
            canonical_causal_clock,
        }))
    }

    pub(crate) fn identity_history(&self) -> Arc<SealedIdentityHistory> {
        Arc::clone(&self.identity_history)
    }
}

/// The sealed side of the SQLite seam: two point lookups, no structure.
impl tine_storage::sealed_accepted_index::SealedAcceptedIndexRead for SealedAcceptedHistory {
    fn batch(
        &self,
        batch_id: [u8; 16],
    ) -> Result<
        Option<tine_storage::sealed_accepted_index::SealedBatchRecords>,
        tine_storage::sealed_accepted_index::SealedAcceptedIndexError,
    > {
        sealed_batch_records(&self.directory, batch_id)
            .map_err(tine_storage::sealed_accepted_index::SealedAcceptedIndexError::Corrupt)
    }

    fn sequence(
        &self,
        sequence: u64,
    ) -> Result<Option<[u8; 16]>, tine_storage::sealed_accepted_index::SealedAcceptedIndexError>
    {
        self.batch_id_at(sequence)
            .map_err(tine_storage::sealed_accepted_index::SealedAcceptedIndexError::Corrupt)
    }
}
pub(crate) struct CleanCheckpointDocuments {
    roster: SealedDocumentRoster,
    directory: SealedGenerationDirectory,
    sequence: u64,
}

impl CleanCheckpointDocuments {
    #[cfg(test)]
    pub(crate) fn directory_for_test(&self) -> &SealedGenerationDirectory {
        &self.directory
    }
}

impl CleanCheckpointLoaded {
    pub(crate) fn document_image_reference(
        &self,
        document: DocumentId,
    ) -> Result<ContentDigest, String> {
        self.documents
            .roster
            .document_record(&self.documents.directory, document)?
            .map(|record| ContentDigest::from_bytes(*record.checkpoint.sha256()))
            .ok_or_else(|| format!("checkpoint image roster omits document {document}"))
    }

    #[cfg(test)]
    pub(crate) fn fixed_live_curve_metrics_for_test(
        &self,
    ) -> Result<(u64, u64, usize, usize, u64, u64, u64), String> {
        let (documents, blocks, writers, obligations, eligible_through, document_ids) =
            super::hot_engine::clean_checkpoint_curve_dimensions(&self.state_bytes)
                .map_err(|error| error.to_string())?;
        let mut image_bytes = 0_u64;
        let mut latest_state_bytes = 0_u64;
        for document in document_ids {
            let record = self
                .documents
                .roster
                .document_record(&self.documents.directory, document)?
                .ok_or_else(|| format!("checkpoint image roster omits document {document}"))?;
            image_bytes = image_bytes
                .checked_add(record.policy.metrics.image_bytes)
                .ok_or_else(|| "checkpoint image byte count overflowed".to_owned())?;
            latest_state_bytes = latest_state_bytes
                .checked_add(record.policy.metrics.latest_state_bytes)
                .ok_or_else(|| "checkpoint latest-state byte count overflowed".to_owned())?;
        }
        Ok((
            documents,
            blocks,
            writers,
            obligations,
            eligible_through,
            image_bytes,
            image_bytes.saturating_sub(latest_state_bytes),
        ))
    }
}

impl CleanCheckpointDocuments {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn load_document(
        &self,
        catalog: DocumentId,
        document: DocumentId,
    ) -> Result<Option<(DocumentDependencies, loro::LoroDoc)>, String> {
        self.roster
            .load_document(&self.directory, catalog, document)
    }

    pub(crate) fn qualify_complete(
        &self,
        documents: impl Iterator<Item = DocumentId>,
    ) -> Result<(), String> {
        self.roster
            .qualify_complete_keys(&self.directory, documents)
    }

    pub(crate) fn dependencies(
        &self,
        documents: impl Iterator<Item = DocumentId>,
    ) -> Result<BTreeMap<DocumentId, DocumentDependencies>, String> {
        documents
            .map(|document| {
                self.roster
                    .document_record(&self.directory, document)?
                    .map(|record| (document, record.dependencies))
                    .ok_or_else(|| format!("checkpoint image roster omits document {document}"))
            })
            .collect()
    }
}

#[derive(Debug)]
pub(crate) enum CleanCheckpointOpenError {
    ArchiveDamage(String),
    Store(String),
}

fn invalid(message: impl Into<String>) -> CleanCheckpointOpen {
    CleanCheckpointOpen::Invalid(message.into())
}

pub(crate) fn open_checkpoint(
    store: &ObjectStore,
) -> Result<CleanCheckpointOpen, CleanCheckpointOpenError> {
    open_checkpoint_impl(store, false)
}

pub(crate) fn open_checkpoint_with_cold_history(
    store: &ObjectStore,
) -> Result<CleanCheckpointOpen, CleanCheckpointOpenError> {
    open_checkpoint_impl(store, true)
}

fn open_checkpoint_impl(
    store: &ObjectStore,
    logical_cold_history: bool,
) -> Result<CleanCheckpointOpen, CleanCheckpointOpenError> {
    let root = store
        .private_derived_root_capability()
        .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?;
    let Some(directory) = tine_storage::open_existing_dir_nofollow(&root, CHECKPOINT_DIRECTORY)
        .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
    else {
        return Ok(CleanCheckpointOpen::Absent);
    };
    let Some(pointer_bytes) =
        tine_storage::read_optional_regular(&directory, CHECKPOINT_POINTER, 4 * 1024, None)
            .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
    else {
        return Ok(CleanCheckpointOpen::Absent);
    };
    let pointer: CheckpointPointerV2 = match decode_canonical::<CheckpointPointerV2>(&pointer_bytes)
    {
        Ok(pointer) if pointer.schema_version == CHECKPOINT_SCHEMA_VERSION && pointer.slot < 2 => {
            pointer
        }
        Ok(_) | Err(_) => return Ok(invalid("clean checkpoint pointer is invalid")),
    };
    let slot = pointer.slot as usize;
    let generation_bytes = match tine_storage::read_optional_regular(
        &directory,
        CHECKPOINT_GENERATION_NAMES[slot],
        16 * 1024,
        None,
    )
    .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
    {
        Some(bytes) => bytes,
        None => return Ok(invalid("clean checkpoint generation is missing")),
    };
    if ContentDigest::of(&generation_bytes) != pointer.generation_digest {
        return Ok(invalid("clean checkpoint generation digest differs"));
    }
    let generation: CheckpointGenerationV2 =
        match decode_canonical::<CheckpointGenerationV2>(&generation_bytes) {
            Ok(generation)
                if generation.schema_version == CHECKPOINT_SCHEMA_VERSION
                    && generation.slot == pointer.slot
                    && generation.sequence == pointer.sequence =>
            {
                generation
            }
            Ok(_) | Err(_) => return Ok(invalid("clean checkpoint generation is invalid")),
        };
    let payload_bytes = match tine_storage::read_optional_regular(
        &directory,
        CHECKPOINT_PAYLOAD_NAMES[slot],
        MAX_CHECKPOINT_BYTES,
        Some(generation.payload_len),
    )
    .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
    {
        Some(bytes) => bytes,
        None => return Ok(invalid("clean checkpoint payload is missing")),
    };
    if ContentDigest::of(&payload_bytes) != generation.payload_digest {
        return Ok(invalid("clean checkpoint payload digest differs"));
    }
    let payload: CheckpointPayloadV2 = match decode_canonical::<CheckpointPayloadV2>(&payload_bytes)
    {
        Ok(payload) if payload.schema_version == CHECKPOINT_SCHEMA_VERSION => payload,
        Ok(_) | Err(_) => return Ok(invalid("clean checkpoint payload is invalid")),
    };
    if let Err(error) =
        validate_checkpoint_payload_metadata(store, &directory, generation, &payload, None)
    {
        return Ok(invalid(error));
    }
    let state_binding =
        match super::hot_engine::clean_checkpoint_state_binding(&payload.state_bytes) {
            Ok(binding) => Some(binding),
            Err(_) if payload.document_dependencies.is_empty() => None,
            Err(error) => return Ok(invalid(error.to_string())),
        };
    if let Some(state_binding) = state_binding.as_ref() {
        if state_binding.workspace_id != payload.binding.workspace_id
            || state_binding.lineage_digest != payload.binding.lineage_digest
            || state_binding.catalog_document_id != payload.binding.catalog_document_id
            || state_binding.accepted_sequence != payload.recovery_fence.accepted_sequence
            || state_binding.accepted_state_digest != payload.recovery_fence.accepted_state_digest
            || state_binding.eligible_through != payload.recovery_fence.eligible_through
        {
            return Ok(invalid("clean checkpoint state/recovery binding differs"));
        }
    }
    if payload.recovery_fence.accepted_sequence != generation.sequence {
        return Ok(invalid("clean checkpoint roster sequence differs"));
    }
    let sealed_root = payload.sealed_root.locator();
    // D-3: the root a generation names is derived state. A root record that is
    // absent, undecodable, or not the one the binding names is a REBUILD, not
    // a refusal -- `invalid` discards this generation and the store rebuilds
    // from the untouched Markdown/Org originals.
    let accepted_directory = match SealedGenerationDirectory::open_generation(
        &directory,
        sealed_root,
        payload.sealed_root_digest,
    ) {
        Ok(directory) => directory,
        Err(error) => return Ok(invalid(error)),
    };
    let accepted_history = Arc::new(SealedAcceptedHistory {
        directory: accepted_directory,
        sequence: generation.sequence,
        identity_history: Arc::new(SealedIdentityHistory::new(
            match SealedGenerationDirectory::open_generation(
                &directory,
                sealed_root,
                payload.sealed_root_digest,
            ) {
                Ok(directory) => directory,
                Err(error) => return Err(CleanCheckpointOpenError::Store(error)),
            },
        )),
        sequence_enumerations: AtomicUsize::new(0),
        sequence_row_reads: AtomicUsize::new(0),
    });
    let terminal = if generation.sequence != 0 {
        let terminal = match accepted_history.row_by_sequence(generation.sequence) {
            Ok(Some(row)) => row,
            Ok(None) | Err(_) => {
                return Ok(invalid(
                    "clean checkpoint terminal accepted proof is missing",
                ))
            }
        };
        if terminal.evidence.post_frontier_root().state_digest()
            != payload.recovery_fence.accepted_state_digest
        {
            return Ok(invalid(
                "clean checkpoint terminal frontier binding differs",
            ));
        }
        Some(terminal)
    } else {
        None
    };
    let sqlite_anchor = match state_binding
        .as_ref()
        .map(|binding| {
            let generation_sequence = generation.sequence;
            let sqlite = &payload.sqlite_generation;
            let generation = tine_storage::sqlite::PhysicalCheckpointGenerationBinding {
                generation_id: sqlite.generation_id,
                predecessor_generation_id: sqlite.predecessor_generation_id,
                full_anchor_generation_id: sqlite.full_anchor_generation_id,
                covered_count: generation_sequence,
                covered_document_count: binding.accepted_frontier_root.document_count(),
                covered_block_count: sqlite.covered_block_count,
                covered_retained_bytes_total: binding.accepted_frontier_root.retained_bytes_total(),
                covered_semantic_capsules_root_digest: sqlite.covered_semantic_capsules_root_digest,
                // One field for the whole sealed index. The eight it replaces
                // named node addresses inside structures that no longer exist.
                sealed_root_digest: payload.sealed_root_digest,
                covered_head_facts_root_digest: sqlite.covered_head_facts_root_digest,
                current_projection_payload_pins_root_digest: sqlite
                    .current_projection_payload_pins_root_digest,
                nonlinear_state_root_digest: sqlite.nonlinear_state_root_digest,
                retention_pins_root_digest: sqlite.retention_pins_root_digest,
            };
            let canonical_bytes = binding
                .accepted_frontier_root
                .encode_canonical()
                .map_err(|error| error.to_string())?;
            let empty = tine_storage::sealed_accepted_index::AuthenticatedMapRootV1::empty();
            // The candidate is ANCHORED: `1..=covered_sequence` live only in
            // the sealed tables and the treap below spans the tail alone,
            // which at install time is empty. The generation binding's
            // `sealed_root_digest` is the one value naming the covered half.
            let root = tine_storage::sqlite::PhysicalCheckpointFrontierRoot {
                canonical_bytes: canonical_bytes.clone(),
                acceptance_sequence: generation_sequence,
                document_count: binding.accepted_frontier_root.document_count(),
                document_overlay_count: 0,
                retained_bytes_total: binding.accepted_frontier_root.retained_bytes_total(),
                document_map_root_key: None,
                document_map_root_digest: empty.root_digest(),
                batch_map_root_key: None,
                batch_map_root_digest: empty.root_digest(),
                generation: generation.clone(),
                state_digest: binding.accepted_frontier_root.state_digest(),
            };
            let terminal_batch_id = terminal
                .as_ref()
                .map(|row| row.evidence.batch_id().as_uuid().into_bytes());
            let terminal_evidence_digest = terminal
                .as_ref()
                .map(|row| {
                    row.evidence
                        .encode_canonical()
                        .map(|bytes| ContentDigest::of(&bytes))
                        .map_err(|error| error.to_string())
                })
                .transpose()?;
            Ok::<Arc<CleanCheckpointSqliteAnchor>, String>(Arc::new(CleanCheckpointSqliteAnchor {
                anchor: tine_storage::sqlite::PhysicalCheckpointGenerationAnchor {
                    generation,
                    checkpoint_frontier_root: canonical_bytes,
                    terminal_batch_id,
                    terminal_evidence_digest,
                    materialization_frontier_root_digest: root.digest(),
                },
                root,
            }))
        })
        .transpose()
    {
        Ok(anchor) => anchor,
        Err(error) => return Ok(invalid(error)),
    };
    let hot_pin_batches = if payload.document_dependencies.is_empty() {
        BTreeSet::new()
    } else {
        match super::hot_engine::clean_checkpoint_hot_pin_batches(&payload.state_bytes) {
            Ok(pins) => pins,
            Err(error) => return Ok(invalid(error.to_string())),
        }
    };
    store
        .validate_namespace_for_generation(&accepted_history, &hot_pin_batches)
        .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?;
    let mut tail = BTreeSet::new();
    for batch_id in store
        .committed_manifest_names()
        .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
    {
        if !accepted_history
            .contains_batch(batch_id)
            .map_err(CleanCheckpointOpenError::Store)?
        {
            tail.insert(batch_id);
        }
    }
    if logical_cold_history {
        // The recovery caller has already authorized a full-history oracle;
        // the generation remains point-addressable and contributes no loaded
        // covered roster rows here.
        tail.extend(
            store
                .committed_manifest_names_with_cold_history()
                .map_err(|error| CleanCheckpointOpenError::Store(error.to_string()))?
                .into_iter()
                .filter(|batch_id| !accepted_history.contains_batch(*batch_id).unwrap_or(false)),
        );
    }
    let document_dependencies = payload.document_dependencies;
    if !document_dependencies
        .windows(2)
        .all(|pair| pair[0].document_id() < pair[1].document_id())
    {
        return Ok(invalid(
            "clean checkpoint document dependencies are not strictly ordered",
        ));
    }
    let document_roster = SealedDocumentRoster::with_count(payload.document_count);
    let document_directory = match SealedGenerationDirectory::open_generation(
        &directory,
        sealed_root,
        payload.sealed_root_digest,
    ) {
        Ok(directory) => directory,
        Err(error) => return Err(CleanCheckpointOpenError::Store(error)),
    };
    if let Err(error) = document_roster.qualify_complete_keys(
        &document_directory,
        document_dependencies
            .iter()
            .map(DocumentDependencies::document_id),
    ) {
        return Ok(invalid(error));
    }
    for expected in &document_dependencies {
        let actual =
            match document_roster.document_record(&document_directory, expected.document_id()) {
                Ok(Some(record)) => record.dependencies,
                Ok(None) => return Ok(invalid("clean checkpoint document descriptor is missing")),
                Err(error) => return Ok(invalid(error)),
            };
        if &actual != expected {
            return Ok(invalid(
                "clean checkpoint document descriptor binding differs",
            ));
        }
    }
    let payload_size = payload_bytes.len();
    let open_work = GenerationOpenWork {
        covered_sequence_enumerations: accepted_history.sequence_enumerations(),
    };
    Ok(CleanCheckpointOpen::Loaded(CleanCheckpointLoaded {
        state_bytes: payload.state_bytes,
        accepted_history,
        accepted_sequence: generation.sequence,
        tail,
        capture_work: payload.capture_work,
        payload_bytes: payload_size,
        image_work: payload.image_work,
        documents: Arc::new(CleanCheckpointDocuments {
            roster: document_roster,
            directory: document_directory,
            sequence: generation.sequence,
        }),
        sqlite_anchor,
        open_work,
    }))
}

struct PublisherState {
    in_flight: bool,
    in_flight_sequence: u64,
    queued: Option<CleanCheckpointCapture>,
}

struct PublisherInner {
    store: Arc<ObjectStore>,
    publication_authority: Mutex<Option<CheckpointPublicationAuthority>>,
    state: Mutex<PublisherState>,
    finished: Condvar,
    durable_sequence: AtomicU64,
    published_documents: Mutex<BTreeMap<DocumentId, DocumentDependencies>>,
    current_documents: Mutex<Option<Arc<CleanCheckpointDocuments>>>,
    current_identities: Mutex<Option<Arc<SealedIdentityHistory>>>,
    last_identity_publish_work: Mutex<IdentityPublishWork>,
    last_diagnostics: Mutex<Option<SyncCheckpointPublicationDiagnostics>>,
    elevated_rewrite_observed: AtomicBool,
    rebuild_from_genesis: AtomicBool,
}

enum CheckpointPublicationAuthority {
    Workspace(super::sqlite::WorkspaceRuntimePublicationProof),
    #[cfg(test)]
    Test,
}

impl CheckpointPublicationAuthority {
    fn revalidate(&self) -> Result<(), String> {
        match self {
            Self::Workspace(proof) => proof
                .revalidate_identity()
                .map_err(|error| error.to_string()),
            #[cfg(test)]
            Self::Test => Ok(()),
        }
    }
}

fn initial_checkpoint_publication_authority() -> Option<CheckpointPublicationAuthority> {
    #[cfg(test)]
    {
        Some(CheckpointPublicationAuthority::Test)
    }
    #[cfg(not(test))]
    {
        None
    }
}

pub(crate) struct CleanCheckpointPublisher {
    inner: Arc<PublisherInner>,
}

#[cfg(test)]
static LIVE_PUBLISHERS_BY_ARCHIVE: OnceLock<Mutex<BTreeMap<PathBuf, (usize, usize)>>> =
    OnceLock::new();

#[cfg(test)]
fn record_publisher_open(root: &std::path::Path) {
    let mut publishers = LIVE_PUBLISHERS_BY_ARCHIVE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (live, high_water) = publishers.entry(root.to_path_buf()).or_default();
    *live = live.saturating_add(1);
    *high_water = (*high_water).max(*live);
}

#[cfg(test)]
fn record_publisher_close(root: &std::path::Path) {
    let mut publishers = LIVE_PUBLISHERS_BY_ARCHIVE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (live, _) = publishers.entry(root.to_path_buf()).or_default();
    *live = live.saturating_sub(1);
}

#[cfg(test)]
pub(crate) fn publisher_high_water_for_test(root: &std::path::Path) -> usize {
    LIVE_PUBLISHERS_BY_ARCHIVE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(root)
        .map_or(0, |(_, high_water)| *high_water)
}

impl CleanCheckpointPublisher {
    pub(crate) fn new(
        store: ObjectStore,
        durable_sequence: u64,
        published_documents: BTreeMap<DocumentId, DocumentDependencies>,
        current_documents: Option<Arc<CleanCheckpointDocuments>>,
        current_identities: Option<Arc<SealedIdentityHistory>>,
    ) -> Self {
        #[cfg(test)]
        record_publisher_open(store.root_path());
        Self {
            inner: Arc::new(PublisherInner {
                store: Arc::new(store),
                publication_authority: Mutex::new(initial_checkpoint_publication_authority()),
                state: Mutex::new(PublisherState {
                    in_flight: false,
                    in_flight_sequence: durable_sequence,
                    queued: None,
                }),
                finished: Condvar::new(),
                durable_sequence: AtomicU64::new(durable_sequence),
                published_documents: Mutex::new(published_documents),
                current_documents: Mutex::new(current_documents),
                current_identities: Mutex::new(current_identities),
                last_identity_publish_work: Mutex::new(IdentityPublishWork::default()),
                last_diagnostics: Mutex::new(None),
                elevated_rewrite_observed: AtomicBool::new(false),
                rebuild_from_genesis: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn enqueue(&self, capture: CleanCheckpointCapture) {
        let sequence = capture.target_sequence;
        if sequence.saturating_sub(self.durable_sequence()) > CLEAN_CHECKPOINT_LAG_MAX {
            self.inner
                .elevated_rewrite_observed
                .store(true, Ordering::Release);
        }
        let authority = self
            .inner
            .publication_authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.in_flight || authority.is_none() {
            if state
                .queued
                .as_ref()
                .is_none_or(|queued| queued.target_sequence <= capture.target_sequence)
            {
                state.queued = Some(capture);
            }
            return;
        }
        state.in_flight = true;
        state.in_flight_sequence = sequence;
        drop(state);
        drop(authority);
        spawn_publisher(Arc::clone(&self.inner), capture);
    }

    pub(crate) fn install_publication_authority(
        &self,
        proof: super::sqlite::WorkspaceRuntimePublicationProof,
    ) {
        self.install_publication_authority_inner(CheckpointPublicationAuthority::Workspace(proof));
    }

    fn install_publication_authority_inner(&self, authority: CheckpointPublicationAuthority) {
        let mut installed = self
            .inner
            .publication_authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *installed = Some(authority);
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let queued = (!state.in_flight).then(|| state.queued.take()).flatten();
        if let Some(capture) = queued.as_ref() {
            state.in_flight = true;
            state.in_flight_sequence = capture.target_sequence;
        }
        drop(state);
        drop(installed);
        if let Some(capture) = queued {
            spawn_publisher(Arc::clone(&self.inner), capture);
        }
    }

    pub(crate) fn durable_sequence(&self) -> u64 {
        self.inner.durable_sequence.load(Ordering::Acquire)
    }

    /// The newest acceptance sequence already owned by either the running
    /// publication or its single coalesced successor. Schedulers use this
    /// instead of the durable marker so a slow bootstrap cannot cause one
    /// whole-state capture per rapidly accepted batch.
    pub(crate) fn scheduled_sequence(&self) -> u64 {
        let durable = self.durable_sequence();
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let running = state.in_flight.then_some(state.in_flight_sequence);
        let queued = state.queued.as_ref().map(|capture| capture.target_sequence);
        durable
            .max(running.unwrap_or(durable))
            .max(queued.unwrap_or(durable))
    }

    pub(crate) fn last_diagnostics(&self) -> Option<SyncCheckpointPublicationDiagnostics> {
        self.inner
            .last_diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn rebuild_next_from_genesis(&self) {
        self.inner
            .rebuild_from_genesis
            .store(true, Ordering::Release);
    }

    pub(crate) fn published_document_dependencies(
        &self,
    ) -> BTreeMap<DocumentId, DocumentDependencies> {
        self.inner
            .published_documents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn load_current_document(
        &self,
        catalog: DocumentId,
        document: DocumentId,
    ) -> Result<Option<(u64, DocumentDependencies, loro::LoroDoc)>, String> {
        let current = self
            .inner
            .current_documents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        current
            .as_ref()
            .map(|images| {
                images.load_document(catalog, document).map(|loaded| {
                    loaded
                        .map(|(dependencies, document)| (images.sequence(), dependencies, document))
                })
            })
            .transpose()
            .map(Option::flatten)
    }

    pub(crate) fn current_identity_history(&self) -> Option<Arc<SealedIdentityHistory>> {
        self.inner
            .current_identities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
    }

    pub(crate) fn last_identity_publish_work(&self) -> IdentityPublishWork {
        *self
            .inner
            .last_identity_publish_work
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn wait_for_idle(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.in_flight {
            state = self
                .inner
                .finished
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        Ok(())
    }

    pub(crate) fn durable_lag(&self, accepted_sequence: u64) -> u64 {
        accepted_sequence.saturating_sub(self.durable_sequence())
    }

    #[cfg(test)]
    pub(crate) fn elevated_rewrite_observed(&self) -> bool {
        self.inner.elevated_rewrite_observed.load(Ordering::Acquire)
    }
}

impl Drop for CleanCheckpointPublisher {
    fn drop(&mut self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while state.in_flight {
            state = self
                .inner
                .finished
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        #[cfg(test)]
        record_publisher_close(self.inner.store.root_path());
    }
}

fn publisher_loop(inner: Arc<PublisherInner>, mut capture: CleanCheckpointCapture) {
    loop {
        let published_documents = capture
            .documents
            .as_ref()
            .map(|documents| documents.dependencies.clone());
        let rebuild_from_genesis = inner.rebuild_from_genesis.load(Ordering::Acquire);
        let authority = inner
            .publication_authority
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let published = match authority.as_ref() {
            Some(authority) => authority.revalidate().and_then(|()| {
                publish_capture_with_predecessor(
                    &inner.store,
                    capture,
                    !rebuild_from_genesis,
                    Some(authority),
                )
            }),
            None => Err("checkpoint publication has no workspace lease proof".to_owned()),
        };
        drop(authority);
        match published {
            Ok(published) => {
                if rebuild_from_genesis {
                    inner.rebuild_from_genesis.store(false, Ordering::Release);
                }
                if let Some(documents) = published_documents {
                    *inner
                        .published_documents
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = documents;
                }
                if let Some(documents) = published.documents {
                    let mut current = inner
                        .current_documents
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *current = Some(documents);
                }
                *inner
                    .current_identities
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(published.identities);
                *inner
                    .last_identity_publish_work
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    published.identity_publish_work;
                *inner
                    .last_diagnostics
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(published.diagnostics);
                inner
                    .durable_sequence
                    .store(published.sequence, Ordering::Release);
            }
            Err(error) => {
                eprintln!("clean checkpoint write failed; retrying at the next trigger: {error}")
            }
        }
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(next) = state.queued.take() else {
            state.in_flight = false;
            inner.finished.notify_all();
            return;
        };
        state.in_flight_sequence = next.target_sequence;
        capture = next;
    }
}

fn spawn_publisher(inner: Arc<PublisherInner>, capture: CleanCheckpointCapture) {
    let spawn = std::thread::Builder::new()
        .name("tine-clean-checkpoint".into())
        .spawn({
            let worker = Arc::clone(&inner);
            move || publisher_loop(worker, capture)
        });
    if let Err(error) = spawn {
        eprintln!("clean checkpoint writer could not start: {error}");
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.in_flight = false;
        inner.finished.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oplog::hot_engine::{
        accepted_causal_record_digest, authenticated_causal_clock_root, AcceptedFrontierRoot,
    };
    use crate::oplog::{
        BatchCausalDot, BatchId, CausalPeerId, ContentDigest, DeviceId, DocumentKey,
    };
    use tine_storage::sealed_accepted_index::{
        AcceptedStatusRecordV2, SealedAcceptedCausalClockEntryV2, SealedAcceptedCausalRecordV2,
        SealedAcceptedEvidenceDecoder,
    };

    /// One sealed staging store over a private temporary directory.
    ///
    /// This replaces the in-memory `SealedAcceptedIndexObjectStore` doubles the
    /// treap-era tests used. There is no in-memory substitute any more, and
    /// that is deliberate: a sealed cut's cost IS its pack and table writes, so
    /// a double that never touches a pack cannot observe what these tests
    /// exist to observe (I-14, I-25). The `TempDir` must outlive the store.
    fn sealed_test_store() -> (tempfile::TempDir, SealedGenerationStagingStore) {
        let root = tempfile::tempdir().expect("sealed test root");
        let directory =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority())
                .expect("sealed test directory");
        let store = SealedGenerationStagingStore::open(&directory).expect("sealed staging store");
        (root, store)
    }

    #[derive(Clone, Copy, Debug)]
    struct IdentityPointSample {
        micros: u128,
        table_reads: u64,
        value_reads: u64,
        bytes_read: usize,
        allocation_calls: usize,
        allocation_bytes: usize,
    }

    fn identity_percentile(mut values: Vec<u128>, percentile: f64) -> u128 {
        values.sort_unstable();
        let index = ((values.len() as f64 - 1.0) * percentile).round() as usize;
        values[index]
    }

    /// One identity point read against the published sorted tables.
    ///
    /// The treap-depth axis this used to measure is gone with the treap: a
    /// point read is now one table lookup per table the key's domain holds,
    /// which is what `index_reads` counts.
    fn measure_identity_point(
        store: &SealedGenerationDirectory,
        domain: TableDomain,
        key: &[u8],
    ) -> IdentityPointSample {
        let before = store.index_reads();
        let started = Instant::now();
        let (value, (allocation_calls, allocation_bytes)) =
            crate::sync_runtime::tests::c7b_alloc::measure_thread(|| {
                store.table_value(domain, key).unwrap()
            });
        let bytes_read = value.as_ref().map_or(0, Vec::len);
        let value_reads = u64::from(value.is_some());
        std::hint::black_box(value);
        let micros = started.elapsed().as_micros();
        IdentityPointSample {
            micros,
            table_reads: store.index_reads() - before,
            value_reads,
            bytes_read,
            allocation_calls,
            allocation_bytes,
        }
    }

    /// P4b's synthetic identity qualification matrix, rebased onto sorted
    /// tables.
    ///
    /// What it used to measure -- authenticated-map node reads, i.e. treap
    /// depth -- no longer exists (D-2, P4c2). What replaces it is the quantity
    /// that now bounds an identity point read: table records opened. Under
    /// size-tiered compaction at R=8 that is O(log_R N) tables, not O(log2 N)
    /// nodes, and it must not grow with the value width. Run in release mode
    /// and preserve the report.
    #[test]
    #[ignore = "P4b 1k/10k/50k identity point-read measurement; run explicitly in release"]
    fn generation_identity_point_measurement() {
        const SIZES: [usize; 3] = [1_000, 10_000, 50_000];
        const WARM_ROUNDS: usize = 31;
        const MAPS: [(&str, usize, CheckpointIdentityKind); 4] = [
            ("block_home", 160, CheckpointIdentityKind::BlockHome),
            ("logseq_uuid", 192, CheckpointIdentityKind::LogseqUuid),
            ("portable_path", 320, CheckpointIdentityKind::PortablePath),
            ("page_name", 512, CheckpointIdentityKind::PageName),
        ];

        for (map_name, value_len, kind) in MAPS {
            let (_root, mut staging) = sealed_test_store();
            let mut points: BTreeMap<usize, (Vec<u8>, Vec<u8>)> = BTreeMap::new();
            let mut current_key = None;
            let mut released_key = None;
            for ordinal in 0..SIZES[SIZES.len() - 1] {
                let mut key_material = format!("p4b/{map_name}/{ordinal}").into_bytes();
                let key_digest = ContentDigest::of(&key_material);
                let key = key_digest.as_bytes()[..16].to_vec();
                key_material.resize(value_len, (ordinal % 251) as u8);
                let (_, locator) = staging.stage_capsule_blob(&key_material).unwrap();
                let framed = framed_identity_key(&key).unwrap();
                staging
                    .put(kind.complete_domain(), &framed, &locator.to_bytes())
                    .unwrap();
                if ordinal % 2 == 0 {
                    staging
                        .put(kind.current_domain(), &framed, &locator.to_bytes())
                        .unwrap();
                    current_key = Some(key.clone());
                } else {
                    released_key = Some(key.clone());
                }
                let count = ordinal + 1;
                if SIZES.contains(&count) {
                    points.insert(
                        count,
                        (current_key.clone().unwrap(), released_key.clone().unwrap()),
                    );
                }
            }
            let (work, published) = staging.finish().unwrap();
            println!(
                "P4BCUT\tmap={map_name}\tindex_bytes={}\tfiles={}\tpacks={}",
                work.index_bytes, work.files_written, work.packs_published
            );

            for size in SIZES {
                let (current_key, released_key) = points[&size].clone();
                let absent_digest =
                    ContentDigest::of(format!("p4b/{map_name}/{size}/absent").as_bytes());
                let absent_key = absent_digest.as_bytes()[..16].to_vec();
                for (class, domain, key, expected_value_reads) in [
                    ("current", kind.current_domain(), current_key, 1),
                    ("released", kind.complete_domain(), released_key, 1),
                    ("absent", kind.complete_domain(), absent_key, 0),
                ] {
                    let framed = framed_identity_key(&key).unwrap();
                    let cold = measure_identity_point(&published, domain, &framed);
                    let warm = (0..WARM_ROUNDS)
                        .map(|_| measure_identity_point(&published, domain, &framed))
                        .collect::<Vec<_>>();
                    assert_eq!(cold.value_reads, expected_value_reads);
                    // O(log_R N) tables, never the treap's O(log2 N) nodes and
                    // never a function of the value width.
                    assert!(cold.table_reads > 0 && cold.table_reads <= 64);
                    assert!(warm.iter().all(|sample| {
                        sample.value_reads == expected_value_reads
                            && sample.table_reads > 0
                            && sample.table_reads <= 64
                    }));
                    let warm_p50_micros = identity_percentile(
                        warm.iter().map(|sample| sample.micros).collect(),
                        0.50,
                    );
                    let warm_p99_micros = identity_percentile(
                        warm.iter().map(|sample| sample.micros).collect(),
                        0.99,
                    );
                    let warm_p99_reads = identity_percentile(
                        warm.iter()
                            .map(|sample| sample.table_reads as u128)
                            .collect(),
                        0.99,
                    );
                    let warm_p99_bytes = identity_percentile(
                        warm.iter()
                            .map(|sample| sample.bytes_read as u128)
                            .collect(),
                        0.99,
                    );
                    let warm_p99_allocations = identity_percentile(
                        warm.iter()
                            .map(|sample| sample.allocation_calls as u128)
                            .collect(),
                        0.99,
                    );
                    let warm_p99_allocation_bytes = identity_percentile(
                        warm.iter()
                            .map(|sample| sample.allocation_bytes as u128)
                            .collect(),
                        0.99,
                    );
                    println!(
                        "P4BMEASURE\tmap={map_name}\tkeys={size}\tclass={class}\tcache_entries=0\tcold_cache_misses=1\tcold_table_reads={}\tcold_value_reads={}\tcold_bytes={}\tcold_allocations={}\tcold_allocation_bytes={}\tcold_micros={}\twarm_cache_misses=1\twarm_p50_micros={warm_p50_micros}\twarm_p99_micros={warm_p99_micros}\twarm_p99_table_reads={warm_p99_reads}\twarm_p99_bytes={warm_p99_bytes}\twarm_p99_allocations={warm_p99_allocations}\twarm_p99_allocation_bytes={warm_p99_allocation_bytes}\trounds={WARM_ROUNDS}",
                        cold.table_reads,
                        cold.value_reads,
                        cold.bytes_read,
                        cold.allocation_calls,
                        cold.allocation_bytes,
                        cold.micros,
                    );
                }
            }
        }
    }

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_bytes([byte; 32])
    }

    fn evidence() -> AcceptedBatchEvidence {
        let batch_id = BatchId::from_uuid(uuid::Uuid::from_bytes([0x51; 16]));
        AcceptedBatchEvidence::for_test(
            batch_id,
            digest(0x61),
            digest(0x71),
            AcceptedFrontierRoot::empty(),
            Vec::new(),
            Vec::new(),
            vec![(batch_id, digest(0x81))],
            0,
        )
    }

    fn evidence_after(prior: &AcceptedBatchEvidence) -> AcceptedBatchEvidence {
        let first = prior.batch_id();
        let batch_id = BatchId::from_uuid(uuid::Uuid::from_bytes([0x52; 16]));
        AcceptedBatchEvidence::for_test(
            batch_id,
            digest(0x62),
            digest(0x72),
            prior.post_frontier_root().clone(),
            Vec::new(),
            Vec::new(),
            vec![(first, digest(0x81)), (batch_id, digest(0x82))],
            0,
        )
    }

    fn generation_rows(count: u64) -> Vec<CleanCheckpointAcceptedRow> {
        generation_rows_with_dots(&(1..=count).map(|counter| (19, counter)).collect::<Vec<_>>())
    }

    /// Explicit, stable fixture writer incarnation. Fixtures have no durable
    /// writer-lane record to read a real one from; production always does.
    fn fixture_incarnation(seed: u128) -> CausalPeerId {
        CausalPeerId::from_key(WriterIncarnationId::fixture_for_device(
            DeviceId::from_uuid(uuid::Uuid::from_u128(seed)),
        ))
    }

    fn generation_rows_with_dots(dots: &[(u128, u64)]) -> Vec<CleanCheckpointAcceptedRow> {
        let mut prior = AcceptedFrontierRoot::empty();
        let mut entries = Vec::new();
        dots.iter()
            .enumerate()
            .map(|(index, &(peer_id, counter))| {
                let sequence = index as u64 + 1;
                let peer = CausalPeerId::from_key(WriterIncarnationId::from_uuid(
                    uuid::Uuid::from_u128(peer_id),
                ));
                let batch_id = BatchId::from_uuid(uuid::Uuid::from_u128(sequence as u128));
                let fingerprint = ContentDigest::of(&sequence.to_le_bytes());
                let event = ContentDigest::of(&sequence.to_be_bytes());
                let dot = BatchCausalDot::new(peer, counter).unwrap();
                let clock = vec![(peer, counter)];
                let (key, digest) = authenticated_causal_clock_root(&clock).unwrap();
                let causal =
                    accepted_causal_record_digest(batch_id, fingerprint, event, dot, key, digest);
                entries.push((batch_id, causal));
                let evidence = AcceptedBatchEvidence::for_test(
                    batch_id,
                    fingerprint,
                    event,
                    prior.clone(),
                    Vec::new(),
                    Vec::new(),
                    entries.clone(),
                    0,
                );
                prior = evidence.post_frontier_root().clone();
                CleanCheckpointAcceptedRow {
                    no_op: sequence % 2 == 0,
                    evidence,
                    causal_dot: dot,
                    canonical_causal_clock: clock,
                }
            })
            .collect()
    }

    #[test]
    #[ignore = "manual architecture census: fixed live graph with increasing create/delete history"]
    fn rebaselining_constant_live_churn_census() {
        use crate::oplog::hot_engine::{LazyGenesisCheckpointBuilder, ShardedHotEngine};
        use crate::oplog::lazy_genesis::LazyGenesisPackBuilder;
        use crate::oplog::{
            AuthorBatch, BatchDisposition, BlobDescription, BlockId, BlockLocation, CrdtPeerId,
            DocumentId, LineageDigest, LogicalPageName, ManagedPath, ManagedTextKind,
            OperationTransaction, PageId, SemanticOperation, SessionId, WorkspaceId,
        };
        let root = std::env::temp_dir().join(format!("tine-churn-census-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(101));
        let lineage = LineageDigest::of(b"sealed-cutoff-engine");
        let catalog = DocumentId::from_uuid(uuid::Uuid::from_u128(102));
        let (checkpoint, dependencies) = LazyGenesisCheckpointBuilder::new(catalog)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = Arc::new(
            LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog,
                BlobDescription::of(b"empty source"),
                &root,
            )
            .unwrap()
            .finish(checkpoint, dependencies)
            .unwrap(),
        );
        let archive = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        engine
            .install_lazy_genesis_baseline(Arc::clone(&baseline))
            .unwrap();
        engine
            .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
            .unwrap();
        let claims = engine
            .clean_transient_projection_claim_snapshot()
            .unwrap()
            .unwrap();
        let commit = |engine: &mut ShardedHotEngine, sequence: u128, operations| {
            let transaction = OperationTransaction::new(operations).unwrap();
            let prepared = engine
                .prepare_fixture_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(100_000 + sequence)),
                        author_device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(500)),
                        author_session_id: SessionId::from_uuid(uuid::Uuid::from_u128(501)),
                        crdt_peer_id: CrdtPeerId::from_u64(502),
                        causal_peer_id: fixture_incarnation(500),
                    },
                    &transaction,
                )
                .unwrap();
            let result = engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap();
            assert!(
                matches!(result.disposition(), BatchDisposition::Accepted { .. }),
                "{:?}",
                result.disposition()
            );
        };
        let create = |n: u128| {
            vec![
                SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(uuid::Uuid::from_u128(200_000 + n)),
                    home_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(300_000 + n)),
                    name: LogicalPageName::parse(format!("Churn {n}")).unwrap(),
                    path: ManagedPath::parse(format!("pages/Churn{n}.md")).unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(uuid::Uuid::from_u128(400_000 + n)),
                        home_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(300_000 + n)),
                    },
                    page_id: PageId::from_uuid(uuid::Uuid::from_u128(200_000 + n)),
                    parent: None,
                    order: "a".into(),
                    content: "Stable probe content".repeat(8),
                },
            ]
        };
        commit(&mut engine, 1, create(0));
        let live = engine.canonical_snapshot().unwrap();
        assert_eq!(live.pages.len(), 1);
        assert_eq!(live.blocks.len(), 1);
        let mut cutoff = None;
        let capsule_root = root.join("live-closure-capsules");
        std::fs::create_dir(&capsule_root).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&capsule_root, cap_std::ambient_authority())
                .unwrap();
        let mut previous_closure = None;

        for n in 1..=128_u128 {
            commit(&mut engine, 2 * n, create(n));
            commit(
                &mut engine,
                2 * n + 1,
                vec![SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(uuid::Uuid::from_u128(200_000 + n)),
                }],
            );
            if [8, 32, 128].contains(&n) {
                assert_eq!(engine.canonical_snapshot().unwrap(), live);
                // One cut per generation: the accepted delta and this
                // generation's document capsules publish together (I-2).
                let mut staging = SealedGenerationStagingStore::open(&directory).unwrap();
                let current = engine
                    .build_sealed_accepted_cutoff(&mut staging, cutoff.as_ref())
                    .unwrap();
                let compact = engine
                    .build_compact_accepted_document(&current, catalog)
                    .unwrap();
                let pruned_bytes = crate::oplog::hot_engine::probe_pruned_checkpoint_bytes(
                    catalog,
                    compact.dependencies(),
                    &compact.checkpoint().to_vec(),
                )
                .unwrap();
                let closure = engine
                    .capture_live_graph_document_closure(&current)
                    .unwrap();
                assert_eq!(closure.document_count(), 2);
                assert!(closure.contains(catalog));
                assert!(closure.contains(DocumentId::from_uuid(uuid::Uuid::from_u128(300_000))));
                if let Some(previous) = &previous_closure {
                    assert!(engine
                        .build_live_graph_document_roster(&current, &mut staging, previous)
                        .is_err());
                }
                let active = engine
                    .build_live_graph_document_roster(&current, &mut staging, &closure)
                    .unwrap();
                assert_eq!(active.document_count(), 2);
                let (_cut_work, disk) = staging.finish().unwrap();
                engine
                    .qualify_live_graph_document_roster(&current, active, &disk, &closure)
                    .unwrap();
                assert!(engine
                    .qualify_full_document_roster(&current, active, &disk)
                    .is_err());
                drop(disk);
                eprintln!("rebaselining_churn cycles={n} live_pages=1 live_blocks=1 accepted={} accepted_documents={} compact_catalog_bytes={} live_capsules=2 experimental_pruned_bytes={pruned_bytes}",
                    current.frontier().acceptance_sequence(), current.frontier().document_count(), compact.checkpoint().len());
                previous_closure = Some(closure);
                cutoff = Some(current);
            }
        }
        // A bounded document count is insufficient too: churn inside one
        // still-live home shard can retain deleted block state and text.
        let page = PageId::from_uuid(uuid::Uuid::from_u128(200_000));
        let home = DocumentId::from_uuid(uuid::Uuid::from_u128(300_000));
        for n in 1..=128_u128 {
            let block_id = BlockId::from_uuid(uuid::Uuid::from_u128(500_000 + n));
            commit(
                &mut engine,
                1_000 + 2 * n,
                vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: home,
                    },
                    page_id: page,
                    parent: None,
                    order: "b".into(),
                    content: "Deleted block probe content".repeat(8),
                }],
            );
            commit(
                &mut engine,
                1_001 + 2 * n,
                vec![SemanticOperation::DeleteSubtree {
                    root_block_id: block_id,
                    page_id: page,
                }],
            );
            if [8, 32, 128].contains(&n) {
                assert_eq!(engine.canonical_snapshot().unwrap(), live);
                let mut staging = SealedGenerationStagingStore::open(&directory).unwrap();
                let current = engine
                    .build_sealed_accepted_cutoff(&mut staging, cutoff.as_ref())
                    .unwrap();
                let compact = engine
                    .build_compact_accepted_document(&current, home)
                    .unwrap();
                let pruned_bytes = crate::oplog::hot_engine::probe_pruned_checkpoint_bytes(
                    catalog,
                    compact.dependencies(),
                    &compact.checkpoint().to_vec(),
                )
                .unwrap();
                let closure = engine
                    .capture_live_graph_document_closure(&current)
                    .unwrap();
                assert_eq!(closure.document_count(), 2);
                eprintln!("rebaselining_block_churn cycles={n} live_pages=1 live_blocks=1 accepted={} accepted_documents={} compact_home_bytes={} live_capsules=2 experimental_pruned_bytes={pruned_bytes}",
                    current.frontier().acceptance_sequence(), current.frontier().document_count(), compact.checkpoint().len());
                cutoff = Some(current);
            }
        }
        drop(directory);
        drop(engine);
        drop(archive);
        drop(baseline);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_cutoff_streams_real_engine_evidence_and_matches_clean_replay() {
        use crate::oplog::hot_engine::{LazyGenesisCheckpointBuilder, ShardedHotEngine};
        use crate::oplog::lazy_genesis::LazyGenesisPackBuilder;
        use crate::oplog::BlobDescription;
        use crate::oplog::{
            AuthorBatch, BatchDisposition, CrdtPeerId, DocumentId, LineageDigest, LogicalPageName,
            ManagedPath, ManagedTextKind, OperationTransaction, PageId, SemanticOperation,
            SessionId, WorkspaceId,
        };
        let root =
            std::env::temp_dir().join(format!("tine-sealed-cutoff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(101));
        let lineage = LineageDigest::of(b"sealed-cutoff-engine");
        let catalog = DocumentId::from_uuid(uuid::Uuid::from_u128(102));
        let (checkpoint, dependencies) = LazyGenesisCheckpointBuilder::new(catalog)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = Arc::new(
            LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog,
                BlobDescription::of(b"empty source"),
                &root,
            )
            .unwrap()
            .finish(checkpoint, dependencies)
            .unwrap(),
        );
        let archive = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        engine
            .install_lazy_genesis_baseline(Arc::clone(&baseline))
            .unwrap();
        engine
            .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
            .unwrap();
        let claims = engine
            .clean_transient_projection_claim_snapshot()
            .unwrap()
            .unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut cutoff = engine
            .build_sealed_accepted_cutoff(&mut store, None)
            .unwrap();
        assert_eq!(cutoff.sequence(), 0);
        let capsule_root = root.join("capsules");
        std::fs::create_dir(&capsule_root).unwrap();
        let capsule_dir =
            cap_std::fs::Dir::open_ambient_dir(&capsule_root, cap_std::ambient_authority())
                .unwrap();
        // A SECOND sealed directory for the engine-driven incremental census.
        // It must carry the PREDECESSOR generation and nothing newer, which is
        // what a real cut sees; `capsule_dir` below also receives this
        // iteration's hand-built roster before the engine runs, so reusing it
        // would let the engine inherit its own answer and write nothing.
        let census_root = root.join("census");
        std::fs::create_dir(&census_root).unwrap();
        let census_dir =
            cap_std::fs::Dir::open_ambient_dir(&census_root, cap_std::ambient_authority()).unwrap();
        let mut roster = SealedDocumentRoster::empty();
        let mut empty_store = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
        let (empty_roster, empty_written) = engine
            .build_compact_document_roster(&cutoff, &mut empty_store, None)
            .unwrap();
        assert_eq!(empty_written, cutoff.frontier().document_count());
        let (_, empty_disk) = empty_store.finish().unwrap();
        engine
            .qualify_full_document_roster(&cutoff, empty_roster, &empty_disk)
            .unwrap();
        drop(empty_disk);

        for n in 1..=2 {
            let transaction = OperationTransaction::new(vec![
                SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(uuid::Uuid::from_u128(200 + n)),
                    home_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(300 + n)),
                    name: LogicalPageName::parse(format!("Page {n}")).unwrap(),
                    path: ManagedPath::parse(format!("pages/Page{n}.md")).unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SemanticOperation::CreateBlock {
                    block: crate::oplog::BlockLocation {
                        block_id: crate::oplog::BlockId::from_uuid(uuid::Uuid::from_u128(600 + n)),
                        home_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(300 + n)),
                    },
                    page_id: PageId::from_uuid(uuid::Uuid::from_u128(200 + n)),
                    parent: None,
                    order: "a".into(),
                    content: format!("Nested CRDT text {n}"),
                },
            ])
            .unwrap();
            let prepared = engine
                .prepare_fixture_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(400 + n)),
                        author_device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(500)),
                        author_session_id: SessionId::from_uuid(uuid::Uuid::from_u128(501)),
                        crdt_peer_id: CrdtPeerId::from_u64(502),
                        causal_peer_id: fixture_incarnation(500),
                    },
                    &transaction,
                )
                .unwrap();
            let outcome = engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap();
            assert!(
                matches!(outcome.disposition(), BatchDisposition::Accepted { .. }),
                "{:?}",
                outcome.disposition()
            );
            assert!(engine
                .build_compact_accepted_document(&cutoff, catalog)
                .is_err());
            let before = engine.capture_clean_checkpoint(0).unwrap().state_bytes;
            let manifests = archive.committed_manifest_names().unwrap();
            cutoff = engine
                .build_sealed_accepted_cutoff(&mut store, Some(&cutoff))
                .unwrap();
            assert_eq!(cutoff.sequence(), n as u64);
            let retained = engine
                .build_policy_compact_accepted_document_at_cutoff(
                    &cutoff,
                    catalog,
                    0,
                    crate::oplog::checkpoint_floor_policy::FloorPolicyConfig::default(),
                )
                .unwrap();
            let crate::oplog::checkpoint_floor_policy::LoroFloorDecision::Keep {
                retained: catalog_image,
                metrics: catalog_metrics,
                work: catalog_work,
            } = retained.decision()
            else {
                panic!("a fresh default-budget catalog must retain full history")
            };
            assert!(!catalog_image.checkpoint.is_empty());
            assert_eq!(catalog_image.actual_floor, loro::Frontiers::default());
            assert!(catalog_metrics.image_bytes > 0);
            assert_eq!(catalog_work.measurement_exports, 2);
            assert_eq!(
                retained.cutoff_state_digest(),
                cutoff.frontier().state_digest()
            );
            assert_eq!(retained.dependencies().document_id(), catalog);
            let page_retained = engine
                .build_policy_compact_accepted_document_at_cutoff(
                    &cutoff,
                    DocumentId::from_uuid(uuid::Uuid::from_u128(300 + n)),
                    0,
                    crate::oplog::checkpoint_floor_policy::FloorPolicyConfig::default(),
                )
                .unwrap();
            let crate::oplog::checkpoint_floor_policy::LoroFloorDecision::Keep {
                retained: page_image,
                ..
            } = page_retained.decision()
            else {
                panic!("a fresh default-budget page must retain full history")
            };
            assert!(!page_image.checkpoint.is_empty());
            // From the second round on, the roster RESUMES the generation the
            // directories already published, exactly as a real cut does; a
            // census that does not say so re-counts every document it rewrites.
            let previous_roster = SealedDocumentRoster::with_count(roster.document_count());
            if n > 1 {
                roster = previous_roster;
            }
            let mut capsule_store = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            for id in [
                catalog,
                DocumentId::from_uuid(uuid::Uuid::from_u128(300 + n)),
            ] {
                let compact = engine.build_compact_accepted_document(&cutoff, id).unwrap();
                assert_eq!(
                    compact.cutoff_state_digest(),
                    cutoff.frontier().state_digest()
                );
                assert_eq!(compact.dependencies().document_id(), id);
                let restored = loro::LoroDoc::new();
                assert!(restored
                    .import(compact.checkpoint())
                    .unwrap()
                    .pending
                    .is_none());
                assert!(!compact.checkpoint().is_empty());
                roster = roster
                    .with_document(&mut capsule_store, &cutoff, &compact)
                    .unwrap();
                let record = DocumentCapsuleRecord {
                    schema: DOCUMENT_CAPSULE_SCHEMA,
                    dependencies: compact.dependencies().clone(),
                    checkpoint: BlobDescription::of(compact.checkpoint()),
                    policy: DocumentCheckpointPolicyV1::uncut(compact.checkpoint().len()).unwrap(),
                };
                let canonical = record.encode().unwrap();
                assert_eq!(DocumentCapsuleRecord::decode(&canonical).unwrap(), record);
                let mut trailing = canonical.clone();
                trailing.push(0);
                assert!(DocumentCapsuleRecord::decode(&trailing).is_err());
                let mut wrong_schema = record;
                wrong_schema.schema += 1;
                assert!(DocumentCapsuleRecord::decode(&wrong_schema.encode().unwrap()).is_err());
            }
            drop(capsule_store.finish().unwrap());
            let reopened_capsules = SealedGenerationDirectory::open(&capsule_dir).unwrap();
            for id in std::iter::once(catalog)
                .chain((1..=n).map(|i| DocumentId::from_uuid(uuid::Uuid::from_u128(300 + i))))
            {
                let (dependencies, restored) = roster
                    .load_document(&reopened_capsules, catalog, id)
                    .unwrap()
                    .unwrap();
                let compact = engine.build_compact_accepted_document(&cutoff, id).unwrap();
                let expected = super::super::hot_engine::qualify_compact_document(
                    catalog,
                    compact.dependencies(),
                    &compact.checkpoint().to_vec(),
                )
                .unwrap();
                assert_eq!(&dependencies, compact.dependencies());
                assert_eq!(restored.get_deep_value(), expected.get_deep_value());
                assert_eq!(restored.oplog_frontiers(), expected.oplog_frontiers());
            }
            assert_eq!(roster.document_count(), n as u64 + 1);
            if n == 2 {
                let id = DocumentId::from_uuid(uuid::Uuid::from_u128(301));
                let old = previous_roster
                    .load_document(&reopened_capsules, catalog, id)
                    .unwrap()
                    .unwrap();
                let new = roster
                    .load_document(&reopened_capsules, catalog, id)
                    .unwrap()
                    .unwrap();
                assert_eq!(old.0, new.0);
                assert_eq!(old.1.get_deep_value(), new.1.get_deep_value());
            }
            let mut complete_store = SealedGenerationStagingStore::open(&census_dir).unwrap();
            let (automatic, written) = engine
                .build_compact_document_roster(
                    &cutoff,
                    &mut complete_store,
                    if n == 1 { None } else { Some(previous_roster) },
                )
                .unwrap();
            assert_eq!(written, 2, "only catalog plus new page need compaction");
            assert_eq!(automatic.map, roster.map);
            drop(complete_store.finish().unwrap());
            engine
                .qualify_full_document_roster(&cutoff, automatic, &reopened_capsules)
                .unwrap();
            assert!(engine
                .qualify_full_document_roster(
                    &cutoff,
                    SealedDocumentRoster::empty(),
                    &reopened_capsules
                )
                .is_err());
            let mut unchanged = SealedGenerationStagingStore::open(&census_dir).unwrap();
            let (same, written) = engine
                .build_compact_document_roster(&cutoff, &mut unchanged, Some(automatic))
                .unwrap();
            assert_eq!(written, 0);
            assert_eq!(same.map, automatic.map);
            // An unchanged roster must cost NOTHING: no table entry, no
            // packed record. This is the I-25 statement of "written = 0"
            // above, at the physical layer the old `pending.objects` check
            // was reaching for.
            assert_eq!(unchanged.entry_writes(), 0);
            assert_eq!(unchanged.record_writes(), 0);
            drop(unchanged);

            // The descriptor is a packed record addressed by a table entry,
            // not a file. Read its locator out of the roster domain.
            let catalog_key = framed_document_key(
                DocumentKey::Entity(catalog)
                    .authenticated_map_key()
                    .as_slice(),
            )
            .unwrap();
            let descriptor_locator = locator_from_value(
                &reopened_capsules
                    .table_value(DOMAIN_DOCUMENT_ROSTER, &catalog_key)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let exact = reopened_capsules
                .sealed_record(descriptor_locator, MAX_CHECKPOINT_BYTES)
                .unwrap();
            // A roster carrying one EXTRA document must not qualify, even when
            // its declared count is the honest one: the count alone never
            // certifies completeness.
            let mut extra_store = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            let extra_id = DocumentId::from_uuid(uuid::Uuid::from_u128(888_888));
            let extra_key = framed_document_key(
                DocumentKey::Entity(extra_id)
                    .authenticated_map_key()
                    .as_slice(),
            )
            .unwrap();
            extra_store
                .put(
                    DOMAIN_DOCUMENT_ROSTER,
                    &extra_key,
                    &descriptor_locator.to_bytes(),
                )
                .unwrap();
            let (_, forged) = extra_store.finish().unwrap();
            assert!(engine
                .qualify_full_document_roster(
                    &cutoff,
                    SealedDocumentRoster::with_count(roster.document_count()),
                    &forged
                )
                .is_err());
            // The forged row is now PUBLISHED in this directory. Tombstone it
            // again, or the next round's census counts it as an accepted
            // document -- the roster domain is exactly the live key set.
            let mut unforge = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            unforge.remove(DOMAIN_DOCUMENT_ROSTER, &extra_key).unwrap();
            drop(unforge.finish().unwrap());

            let record = DocumentCapsuleRecord::decode(&exact).unwrap();
            // Tear the IMAGE record inside its pack: a torn write or a partial
            // sync delivery damages bytes in place, it does not unlink a file.
            let image_digest = ContentDigest::from_bytes(*record.checkpoint.sha256());
            let image_locator = locator_from_value(
                &reopened_capsules
                    .table_value(DOMAIN_CAPSULE_BLOB, image_digest.as_bytes())
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            reopened_capsules.tear_packed_record(image_locator).unwrap();
            assert!(roster
                .load_document(&reopened_capsules, catalog, catalog)
                .is_err());
            // Tearing the DESCRIPTOR record is the other half of the same
            // scenario and must also refuse rather than answer.
            let (_repair_root, repaired) = {
                let mut repair = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
                let _ = repair.stage_capsule_blob(&exact).unwrap();
                repair.finish().unwrap()
            };
            repaired.tear_packed_record(descriptor_locator).unwrap();
            assert!(roster.load_document(&repaired, catalog, catalog).is_err());
            // Validly addressed but semantically wrong bytes must not qualify.
            let mut malformed = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            let (bad_checkpoint, _) = malformed
                .stage_capsule_blob(b"not a CRDT checkpoint")
                .unwrap();
            let bad_record = DocumentCapsuleRecord {
                checkpoint: bad_checkpoint,
                ..record.clone()
            };
            let (_, bad_locator) = malformed
                .stage_capsule_blob(&bad_record.encode().unwrap())
                .unwrap();
            let mut wrong_vector = record.dependencies.peer_counters().to_vec();
            wrong_vector.push(crate::oplog::CrdtPeerCounter::new(
                CrdtPeerId::from_u64(999_999),
                0,
            ));
            let wrong_dependencies = DocumentDependencies::new(
                catalog,
                wrong_vector,
                record.dependencies.direct_dependency_heads().to_vec(),
            )
            .unwrap();
            let wrong_record = DocumentCapsuleRecord {
                dependencies: wrong_dependencies,
                ..record.clone()
            };
            let (_, wrong_locator) = malformed
                .stage_capsule_blob(&wrong_record.encode().unwrap())
                .unwrap();
            malformed
                .put(
                    DOMAIN_DOCUMENT_ROSTER,
                    &catalog_key,
                    &bad_locator.to_bytes(),
                )
                .unwrap();
            let (_, with_bad) = malformed.finish().unwrap();
            assert!(SealedDocumentRoster::with_count(roster.document_count())
                .load_document(&with_bad, catalog, catalog)
                .is_err());
            let mut malformed = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            malformed
                .put(
                    DOMAIN_DOCUMENT_ROSTER,
                    &catalog_key,
                    &wrong_locator.to_bytes(),
                )
                .unwrap();
            let (_, with_wrong) = malformed.finish().unwrap();
            assert!(SealedDocumentRoster::with_count(roster.document_count())
                .load_document(&with_wrong, catalog, catalog)
                .is_err());
            // Restore the honest entry so the loop's next round sees a healthy
            // roster, and prove it reads again.
            let mut restore = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
            // Both the image record and the descriptor record were torn in
            // place above; dedup would hand their damaged locators straight
            // back, so recovery re-appends the exact bytes.
            let healed_image = engine
                .build_compact_accepted_document(&cutoff, catalog)
                .unwrap();
            let (_, _) = restore
                .restage_capsule_blob(healed_image.checkpoint())
                .unwrap();
            let (_, restored_descriptor) = restore.restage_capsule_blob(&exact).unwrap();
            restore
                .put(
                    DOMAIN_DOCUMENT_ROSTER,
                    &catalog_key,
                    &restored_descriptor.to_bytes(),
                )
                .unwrap();
            let (_, reopened_capsules) = restore.finish().unwrap();
            assert!(roster
                .load_document(&reopened_capsules, catalog, catalog)
                .unwrap()
                .is_some());
            drop(reopened_capsules);
            assert_eq!(cutoff.frontier(), &engine.accepted_frontier_root().unwrap());
            assert_eq!(archive.committed_manifest_names().unwrap(), manifests);
            assert_eq!(
                engine.capture_clean_checkpoint(0).unwrap().state_bytes,
                before
            );
        }
        let mut replay = ShardedHotEngine::new(workspace, lineage, catalog);
        replay.install_lazy_genesis_baseline(baseline).unwrap();
        replay
            .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
            .unwrap();
        assert_eq!(
            replay.replay_clean_committed_tail(claims.as_ref()).unwrap(),
            2
        );
        let independent = replay
            .build_sealed_accepted_cutoff(&mut sealed_test_store().1, None)
            .unwrap();
        assert_eq!(cutoff.identity_digest(), independent.identity_digest());
        assert_eq!(cutoff.frontier(), independent.frontier());
        let mut disk = SealedGenerationStagingStore::open(&capsule_dir).unwrap();
        // Over the SAME base the incremental roster was built on. The census is
        // the store's live document rows, not a run-local tally, so a roster
        // that starts empty over a store that already holds those rows counts
        // nothing: `with_document` correctly sees every row as already present.
        let mut full_roster = SealedDocumentRoster::over_base(&disk).unwrap();
        for id in [
            catalog,
            DocumentId::from_uuid(uuid::Uuid::from_u128(301)),
            DocumentId::from_uuid(uuid::Uuid::from_u128(302)),
        ] {
            let compact = replay
                .build_compact_accepted_document(&independent, id)
                .unwrap();
            full_roster = full_roster
                .with_document(&mut disk, &independent, &compact)
                .unwrap();
        }
        assert_eq!(full_roster.map, roster.map);
        drop(disk.finish().unwrap());
        drop(census_dir);
        drop(capsule_dir);
        drop(replay);
        drop(engine);
        drop(archive);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn rebaselining_reconstructs_real_engine_ancestry_for_two_returning_peers() {
        use crate::oplog::hot_engine::{LazyGenesisCheckpointBuilder, ShardedHotEngine};
        use crate::oplog::lazy_genesis::LazyGenesisPackBuilder;
        use crate::oplog::{
            AuthorBatch, BatchDisposition, BlobDescription, BlockId, BlockLocation, CrdtPeerId,
            DocumentId, LineageDigest, LogicalPageName, ManagedPath, ManagedTextKind,
            OperationTransaction, PageId, SemanticOperation, SessionId, WorkspaceId,
        };
        let root = std::env::temp_dir().join(format!(
            "tine-rebaseline-returning-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(1101));
        let lineage = LineageDigest::of(b"real-engine-returning-peers");
        let catalog = DocumentId::from_uuid(uuid::Uuid::from_u128(1102));
        let home = DocumentId::from_uuid(uuid::Uuid::from_u128(1103));
        let page = PageId::from_uuid(uuid::Uuid::from_u128(1104));
        let destination = PageId::from_uuid(uuid::Uuid::from_u128(1106));
        let destination_home = DocumentId::from_uuid(uuid::Uuid::from_u128(1107));
        let block = BlockLocation {
            block_id: BlockId::from_uuid(uuid::Uuid::from_u128(1105)),
            home_document_id: home,
        };
        let (bytes, dependencies) = LazyGenesisCheckpointBuilder::new(catalog)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = Arc::new(
            LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog,
                BlobDescription::of(b"empty source"),
                &root,
            )
            .unwrap()
            .finish(bytes, dependencies)
            .unwrap(),
        );
        // One scratch archive collects the exact immutable bytes accepted by
        // the simulated devices. No production inbound publication is implied.
        let archive = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let fresh = || {
            let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
            engine
                .install_lazy_genesis_baseline(Arc::clone(&baseline))
                .unwrap();
            engine
                .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
                .unwrap();
            engine
        };
        let mut engine = fresh();
        let claims = engine
            .clean_transient_projection_claim_snapshot()
            .unwrap()
            .unwrap();
        let author = |batch: u128, peer: u64| AuthorBatch {
            batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(batch)),
            author_device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(peer as u128)),
            author_session_id: SessionId::from_uuid(uuid::Uuid::from_u128(peer as u128 + 1)),
            crdt_peer_id: CrdtPeerId::from_u64(peer),
            causal_peer_id: fixture_incarnation(peer as u128),
        };
        let commit = |engine: &mut ShardedHotEngine, batch, peer, operations| {
            let transaction = OperationTransaction::new(operations).unwrap();
            let prepared = engine
                .prepare_fixture_transaction(author(batch, peer), &transaction)
                .unwrap();
            let outcome = engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap();
            assert!(
                matches!(outcome.disposition(), BatchDisposition::Accepted { .. }),
                "{:?}",
                outcome.disposition()
            );
        };
        commit(
            &mut engine,
            1,
            100,
            vec![
                SemanticOperation::CreatePage {
                    page_id: page,
                    home_document_id: home,
                    name: LogicalPageName::parse("Recovery").unwrap(),
                    path: ManagedPath::parse("pages/Recovery.md").unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SemanticOperation::CreatePage {
                    page_id: destination,
                    home_document_id: destination_home,
                    name: LogicalPageName::parse("Destination").unwrap(),
                    path: ManagedPath::parse("pages/Destination.md").unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SemanticOperation::CreateBlock {
                    block,
                    page_id: page,
                    parent: None,
                    order: "a".into(),
                    content: "root".into(),
                },
            ],
        );
        let mut offline_a = fresh();
        let mut offline_b = fresh();
        for offline in [&mut offline_a, &mut offline_b] {
            assert_eq!(
                offline
                    .replay_clean_committed_tail(claims.as_ref())
                    .unwrap(),
                1
            );
        }
        commit(
            &mut engine,
            2,
            100,
            vec![
                SemanticOperation::MoveSubtree {
                    root: block,
                    from_page_id: page,
                    to_page_id: destination,
                    parent: None,
                    order: "a".into(),
                },
                SemanticOperation::EditBlockContent {
                    block,
                    content: "root MAIN".into(),
                },
            ],
        );
        let content = |engine: &ShardedHotEngine| {
            engine
                .canonical_snapshot()
                .unwrap()
                .blocks
                .into_iter()
                .find(|state| state.block_id == block.block_id)
                .unwrap()
                .content
        };
        let mut accepted = BTreeSet::from([author(1, 100).batch_id, author(2, 100).batch_id]);
        for (round, offline, offline_peer, offline_label, tail_label) in [
            (0, &mut offline_a, 200, "OFFLINE_A", "TAIL"),
            (1, &mut offline_b, 300, "OFFLINE_B", "NEXT"),
        ] {
            let cutoff = engine
                .build_sealed_accepted_cutoff(&mut sealed_test_store().1, None)
                .unwrap();
            let compact = engine
                .build_compact_accepted_document(&cutoff, home)
                .unwrap();
            let compact_bytes = compact.checkpoint().to_vec();
            let tail_id = 3 + round * 2;
            let incoming_id = tail_id + 1;
            let updated = format!("{} {tail_label}", content(&engine));
            commit(
                &mut engine,
                tail_id,
                100,
                vec![SemanticOperation::EditBlockContent {
                    block,
                    content: updated,
                }],
            );
            accepted.insert(author(tail_id, 100).batch_id);
            let acknowledged = engine.canonical_snapshot().unwrap();
            let acknowledged_root = engine.accepted_frontier_root().unwrap();
            commit(
                offline,
                incoming_id,
                offline_peer,
                vec![SemanticOperation::EditBlockContent {
                    block,
                    content: format!("root {offline_label}"),
                }],
            );

            // Reconstruct *all* acknowledged ancestry in an isolated engine,
            // including the tail accepted after C. Then use the production
            // admission path for the returning batch, not a shallow import.
            let mut recovered = fresh();
            assert_eq!(
                recovered
                    .replay_clean_checkpoint_tail(&accepted, claims.as_ref())
                    .unwrap(),
                accepted.len()
            );
            assert_eq!(recovered.canonical_snapshot().unwrap(), acknowledged);
            assert_eq!(
                recovered.accepted_frontier_root().unwrap(),
                acknowledged_root
            );
            let incoming = BTreeSet::from([author(incoming_id, offline_peer).batch_id]);
            assert_eq!(
                recovered
                    .replay_clean_checkpoint_tail(&incoming, claims.as_ref())
                    .unwrap(),
                1
            );
            assert_eq!(
                engine.canonical_snapshot().unwrap(),
                acknowledged,
                "isolated recovery mutated live state"
            );
            assert_eq!(
                compact.checkpoint(),
                compact_bytes,
                "old compact bytes changed"
            );
            for preserved in ["MAIN", "TAIL", offline_label] {
                assert!(
                    content(&recovered).contains(preserved),
                    "recovery lost {preserved}"
                );
            }
            if round == 1 {
                assert!(content(&recovered).contains("OFFLINE_A"));
                assert!(content(&recovered).contains("NEXT"));
            }
            let snapshot = recovered.canonical_snapshot().unwrap();
            let membership = snapshot
                .memberships
                .iter()
                .find(|entry| entry.block_id == block.block_id)
                .unwrap();
            assert_eq!(membership.page_id, destination);
            assert_eq!(membership.home_document_id, home);
            assert_eq!(
                snapshot
                    .blocks
                    .iter()
                    .find(|entry| entry.block_id == block.block_id)
                    .unwrap()
                    .home_document_id,
                home
            );

            // Negative control: rebuilding only through C and then accepting
            // the offline branch loses the acknowledged post-C tail. The
            // recovery protocol must explicitly carry that tail forward.
            let mut omitted_tail = fresh();
            let mut incomplete = accepted.clone();
            incomplete.remove(&author(tail_id, 100).batch_id);
            omitted_tail
                .replay_clean_checkpoint_tail(&incomplete, claims.as_ref())
                .unwrap();
            omitted_tail
                .replay_clean_checkpoint_tail(&incoming, claims.as_ref())
                .unwrap();
            assert!(!content(&omitted_tail).contains(tail_label));
            assert_ne!(omitted_tail.canonical_snapshot().unwrap(), snapshot);
            accepted.extend(incoming);
            let next = recovered
                .build_sealed_accepted_cutoff(&mut sealed_test_store().1, None)
                .unwrap();
            for id in [catalog, home, destination_home] {
                let next_compact = recovered
                    .build_compact_accepted_document(&next, id)
                    .unwrap();
                assert_eq!(next_compact.dependencies().document_id(), id);
            }
            assert_eq!(next.sequence() as usize, accepted.len());
            // Only the test's engine handle moves here. Durable marker/actor
            // installation remains a separate production qualification gate.
            engine = recovered;
        }
        let mut oracle = fresh();
        assert_eq!(
            oracle.replay_clean_committed_tail(claims.as_ref()).unwrap(),
            6
        );
        assert_eq!(
            oracle.canonical_snapshot().unwrap(),
            engine.canonical_snapshot().unwrap()
        );
        assert_eq!(
            oracle.accepted_frontier_root().unwrap(),
            engine.accepted_frontier_root().unwrap()
        );
        assert_eq!(
            archive
                .committed_manifest_names_with_cold_history()
                .unwrap(),
            accepted
        );
        // Delete the now-empty original page. Its immutable home still owns
        // the moved block and must survive even though it is not a live page.
        commit(
            &mut engine,
            7,
            100,
            vec![SemanticOperation::DeletePage { page_id: page }],
        );
        assert_eq!(engine.canonical_snapshot().unwrap().pages.len(), 1);
        assert!(content(&engine).contains("OFFLINE_A"));
        assert!(content(&engine).contains("OFFLINE_B"));
        let final_cutoff = engine
            .build_sealed_accepted_cutoff(&mut sealed_test_store().1, None)
            .unwrap();
        let capsule_root = root.join("retained-home-capsules");
        std::fs::create_dir(&capsule_root).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&capsule_root, cap_std::ambient_authority())
                .unwrap();
        let mut staging = SealedGenerationStagingStore::open(&directory).unwrap();
        let (complete, written) = engine
            .build_compact_document_roster(&final_cutoff, &mut staging, None)
            .unwrap();
        assert_eq!(written, 3);
        let (_, disk) = staging.finish().unwrap();
        engine
            .qualify_full_document_roster(&final_cutoff, complete, &disk)
            .unwrap();
        assert!(complete
            .load_document(&disk, catalog, home)
            .unwrap()
            .is_some());
        let closure = engine
            .capture_live_graph_document_closure(&final_cutoff)
            .unwrap();
        assert_eq!(closure.document_count(), 3);
        assert!(closure.contains(home));
        let mut live_store = SealedGenerationStagingStore::open(&directory).unwrap();
        let live_roster = engine
            .build_live_graph_document_roster(&final_cutoff, &mut live_store, &closure)
            .unwrap();
        drop(live_store.finish().unwrap());
        engine
            .qualify_live_graph_document_roster(&final_cutoff, live_roster, &disk, &closure)
            .unwrap();
        let mut incomplete_store = SealedGenerationStagingStore::open(&directory).unwrap();
        let mut visible_only = SealedDocumentRoster::empty();
        for id in [catalog, destination_home] {
            let compact = engine
                .build_compact_accepted_document(&final_cutoff, id)
                .unwrap();
            visible_only = visible_only
                .with_document(&mut incomplete_store, &final_cutoff, &compact)
                .unwrap();
        }
        drop(incomplete_store.finish().unwrap());
        assert!(engine
            .qualify_full_document_roster(&final_cutoff, visible_only, &disk)
            .is_err());
        assert!(engine
            .qualify_live_graph_document_roster(&final_cutoff, visible_only, &disk, &closure,)
            .is_err());
        drop(disk);
        drop(directory);
        drop(oracle);
        drop(offline_a);
        drop(offline_b);
        drop(engine);
        drop(archive);
        drop(baseline);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_directory_roundtrip_and_incremental_roots_match_memory_oracle() {
        let root =
            std::env::temp_dir().join(format!("tine-sealed-directory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let rows = generation_rows(17);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let mut staging = SealedGenerationStagingStore::open(&directory).unwrap();
        let mut cutoff = empty.clone();
        for row in &rows[..16] {
            let mut builder = cutoff.builder(&mut staging);
            builder.append(row).unwrap();
            cutoff = builder.finish(row.evidence.post_frontier_root()).unwrap();
        }
        let prefix = cutoff.clone();
        // Sixteen accepted rows: two records each, and ONE cut. The old byte
        // and object budgets are gone with the staging batcher -- what bounds
        // a cut now is that it writes O(1) files regardless of row count
        // (I-14/T2).
        let staged_records = staging.record_writes();
        let (work, reopened) = staging.finish().unwrap();
        assert_eq!(staged_records, 32, "two records per accepted row");
        assert!(
            work.files_written <= 3 + work.packs_published,
            "one cut must stay O(1) files, saw {}",
            work.files_written
        );
        for row in &rows[..16] {
            let records =
                sealed_batch_records(&reopened, row.evidence.batch_id().as_uuid().into_bytes())
                    .unwrap()
                    .expect("a published accepted row is readable");
            records
                .verify(
                    row.evidence.acceptance_sequence(),
                    row.evidence.batch_id().as_uuid().into_bytes(),
                    &TineAcceptedEvidenceDecoder,
                )
                .unwrap();
            assert_eq!(records.status.no_op, row.no_op);
            assert_eq!(
                records.status.exact_evidence_bytes,
                row.evidence.encode_canonical().unwrap()
            );
        }
        let mut extension = SealedGenerationStagingStore::open(&directory).unwrap();
        let mut builder = prefix.builder(&mut extension);
        builder.append(&rows[16]).unwrap();
        let complete = builder
            .finish(rows[16].evidence.post_frontier_root())
            .unwrap();
        let (_, extended) = extension.finish().unwrap();
        // An INCREMENTAL extension and a full rederivation in a separate
        // directory must agree on the cutoff's whole identity.
        let (_sealed_root_memory, mut memory) = sealed_test_store();
        let mut builder = empty.builder(&mut memory);
        for row in &rows {
            builder.append(row).unwrap();
        }
        let expected = builder
            .finish(rows[16].evidence.post_frontier_root())
            .unwrap();
        let (_, rederived) = memory.finish().unwrap();
        assert_eq!(complete.identity_digest(), expected.identity_digest());
        assert_eq!(
            complete.causal_tips().copied().collect::<Vec<_>>(),
            expected.causal_tips().copied().collect::<Vec<_>>()
        );
        // The pre-existing reader, opened before the extension's cut, still
        // resolves the rows it was opened over: a tier merge concatenates a
        // contiguous virtual run, so no published record's offset ever moves.
        assert!(sealed_batch_records(&reopened, 16u128.to_be_bytes())
            .unwrap()
            .is_some());
        assert!(sealed_batch_records(&extended, 17u128.to_be_bytes())
            .unwrap()
            .is_some());
        for row in &rows {
            let batch_id = row.evidence.batch_id().as_uuid().into_bytes();
            let from_extension = sealed_batch_records(&extended, batch_id).unwrap().unwrap();
            let from_rederivation = sealed_batch_records(&rederived, batch_id).unwrap().unwrap();
            assert_eq!(
                from_extension.status.exact_evidence_bytes,
                from_rederivation.status.exact_evidence_bytes
            );
        }
        drop(rederived);
        drop(extended);
        drop(reopened);
        drop(directory);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_directory_collision_or_corruption_never_changes_predecessor_authority() {
        let root =
            std::env::temp_dir().join(format!("tine-sealed-damage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let row = generation_rows(1).remove(0);
        let batch_id = row.evidence.batch_id().as_uuid().into_bytes();
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let mut staging = SealedGenerationStagingStore::open(&directory).unwrap();
        let mut builder = empty.builder(&mut staging);
        builder.append(&row).unwrap();
        let _cutoff = builder.finish(row.evidence.post_frontier_root()).unwrap();
        let (_, reopened) = staging.finish().unwrap();
        let answered = sealed_batch_records(&reopened, batch_id).unwrap().unwrap();

        // A torn record must REFUSE, never answer with the wrong bytes: the
        // record header carries its payload digest, so a torn write inside a
        // pack is caught on read.
        let (causal_locator, _status_locator) = batch_row_locators(
            &reopened
                .table_value(DOMAIN_BATCH, &batch_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let original = reopened
            .sealed_record(causal_locator, MAX_CHECKPOINT_BYTES)
            .unwrap();
        reopened.tear_packed_record(causal_locator).unwrap();
        assert!(sealed_batch_records(&reopened, batch_id).is_err());

        // Republishing the same content-addressed bytes through an ordinary
        // cut repairs it, and the predecessor's answer is unchanged.
        let mut repair = SealedGenerationStagingStore::open(&directory).unwrap();
        let repaired_locator = repair.append_record(&original).unwrap();
        let (_, status_locator) = batch_row_locators(
            &reopened
                .table_value(DOMAIN_BATCH, &batch_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        repair
            .put(
                DOMAIN_BATCH,
                &batch_id,
                &batch_row_value(repaired_locator, status_locator),
            )
            .unwrap();
        let (_, healed) = repair.finish().unwrap();
        let after = sealed_batch_records(&healed, batch_id).unwrap().unwrap();
        assert_eq!(
            after.status.exact_evidence_bytes,
            answered.status.exact_evidence_bytes
        );
        after
            .verify(1, batch_id, &TineAcceptedEvidenceDecoder)
            .unwrap();

        // A pack replaced by a symlink is refused, not followed. Refusal
        // scenario: hostile or accidental substitution of a private file
        // through a path the capability root does not own.
        #[cfg(unix)]
        {
            // `directory` IS the sealed directory here, not its parent. Every
            // pack is substituted: which one holds the record is an internal
            // placement detail, and the refusal must not depend on it.
            let outside = root.join("outside-packs");
            std::fs::create_dir_all(&outside).unwrap();
            let pack_names: Vec<String> = std::fs::read_dir(&root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("pack-v1"))
                .collect();
            assert!(
                !pack_names.is_empty(),
                "a published cut has at least one pack"
            );
            for name in &pack_names {
                let pack_path = root.join(name);
                let moved = outside.join(name);
                std::fs::rename(&pack_path, &moved).unwrap();
                std::os::unix::fs::symlink(&moved, &pack_path).unwrap();
            }
            assert!(sealed_batch_records(&healed, batch_id).is_err());
            for name in &pack_names {
                let pack_path = root.join(name);
                std::fs::remove_file(&pack_path).unwrap();
                std::fs::rename(outside.join(name), &pack_path).unwrap();
            }
            std::fs::remove_dir(&outside).unwrap();
            assert!(sealed_batch_records(&healed, batch_id).unwrap().is_some());
        }
        drop(healed);
        drop(reopened);
        drop(directory);
        crate::test_support::remove_dir_all(root);
    }

    /// The prevention guard for the reopen regression of 2026-09-14.
    ///
    /// `tine_storage` verifies a table's digest exactly ONCE per
    /// `TableSetReader`, and Tine builds a fresh reader inside
    /// `SealedTableAccess::table_value`. A loop that point-reads one key per
    /// DOCUMENT therefore re-hashed the whole roster table per document: a
    /// 1,077-document graph spent 20 s in checkpoint qualification alone.
    /// `DOCUMENT_SCALE_CACHED_DOMAINS` is the fix; this is what fails if a
    /// later change drops a domain out of it, and what must NOT be extended to
    /// a batch-scale domain (the `DOMAIN_BATCH` half below is that half of the
    /// contract: batch-scale keys stay uncached, and the remedy for a loop over
    /// them is to stop looping).
    #[test]
    fn a_document_scale_domain_is_hashed_once_per_reader_not_once_per_lookup() {
        let root = tempfile::tempdir().expect("sealed test root");
        let directory =
            cap_std::fs::Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        let mut store = SealedGenerationStagingStore::open(&directory).unwrap();
        const DOCUMENTS: u128 = 64;
        let document_keys: Vec<[u8; crate::oplog::cold_object_store::DOCUMENT_ROSTER_KEY_BYTES]> =
            (0..DOCUMENTS)
                .map(|ordinal| {
                    framed_document_key(
                        DocumentKey::Entity(DocumentId::from_uuid(uuid::Uuid::from_u128(
                            0x5eed_0000 + ordinal,
                        )))
                        .authenticated_map_key()
                        .as_slice(),
                    )
                    .unwrap()
                })
                .collect();
        let value = ColdLocatorV1 {
            offset: 0,
            length: 1,
        }
        .to_bytes();
        for key in &document_keys {
            store.put(DOMAIN_DOCUMENT_ROSTER, key, &value).unwrap();
        }
        let object_keys: Vec<[u8; 32]> = (0..DOCUMENTS)
            .map(|ordinal| *ContentDigest::of(format!("covered/{ordinal}").as_bytes()).as_bytes())
            .collect();
        for key in &object_keys {
            store.put(DOMAIN_COVERED_OBJECT, key, &[1]).unwrap();
        }
        let (_, reader) = store.finish().unwrap();

        // Reader-LOCAL, not the process-wide `table_digest_verifications()`:
        // this suite runs in parallel, and a delta on a global counter is a
        // flaky test. One `read_table` is one whole-table digest verification,
        // so the reader's own count answers the same question deterministically.
        let before = reader.index_reads();
        for key in &document_keys {
            assert!(reader
                .table_value(DOMAIN_DOCUMENT_ROSTER, key)
                .unwrap()
                .is_some());
        }
        let hashed = reader.index_reads() - before;
        assert!(
            hashed <= 2,
            "a document-scale domain read {hashed} whole tables for {DOCUMENTS} point \
             reads; the per-lookup reader is back"
        );

        // The other half: an object/batch-scale domain is NOT cached, so its
        // absence from the list is observable rather than merely intended.
        let before = reader.index_reads();
        for key in &object_keys {
            assert!(reader
                .table_value(DOMAIN_COVERED_OBJECT, key)
                .unwrap()
                .is_some());
        }
        assert!(
            reader.index_reads() - before >= u64::try_from(DOCUMENTS).unwrap(),
            "an object-scale domain must stay off the document-scale cache list"
        );
    }

    #[test]
    fn sealed_directory_publication_fault_can_retry_without_replacing_predecessor() {
        use crate::oplog::cold_object_store::SEALED_TIER_FANOUT;

        let root = std::env::temp_dir().join(format!("tine-sealed-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        // One row per seeded cut, plus the row the interrupted cut carries.
        // SEALED_TIER_FANOUT - 1 seeded cuts leave level 0 one table short in
        // every domain, so the interrupted cut is the cut that FILLS level 0:
        // it merges a table level and retires a full pack run. That is the case
        // a two-cut matrix never reached, and the one where an interruption can
        // strand a merged artifact that nothing names yet.
        let rows = generation_rows(SEALED_TIER_FANOUT as u64);
        let last = SEALED_TIER_FANOUT - 1;
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let first_id = rows[0].evidence.batch_id().as_uuid().into_bytes();
        let penultimate_id = rows[last - 1].evidence.batch_id().as_uuid().into_bytes();
        let last_id = rows[last].evidence.batch_id().as_uuid().into_bytes();

        // THE crash matrix. Every prefix of the publication protocol must
        // leave the predecessor intact and a retry able to complete.
        //
        // Each kill point gets its OWN sealed directory seeded with the same
        // predecessor cuts: the retry below genuinely publishes into the shared
        // directory, so reusing one across kills would offer an already
        // accepted batch to the next builder.
        for kill in [
            SealedCutKill::AfterPayloadPacks,
            SealedCutKill::AfterTables,
            SealedCutKill::MidPackTierMerge,
            SealedCutKill::AfterRootRecord,
            SealedCutKill::AfterMarkerBeforeRetire,
        ] {
            let kill_root = root.join(format!("{kill:?}"));
            std::fs::create_dir_all(&kill_root).unwrap();
            let directory =
                cap_std::fs::Dir::open_ambient_dir(&kill_root, cap_std::ambient_authority())
                    .unwrap();
            let mut prefix = empty.clone();
            let mut readers = Vec::new();
            for row in &rows[..last] {
                let mut seed = SealedGenerationStagingStore::open(&directory).unwrap();
                let mut builder = prefix.builder(&mut seed);
                builder.append(row).unwrap();
                prefix = builder.finish(row.evidence.post_frontier_root()).unwrap();
                let (_, reader) = seed.finish().unwrap();
                readers.push(reader);
            }
            let mut interrupted = SealedGenerationStagingStore::open(&directory).unwrap();
            let mut builder = prefix.builder(&mut interrupted);
            builder.append(&rows[last]).unwrap();
            let candidate = builder
                .finish(rows[last].evidence.post_frontier_root())
                .unwrap();
            let killed = interrupted.finish_killed_at(kill).unwrap();
            // The interrupted cut really is a merging cut: everything from the
            // table step on has merged a table level, and everything from the
            // pack-merge step on has written the merged pack.
            if !matches!(kill, SealedCutKill::AfterPayloadPacks) {
                assert!(
                    killed.tables_merged >= 1,
                    "{kill:?} must interrupt a cut that merged a table level"
                );
            }
            if matches!(
                kill,
                SealedCutKill::MidPackTierMerge
                    | SealedCutKill::AfterRootRecord
                    | SealedCutKill::AfterMarkerBeforeRetire
            ) {
                assert!(
                    killed.merged_pack_bytes > 0,
                    "{kill:?} must interrupt a cut that merged a pack level"
                );
                assert_eq!(killed.packs_retired, 0, "{kill:?} precedes pack retirement");
            }

            // Reopen exactly as a cold start would.
            let reopened = SealedGenerationDirectory::open(&directory).unwrap();
            let committed_last = sealed_batch_records(&reopened, last_id).unwrap();
            match kill {
                SealedCutKill::AfterMarkerBeforeRetire => {
                    // The marker names this cut: the row IS committed, and an
                    // unretired superseded pack is residue, not damage.
                    assert!(committed_last.is_some(), "{kill:?} installed its marker");
                }
                _ => {
                    // Before the marker, the whole cut is residue.
                    assert!(
                        committed_last.is_none(),
                        "{kill:?} must not publish its row"
                    );
                }
            }
            // NO accepted original is lost: the oldest and the newest seeded
            // rows both still answer, and both still verify at their sequence.
            sealed_batch_records(&reopened, first_id)
                .unwrap()
                .expect("the oldest accepted row survives every interruption")
                .verify(1, first_id, &TineAcceptedEvidenceDecoder)
                .unwrap();
            sealed_batch_records(&reopened, penultimate_id)
                .unwrap()
                .expect("the newest accepted row survives every interruption")
                .verify(last as u64, penultimate_id, &TineAcceptedEvidenceDecoder)
                .unwrap();

            // Before the marker, a retry from the same predecessor completes
            // and agrees with the interrupted candidate's identity. AFTER the
            // marker there is nothing to retry -- the cut committed -- and
            // re-offering the same batch is correctly refused as a repeat;
            // what must hold there is that the committed row verifies.
            if matches!(kill, SealedCutKill::AfterMarkerBeforeRetire) {
                let committed = SealedGenerationDirectory::open(&directory).unwrap();
                sealed_batch_records(&committed, last_id)
                    .unwrap()
                    .expect("a marked cut committed its row")
                    .verify(last as u64 + 1, last_id, &TineAcceptedEvidenceDecoder)
                    .unwrap();
                let mut repeat = SealedGenerationStagingStore::open(&directory).unwrap();
                let mut builder = prefix.builder(&mut repeat);
                assert!(builder.append(&rows[last]).is_err());
                drop(repeat);
                drop(committed);
                drop(reopened);
                drop(readers);
                drop(directory);
                continue;
            }
            let mut retry = SealedGenerationStagingStore::open(&directory).unwrap();
            let mut builder = prefix.builder(&mut retry);
            builder.append(&rows[last]).unwrap();
            let retried = builder
                .finish(rows[last].evidence.post_frontier_root())
                .unwrap();
            let (retry_work, complete) = retry.finish().unwrap();
            assert!(
                retry_work.tables_merged >= 1 && retry_work.packs_retired >= 1,
                "the retry is the merging cut the interruption abandoned: {retry_work:?}"
            );
            assert_eq!(retried.identity_digest(), candidate.identity_digest());
            assert_eq!(
                retried.causal_tips().copied().collect::<Vec<_>>(),
                candidate.causal_tips().copied().collect::<Vec<_>>()
            );
            sealed_batch_records(&complete, last_id)
                .unwrap()
                .expect("the retry publishes the row")
                .verify(last as u64 + 1, last_id, &TineAcceptedEvidenceDecoder)
                .unwrap();
            sealed_batch_records(&complete, first_id)
                .unwrap()
                .expect("the merged level still answers the oldest row")
                .verify(1, first_id, &TineAcceptedEvidenceDecoder)
                .unwrap();
            drop(complete);
            drop(reopened);
            drop(readers);
            drop(directory);
        }
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_cutoff_incremental_build_matches_independent_full_rederivation() {
        let rows = generation_rows(65);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        for row in &rows[..64] {
            builder.append(row).unwrap();
        }
        let first = builder
            .finish(rows[63].evidence.post_frontier_root())
            .unwrap();
        let mut builder = first.builder(&mut store);
        builder.append(&rows[64]).unwrap();
        let second = builder
            .finish(rows[64].evidence.post_frontier_root())
            .unwrap();

        let (_sealed_root_independent, mut independent) = sealed_test_store();
        let mut builder = empty.builder(&mut independent);
        for row in &rows {
            builder.append(row).unwrap();
        }
        let full = builder
            .finish(rows[64].evidence.post_frontier_root())
            .unwrap();
        assert_eq!(second.identity_digest(), full.identity_digest());
        assert_eq!(second.frontier(), full.frontier());
        assert_eq!(second.causal_tip_digest(), full.causal_tip_digest());
        assert_eq!(
            second.causal_tips().collect::<Vec<_>>(),
            full.causal_tips().collect::<Vec<_>>()
        );
        let (_, published) = store.finish().unwrap();
        for row in &rows {
            let sequence = row.evidence.acceptance_sequence();
            let id = row.evidence.batch_id().as_uuid().into_bytes();
            let records = sealed_batch_records(&published, id).unwrap().unwrap();
            records
                .verify(sequence, id, &TineAcceptedEvidenceDecoder)
                .unwrap();
            assert_eq!(records.status.no_op, row.no_op);
            assert_eq!(
                records.status.exact_evidence_bytes,
                row.evidence.encode_canonical().unwrap()
            );
            // Every covered sequence resolves to its batch, incremental or not.
            assert_eq!(sequence_batch_id(&published, sequence).unwrap(), Some(id));
        }
    }

    #[test]
    fn sealed_cutoff_causal_tips_keep_exact_highest_per_peer_and_reject_tip_forks() {
        let rows = generation_rows_with_dots(&[(19, 1), (23, 8), (19, 2), (23, 3)]);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        for row in &rows[..3] {
            builder.append(row).unwrap();
        }
        let first = builder
            .finish(rows[2].evidence.post_frontier_root())
            .unwrap();
        let mut builder = first.builder(&mut store);
        builder.append(&rows[3]).unwrap();
        let second = builder
            .finish(rows[3].evidence.post_frontier_root())
            .unwrap();
        // An older accepted counter must not replace the already qualified tip.
        assert_eq!(second.causal_tip_digest(), first.causal_tip_digest());
        let tips = second.causal_tips().copied().collect::<Vec<_>>();
        assert_eq!(tips.len(), 2);
        assert_eq!(
            (tips[0].highest_accepted_counter, tips[0].batch_id),
            (2, 3u128.to_be_bytes())
        );
        assert_eq!(
            (tips[1].highest_accepted_counter, tips[1].batch_id),
            (8, 2u128.to_be_bytes())
        );
        let unchanged = second
            .builder(&mut store)
            .finish(second.frontier())
            .unwrap();
        // The tips are durable: each peer's exact tip reads back from the
        // causal-tip domain after publication.
        let (_, published) = store.finish().unwrap();
        for tip in &tips {
            let stored = published
                .table_value(DOMAIN_CAUSAL_TIP, &tip.peer_id)
                .unwrap()
                .expect("every qualified peer has a durable tip");
            assert_eq!(causal_tip_record(tip.peer_id, &stored).unwrap(), *tip);
        }
        assert_eq!(unchanged.causal_tip_digest(), second.causal_tip_digest());
        assert_eq!(
            unchanged.causal_tips().collect::<Vec<_>>(),
            second.causal_tips().collect::<Vec<_>>()
        );

        let forks = generation_rows_with_dots(&[(19, 1), (19, 1)]);
        let (_sealed_root_fork_store, mut fork_store) = sealed_test_store();
        let mut builder = empty.builder(&mut fork_store);
        builder.append(&forks[0]).unwrap();
        assert!(builder
            .append(&forks[1])
            .unwrap_err()
            .contains("conflicting batches"));
        let preserved = builder
            .finish(forks[0].evidence.post_frontier_root())
            .unwrap();
        assert_eq!(preserved.sequence(), 1);
        assert_eq!(
            preserved.causal_tips().next().unwrap().batch_id,
            1u128.to_be_bytes()
        );
    }

    #[test]
    fn sealed_cutoff_damaged_causal_tip_predecessor_preserves_cutoff() {
        let rows = generation_rows(2);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        builder.append(&rows[0]).unwrap();
        let first = builder
            .finish(rows[0].evidence.post_frontier_root())
            .unwrap();
        // A failed append must leave the cutoff exactly as it was -- no tip
        // advanced, no sequence moved.
        let mut fork = rows[1].clone();
        let peer = rows[0].causal_dot.peer_id();
        fork.causal_dot = BatchCausalDot::new(peer, rows[0].causal_dot.counter()).unwrap();
        let mut builder = first.builder(&mut store);
        assert!(builder.append(&fork).is_err());
        let preserved = builder.finish(first.frontier()).unwrap();
        assert_eq!(preserved.identity_digest(), first.identity_digest());
        assert_eq!(preserved.causal_tip_digest(), first.causal_tip_digest());
        // A damaged PERSISTED tip value is refused on read, never decoded into
        // a plausible-looking tip. Refusal scenario: a torn write or partial
        // sync delivery inside the causal-tip domain.
        let (_, published) = store.finish().unwrap();
        let tip_key = peer.key().as_uuid().into_bytes();
        let honest = published
            .table_value(DOMAIN_CAUSAL_TIP, &tip_key)
            .unwrap()
            .expect("the published cutoff records its tip");
        assert!(causal_tip_record(tip_key, &honest).is_ok());
        assert!(causal_tip_record(tip_key, &honest[..16]).is_err());
        // And the in-memory cutoff is still the one the builder returned. It
        // continues over the SAME sealed state it was built on: a cutoff and
        // its store are one pair, and offering it an empty store is the
        // "causal-tip predecessor does not authenticate" case above.
        let continued = cap_std::fs::Dir::open_ambient_dir(
            _sealed_root_store.path(),
            cap_std::ambient_authority(),
        )
        .unwrap();
        let mut fresh = SealedGenerationStagingStore::open(&continued).unwrap();
        let mut builder = preserved.builder(&mut fresh);
        builder.append(&rows[1]).unwrap();
        assert_eq!(
            builder
                .finish(rows[1].evidence.post_frontier_root())
                .unwrap()
                .causal_tips()
                .next()
                .unwrap()
                .highest_accepted_counter,
            2
        );
    }

    #[test]
    fn sealed_cutoff_one_row_delta_does_not_visit_historical_status_or_sequence_leaves() {
        let rows = generation_rows(513);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        for row in &rows[..512] {
            builder.append(row).unwrap();
        }
        let first = builder
            .finish(rows[511].evidence.post_frontier_root())
            .unwrap();
        let (_, published) = store.finish().unwrap();
        drop(published);

        // I-14: appending ONE row to a 512-row history must cost work
        // proportional to the row, not to the history. The unit is table
        // records opened; under size-tiered compaction at R=8 that is
        // O(log_R N) and never an enumeration of retained rows.
        let directory = cap_std::fs::Dir::open_ambient_dir(
            _sealed_root_store.path(),
            cap_std::ambient_authority(),
        )
        .unwrap();
        let mut store = SealedGenerationStagingStore::open(&directory).unwrap();
        let before = store.index_reads();
        let mut builder = first.builder(&mut store);
        builder.append(&rows[512]).unwrap();
        let second = builder
            .finish(rows[512].evidence.post_frontier_root())
            .unwrap();
        let reads = store.index_reads() - before;
        assert!(
            reads < 256,
            "one-row append enumerated retained history: {reads} table reads"
        );
        assert_eq!(second.sequence(), 513);
    }

    #[test]
    fn sealed_cutoff_rejects_gaps_forks_wrong_causal_membership_and_target() {
        let rows = generation_rows(3);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        assert!(builder.append(&rows[1]).is_err());
        builder.append(&rows[0]).unwrap();
        assert!(builder.append(&rows[0]).is_err());
        // Wrong causal membership: the row's dot is not in its own canonical
        // clock. The batch-map-root comparison that used to catch this went
        // with the authenticated map (D-2); the sealed causal record's own
        // canonicality rule is what remains, and it is where the invariant
        // belongs -- a record whose clock omits its dot cannot be encoded.
        let mut wrong = rows[1].clone();
        let peer = wrong.causal_dot.peer_id();
        wrong.causal_dot = BatchCausalDot::new(peer, 99).unwrap();
        assert!(builder.append(&wrong).unwrap_err().contains("canonical"));
        // The failed append did not move the roots. The correct immutable
        // records can still extend the preceding accepted cutoff.
        builder.append(&rows[1]).unwrap();
        assert!(builder
            .finish(rows[2].evidence.post_frontier_root())
            .is_err());
        assert!(
            SealedAcceptedCutoff::empty(rows[0].evidence.post_frontier_root().clone()).is_err()
        );
    }

    #[test]
    fn sealed_cutoff_damaged_predecessor_fails_without_changing_other_roots() {
        let rows = generation_rows(2);
        let empty = SealedAcceptedCutoff::empty(AcceptedFrontierRoot::empty()).unwrap();
        let (_sealed_root_store, mut store) = sealed_test_store();
        let mut builder = empty.builder(&mut store);
        builder.append(&rows[0]).unwrap();
        let first = builder
            .finish(rows[0].evidence.post_frontier_root())
            .unwrap();
        let first_id = rows[0].evidence.batch_id().as_uuid().into_bytes();
        let (_, published) = store.finish().unwrap();

        // Damage the predecessor's causal record inside its pack. Extending
        // over a damaged predecessor must FAIL; it must never silently accept
        // a successor over history it could not read.
        let (causal_locator, _) = batch_row_locators(
            &published
                .table_value(DOMAIN_BATCH, &first_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let saved = published
            .sealed_record(causal_locator, MAX_CHECKPOINT_BYTES)
            .unwrap();
        published.tear_packed_record(causal_locator).unwrap();
        assert!(sealed_batch_records(&published, first_id).is_err());

        // Republishing the exact bytes restores the predecessor's answer: the
        // damage was to a disposable copy of durable content, and everything
        // that did not depend on it is untouched (D-3).
        let directory = cap_std::fs::Dir::open_ambient_dir(
            _sealed_root_store.path(),
            cap_std::ambient_authority(),
        )
        .unwrap();
        let mut repair = SealedGenerationStagingStore::open(&directory).unwrap();
        let repaired = repair.append_record(&saved).unwrap();
        let (_, status_locator) = batch_row_locators(
            &published
                .table_value(DOMAIN_BATCH, &first_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        repair
            .put(
                DOMAIN_BATCH,
                &first_id,
                &batch_row_value(repaired, status_locator),
            )
            .unwrap();
        let (_, healed) = repair.finish().unwrap();
        sealed_batch_records(&healed, first_id)
            .unwrap()
            .expect("the repaired predecessor answers again")
            .verify(1, first_id, &TineAcceptedEvidenceDecoder)
            .unwrap();
        assert_eq!(first.sequence(), 1);
    }

    #[test]
    fn p3_round_7_live_floor_is_driven_by_acceptance_age() {
        let production = include_str!("checkpoint_generation.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("checkpoint_generation.rs has a test module boundary");
        let engine = include_str!("hot_engine.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("hot_engine.rs has a test module boundary");
        let contract = include_str!("../../../../docs/storage-sync-contract.md");

        assert!(
            !production.contains("LIVE_FLOOR_ELIGIBILITY_DISABLED"),
            "round 7 must remove the named inert live-floor constant"
        );
        assert!(
            engine.contains("acceptance_age_policy.eligible_through()"),
            "live capture must carry the device-local accepted-prefix E"
        );
        assert!(
            !contract.contains("The current live policy supplies E=0"),
            "the living contract must move with the activated live policy"
        );
    }

    #[test]
    fn p3_return_after_29_and_31_days() {
        use crate::oplog::hot_engine::{LazyGenesisCheckpointBuilder, ShardedHotEngine};
        use crate::oplog::lazy_genesis::LazyGenesisPackBuilder;
        use crate::oplog::{
            AuthorBatch, BatchDisposition, BlobDescription, BlockId, BlockLocation, CrdtPeerId,
            DocumentId, LineageDigest, LogicalPageName, ManagedPath, ManagedTextKind,
            OperationTransaction, PageId, SemanticOperation, SessionId, ValidatedBatch,
            WorkspaceId,
        };

        let day = 24 * 60 * 60 * 1_000_i64;
        let root = std::env::temp_dir().join(format!(
            "tine-p3-floor-returning-peers-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x7700));
        let lineage = LineageDigest::of(b"p3-floor-returning-peers");
        let catalog = DocumentId::from_uuid(uuid::Uuid::from_u128(0x7701));
        let home = DocumentId::from_uuid(uuid::Uuid::from_u128(0x7702));
        let page = PageId::from_uuid(uuid::Uuid::from_u128(0x7703));
        let block = BlockLocation {
            block_id: BlockId::from_uuid(uuid::Uuid::from_u128(0x7704)),
            home_document_id: home,
        };
        let (catalog_checkpoint, catalog_dependencies) = LazyGenesisCheckpointBuilder::new(catalog)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = Arc::new(
            LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog,
                BlobDescription::of(b"empty source"),
                &root,
            )
            .unwrap()
            .finish(catalog_checkpoint, catalog_dependencies)
            .unwrap(),
        );
        let archive = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let fresh = || {
            let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
            engine
                .set_checkpoint_floor_clock_for_test(1_000, 1_000)
                .unwrap();
            engine
                .install_lazy_genesis_baseline(Arc::clone(&baseline))
                .unwrap();
            engine
                .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
                .unwrap();
            engine
        };
        let author = |batch: u128, peer: u64| AuthorBatch {
            batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(batch)),
            author_device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(peer as u128)),
            author_session_id: SessionId::from_uuid(uuid::Uuid::from_u128(peer as u128 + 1)),
            crdt_peer_id: CrdtPeerId::from_u64(peer),
            causal_peer_id: fixture_incarnation(peer as u128),
        };
        let mut receiver = fresh();
        receiver.set_checkpoint_floor_config_for_test(
            super::super::checkpoint_floor_policy::FloorPolicyConfig {
                revision: 77,
                minimum_tail_bytes: 4 * 1024,
                live_size_multiplier: 1,
            },
        );
        let claims = receiver
            .clean_transient_projection_claim_snapshot()
            .unwrap()
            .unwrap();
        let prepare = |engine: &ShardedHotEngine, batch, peer, operations| {
            engine
                .prepare_fixture_transaction(
                    author(batch, peer),
                    &OperationTransaction::new(operations).unwrap(),
                )
                .unwrap()
        };
        let create = prepare(
            &receiver,
            0x7710,
            0x77,
            vec![
                SemanticOperation::CreatePage {
                    page_id: page,
                    home_document_id: home,
                    name: LogicalPageName::parse("Returning peers").unwrap(),
                    path: ManagedPath::parse("pages/returning-peers.md").unwrap(),
                    kind: ManagedTextKind::Page,
                },
                SemanticOperation::CreateBlock {
                    block,
                    page_id: page,
                    parent: None,
                    order: "a".into(),
                    content: "departure".into(),
                },
            ],
        );
        receiver
            .commit_clean_prepared(&create, claims.as_ref())
            .unwrap();
        receiver.wait_for_clean_checkpoint().unwrap();

        let mut peer_29 = fresh();
        let mut peer_31 = fresh();
        for peer in [&mut peer_29, &mut peer_31] {
            peer.stop_clean_checkpoint_publisher();
            assert_eq!(
                peer.replay_clean_committed_tail(claims.as_ref()).unwrap(),
                1
            );
        }
        let stale_29 = prepare(
            &peer_29,
            0x7729,
            0x29,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: "peer returned on day 29".into(),
            }],
        );
        peer_29
            .commit_clean_prepared(&stale_29, claims.as_ref())
            .unwrap();
        let stale_31 = prepare(
            &peer_31,
            0x7731,
            0x31,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: "exact peer bytes returned on day 31".into(),
            }],
        );
        peer_31
            .commit_clean_prepared(&stale_31, claims.as_ref())
            .unwrap();

        // Age alone never creates a cut. This small graph uses the production
        // 256 KiB/4x policy: after six months its returning peer still admits
        // directly, and the next real checkpoint retains every native floor.
        let within_budget_archive =
            ObjectStore::open(&root.join("within-budget-archive"), workspace).unwrap();
        let mut within_budget = ShardedHotEngine::new(workspace, lineage, catalog);
        within_budget
            .set_checkpoint_floor_clock_for_test(1_000, 1_000)
            .unwrap();
        within_budget
            .install_lazy_genesis_baseline(Arc::clone(&baseline))
            .unwrap();
        within_budget
            .attach_clean_archive_store(
                within_budget_archive
                    .duplicate_retained_capability()
                    .unwrap(),
            )
            .unwrap();
        within_budget
            .commit_clean_prepared(&create, claims.as_ref())
            .unwrap();
        within_budget.wait_for_clean_checkpoint().unwrap();
        within_budget
            .set_checkpoint_floor_clock_for_test(1_000 + 180 * day, 1_000 + 180 * day as u64)
            .unwrap();
        assert!(within_budget
            .preflight_prepared_full_history(&stale_29)
            .unwrap()
            .is_none());
        within_budget
            .commit_clean_prepared(&stale_29, claims.as_ref())
            .unwrap();
        within_budget.wait_for_clean_checkpoint().unwrap();
        let six_months = within_budget.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(six_months.eligible_through, 1);
        assert!(six_months.documents.iter().all(|document| {
            document.removable_bytes <= document.budget_bytes
                && document.requested_floor.is_none()
                && document.actual_floor.is_empty()
        }));

        // A high-entropy historical value makes removable image bytes exceed
        // this fixture's injected budget without inflating latest live state.
        // Separate constant guards pin the production 256 KiB/4x defaults.
        let mut random = 0x9e37_79b9_u32;
        let historical = (0..32_000)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                char::from(b'!' + (random % 90) as u8)
            })
            .collect::<String>();
        let huge = prepare(
            &receiver,
            0x7740,
            0x77,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: historical,
            }],
        );
        receiver
            .commit_clean_prepared(&huge, claims.as_ref())
            .unwrap();
        let shrink = prepare(
            &receiver,
            0x7741,
            0x77,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: "small live state".into(),
            }],
        );
        receiver
            .commit_clean_prepared(&shrink, claims.as_ref())
            .unwrap();
        receiver.wait_for_clean_checkpoint().unwrap();

        let main_at_departure = BTreeSet::from([
            create.manifest().batch_id(),
            huge.manifest().batch_id(),
            shrink.manifest().batch_id(),
        ]);
        let mut direct_29 = fresh();
        direct_29.stop_clean_checkpoint_publisher();
        direct_29
            .replay_clean_checkpoint_tail(&main_at_departure, claims.as_ref())
            .unwrap();
        direct_29
            .set_checkpoint_floor_clock_for_test(1_000 + 29 * day, 1_000 + 29 * day as u64)
            .unwrap();
        assert!(direct_29
            .preflight_prepared_full_history(&stale_29)
            .unwrap()
            .is_none());
        let day_29 = direct_29.stage_ready(ValidatedBatch::new(stale_29.clone()));
        assert!(matches!(
            day_29.disposition(),
            BatchDisposition::Accepted { .. }
        ));

        receiver
            .set_checkpoint_floor_clock_for_test(1_000 + 29 * day, 1_000 + 29 * day as u64)
            .unwrap();
        let day_29_activity = prepare(
            &receiver,
            0x7743,
            0x77,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: "linear activity on day 29".into(),
            }],
        );
        receiver
            .commit_clean_prepared(&day_29_activity, claims.as_ref())
            .unwrap();
        receiver.wait_for_clean_checkpoint().unwrap();
        let before_cut = receiver.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(before_cut.eligible_through, 0);
        assert!(before_cut
            .documents
            .iter()
            .filter(|document| document.document_id == home)
            .all(|document| document.requested_floor.is_none()));
        let age_after_day_29 = receiver.checkpoint_floor_policy_probe_for_test();
        receiver
            .set_checkpoint_floor_clock_for_test(1_000 + 100 * day, 1_000 + 100 * day as u64)
            .unwrap();
        assert!(matches!(
            receiver
                .stage_ready(ValidatedBatch::new(day_29_activity.clone()))
                .disposition(),
            BatchDisposition::DuplicateAccepted { .. }
        ));
        let durable = receiver
            .accepted_frontier_root()
            .unwrap()
            .acceptance_sequence();
        let _export_only = receiver.capture_clean_checkpoint(durable).unwrap();
        assert_eq!(
            receiver.checkpoint_floor_policy_probe_for_test(),
            age_after_day_29,
            "duplicate delivery and checkpoint export must not advance T"
        );

        receiver
            .set_checkpoint_floor_clock_for_test(1_000 + 31 * day, 1_000 + 31 * day as u64)
            .unwrap();
        let covering = prepare(
            &receiver,
            0x7742,
            0x77,
            vec![SemanticOperation::EditBlockContent {
                block,
                content: "covers the day-29 head".into(),
            }],
        );
        receiver
            .commit_clean_prepared(&covering, claims.as_ref())
            .unwrap();
        receiver.wait_for_clean_checkpoint().unwrap();
        let cut = receiver.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(cut.eligible_through, 3);
        let home_cut = cut
            .documents
            .iter()
            .find(|document| document.document_id == home)
            .expect("day-31 publication measured the changed busy page");
        assert!(home_cut.requested_floor.is_some());
        assert!(home_cut.post_cut_removable_bytes < home_cut.removable_bytes);
        assert!(home_cut.post_cut_removable_bytes < home_cut.budget_bytes / 2);

        receiver
            .install_published_checkpoint_document_for_test(home)
            .unwrap();
        assert_eq!(
            receiver.checkpoint_floor_policy_probe_for_test().2,
            cut.latest_acceptance_utc_ms,
            "checkpoint installation must preserve rather than advance T"
        );
        let need = receiver
            .preflight_prepared_full_history(&stale_31)
            .unwrap()
            .expect("day-31 dependency must be below the published native floor");
        assert_eq!(need.document_id, home);

        let accepted_before_stale_31 = BTreeSet::from([
            create.manifest().batch_id(),
            huge.manifest().batch_id(),
            shrink.manifest().batch_id(),
            day_29_activity.manifest().batch_id(),
            covering.manifest().batch_id(),
        ]);
        let mut recovered = fresh();
        recovered.stop_clean_checkpoint_publisher();
        assert_eq!(
            recovered
                .replay_clean_checkpoint_tail(&accepted_before_stale_31, claims.as_ref())
                .unwrap(),
            accepted_before_stale_31.len()
        );
        let recovered_outcome = recovered.stage_ready(ValidatedBatch::new(stale_31.clone()));
        assert!(matches!(
            recovered_outcome.disposition(),
            BatchDisposition::Accepted { .. }
        ));
        assert!(recovered
            .accepted_batch_evidence(stale_31.manifest().batch_id())
            .is_ok());
        assert_eq!(
            archive
                .resolve_logical_manifest_bytes(stale_31.manifest().batch_id())
                .unwrap(),
            stale_31.manifest().encode().unwrap(),
            "full-history recovery changed the original manifest bytes"
        );

        drop(receiver);
        drop(peer_29);
        drop(peer_31);
        drop(within_budget);
        drop(within_budget_archive);
        drop(direct_29);
        drop(recovered);
        drop(archive);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn sealed_cutoff_and_live_images_keep_one_audited_publication_path() {
        let engine = include_str!("hot_engine.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        let production = include_str!("checkpoint_generation.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        for forbidden in ["fs::write", "fs::rename"] {
            assert!(
                !production.contains(forbidden),
                "checkpoint publication acquired unaudited {forbidden}"
            );
        }
        assert!(production.contains("publish_document_images("));
        assert!(production.contains("ShardedHotEngine::build_policy_compact_worker_document("));
        assert!(production.contains("DurableDirectoryPublication"));
        assert!(production.contains("publish_new_exact_single_writer"));
        assert_eq!(
            production.matches(".remove_file(").count(),
            0,
            "checkpoint authoring unlinks nothing itself: retiring superseded \
             sealed bytes belongs to the sealed directory's marker-last \
             publication (I-14)"
        );
        assert!(engine.contains("changed_snapshots"));
        assert!(engine.contains("checkpoint_document_at_current"));
    }

    #[test]
    fn decoder_accepts_only_the_one_current_evidence_schema() {
        let evidence = evidence();
        let bytes = evidence.encode_canonical().unwrap();
        let binding = TineAcceptedEvidenceDecoder
            .decode_accepted_evidence(ACCEPTED_EVIDENCE_SCHEMA_VERSION, &bytes)
            .unwrap();
        assert_eq!(binding.batch_id, [0x51; 16]);
        assert_eq!(binding.manifest_fingerprint, digest(0x61));
        assert_eq!(binding.event_binding_digest, digest(0x71));
        assert_eq!(binding.acceptance_sequence, 1);
        assert!(TineAcceptedEvidenceDecoder
            .decode_accepted_evidence(ACCEPTED_EVIDENCE_SCHEMA_VERSION - 1, &bytes)
            .is_err());

        let mut trailing = bytes;
        trailing.push(0);
        assert!(TineAcceptedEvidenceDecoder
            .decode_accepted_evidence(ACCEPTED_EVIDENCE_SCHEMA_VERSION, &trailing)
            .is_err());
    }

    #[test]
    fn tine_and_storage_share_the_exact_causal_record_address() {
        let low = CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
            [0x11; 16],
        )));
        let author = CausalPeerId::from_key(WriterIncarnationId::from_uuid(
            uuid::Uuid::from_bytes([0x44; 16]),
        ));
        let (root_key, root_digest) =
            authenticated_causal_clock_root(&[(low, 3), (author, 7)]).unwrap();
        let engine_address = accepted_causal_record_digest(
            BatchId::from_uuid(uuid::Uuid::from_bytes([0x51; 16])),
            digest(0x22),
            digest(0x33),
            BatchCausalDot::new(author, 7).unwrap(),
            root_key,
            root_digest,
        );
        let storage_record = SealedAcceptedCausalRecordV2 {
            batch_id: [0x51; 16],
            manifest_fingerprint: digest(0x22),
            event_binding_digest: digest(0x33),
            causal_peer_id: [0x44; 16],
            causal_counter: 7,
            canonical_causal_clock: vec![
                SealedAcceptedCausalClockEntryV2 {
                    peer_id: [0x11; 16],
                    counter: 3,
                },
                SealedAcceptedCausalClockEntryV2 {
                    peer_id: [0x44; 16],
                    counter: 7,
                },
            ],
        };
        assert_eq!(storage_record.address().unwrap(), engine_address);
    }

    #[test]
    fn tine_decoder_completes_the_shared_membership_proof() {
        let evidence = evidence();
        let batch_id = [0x51_u8; 16];
        let causal = SealedAcceptedCausalRecordV2 {
            batch_id,
            manifest_fingerprint: digest(0x61),
            event_binding_digest: digest(0x71),
            causal_peer_id: [0x44; 16],
            causal_counter: 7,
            canonical_causal_clock: vec![SealedAcceptedCausalClockEntryV2 {
                peer_id: [0x44; 16],
                counter: 7,
            }],
        };
        let (_sealed_root_store, mut store) = sealed_test_store();
        let causal_bytes = causal.encode().unwrap();
        let causal_address = causal.address().unwrap();
        let causal_locator = store
            .append_record(&addressed_record(causal_address, &causal_bytes))
            .unwrap();
        let status = AcceptedStatusRecordV2 {
            batch_id,
            no_op: false,
            evidence_schema: ACCEPTED_EVIDENCE_SCHEMA_VERSION,
            exact_evidence_bytes: evidence.encode_canonical().unwrap(),
            accepted_causal_record_digest: causal_address,
        };
        let status_bytes = status.encode().unwrap();
        let status_locator = store
            .append_record(&addressed_record(status.value_digest(), &status_bytes))
            .unwrap();
        store
            .put(
                DOMAIN_BATCH,
                &batch_id,
                &batch_row_value(causal_locator, status_locator),
            )
            .unwrap();
        store
            .put(DOMAIN_SEQUENCE, &1_u64.to_be_bytes(), &batch_id)
            .unwrap();
        let (_, published) = store.finish().unwrap();

        // The Merkle path is gone (D-2); what remains -- and what this test
        // exists for -- is that Tine's evidence decoder and tine-storage's
        // records agree on ONE identity for the batch.
        let records = sealed_batch_records(&published, batch_id)
            .unwrap()
            .expect("the published batch row resolves");
        records
            .verify(1, batch_id, &TineAcceptedEvidenceDecoder)
            .unwrap();
        assert_eq!(sequence_batch_id(&published, 1).unwrap(), Some(batch_id));
        assert_eq!(
            records.status.exact_evidence_bytes,
            evidence.encode_canonical().unwrap()
        );
        assert_eq!(records.causal, causal);
        // A wrong sequence or batch id must not verify.
        assert!(records
            .verify(2, batch_id, &TineAcceptedEvidenceDecoder)
            .is_err());
        assert!(records
            .verify(1, [0x52; 16], &TineAcceptedEvidenceDecoder)
            .is_err());
    }

    #[test]
    fn checkpoint_authoring_uses_only_the_audited_publication_boundary() {
        let production = include_str!("checkpoint_generation.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        for forbidden in ["write_all", "std::fs::rename"] {
            assert!(
                !production.contains(forbidden),
                "checkpoint authoring bypassed the audited publication boundary: {forbidden}"
            );
        }
        // The sealed accepted index is authored through ONE surface: the
        // sorted-table staging store (`SealedAcceptedIndexWriter`, the
        // path-copied treap's writer, no longer exists -- P4c2 4.1).
        assert!(production.contains("SealedGenerationStagingStore"));
        assert!(production.contains("DurableDirectoryPublication"));
        assert!(production.contains("publish_new_exact_single_writer"));
        assert!(production.contains("replace_exact"));
        assert_eq!(
            production.matches(".remove_file(").count(),
            0,
            "checkpoint authoring unlinks nothing itself: retiring superseded \
             sealed bytes belongs to the sealed directory's marker-last \
             publication (I-14)"
        );
    }

    #[test]
    fn p3_checkpoint_payload_names_floor_policy_recovery_fence_and_candidate_qualification() {
        let production = include_str!("checkpoint_generation.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        let contract = include_str!("../../../../docs/storage-sync-contract.md");
        for required in [
            "binding: CheckpointBindingV1",
            "recovery_fence: CheckpointRecoveryFenceV1",
            "retained_history_ms: i64",
            "policy: DocumentCheckpointPolicyV1",
            "actual_floor: Vec<CrdtPeerCounter>",
            "validate_published_candidate(",
        ] {
            assert!(
                production.contains(required),
                "current checkpoint publication omits required field/seam: {required}"
            );
        }
        assert!(
            production.find("validate_published_candidate(").unwrap()
                < production
                    .find(".replace_exact(CHECKPOINT_POINTER")
                    .unwrap(),
            "the complete candidate must be qualified before pointer replacement"
        );
        let pointer_reproof = production
            .find("authority.revalidate()?;")
            .expect("checkpoint pointer publication omits its live lease re-proof");
        assert!(
            production.find("validate_published_candidate(").unwrap() < pointer_reproof
                && pointer_reproof
                    < production
                        .find(".replace_exact(CHECKPOINT_POINTER")
                        .unwrap(),
            "lease identity must be re-proved after qualification and before current"
        );
        let runtime = include_str!("local_active.rs");
        assert_eq!(
            runtime
                .matches("install_clean_checkpoint_publication_authority")
                .count(),
            2,
            "cold installation and full-history actor swap must bind the publisher"
        );
        assert!(include_str!("hot_engine.rs")
            .contains("eligible_through: self.acceptance_age_policy.eligible_through(),"));
        assert!(include_str!("../sync_runtime.rs")
            .contains("const MANAGED_LOCAL_IDLE_TICK: Duration = Duration::from_millis(50);"));
        assert_eq!(
            super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            30 * 24 * 60 * 60 * 1_000
        );
        assert_eq!(
            super::super::checkpoint_floor_policy::DEFAULT_MINIMUM_TAIL_BYTES,
            256 * 1024
        );
        assert_eq!(
            super::super::checkpoint_floor_policy::DEFAULT_LIVE_SIZE_MULTIPLIER,
            4
        );
        for required in [
            "device-local acceptance observations",
            "30-day lower bound",
            "minimum-tail bytes",
            "multiplier. Each document-to-image record",
            "not by the 50 ms actor tick",
        ] {
            assert!(
                contract.contains(required),
                "storage contract omits load-bearing checkpoint value: {required}"
            );
        }
    }

    #[test]
    fn checkpoint_open_counter_distinguishes_checkpoint_from_full_replay() {
        let runtime = include_str!("../sync_runtime.rs");
        assert!(runtime.contains("pub checkpoint_opens: usize"));
        assert!(runtime.contains("pub full_replay_opens: usize"));
        assert!(runtime.contains("CleanCheckpointOpen::Loaded"));
        assert!(runtime.contains("is_clean_genesis_frontier"));
        assert!(!runtime.contains("let projection = if replayed == 0"));
    }

    #[test]
    fn checkpoint_payload_uses_the_shared_sealed_roster_round_trip() {
        let evidence = evidence();
        let peer = CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
            [0x44; 16],
        )));
        let capture = CleanCheckpointCapture {
            workspace_id: WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x7001)),
            lineage_digest: LineageDigest::of(b"checkpoint-payload-test"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0x7002)),
            cutoff_state_digest: evidence.post_frontier_root().state_digest(),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 0,
            target_sequence: 1,
            covered_block_count: 0,
            state_bytes: b"state".to_vec(),
            accepted_rows: vec![CleanCheckpointAcceptedRow {
                no_op: false,
                evidence: evidence.clone(),
                causal_dot: BatchCausalDot::new(peer, 7).unwrap(),
                canonical_causal_clock: vec![(peer, 7)],
            }],
            required_objects: BTreeSet::from([digest(0x91)]),
            identity_changes: Vec::new(),
            capture_work: 3,
            documents: None,
        };
        let (_sealed_root_store, mut store) = sealed_test_store();
        let (sequence, payload) = build_payload(capture, None, &mut store).unwrap();
        assert_eq!(sequence, 1);
        assert_eq!(payload.state_bytes, b"state");
        // One covered object, recorded as a membership entry rather than a
        // map root: the payload names the sealed root, not a treap.
        assert_eq!(store.table_entries(DOMAIN_COVERED_OBJECT).unwrap().len(), 1);
        let (work, published) = store.finish().unwrap();
        assert_eq!(work.root_digest, published.sealed_root_digest().unwrap());
        let records = sealed_batch_records(&published, [0x51; 16])
            .unwrap()
            .expect("the payload's accepted row is in the sealed tables");
        records
            .verify(1, [0x51; 16], &TineAcceptedEvidenceDecoder)
            .unwrap();
        assert_eq!(
            records.status.exact_evidence_bytes,
            evidence.encode_canonical().unwrap()
        );
    }

    #[test]
    fn generation_payload_does_not_embed_covered_roster() {
        let rows = generation_rows(32);
        let capture = CleanCheckpointCapture {
            workspace_id: WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x4a01)),
            lineage_digest: LineageDigest::of(b"generation-payload-bound"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0x4a02)),
            cutoff_state_digest: rows
                .last()
                .unwrap()
                .evidence
                .post_frontier_root()
                .state_digest(),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 0,
            target_sequence: rows.len() as u64,
            covered_block_count: 0,
            state_bytes: b"bounded generation".to_vec(),
            accepted_rows: rows,
            required_objects: BTreeSet::new(),
            identity_changes: Vec::new(),
            capture_work: 0,
            documents: None,
        };
        let (_sealed_root_store, mut store) = sealed_test_store();
        let (_, mut payload) = build_payload(capture, None, &mut store).unwrap();
        let bytes = encode_canonical(&payload).unwrap();
        assert!(
            bytes.len() < 1024,
            "covered sealed roster objects remain embedded in the generation payload: {} bytes",
            bytes.len()
        );
        // The covered half is named by ONE value, not carried: 32 accepted
        // rows leave the payload the same size as one (I-25).
        let (work, published) = store.finish().unwrap();
        for sequence in 1..=32_u64 {
            assert!(sequence_batch_id(&published, sequence).unwrap().is_some());
        }
        // Production stamps the cut's root onto the payload after `finish`
        // (`write_clean_checkpoint`); the payload names the covered half by
        // that ONE value.
        payload.sealed_root = SealedRootWire::of(work.root_locator);
        payload.sealed_root_digest = work.root_digest;
        assert_eq!(
            payload.sealed_root_digest,
            published.sealed_root_digest().unwrap()
        );
    }

    #[test]
    fn checkpoint_payload_extends_the_durable_frontier_from_one_row_delta() {
        let first = evidence();
        let second = evidence_after(&first);
        let peer = CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
            [0x44; 16],
        )));
        let first_capture = CleanCheckpointCapture {
            workspace_id: WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x7001)),
            lineage_digest: LineageDigest::of(b"checkpoint-payload-test"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0x7002)),
            cutoff_state_digest: first.post_frontier_root().state_digest(),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 0,
            target_sequence: 1,
            covered_block_count: 0,
            state_bytes: b"frontier-one".to_vec(),
            accepted_rows: vec![CleanCheckpointAcceptedRow {
                no_op: false,
                evidence: first.clone(),
                causal_dot: BatchCausalDot::new(peer, 7).unwrap(),
                canonical_causal_clock: vec![(peer, 7)],
            }],
            required_objects: BTreeSet::from([digest(0x91)]),
            identity_changes: Vec::new(),
            capture_work: 3,
            documents: None,
        };
        let (_sealed_root_store, mut store) = sealed_test_store();
        let (_, first_payload) = build_payload(first_capture, None, &mut store).unwrap();
        let second_capture = CleanCheckpointCapture {
            workspace_id: WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x7001)),
            lineage_digest: LineageDigest::of(b"checkpoint-payload-test"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0x7002)),
            cutoff_state_digest: second.post_frontier_root().state_digest(),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 1,
            target_sequence: 2,
            covered_block_count: 0,
            state_bytes: b"frontier-two".to_vec(),
            accepted_rows: vec![CleanCheckpointAcceptedRow {
                no_op: false,
                evidence: second,
                causal_dot: BatchCausalDot::new(peer, 8).unwrap(),
                canonical_causal_clock: vec![(peer, 8)],
            }],
            required_objects: BTreeSet::from([digest(0x92)]),
            identity_changes: Vec::new(),
            capture_work: 4,
            documents: None,
        };
        let (sequence, mut payload) =
            build_payload(second_capture, Some((1, first_payload)), &mut store).unwrap();
        assert_eq!(sequence, 2);
        assert_eq!(payload.state_bytes, b"frontier-two");
        assert_eq!(store.table_entries(DOMAIN_COVERED_OBJECT).unwrap().len(), 2);
        assert_eq!(payload.capture_work, 4);
        let (work, published) = store.finish().unwrap();
        // A ONE-row delta extended the durable frontier: both covered rows
        // resolve, and the payload names the covered half by one digest.
        for (sequence, batch_id) in [(1_u64, [0x51_u8; 16]), (2, [0x52; 16])] {
            assert_eq!(
                sequence_batch_id(&published, sequence).unwrap(),
                Some(batch_id)
            );
            sealed_batch_records(&published, batch_id)
                .unwrap()
                .expect("both covered rows resolve")
                .verify(sequence, batch_id, &TineAcceptedEvidenceDecoder)
                .unwrap();
        }
        // Production stamps the cut's root onto the payload after `finish`
        // (`write_clean_checkpoint`).
        payload.sealed_root = SealedRootWire::of(work.root_locator);
        payload.sealed_root_digest = work.root_digest;
        assert_eq!(
            payload.sealed_root_digest,
            published.sealed_root_digest().unwrap()
        );
    }

    #[test]
    fn lag_over_sixty_four_marks_the_next_coalesced_rewrite_elevated() {
        let root = std::env::temp_dir().join(format!(
            "tine-clean-checkpoint-lag-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = crate::oplog::WorkspaceId::from_uuid(uuid::Uuid::from_u128(0xa564));
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let publisher = CleanCheckpointPublisher::new(store, 0, BTreeMap::new(), None, None);
        let peer = CausalPeerId::from_key(WriterIncarnationId::from_uuid(uuid::Uuid::from_bytes(
            [0x44; 16],
        )));
        let row = CleanCheckpointAcceptedRow {
            no_op: false,
            evidence: evidence(),
            causal_dot: BatchCausalDot::new(peer, 7).unwrap(),
            canonical_causal_clock: vec![(peer, 7)],
        };
        publisher.enqueue(CleanCheckpointCapture {
            workspace_id: workspace,
            lineage_digest: LineageDigest::of(b"checkpoint-lag-test"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0xa565)),
            cutoff_state_digest: digest(0xa3),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 0,
            target_sequence: CLEAN_CHECKPOINT_LAG_MAX + 1,
            covered_block_count: 0,
            state_bytes: Vec::new(),
            accepted_rows: vec![row; CLEAN_CHECKPOINT_LAG_MAX as usize + 1],
            required_objects: BTreeSet::new(),
            identity_changes: Vec::new(),
            capture_work: 0,
            documents: None,
        });
        assert!(publisher.elevated_rewrite_observed());
        drop(publisher);
        crate::test_support::remove_dir_all(root);
    }

    fn checkpoint_fault_fixture(tag: &str) -> (std::path::PathBuf, ObjectStore) {
        let root = std::env::temp_dir().join(format!(
            "tine-clean-checkpoint-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = crate::oplog::WorkspaceId::from_uuid(uuid::Uuid::new_v4());
        let store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        (root, store)
    }

    fn empty_capture(store: &ObjectStore, state: &[u8]) -> CleanCheckpointCapture {
        CleanCheckpointCapture {
            workspace_id: store.workspace_id(),
            lineage_digest: LineageDigest::of(b"checkpoint-publication-test"),
            catalog_document_id: DocumentId::from_uuid(uuid::Uuid::from_u128(0x7004)),
            cutoff_state_digest: digest(0xa4),
            eligible_through: 0,
            latest_acceptance_utc_ms: 0,
            age_cutoff_utc_ms: -super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS,
            clock_frozen: false,
            last_clock_reset_utc_ms: None,
            floor_policy: super::super::checkpoint_floor_policy::FloorPolicyConfig::default(),
            base_sequence: 0,
            target_sequence: 0,
            covered_block_count: 0,
            state_bytes: state.to_vec(),
            accepted_rows: Vec::new(),
            required_objects: BTreeSet::new(),
            identity_changes: Vec::new(),
            capture_work: 0,
            documents: None,
        }
    }

    fn loaded_state(store: &ObjectStore) -> Vec<u8> {
        match open_checkpoint(store).unwrap() {
            CleanCheckpointOpen::Loaded(loaded) => loaded.state_bytes,
            CleanCheckpointOpen::Absent => panic!("checkpoint unexpectedly absent"),
            CleanCheckpointOpen::Invalid(detail) => {
                panic!("checkpoint unexpectedly invalid: {detail}")
            }
        }
    }

    #[test]
    fn every_checkpoint_publication_prefix_keeps_the_complete_predecessor() {
        let (root, store) = checkpoint_fault_fixture("publication-prefix");
        let published = publish_capture(&store, empty_capture(&store, b"predecessor")).unwrap();
        assert_eq!(published.diagnostics.measurement_sequence, 0);
        assert_eq!(published.diagnostics.policy_revision, 1);
        assert!(published.diagnostics.checkpoint_bytes > 0);
        assert_eq!(
            published.diagnostics.publication_edge,
            SyncCheckpointPublicationEdge::CurrentPointerDurable,
        );
        assert_eq!(published.diagnostics.latest_acceptance_utc_ms, 0);
        assert_eq!(published.diagnostics.eligible_through, 0);
        assert_eq!(
            published.diagnostics.age_cutoff_utc_ms,
            Some(-super::super::checkpoint_floor_policy::RETAINED_HISTORY_MS)
        );
        assert_eq!(published.diagnostics.clock_frozen, Some(false));
        assert_eq!(published.diagnostics.last_clock_reset_utc_ms, None);
        assert_eq!(published.diagnostics.peak_rss_bytes, None);
        let directory = root.join("archive").join(CHECKPOINT_DIRECTORY);
        let predecessor_pointer = std::fs::read(directory.join(CHECKPOINT_POINTER)).unwrap();
        let predecessor: CheckpointPointerV2 = decode_canonical(&predecessor_pointer).unwrap();
        assert_eq!(loaded_state(&store), b"predecessor");

        let slot = 1 - predecessor.slot as usize;
        let predecessor_payload_bytes =
            std::fs::read(directory.join(CHECKPOINT_PAYLOAD_NAMES[predecessor.slot as usize]))
                .unwrap();
        let predecessor_payload: CheckpointPayloadV2 =
            decode_canonical(&predecessor_payload_bytes).unwrap();
        let (_sealed_root_sealed, mut sealed) = sealed_test_store();
        let (sequence, mut payload) =
            build_payload(empty_capture(&store, b"successor"), None, &mut sealed).unwrap();
        // The successor covers exactly the accepted history the predecessor
        // did (both captures are empty), so it names the SAME sealed root --
        // the one this archive's `sealed-v3` directory actually holds.
        // Forging a payload against a throwaway staging directory would name a
        // root no reader can resolve, which is a different (already covered)
        // fault than the publication-prefix question this test asks.
        payload.sealed_root = predecessor_payload.sealed_root;
        payload.sealed_root_digest = predecessor_payload.sealed_root_digest;
        let payload = encode_canonical(&payload).unwrap();
        std::fs::write(directory.join(CHECKPOINT_PAYLOAD_NAMES[slot]), &payload).unwrap();
        assert_eq!(loaded_state(&store), b"predecessor");

        let generation = CheckpointGenerationV2 {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            sequence,
            slot: slot as u8,
            payload_digest: ContentDigest::of(&payload),
            payload_len: u64::try_from(payload.len()).unwrap(),
        };
        let generation_bytes = encode_canonical(&generation).unwrap();
        std::fs::write(
            directory.join(CHECKPOINT_GENERATION_NAMES[slot]),
            &generation_bytes,
        )
        .unwrap();
        assert_eq!(loaded_state(&store), b"predecessor");

        let successor_pointer = encode_canonical(&CheckpointPointerV2 {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            sequence,
            slot: slot as u8,
            generation_digest: ContentDigest::of(&generation_bytes),
        })
        .unwrap();
        std::fs::write(directory.join(CHECKPOINT_POINTER), successor_pointer).unwrap();
        assert_eq!(loaded_state(&store), b"successor");

        // Rolling back the pointer to an older still-complete generation is a
        // valid crash image: it restores the predecessor and leaves any newer
        // archive manifests to ordinary tail admission.
        std::fs::write(directory.join(CHECKPOINT_POINTER), predecessor_pointer).unwrap();
        assert_eq!(loaded_state(&store), b"predecessor");
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn post_publication_checkpoint_damage_is_private_fallback_state() {
        for damage in [
            "pointer-bitflip",
            "generation-truncate",
            "payload-truncate",
            "payload-oversize",
        ] {
            let (root, store) = checkpoint_fault_fixture(damage);
            publish_capture(&store, empty_capture(&store, b"disposable")).unwrap();
            let directory = root.join("archive").join(CHECKPOINT_DIRECTORY);
            let pointer_bytes = std::fs::read(directory.join(CHECKPOINT_POINTER)).unwrap();
            let pointer: CheckpointPointerV2 = decode_canonical(&pointer_bytes).unwrap();
            let slot = pointer.slot as usize;
            match damage {
                "pointer-bitflip" => {
                    let mut bytes = pointer_bytes;
                    bytes[0] ^= 0x80;
                    std::fs::write(directory.join(CHECKPOINT_POINTER), bytes).unwrap();
                }
                "generation-truncate" => {
                    std::fs::write(directory.join(CHECKPOINT_GENERATION_NAMES[slot]), [0x01])
                        .unwrap();
                }
                "payload-truncate" => {
                    std::fs::write(directory.join(CHECKPOINT_PAYLOAD_NAMES[slot]), [0x01]).unwrap();
                }
                "payload-oversize" => {
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .open(directory.join(CHECKPOINT_PAYLOAD_NAMES[slot]))
                        .unwrap();
                    file.set_len(MAX_CHECKPOINT_BYTES + 1).unwrap();
                }
                _ => unreachable!(),
            }
            match open_checkpoint(&store) {
                Ok(CleanCheckpointOpen::Invalid(_)) | Err(CleanCheckpointOpenError::Store(_)) => {}
                Ok(CleanCheckpointOpen::Absent) => panic!("{damage} erased the checkpoint pointer"),
                Ok(CleanCheckpointOpen::Loaded(_)) => panic!("{damage} loaded damaged state"),
                Err(CleanCheckpointOpenError::ArchiveDamage(detail)) => {
                    panic!("{damage} was misclassified as archive authority damage: {detail}")
                }
            }
            crate::test_support::remove_dir_all(root);
        }
    }

    #[test]
    fn object_only_crash_residue_does_not_change_checkpoint_membership() {
        use crate::oplog::{DocumentId, ObjectKind, OperationObject};

        let (root, store) = checkpoint_fault_fixture("object-only-residue");
        publish_capture(&store, empty_capture(&store, b"stable checkpoint")).unwrap();
        let object = OperationObject::new(
            store.workspace_id(),
            DocumentId::from_uuid(uuid::Uuid::new_v4()),
            ObjectKind::CrdtUpdate,
            b"valid orphaned operation object".to_vec(),
        )
        .unwrap();
        store.stage_object_bytes(&object.encode().unwrap()).unwrap();
        assert_eq!(store.committed_manifest_names().unwrap().len(), 0);
        assert_eq!(store.object_names().unwrap().len(), 1);
        assert_eq!(loaded_state(&store), b"stable checkpoint");
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn generation_hot_retirement_bounded_open() {
        use crate::oplog::hot_engine::{LazyGenesisCheckpointBuilder, ShardedHotEngine};
        use crate::oplog::lazy_genesis::LazyGenesisPackBuilder;
        use crate::oplog::{
            AuthorBatch, BatchDisposition, BlobDescription, BlockId, BlockLocation, CrdtPeerId,
            DeviceId, LineageDigest, LogicalPageName, ManagedPath, ManagedTextKind,
            OperationTransaction, PageId, SemanticOperation, SessionId, WorkspaceId,
        };

        let root = std::env::temp_dir().join(format!(
            "tine-p3-incremental-images-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = WorkspaceId::from_uuid(uuid::Uuid::from_u128(0x3001));
        let lineage = LineageDigest::of(b"p3-incremental-images");
        let catalog = DocumentId::from_uuid(uuid::Uuid::from_u128(0x3002));
        let page = PageId::from_uuid(uuid::Uuid::from_u128(0x3003));
        let home = DocumentId::from_uuid(uuid::Uuid::from_u128(0x3004));
        let block = BlockId::from_uuid(uuid::Uuid::from_u128(0x3005));
        let (catalog_checkpoint, catalog_dependencies) = LazyGenesisCheckpointBuilder::new(catalog)
            .unwrap()
            .finish()
            .unwrap();
        let baseline = Arc::new(
            LazyGenesisPackBuilder::new(
                workspace,
                lineage,
                catalog,
                BlobDescription::of(b"empty source"),
                &root,
            )
            .unwrap()
            .finish(catalog_checkpoint, catalog_dependencies)
            .unwrap(),
        );
        let archive = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let mut engine = ShardedHotEngine::new(workspace, lineage, catalog);
        engine
            .install_lazy_genesis_baseline(Arc::clone(&baseline))
            .unwrap();
        engine
            .attach_clean_archive_store(archive.duplicate_retained_capability().unwrap())
            .unwrap();
        let claims = engine
            .clean_transient_projection_claim_snapshot()
            .unwrap()
            .unwrap();
        let author = AuthorBatch {
            batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(0x3010)),
            author_device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(0x3011)),
            author_session_id: SessionId::from_uuid(uuid::Uuid::from_u128(0x3012)),
            crdt_peer_id: CrdtPeerId::from_u64(0x3013),
            causal_peer_id: fixture_incarnation(0x3014),
        };
        let create = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: page,
                home_document_id: home,
                name: LogicalPageName::parse("Incremental Images").unwrap(),
                path: ManagedPath::parse("pages/incremental-images.md").unwrap(),
                kind: ManagedTextKind::Page,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: block,
                    home_document_id: home,
                },
                page_id: page,
                parent: None,
                order: "a".into(),
                content: "before".into(),
            },
        ])
        .unwrap();
        let prepared = engine.prepare_fixture_transaction(author, &create).unwrap();
        assert!(matches!(
            engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        engine.wait_for_clean_checkpoint().unwrap();
        let first_diagnostics = engine.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(first_diagnostics.relocation_batch_visits, 1);
        let checkpoint_reader =
            ObjectStore::open_structural(&root.join("archive"), workspace).unwrap();
        let first = match open_checkpoint(&checkpoint_reader).unwrap() {
            CleanCheckpointOpen::Loaded(loaded) => loaded,
            CleanCheckpointOpen::Invalid(error) => {
                panic!("bootstrap checkpoint is invalid: {error}")
            }
            CleanCheckpointOpen::Absent => panic!("bootstrap checkpoint was not published"),
        };
        assert_eq!(first.open_work, GenerationOpenWork::default());
        assert_eq!(first.accepted_history.sequence_enumerations(), 0);
        assert!(checkpoint_reader.committed_manifest_names().unwrap().len() <= 2);
        assert_eq!(first.image_work.changed_documents, 2);
        assert_eq!(first.image_work.exported_documents, 2);
        assert_eq!(first.image_work.reused_documents, 0);
        assert_eq!(first.image_work.unchanged_image_imports, 0);
        assert_eq!(first.image_work.unchanged_image_reconstructions, 0);
        let catalog_reference = first.document_image_reference(catalog).unwrap();
        let first_home_reference = first.document_image_reference(home).unwrap();
        engine.evict_hot_document_for_checkpoint_test(home);
        let before_evicted_edit = crate::oplog::hot_engine::live_write_benchmark_counters();

        let edit = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: block,
                home_document_id: home,
            },
            content: "after".into(),
        }])
        .unwrap();
        let prepared = engine
            .prepare_fixture_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(0x3020)),
                    ..author
                },
                &edit,
            )
            .unwrap();
        let interrupted_retirement_batch = prepared.clone();
        let interrupted_retirement_batch_id = prepared.manifest().batch_id();
        assert!(matches!(
            engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        let mut cold_capture = engine.capture_clean_checkpoint(1).unwrap();
        let accepted_rows = cold_capture.accepted_rows.clone();
        let cold_documents = cold_capture.documents.as_mut().unwrap();
        cold_documents.changed_snapshots.insert(home, None);
        let (predecessor_dependencies, predecessor_document) = first
            .documents
            .load_document(catalog, home)
            .unwrap()
            .unwrap();
        let worker_store = ObjectStore::open(&root.join("archive"), workspace).unwrap();
        let materialized = ShardedHotEngine::materialize_checkpoint_worker_document(
            cold_documents,
            &worker_store,
            home,
            Some((
                first.documents.sequence(),
                predecessor_dependencies,
                predecessor_document,
            )),
            &accepted_rows,
        )
        .unwrap();
        assert!(materialized.reconstructed);
        assert!(!materialized.imported_handoff);
        assert!(materialized.imported_predecessor_image);
        drop(worker_store);
        let evicted_edit_work =
            crate::oplog::hot_engine::live_write_benchmark_counters().since(before_evicted_edit);
        assert_eq!(
            evicted_edit_work.document_head_reconstructions, 0,
            "a same-session post-eviction edit must load the published image, not replay ancestry"
        );
        engine.wait_for_clean_checkpoint().unwrap();
        let second_diagnostics = engine.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(second_diagnostics.relocation_batch_visits, 1);
        let checkpoint_reader =
            ObjectStore::open_structural(&root.join("archive"), workspace).unwrap();
        let second = match open_checkpoint(&checkpoint_reader).unwrap() {
            CleanCheckpointOpen::Loaded(loaded) => loaded,
            _ => panic!("incremental checkpoint was not published"),
        };
        assert_eq!(second.open_work, GenerationOpenWork::default());
        assert_eq!(second.accepted_history.sequence_enumerations(), 0);
        assert!(
            checkpoint_reader.committed_manifest_names().unwrap().len() <= 2,
            "hot manifests are bounded by the two live document heads, not accepted lifetime"
        );
        assert_eq!(second.image_work.changed_documents, 1);
        assert_eq!(second.image_work.exported_documents, 1);
        assert_eq!(second.image_work.reused_documents, 1);
        assert_eq!(second.image_work.unchanged_image_imports, 0);
        assert_eq!(second.image_work.unchanged_image_reconstructions, 0);
        assert_eq!(
            second.document_image_reference(catalog).unwrap(),
            catalog_reference,
            "the unchanged catalog must retain its exact immutable image reference"
        );
        assert_ne!(
            second.document_image_reference(home).unwrap(),
            first_home_reference
        );

        let third_edit = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: block,
                home_document_id: home,
            },
            content: "third".into(),
        }])
        .unwrap();
        let prepared = engine
            .prepare_fixture_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(uuid::Uuid::from_u128(0x3030)),
                    ..author
                },
                &third_edit,
            )
            .unwrap();
        assert!(matches!(
            engine
                .commit_clean_prepared(&prepared, claims.as_ref())
                .unwrap()
                .disposition(),
            BatchDisposition::Accepted { .. }
        ));
        engine.wait_for_clean_checkpoint().unwrap();
        let third_diagnostics = engine.clean_checkpoint_diagnostics().unwrap();
        assert_eq!(third_diagnostics.relocation_batch_visits, 1);
        let checkpoint_reader =
            ObjectStore::open_structural(&root.join("archive"), workspace).unwrap();
        let third = match open_checkpoint(&checkpoint_reader).unwrap() {
            CleanCheckpointOpen::Loaded(loaded) => loaded,
            _ => panic!("third checkpoint was not published"),
        };
        assert_eq!(third.open_work, GenerationOpenWork::default());
        assert_eq!(third.accepted_history.sequence_enumerations(), 0);
        let third_open_store_work = checkpoint_reader.instrumentation();
        assert!(
            third_open_store_work.namespace_manifest_decodes <= third.tail.len() + 2,
            "hot manifest decode work is bounded by tail plus the two live document pins"
        );
        assert_eq!(
            third_open_store_work.namespace_object_decodes, 0,
            "covered objects are not decoded by namespace open"
        );
        assert!(
            checkpoint_reader.committed_manifest_names().unwrap().len() <= 2,
            "a third accepted generation must not add a third hot manifest"
        );
        assert_eq!(third.image_work.changed_documents, 1);
        assert_eq!(third.image_work.exported_documents, 1);
        assert_eq!(third.image_work.reused_documents, 1);
        archive
            .publish_prepared_fixture(&interrupted_retirement_batch)
            .unwrap();
        assert!(
            archive
                .read_manifest(interrupted_retirement_batch_id)
                .unwrap()
                .is_some(),
            "fixture recreates the hot+cold state left by an interrupted retirement"
        );
        let resume_reader = ObjectStore::open_structural(&root.join("archive"), workspace).unwrap();
        let resumed = match open_checkpoint(&resume_reader).unwrap() {
            CleanCheckpointOpen::Loaded(loaded) => loaded,
            _ => panic!("interrupted retirement did not reopen its generation"),
        };
        assert_eq!(resumed.open_work, GenerationOpenWork::default());
        assert!(
            resume_reader
                .read_manifest(interrupted_retirement_batch_id)
                .unwrap()
                .is_none(),
            "ordinary reopen resumes idempotent post-marker hot retirement"
        );
        // There are no per-object files and no per-object GC any more: an
        // image is a record inside a pack, and the marker's pack table
        // resolves any locator ever published. So an OLDER generation's image
        // stays readable after the marker advances -- which is what the
        // retired reader-pin machinery was for, obtained by construction
        // instead of by a pin registry (I-14).
        let reopened_documents = resumed.documents.directory_for_test();
        let locator = locator_from_value(
            &reopened_documents
                .table_value(DOMAIN_CAPSULE_BLOB, first_home_reference.as_bytes())
                .unwrap()
                .expect("an earlier generation's image stays addressable"),
        )
        .unwrap();
        let bytes = reopened_documents
            .sealed_record(locator, MAX_CHECKPOINT_BYTES)
            .unwrap();
        assert_eq!(
            ContentDigest::of(&bytes),
            first_home_reference,
            "an earlier generation's image must stay readable through the marker's pack table"
        );
        drop(first);

        drop(engine);
        drop(archive);
        drop(baseline);
        crate::test_support::remove_dir_all(root);
    }
}
