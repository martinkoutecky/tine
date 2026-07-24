use serde_json::{json, Value};
use tine_core::oplog::{
    AnnotatedIdentity, BatchId, BlobDescription, BlockId, CrdtPeerCounter, CrdtPeerId, DeviceId,
    DocumentDependencies, DocumentId, FrontierV2, ImportId, ImportInventoryEntry,
    ImportInventoryState, ImportLocator, LogseqUuid, ManagedPath, PageId, PortablePathKey,
    ProjectionClaimEvidence, ProjectionClaimParticipant, ProjectionCompletion, ProjectionIntent,
    ProjectionPrecondition, ReceiptError, SessionId, StructuralLocator, StructuralSpan,
    WorkspaceId, DIFF_SCHEMA_VERSION,
};
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn workspace(value: u128) -> WorkspaceId {
    WorkspaceId::from_uuid(uuid(value))
}

fn page(value: u128) -> PageId {
    PageId::from_uuid(uuid(value))
}

fn block(value: u128) -> BlockId {
    BlockId::from_uuid(uuid(value))
}

fn batch(value: u128) -> BatchId {
    BatchId::from_uuid(uuid(value))
}

fn document(value: u128) -> DocumentId {
    DocumentId::from_uuid(uuid(value))
}

fn peer(value: u64, max_counter: u64) -> CrdtPeerCounter {
    CrdtPeerCounter::new(CrdtPeerId::from_u64(value), max_counter)
}

fn assert_uuid_v4(value: Uuid) {
    assert_eq!(value.get_version_num(), 4);
}

fn locator(parts: &[u32]) -> StructuralLocator {
    StructuralLocator::new(parts.to_vec()).unwrap()
}

fn annotation(
    parts: &[u32],
    start: u64,
    end: u64,
    block_value: u128,
    logseq_value: Option<u128>,
) -> AnnotatedIdentity {
    AnnotatedIdentity::new(
        locator(parts),
        StructuralSpan::new(start, end).unwrap(),
        block(block_value),
        logseq_value.map(|value| LogseqUuid::from_uuid(uuid(value))),
    )
}

fn sample_intent() -> ProjectionIntent {
    let frontier = FrontierV2::new(vec![
        DocumentDependencies::new(
            document(20),
            vec![peer(9, 12), peer(2, 7)],
            vec![batch(4), batch(3)],
        )
        .unwrap(),
        DocumentDependencies::new(document(10), vec![peer(5, 8)], vec![batch(2), batch(1)])
            .unwrap(),
    ])
    .unwrap();

    intent_with_frontier(frontier)
}

fn intent_with_frontier(frontier: FrontierV2) -> ProjectionIntent {
    let base = b"- old\n".to_vec();
    let target = b"- first\n  - second\n";
    let participant_home = frontier
        .documents()
        .first()
        .map(DocumentDependencies::document_id);
    let claim_evidence = participant_home
        .map(|home| {
            vec![
                ProjectionClaimEvidence::new(
                    LogseqUuid::from_uuid(uuid(111)),
                    vec![ProjectionClaimParticipant::new(block(11), home)],
                )
                .unwrap(),
                ProjectionClaimEvidence::new(
                    LogseqUuid::from_uuid(uuid(112)),
                    vec![ProjectionClaimParticipant::new(block(12), home)],
                )
                .unwrap(),
            ]
        })
        .unwrap_or_default();

    ProjectionIntent::new(
        workspace(1),
        page(2),
        ManagedPath::parse("pages/hello.md").unwrap(),
        frontier,
        claim_evidence,
        ProjectionPrecondition::Base(BlobDescription::of(&base)),
        BlobDescription::of(target),
        vec![
            annotation(
                &[1, 0],
                10,
                target.len() as u64,
                12,
                participant_home.map(|_| 112),
            ),
            annotation(&[0], 0, 8, 11, participant_home.map(|_| 111)),
        ],
    )
    .unwrap()
}

#[test]
fn intent_and_completion_semantic_roundtrip() {
    let intent = sample_intent();
    let intent_bytes = intent.encode().unwrap();
    let decoded = ProjectionIntent::decode(&intent_bytes).unwrap();
    assert_eq!(decoded, intent);

    let target = b"- first\n  - second\n";
    let completion = ProjectionCompletion::for_intent(&decoded, target).unwrap();
    let completion_bytes = completion.encode().unwrap();
    let decoded_completion =
        ProjectionCompletion::decode_bound(&completion_bytes, &intent).unwrap();
    assert_eq!(decoded_completion, completion);
    assert_eq!(decoded_completion.intent_id(), intent.id().unwrap());
}

#[test]
fn ordinary_application_ids_are_minted_as_uuid_v4() {
    assert_uuid_v4(WorkspaceId::new().as_uuid());
    assert_uuid_v4(PageId::new().as_uuid());
    assert_uuid_v4(BlockId::new().as_uuid());
    assert_uuid_v4(BatchId::new().as_uuid());
    assert_uuid_v4(DeviceId::new().as_uuid());
    assert_uuid_v4(SessionId::new().as_uuid());
    assert_uuid_v4(DocumentId::new().as_uuid());
}

#[test]
fn decode_rejects_truncation_corruption_and_unknown_versions() {
    let intent = sample_intent();
    let encoded = intent.encode().unwrap();
    let current: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(current["receipt_schema_version"], json!(5));
    assert_eq!(current["projection_schema_version"], json!(4));
    assert_eq!(current["projection_policy_version"], json!(1));
    assert_eq!(current["managed_entity_set_version"], json!(1));
    assert!(current["frontier"][0]["direct_dependency_heads"].is_array());
    assert!(current["frontier"][0]["causal_state_digest"].is_string());
    assert!(ProjectionIntent::decode(&encoded[..encoded.len() - 3]).is_err());

    let mut corrupt: Value = serde_json::from_slice(&encoded).unwrap();
    corrupt["target"]["sha256"] = json!("not-a-digest");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&corrupt).unwrap()).is_err());

    for (field, version) in [
        ("receipt_schema_version", 9),
        ("projection_schema_version", 9),
        ("projection_policy_version", 9),
        ("managed_entity_set_version", 9),
    ] {
        let mut unknown: Value = serde_json::from_slice(&encoded).unwrap();
        unknown[field] = json!(version);
        let error = ProjectionIntent::decode(&serde_json::to_vec(&unknown).unwrap()).unwrap_err();
        assert!(matches!(error, ReceiptError::Decode(_)), "{field}: {error}");
    }

    let mut unknown_policy: Value = serde_json::from_slice(&encoded).unwrap();
    unknown_policy["policy"] = json!("future_policy");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&unknown_policy).unwrap()).is_err());

    for (field, old_version) in [
        ("receipt_schema_version", 4),
        ("projection_schema_version", 3),
    ] {
        let mut old: Value = serde_json::from_slice(&encoded).unwrap();
        old[field] = json!(old_version);
        assert!(
            ProjectionIntent::decode(&serde_json::to_vec(&old).unwrap()).is_err(),
            "accepted old {field}"
        );
    }
    let mut old_shape: Value = serde_json::from_slice(&encoded).unwrap();
    old_shape.as_object_mut().unwrap().remove("claim_evidence");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&old_shape).unwrap()).is_err());

    let mut embedded_base: Value = serde_json::from_slice(&encoded).unwrap();
    let description = embedded_base["precondition"]["base"].clone();
    embedded_base["precondition"]["base"] = json!({
        "description": description,
        "bytes": [45, 32, 111, 108, 100, 10]
    });
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&embedded_base).unwrap()).is_err());

    let mut omitted_home: Value = serde_json::from_slice(&encoded).unwrap();
    omitted_home["claim_evidence"][0]["participants"][0]["home_document_id"] =
        json!(document(999).to_string());
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&omitted_home).unwrap()).is_err());
}

#[test]
fn intent_wire_size_does_not_scale_with_base_byte_length() {
    let make = |description| {
        ProjectionIntent::new(
            workspace(1),
            page(2),
            ManagedPath::parse("archive/pages/size.md").unwrap(),
            FrontierV2::default(),
            Vec::new(),
            ProjectionPrecondition::Base(description),
            BlobDescription::of(b"- target\n"),
            Vec::new(),
        )
        .unwrap()
        .encode()
        .unwrap()
    };
    let tiny = make(BlobDescription::of(b"x"));
    let huge = make(BlobDescription::from_parts([7; 32], 32 * 1024 * 1024));

    assert!(tiny.len() < 1024);
    assert!(huge.len() < 1024);
    assert!(tiny.len().abs_diff(huge.len()) < 16);
    let wire: Value = serde_json::from_slice(&tiny).unwrap();
    assert!(wire["precondition"]["base"]["sha256"].is_string());
    assert!(wire["precondition"]["base"].get("bytes").is_none());
}

#[test]
fn managed_paths_are_canonical_safe_graph_relative_paths() {
    for valid in [
        "pages/a.md",
        "pages/nested/a.org",
        "journals/2026_07_22.md",
        "journals/archive/day.org",
        "archive/pages/foo.md",
        "custom/notes/topic.org",
        "assets/a.md",
        ".tine-sync/page.md",
    ] {
        assert_eq!(ManagedPath::parse(valid).unwrap().as_str(), valid);
    }

    for invalid in [
        "",
        "/pages/a.md",
        "pages",
        "pages/a.txt",
        "pages/a.markdown",
        "pages/../a.md",
        "pages/./a.md",
        "pages//a.md",
        "pages\\a.md",
        "pages/C:a.md",
        "pages/a<b.md",
        "pages/a>b.md",
        "pages/a\"b.md",
        "pages/a|b.md",
        "pages/a?b.md",
        "pages/a*b.md",
        "pages/CON.md",
        "pages/Lpt9.org",
        "pages/COM¹.md",
        "pages/com².any.md",
        "pages/LpT³.org",
        "pages/trailing /a.md",
        "pages/.md",
        "config.edn",
        "journals/a.MD",
        " pages/a.md",
        "pages/a.md ",
    ] {
        assert!(ManagedPath::parse(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn portable_path_keys_use_versioned_canonical_full_case_folding() {
    fn key(path: &str) -> PortablePathKey {
        ManagedPath::parse(path).unwrap().portable_key()
    }

    for (left, right) in [
        ("pages/Foo.md", "pages/foo.md"),
        ("pages/Café.md", "pages/Cafe\u{301}.md"),
        ("pages/Straße.md", "pages/STRASSE.md"),
        ("pages/Σίσυφος.md", "pages/σίσυφοσ.md"),
        ("pages/Kelvin.md", "pages/kelvin.md"),
    ] {
        assert_eq!(key(left), key(right), "{left:?} and {right:?}");
        assert_eq!(key(left).digest(), key(right).digest());
    }
    assert_ne!(
        key("pages/①.md"),
        key("pages/1.md"),
        "compatibility-only normalization must remain distinct"
    );
    assert_eq!(
        ManagedPath::parse("pages/Cafe\u{301}.md").unwrap().as_str(),
        "pages/Cafe\u{301}.md",
        "the projected spelling must remain untouched"
    );
}

#[test]
fn malformed_and_duplicate_identity_evidence_is_rejected() {
    assert!(LogseqUuid::parse("not-a-uuid").is_err());
    let target = b"abcdefgh";
    let common_uuid = Some(900);

    let duplicate_locator = ProjectionIntent::new(
        workspace(1),
        page(1),
        ManagedPath::parse("pages/a.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(target),
        vec![
            annotation(&[0], 0, 4, 1, Some(1)),
            annotation(&[0], 4, 8, 2, Some(2)),
        ],
    )
    .unwrap_err();
    assert_eq!(duplicate_locator, ReceiptError::DuplicateLocator);

    let duplicate_anchor = ProjectionIntent::new(
        workspace(1),
        page(1),
        ManagedPath::parse("pages/a.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(target),
        vec![
            annotation(&[0], 0, 4, 1, common_uuid),
            annotation(&[1], 4, 8, 2, common_uuid),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_anchor,
        ReceiptError::DuplicateLogseqIdentity(_)
    ));

    let duplicate_internal = ProjectionIntent::new(
        workspace(1),
        page(1),
        ManagedPath::parse("pages/a.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(target),
        vec![
            annotation(&[0], 0, 4, 1, Some(1)),
            annotation(&[1], 4, 8, 1, Some(2)),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        duplicate_internal,
        ReceiptError::DuplicateBlockIdentity(_)
    ));
}

#[test]
fn decode_rejects_noncanonical_dependencies_and_annotations() {
    let encoded = sample_intent().encode().unwrap();

    let mut unsorted_documents: Value = serde_json::from_slice(&encoded).unwrap();
    unsorted_documents["frontier"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&unsorted_documents).unwrap()).is_err());

    let mut unsorted_batches: Value = serde_json::from_slice(&encoded).unwrap();
    unsorted_batches["frontier"][0]["direct_dependency_heads"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&unsorted_batches).unwrap()).is_err());

    let mut unsorted_peers: Value = serde_json::from_slice(&encoded).unwrap();
    unsorted_peers["frontier"][1]["peer_counters"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&unsorted_peers).unwrap()).is_err());

    let mut unsorted_annotations: Value = serde_json::from_slice(&encoded).unwrap();
    unsorted_annotations["annotations"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&unsorted_annotations).unwrap()).is_err());
}

#[test]
fn decode_rejects_duplicate_document_and_batch_dependencies() {
    let encoded = sample_intent().encode().unwrap();

    let mut duplicate_document: Value = serde_json::from_slice(&encoded).unwrap();
    duplicate_document["frontier"][1]["document_id"] =
        duplicate_document["frontier"][0]["document_id"].clone();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&duplicate_document).unwrap()).is_err());

    let mut duplicate_batch: Value = serde_json::from_slice(&encoded).unwrap();
    duplicate_batch["frontier"][0]["direct_dependency_heads"][1] =
        duplicate_batch["frontier"][0]["direct_dependency_heads"][0].clone();
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&duplicate_batch).unwrap()).is_err());

    assert!(matches!(
        FrontierV2::new(vec![
            DocumentDependencies::new(document(1), vec![peer(1, 1)], vec![batch(1)]).unwrap(),
            DocumentDependencies::new(document(1), vec![peer(2, 1)], vec![batch(2)]).unwrap(),
        ]),
        Err(ReceiptError::DuplicateDocument(_))
    ));
    assert!(matches!(
        DocumentDependencies::new(document(1), vec![peer(1, 1)], vec![batch(1), batch(1)]),
        Err(ReceiptError::DuplicateDependency(_))
    ));
    assert!(matches!(
        DocumentDependencies::new(document(1), vec![peer(1, 1), peer(1, 2)], vec![batch(1)]),
        Err(ReceiptError::DuplicateCrdtPeer(_))
    ));
    assert!(matches!(
        DocumentDependencies::new(document(1), vec![], vec![]),
        Err(ReceiptError::EmptyDocumentFrontier(_))
    ));
}

#[test]
fn frontier_construction_is_canonical_and_true_empty_baseline_is_valid() {
    let first = DocumentDependencies::new(
        document(1),
        vec![peer(9, 4), peer(2, 8)],
        vec![batch(7), batch(3)],
    )
    .unwrap();
    assert_eq!(first.peer_counters(), &[peer(2, 8), peer(9, 4)]);
    assert_eq!(first.direct_dependency_heads(), &[batch(3), batch(7)]);

    let equivalent = DocumentDependencies::new(
        document(1),
        vec![peer(2, 8), peer(9, 4)],
        vec![batch(3), batch(7)],
    )
    .unwrap();
    assert_eq!(first, equivalent);
    assert_eq!(
        first.causal_state_digest(),
        equivalent.causal_state_digest()
    );

    assert!(FrontierV2::default().documents().is_empty());
    let empty_intent = intent_with_frontier(FrontierV2::default());
    assert_eq!(
        ProjectionIntent::decode(&empty_intent.encode().unwrap()).unwrap(),
        empty_intent
    );
    assert!(DocumentDependencies::new(document(2), vec![peer(1, 0)], vec![]).is_ok());
    assert!(DocumentDependencies::new(document(3), vec![], vec![batch(1)]).is_ok());
}

#[test]
fn frontier_decode_recomputes_causal_digest_and_rejects_malformed_entries() {
    let encoded = sample_intent().encode().unwrap();

    let mut mismatch: Value = serde_json::from_slice(&encoded).unwrap();
    mismatch["frontier"][0]["causal_state_digest"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&mismatch).unwrap()).is_err());

    let mut malformed_digest: Value = serde_json::from_slice(&encoded).unwrap();
    malformed_digest["frontier"][0]["causal_state_digest"] = json!("ABCDEF");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&malformed_digest).unwrap()).is_err());

    let mut missing_counters: Value = serde_json::from_slice(&encoded).unwrap();
    missing_counters["frontier"][0]
        .as_object_mut()
        .unwrap()
        .remove("peer_counters");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&missing_counters).unwrap()).is_err());

    let peer_only = ProjectionIntent::new(
        workspace(1),
        page(1),
        ManagedPath::parse("pages/a.md").unwrap(),
        FrontierV2::new(vec![DocumentDependencies::new(
            document(1),
            vec![peer(1, 0)],
            vec![],
        )
        .unwrap()])
        .unwrap(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(b""),
        vec![],
    )
    .unwrap();
    let mut both_empty: Value = serde_json::from_slice(&peer_only.encode().unwrap()).unwrap();
    both_empty["frontier"][0]["peer_counters"] = json!([]);
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&both_empty).unwrap()).is_err());
}

#[test]
fn decode_rejects_empty_locator_and_reversed_span() {
    let encoded = sample_intent().encode().unwrap();

    let mut empty_locator: Value = serde_json::from_slice(&encoded).unwrap();
    empty_locator["annotations"][0]["locator"] = json!([]);
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&empty_locator).unwrap()).is_err());

    let mut reversed_span: Value = serde_json::from_slice(&encoded).unwrap();
    reversed_span["annotations"][0]["span"] = json!({ "start": 8, "end": 7 });
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&reversed_span).unwrap()).is_err());
}

#[test]
fn base_blob_and_target_consistency_is_validated() {
    let encoded = sample_intent().encode().unwrap();

    let mut bad_length: Value = serde_json::from_slice(&encoded).unwrap();
    bad_length["precondition"]["base"]["description"]["byte_length"] = json!(999);
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&bad_length).unwrap()).is_err());

    let mut bad_digest: Value = serde_json::from_slice(&encoded).unwrap();
    bad_digest["precondition"]["base"]["description"]["sha256"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&bad_digest).unwrap()).is_err());

    let too_short = ProjectionIntent::new(
        workspace(1),
        page(1),
        ManagedPath::parse("pages/a.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(b"short"),
        vec![annotation(&[0], 0, 6, 1, None)],
    )
    .unwrap_err();
    assert!(matches!(too_short, ReceiptError::SpanOutsideTarget { .. }));
}

#[test]
fn import_id_and_unmatched_ids_are_deterministic_and_domain_separated() {
    let intent = sample_intent();
    let logical_completion_ids =
        [
            ProjectionCompletion::for_intent(&intent, b"- first\n  - second\n")
                .unwrap()
                .logical_completion_id(),
        ];
    let inventory = [
        ImportInventoryEntry::new(
            ManagedPath::parse("journals/2026_07_22.md").unwrap(),
            ImportInventoryState::Absent,
        ),
        ImportInventoryEntry::new(
            ManagedPath::parse("pages/hello.md").unwrap(),
            ImportInventoryState::Present(BlobDescription::of(b"external bytes")),
        ),
    ];

    let first = ImportId::derive(
        workspace(1),
        &logical_completion_ids,
        &inventory,
        DIFF_SCHEMA_VERSION,
    )
    .unwrap();
    let second = ImportId::derive(
        workspace(1),
        &logical_completion_ids,
        &inventory,
        DIFF_SCHEMA_VERSION,
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.batch_id(), second.batch_id());

    let page_locator = ImportLocator::page(ManagedPath::parse("pages/hello.md").unwrap());
    let block_locator = ImportLocator::block(
        ManagedPath::parse("pages/hello.md").unwrap(),
        locator(&[0, 2]),
    );
    assert_eq!(
        first.unmatched_page_id(&page_locator),
        second.unmatched_page_id(&page_locator)
    );
    assert_eq!(
        first.unmatched_block_id(&block_locator),
        second.unmatched_block_id(&block_locator)
    );
    assert_ne!(
        first.unmatched_page_id(&block_locator).as_uuid(),
        first.unmatched_block_id(&block_locator).as_uuid()
    );
    assert_ne!(
        first.batch_id().as_uuid(),
        first.unmatched_page_id(&page_locator).as_uuid()
    );

    let changed_inventory = [
        inventory[0].clone(),
        ImportInventoryEntry::new(
            ManagedPath::parse("pages/hello.md").unwrap(),
            ImportInventoryState::Present(BlobDescription::of(b"different bytes")),
        ),
    ];
    assert_ne!(
        first,
        ImportId::derive(
            workspace(1),
            &logical_completion_ids,
            &changed_inventory,
            DIFF_SCHEMA_VERSION,
        )
        .unwrap()
    );
    assert!(ImportId::derive(workspace(1), &logical_completion_ids, &inventory, 99).is_err());
}

#[test]
fn import_derivation_rejects_noncanonical_evidence() {
    let intent_a = sample_intent();
    let intent_b = ProjectionIntent::new(
        workspace(2),
        page(2),
        ManagedPath::parse("pages/b.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::of(b"b"),
        vec![],
    )
    .unwrap();
    let mut completions = vec![
        ProjectionCompletion::for_intent(&intent_a, b"- first\n  - second\n")
            .unwrap()
            .logical_completion_id(),
        ProjectionCompletion::for_intent(&intent_b, b"b")
            .unwrap()
            .logical_completion_id(),
    ];
    completions.sort_unstable();
    completions.reverse();
    assert!(matches!(
        ImportId::derive(workspace(1), &completions, &[], DIFF_SCHEMA_VERSION),
        Err(ReceiptError::NonCanonicalLogicalCompletionIds)
    ));

    let reversed_inventory = [
        ImportInventoryEntry::new(
            ManagedPath::parse("pages/z.md").unwrap(),
            ImportInventoryState::Absent,
        ),
        ImportInventoryEntry::new(
            ManagedPath::parse("pages/a.md").unwrap(),
            ImportInventoryState::Absent,
        ),
    ];
    assert!(matches!(
        ImportId::derive(workspace(1), &[], &reversed_inventory, DIFF_SCHEMA_VERSION),
        Err(ReceiptError::NonCanonicalInventory)
    ));
}

#[test]
fn dense_projection_policy_wire_is_rejected() {
    let intent = sample_intent();
    let mut wire: Value = serde_json::from_slice(&intent.encode().unwrap()).unwrap();
    assert_eq!(wire["policy"], json!("sparse_logseq_ids"));
    wire["policy"] = json!("dense_logseq_ids");
    assert!(ProjectionIntent::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
}

#[test]
fn projection_intent_id_is_stable_across_equivalent_encodings() {
    let intent = sample_intent();
    let compact = intent.encode().unwrap();
    let value: Value = serde_json::from_slice(&compact).unwrap();
    let pretty = serde_json::to_vec_pretty(&value).unwrap();
    assert_ne!(compact, pretty);

    let compact_decoded = ProjectionIntent::decode(&compact).unwrap();
    let pretty_decoded = ProjectionIntent::decode(&pretty).unwrap();
    assert_eq!(compact_decoded, pretty_decoded);
    assert_eq!(intent.id().unwrap(), compact_decoded.id().unwrap());
    assert_eq!(intent.id().unwrap(), pretty_decoded.id().unwrap());
}

#[test]
fn projection_intent_id_binds_peer_counters_and_direct_dependency_heads() {
    let make_intent = |peer_id, max_counter, dependencies| {
        intent_with_frontier(
            FrontierV2::new(vec![DocumentDependencies::new(
                document(10),
                vec![peer(peer_id, max_counter)],
                dependencies,
            )
            .unwrap()])
            .unwrap(),
        )
    };

    let baseline = make_intent(2, 7, vec![batch(1), batch(2)]);
    let changed_peer = make_intent(3, 7, vec![batch(1), batch(2)]);
    let changed_counter = make_intent(2, 8, vec![batch(1), batch(2)]);
    let changed_heads = make_intent(2, 7, vec![batch(1), batch(2), batch(3)]);

    assert_ne!(baseline.id().unwrap(), changed_peer.id().unwrap());
    assert_ne!(baseline.id().unwrap(), changed_counter.id().unwrap());
    assert_ne!(baseline.id().unwrap(), changed_heads.id().unwrap());

    let encoded: Value = serde_json::from_slice(&baseline.encode().unwrap()).unwrap();
    assert_eq!(encoded["projection_schema_version"], json!(4));
    assert!(encoded["frontier"][0]["causal_state_digest"].is_string());
}

#[test]
fn completion_version_corruption_is_rejected_by_bound_decode() {
    let intent = sample_intent();
    let completion = ProjectionCompletion::for_intent(&intent, b"- first\n  - second\n").unwrap();
    let encoded = completion.encode().unwrap();

    for (field, expected) in [
        ("receipt_schema_version", "receipt schema"),
        ("projection_schema_version", "projection schema"),
        ("projection_policy_version", "projection policy version"),
        ("managed_entity_set_version", "managed entity-set version"),
    ] {
        let mut corrupt: Value = serde_json::from_slice(&encoded).unwrap();
        corrupt[field] = json!(99);
        let error =
            ProjectionCompletion::decode_bound(&serde_json::to_vec(&corrupt).unwrap(), &intent)
                .unwrap_err();
        assert!(error.to_string().contains(expected), "{field}: {error}");
    }
}

#[test]
fn logical_completion_is_replica_stable_and_contains_no_local_artifacts() {
    let intent = sample_intent();
    let target = b"- first\n  - second\n";
    let first_device = ProjectionCompletion::for_intent(&intent, target).unwrap();
    let second_device = ProjectionCompletion::for_intent(&intent, target).unwrap();
    let bytes = first_device.encode().unwrap();
    assert_eq!(bytes, second_device.encode().unwrap());
    assert_eq!(
        first_device.logical_completion_id(),
        second_device.logical_completion_id()
    );
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(wire.get("displacements").is_none());
    assert!(wire.get("attempt_id").is_none());
    assert!(wire.get("recovery_filename").is_none());
    assert!(wire.get("base").is_none());
    let decoded = ProjectionCompletion::decode_bound(&bytes, &intent).unwrap();
    assert_eq!(decoded, first_device);

    for field in ["logical_completion_id", "intent_id"] {
        let mut old_shape: Value = serde_json::from_slice(&bytes).unwrap();
        old_shape.as_object_mut().unwrap().remove(field);
        assert!(
            ProjectionCompletion::decode_bound(&serde_json::to_vec(&old_shape).unwrap(), &intent)
                .is_err(),
            "accepted completion without {field}"
        );
    }
    let mut old_shape: Value = serde_json::from_slice(&bytes).unwrap();
    old_shape["displacements"] = json!([]);
    assert!(
        ProjectionCompletion::decode_bound(&serde_json::to_vec(&old_shape).unwrap(), &intent)
            .is_err()
    );
}

#[test]
fn trusted_completion_cannot_be_obtained_without_exact_bound_decode() {
    let sparse = sample_intent();
    assert_eq!(
        ProjectionCompletion::for_intent(&sparse, b"wrong bytes").unwrap_err(),
        ReceiptError::CompletionTargetMismatch
    );

    let target = b"- first\n  - second\n";
    let completion = ProjectionCompletion::for_intent(&sparse, target).unwrap();
    let mut other_wire: Value = serde_json::from_slice(&sparse.encode().unwrap()).unwrap();
    other_wire["path"] = json!("pages/other.md");
    let other = ProjectionIntent::decode(&serde_json::to_vec(&other_wire).unwrap()).unwrap();
    assert_eq!(
        completion.validate_against(&other).unwrap_err(),
        ReceiptError::CompletionIntentMismatch
    );
    // Completion has no public unbound decoder and does not implement Deserialize.
    // The only byte-decoding API requires the exact intent and rejects this mismatch.
    assert!(ProjectionCompletion::decode_bound(&completion.encode().unwrap(), &other).is_err());
}
