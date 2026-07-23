use std::{
    collections::BTreeMap,
    ops::Bound,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use bytes::Bytes;
use loro::{
    kv_store_handle, ExportMode, KvStore, KvStoreHandle, LoroDoc, LoroMap, ValueOrContainer,
};
use loro_internal::diff_calc::{
    external_tracker_codec_stats, reset_external_tracker_codec_stats, ExternalTrackerCodecStats,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
struct FaultStore {
    inner: Arc<FaultStoreInner>,
}

#[derive(Debug, Default)]
struct FaultStoreInner {
    data: Mutex<BTreeMap<Vec<u8>, Bytes>>,
    fail_reads: AtomicBool,
    fail_scans: AtomicBool,
    error: Mutex<Option<String>>,
    read_ops: AtomicUsize,
    read_bytes: AtomicUsize,
    read_keys: Mutex<Vec<Vec<u8>>>,
}

impl FaultStore {
    fn new() -> (Self, KvStoreHandle) {
        let store = Self {
            inner: Arc::new(FaultStoreInner::default()),
        };
        (store.clone(), kv_store_handle(store))
    }

    fn set_fail_reads(&self, fail: bool) {
        self.inner.fail_reads.store(fail, Ordering::SeqCst);
    }

    fn set_fail_scans(&self, fail: bool) {
        self.inner.fail_scans.store(fail, Ordering::SeqCst);
    }

    fn failed_read(&self) -> bool {
        if !self.inner.fail_reads.load(Ordering::SeqCst) {
            return false;
        }
        *self.inner.error.lock().unwrap() = Some("authentication denied".to_string());
        true
    }

    fn failed_scan(&self) -> bool {
        if !self.inner.fail_scans.load(Ordering::SeqCst) {
            return false;
        }
        *self.inner.error.lock().unwrap() =
            Some("authentication denied on change-block read".to_string());
        true
    }

    fn fork(&self) -> (Self, KvStoreHandle) {
        let (fork, handle) = Self::new();
        *fork.inner.data.lock().unwrap() = self.inner.data.lock().unwrap().clone();
        (fork, handle)
    }

    fn raw_get(&self, key: &[u8]) -> Option<Bytes> {
        self.inner.data.lock().unwrap().get(key).cloned()
    }

    fn raw_set(&self, key: &[u8], value: impl Into<Bytes>) {
        self.inner
            .data
            .lock()
            .unwrap()
            .insert(key.to_vec(), value.into());
    }

    fn raw_remove(&self, key: &[u8]) {
        self.inner.data.lock().unwrap().remove(key);
    }

    fn raw_change_blocks(&self) -> Vec<(Vec<u8>, Bytes)> {
        self.inner
            .data
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.len() == 12)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn reset_reads(&self) {
        self.inner.read_ops.store(0, Ordering::SeqCst);
        self.inner.read_bytes.store(0, Ordering::SeqCst);
        self.inner.read_keys.lock().unwrap().clear();
    }

    fn reads(&self) -> StoreReads {
        StoreReads {
            ops: self.inner.read_ops.load(Ordering::SeqCst),
            bytes: self.inner.read_bytes.load(Ordering::SeqCst),
            keys: self.inner.read_keys.lock().unwrap().clone(),
        }
    }

    fn record_read(&self, key: &[u8], value: Option<&Bytes>) {
        self.inner.read_ops.fetch_add(1, Ordering::SeqCst);
        self.inner
            .read_bytes
            .fetch_add(key.len() + value.map_or(0, Bytes::len), Ordering::SeqCst);
        self.inner.read_keys.lock().unwrap().push(key.to_vec());
    }
}

#[derive(Clone, Debug)]
struct StoreReads {
    ops: usize,
    bytes: usize,
    keys: Vec<Vec<u8>>,
}

impl KvStore for FaultStore {
    fn get(&self, key: &[u8]) -> Option<Bytes> {
        if self.failed_read() {
            return None;
        }
        let value = self.inner.data.lock().unwrap().get(key).cloned();
        self.record_read(key, value.as_ref());
        value
    }

    fn set(&mut self, key: &[u8], value: Bytes) {
        self.inner.data.lock().unwrap().insert(key.to_vec(), value);
    }

    fn compare_and_swap(&mut self, key: &[u8], old: Option<Bytes>, new: Bytes) -> bool {
        let mut data = self.inner.data.lock().unwrap();
        if data.get(key) != old.as_ref() {
            return false;
        }
        data.insert(key.to_vec(), new);
        true
    }

    fn remove(&mut self, key: &[u8]) -> Option<Bytes> {
        self.inner.data.lock().unwrap().remove(key)
    }

    fn contains_key(&self, key: &[u8]) -> bool {
        if self.failed_read() {
            return false;
        }
        self.inner.data.lock().unwrap().contains_key(key)
    }

    fn scan(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
    ) -> Box<dyn DoubleEndedIterator<Item = (Bytes, Bytes)> + '_> {
        if self.failed_read() || self.failed_scan() {
            return Box::new(Vec::new().into_iter());
        }
        let data = self.inner.data.lock().unwrap();
        let rows = data
            .iter()
            .filter(|(key, _)| within_bounds(key, &start, &end))
            .map(|(key, value)| (Bytes::copy_from_slice(key), value.clone()))
            .collect::<Vec<_>>();
        drop(data);
        for (key, value) in &rows {
            self.record_read(key, Some(value));
        }
        Box::new(rows.into_iter())
    }

    fn len(&self) -> usize {
        if self.failed_read() {
            return 0;
        }
        self.inner.data.lock().unwrap().len()
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn size(&self) -> usize {
        if self.failed_read() {
            return 0;
        }
        self.inner
            .data
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum()
    }

    fn export_all(&mut self) -> Bytes {
        panic!("external checkpoint tests do not serialize the injected store")
    }

    fn import_all(&mut self, _bytes: Bytes) -> Result<(), String> {
        Err("external checkpoint tests do not import serialized stores".to_string())
    }

    fn clone_store(&self) -> KvStoreHandle {
        kv_store_handle(self.clone())
    }

    fn take_error(&mut self) -> Option<String> {
        self.inner.error.lock().unwrap().take()
    }
}

fn within_bounds(key: &[u8], start: &Bound<&[u8]>, end: &Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Included(start) => key >= *start,
        Bound::Excluded(start) => key > *start,
        Bound::Unbounded => true,
    };
    let before_end = match end {
        Bound::Included(end) => key <= *end,
        Bound::Excluded(end) => key < *end,
        Bound::Unbounded => true,
    };
    after_start && before_end
}

#[derive(Clone)]
struct XmParts {
    vv: Vec<u8>,
    frontiers: Vec<u8>,
    start_vv: Vec<u8>,
    start_frontiers: Vec<u8>,
    causal: Vec<u8>,
    baseline: Vec<u8>,
    timestamp: [u8; 8],
}

fn decode_xm(bytes: &[u8]) -> XmParts {
    assert_eq!(&bytes[..8], b"LOROXM03");
    let mut offset = 8;
    let vv = read_test_blob(bytes, &mut offset);
    let frontiers = read_test_blob(bytes, &mut offset);
    let start_vv = read_test_blob(bytes, &mut offset);
    let start_frontiers = read_test_blob(bytes, &mut offset);
    let causal = read_test_blob(bytes, &mut offset);
    let baseline = read_test_blob(bytes, &mut offset);
    let timestamp = bytes[offset..offset + 8].try_into().unwrap();
    offset += 8;
    assert_eq!(offset, bytes.len());
    XmParts {
        vv,
        frontiers,
        start_vv,
        start_frontiers,
        causal,
        baseline,
        timestamp,
    }
}

fn encode_xm(parts: &XmParts) -> Vec<u8> {
    let mut bytes = b"LOROXM03".to_vec();
    for blob in [
        &parts.vv,
        &parts.frontiers,
        &parts.start_vv,
        &parts.start_frontiers,
        &parts.causal,
        &parts.baseline,
    ] {
        bytes.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        bytes.extend_from_slice(blob);
    }
    bytes.extend_from_slice(&parts.timestamp);
    bytes
}

fn read_test_blob(bytes: &[u8], offset: &mut usize) -> Vec<u8> {
    let len = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap()) as usize;
    *offset += 4;
    let blob = bytes[*offset..*offset + len].to_vec();
    *offset += len;
    blob
}

fn baseline_tracker_count_offset(bytes: &[u8]) -> usize {
    assert_eq!(&bytes[..8], b"LOROIB03");
    let mut offset = 8;
    let vv_count = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4 + vv_count * 12;
    let frontier_count = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    offset += 4 + frontier_count * 12;
    offset
}

fn remove_baseline_trackers(bytes: &[u8]) -> Vec<u8> {
    let count_offset = baseline_tracker_count_offset(bytes);
    let mut output = bytes[..count_offset].to_vec();
    output.extend_from_slice(&0_u32.to_le_bytes());
    output
}

fn alter_first_baseline_tracker(bytes: &[u8]) -> Vec<u8> {
    let mut entries = decode_baseline_tracker_blobs(bytes);
    let tracker = &mut entries.first_mut().unwrap().1.tracker;
    assert!(tracker.len() > 8);
    let midpoint = tracker.len() / 2;
    tracker[midpoint] ^= 0x80;
    encode_baseline_tracker_blobs(bytes, &entries)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct WireId {
    peer: u64,
    counter: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireCausalDag {
    vv: Vec<(u64, i32)>,
    frontiers: Vec<WireId>,
    nodes: Vec<WireCausalNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireCausalNode {
    peer: u64,
    cnt: i32,
    lamport: u32,
    deps: Vec<WireId>,
    vv: Vec<(u64, i32)>,
    has_succ: bool,
    len: u32,
    boundary_proof: Option<WireCausalBoundaryProof>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireCausalBoundaryProof {
    parts: Vec<WireCausalBoundaryPart>,
    node_digest: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireCausalBoundaryPart {
    start: i32,
    end: i32,
    source_start: i32,
    source_end: i32,
    source_lamport: u32,
    source_deps: Vec<WireId>,
    change_count: u32,
    last_change_start: i32,
    boundary_digest: [u8; 32],
    anchor_metadata_digest: [u8; 32],
    source_digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WireAuthenticatedTracker {
    owner: Vec<u8>,
    tracker: Vec<u8>,
    anchor_metadata_digest: [u8; 32],
    digest: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireTracker {
    applied_vv: loro::VersionVector,
    current_vv: loro::VersionVector,
    spans: Vec<WireTrackerSpan>,
    deletes: Vec<WireTrackerDelete>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireTrackerSpan {
    id_peer: u64,
    id_counter: i32,
    id_lamport: u32,
    real_id: Option<(u64, i32)>,
    future: bool,
    delete_times: i16,
    origin_left: Option<(u64, i32)>,
    origin_right: Option<(u64, i32)>,
    len: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireTrackerDelete {
    op_peer: u64,
    op_counter: i32,
    target_peer: u64,
    target_start: i32,
    target_end: i32,
}

fn decode_baseline_tracker_blobs(baseline: &[u8]) -> Vec<(u32, WireAuthenticatedTracker)> {
    let count_offset = baseline_tracker_count_offset(baseline);
    let mut input_offset = count_offset;
    let count = u32::from_le_bytes(baseline[input_offset..input_offset + 4].try_into().unwrap());
    input_offset += 4;

    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let index =
            u32::from_le_bytes(baseline[input_offset..input_offset + 4].try_into().unwrap());
        input_offset += 4;
        let owner_len =
            u32::from_le_bytes(baseline[input_offset..input_offset + 4].try_into().unwrap())
                as usize;
        input_offset += 4;
        let owner = baseline[input_offset..input_offset + owner_len].to_vec();
        input_offset += owner_len;
        let len = u32::from_le_bytes(baseline[input_offset..input_offset + 4].try_into().unwrap())
            as usize;
        input_offset += 4;
        let tracker = baseline[input_offset..input_offset + len].to_vec();
        input_offset += len;
        let anchor_metadata_digest = baseline[input_offset..input_offset + 32]
            .try_into()
            .unwrap();
        input_offset += 32;
        let digest = baseline[input_offset..input_offset + 32]
            .try_into()
            .unwrap();
        input_offset += 32;
        entries.push((
            index,
            WireAuthenticatedTracker {
                owner,
                tracker,
                anchor_metadata_digest,
                digest,
            },
        ));
    }
    assert_eq!(input_offset, baseline.len());
    entries
}

fn encode_baseline_tracker_blobs(
    baseline: &[u8],
    entries: &[(u32, WireAuthenticatedTracker)],
) -> Vec<u8> {
    let count_offset = baseline_tracker_count_offset(baseline);
    let mut output = baseline[..count_offset].to_vec();
    output.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (index, authenticated) in entries {
        output.extend_from_slice(&index.to_le_bytes());
        output.extend_from_slice(&(authenticated.owner.len() as u32).to_le_bytes());
        output.extend_from_slice(&authenticated.owner);
        output.extend_from_slice(&(authenticated.tracker.len() as u32).to_le_bytes());
        output.extend_from_slice(&authenticated.tracker);
        output.extend_from_slice(&authenticated.anchor_metadata_digest);
        output.extend_from_slice(&authenticated.digest);
    }
    output
}

fn rewrite_first_baseline_tracker(
    baseline: &[u8],
    mutate: impl FnOnce(&mut WireTracker),
) -> Vec<u8> {
    let mut entries = decode_baseline_tracker_blobs(baseline);
    let (_, first) = entries.first_mut().expect("at least one text tracker");
    let mut tracker: WireTracker = postcard::from_bytes(&first.tracker).unwrap();
    mutate(&mut tracker);
    first.tracker = postcard::to_allocvec(&tracker).unwrap();
    encode_baseline_tracker_blobs(baseline, &entries)
}

fn checkpoint_for_xm(checkpoint: &[u8], xm: &[u8]) -> Vec<u8> {
    assert_eq!(&checkpoint[..8], b"LOROXS05");
    let mut checkpoint = checkpoint.to_vec();
    checkpoint[8..40].copy_from_slice(&loro_internal::external_store_metadata_digest(xm));
    checkpoint
}

fn digest_hex(bytes: &[u8]) -> String {
    loro_internal::external_store_metadata_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tracker_digest(authenticated: &WireAuthenticatedTracker) -> [u8; 32] {
    let mut bytes = b"LORO canonical text tracker v1".to_vec();
    bytes.extend_from_slice(&(authenticated.owner.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&authenticated.owner);
    bytes.extend_from_slice(&(authenticated.tracker.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&authenticated.tracker);
    bytes.extend_from_slice(&authenticated.anchor_metadata_digest);
    loro_internal::external_store_metadata_digest(&bytes)
}

fn wire_frontiers_digest(bytes: &mut Vec<u8>, frontiers: &[WireId]) {
    bytes.extend_from_slice(&(frontiers.len() as u64).to_le_bytes());
    for id in frontiers {
        bytes.extend_from_slice(&id.peer.to_le_bytes());
        bytes.extend_from_slice(&id.counter.to_le_bytes());
    }
}

fn wire_source_digest(peer: u64, part: &WireCausalBoundaryPart) -> [u8; 32] {
    let mut bytes = b"LORO causal authenticated source v1".to_vec();
    bytes.extend_from_slice(&peer.to_le_bytes());
    bytes.extend_from_slice(&part.source_start.to_le_bytes());
    bytes.extend_from_slice(&part.source_end.to_le_bytes());
    bytes.extend_from_slice(&part.source_lamport.to_le_bytes());
    wire_frontiers_digest(&mut bytes, &part.source_deps);
    bytes.extend_from_slice(&part.change_count.to_le_bytes());
    bytes.extend_from_slice(&part.last_change_start.to_le_bytes());
    bytes.extend_from_slice(&part.boundary_digest);
    bytes.extend_from_slice(&part.anchor_metadata_digest);
    loro_internal::external_store_metadata_digest(&bytes)
}

fn wire_node_digest(node: &WireCausalNode) -> [u8; 32] {
    let proof = node.boundary_proof.as_ref().unwrap();
    let mut bytes = b"LORO causal span commitment v2".to_vec();
    bytes.extend_from_slice(&node.peer.to_le_bytes());
    bytes.extend_from_slice(&node.cnt.to_le_bytes());
    bytes.extend_from_slice(&node.lamport.to_le_bytes());
    bytes.extend_from_slice(&(node.len as u64).to_le_bytes());
    bytes.push(node.has_succ as u8);
    wire_frontiers_digest(&mut bytes, &node.deps);
    bytes.extend_from_slice(&(node.vv.len() as u64).to_le_bytes());
    for (peer, counter) in &node.vv {
        bytes.extend_from_slice(&peer.to_le_bytes());
        bytes.extend_from_slice(&counter.to_le_bytes());
    }
    bytes.extend_from_slice(&(proof.parts.len() as u64).to_le_bytes());
    for part in &proof.parts {
        bytes.extend_from_slice(&part.start.to_le_bytes());
        bytes.extend_from_slice(&part.end.to_le_bytes());
        bytes.extend_from_slice(&part.source_digest);
    }
    loro_internal::external_store_metadata_digest(&bytes)
}

#[test]
fn rustcrypto_sha256_matches_standard_known_answers() {
    assert_eq!(
        digest_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        digest_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        digest_hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

struct ConcurrentFixture {
    store_control: FaultStore,
    store: KvStoreHandle,
    checkpoint: Vec<u8>,
    old_base_update: Vec<u8>,
    full_memory: LoroDoc,
}

fn concurrent_fixture() -> ConcurrentFixture {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("key", "base").unwrap();
    external.get_text("text").insert(0, "base").unwrap();
    external.commit();

    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let old_branch = LoroDoc::new();
    old_branch.import(&base_updates).unwrap();
    old_branch.set_peer_id(2).unwrap();
    old_branch
        .get_map("map")
        .insert("key", "old-branch")
        .unwrap();
    old_branch.get_text("text").insert(4, "-old").unwrap();
    old_branch.commit();
    let old_base_update = old_branch.export(ExportMode::updates(&base_vv)).unwrap();

    let full_memory = LoroDoc::new();
    full_memory.import(&base_updates).unwrap();
    full_memory.set_peer_id(3).unwrap();
    full_memory
        .get_map("map")
        .insert("key", "new-branch")
        .unwrap();
    full_memory.get_text("text").insert(4, "-new").unwrap();
    full_memory.commit();
    let status = full_memory.import(&old_base_update).unwrap();
    assert!(status.pending.is_none());

    external.set_peer_id(3).unwrap();
    external.get_map("map").insert("key", "new-branch").unwrap();
    external.get_text("text").insert(4, "-new").unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    ConcurrentFixture {
        store_control,
        store,
        checkpoint,
        old_base_update,
        full_memory,
    }
}

struct RevisionFixture {
    checkpoint_1: Vec<u8>,
    store_1_control: FaultStore,
    store_1: KvStoreHandle,
    checkpoint_2: Vec<u8>,
    store_2_control: FaultStore,
    store_2: KvStoreHandle,
}

fn revision_fixture() -> RevisionFixture {
    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store.clone()).unwrap();
    doc.set_peer_id(7).unwrap();
    doc.get_map("map").insert("A", 1).unwrap();
    doc.get_text("text").insert(0, "A").unwrap();
    doc.commit();
    let checkpoint_1 = doc.flush_external_store().unwrap();
    let (store_1_control, store_1) = store_control.fork();

    doc.get_map("map").insert("B", 2).unwrap();
    doc.get_text("text").insert(1, "B").unwrap();
    doc.commit();
    let checkpoint_2 = doc.flush_external_store().unwrap();
    drop(doc);
    let (store_2_control, store_2) = store_control.fork();
    assert_eq!(
        checkpoint_for_xm(&checkpoint_2, &store_2_control.raw_get(b"xm").unwrap()),
        checkpoint_2,
        "test digest helper must reproduce the fork's exact-pair digest"
    );

    RevisionFixture {
        checkpoint_1,
        store_1_control,
        store_1,
        checkpoint_2,
        store_2_control,
        store_2,
    }
}

#[test]
fn old_base_map_and_text_merge_survive_reopen_flush_and_cache_eviction() {
    let fixture = concurrent_fixture();
    let reopened =
        LoroDoc::from_external_store(Some(&fixture.checkpoint), fixture.store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();

    let status = reopened.import(&fixture.old_base_update).unwrap();
    assert!(status.pending.is_none());
    let merged_checkpoint = reopened.flush_external_store().unwrap();
    drop(reopened);

    let reopened =
        LoroDoc::from_external_store(Some(&merged_checkpoint), fixture.store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(
        reopened.get_deep_value(),
        fixture.full_memory.get_deep_value()
    );
    assert_eq!(reopened.oplog_vv(), fixture.full_memory.oplog_vv());
    assert_eq!(
        reopened.get_map("map").get_deep_value(),
        fixture.full_memory.get_map("map").get_deep_value()
    );
    let merged_text = reopened.get_text("text").to_string();
    assert!(merged_text.contains("-new"));
    assert!(merged_text.contains("-old"));
}

#[test]
fn old_base_text_deletes_survive_reopen_and_cache_eviction() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_text("text").insert(0, "abcd").unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();

    let incoming = LoroDoc::new();
    incoming.import(&base_updates).unwrap();
    incoming.set_peer_id(2).unwrap();
    incoming.get_text("text").delete(2, 1).unwrap();
    incoming.commit();
    let incoming_update = incoming.export(ExportMode::updates(&base_vv)).unwrap();

    let expected = LoroDoc::new();
    expected.import(&base_updates).unwrap();
    expected.set_peer_id(3).unwrap();
    expected.get_text("text").delete(1, 1).unwrap();
    expected.commit();
    assert!(expected.import(&incoming_update).unwrap().pending.is_none());

    external.set_peer_id(3).unwrap();
    external.get_text("text").delete(1, 1).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    store_control.reset_reads();
    assert!(reopened.import(&incoming_update).unwrap().pending.is_none());
    let merged_checkpoint = reopened.flush_external_store().unwrap();
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&merged_checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
    assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
}

#[test]
fn multi_block_prefix_dependency_can_split_an_authenticated_interior_span() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(71).unwrap();
    let mut prefix_updates = Vec::new();
    let mut prefix_vv = loro::VersionVector::default();
    for batch in 0..6 {
        for index in 0..1024 {
            external
                .get_map("map")
                .insert(&format!("resident-{batch}-{index}"), batch * 1024 + index)
                .unwrap();
        }
        external.commit();
        if batch == 2 {
            prefix_vv = external.oplog_vv();
            prefix_updates = external.export(ExportMode::all_updates()).unwrap();
        }
    }
    let resident_updates = external.export(ExportMode::all_updates()).unwrap();
    let checkpoint = external.flush_external_store().unwrap();
    let peer_blocks = store_control
        .raw_change_blocks()
        .into_iter()
        .filter(|(key, _)| key[..8] == 71_u64.to_be_bytes())
        .count();
    assert!(
        peer_blocks >= 2,
        "fixture must persist the compact causal span across multiple physical blocks"
    );
    drop(external);

    let old_prefix = LoroDoc::new();
    old_prefix.import(&prefix_updates).unwrap();
    old_prefix.set_peer_id(72).unwrap();
    old_prefix
        .get_map("map")
        .insert("prefix-child", "depends-on-interior")
        .unwrap();
    old_prefix.commit();
    let interior_update = old_prefix.export(ExportMode::updates(&prefix_vv)).unwrap();

    let expected = LoroDoc::new();
    expected.import(&resident_updates).unwrap();
    assert!(expected.import(&interior_update).unwrap().pending.is_none());

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert!(reopened.import(&interior_update).unwrap().pending.is_none());
    let successor = reopened.flush_external_store().unwrap();
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&successor), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
    assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
}

#[test]
fn checkpoint_and_store_are_an_exact_pair_in_both_revision_directions() {
    let fixture = revision_fixture();
    assert!(
        LoroDoc::from_external_store(Some(&fixture.checkpoint_1), fixture.store_2.clone()).is_err()
    );
    assert!(
        LoroDoc::from_external_store(Some(&fixture.checkpoint_2), fixture.store_1.clone()).is_err()
    );

    let current =
        LoroDoc::from_external_store(Some(&fixture.checkpoint_2), fixture.store_2.clone()).unwrap();
    assert!(matches!(
        current.get_map("map").get("B"),
        Some(ValueOrContainer::Value(value)) if value == 2_i64.into()
    ));
    assert_eq!(current.get_text("text").to_string(), "AB");
    let checkpoint_3 = current.flush_external_store().unwrap();
    drop(current);

    let current =
        LoroDoc::from_external_store(Some(&checkpoint_3), fixture.store_2.clone()).unwrap();
    assert!(matches!(
        current.get_map("map").get("B"),
        Some(ValueOrContainer::Value(value)) if value == 2_i64.into()
    ));
    assert_eq!(current.get_text("text").to_string(), "AB");
}

#[test]
fn missing_tampered_or_truncated_store_metadata_is_rejected() {
    let fixture = revision_fixture();
    let valid_xm = fixture.store_2_control.raw_get(b"xm").unwrap();

    let (missing_control, missing_store) = fixture.store_2_control.fork();
    missing_control.raw_remove(b"xm");
    assert!(LoroDoc::from_external_store(Some(&fixture.checkpoint_2), missing_store).is_err());

    let (tampered_control, tampered_store) = fixture.store_2_control.fork();
    let mut tampered = valid_xm.to_vec();
    tampered[8] ^= 0x40;
    tampered_control.raw_set(b"xm", tampered);
    assert!(LoroDoc::from_external_store(Some(&fixture.checkpoint_2), tampered_store).is_err());

    let (truncated_control, truncated_store) = fixture.store_2_control.fork();
    truncated_control.raw_set(b"xm", valid_xm.slice(..valid_xm.len() - 5));
    assert!(LoroDoc::from_external_store(Some(&fixture.checkpoint_2), truncated_store).is_err());

    assert!(LoroDoc::from_external_store(Some(&fixture.checkpoint_2), fixture.store_2).is_ok());
}

#[test]
fn causal_and_baseline_must_match_the_authenticated_store_version() {
    let fixture = revision_fixture();
    let xm_1 = decode_xm(&fixture.store_1_control.raw_get(b"xm").unwrap());
    let xm_2 = decode_xm(&fixture.store_2_control.raw_get(b"xm").unwrap());

    for mixed in [
        XmParts {
            causal: xm_1.causal.clone(),
            ..xm_2.clone()
        },
        XmParts {
            baseline: xm_1.baseline,
            ..xm_2
        },
    ] {
        let mixed_xm = encode_xm(&mixed);
        let mixed_checkpoint = checkpoint_for_xm(&fixture.checkpoint_2, &mixed_xm);
        let (control, store) = fixture.store_2_control.fork();
        control.raw_set(b"xm", mixed_xm);
        assert!(LoroDoc::from_external_store(Some(&mixed_checkpoint), store).is_err());
    }
}

#[test]
fn omitted_or_altered_existing_text_tracker_is_rejected() {
    let fixture = revision_fixture();
    let xm = decode_xm(&fixture.store_2_control.raw_get(b"xm").unwrap());
    for baseline in [
        remove_baseline_trackers(&xm.baseline),
        alter_first_baseline_tracker(&xm.baseline),
    ] {
        let changed = XmParts {
            baseline,
            ..xm.clone()
        };
        let changed_xm = encode_xm(&changed);
        let changed_checkpoint = checkpoint_for_xm(&fixture.checkpoint_2, &changed_xm);
        let (control, store) = fixture.store_2_control.fork();
        control.raw_set(b"xm", changed_xm);
        assert!(LoroDoc::from_external_store(Some(&changed_checkpoint), store).is_err());
    }
}

#[test]
fn clearing_complete_tracker_arrays_is_rejected_by_the_authenticated_snapshot() {
    let (store_control, checkpoint) = two_text_checkpoint(true);
    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    parts.baseline = rewrite_first_baseline_tracker(&parts.baseline, |tracker| {
        tracker.spans.clear();
        tracker.deletes.clear();
    });
    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(error
        .to_string()
        .contains("text tracker commitment is invalid"));
}

#[test]
fn unknown_placeholder_tracker_span_is_rejected_even_with_recomputed_public_digests() {
    let (store_control, checkpoint) = two_text_checkpoint(false);
    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    let mut entries = decode_baseline_tracker_blobs(&parts.baseline);
    let authenticated = &mut entries[0].1;
    let mut tracker: WireTracker = postcard::from_bytes(&authenticated.tracker).unwrap();
    tracker.spans[0].id_peer = u64::MAX;
    authenticated.tracker = postcard::to_allocvec(&tracker).unwrap();
    authenticated.digest = tracker_digest(authenticated);
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(error.to_string().contains("UNKNOWN placeholder spans"));
}

#[test]
fn non_frontier_causal_forgery_is_rejected_against_change_headers() {
    let root_a = LoroDoc::new();
    root_a.set_peer_id(1).unwrap();
    root_a.get_map("map").insert("A", 1).unwrap();
    root_a.commit();
    let update_a = root_a.export(ExportMode::all_updates()).unwrap();

    let root_b = LoroDoc::new();
    root_b.set_peer_id(2).unwrap();
    root_b.get_map("map").insert("B", 2).unwrap();
    root_b.commit();
    let update_b = root_b.export(ExportMode::all_updates()).unwrap();

    let child_c = LoroDoc::new();
    child_c.import(&update_a).unwrap();
    child_c.set_peer_id(3).unwrap();
    child_c.get_map("map").insert("C", 3).unwrap();
    child_c.commit();
    let update_c = child_c.export(ExportMode::all_updates()).unwrap();

    let child_d = LoroDoc::new();
    child_d.import(&update_b).unwrap();
    child_d.set_peer_id(4).unwrap();
    child_d.get_map("map").insert("D", 4).unwrap();
    child_d.commit();
    let update_d = child_d.export(ExportMode::all_updates()).unwrap();

    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store).unwrap();
    external.import(&update_c).unwrap();
    external.import(&update_d).unwrap();
    external.set_peer_id(5).unwrap();
    external.get_map("map").insert("E", 5).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let original_xm = store_control.raw_get(b"xm").unwrap();
    let mut parts = decode_xm(&original_xm);
    assert_eq!(&parts.causal[..8], b"LOROCI02");
    let mut causal: WireCausalDag = postcard::from_bytes(&parts.causal[8..]).unwrap();
    let c = causal.nodes.iter().position(|node| node.peer == 3).unwrap();
    let d = causal.nodes.iter().position(|node| node.peer == 4).unwrap();
    assert!(!causal
        .frontiers
        .iter()
        .any(|id| id.peer == 3 || id.peer == 4));
    let c_deps = causal.nodes[c].deps.clone();
    let c_vv = causal.nodes[c].vv.clone();
    causal.nodes[c].deps = causal.nodes[d].deps.clone();
    causal.nodes[c].vv = causal.nodes[d].vv.clone();
    causal.nodes[d].deps = c_deps;
    causal.nodes[d].vv = c_vv;
    parts.causal = b"LOROCI02".to_vec();
    parts
        .causal
        .extend_from_slice(&postcard::to_allocvec(&causal).unwrap());

    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(error.to_string().contains("invalid DAG boundary proof"));
}

#[test]
fn compact_span_cannot_hide_a_nonlinear_interior_dependency_boundary() {
    let x = LoroDoc::new();
    x.set_peer_id(10).unwrap();
    x.get_map("map").insert("X", 1).unwrap();
    x.commit();
    let x_update = x.export(ExportMode::all_updates()).unwrap();

    let y = LoroDoc::new();
    y.import(&x_update).unwrap();
    y.set_peer_id(11).unwrap();
    y.get_map("map").insert("Y", 1).unwrap();
    y.commit();
    let y_update = y.export(ExportMode::all_updates()).unwrap();

    let p = LoroDoc::new();
    p.set_peer_id(20).unwrap();
    p.get_map("map").insert("P0", 0).unwrap();
    p.set_next_commit_message("P0");
    p.commit();
    p.get_map("map").insert("P1", 1).unwrap();
    p.set_next_commit_message("P1");
    p.commit();
    p.import(&x_update).unwrap();
    p.get_map("map").insert("P2", 2).unwrap();
    p.set_next_commit_message("P2");
    p.commit();
    p.get_map("map").insert("P3", 3).unwrap();
    p.set_next_commit_message("P3");
    p.commit();
    let p_update = p.export(ExportMode::all_updates()).unwrap();

    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store).unwrap();
    external.import(&y_update).unwrap();
    external.import(&p_update).unwrap();
    external.set_peer_id(30).unwrap();
    external.get_map("map").insert("C", 1).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    let mut causal: WireCausalDag = postcard::from_bytes(&parts.causal[8..]).unwrap();
    let mut p_nodes = causal
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.peer == 20)
        .map(|(index, node)| (index, node.cnt))
        .collect::<Vec<_>>();
    p_nodes.sort_by_key(|(_, counter)| *counter);
    assert_eq!(
        p_nodes.len(),
        2,
        "P2's dependency on X must create an interior causal boundary"
    );
    let first_index = p_nodes[0].0;
    let second_index = p_nodes[1].0;
    let second = causal.nodes[second_index].clone();
    assert_eq!(
        second.deps,
        vec![
            WireId {
                peer: 10,
                counter: 0
            },
            WireId {
                peer: 20,
                counter: 1
            }
        ]
    );
    causal.nodes[first_index].len =
        (second.cnt + second.len as i32 - causal.nodes[first_index].cnt) as u32;
    causal.nodes[first_index].has_succ = second.has_succ;
    causal.nodes.remove(second_index);
    parts.causal = b"LOROCI02".to_vec();
    parts
        .causal
        .extend_from_slice(&postcard::to_allocvec(&causal).unwrap());

    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(
        error.to_string().contains("invalid DAG boundary proof"),
        "hidden nonlinear interior boundary was rejected for the wrong reason: {error}"
    );
}

#[test]
fn forged_successor_with_all_public_digests_recomputed_cannot_replace_its_predecessor() {
    let root_a = LoroDoc::new();
    root_a.set_peer_id(1).unwrap();
    root_a.get_map("map").insert("A", 1).unwrap();
    root_a.commit();
    let update_a = root_a.export(ExportMode::all_updates()).unwrap();

    let root_b = LoroDoc::new();
    root_b.set_peer_id(2).unwrap();
    root_b.get_map("map").insert("B", 2).unwrap();
    root_b.commit();
    let update_b = root_b.export(ExportMode::all_updates()).unwrap();

    let child_c = LoroDoc::new();
    child_c.import(&update_a).unwrap();
    child_c.set_peer_id(3).unwrap();
    child_c.get_map("map").insert("C", 3).unwrap();
    child_c.commit();

    let child_d = LoroDoc::new();
    child_d.import(&update_b).unwrap();
    child_d.set_peer_id(4).unwrap();
    child_d.get_map("map").insert("D", 4).unwrap();
    child_d.commit();

    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external
        .import(&child_c.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    external
        .import(&child_d.export(ExportMode::all_updates()).unwrap())
        .unwrap();
    external.set_peer_id(5).unwrap();
    external.get_map("map").insert("E", 5).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();

    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    let mut causal: WireCausalDag = postcard::from_bytes(&parts.causal[8..]).unwrap();
    let c = causal.nodes.iter().position(|node| node.peer == 3).unwrap();
    let d = causal.nodes.iter().position(|node| node.peer == 4).unwrap();
    let c_deps = causal.nodes[c].deps.clone();
    let c_vv = causal.nodes[c].vv.clone();
    causal.nodes[c].deps = causal.nodes[d].deps.clone();
    causal.nodes[c].vv = causal.nodes[d].vv.clone();
    causal.nodes[d].deps = c_deps;
    causal.nodes[d].vv = c_vv;
    for index in [c, d] {
        let node = &mut causal.nodes[index];
        let deps = node.deps.clone();
        let peer = node.peer;
        let cnt = node.cnt;
        for part in &mut node.boundary_proof.as_mut().unwrap().parts {
            if part.source_start == cnt {
                part.source_deps = deps.clone();
            }
            part.source_digest = wire_source_digest(peer, part);
        }
        let digest = wire_node_digest(node);
        node.boundary_proof.as_mut().unwrap().node_digest = digest;
    }
    parts.causal = b"LOROCI02".to_vec();
    parts
        .causal
        .extend_from_slice(&postcard::to_allocvec(&causal).unwrap());
    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    assert_ne!(forged_checkpoint, checkpoint);
    let (replacement_control, replacement_store) = store_control.fork();
    replacement_control.raw_set(b"xm", forged_xm.clone());
    assert!(
        LoroDoc::from_external_store(Some(&forged_checkpoint), replacement_store).is_ok(),
        "public digests are not a MAC when the caller-authenticated record is replaced"
    );
    store_control.raw_set(b"xm", forged_xm);

    reopened.set_peer_id(6).unwrap();
    reopened.get_map("map").insert("F", 6).unwrap();
    reopened.commit();
    let error = reopened.flush_external_store().unwrap_err();
    assert!(error
        .to_string()
        .contains("authenticated predecessor metadata changed"));
}

#[test]
fn semantic_rich_text_delete_target_forgery_is_rejected() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_text("text").insert(0, "abcd").unwrap();
    external.commit();
    external.set_peer_id(2).unwrap();
    external.get_text("text").delete(1, 1).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let original_xm = store_control.raw_get(b"xm").unwrap();
    let mut parts = decode_xm(&original_xm);
    parts.baseline = rewrite_first_baseline_tracker(&parts.baseline, |tracker| {
        assert_eq!(tracker.deletes.len(), 1);
        let delete = &mut tracker.deletes[0];
        let len = delete.target_end - delete.target_start;
        assert_eq!(len.abs(), 1);
        if delete.target_start == 0 || delete.target_end == 0 {
            delete.target_start += 1;
            delete.target_end += 1;
        } else {
            delete.target_start -= 1;
            delete.target_end -= 1;
        }
    });
    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(error
        .to_string()
        .contains("text tracker commitment is invalid"));

    let (_, original_store) = store_control.fork();
    assert!(LoroDoc::from_external_store(Some(&checkpoint), original_store).is_ok());
}

#[test]
fn tracker_commitment_binds_real_ids_origins_and_delete_status() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_text("text").insert(0, "abcd").unwrap();
    external.commit();
    external.set_peer_id(2).unwrap();
    external.get_text("text").insert(2, "X").unwrap();
    external.commit();
    external.get_text("text").delete(1, 2).unwrap();
    external.commit();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let original_parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    for field in 0..5 {
        let mut parts = original_parts.clone();
        parts.baseline = rewrite_first_baseline_tracker(&parts.baseline, |tracker| {
            let span = tracker.spans.first_mut().unwrap();
            match field {
                0 => span.real_id = Some((91, 7)),
                1 => span.origin_left = Some((92, 8)),
                2 => span.origin_right = Some((93, 9)),
                3 => span.future = !span.future,
                4 => span.delete_times = span.delete_times.saturating_add(1),
                _ => unreachable!(),
            }
        });
        let forged_xm = encode_xm(&parts);
        let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
        let (forged_control, forged_store) = store_control.fork();
        forged_control.raw_set(b"xm", forged_xm);
        let error =
            LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("text tracker commitment is invalid"),
            "tracker field {field} escaped its complete snapshot commitment: {error}"
        );
    }
}

fn two_text_checkpoint(with_delete: bool) -> (FaultStore, Vec<u8>) {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_text("left").insert(0, "left").unwrap();
    external.commit();
    external.set_peer_id(2).unwrap();
    external.get_text("right").insert(0, "right").unwrap();
    external.commit();
    if with_delete {
        external.set_peer_id(3).unwrap();
        external.get_text("left").delete(1, 1).unwrap();
        external.commit();
    }
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);
    (store_control, checkpoint)
}

#[test]
fn swapping_complete_trackers_between_text_containers_is_rejected_without_deletes() {
    let (store_control, checkpoint) = two_text_checkpoint(false);
    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    let mut entries = decode_baseline_tracker_blobs(&parts.baseline);
    assert_eq!(entries.len(), 2);
    for (_, authenticated) in &entries {
        let tracker: WireTracker = postcard::from_bytes(&authenticated.tracker).unwrap();
        assert!(tracker.deletes.is_empty());
        assert!(!tracker.spans.is_empty());
    }
    let first = entries[0].1.clone();
    entries[0].1 = entries[1].1.clone();
    entries[1].1 = first;
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);

    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("text tracker owner or commitment does not match"),
        "whole-tracker transplant was rejected for the wrong reason: {error}"
    );
}

#[test]
fn moving_a_valid_delete_cursor_to_another_text_tracker_is_rejected() {
    let (store_control, checkpoint) = two_text_checkpoint(true);
    let mut parts = decode_xm(&store_control.raw_get(b"xm").unwrap());
    let mut entries = decode_baseline_tracker_blobs(&parts.baseline);
    assert_eq!(entries.len(), 2);
    let mut trackers = entries
        .iter()
        .map(|(_, authenticated)| {
            postcard::from_bytes::<WireTracker>(&authenticated.tracker).unwrap()
        })
        .collect::<Vec<_>>();
    let source = trackers
        .iter()
        .position(|tracker| !tracker.deletes.is_empty())
        .unwrap();
    let destination = 1 - source;
    assert!(trackers[destination].deletes.is_empty());
    let delete = trackers[source].deletes.remove(0);
    trackers[destination].deletes.push(delete);
    for ((_, authenticated), tracker) in entries.iter_mut().zip(trackers) {
        authenticated.tracker = postcard::to_allocvec(&tracker).unwrap();
    }
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);

    let forged_xm = encode_xm(&parts);
    let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
    let (forged_control, forged_store) = store_control.fork();
    forged_control.raw_set(b"xm", forged_xm);
    let error = LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("text tracker commitment is invalid"),
        "cross-container delete cursor was rejected for the wrong reason: {error}"
    );
}

#[test]
fn two_text_containers_converge_after_old_base_replay_and_reopen() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_text("left").insert(0, "left").unwrap();
    external.get_text("right").insert(0, "right").unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();

    let incoming = LoroDoc::new();
    incoming.import(&base_updates).unwrap();
    incoming.set_peer_id(2).unwrap();
    incoming.get_text("left").insert(4, "-incoming").unwrap();
    incoming.get_text("right").delete(0, 1).unwrap();
    incoming.commit();
    let incoming_update = incoming.export(ExportMode::updates(&base_vv)).unwrap();

    external.set_peer_id(3).unwrap();
    external.get_text("left").delete(0, 1).unwrap();
    external.get_text("right").insert(5, "-resident").unwrap();
    external.commit();
    let resident_updates = external.export(ExportMode::all_updates()).unwrap();
    let expected = LoroDoc::new();
    expected.import(&resident_updates).unwrap();
    assert!(expected.import(&incoming_update).unwrap().pending.is_none());

    let checkpoint = external.flush_external_store().unwrap();
    drop(external);
    reset_external_tracker_codec_stats();
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats::default(),
        "reopen must validate authenticated tracker wires without materializing mutable trackers"
    );
    assert!(reopened.import(&incoming_update).unwrap().pending.is_none());
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_decodes: 2,
            ..ExternalTrackerCodecStats::default()
        },
        "old-base replay must materialize only its two touched text trackers"
    );
    let checkpoint = reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 2,
            snapshot_decodes: 2,
            commitment_hashes: 2,
            coverage_scans: 0,
            compact_births: 0,
            compact_promotions: 0,
            generic_encodes: 2,
        },
        "sealing old-base replay must encode and hash only its touched trackers"
    );
    drop(reopened);
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
    assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
    assert!(store_control.raw_get(b"xm").is_some());
}

#[test]
fn incremental_tracker_sealing_reuses_untouched_authenticated_entries() {
    const TRACKERS: usize = 256;
    const TOUCHED: usize = 137;

    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store).unwrap();
    doc.set_peer_id(1).unwrap();
    for index in 0..TRACKERS {
        doc.get_text(format!("text-{index:03}"))
            .insert(0, "x")
            .unwrap();
    }
    doc.commit();
    doc.flush_external_store().unwrap();

    let first_xm = store_control.raw_get(b"xm").unwrap();
    let first_entries = decode_baseline_tracker_blobs(&decode_xm(&first_xm).baseline);
    assert_eq!(first_entries.len(), TRACKERS);

    reset_external_tracker_codec_stats();
    doc.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats::default(),
        "a no-delta flush must reuse the exact cached IB03 baseline"
    );
    let no_delta_xm = store_control.raw_get(b"xm").unwrap();
    let no_delta_entries = decode_baseline_tracker_blobs(&decode_xm(&no_delta_xm).baseline);
    assert_eq!(
        no_delta_entries, first_entries,
        "a no-delta flush rewrote an authenticated tracker entry"
    );

    reset_external_tracker_codec_stats();
    doc.set_peer_id(2).unwrap();
    doc.get_text(format!("text-{TOUCHED:03}"))
        .insert(1, "y")
        .unwrap();
    doc.commit();
    doc.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            snapshot_decodes: 1,
            commitment_hashes: 1,
            coverage_scans: 0,
            compact_births: 0,
            compact_promotions: 0,
            generic_encodes: 1,
        },
        "one touched tracker among many must be the only tracker decoded, encoded, and hashed"
    );

    let successor_xm = store_control.raw_get(b"xm").unwrap();
    let successor_entries = decode_baseline_tracker_blobs(&decode_xm(&successor_xm).baseline);
    assert_eq!(successor_entries.len(), TRACKERS);
    let changed = first_entries
        .iter()
        .zip(&successor_entries)
        .filter(|(before, after)| before != after)
        .collect::<Vec<_>>();
    assert_eq!(
        changed.len(),
        1,
        "successor publication changed more than the touched tracker"
    );
    assert_eq!(
        changed[0].1 .1.anchor_metadata_digest,
        loro_internal::external_store_metadata_digest(&no_delta_xm),
        "changed tracker was not anchored to the exact prior authenticated metadata"
    );
}

#[test]
fn arena_growth_invalidates_cached_tracker_coverage() {
    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store.clone()).unwrap();
    doc.get_map("map").insert("base", 1).unwrap();
    doc.commit();
    doc.flush_external_store().unwrap();

    reset_external_tracker_codec_stats();
    doc.get_text("empty");
    let checkpoint = doc.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            commitment_hashes: 1,
            coverage_scans: 1,
            generic_encodes: 1,
            ..ExternalTrackerCodecStats::default()
        },
        "registering an empty text container must invalidate cached complete coverage"
    );
    assert_eq!(
        decode_baseline_tracker_blobs(&decode_xm(&store_control.raw_get(b"xm").unwrap()).baseline)
            .len(),
        1
    );
    drop(doc);
    LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
}

fn publish_imported_fresh_text(
    text: &str,
    register_empty: bool,
) -> (
    FaultStore,
    KvStoreHandle,
    Vec<u8>,
    Vec<u8>,
    ExternalTrackerCodecStats,
) {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 1).unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    if register_empty {
        external.get_text("later");
    }
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let branch = LoroDoc::new();
    branch.import(&base_updates).unwrap();
    branch.set_peer_id(2).unwrap();
    branch.get_text("later").insert(0, text).unwrap();
    branch.commit();
    let update = branch.export(ExportMode::updates(&base_vv)).unwrap();

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    reset_external_tracker_codec_stats();
    assert!(reopened.import(&update).unwrap().pending.is_none());
    assert_eq!(reopened.get_text("later").to_string(), text);
    let checkpoint = reopened.flush_external_store().unwrap();
    let stats = external_tracker_codec_stats();
    let all_updates = reopened.export(ExportMode::all_updates()).unwrap();
    let published_xm = store_control.raw_get(b"xm").unwrap();
    reset_external_tracker_codec_stats();
    reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats::default(),
        "a no-delta fresh-text flush must perform zero tracker codec work"
    );
    assert_eq!(
        store_control.raw_get(b"xm").unwrap(),
        published_xm,
        "a no-delta fresh-text flush must reuse its exact encoding"
    );
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_text("later").to_string(), text);
    drop(reopened);
    (store_control, store, checkpoint, all_updates, stats)
}

#[test]
fn compact_fresh_unicode_text_matches_forced_generic_snapshot_and_reuses_encoding() {
    let text = "A🙂e\u{301}中";
    let (compact_control, compact_store, compact_checkpoint, _, compact_stats) =
        publish_imported_fresh_text(text, false);
    let (generic_control, _generic_store, _checkpoint, _, generic_stats) =
        publish_imported_fresh_text(text, true);

    let compact_xm = compact_control.raw_get(b"xm").unwrap();
    let generic_xm = generic_control.raw_get(b"xm").unwrap();
    let compact_entries = decode_baseline_tracker_blobs(&decode_xm(&compact_xm).baseline);
    let generic_entries = decode_baseline_tracker_blobs(&decode_xm(&generic_xm).baseline);
    assert_eq!(compact_entries.len(), 1);
    assert_eq!(generic_entries.len(), 1);
    assert_eq!(
        compact_entries[0].1.tracker, generic_entries[0].1.tracker,
        "compact sealing must emit the canonical generic tracker bytes"
    );
    let tracker: WireTracker = postcard::from_bytes(&compact_entries[0].1.tracker).unwrap();
    assert_eq!(tracker.spans.len(), 1);
    assert!(tracker.deletes.is_empty());
    assert_eq!(tracker.applied_vv, tracker.current_vv);
    let span = &tracker.spans[0];
    assert_eq!(span.real_id, Some((span.id_peer, span.id_counter)));
    assert!(!span.future);
    assert_eq!(span.delete_times, 0);
    assert_eq!(span.origin_left, None);
    assert_eq!(span.origin_right, None);
    assert_eq!(span.len as usize, text.chars().count());

    assert_eq!(
        compact_stats,
        ExternalTrackerCodecStats {
            compact_births: 1,
            commitment_hashes: 1,
            coverage_scans: 1,
            ..ExternalTrackerCodecStats::default()
        }
    );
    assert_eq!(
        generic_stats,
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            snapshot_decodes: 1,
            commitment_hashes: 1,
            coverage_scans: 0,
            compact_births: 0,
            compact_promotions: 0,
            generic_encodes: 1,
        }
    );

    drop(compact_store);
    drop(compact_checkpoint);
}

#[test]
fn multiple_fresh_text_ops_in_one_import_use_the_generic_tracker() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 1).unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let branch = LoroDoc::new();
    branch.import(&base_updates).unwrap();
    branch.set_peer_id(2).unwrap();
    branch.get_text("later").insert(0, "ab").unwrap();
    branch.commit();
    branch.get_text("later").delete(0, 1).unwrap();
    branch.commit();
    let update = branch.export(ExportMode::updates(&base_vv)).unwrap();

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    reset_external_tracker_codec_stats();
    assert!(reopened.import(&update).unwrap().pending.is_none());
    assert_eq!(reopened.get_text("later").to_string(), "b");
    reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            commitment_hashes: 1,
            coverage_scans: 1,
            generic_encodes: 1,
            ..ExternalTrackerCodecStats::default()
        }
    );
    assert_eq!(
        decode_baseline_tracker_blobs(&decode_xm(&store_control.raw_get(b"xm").unwrap()).baseline)
            .len(),
        1
    );
}

#[test]
fn compact_birth_promotes_for_later_sequential_insert_and_delete() {
    let (_store_control, store, checkpoint, base_updates, _) =
        publish_imported_fresh_text("abcd", false);
    let editor = LoroDoc::new();
    editor.import(&base_updates).unwrap();
    editor.set_peer_id(3).unwrap();
    let base_vv = editor.oplog_vv();
    editor.get_text("later").insert(2, "X").unwrap();
    editor.commit();
    let after_insert_vv = editor.oplog_vv();
    let insert_update = editor.export(ExportMode::updates(&base_vv)).unwrap();
    editor.get_text("later").delete(1, 2).unwrap();
    editor.commit();
    let delete_update = editor
        .export(ExportMode::updates(&after_insert_vv))
        .unwrap();

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reset_external_tracker_codec_stats();
    assert!(reopened.import(&insert_update).unwrap().pending.is_none());
    assert_eq!(reopened.get_text("later").to_string(), "abXcd");
    let checkpoint = reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            snapshot_decodes: 1,
            commitment_hashes: 1,
            compact_promotions: 1,
            generic_encodes: 1,
            ..ExternalTrackerCodecStats::default()
        }
    );
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    reset_external_tracker_codec_stats();
    assert!(reopened.import(&delete_update).unwrap().pending.is_none());
    assert_eq!(reopened.get_text("later").to_string(), "acd");
    let checkpoint = reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            snapshot_encodes: 1,
            snapshot_decodes: 1,
            commitment_hashes: 1,
            generic_encodes: 1,
            ..ExternalTrackerCodecStats::default()
        }
    );
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_text("later").to_string(), "acd");
    assert_eq!(reopened.oplog_vv(), editor.oplog_vv());
}

#[test]
fn compact_birth_handles_concurrent_old_base_insert_delete_in_both_orders() {
    let (store_control, first_store, checkpoint, base_updates, _) =
        publish_imported_fresh_text("abcd", false);
    let (_second_control, second_store) = store_control.fork();

    let base = LoroDoc::new();
    base.import(&base_updates).unwrap();
    let base_vv = base.oplog_vv();

    let insert = LoroDoc::new();
    insert.import(&base_updates).unwrap();
    insert.set_peer_id(3).unwrap();
    insert.get_text("later").insert(2, "X").unwrap();
    insert.commit();
    let insert_update = insert.export(ExportMode::updates(&base_vv)).unwrap();

    let delete = LoroDoc::new();
    delete.import(&base_updates).unwrap();
    delete.set_peer_id(4).unwrap();
    delete.get_text("later").delete(1, 1).unwrap();
    delete.commit();
    let delete_update = delete.export(ExportMode::updates(&base_vv)).unwrap();

    for (store, resident, incoming) in [
        (first_store, &insert_update, &delete_update),
        (second_store, &delete_update, &insert_update),
    ] {
        let expected = LoroDoc::new();
        expected.import(&base_updates).unwrap();
        assert!(expected.import(resident).unwrap().pending.is_none());
        assert!(expected.import(incoming).unwrap().pending.is_none());

        reset_external_tracker_codec_stats();
        let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert!(reopened.import(resident).unwrap().pending.is_none());
        let resident_checkpoint = reopened.flush_external_store().unwrap();
        drop(reopened);

        let reopened =
            LoroDoc::from_external_store(Some(&resident_checkpoint), store.clone()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert!(reopened.import(incoming).unwrap().pending.is_none());
        let merged_checkpoint = reopened.flush_external_store().unwrap();
        drop(reopened);

        assert_eq!(
            external_tracker_codec_stats(),
            ExternalTrackerCodecStats {
                snapshot_encodes: 2,
                snapshot_decodes: 2,
                commitment_hashes: 2,
                compact_promotions: 1,
                generic_encodes: 2,
                ..ExternalTrackerCodecStats::default()
            }
        );
        let reopened = LoroDoc::from_external_store(Some(&merged_checkpoint), store).unwrap();
        reopened.evict_external_store_cache().unwrap();
        assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
        assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
    }
}

#[test]
fn unsupported_rich_text_operation_never_enters_the_compact_path() {
    let (_store_control, store, checkpoint, base_updates, _) =
        publish_imported_fresh_text("x", false);
    let branch = LoroDoc::new();
    branch.import(&base_updates).unwrap();
    let base_vv = branch.oplog_vv();
    branch.set_peer_id(3).unwrap();
    branch.get_text("later").mark(0..1, "bold", true).unwrap();
    branch.commit();
    let update = branch.export(ExportMode::updates(&base_vv)).unwrap();

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    let before_vv = reopened.oplog_vv();
    let before_state = reopened.get_deep_value();
    reset_external_tracker_codec_stats();
    let error = reopened.import(&update).unwrap_err();
    assert!(error
        .to_string()
        .contains("rich-text styles and non-text list operations are unsupported"));
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats::default()
    );
    assert_eq!(reopened.oplog_vv(), before_vv);
    assert_eq!(reopened.get_deep_value(), before_state);
}

#[test]
fn compact_birth_baseline_rejects_owner_digest_anchor_truncation_and_version_forgery() {
    let (store_control, _store, checkpoint, _, _) = publish_imported_fresh_text("compact", false);
    let original_xm = store_control.raw_get(b"xm").unwrap();
    let original_parts = decode_xm(&original_xm);
    let original_entries = decode_baseline_tracker_blobs(&original_parts.baseline);
    assert_eq!(original_entries.len(), 1);

    let mut cases = Vec::new();

    let mut entries = original_entries.clone();
    entries[0].1.owner.push(0xff);
    let mut parts = original_parts.clone();
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    cases.push(("owner", encode_xm(&parts), "text tracker owner is invalid"));

    let mut entries = original_entries.clone();
    entries[0].1.digest[0] ^= 0x80;
    let mut parts = original_parts.clone();
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    cases.push((
        "digest",
        encode_xm(&parts),
        "text tracker commitment is invalid",
    ));

    let mut entries = original_entries.clone();
    entries[0].1.anchor_metadata_digest[0] ^= 0x40;
    let mut parts = original_parts.clone();
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    cases.push((
        "anchor",
        encode_xm(&parts),
        "text tracker commitment is invalid",
    ));

    let mut entries = original_entries.clone();
    let truncated_len = entries[0].1.tracker.len() / 2;
    entries[0].1.tracker.truncate(truncated_len);
    entries[0].1.digest = tracker_digest(&entries[0].1);
    let mut parts = original_parts.clone();
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    cases.push((
        "truncation",
        encode_xm(&parts),
        "invalid rich-text tracker encoding",
    ));

    let mut entries = original_entries;
    let mut tracker: WireTracker = postcard::from_bytes(&entries[0].1.tracker).unwrap();
    tracker.current_vv.insert(99, 1);
    entries[0].1.tracker = postcard::to_allocvec(&tracker).unwrap();
    entries[0].1.digest = tracker_digest(&entries[0].1);
    let mut parts = original_parts;
    parts.baseline = encode_baseline_tracker_blobs(&parts.baseline, &entries);
    cases.push((
        "version",
        encode_xm(&parts),
        "rich-text tracker current version is invalid",
    ));

    for (label, forged_xm, expected_error) in cases {
        let (forged_control, forged_store) = store_control.fork();
        forged_control.raw_set(b"xm", forged_xm.clone());
        let forged_checkpoint = checkpoint_for_xm(&checkpoint, &forged_xm);
        let error = match LoroDoc::from_external_store(Some(&forged_checkpoint), forged_store) {
            Ok(_) => panic!("{label} forgery was accepted"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected_error),
            "{label} forgery failed with unexpected error: {error}"
        );
    }
}

#[test]
fn text_container_created_after_baseline_gets_a_new_tracker() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 1).unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let branch = LoroDoc::new();
    branch.import(&base_updates).unwrap();
    branch.set_peer_id(2).unwrap();
    branch.get_text("later").insert(0, "post-baseline").unwrap();
    branch.commit();
    let update = branch.export(ExportMode::updates(&base_vv)).unwrap();

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    reset_external_tracker_codec_stats();
    assert!(reopened.import(&update).unwrap().pending.is_none());
    let checkpoint = reopened.flush_external_store().unwrap();
    assert_eq!(
        external_tracker_codec_stats(),
        ExternalTrackerCodecStats {
            compact_births: 1,
            commitment_hashes: 1,
            coverage_scans: 1,
            ..ExternalTrackerCodecStats::default()
        }
    );
    drop(reopened);
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    assert_eq!(reopened.get_text("later").to_string(), "post-baseline");
    assert!(store_control.raw_get(b"xm").is_some());
}

#[test]
fn authentication_read_failure_is_returned_before_open_or_import_success() {
    let fixture = concurrent_fixture();
    fixture.store_control.set_fail_reads(true);
    let open_error =
        LoroDoc::from_external_store(Some(&fixture.checkpoint), fixture.store.clone()).unwrap_err();
    assert!(open_error.to_string().contains("authentication denied"));

    fixture.store_control.set_fail_reads(false);
    let reopened =
        LoroDoc::from_external_store(Some(&fixture.checkpoint), fixture.store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    fixture.store_control.set_fail_reads(true);
    let import_error = reopened.import(&fixture.old_base_update).unwrap_err();
    assert!(import_error.to_string().contains("authentication denied"));

    fixture.store_control.set_fail_reads(false);
    let status = reopened.import(&fixture.old_base_update).unwrap();
    assert!(status.pending.is_none());
    assert_eq!(
        reopened.get_deep_value(),
        fixture.full_memory.get_deep_value()
    );
}

#[test]
fn late_change_block_read_failure_rolls_back_before_recovery() {
    let fixture = concurrent_fixture();
    let reopened =
        LoroDoc::from_external_store(Some(&fixture.checkpoint), fixture.store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    let before_vv = reopened.oplog_vv();
    let before_state = reopened.get_deep_value();

    fixture.store_control.reset_reads();
    fixture.store_control.set_fail_scans(true);
    let error = reopened.import(&fixture.old_base_update).unwrap_err();
    assert!(error
        .to_string()
        .contains("authentication denied on change-block read"));
    assert_eq!(reopened.oplog_vv(), before_vv);
    assert_eq!(reopened.get_deep_value(), before_state);

    fixture.store_control.set_fail_scans(false);
    assert!(reopened
        .import(&fixture.old_base_update)
        .unwrap()
        .pending
        .is_none());
    assert_eq!(
        reopened.get_deep_value(),
        fixture.full_memory.get_deep_value()
    );
}

#[test]
fn physically_missing_or_corrupt_change_block_rolls_back_and_recovers() {
    for corrupt in [false, true] {
        let fixture = concurrent_fixture();
        let reopened =
            LoroDoc::from_external_store(Some(&fixture.checkpoint), fixture.store.clone()).unwrap();
        reopened.evict_external_store_cache().unwrap();
        let before_vv = reopened.oplog_vv();
        let before_state = reopened.get_deep_value();
        let blocks = fixture.store_control.raw_change_blocks();
        assert!(!blocks.is_empty());

        for (key, value) in &blocks {
            if corrupt {
                fixture
                    .store_control
                    .raw_set(key, value.slice(..value.len().max(2) / 2));
            } else {
                fixture.store_control.raw_remove(key);
            }
        }
        fixture.store_control.reset_reads();
        let error = reopened.import(&fixture.old_base_update).unwrap_err();
        assert!(
            error.to_string().contains("missing") || error.to_string().contains("corrupt"),
            "unexpected physical block failure: {error}"
        );
        assert_eq!(reopened.oplog_vv(), before_vv);
        assert_eq!(reopened.get_deep_value(), before_state);

        for (key, value) in blocks {
            fixture.store_control.raw_set(&key, value);
        }
        assert!(reopened
            .import(&fixture.old_base_update)
            .unwrap()
            .pending
            .is_none());
        assert_eq!(
            reopened.get_deep_value(),
            fixture.full_memory.get_deep_value()
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum ContentKind {
    Map,
    Text,
}

#[derive(Clone, Copy, Debug)]
enum DeliveryOrder {
    LeftThenRight,
    RightThenLeft,
}

fn branch_update(
    base_updates: &[u8],
    base_vv: &loro::VersionVector,
    peer: u64,
    kind: ContentKind,
    prefix: &str,
    count: usize,
) -> Vec<u8> {
    let branch = LoroDoc::new();
    branch.import(base_updates).unwrap();
    branch.set_peer_id(peer).unwrap();
    for index in 0..count {
        match kind {
            ContentKind::Map => branch
                .get_map("map")
                .insert(&format!("{prefix}-{index:04}"), index as i64)
                .unwrap(),
            ContentKind::Text => branch.get_text("text").insert(1 + index, prefix).unwrap(),
        }
        branch.commit();
    }
    branch.export(ExportMode::updates(base_vv)).unwrap()
}

#[test]
fn unsupported_nested_container_fails_before_import_or_history_scan() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 1).unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let branch = LoroDoc::new();
    branch.import(&base_updates).unwrap();
    branch.set_peer_id(2).unwrap();
    branch
        .get_map("map")
        .insert_container("nested", LoroMap::new())
        .unwrap()
        .insert("value", 2)
        .unwrap();
    branch.commit();
    let nested_update = branch.export(ExportMode::updates(&base_vv)).unwrap();

    let memory = LoroDoc::new();
    memory.import(&base_updates).unwrap();
    assert!(memory.import(&nested_update).is_ok());

    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    let before_vv = reopened.oplog_vv();
    let before_state = reopened.get_deep_value();
    store_control.reset_reads();
    let error = reopened.import(&nested_update).unwrap_err();
    assert!(error
        .to_string()
        .contains("nested containers are unsupported"));
    assert_eq!(reopened.oplog_vv(), before_vv);
    assert_eq!(reopened.get_deep_value(), before_state);
    assert_eq!(
        store_control.reads().ops,
        0,
        "unsupported input must fail before touching historical blocks"
    );
}

#[test]
fn unsupported_list_tree_and_style_fail_before_import_or_history_scan() {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 1).unwrap();
    external.get_text("text").insert(0, "x").unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    let list = LoroDoc::new();
    list.import(&base_updates).unwrap();
    list.set_peer_id(2).unwrap();
    list.get_list("list").insert(0, "item").unwrap();
    list.commit();

    let tree = LoroDoc::new();
    tree.import(&base_updates).unwrap();
    tree.set_peer_id(3).unwrap();
    tree.get_tree("tree").create(None).unwrap();
    tree.commit();

    let style = LoroDoc::new();
    style.import(&base_updates).unwrap();
    style.set_peer_id(4).unwrap();
    style.get_text("text").mark(0..1, "bold", true).unwrap();
    style.commit();

    let cases = [
        (
            "list",
            list.export(ExportMode::updates(&base_vv)).unwrap(),
            "only scalar maps and plain text are supported",
        ),
        (
            "tree",
            tree.export(ExportMode::updates(&base_vv)).unwrap(),
            "only scalar maps and plain text are supported",
        ),
        (
            "style",
            style.export(ExportMode::updates(&base_vv)).unwrap(),
            "rich-text styles and non-text list operations are unsupported",
        ),
    ];
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    for (label, update, expected_error) in cases {
        let before_vv = reopened.oplog_vv();
        let before_state = reopened.get_deep_value();
        store_control.reset_reads();
        let error = reopened.import(&update).unwrap_err();
        assert!(
            error.to_string().contains(expected_error),
            "{label} failed with unexpected error: {error}"
        );
        assert_eq!(reopened.oplog_vv(), before_vv);
        assert_eq!(reopened.get_deep_value(), before_state);
        assert_eq!(
            store_control.reads().ops,
            0,
            "{label} preflight touched historical blocks"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum LongBranch {
    Left,
    Right,
}

fn counting_case(
    kind: ContentKind,
    delivery: DeliveryOrder,
    long_branch: LongBranch,
    age: usize,
) -> StoreReads {
    let (store_control, store) = FaultStore::new();
    let external = LoroDoc::from_external_store(None, store.clone()).unwrap();
    external.set_peer_id(1).unwrap();
    external.get_map("map").insert("base", 0).unwrap();
    external.get_text("text").insert(0, "b").unwrap();
    external.commit();
    let base_vv = external.oplog_vv();
    let base_updates = external.export(ExportMode::all_updates()).unwrap();
    let (left_len, right_len) = match long_branch {
        LongBranch::Left => (age, 1),
        LongBranch::Right => (1, age),
    };
    let left = branch_update(&base_updates, &base_vv, 2, kind, "l", left_len);
    let right = branch_update(&base_updates, &base_vv, 3, kind, "r", right_len);
    let (resident, incoming) = match delivery {
        DeliveryOrder::LeftThenRight => (&left, &right),
        DeliveryOrder::RightThenLeft => (&right, &left),
    };

    let expected = LoroDoc::new();
    expected.import(&base_updates).unwrap();
    assert!(expected.import(resident).unwrap().pending.is_none());
    assert!(expected.import(incoming).unwrap().pending.is_none());

    assert!(external.import(resident).unwrap().pending.is_none());
    let checkpoint = external.flush_external_store().unwrap();
    drop(external);

    store_control.reset_reads();
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store.clone()).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert!(reopened.import(incoming).unwrap().pending.is_none());
    let reads = store_control.reads();
    let merged_checkpoint = reopened.flush_external_store().unwrap();
    drop(reopened);

    let reopened = LoroDoc::from_external_store(Some(&merged_checkpoint), store).unwrap();
    reopened.evict_external_store_cache().unwrap();
    assert_eq!(reopened.get_deep_value(), expected.get_deep_value());
    assert_eq!(reopened.oplog_vv(), expected.oplog_vv());
    reads
}

#[test]
fn old_base_import_store_work_is_history_age_independent() {
    let mut cases = 0;
    for kind in [ContentKind::Map, ContentKind::Text] {
        for delivery in [DeliveryOrder::LeftThenRight, DeliveryOrder::RightThenLeft] {
            for long_branch in [LongBranch::Left, LongBranch::Right] {
                let short = counting_case(kind, delivery, long_branch, 8);
                let long = counting_case(kind, delivery, long_branch, 4096);
                cases += 2;
                assert!(short.ops > 0 && long.ops > 0);
                assert!(
                    short.keys.iter().any(|key| key == b"xm")
                        && long.keys.iter().any(|key| key == b"xm"),
                    "every reopen must authenticate canonical store metadata"
                );
                assert!(
                    long.ops <= short.ops + 4,
                    "{kind:?} {delivery:?} {long_branch:?} reads grew with page age: short={short:?} long={long:?}"
                );
                assert!(
                    long.bytes <= short.bytes.saturating_mul(2).saturating_add(16 * 1024),
                    "{kind:?} {delivery:?} {long_branch:?} bytes grew with page age: short={short:?} long={long:?}"
                );
                if std::env::var_os("LORO_REPORT_STORE_READS").is_some() {
                    println!(
                        "{kind:?} {delivery:?} {long_branch:?}: age8={}/{} age4096={}/{}",
                        short.ops, short.bytes, long.ops, long.bytes
                    );
                }
            }
        }
    }
    assert_eq!(cases, 16, "matrix must execute all 16 branch/age cases");
}

fn assert_reopen_reads_only_compact_authenticated_metadata(
    store: &FaultStore,
    reads: &StoreReads,
    label: &str,
) -> usize {
    let block_reads = reads
        .keys
        .iter()
        .filter(|key| key.len() == 12)
        .collect::<Vec<_>>();
    assert!(
        block_reads.is_empty(),
        "{label} read historical text change blocks during reopen: {block_reads:?}"
    );
    assert!(
        reads.keys.iter().any(|key| key == b"xm"),
        "{label} did not authenticate canonical metadata"
    );
    let xm = store.raw_get(b"xm").unwrap();
    assert!(
        reads.bytes <= b"xm".len() + xm.len() + 128,
        "{label} read bytes beyond compact authenticated metadata: {reads:?}"
    );
    xm.len()
}

fn alternating_peer_reopen_case(operation_count: usize) -> (StoreReads, usize) {
    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store.clone()).unwrap();
    for index in 0..operation_count {
        doc.set_peer_id(100 + (index % 4) as u64).unwrap();
        doc.get_text("text").insert(index, "x").unwrap();
        doc.commit();
    }
    let expected = doc.get_deep_value();
    let checkpoint = doc.flush_external_store().unwrap();
    drop(doc);

    store_control.reset_reads();
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    assert_eq!(reopened.get_deep_value(), expected);
    let reads = store_control.reads();
    let proof_bytes = assert_reopen_reads_only_compact_authenticated_metadata(
        &store_control,
        &reads,
        "alternating-peer reopen",
    );
    (reads, proof_bytes)
}

#[test]
fn alternating_peer_reopen_validation_is_bounded_by_distinct_blocks() {
    let (short, short_proof_bytes) = alternating_peer_reopen_case(64);
    let (long, long_proof_bytes) = alternating_peer_reopen_case(2048);
    assert!(
        long.ops <= 2 && short.ops <= 2,
        "alternating-peer reopen issued more than fixed metadata reads: short={short:?} long={long:?}"
    );
    assert!(
        long.bytes <= long_proof_bytes + 128 && short.bytes <= short_proof_bytes + 128,
        "alternating-peer reopen exceeded compact proof bytes"
    );
}

fn delete_heavy_reopen_case(delete_count: usize) -> (StoreReads, usize) {
    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store.clone()).unwrap();
    doc.set_peer_id(1).unwrap();
    doc.get_text("text")
        .insert(0, &"x".repeat(delete_count + 1))
        .unwrap();
    doc.commit();
    for index in 0..delete_count {
        doc.set_peer_id(200 + (index % 4) as u64).unwrap();
        doc.get_text("text").delete(0, 1).unwrap();
        doc.commit();
    }
    let expected = doc.get_deep_value();
    let checkpoint = doc.flush_external_store().unwrap();
    drop(doc);

    store_control.reset_reads();
    let reopened = LoroDoc::from_external_store(Some(&checkpoint), store).unwrap();
    assert_eq!(reopened.get_deep_value(), expected);
    let reads = store_control.reads();
    let proof_bytes = assert_reopen_reads_only_compact_authenticated_metadata(
        &store_control,
        &reads,
        "delete-heavy reopen",
    );
    (reads, proof_bytes)
}

#[test]
fn delete_heavy_reopen_validation_is_bounded_by_distinct_blocks() {
    let (short, short_proof_bytes) = delete_heavy_reopen_case(64);
    let (long, long_proof_bytes) = delete_heavy_reopen_case(1024);
    assert!(
        long.ops <= 2 && short.ops <= 2,
        "delete-heavy reopen issued more than fixed metadata reads: short={short:?} long={long:?}"
    );
    assert!(
        long.bytes <= long_proof_bytes + 128 && short.bytes <= short_proof_bytes + 128,
        "delete-heavy reopen exceeded compact proof bytes"
    );
}

#[test]
fn single_copy_metadata_and_structural_size_smoke_ceiling() {
    let (store_control, store) = FaultStore::new();
    let doc = LoroDoc::from_external_store(None, store).unwrap();
    doc.set_peer_id(1).unwrap();
    let text = doc.get_text("text");
    for _ in 0..1024 {
        text.insert(0, "x").unwrap();
        doc.commit();
    }
    let front_insert_checkpoint = doc.flush_external_store().unwrap();
    let front_insert_xm = store_control.raw_get(b"xm").unwrap();
    let front_insert_parts = decode_xm(&front_insert_xm);
    assert!(store_control.raw_get(b"ci").is_none());
    assert!(store_control.raw_get(b"ib").is_none());
    assert_eq!(
        front_insert_checkpoint
            .windows(b"LOROCI02".len())
            .filter(|window| *window == b"LOROCI02")
            .count(),
        0
    );
    assert_eq!(
        front_insert_checkpoint
            .windows(b"LOROIB03".len())
            .filter(|window| *window == b"LOROIB03")
            .count(),
        0
    );
    assert_eq!(
        front_insert_xm
            .windows(b"LOROCI02".len())
            .filter(|window| *window == b"LOROCI02")
            .count(),
        1
    );
    assert_eq!(
        front_insert_xm
            .windows(b"LOROIB03".len())
            .filter(|window| *window == b"LOROIB03")
            .count(),
        1
    );
    assert!(
        front_insert_parts.baseline.len() <= 1024 * 96 + 16 * 1024,
        "text baseline exceeded the structural-metadata smoke ceiling: {} bytes",
        front_insert_parts.baseline.len()
    );

    for _ in 0..1024 {
        text.insert(0, "y").unwrap();
        text.delete(0, 1).unwrap();
        doc.commit();
    }
    let edit_delete_checkpoint = doc.flush_external_store().unwrap();
    let edit_delete_xm = store_control.raw_get(b"xm").unwrap();
    let edit_delete_parts = decode_xm(&edit_delete_xm);
    assert!(
        edit_delete_parts.baseline.len() <= 2048 * 96 + 16 * 1024,
        "text tombstone metadata exceeded the structural-metadata smoke ceiling: {} bytes",
        edit_delete_parts.baseline.len()
    );
    assert!(
        edit_delete_checkpoint.len() < edit_delete_xm.len() + text.to_string().len() + 64 * 1024,
        "checkpoint unexpectedly contains a second causal/baseline copy"
    );
}
