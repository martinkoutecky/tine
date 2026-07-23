use bytes::Bytes;
use loro_common::{ContainerID, Counter, LoroError, LoroResult, PeerID, ID};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    change::Lamport,
    version::{Frontiers, ImVersionVector, VersionVector},
};

const CHECKPOINT_MAGIC: &[u8; 8] = b"LOROXS05";
const CAUSAL_MAGIC: &[u8; 8] = b"LOROCI02";
const IMPORT_BASELINE_MAGIC: &[u8; 8] = b"LOROIB03";
const STORE_METADATA_MAGIC: &[u8; 8] = b"LOROXM03";

pub(crate) struct StateCheckpoint {
    pub frontiers: Frontiers,
    pub state: Bytes,
    pub metadata_digest: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct CausalDagSnapshot {
    pub vv: VersionVector,
    pub frontiers: Frontiers,
    pub nodes: Vec<CausalDagNode>,
}

#[derive(Clone, Debug)]
pub(crate) struct CausalDagNode {
    pub peer: PeerID,
    pub cnt: Counter,
    pub lamport: Lamport,
    pub deps: Frontiers,
    pub vv: ImVersionVector,
    pub has_succ: bool,
    pub len: usize,
    pub boundary_proof: Option<CausalBoundaryProof>,
}

#[derive(Clone, Debug)]
pub(crate) struct CausalBoundaryProof {
    pub parts: Vec<CausalBoundaryPart>,
    pub node_digest: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct CausalBoundaryPart {
    pub start: Counter,
    pub end: Counter,
    pub source_start: Counter,
    pub source_end: Counter,
    pub source_lamport: Lamport,
    pub source_deps: Frontiers,
    pub change_count: u32,
    pub last_change_start: Counter,
    pub boundary_digest: [u8; 32],
    pub anchor_metadata_digest: [u8; 32],
    pub source_digest: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedTextTrackerSnapshot {
    pub owner: Bytes,
    pub tracker: Bytes,
    pub anchor_metadata_digest: [u8; 32],
    pub digest: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct ImportBaselineSnapshot {
    pub vv: VersionVector,
    pub frontiers: Frontiers,
    pub text_containers: BTreeSet<u32>,
    pub text_trackers: BTreeMap<u32, AuthenticatedTextTrackerSnapshot>,
    pub encoded: Option<Bytes>,
}

#[derive(Clone)]
pub(crate) struct ExternalStoreMetadata {
    pub vv: Bytes,
    pub frontiers: Bytes,
    pub start_vv: Bytes,
    pub start_frontiers: Bytes,
    pub causal: Bytes,
    pub import_baseline: Bytes,
    pub greatest_timestamp: i64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct EncodedId {
    peer: PeerID,
    counter: Counter,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedCausalDag {
    vv: Vec<(PeerID, Counter)>,
    frontiers: Vec<EncodedId>,
    nodes: Vec<EncodedCausalDagNode>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedCausalDagNode {
    peer: PeerID,
    cnt: Counter,
    lamport: Lamport,
    deps: Vec<EncodedId>,
    vv: Vec<(PeerID, Counter)>,
    has_succ: bool,
    len: u32,
    boundary_proof: Option<EncodedCausalBoundaryProof>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedCausalBoundaryProof {
    parts: Vec<EncodedCausalBoundaryPart>,
    node_digest: [u8; 32],
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedCausalBoundaryPart {
    start: Counter,
    end: Counter,
    source_start: Counter,
    source_end: Counter,
    source_lamport: Lamport,
    source_deps: Vec<EncodedId>,
    change_count: u32,
    last_change_start: Counter,
    boundary_digest: [u8; 32],
    anchor_metadata_digest: [u8; 32],
    source_digest: [u8; 32],
}

pub(crate) fn encode_store_metadata(metadata: ExternalStoreMetadata) -> LoroResult<Bytes> {
    let mut bytes = Vec::with_capacity(
        STORE_METADATA_MAGIC.len()
            + 28
            + metadata
                .vv
                .len()
                .saturating_add(metadata.frontiers.len())
                .saturating_add(metadata.start_vv.len())
                .saturating_add(metadata.start_frontiers.len())
                .saturating_add(metadata.causal.len())
                .saturating_add(metadata.import_baseline.len()),
    );
    bytes.extend_from_slice(STORE_METADATA_MAGIC);
    push_blob(&mut bytes, &metadata.vv, "version vector")?;
    push_blob(&mut bytes, &metadata.frontiers, "frontiers")?;
    push_blob(&mut bytes, &metadata.start_vv, "start version vector")?;
    push_blob(&mut bytes, &metadata.start_frontiers, "start frontiers")?;
    push_blob(&mut bytes, &metadata.causal, "causal DAG")?;
    push_blob(&mut bytes, &metadata.import_baseline, "import baseline")?;
    bytes.extend_from_slice(&metadata.greatest_timestamp.to_le_bytes());
    Ok(bytes.into())
}

pub(crate) fn decode_store_metadata(bytes: &[u8]) -> LoroResult<(ExternalStoreMetadata, [u8; 32])> {
    if bytes.get(..STORE_METADATA_MAGIC.len()) != Some(STORE_METADATA_MAGIC) {
        return Err(causal_error("invalid store metadata magic"));
    }
    let mut offset = STORE_METADATA_MAGIC.len();
    let vv = read_blob(bytes, &mut offset)?;
    let frontiers = read_blob(bytes, &mut offset)?;
    let start_vv = read_blob(bytes, &mut offset)?;
    let start_frontiers = read_blob(bytes, &mut offset)?;
    let causal = read_blob(bytes, &mut offset)?;
    let import_baseline = read_blob(bytes, &mut offset)?;
    let timestamp = take(bytes, &mut offset, std::mem::size_of::<i64>())?;
    if offset != bytes.len() {
        return Err(causal_error("trailing store metadata bytes"));
    }
    if vv.is_empty() || frontiers.is_empty() || causal.is_empty() || import_baseline.is_empty() {
        return Err(causal_error("incomplete store metadata"));
    }
    Ok((
        ExternalStoreMetadata {
            vv: Bytes::copy_from_slice(vv),
            frontiers: Bytes::copy_from_slice(frontiers),
            start_vv: Bytes::copy_from_slice(start_vv),
            start_frontiers: Bytes::copy_from_slice(start_frontiers),
            causal: Bytes::copy_from_slice(causal),
            import_baseline: Bytes::copy_from_slice(import_baseline),
            greatest_timestamp: i64::from_le_bytes(timestamp.try_into().unwrap()),
        },
        metadata_digest(bytes),
    ))
}

pub(crate) fn encode_checkpoint(
    frontiers: &Frontiers,
    state: Bytes,
    metadata_digest: [u8; 32],
) -> LoroResult<Vec<u8>> {
    let frontiers = frontiers.encode();
    let frontiers_len =
        u32::try_from(frontiers.len()).map_err(|_| checkpoint_error("frontiers are too large"))?;
    let state_len =
        u32::try_from(state.len()).map_err(|_| checkpoint_error("state is too large"))?;
    let mut output = Vec::with_capacity(
        CHECKPOINT_MAGIC.len() + 40 + frontiers.len().saturating_add(state.len()),
    );
    output.extend_from_slice(CHECKPOINT_MAGIC);
    output.extend_from_slice(&metadata_digest);
    output.extend_from_slice(&frontiers_len.to_le_bytes());
    output.extend_from_slice(&frontiers);
    output.extend_from_slice(&state_len.to_le_bytes());
    output.extend_from_slice(&state);
    Ok(output)
}

pub(crate) fn decode_checkpoint(bytes: &[u8]) -> LoroResult<StateCheckpoint> {
    if bytes.get(..CHECKPOINT_MAGIC.len()) != Some(CHECKPOINT_MAGIC) {
        return Err(checkpoint_error("invalid magic"));
    }

    let mut offset = CHECKPOINT_MAGIC.len();
    let metadata_digest = take(bytes, &mut offset, 32)?
        .try_into()
        .map_err(|_| checkpoint_error("invalid metadata digest"))?;
    let frontiers_len = read_len(bytes, &mut offset)?;
    let frontiers_bytes = take(bytes, &mut offset, frontiers_len)?;
    let state_len = read_len(bytes, &mut offset)?;
    let state = take(bytes, &mut offset, state_len)?;
    if offset != bytes.len() {
        return Err(checkpoint_error("trailing bytes"));
    }

    let frontiers =
        Frontiers::decode(frontiers_bytes).map_err(|_| checkpoint_error("invalid frontiers"))?;

    Ok(StateCheckpoint {
        frontiers,
        state: Bytes::copy_from_slice(state),
        metadata_digest,
    })
}

pub(crate) fn encode_causal_snapshot(snapshot: CausalDagSnapshot) -> LoroResult<Bytes> {
    validate_causal_snapshot(&snapshot)?;
    let encoded = EncodedCausalDag {
        vv: encode_vv(snapshot.vv.iter()),
        frontiers: encode_frontiers(&snapshot.frontiers),
        nodes: snapshot
            .nodes
            .into_iter()
            .map(|node| {
                Ok(EncodedCausalDagNode {
                    peer: node.peer,
                    cnt: node.cnt,
                    lamport: node.lamport,
                    deps: encode_frontiers(&node.deps),
                    vv: encode_vv(node.vv.iter()),
                    has_succ: node.has_succ,
                    len: u32::try_from(node.len)
                        .map_err(|_| causal_error("node span is too large"))?,
                    boundary_proof: node.boundary_proof.map(|proof| EncodedCausalBoundaryProof {
                        parts: proof
                            .parts
                            .into_iter()
                            .map(|part| EncodedCausalBoundaryPart {
                                start: part.start,
                                end: part.end,
                                source_start: part.source_start,
                                source_end: part.source_end,
                                source_lamport: part.source_lamport,
                                source_deps: encode_frontiers(&part.source_deps),
                                change_count: part.change_count,
                                last_change_start: part.last_change_start,
                                boundary_digest: part.boundary_digest,
                                anchor_metadata_digest: part.anchor_metadata_digest,
                                source_digest: part.source_digest,
                            })
                            .collect(),
                        node_digest: proof.node_digest,
                    }),
                })
            })
            .collect::<LoroResult<Vec<_>>>()?,
    };
    let body = postcard::to_allocvec(&encoded)
        .map_err(|_| causal_error("failed to encode causal metadata"))?;
    let mut bytes = Vec::with_capacity(CAUSAL_MAGIC.len() + body.len());
    bytes.extend_from_slice(CAUSAL_MAGIC);
    bytes.extend_from_slice(&body);
    Ok(bytes.into())
}

pub(crate) fn decode_causal_snapshot(bytes: &[u8]) -> LoroResult<CausalDagSnapshot> {
    if bytes.get(..CAUSAL_MAGIC.len()) != Some(CAUSAL_MAGIC) {
        return Err(causal_error("invalid magic"));
    }

    let encoded: EncodedCausalDag = postcard::from_bytes(&bytes[CAUSAL_MAGIC.len()..])
        .map_err(|_| causal_error("invalid encoding"))?;
    let snapshot = CausalDagSnapshot {
        vv: decode_vv(&encoded.vv)?,
        frontiers: decode_frontiers(&encoded.frontiers)?,
        nodes: encoded
            .nodes
            .into_iter()
            .map(|node| {
                Ok(CausalDagNode {
                    peer: node.peer,
                    cnt: node.cnt,
                    lamport: node.lamport,
                    deps: decode_frontiers(&node.deps)?,
                    vv: ImVersionVector::from_vv(&decode_vv(&node.vv)?),
                    has_succ: node.has_succ,
                    len: node.len as usize,
                    boundary_proof: node
                        .boundary_proof
                        .map(|proof| {
                            Ok::<CausalBoundaryProof, LoroError>(CausalBoundaryProof {
                                parts: proof
                                    .parts
                                    .into_iter()
                                    .map(|part| {
                                        Ok(CausalBoundaryPart {
                                            start: part.start,
                                            end: part.end,
                                            source_start: part.source_start,
                                            source_end: part.source_end,
                                            source_lamport: part.source_lamport,
                                            source_deps: decode_frontiers(&part.source_deps)?,
                                            change_count: part.change_count,
                                            last_change_start: part.last_change_start,
                                            boundary_digest: part.boundary_digest,
                                            anchor_metadata_digest: part.anchor_metadata_digest,
                                            source_digest: part.source_digest,
                                        })
                                    })
                                    .collect::<LoroResult<Vec<_>>>()?,
                                node_digest: proof.node_digest,
                            })
                        })
                        .transpose()?,
                })
            })
            .collect::<LoroResult<Vec<_>>>()?,
    };
    validate_causal_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub(crate) fn encode_import_baseline(snapshot: &ImportBaselineSnapshot) -> LoroResult<Bytes> {
    let tracker_indices = snapshot
        .text_trackers
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    if snapshot.text_containers != tracker_indices {
        return Err(baseline_error("text tracker coverage is incomplete"));
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(IMPORT_BASELINE_MAGIC);
    push_vv(&mut bytes, &snapshot.vv)?;
    push_frontiers(&mut bytes, &snapshot.frontiers)?;
    let tracker_count = u32::try_from(snapshot.text_trackers.len())
        .map_err(|_| baseline_error("too many text trackers"))?;
    bytes.extend_from_slice(&tracker_count.to_le_bytes());
    for (index, tracker) in &snapshot.text_trackers {
        if tracker.owner.is_empty() || tracker.tracker.is_empty() {
            return Err(baseline_error("empty authenticated text tracker"));
        }
        bytes.extend_from_slice(&index.to_le_bytes());
        push_blob(&mut bytes, &tracker.owner, "text tracker owner")?;
        push_blob(&mut bytes, &tracker.tracker, "text tracker")?;
        bytes.extend_from_slice(&tracker.anchor_metadata_digest);
        bytes.extend_from_slice(&tracker.digest);
    }
    Ok(bytes.into())
}

pub(crate) fn decode_import_baseline(bytes: &[u8]) -> LoroResult<ImportBaselineSnapshot> {
    if bytes.get(..IMPORT_BASELINE_MAGIC.len()) != Some(IMPORT_BASELINE_MAGIC) {
        return Err(baseline_error("invalid magic"));
    }
    let mut offset = IMPORT_BASELINE_MAGIC.len();
    let vv_count = read_u32(bytes, &mut offset)? as usize;
    let mut vv_pairs = Vec::with_capacity(vv_count);
    for _ in 0..vv_count {
        vv_pairs.push((read_u64(bytes, &mut offset)?, read_i32(bytes, &mut offset)?));
    }
    let vv = decode_vv(&vv_pairs).map_err(|_| baseline_error("invalid version vector"))?;
    let frontier_count = read_u32(bytes, &mut offset)? as usize;
    let mut frontier_ids = Vec::with_capacity(frontier_count);
    for _ in 0..frontier_count {
        frontier_ids.push(EncodedId {
            peer: read_u64(bytes, &mut offset)?,
            counter: read_i32(bytes, &mut offset)?,
        });
    }
    let frontiers =
        decode_frontiers(&frontier_ids).map_err(|_| baseline_error("invalid frontiers"))?;
    let tracker_count = read_u32(bytes, &mut offset)? as usize;
    let mut text_trackers = BTreeMap::new();
    let mut previous = None;
    for _ in 0..tracker_count {
        let index = read_u32(bytes, &mut offset)?;
        let owner = read_blob(bytes, &mut offset)?;
        let tracker = read_blob(bytes, &mut offset)?;
        let anchor_metadata_digest: [u8; 32] = take(bytes, &mut offset, 32)?
            .try_into()
            .map_err(|_| baseline_error("invalid text tracker anchor"))?;
        let digest: [u8; 32] = take(bytes, &mut offset, 32)?
            .try_into()
            .map_err(|_| baseline_error("invalid text tracker commitment"))?;
        let authenticated = AuthenticatedTextTrackerSnapshot {
            owner: Bytes::copy_from_slice(owner),
            tracker: Bytes::copy_from_slice(tracker),
            anchor_metadata_digest,
            digest,
        };
        if previous.is_some_and(|previous| previous >= index) || text_trackers.contains_key(&index)
        {
            return Err(baseline_error("text tracker index is not canonical"));
        }
        if owner.is_empty()
            || ContainerID::try_from_bytes(owner)
                .map(|owner| owner.container_type() != crate::ContainerType::Text)
                .unwrap_or(true)
        {
            return Err(baseline_error("text tracker owner is invalid"));
        }
        if tracker.is_empty() {
            return Err(baseline_error("canonical text tracker snapshot is empty"));
        }
        if digest != text_tracker_digest(owner, tracker, anchor_metadata_digest) {
            return Err(baseline_error("text tracker commitment is invalid"));
        }
        text_trackers.insert(index, authenticated);
        previous = Some(index);
    }
    if offset != bytes.len() {
        return Err(baseline_error("trailing bytes"));
    }
    let text_containers = text_trackers.keys().copied().collect();
    Ok(ImportBaselineSnapshot {
        vv,
        frontiers,
        text_containers,
        text_trackers,
        encoded: Some(Bytes::copy_from_slice(bytes)),
    })
}

fn validate_causal_snapshot(snapshot: &CausalDagSnapshot) -> LoroResult<()> {
    let mut by_start = BTreeMap::new();
    let mut next_counter = BTreeMap::<PeerID, Counter>::new();
    let mut proof_presence = None;
    for (index, node) in snapshot.nodes.iter().enumerate() {
        if node.len == 0 {
            return Err(causal_error("empty DAG node"));
        }
        let len =
            Counter::try_from(node.len).map_err(|_| causal_error("node span is too large"))?;
        let end = node
            .cnt
            .checked_add(len)
            .ok_or_else(|| causal_error("node counter overflow"))?;
        if node.cnt < 0 || end <= node.cnt {
            return Err(causal_error("invalid node span"));
        }
        let has_proof = node.boundary_proof.is_some();
        if proof_presence
            .replace(has_proof)
            .is_some_and(|old| old != has_proof)
        {
            return Err(causal_error("DAG boundary proofs are incomplete"));
        }
        if let Some(proof) = &node.boundary_proof {
            validate_causal_boundary_proof(node, end, proof)?;
        }
        let expected = next_counter.entry(node.peer).or_insert(0);
        if node.cnt != *expected {
            return Err(causal_error("DAG node spans are not contiguous"));
        }
        *expected = end;
        if by_start
            .insert(ID::new(node.peer, node.cnt), index)
            .is_some()
        {
            return Err(causal_error("duplicate DAG node"));
        }
    }

    if next_counter.len() != snapshot.vv.len()
        || next_counter
            .iter()
            .any(|(peer, end)| snapshot.vv.get(peer).copied() != Some(*end))
    {
        return Err(causal_error(
            "DAG node spans do not cover the version vector",
        ));
    }

    let mut order = (0..snapshot.nodes.len()).collect::<Vec<_>>();
    order.sort_unstable_by_key(|index| {
        let node = &snapshot.nodes[*index];
        (node.lamport, node.peer, node.cnt)
    });
    let mut validated = BTreeSet::new();
    let mut cross_peer_successors = BTreeSet::new();
    for index in order {
        let node = &snapshot.nodes[index];
        let mut expected_vv = VersionVector::new();
        let mut expected_lamport = 0;
        for dep in node.deps.iter() {
            let dep_index = find_node(&snapshot.nodes, &by_start, dep)
                .ok_or_else(|| causal_error("dependency is outside the DAG"))?;
            if !validated.contains(&dep_index) {
                return Err(causal_error("dependency is not causally earlier"));
            }
            let dep_node = &snapshot.nodes[dep_index];
            if dep != node_last_id(dep_node)? {
                return Err(causal_error("dependency does not end a DAG node"));
            }
            expected_vv.extend_to_include_vv(dep_node.vv.iter());
            expected_vv.extend_to_include_last_id(dep);
            let dep_offset = u32::try_from(dep.counter - dep_node.cnt)
                .map_err(|_| causal_error("invalid dependency offset"))?;
            expected_lamport = expected_lamport.max(dep_node.lamport + dep_offset + 1);
            if dep.peer != node.peer {
                cross_peer_successors.insert(dep);
            }
        }
        if expected_vv != VersionVector::from_im_vv(&node.vv) {
            return Err(causal_error("precomputed version vector is invalid"));
        }
        if node.lamport != expected_lamport {
            return Err(causal_error("node lamport is invalid"));
        }
        validated.insert(index);
    }

    for node in &snapshot.nodes {
        if node.has_succ != cross_peer_successors.contains(&node_last_id(node)?) {
            return Err(causal_error("node successor flag is invalid"));
        }
    }

    let mut expected_frontiers = snapshot
        .nodes
        .iter()
        .map(node_last_id)
        .collect::<LoroResult<BTreeSet<_>>>()?;
    for node in &snapshot.nodes {
        for dep in node.deps.iter() {
            expected_frontiers.remove(&dep);
        }
    }
    let actual_frontiers = snapshot.frontiers.iter().collect::<BTreeSet<_>>();
    if actual_frontiers != expected_frontiers {
        return Err(causal_error("frontiers do not match DAG leaves"));
    }

    let mut frontier_vv = VersionVector::new();
    for frontier in snapshot.frontiers.iter() {
        let index = find_node(&snapshot.nodes, &by_start, frontier)
            .ok_or_else(|| causal_error("frontier is outside the DAG"))?;
        frontier_vv.extend_to_include_vv(snapshot.nodes[index].vv.iter());
        frontier_vv.extend_to_include_last_id(frontier);
    }
    if frontier_vv != snapshot.vv {
        return Err(causal_error("frontiers do not cover the version vector"));
    }
    Ok(())
}

fn validate_causal_boundary_proof(
    node: &CausalDagNode,
    node_end: Counter,
    proof: &CausalBoundaryProof,
) -> LoroResult<()> {
    if proof.parts.is_empty() || proof.node_digest != causal_node_digest(node, proof) {
        return Err(causal_error("invalid DAG boundary proof"));
    }

    let mut cursor = node.cnt;
    for part in &proof.parts {
        if part.start != cursor
            || part.end <= part.start
            || part.source_start < 0
            || part.source_start > part.start
            || part.source_end < part.end
            || part.source_end <= part.source_start
            || part.change_count == 0
            || part.last_change_start < 0
            || part.last_change_start >= part.source_end
            || part.source_digest != causal_source_digest(node.peer, part)
        {
            return Err(causal_error("invalid DAG boundary proof part"));
        }

        let source_offset = u32::try_from(part.start - part.source_start)
            .map_err(|_| causal_error("invalid DAG boundary proof offset"))?;
        let node_offset = u32::try_from(part.start - node.cnt)
            .map_err(|_| causal_error("invalid DAG boundary proof offset"))?;
        if part.source_lamport + source_offset != node.lamport + node_offset {
            return Err(causal_error("DAG boundary proof lamport is inconsistent"));
        }
        let source_deps = if part.start == part.source_start {
            part.source_deps.clone()
        } else {
            Frontiers::from_id(ID::new(node.peer, part.start - 1))
        };
        let node_deps = if part.start == node.cnt {
            node.deps.clone()
        } else {
            Frontiers::from_id(ID::new(node.peer, part.start - 1))
        };
        if source_deps != node_deps {
            return Err(causal_error(
                "DAG boundary proof crosses a nonlinear source boundary",
            ));
        }
        cursor = part.end;
    }
    if cursor != node_end {
        return Err(causal_error("DAG boundary proof does not cover its node"));
    }
    Ok(())
}

pub(crate) fn causal_source_digest(peer: PeerID, part: &CausalBoundaryPart) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"LORO causal authenticated source v1");
    digest.update(peer.to_le_bytes());
    digest.update(part.source_start.to_le_bytes());
    digest.update(part.source_end.to_le_bytes());
    digest.update(part.source_lamport.to_le_bytes());
    update_frontiers_digest(&mut digest, &part.source_deps);
    digest.update(part.change_count.to_le_bytes());
    digest.update(part.last_change_start.to_le_bytes());
    digest.update(part.boundary_digest);
    digest.update(part.anchor_metadata_digest);
    digest.finalize().into()
}

pub(crate) fn causal_node_digest(node: &CausalDagNode, proof: &CausalBoundaryProof) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"LORO causal span commitment v2");
    digest.update(node.peer.to_le_bytes());
    digest.update(node.cnt.to_le_bytes());
    digest.update(node.lamport.to_le_bytes());
    digest.update((node.len as u64).to_le_bytes());
    digest.update([node.has_succ as u8]);
    update_frontiers_digest(&mut digest, &node.deps);
    digest.update((node.vv.iter().count() as u64).to_le_bytes());
    for (peer, counter) in node.vv.iter() {
        digest.update(peer.to_le_bytes());
        digest.update(counter.to_le_bytes());
    }
    digest.update((proof.parts.len() as u64).to_le_bytes());
    for part in &proof.parts {
        digest.update(part.start.to_le_bytes());
        digest.update(part.end.to_le_bytes());
        digest.update(part.source_digest);
    }
    digest.finalize().into()
}

pub(crate) fn text_tracker_digest(
    owner: &[u8],
    tracker: &[u8],
    anchor_metadata_digest: [u8; 32],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"LORO canonical text tracker v1");
    digest.update((owner.len() as u64).to_le_bytes());
    digest.update(owner);
    digest.update((tracker.len() as u64).to_le_bytes());
    digest.update(tracker);
    digest.update(anchor_metadata_digest);
    digest.finalize().into()
}

fn update_frontiers_digest(digest: &mut Sha256, frontiers: &Frontiers) {
    digest.update((frontiers.len() as u64).to_le_bytes());
    for id in frontiers.iter() {
        digest.update(id.peer.to_le_bytes());
        digest.update(id.counter.to_le_bytes());
    }
}

fn find_node(nodes: &[CausalDagNode], by_start: &BTreeMap<ID, usize>, id: ID) -> Option<usize> {
    let (_, index) = by_start.range(..=id).next_back()?;
    let node = &nodes[*index];
    (node.peer == id.peer && node.cnt <= id.counter && node_last_id(node).ok()? >= id)
        .then_some(*index)
}

fn node_last_id(node: &CausalDagNode) -> LoroResult<ID> {
    let len = Counter::try_from(node.len).map_err(|_| causal_error("node span is too large"))?;
    let offset = len
        .checked_sub(1)
        .ok_or_else(|| causal_error("empty DAG node"))?;
    let counter = node
        .cnt
        .checked_add(offset)
        .ok_or_else(|| causal_error("node counter overflow"))?;
    Ok(ID::new(node.peer, counter))
}

fn encode_vv<'a>(vv: impl Iterator<Item = (&'a PeerID, &'a Counter)>) -> Vec<(PeerID, Counter)> {
    let mut pairs = vv
        .filter_map(|(peer, counter)| (*counter > 0).then_some((*peer, *counter)))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    pairs
}

fn decode_vv(pairs: &[(PeerID, Counter)]) -> LoroResult<VersionVector> {
    if pairs.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        || pairs.iter().any(|(_, counter)| *counter <= 0)
    {
        return Err(causal_error("version vector is not canonical"));
    }
    Ok(VersionVector::from_iter(pairs.iter().copied()))
}

fn encode_frontiers(frontiers: &Frontiers) -> Vec<EncodedId> {
    let mut ids = frontiers
        .iter()
        .map(|id| EncodedId {
            peer: id.peer,
            counter: id.counter,
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn decode_frontiers(ids: &[EncodedId]) -> LoroResult<Frontiers> {
    if ids.windows(2).any(|pair| pair[0] >= pair[1]) || ids.iter().any(|id| id.counter < 0) {
        return Err(causal_error("frontiers are not canonical"));
    }
    Ok(ids.iter().map(|id| ID::new(id.peer, id.counter)).collect())
}

fn read_len(bytes: &[u8], offset: &mut usize) -> LoroResult<usize> {
    Ok(read_u32(bytes, offset)? as usize)
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> LoroResult<u32> {
    let raw = take(bytes, offset, 4)?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> LoroResult<u64> {
    let raw = take(bytes, offset, 8)?;
    Ok(u64::from_le_bytes(raw.try_into().unwrap()))
}

fn read_i32(bytes: &[u8], offset: &mut usize) -> LoroResult<i32> {
    let raw = take(bytes, offset, 4)?;
    Ok(i32::from_le_bytes(raw.try_into().unwrap()))
}

fn read_blob<'a>(bytes: &'a [u8], offset: &mut usize) -> LoroResult<&'a [u8]> {
    let len = read_len(bytes, offset)?;
    take(bytes, offset, len)
}

fn push_vv(output: &mut Vec<u8>, vv: &VersionVector) -> LoroResult<()> {
    let pairs = encode_vv(vv.iter());
    let count =
        u32::try_from(pairs.len()).map_err(|_| baseline_error("version vector is too large"))?;
    output.extend_from_slice(&count.to_le_bytes());
    for (peer, counter) in pairs {
        output.extend_from_slice(&peer.to_le_bytes());
        output.extend_from_slice(&counter.to_le_bytes());
    }
    Ok(())
}

fn push_frontiers(output: &mut Vec<u8>, frontiers: &Frontiers) -> LoroResult<()> {
    let ids = encode_frontiers(frontiers);
    let count = u32::try_from(ids.len()).map_err(|_| baseline_error("frontiers are too large"))?;
    output.extend_from_slice(&count.to_le_bytes());
    for id in ids {
        output.extend_from_slice(&id.peer.to_le_bytes());
        output.extend_from_slice(&id.counter.to_le_bytes());
    }
    Ok(())
}

fn push_blob(output: &mut Vec<u8>, bytes: &[u8], name: &str) -> LoroResult<()> {
    let len =
        u32::try_from(bytes.len()).map_err(|_| causal_error(&format!("{name} is too large")))?;
    output.extend_from_slice(&len.to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn take<'a>(bytes: &'a [u8], offset: &mut usize, len: usize) -> LoroResult<&'a [u8]> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| checkpoint_error("length overflow"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| checkpoint_error("truncated data"))?;
    *offset = end;
    Ok(value)
}

#[doc(hidden)]
pub fn metadata_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn checkpoint_error(message: &str) -> LoroError {
    LoroError::DecodeError(format!("external change-store checkpoint: {message}").into_boxed_str())
}

fn causal_error(message: &str) -> LoroError {
    LoroError::DecodeError(
        format!("external change-store causal metadata: {message}").into_boxed_str(),
    )
}

fn baseline_error(message: &str) -> LoroError {
    LoroError::DecodeError(
        format!("external change-store import baseline: {message}").into_boxed_str(),
    )
}
