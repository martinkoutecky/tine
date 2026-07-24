use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tine_core::oplog::{
    AnnotatedProjectionBase, BatchCausalDot, BatchError, BatchId, BatchInspection, BatchOrigin,
    CausalPeerId, ContentDigest, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies,
    DocumentId, FrontierV2, LineageDigest, ManagedPath, ManifestObjectRef,
    ManifestProjectionPrecondition, ManifestProjectionTarget, ManifestedProjectionIntent,
    ObjectDescriptor, ObjectKind, ObjectStore, OperationBatch, OperationObject, PreparedBatch,
    ProjectionEndpointId, SemanticEffectDigest, SessionId, StoreError, WorkspaceId,
    MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
};
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-oplog-{label}-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
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

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn workspace(value: u128) -> WorkspaceId {
    WorkspaceId::from_uuid(uuid(value))
}

fn document(value: u128) -> DocumentId {
    DocumentId::from_uuid(uuid(value))
}

fn batch(value: u128) -> BatchId {
    BatchId::from_uuid(uuid(value))
}

fn object(
    workspace_id: WorkspaceId,
    document_id: DocumentId,
    kind: ObjectKind,
    payload: &[u8],
) -> OperationObject {
    OperationObject::new(workspace_id, document_id, kind, payload.to_vec()).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn test_manifest(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    batch_id: BatchId,
    author_device_id: DeviceId,
    author_session_id: SessionId,
    dependency_frontier: FrontierV2,
    semantic_effect_digest: SemanticEffectDigest,
    required_objects: Vec<ObjectDescriptor>,
) -> Result<OperationBatch, BatchError> {
    let causal_dependency_heads = dependency_frontier
        .documents()
        .iter()
        .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
        .collect();
    OperationBatch::new_with_causality(
        workspace_id,
        lineage_digest,
        batch_id,
        author_device_id,
        author_session_id,
        BatchOrigin::BootstrapImport,
        BatchCausalDot::new(CausalPeerId::from_device_id(author_device_id), 1).unwrap(),
        causal_dependency_heads,
        dependency_frontier,
        semantic_effect_digest,
        required_objects,
    )
}

fn manifest(
    workspace_id: WorkspaceId,
    batch_id: BatchId,
    semantic_payload: &[u8],
    descriptors: Vec<ObjectDescriptor>,
) -> OperationBatch {
    let frontier = FrontierV2::new(vec![DocumentDependencies::new(
        document(20),
        vec![
            CrdtPeerCounter::new(CrdtPeerId::from_u64(8), 12),
            CrdtPeerCounter::new(CrdtPeerId::from_u64(2), 9),
        ],
        vec![batch(300), batch(200)],
    )
    .unwrap()])
    .unwrap();
    test_manifest(
        workspace_id,
        LineageDigest::of(b"immutable-lineage"),
        batch_id,
        DeviceId::from_uuid(uuid(30)),
        SessionId::from_uuid(uuid(31)),
        frontier,
        SemanticEffectDigest::of(semantic_payload),
        descriptors,
    )
    .unwrap()
}

fn sample(workspace_id: WorkspaceId, batch_id: BatchId, semantic_payload: &[u8]) -> PreparedBatch {
    let semantic = object(
        workspace_id,
        document(10),
        ObjectKind::SemanticEffect,
        semantic_payload,
    );
    let update = object(
        workspace_id,
        document(20),
        ObjectKind::CrdtUpdate,
        b"crdt update bytes",
    );
    let endpoint_id = ProjectionEndpointId::from_uuid(uuid(32));
    let page_id = tine_core::oplog::PageId::from_uuid(uuid(33));
    let managed_path = ManagedPath::parse("pages/sample.md").unwrap();
    let empty_frontier = FrontierV2::new(Vec::new()).unwrap();
    let base = AnnotatedProjectionBase::new(
        workspace_id,
        endpoint_id,
        page_id,
        managed_path.clone(),
        None,
        empty_frontier.clone(),
        b"large base bytes stay binary".to_vec(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let base = OperationObject::new(
        workspace_id,
        base.descriptor_document_id().unwrap(),
        ObjectKind::AnnotatedBaseBlob,
        base.encode().unwrap(),
    )
    .unwrap();
    let intent = ManifestedProjectionIntent::new(
        workspace_id,
        batch_id,
        DeviceId::from_uuid(uuid(30)),
        SessionId::from_uuid(uuid(31)),
        endpoint_id,
        page_id,
        managed_path,
        tine_core::oplog::PortablePathIndexRoot::empty(),
        ManifestProjectionPrecondition::Present {
            base: ManifestObjectRef::from_descriptor(&base.descriptor().unwrap()),
        },
        None,
        ManifestProjectionTarget::present(b"target".to_vec(), Vec::new()).unwrap(),
        empty_frontier,
        Vec::new(),
    )
    .unwrap();
    let intent = OperationObject::new(
        workspace_id,
        intent.descriptor_document_id(),
        ObjectKind::ProjectionIntent,
        intent.encode().unwrap(),
    )
    .unwrap();
    let objects = vec![intent, semantic, base, update];
    let descriptors = objects
        .iter()
        .rev()
        .map(|object| object.descriptor().unwrap())
        .collect();
    let manifest = manifest(workspace_id, batch_id, semantic_payload, descriptors);
    PreparedBatch::new(manifest, objects).unwrap()
}

fn open_store(dir: &TestDir, workspace_id: WorkspaceId) -> ObjectStore {
    ObjectStore::open(&dir.path().join("candidate-v2-store"), workspace_id).unwrap()
}

fn rewrite_checksum(bytes: &mut [u8]) {
    let body_len = bytes.len() - 32;
    let checksum = Sha256::digest(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(&checksum);
}

fn rewrite_object_header(bytes: &[u8], update: impl FnOnce(&mut Value)) -> Vec<u8> {
    let old_header_len = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let payload_len = u64::from_be_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let mut header: Value = serde_json::from_slice(&bytes[20..20 + old_header_len]).unwrap();
    update(&mut header);
    let header = serde_json::to_vec(&header).unwrap();
    let payload = &bytes[20 + old_header_len..20 + old_header_len + payload_len];
    let mut rewritten = Vec::new();
    rewritten.extend_from_slice(&bytes[..8]);
    rewritten.extend_from_slice(&(header.len() as u32).to_be_bytes());
    rewritten.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    rewritten.extend_from_slice(&header);
    rewritten.extend_from_slice(payload);
    let checksum = Sha256::digest(&rewritten);
    rewritten.extend_from_slice(&checksum);
    rewritten
}

fn authoritative_file_count(root: &Path) -> usize {
    ["objects", "batches"]
        .into_iter()
        .map(|directory| fs::read_dir(root.join(directory)).unwrap().count())
        .sum()
}

#[test]
fn canonical_roundtrip_and_deterministic_bytes() {
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(40), b"canonical semantic effect");
    let manifest_bytes = prepared.manifest().encode().unwrap();
    assert_eq!(
        OperationBatch::decode(&manifest_bytes).unwrap(),
        *prepared.manifest()
    );
    assert_eq!(
        OperationBatch::decode(&manifest_bytes)
            .unwrap()
            .encode()
            .unwrap(),
        manifest_bytes
    );

    for operation_object in prepared.objects() {
        let first = operation_object.encode().unwrap();
        let second = operation_object.encode().unwrap();
        assert_eq!(first, second);
        assert_eq!(OperationObject::decode(&first).unwrap(), *operation_object);
        assert!(!operation_object.payload().is_empty());
    }

    let ordered = prepared.manifest().required_objects().to_vec();
    let equivalent = manifest(
        workspace_id,
        batch(40),
        b"canonical semantic effect",
        ordered,
    );
    assert_eq!(equivalent.encode().unwrap(), manifest_bytes);
}

#[test]
fn manifest_constructor_canonicalizes_and_rejects_generic_invariant_violations() {
    let workspace_id = workspace(1);
    let semantic = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"semantic",
    );
    let update_a = object(
        workspace_id,
        document(2),
        ObjectKind::CrdtUpdate,
        b"update a",
    );
    let update_b = object(
        workspace_id,
        document(2),
        ObjectKind::CrdtUpdate,
        b"update b",
    );
    let semantic_descriptor = semantic.descriptor().unwrap();
    let update_a_descriptor = update_a.descriptor().unwrap();
    let update_b_descriptor = update_b.descriptor().unwrap();

    let canonical = manifest(
        workspace_id,
        batch(1),
        b"semantic",
        vec![update_a_descriptor.clone(), semantic_descriptor.clone()],
    );
    assert!(canonical
        .required_objects()
        .windows(2)
        .all(|pair| pair[0] < pair[1]));

    let duplicate = test_manifest(
        workspace_id,
        LineageDigest::of(b"lineage"),
        batch(2),
        DeviceId::new(),
        SessionId::new(),
        FrontierV2::default(),
        SemanticEffectDigest::of(b"semantic"),
        vec![semantic_descriptor.clone(), semantic_descriptor.clone()],
    )
    .unwrap_err();
    assert!(matches!(duplicate, BatchError::DuplicateDescriptor(_)));

    let duplicate_update = test_manifest(
        workspace_id,
        LineageDigest::of(b"lineage"),
        batch(3),
        DeviceId::new(),
        SessionId::new(),
        FrontierV2::default(),
        SemanticEffectDigest::of(b"semantic"),
        vec![
            semantic_descriptor.clone(),
            update_a_descriptor.clone(),
            update_b_descriptor,
        ],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_update,
        BatchError::DuplicateCrdtDocument(_)
    ));

    let no_semantic = test_manifest(
        workspace_id,
        LineageDigest::of(b"lineage"),
        batch(4),
        DeviceId::new(),
        SessionId::new(),
        FrontierV2::default(),
        SemanticEffectDigest::of(b"none"),
        vec![update_a_descriptor],
    )
    .unwrap_err();
    assert_eq!(no_semantic, BatchError::SemanticEffectCardinality(0));

    let second_semantic = object(
        workspace_id,
        document(3),
        ObjectKind::SemanticEffect,
        b"other semantic",
    );
    let two_semantics = test_manifest(
        workspace_id,
        LineageDigest::of(b"lineage"),
        batch(5),
        DeviceId::new(),
        SessionId::new(),
        FrontierV2::default(),
        SemanticEffectDigest::of(b"semantic"),
        vec![semantic_descriptor, second_semantic.descriptor().unwrap()],
    )
    .unwrap_err();
    assert_eq!(two_semantics, BatchError::SemanticEffectCardinality(2));

    assert_eq!(
        ObjectDescriptor::new(
            document(1),
            ObjectKind::ProjectionIntent,
            ContentDigest::of(b"object"),
            0,
        )
        .unwrap_err(),
        BatchError::InvalidObjectLength(0)
    );
}

#[test]
fn semantic_effect_payload_is_cryptographically_bound_to_manifest() {
    let workspace_id = workspace(1);
    let semantic = object(
        workspace_id,
        document(10),
        ObjectKind::SemanticEffect,
        b"actual semantic effect",
    );
    let manifest = manifest(
        workspace_id,
        batch(90),
        b"different declared semantic effect",
        vec![semantic.descriptor().unwrap()],
    );
    let error = PreparedBatch::new(manifest.clone(), vec![semantic.clone()]).unwrap_err();
    assert!(matches!(
        error,
        BatchError::SemanticEffectDigestMismatch { expected, actual }
            if expected == manifest.semantic_effect_digest()
                && actual == SemanticEffectDigest::of(semantic.payload())
    ));

    let dir = TestDir::new("semantic-digest-mismatch");
    let store = open_store(&dir, workspace_id);
    store
        .stage_object_bytes(&semantic.encode().unwrap())
        .unwrap();
    store
        .stage_manifest_bytes(&manifest.encode().unwrap())
        .unwrap();
    assert!(matches!(
        store.inspect_batch(manifest.batch_id()),
        Err(StoreError::Batch(
            BatchError::SemanticEffectDigestMismatch { .. }
        ))
    ));
}

#[test]
fn objects_first_then_manifest_becomes_ready() {
    let dir = TestDir::new("objects-first");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(1), b"semantic");
    let store = open_store(&dir, workspace_id);

    for operation_object in prepared.objects().iter().rev() {
        store
            .stage_object_bytes(&operation_object.encode().unwrap())
            .unwrap();
    }
    assert_eq!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Absent
    );
    store
        .stage_manifest_bytes(&prepared.manifest().encode().unwrap())
        .unwrap();
    assert!(matches!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Ready(_)
    ));
}

#[test]
fn manifest_first_stages_canonical_missing_list_then_reordered_objects_make_it_ready() {
    let dir = TestDir::new("manifest-first");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(2), b"semantic");
    let store = open_store(&dir, workspace_id);
    store
        .stage_manifest_bytes(&prepared.manifest().encode().unwrap())
        .unwrap();

    match store.inspect_batch(prepared.manifest().batch_id()).unwrap() {
        BatchInspection::Staged { missing, .. } => {
            assert_eq!(missing, prepared.manifest().required_objects());
        }
        other => panic!("expected staged batch, found {other:?}"),
    }

    for operation_object in prepared.objects().iter().rev() {
        store
            .stage_object_bytes(&operation_object.encode().unwrap())
            .unwrap();
        let inspection = store.inspect_batch(prepared.manifest().batch_id()).unwrap();
        if let BatchInspection::Staged { missing, .. } = inspection {
            assert!(missing.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
    assert!(matches!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Ready(_)
    ));
}

#[test]
fn object_only_crash_residue_is_invisible_and_retained_across_reopen() {
    let dir = TestDir::new("object-residue");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(3), b"semantic");
    let digest = {
        let store = open_store(&dir, workspace_id);
        let digest = store
            .stage_object_bytes(&prepared.objects()[0].encode().unwrap())
            .unwrap();
        assert!(store.committed_manifests().unwrap().is_empty());
        assert_eq!(
            store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
            BatchInspection::Absent
        );
        digest
    };

    let reopened = open_store(&dir, workspace_id);
    assert!(reopened.contains_object(digest).unwrap());
    assert!(reopened.committed_manifests().unwrap().is_empty());
}

#[test]
fn duplicate_delivery_is_idempotent_and_batch_collision_is_fatal() {
    let dir = TestDir::new("duplicates");
    let workspace_id = workspace(1);
    let first = sample(workspace_id, batch(4), b"first semantic");
    let store = open_store(&dir, workspace_id);
    store.publish_prepared(&first).unwrap();
    let count = authoritative_file_count(store.root_path());
    store.publish_prepared(&first).unwrap();
    assert_eq!(authoritative_file_count(store.root_path()), count);

    let conflicting = sample(workspace_id, batch(4), b"different semantic");
    let error = store
        .stage_manifest_bytes(&conflicting.manifest().encode().unwrap())
        .unwrap_err();
    assert!(matches!(error, StoreError::BatchCollision(id) if id == batch(4)));
    assert!(matches!(
        store.inspect_batch(batch(4)).unwrap(),
        BatchInspection::Ready(_)
    ));

    let foreign_lineage = test_manifest(
        workspace_id,
        LineageDigest::of(b"independent lineage"),
        batch(5),
        first.manifest().author_device_id(),
        first.manifest().author_session_id(),
        first.manifest().dependency_frontier().clone(),
        first.manifest().semantic_effect_digest(),
        first.manifest().required_objects().to_vec(),
    )
    .unwrap();
    assert!(matches!(
        store.stage_manifest_bytes(&foreign_lineage.encode().unwrap()),
        Err(StoreError::LineageMismatch { .. })
    ));
}

#[test]
fn object_decode_and_store_reject_corruption_workspace_and_descriptor_mismatch() {
    let workspace_id = workspace(1);
    let valid = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"semantic",
    )
    .encode()
    .unwrap();
    assert!(matches!(
        OperationObject::decode(&valid[..valid.len() - 1]),
        Err(BatchError::ObjectLengthMismatch { .. })
    ));
    let mut corrupt = valid.clone();
    *corrupt.last_mut().unwrap() ^= 0x80;
    assert_eq!(
        OperationObject::decode(&corrupt).unwrap_err(),
        BatchError::ChecksumMismatch
    );
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(matches!(
        OperationObject::decode(&trailing),
        Err(BatchError::ObjectLengthMismatch { .. })
    ));

    let foreign = object(
        workspace(2),
        document(1),
        ObjectKind::SemanticEffect,
        b"semantic",
    );
    let dir = TestDir::new("wrong-workspace");
    let store = open_store(&dir, workspace_id);
    assert!(matches!(
        store.stage_object_bytes(&foreign.encode().unwrap()),
        Err(StoreError::WorkspaceMismatch { .. })
    ));

    for (label, replacement) in [
        (
            "wrong-kind",
            Box::new(|descriptor: &ObjectDescriptor| {
                ObjectDescriptor::new(
                    descriptor.document_id(),
                    ObjectKind::ProjectionIntent,
                    descriptor.content_digest(),
                    descriptor.encoded_byte_length(),
                )
                .unwrap()
            }) as Box<dyn Fn(&ObjectDescriptor) -> ObjectDescriptor>,
        ),
        (
            "wrong-document",
            Box::new(|descriptor: &ObjectDescriptor| {
                ObjectDescriptor::new(
                    document(999),
                    descriptor.kind(),
                    descriptor.content_digest(),
                    descriptor.encoded_byte_length(),
                )
                .unwrap()
            }),
        ),
    ] {
        let dir = TestDir::new(label);
        let prepared = sample(workspace_id, BatchId::new(), b"semantic");
        let changed = prepared
            .manifest()
            .required_objects()
            .iter()
            .map(|descriptor| {
                if descriptor.kind() == ObjectKind::CrdtUpdate {
                    replacement(descriptor)
                } else {
                    descriptor.clone()
                }
            })
            .collect();
        let malformed_manifest = manifest(
            workspace_id,
            prepared.manifest().batch_id(),
            b"semantic",
            changed,
        );
        let store = open_store(&dir, workspace_id);
        store
            .stage_manifest_bytes(&malformed_manifest.encode().unwrap())
            .unwrap();
        for operation_object in prepared.objects() {
            store
                .stage_object_bytes(&operation_object.encode().unwrap())
                .unwrap();
        }
        assert!(matches!(
            store.inspect_batch(malformed_manifest.batch_id()),
            Err(StoreError::Batch(BatchError::DescriptorMismatch { .. }))
        ));
    }
}

#[test]
fn unknown_versions_fields_digest_forms_and_canonical_order_fail_closed() {
    let prepared = sample(workspace(1), batch(6), b"semantic");
    let encoded = prepared.manifest().encode().unwrap();
    let current: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(current["manifest_encoding_version"], json!(4));
    assert_eq!(current["protocol_version"], json!(2));
    assert_eq!(current["operation_schema_version"], json!(7));
    assert_eq!(current["object_envelope_schema_version"], json!(2));
    assert_eq!(current["managed_entity_set_version"], json!(2));

    for field in [
        "manifest_encoding_version",
        "protocol_version",
        "operation_schema_version",
        "object_envelope_schema_version",
        "managed_entity_set_version",
    ] {
        let mut value: Value = serde_json::from_slice(&encoded).unwrap();
        value[field] = json!(99);
        assert!(matches!(
            OperationBatch::decode(&serde_json::to_vec(&value).unwrap()),
            Err(BatchError::UnknownVersion { .. })
        ));
    }
    for version in [1, 3] {
        let mut value: Value = serde_json::from_slice(&encoded).unwrap();
        value["object_envelope_schema_version"] = json!(version);
        assert!(matches!(
            OperationBatch::decode(&serde_json::to_vec(&value).unwrap()),
            Err(BatchError::UnknownVersion {
                field: "object_envelope_schema_version",
                ..
            })
        ));
    }

    let mut unknown_field: Value = serde_json::from_slice(&encoded).unwrap();
    unknown_field["future_field"] = json!(true);
    assert!(matches!(
        OperationBatch::decode(&serde_json::to_vec(&unknown_field).unwrap()),
        Err(BatchError::Decode(_))
    ));

    let mut noncanonical: Value = serde_json::from_slice(&encoded).unwrap();
    noncanonical["required_objects"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(
        OperationBatch::decode(&serde_json::to_vec(&noncanonical).unwrap()).unwrap_err(),
        BatchError::NonCanonicalDescriptors
    );

    let mut uppercase_digest: Value = serde_json::from_slice(&encoded).unwrap();
    uppercase_digest["lineage_digest"] = json!(uppercase_digest["lineage_digest"]
        .as_str()
        .unwrap()
        .to_ascii_uppercase());
    assert!(matches!(
        OperationBatch::decode(&serde_json::to_vec(&uppercase_digest).unwrap()),
        Err(BatchError::Decode(_))
    ));

    let object = prepared.objects()[0].encode().unwrap();
    for version in [1, 3] {
        let incompatible = rewrite_object_header(&object, |header| {
            header["envelope_schema_version"] = json!(version);
        });
        assert!(matches!(
            OperationObject::decode(&incompatible),
            Err(BatchError::UnknownVersion {
                field: "object_envelope_schema_version",
                ..
            })
        ));
    }
    let unknown_header = rewrite_object_header(&object, |header| {
        header["future_field"] = json!(true);
    });
    assert!(matches!(
        OperationObject::decode(&unknown_header),
        Err(BatchError::Decode(_))
    ));

    let header_len = u32::from_be_bytes(object[8..12].try_into().unwrap()) as usize;
    let payload_len = u64::from_be_bytes(object[12..20].try_into().unwrap()) as usize;
    let header: Value = serde_json::from_slice(&object[20..20 + header_len]).unwrap();
    let noncanonical_header = serde_json::to_vec_pretty(&header).unwrap();
    let mut noncanonical_object = Vec::new();
    noncanonical_object.extend_from_slice(&object[..8]);
    noncanonical_object.extend_from_slice(&(noncanonical_header.len() as u32).to_be_bytes());
    noncanonical_object.extend_from_slice(&(payload_len as u64).to_be_bytes());
    noncanonical_object.extend_from_slice(&noncanonical_header);
    noncanonical_object.extend_from_slice(&object[20 + header_len..20 + header_len + payload_len]);
    let checksum = Sha256::digest(&noncanonical_object);
    noncanonical_object.extend_from_slice(&checksum);
    assert_eq!(
        OperationObject::decode(&noncanonical_object).unwrap_err(),
        BatchError::NonCanonicalObjectHeader
    );
}

#[test]
fn projection_closed_set_rejects_malformed_missing_and_orphan_objects_before_ready() {
    let prepared = sample(workspace(1), batch(60), b"semantic");
    let rebuild = |objects: Vec<OperationObject>| {
        let manifest = prepared.manifest();
        let descriptors = objects
            .iter()
            .map(OperationObject::descriptor)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let rebuilt = OperationBatch::new_with_causality(
            manifest.workspace_id(),
            manifest.lineage_digest(),
            manifest.batch_id(),
            manifest.author_device_id(),
            manifest.author_session_id(),
            manifest.origin(),
            manifest.causal_dot(),
            manifest.causal_dependency_heads().to_vec(),
            manifest.dependency_frontier().clone(),
            manifest.semantic_effect_digest(),
            descriptors,
        )
        .unwrap();
        PreparedBatch::new(rebuilt, objects).unwrap_err()
    };

    let mut malformed = prepared.objects().to_vec();
    let index = malformed
        .iter()
        .position(|object| object.kind() == ObjectKind::ProjectionIntent)
        .unwrap();
    let original = &malformed[index];
    malformed[index] = OperationObject::new(
        original.workspace_id(),
        original.document_id(),
        original.kind(),
        b"malformed manifested intent".to_vec(),
    )
    .unwrap();
    assert!(matches!(
        rebuild(malformed),
        BatchError::ProjectionObject(_)
    ));

    let missing_base = prepared
        .objects()
        .iter()
        .filter(|object| object.kind() != ObjectKind::AnnotatedBaseBlob)
        .cloned()
        .collect();
    assert!(matches!(
        rebuild(missing_base),
        BatchError::ProjectionObject(_)
    ));

    let orphan_base = prepared
        .objects()
        .iter()
        .filter(|object| object.kind() != ObjectKind::ProjectionIntent)
        .cloned()
        .collect();
    assert!(matches!(
        rebuild(orphan_base),
        BatchError::ProjectionObject(_)
    ));
}

#[test]
fn partial_publish_boundary_has_no_ready_effect() {
    let dir = TestDir::new("partial-publish");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(7), b"semantic");
    let store = open_store(&dir, workspace_id);
    for operation_object in &prepared.objects()[..prepared.objects().len() - 1] {
        store
            .stage_object_bytes(&operation_object.encode().unwrap())
            .unwrap();
    }
    assert!(store.committed_manifests().unwrap().is_empty());
    assert_eq!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Absent
    );

    store
        .stage_manifest_bytes(&prepared.manifest().encode().unwrap())
        .unwrap();
    assert!(matches!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Staged { missing, .. } if missing.len() == 1
    ));
}

#[test]
fn malformed_namespace_and_content_address_collision_fail_closed() {
    let dir = TestDir::new("malformed-path");
    let workspace_id = workspace(1);
    let store = open_store(&dir, workspace_id);
    fs::write(
        store.root_path().join("objects/NOT-A-DIGEST.object"),
        b"bad",
    )
    .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(&dir.path().join("candidate-v2-store"), workspace_id),
        Err(StoreError::MalformedPath(_))
    ));

    let collision_dir = TestDir::new("object-collision");
    let store = open_store(&collision_dir, workspace_id);
    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"semantic",
    );
    let bytes = operation_object.encode().unwrap();
    let digest = ContentDigest::of(&bytes);
    fs::write(
        store.root_path().join(format!("objects/{digest}.object")),
        b"different bytes",
    )
    .unwrap();
    assert!(matches!(
        store.stage_object_bytes(&bytes),
        Err(StoreError::ObjectCollision(found)) if found == digest
    ));
}

#[test]
fn reopen_validates_bounded_canonical_namespace_content() {
    let workspace_id = workspace(1);

    let oversized_objects = TestDir::new("oversized-object-reopen");
    let store = open_store(&oversized_objects, workspace_id);
    let path = store.root_path().join(format!(
        "objects/{}.object",
        ContentDigest::of(b"oversized placeholder")
    ));
    fs::File::create(path)
        .unwrap()
        .set_len(MAX_OBJECT_BYTES as u64 + 1)
        .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(
            &oversized_objects.path().join("candidate-v2-store"),
            workspace_id
        ),
        Err(StoreError::StoredFileTooLarge { .. })
    ));

    let corrupt_objects = TestDir::new("corrupt-object-reopen");
    let store = open_store(&corrupt_objects, workspace_id);
    let corrupt = b"not an object envelope";
    fs::write(
        store
            .root_path()
            .join(format!("objects/{}.object", ContentDigest::of(corrupt))),
        corrupt,
    )
    .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(
            &corrupt_objects.path().join("candidate-v2-store"),
            workspace_id
        ),
        Err(StoreError::Batch(BatchError::TruncatedObject))
    ));

    let wrong_address = TestDir::new("wrong-object-address-reopen");
    let store = open_store(&wrong_address, workspace_id);
    let valid = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"valid object",
    )
    .encode()
    .unwrap();
    let claimed = ContentDigest::of(b"different bytes");
    fs::write(
        store.root_path().join(format!("objects/{claimed}.object")),
        valid,
    )
    .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(
            &wrong_address.path().join("candidate-v2-store"),
            workspace_id
        ),
        Err(StoreError::ObjectPathMismatch(found)) if found == claimed
    ));

    let oversized_manifests = TestDir::new("oversized-manifest-reopen");
    let store = open_store(&oversized_manifests, workspace_id);
    let path = store
        .root_path()
        .join(format!("batches/{}.manifest", batch(91)));
    fs::File::create(path)
        .unwrap()
        .set_len(MAX_MANIFEST_BYTES as u64 + 1)
        .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(
            &oversized_manifests.path().join("candidate-v2-store"),
            workspace_id
        ),
        Err(StoreError::StoredFileTooLarge { .. })
    ));

    let corrupt_manifests = TestDir::new("corrupt-manifest-reopen");
    let store = open_store(&corrupt_manifests, workspace_id);
    fs::write(
        store
            .root_path()
            .join(format!("batches/{}.manifest", batch(92))),
        b"not canonical json",
    )
    .unwrap();
    drop(store);
    assert!(matches!(
        ObjectStore::open(
            &corrupt_manifests.path().join("candidate-v2-store"),
            workspace_id
        ),
        Err(StoreError::Batch(BatchError::Decode(_)))
    ));
}

#[test]
fn direct_mixed_lineage_is_inert_on_point_lookup_and_blocks_target_or_reopen() {
    let dir = TestDir::new("direct-mixed-lineage");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(93), b"semantic");
    let store = open_store(&dir, workspace_id);
    store.publish_prepared(&prepared).unwrap();

    let foreign = test_manifest(
        workspace_id,
        LineageDigest::of(b"provider-delivered-foreign-lineage"),
        batch(94),
        prepared.manifest().author_device_id(),
        prepared.manifest().author_session_id(),
        prepared.manifest().dependency_frontier().clone(),
        prepared.manifest().semantic_effect_digest(),
        prepared.manifest().required_objects().to_vec(),
    )
    .unwrap();
    fs::write(
        store
            .root_path()
            .join(format!("batches/{}.manifest", foreign.batch_id())),
        foreign.encode().unwrap(),
    )
    .unwrap();

    assert!(matches!(
        store.inspect_batch(prepared.manifest().batch_id()),
        Ok(BatchInspection::Ready(_))
    ));
    assert!(matches!(
        store.inspect_batch(foreign.batch_id()),
        Err(StoreError::LineageMismatch { .. })
    ));
    drop(store);
    assert!(matches!(
        ObjectStore::open(&dir.path().join("candidate-v2-store"), workspace_id),
        Err(StoreError::LineageMismatch { .. })
    ));
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
))]
#[test]
fn concurrent_manifest_publication_is_atomic_and_never_clobbers() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let dir = TestDir::new("concurrent-no-clobber");
    let workspace_id = workspace(1);
    let batch_id = batch(95);
    let first = Arc::new(sample(workspace_id, batch_id, b"first semantic"));
    let second = Arc::new(sample(workspace_id, batch_id, b"second semantic"));
    let store = Arc::new(open_store(&dir, workspace_id));
    let barrier = Arc::new(Barrier::new(2));

    let handles: Vec<_> = [first, second]
        .into_iter()
        .map(|prepared| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                for operation_object in prepared.objects() {
                    store
                        .stage_object_bytes(&operation_object.encode().unwrap())
                        .unwrap();
                }
                barrier.wait();
                store.stage_manifest_bytes(&prepared.manifest().encode().unwrap())
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(
                |result| matches!(result, Err(StoreError::BatchCollision(id)) if *id == batch_id)
            )
            .count(),
        1
    );
    assert!(matches!(
        store.inspect_batch(batch_id).unwrap(),
        BatchInspection::Ready(_)
    ));
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    windows
))]
#[test]
fn concurrent_first_lineages_can_never_be_exposed_together() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let dir = TestDir::new("concurrent-first-lineages");
    let workspace_id = workspace(1);
    let first = sample(workspace_id, batch(96), b"first lineage semantic");
    let second_base = sample(workspace_id, batch(97), b"second lineage semantic");
    let second_manifest = test_manifest(
        workspace_id,
        LineageDigest::of(b"second concurrent lineage"),
        second_base.manifest().batch_id(),
        second_base.manifest().author_device_id(),
        second_base.manifest().author_session_id(),
        second_base.manifest().dependency_frontier().clone(),
        second_base.manifest().semantic_effect_digest(),
        second_base.manifest().required_objects().to_vec(),
    )
    .unwrap();
    let second = PreparedBatch::new(second_manifest, second_base.objects().to_vec()).unwrap();
    let first = Arc::new(first);
    let second = Arc::new(second);
    let store = Arc::new(open_store(&dir, workspace_id));
    for prepared in [&first, &second] {
        for operation_object in prepared.objects() {
            store
                .stage_object_bytes(&operation_object.encode().unwrap())
                .unwrap();
        }
    }

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [first.clone(), second.clone()]
        .into_iter()
        .map(|prepared| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.stage_manifest_bytes(&prepared.manifest().encode().unwrap())
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "the immutable lineage claim must admit exactly one first publisher"
    );
    let winner = if results[0].is_ok() { &first } else { &second };
    assert!(matches!(
        store.inspect_batch(winner.manifest().batch_id()).unwrap(),
        BatchInspection::Ready(_)
    ));
    assert!(results
        .iter()
        .any(|result| matches!(result, Err(StoreError::LineageMismatch { .. }))));
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected_and_opened_capability_survives_parent_retargeting() {
    use std::os::unix::fs::symlink;

    let workspace_id = workspace(1);
    let graph = TestDir::new("initial-link");
    let outside = TestDir::new("initial-link-outside");
    let linked_root = graph.path().join("candidate-v2-store");
    symlink(outside.path(), &linked_root).unwrap();
    assert!(matches!(
        ObjectStore::open(&linked_root, workspace_id),
        Err(StoreError::UnsafeEntry(_))
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let graph = TestDir::new("namespace-link");
    let outside = TestDir::new("namespace-link-outside");
    let root = graph.path().join("candidate-v2-store");
    let store = ObjectStore::open(&root, workspace_id).unwrap();
    fs::remove_dir(root.join("objects")).unwrap();
    symlink(outside.path(), root.join("objects")).unwrap();
    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"must not escape",
    );
    assert!(matches!(
        store.stage_object_bytes(&operation_object.encode().unwrap()),
        Err(StoreError::UnsafeEntry(_))
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let graph = TestDir::new("object-link");
    let outside = TestDir::new("object-link-outside");
    let store = open_store(&graph, workspace_id);
    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"outside object bytes",
    );
    let bytes = operation_object.encode().unwrap();
    let digest = ContentDigest::of(&bytes);
    let outside_file = outside.path().join("outside.object");
    fs::write(&outside_file, bytes).unwrap();
    symlink(
        &outside_file,
        store.root_path().join(format!("objects/{digest}.object")),
    )
    .unwrap();
    assert!(store.contains_object(digest).is_err());
    drop(store);
    assert!(ObjectStore::open(&graph.path().join("candidate-v2-store"), workspace_id).is_err());

    let graph = TestDir::new("retarget");
    let outside = TestDir::new("retarget-outside");
    let root = graph.path().join("candidate-v2-store");
    let store = ObjectStore::open(&root, workspace_id).unwrap();
    let detached = graph.path().join("detached-store");
    fs::rename(&root, &detached).unwrap();
    symlink(outside.path(), &root).unwrap();

    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"stays on capability",
    );
    store
        .stage_object_bytes(&operation_object.encode().unwrap())
        .unwrap();
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(detached.join("objects")).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_no_follow_publication_read_and_directory_flush_succeed() {
    let dir = TestDir::new("windows-publication");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(98), b"windows semantic");
    let store = open_store(&dir, workspace_id);

    store.publish_prepared(&prepared).unwrap();
    for operation_object in prepared.objects() {
        assert!(store
            .contains_object(operation_object.descriptor().unwrap().content_digest())
            .unwrap());
    }
    assert!(matches!(
        store.inspect_batch(prepared.manifest().batch_id()).unwrap(),
        BatchInspection::Ready(_)
    ));
}

#[cfg(windows)]
#[test]
fn windows_reparse_files_and_directories_are_rejected() {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    let workspace_id = workspace(1);

    let graph = TestDir::new("windows-directory-reparse");
    let outside = TestDir::new("windows-directory-reparse-outside");
    let store = open_store(&graph, workspace_id);
    fs::remove_dir(store.root_path().join("objects")).unwrap();
    symlink_dir(outside.path(), store.root_path().join("objects")).unwrap();
    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"must not cross directory reparse point",
    );
    assert!(store
        .stage_object_bytes(&operation_object.encode().unwrap())
        .is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let graph = TestDir::new("windows-file-reparse");
    let outside = TestDir::new("windows-file-reparse-outside");
    let store = open_store(&graph, workspace_id);
    let operation_object = object(
        workspace_id,
        document(1),
        ObjectKind::SemanticEffect,
        b"must not read through file reparse point",
    );
    let bytes = operation_object.encode().unwrap();
    let digest = ContentDigest::of(&bytes);
    let outside_file = outside.path().join("outside.object");
    fs::write(&outside_file, bytes).unwrap();
    symlink_file(
        &outside_file,
        store.root_path().join(format!("objects/{digest}.object")),
    )
    .unwrap();
    assert!(store.contains_object(digest).is_err());
    drop(store);
    assert!(ObjectStore::open(&graph.path().join("candidate-v2-store"), workspace_id).is_err());
}

#[test]
fn reopen_never_compacts_or_deletes_objects_or_manifests() {
    let dir = TestDir::new("no-compaction");
    let workspace_id = workspace(1);
    let prepared = sample(workspace_id, batch(8), b"semantic");
    let root = dir.path().join("candidate-v2-store");
    let before = {
        let store = ObjectStore::open(&root, workspace_id).unwrap();
        store.publish_prepared(&prepared).unwrap();
        authoritative_file_count(&root)
    };
    let reopened = ObjectStore::open(&root, workspace_id).unwrap();
    assert_eq!(authoritative_file_count(&root), before);
    assert_eq!(reopened.committed_manifests().unwrap().len(), 1);
    assert!(matches!(
        reopened.inspect_batch(batch(8)).unwrap(),
        BatchInspection::Ready(_)
    ));
    assert_eq!(authoritative_file_count(&root), before);
}

#[test]
fn object_checksum_helper_matches_envelope_contract() {
    let operation_object = object(
        workspace(1),
        document(1),
        ObjectKind::SemanticEffect,
        b"semantic",
    );
    let mut bytes = operation_object.encode().unwrap();
    bytes[20] ^= 1;
    rewrite_checksum(&mut bytes);
    assert!(OperationObject::decode(&bytes).is_err());
}
