use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tine_core::oplog::{
    derive_receiver_local_projection, execute_manifested_projection_work, plan_projection,
    recover_incomplete_projections, write_projection_exact, AnnotatedIdentity, AuthorBatch,
    BatchDisposition, BatchId, BatchInspection, BatchOrigin, BlobDescription, BlockId,
    BlockLocation, CrdtPeerCounter, CrdtPeerId, DeviceId, DocumentDependencies, DocumentId,
    EngineError, FrontierV2, LineageDigest, LogseqIdentityOrigin, LogseqUuid, ManagedPath,
    ManifestProjectionPrecondition, ManifestProjectionTarget, ManifestedProjectionIntent,
    MaterializationStats, MaterializedBlock, MaterializedPage, ObjectKind, ObjectStore,
    OperationBatch, OperationObject, OperationTransaction, PageId, PolicyGeneratedAnchorReason,
    PreparedBatch, ProjectionClaimEvidence, ProjectionClaimParticipant, ProjectionEndpointBinding,
    ProjectionEndpointId, ProjectionError, ProjectionIntent, ProjectionPageState,
    ProjectionPrecondition, ProjectionReceiptStore, ProjectionStoreError, ProjectionWorkStatus,
    ProjectionWorkTarget, SemanticOperation, SessionId, ShardedHotEngine, StoreError,
    StructuralSpan, WorkspaceId, PORTABLE_PATH_KEY_VERSION,
};
use tine_core::Graph;
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-projection-{label}-{}", Uuid::new_v4()));
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

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn workspace(value: u128) -> WorkspaceId {
    WorkspaceId::from_uuid(uuid(value))
}

fn logseq(value: u128) -> LogseqUuid {
    LogseqUuid::from_uuid(uuid(value))
}

fn projection_binding(graph: &Graph, seed: u128) -> ProjectionEndpointBinding {
    ProjectionEndpointBinding {
        endpoint_id: ProjectionEndpointId::from_uuid(uuid(seed)),
        device_id: DeviceId::from_uuid(uuid(seed + 1)),
        graph_resource_id: graph.canonical_resource_id().unwrap(),
    }
}

fn block(
    value: u128,
    parent: Option<u128>,
    order: &str,
    content: impl Into<String>,
    logseq_uuid: Option<LogseqUuid>,
) -> MaterializedBlock {
    MaterializedBlock {
        block_id: BlockId::from_uuid(uuid(value)),
        home_document_id: DocumentId::from_uuid(uuid(10_000 + value)),
        parent: parent.map(|value| BlockId::from_uuid(uuid(value))),
        order: order.into(),
        logseq_uuid,
        logseq_identity_origin: logseq_uuid.map(|_| LogseqIdentityOrigin::ExternalImported),
        content: content.into(),
    }
}

fn generated_block(
    value: u128,
    parent: Option<u128>,
    order: &str,
    content: impl Into<String>,
    logseq_uuid: LogseqUuid,
    reason: PolicyGeneratedAnchorReason,
) -> MaterializedBlock {
    let mut block = block(value, parent, order, content, Some(logseq_uuid));
    block.logseq_identity_origin = Some(LogseqIdentityOrigin::PolicyGenerated { reason });
    block
}

fn page(path: &str, blocks: Vec<MaterializedBlock>) -> ProjectionPageState {
    let mut participants = BTreeMap::<LogseqUuid, Vec<ProjectionClaimParticipant>>::new();
    for block in &blocks {
        if let Some(logseq_uuid) = block.logseq_uuid {
            participants
                .entry(logseq_uuid)
                .or_default()
                .push(ProjectionClaimParticipant::new(
                    block.block_id,
                    block.home_document_id,
                ));
        }
    }
    let frontier = FrontierV2::new(
        participants
            .values()
            .flatten()
            .map(|participant| participant.home_document_id())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, document_id)| {
                DocumentDependencies::new(
                    document_id,
                    vec![CrdtPeerCounter::new(
                        CrdtPeerId::from_u64(index as u64 + 1),
                        0,
                    )],
                    vec![],
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    ProjectionPageState {
        page: MaterializedPage {
            page_id: tine_core::oplog::PageId::from_uuid(uuid(500)),
            path: ManagedPath::parse(path).unwrap(),
            preamble: None,
            blocks,
            stats: MaterializationStats::default(),
        },
        frontier,
        claim_evidence: participants
            .into_iter()
            .map(|(uuid, participants)| ProjectionClaimEvidence::new(uuid, participants).unwrap())
            .collect(),
    }
}

fn authorized_engine(
    dir: &TestDir,
    relative_path: &str,
    content: &str,
    enrollment: Option<(&Graph, &ProjectionReceiptStore)>,
) -> (ShardedHotEngine, PageId) {
    let workspace_id = workspace(1);
    let lineage = LineageDigest::of(b"projection-test-lineage");
    let catalog = DocumentId::from_uuid(uuid(700));
    let page_id = PageId::from_uuid(uuid(701));
    let home = DocumentId::from_uuid(uuid(702));
    let block_id = BlockId::from_uuid(uuid(703));
    let batch_id = BatchId::from_uuid(uuid(704));
    let author = ShardedHotEngine::new(workspace_id, lineage, catalog);
    let transaction = OperationTransaction::new(vec![
        SemanticOperation::CreatePage {
            page_id,
            home_document_id: home,
            path: ManagedPath::parse(relative_path).unwrap(),
        },
        SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id,
                home_document_id: home,
            },
            page_id,
            parent: None,
            order: "a".into(),
            content: content.into(),
        },
    ])
    .unwrap();
    let prepared = author
        .prepare_bootstrap_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: DeviceId::from_uuid(uuid(705)),
                author_session_id: SessionId::from_uuid(uuid(706)),
                crdt_peer_id: CrdtPeerId::from_u64(707),
            },
            &transaction,
        )
        .unwrap();

    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, workspace_id).unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        writer.inspect_batch(batch_id).unwrap(),
        BatchInspection::Ready(_)
    ));
    drop(writer);

    let reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let mut engine = match enrollment {
        Some((graph, receipts)) => {
            ShardedHotEngine::with_enrolled_projection(reader, lineage, catalog, graph, receipts)
        }
        None => ShardedHotEngine::with_archive_store(reader, lineage, catalog),
    };
    engine.stage_archive_batch(batch_id).unwrap();
    (engine, page_id)
}

fn enrolled_engine_and_store(
    engine_dir: &TestDir,
    receipts_dir: &TestDir,
    graph: &Graph,
    relative_path: &str,
    content: &str,
    endpoint_seed: u128,
) -> (
    ShardedHotEngine,
    PageId,
    ProjectionReceiptStore,
    ProjectionEndpointBinding,
) {
    let binding = projection_binding(graph, endpoint_seed);
    let store =
        ProjectionReceiptStore::open_for_endpoint(receipts_dir.path(), workspace(1), binding)
            .unwrap();
    let (engine, page_id) =
        authorized_engine(engine_dir, relative_path, content, Some((graph, &store)));
    (engine, page_id, store, binding)
}

fn plan(state: &ProjectionPageState, base: Option<&[u8]>) -> tine_core::oplog::ProjectionPlan {
    plan_projection(workspace(1), state, base).unwrap()
}

#[allow(clippy::type_complexity)]
fn manifested_fixture(
    label: &str,
    relative_path: &str,
    content: &str,
    endpoint_seed: u128,
) -> (
    TestDir,
    Graph,
    ProjectionReceiptStore,
    ObjectStore,
    ShardedHotEngine,
    ProjectionEndpointBinding,
    PageId,
    ProjectionIntent,
    Vec<u8>,
) {
    let dir = TestDir::new(label);
    let graph_root = dir.path().join("graph");
    fs::create_dir_all(graph_root.join("pages")).unwrap();
    let graph = Graph::open(&graph_root);
    let binding = projection_binding(&graph, endpoint_seed);
    let receipts = ProjectionReceiptStore::open_for_endpoint(
        &dir.path().join("receipts"),
        workspace(1),
        binding,
    )
    .unwrap();
    let initial_dir = TestDir::new(&format!("{label}-initial-authority"));
    let (initial, page_id) = authorized_engine(
        &initial_dir,
        relative_path,
        content,
        Some((&graph, &receipts)),
    );
    let (archive_seed, archive_page_id) = authorized_engine(&dir, relative_path, content, None);
    assert_eq!(archive_page_id, page_id);
    drop(archive_seed);
    let initial_write = write_projection_exact(&graph, &receipts, &initial, page_id, None).unwrap();
    let prior_intent = initial_write.plan.intent().clone();
    let base = initial_write.plan.target().to_vec();
    drop(initial);

    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, workspace(1)).unwrap();
    let reader = ObjectStore::open(&archive_path, workspace(1)).unwrap();
    let mut engine = ShardedHotEngine::with_enrolled_projection(
        reader,
        LineageDigest::of(b"projection-test-lineage"),
        DocumentId::from_uuid(uuid(700)),
        &graph,
        &receipts,
    );
    engine
        .stage_archive_batch(BatchId::from_uuid(uuid(704)))
        .unwrap();
    (
        dir,
        graph,
        receipts,
        writer,
        engine,
        binding,
        page_id,
        prior_intent,
        base,
    )
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

#[test]
fn hierarchy_order_annotations_and_bytes_are_deterministic() {
    let blocks = vec![
        block(4, Some(1), "b", "child b", None),
        block(2, None, "b", "root b", None),
        block(1, None, "a", "root a", None),
        block(3, Some(1), "a", "child a", None),
    ];
    let expected = "- root a\n\t- child a\n\t- child b\n- root b\n";
    let first = plan(&page("pages/tree.md", blocks.clone()), None);
    let mut reversed = blocks;
    reversed.reverse();
    let second = plan(&page("pages/tree.md", reversed), None);

    assert_eq!(text(first.target()), expected);
    assert_eq!(first.target(), second.target());
    assert_eq!(
        first.intent().encode().unwrap(),
        second.intent().encode().unwrap()
    );

    let annotations = first.intent().annotations();
    assert_eq!(annotations.len(), 4);
    assert_eq!(annotations[0].locator().components(), &[0]);
    assert_eq!(annotations[1].locator().components(), &[0, 0]);
    assert_eq!(annotations[2].locator().components(), &[0, 1]);
    assert_eq!(annotations[3].locator().components(), &[1]);
    for annotation in annotations {
        let span = annotation.span();
        let rendered = &first.target()[span.start() as usize..span.end() as usize];
        assert!(rendered.starts_with(b"- ") || rendered.windows(2).any(|pair| pair == b"- "));
        assert!(!rendered.is_empty());
    }
}

#[test]
fn malformed_hierarchies_fail_closed() {
    let missing = page(
        "pages/missing.md",
        vec![block(1, Some(99), "a", "orphan", None)],
    );
    assert!(matches!(
        plan_projection(workspace(1), &missing, None,),
        Err(ProjectionError::MissingParent { .. })
    ));

    let cycle = page(
        "pages/cycle.md",
        vec![
            block(1, Some(2), "a", "one", None),
            block(2, Some(1), "a", "two", None),
        ],
    );
    assert!(matches!(
        plan_projection(workspace(1), &cycle, None,),
        Err(ProjectionError::CyclicTree(_))
    ));

    let duplicate_order = page(
        "pages/order.md",
        vec![
            block(1, None, "same", "one", None),
            block(2, None, "same", "two", None),
        ],
    );
    assert!(matches!(
        plan_projection(workspace(1), &duplicate_order, None,),
        Err(ProjectionError::DuplicateSiblingOrder { .. })
    ));
}

#[test]
fn sparse_address_state_covers_reference_embed_and_export_deep_link() {
    let reasons = [
        (1, "reference", PolicyGeneratedAnchorReason::BlockReference),
        (2, "embed", PolicyGeneratedAnchorReason::BlockEmbed),
        (3, "export deep link", PolicyGeneratedAnchorReason::Export),
        (
            4,
            "copied deep link",
            PolicyGeneratedAnchorReason::CopiedDeepLink,
        ),
    ];
    let blocks = reasons
        .iter()
        .enumerate()
        .map(|(index, (value, label, reason))| {
            generated_block(
                *value,
                None,
                &format!("{index}"),
                *label,
                logseq(100 + value),
                *reason,
            )
        })
        .collect();
    let result = plan(&page("pages/addressable.md", blocks), None);
    for (value, label, _) in reasons {
        assert!(text(result.target()).contains(label));
        assert!(
            text(result.target()).contains(&format!("id:: {}", logseq(100 + value))),
            "{label} did not receive its addressable UUID"
        );
    }
    assert_eq!(result.generated_anchors().len(), 4);
}

#[test]
fn id_removal_change_existing_invalid_and_duplicate_raw_text_are_preserved() {
    let changed = logseq(200);
    let duplicate = logseq(201);
    let state = page(
        "pages/raw-ids.md",
        vec![
            block(1, None, "a", "removed", None),
            block(
                2,
                None,
                "b",
                format!("changed\nid:: {changed}"),
                Some(changed),
            ),
            block(3, None, "c", "invalid\nid:: definitely-not-a-uuid", None),
            block(
                4,
                None,
                "d",
                format!("duplicate one\nid:: {duplicate}"),
                None,
            ),
            block(
                5,
                None,
                "e",
                format!("duplicate two\nid:: {duplicate}"),
                None,
            ),
        ],
    );
    let result = plan(&state, None);
    let rendered = text(result.target());
    assert_eq!(rendered.matches("id::").count(), 4);
    assert!(rendered.contains("id:: definitely-not-a-uuid"));
    assert_eq!(rendered.matches(&duplicate.to_string()).count(), 2);
    assert!(result.generated_anchors().is_empty());

    let annotations = result.intent().annotations();
    assert_eq!(annotations[0].logseq_uuid(), None);
    assert_eq!(annotations[1].logseq_uuid(), Some(changed));
    assert_eq!(annotations[2].logseq_uuid(), None);
    assert_eq!(annotations[3].logseq_uuid(), None);
    assert_eq!(annotations[4].logseq_uuid(), None);
}

#[test]
fn inconsistent_duplicate_logseq_authority_is_rejected_without_cleaning_bytes() {
    let duplicate = logseq(300);
    let state = page(
        "pages/ambiguous.md",
        vec![
            block(
                1,
                None,
                "a",
                format!("one\nid:: {duplicate}"),
                Some(duplicate),
            ),
            block(2, None, "b", format!("two\nid:: {duplicate}"), None),
        ],
    );
    assert!(matches!(
        plan_projection(
            workspace(1),
            &state,
            None,
        ),
        Err(ProjectionError::AmbiguousRawLogseqId(id)) if id == duplicate
    ));
}

#[test]
fn external_identity_without_parser_confirmed_raw_property_fails_closed() {
    let external = logseq(450);
    for content in [
        format!("outside\n```\nid:: {external}\n```"),
        format!("outside `{external}`\n`id:: {external}`"),
    ] {
        let state = page(
            "pages/external.md",
            vec![block(1, None, "a", content, Some(external))],
        );
        assert!(matches!(
            plan_projection(
                workspace(1),
                &state,
                None,
            ),
            Err(ProjectionError::MissingExternalRawLogseqId {
                block: _,
                logseq_uuid
            }) if logseq_uuid == external
        ));
    }
}

#[test]
fn authoritative_preamble_and_structure_do_not_come_from_base() {
    let mut state = page(
        "pages/preamble.md",
        vec![
            block(1, None, "a", "root\nbody", None),
            block(2, Some(1), "a", "child", None),
        ],
    );
    state.page.preamble = Some("title:: Authoritative\nfree text".into());
    let result = plan(&state, Some(b"title:: stale base\n\n- old\n"));
    assert_eq!(
        text(result.target()),
        "title:: Authoritative\nfree text\n\n- root\n  body\n\t- child\n"
    );
    let parsed = tine_core::doc::parse(text(result.target()));
    assert_eq!(
        parsed.pre_block.as_deref(),
        Some("title:: Authoritative\nfree text")
    );
    assert_eq!(parsed.roots[0].raw, "root\nbody");
    assert_eq!(parsed.roots[0].children[0].raw, "child");
}

#[test]
fn untouched_supported_projection_remains_byte_identical() {
    let base = b"title:: Exact\r\n\r\n- root\r\n  body\r\n  - child\r\n";
    let mut state = page(
        "pages/untouched.md",
        vec![
            block(1, None, "a", "root\nbody", None),
            block(2, Some(1), "a", "child", None),
        ],
    );
    state.page.preamble = Some("title:: Exact".into());
    let result = plan(&state, Some(base));
    assert_eq!(result.target(), base);
}

#[test]
fn org_parser_distinguishes_real_mixed_case_id_from_source_and_example_blocks() {
    let existing = logseq(460);
    let generated = logseq(461);
    let state = page(
        "pages/org-parser.org",
        vec![
            block(
                1,
                None,
                "a",
                format!("real\n:properties:\n:Id: {existing}\n:end:"),
                Some(existing),
            ),
            generated_block(
                2,
                None,
                "b",
                format!(
                    "literal\n#+BEGIN_SRC text\n:PROPERTIES:\n:ID: {generated}\n:END:\n#+END_SRC\n#+BEGIN_EXAMPLE\n:ID: {generated}\n#+END_EXAMPLE"
                ),
                generated,
                PolicyGeneratedAnchorReason::BlockReference,
            ),
        ],
    );
    let result = plan(&state, None);
    let parsed = tine_core::org::parse_org(text(result.target()));
    let existing_text = existing.to_string();
    let generated_text = generated.to_string();
    assert_eq!(
        parsed.roots[0].property("id").as_deref(),
        Some(existing_text.as_str())
    );
    assert_eq!(
        parsed.roots[1].property("id").as_deref(),
        Some(generated_text.as_str())
    );
    assert_eq!(text(result.target()).matches(&generated_text).count(), 3);
}

#[test]
fn markdown_org_and_crlf_use_one_projection_path() {
    let id = logseq(500);
    let markdown = plan(
        &page(
            "pages/format.md",
            vec![
                block(1, None, "a", "root", None),
                generated_block(
                    2,
                    Some(1),
                    "a",
                    "child",
                    id,
                    PolicyGeneratedAnchorReason::BlockEmbed,
                ),
            ],
        ),
        Some(b"- old\r\n  - child\r\n"),
    );
    assert_eq!(
        text(markdown.target()),
        format!("- root\r\n  - child\r\n    id:: {id}\r\n")
    );

    let org = plan(
        &page(
            "pages/format.org",
            vec![generated_block(
                1,
                None,
                "a",
                "TODO title\nSCHEDULED: <2026-07-23 Thu>\nbody",
                id,
                PolicyGeneratedAnchorReason::Export,
            )],
        ),
        Some(b"* old\r\n"),
    );
    assert_eq!(
        text(org.target()),
        format!(
            "* TODO title\r\nSCHEDULED: <2026-07-23 Thu>\r\n:PROPERTIES:\r\n:id: {id}\r\n:END:\r\nbody\r\n"
        )
    );
}

#[test]
fn org_existing_and_invalid_id_drawer_text_is_preserved() {
    let existing = logseq(600);
    let generated = logseq(601);
    let state = page(
        "pages/org-ids.org",
        vec![
            block(
                1,
                None,
                "a",
                format!("existing\n:PROPERTIES:\n:id: {existing}\n:custom: keep\n:END:"),
                Some(existing),
            ),
            generated_block(
                2,
                None,
                "b",
                "invalid\n:PROPERTIES:\n:id: invalid-raw\n:custom: keep-too\n:END:",
                generated,
                PolicyGeneratedAnchorReason::CopiedDeepLink,
            ),
        ],
    );
    let result = plan(&state, None);
    let rendered = text(result.target());
    assert_eq!(rendered.matches(&existing.to_string()).count(), 1);
    assert!(rendered.contains(":id: invalid-raw"));
    assert!(rendered.contains(&format!(":id: {generated}")));
    assert!(rendered.contains(":id: invalid-raw\n:custom: keep-too"));
}

#[test]
fn receipt_store_orders_base_before_intent_and_enumerates_incomplete() {
    let dir = TestDir::new("store-order");
    let state = page("pages/store.md", vec![block(1, None, "a", "target", None)]);
    let projection = plan(&state, Some(b"- base\n"));
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let intent_id = store
        .publish_intent(projection.intent(), Some(b"- base\n"))
        .unwrap();
    let base = match projection.intent().precondition() {
        ProjectionPrecondition::Base(description) => description,
        ProjectionPrecondition::Absent => panic!("expected base"),
    };
    assert!(dir
        .path()
        .join("bases")
        .join(format!("{}.base", hex(base.sha256())))
        .is_file());
    assert!(dir
        .path()
        .join("intents")
        .join(format!("{}.intent", hex(intent_id.as_bytes())))
        .is_file());
    let completion_path = dir
        .path()
        .join("completions")
        .join(format!("{}.completion", hex(intent_id.as_bytes())));
    assert!(!completion_path.exists());
    assert_eq!(
        store.incomplete_intents().unwrap(),
        vec![projection.intent().clone()]
    );
}

#[test]
fn receipt_store_requires_exact_base_presence_and_descriptor_match() {
    let dir = TestDir::new("base-consistency");
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let absent = plan(
        &page(
            "archive/pages/absent.md",
            vec![block(1, None, "a", "target", None)],
        ),
        None,
    );
    assert!(matches!(
        store.publish_intent(absent.intent(), Some(b"unexpected")),
        Err(ProjectionStoreError::UnexpectedBase)
    ));

    let based = plan(
        &page(
            "archive/pages/based.md",
            vec![block(1, None, "a", "target", None)],
        ),
        Some(b"- exact base\n"),
    );
    let description = match based.intent().precondition() {
        ProjectionPrecondition::Base(description) => *description,
        ProjectionPrecondition::Absent => panic!("expected base"),
    };
    assert!(matches!(
        store.publish_intent(based.intent(), None),
        Err(ProjectionStoreError::MissingBase(found)) if found == description
    ));
    assert!(matches!(
        store.publish_intent(based.intent(), Some(b"- wrong base\n")),
        Err(ProjectionStoreError::BaseEvidenceMismatch(found)) if found == description
    ));
    assert!(fs::read_dir(dir.path().join("bases"))
        .unwrap()
        .next()
        .is_none());
    assert!(fs::read_dir(dir.path().join("intents"))
        .unwrap()
        .next()
        .is_none());

    store
        .publish_intent(based.intent(), Some(b"- exact base\n"))
        .unwrap();
    assert_eq!(
        store.load_base(based.intent()).unwrap().unwrap().bytes(),
        b"- exact base\n"
    );
    assert!(store.load_base(absent.intent()).unwrap().is_none());
}

#[test]
fn declared_oversized_target_is_rejected_before_any_evidence_publication() {
    const LIMIT: u64 = 64 * 1024 * 1024;

    let dir = TestDir::new("declared-oversized-target");
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let intent = ProjectionIntent::new(
        workspace(1),
        PageId::from_uuid(uuid(900)),
        ManagedPath::parse("pages/oversized.md").unwrap(),
        FrontierV2::default(),
        Vec::new(),
        ProjectionPrecondition::Absent,
        BlobDescription::from_parts([0; 32], LIMIT + 1),
        Vec::new(),
    )
    .unwrap();

    assert!(matches!(
        store.publish_intent(&intent, None),
        Err(ProjectionStoreError::EvidenceTooLarge {
            kind: "projection target",
            declared,
            limit: LIMIT,
        }) if declared == LIMIT + 1
    ));
    assert!(fs::read_dir(dir.path().join("bases"))
        .unwrap()
        .next()
        .is_none());
    assert!(fs::read_dir(dir.path().join("intents"))
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn incomplete_enumeration_skips_only_canonical_publication_temps() {
    let dir = TestDir::new("enumeration-names");
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let state = page(
        "pages/enumeration.md",
        vec![block(1, None, "a", "target", None)],
    );
    let projection = plan(&state, None);
    store.publish_intent(projection.intent(), None).unwrap();
    fs::write(
        dir.path()
            .join("intents")
            .join(format!(".tmp-{}", Uuid::new_v4())),
        b"crash residue",
    )
    .unwrap();
    assert_eq!(store.incomplete_intents().unwrap().len(), 1);

    fs::write(
        dir.path().join("intents").join(".tmp-not-canonical"),
        b"malformed residue",
    )
    .unwrap();
    assert!(matches!(
        store.incomplete_intents(),
        Err(ProjectionStoreError::MalformedEvidenceName(name))
            if name == ".tmp-not-canonical"
    ));
}

#[cfg(unix)]
#[test]
fn base_survives_when_intent_namespace_fails_before_publication() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new("base-first");
    let projection = plan(
        &page(
            "pages/base-first.md",
            vec![block(1, None, "a", "target", None)],
        ),
        Some(b"- base\n"),
    );
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    fs::remove_dir(dir.path().join("intents")).unwrap();
    symlink(dir.path().join("bases"), dir.path().join("intents")).unwrap();
    assert!(store
        .publish_intent(projection.intent(), Some(b"- base\n"))
        .is_err());

    let base = match projection.intent().precondition() {
        ProjectionPrecondition::Base(description) => description,
        ProjectionPrecondition::Absent => panic!("expected base"),
    };
    assert!(dir
        .path()
        .join("bases")
        .join(format!("{}.base", hex(base.sha256())))
        .is_file());
    assert!(!dir
        .path()
        .join("bases")
        .join(format!(
            "{}.intent",
            hex(projection.intent().id().unwrap().as_bytes())
        ))
        .exists());
}

#[test]
fn corrupt_missing_noncanonical_and_unknown_evidence_fail_closed() {
    let dir = TestDir::new("corrupt");
    let projection = plan(
        &page(
            "pages/corrupt.md",
            vec![block(1, None, "a", "target", None)],
        ),
        Some(b"- base\n"),
    );
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let intent_id = store
        .publish_intent(projection.intent(), Some(b"- base\n"))
        .unwrap();
    let base = match projection.intent().precondition() {
        ProjectionPrecondition::Base(description) => description,
        ProjectionPrecondition::Absent => panic!("expected base"),
    };
    let base_path = dir
        .path()
        .join("bases")
        .join(format!("{}.base", hex(base.sha256())));
    fs::remove_file(&base_path).unwrap();
    assert!(matches!(
        store.load_intent(intent_id),
        Err(ProjectionStoreError::MissingBase(_))
    ));
    fs::write(&base_path, b"- base\n").unwrap();
    fs::write(&base_path, b"- evil\n").unwrap();
    assert!(matches!(
        store.load_intent(intent_id),
        Err(ProjectionStoreError::BaseEvidenceMismatch(_))
    ));
    fs::write(&base_path, b"- base\n").unwrap();

    let intent_path = dir
        .path()
        .join("intents")
        .join(format!("{}.intent", hex(intent_id.as_bytes())));
    let canonical = fs::read(&intent_path).unwrap();
    let future = String::from_utf8(canonical.clone()).unwrap().replacen(
        "\"receipt_schema_version\":5",
        "\"receipt_schema_version\":99",
        1,
    );
    fs::write(&intent_path, future).unwrap();
    assert!(matches!(
        store.load_intent(intent_id),
        Err(ProjectionStoreError::Receipt(error))
            if error.to_string().contains("unknown receipt schema 99")
    ));

    let mut noncanonical = canonical.clone();
    noncanonical.push(b'\n');
    fs::write(&intent_path, noncanonical).unwrap();
    assert!(matches!(
        store.load_intent(intent_id),
        Err(ProjectionStoreError::NonCanonical("projection intent"))
    ));
    let collision = store
        .publish_intent(projection.intent(), Some(b"- base\n"))
        .unwrap_err();
    assert!(matches!(
        collision,
        ProjectionStoreError::Store(error)
            if matches!(
                error.as_ref(),
                StoreError::ImmutableCollision("projection intent")
            )
    ));
    fs::write(&intent_path, b"{").unwrap();
    assert!(store.load_intent(intent_id).is_err());

    let claim_dir = TestDir::new("future-claim");
    let mut claim = Vec::new();
    claim.extend_from_slice(b"TINEPR3\0");
    claim.extend_from_slice(&99_u32.to_be_bytes());
    claim.extend_from_slice(workspace(1).as_uuid().as_bytes());
    claim.extend_from_slice(&[0_u8; 1 + 16 + 16 + 32]);
    fs::write(claim_dir.path().join("projection-receipts.claim"), claim).unwrap();
    assert!(matches!(
        ProjectionReceiptStore::open(claim_dir.path(), workspace(1)),
        Err(ProjectionStoreError::UnknownStoreVersion(99))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_and_special_file_evidence_fail_closed() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new("unsafe-evidence");
    let projection = plan(
        &page("pages/unsafe.md", vec![block(1, None, "a", "target", None)]),
        None,
    );
    let store = ProjectionReceiptStore::open(dir.path(), workspace(1)).unwrap();
    let intent_id = store.publish_intent(projection.intent(), None).unwrap();
    let intent_path = dir
        .path()
        .join("intents")
        .join(format!("{}.intent", hex(intent_id.as_bytes())));
    let outside = dir.path().join("outside");
    fs::write(&outside, projection.intent().encode().unwrap()).unwrap();
    fs::remove_file(&intent_path).unwrap();
    symlink(&outside, &intent_path).unwrap();
    assert!(store.load_intent(intent_id).is_err());

    fs::remove_file(&intent_path).unwrap();
    let path = std::ffi::CString::new(intent_path.as_os_str().as_encoded_bytes()).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0);
    assert!(store.load_intent(intent_id).is_err());
    assert!(store.incomplete_intents().is_err());
}

#[test]
fn graph_bridge_refuses_absent_and_base_conflicts_without_completion() {
    let engine_dir = TestDir::new("graph-conflict-engine");
    let graph_dir = TestDir::new("graph-conflicts");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("graph-conflict-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/conflict.md",
        "target",
        60_000,
    );
    let state = engine.materialize_page_for_projection(page_id).unwrap();

    fs::write(graph_dir.path().join("pages/conflict.md"), b"- external\n").unwrap();
    let absent_plan = plan(&state, None);
    assert!(write_projection_exact(&graph, &store, &engine, page_id, None,).is_err());
    assert!(store
        .load_completion(absent_plan.intent())
        .unwrap()
        .is_none());

    let expected = b"- stale base\n";
    let base_plan = plan(&state, Some(expected));
    assert!(write_projection_exact(&graph, &store, &engine, page_id, Some(expected),).is_err());
    assert!(store.load_completion(base_plan.intent()).unwrap().is_none());
    assert_eq!(
        fs::read(graph_dir.path().join("pages/conflict.md")).unwrap(),
        b"- external\n"
    );
}

#[test]
fn graph_bridge_publishes_completion_only_after_exact_reread() {
    let engine_dir = TestDir::new("graph-success-engine");
    let graph_dir = TestDir::new("graph-success");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("graph-success-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/success.md",
        "landed",
        60_010,
    );
    let written = write_projection_exact(&graph, &store, &engine, page_id, None).unwrap();
    assert_eq!(
        fs::read(graph_dir.path().join("pages/success.md")).unwrap(),
        written.plan.target()
    );
    assert!(store
        .load_completion(written.plan.intent())
        .unwrap()
        .is_some());
}

#[test]
fn normal_replacement_catalogs_local_evidence_outside_stable_completion() {
    let engine_dir = TestDir::new("replacement-engine");
    let graph_dir = TestDir::new("replacement-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let base = b"- external base\n";
    fs::write(graph_dir.path().join("pages/replacement.md"), base).unwrap();
    let receipts_dir = TestDir::new("replacement-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/replacement.md",
        "projected",
        60_020,
    );

    let written = write_projection_exact(&graph, &store, &engine, page_id, Some(base)).unwrap();
    let evidence = store
        .local_forensic_evidence(written.plan.intent())
        .unwrap();
    let displacement = evidence
        .first()
        .expect("replacement completion must catalog the retained inode");
    assert_eq!(evidence.len(), 1);
    assert_eq!(displacement.observed(), BlobDescription::of(base));
    assert!(displacement
        .recovery_relative_path()
        .starts_with("pages/.replacement.md."));
    assert!(displacement
        .recovery_filename()
        .ends_with(".projection.recovery"));
    assert_eq!(
        fs::read(graph_dir.path().join(displacement.recovery_relative_path())).unwrap(),
        base
    );
    let completion_json: Value =
        serde_json::from_slice(&written.completion.encode().unwrap()).unwrap();
    assert!(completion_json.get("displacements").is_none());
    assert_eq!(
        store.load_completion(written.plan.intent()).unwrap(),
        Some(written.completion)
    );
}

#[test]
fn crash_recovery_probes_only_reserved_names_and_catalogs_multiple_attempts() {
    let engine_dir = TestDir::new("multiple-recovery-engine");
    let graph_dir = TestDir::new("multiple-recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("multiple-recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/multiple-recovery.md",
        "landed",
        60_030,
    );
    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let plan = plan_projection(workspace(1), authorization.state(), None).unwrap();
    store.publish_intent(plan.intent(), None).unwrap();
    let first = store.reserve_attempt(plan.intent()).unwrap();
    let second = store.reserve_attempt(plan.intent()).unwrap();
    fs::write(
        graph_dir.path().join("pages/multiple-recovery.md"),
        plan.target(),
    )
    .unwrap();
    for (name, bytes) in [
        (
            second.recovery_filename(),
            b"- second displaced\n".as_slice(),
        ),
        (first.recovery_filename(), b"- first displaced\n".as_slice()),
    ] {
        fs::write(graph_dir.path().join("pages").join(name), bytes).unwrap();
    }

    let recovered = recover_incomplete_projections(&graph, &store, &engine).unwrap();
    let evidence = store
        .local_forensic_evidence(recovered[0].plan.intent())
        .unwrap();
    assert_eq!(evidence.len(), 2);
    assert!(evidence
        .iter()
        .any(|record| record.recovery_filename() == first.recovery_filename()));
    assert!(evidence
        .iter()
        .any(|record| record.recovery_filename() == second.recovery_filename()));
}

#[test]
fn unrelated_generated_looking_sibling_is_never_scanned_and_is_preserved() {
    let engine_dir = TestDir::new("malformed-recovery-engine");
    let graph_dir = TestDir::new("malformed-recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let candidate = graph_dir
        .path()
        .join("pages/.malformed-recovery.md.forged.projection.recovery");
    fs::write(&candidate, b"- preserve me\n").unwrap();
    let receipts_dir = TestDir::new("malformed-recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/malformed-recovery.md",
        "landed",
        60_040,
    );

    write_projection_exact(&graph, &store, &engine, page_id, None).unwrap();
    assert_eq!(fs::read(&candidate).unwrap(), b"- preserve me\n");
    assert!(store.incomplete_intents().unwrap().is_empty());
    assert_eq!(
        fs::read(graph_dir.path().join("pages/malformed-recovery.md")).unwrap(),
        b"- landed\n"
    );
}

#[test]
fn missing_engine_authorization_publishes_nothing_and_writes_nothing() {
    let graph_dir = TestDir::new("unauthorized-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("unauthorized-receipts");
    let graph = Graph::open(graph_dir.path());
    let store = ProjectionReceiptStore::open(receipts_dir.path(), workspace(1)).unwrap();
    let engine = ShardedHotEngine::new(
        workspace(1),
        LineageDigest::of(b"unauthorized"),
        DocumentId::from_uuid(uuid(800)),
    );

    assert!(
        write_projection_exact(&graph, &store, &engine, PageId::from_uuid(uuid(801)), None,)
            .is_err()
    );
    assert!(store.incomplete_intents().unwrap().is_empty());
    assert!(!graph_dir.path().join("pages/unauthorized.md").exists());
}

#[test]
fn fail_before_legacy_sparse_write_and_recovery_cannot_cross_graph_roots() {
    let engine_dir = TestDir::new("legacy-cross-root-engine");
    let receipts_dir = TestDir::new("legacy-cross-root-receipts");
    let enrolled_graph_dir = TestDir::new("legacy-enrolled-graph");
    let foreign_graph_dir = TestDir::new("legacy-foreign-graph");
    for graph_dir in [&enrolled_graph_dir, &foreign_graph_dir] {
        fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
        fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    }
    let enrolled_graph = Graph::open(enrolled_graph_dir.path());
    let foreign_graph = Graph::open(foreign_graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &enrolled_graph,
        "pages/bound.md",
        "target",
        60_045,
    );

    assert!(matches!(
        write_projection_exact(&foreign_graph, &store, &engine, page_id, None),
        Err(ProjectionError::EndpointBindingMismatch)
    ));
    assert!(store.incomplete_intents().unwrap().is_empty());
    assert!(!foreign_graph_dir.path().join("pages/bound.md").exists());

    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let plan = plan_projection(workspace(1), authorization.state(), None).unwrap();
    store.publish_intent(plan.intent(), None).unwrap();
    assert!(matches!(
        recover_incomplete_projections(&foreign_graph, &store, &engine),
        Err(ProjectionError::EndpointBindingMismatch)
    ));
    assert_eq!(
        store.incomplete_intents().unwrap(),
        vec![plan.intent().clone()]
    );
    assert!(!foreign_graph_dir.path().join("pages/bound.md").exists());
    assert!(!enrolled_graph_dir.path().join("pages/bound.md").exists());
}

#[test]
fn receipts_allow_custom_nested_roots_but_graph_authorizes_only_configured_roots() {
    let configured_engine_dir = TestDir::new("configured-root-engine");
    let graph_dir = TestDir::new("configured-root-graph");
    fs::create_dir_all(graph_dir.path().join("logseq")).unwrap();
    fs::write(
        graph_dir.path().join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\"\n\
          :journals-directory \"archive/journals\"}\n",
    )
    .unwrap();
    let receipts_dir = TestDir::new("configured-root-receipts");
    let graph = Graph::open(graph_dir.path());
    let binding = projection_binding(&graph, 60_060);
    let store =
        ProjectionReceiptStore::open_for_endpoint(receipts_dir.path(), workspace(1), binding)
            .unwrap();
    let (configured_engine, configured_page) = authorized_engine(
        &configured_engine_dir,
        "archive/pages/topic.md",
        "configured",
        Some((&graph, &store)),
    );

    let written =
        write_projection_exact(&graph, &store, &configured_engine, configured_page, None).unwrap();
    assert_eq!(
        written.plan.intent().path().as_str(),
        "archive/pages/topic.md"
    );
    assert_eq!(
        fs::read(graph_dir.path().join("archive/pages/topic.md")).unwrap(),
        written.plan.target()
    );

    let unconfigured_engine_dir = TestDir::new("unconfigured-root-engine");
    let (unconfigured_engine, unconfigured_page) = authorized_engine(
        &unconfigured_engine_dir,
        "pages/safe.md",
        "unconfigured",
        Some((&graph, &store)),
    );
    assert!(ManagedPath::parse("pages/safe.md").is_ok());
    assert!(write_projection_exact(
        &graph,
        &store,
        &unconfigured_engine,
        unconfigured_page,
        None,
    )
    .is_err());
    assert!(!graph_dir.path().join("pages/safe.md").exists());
}

#[test]
fn enrolled_engine_rejects_wrong_graph_before_creating_work_namespace() {
    let dir = TestDir::new("enrolled-runtime-gate");
    let enrolled_root = dir.path().join("enrolled-graph");
    let wrong_root = dir.path().join("wrong-graph");
    fs::create_dir_all(&enrolled_root).unwrap();
    fs::create_dir_all(&wrong_root).unwrap();
    let enrolled_graph = Graph::open(&enrolled_root);
    let wrong_graph = Graph::open(&wrong_root);
    let binding = projection_binding(&enrolled_graph, 60_070);
    let receipts = ProjectionReceiptStore::open_for_endpoint(
        &dir.path().join("receipts"),
        workspace(1),
        binding,
    )
    .unwrap();
    let archive_path = dir.path().join("archive");
    let lineage = LineageDigest::of(b"projection-test-lineage");
    let catalog = DocumentId::from_uuid(uuid(700));

    let rejected = ShardedHotEngine::with_enrolled_projection(
        ObjectStore::open(&archive_path, workspace(1)).unwrap(),
        lineage,
        catalog,
        &wrong_graph,
        &receipts,
    );
    assert!(rejected.projection_work_index().is_none());
    assert!(!archive_path.join("projection-work-index-v1").exists());
    drop(rejected);

    let enrolled = ShardedHotEngine::with_enrolled_projection(
        ObjectStore::open(&archive_path, workspace(1)).unwrap(),
        lineage,
        catalog,
        &enrolled_graph,
        &receipts,
    );
    assert!(enrolled.projection_work_index().is_some());
    assert!(archive_path
        .join("projection-work-index-v1")
        .join(binding.endpoint_id.to_string())
        .exists());
}

#[test]
fn incomplete_absent_intent_recovery_resumes_through_reserved_writer() {
    let engine_dir = TestDir::new("recovery-engine");
    let graph_dir = TestDir::new("recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/recovery.md",
        "landed",
        60_050,
    );
    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let plan = plan_projection(workspace(1), authorization.state(), None).unwrap();
    store.publish_intent(plan.intent(), None).unwrap();

    let recovered = recover_incomplete_projections(&graph, &store, &engine).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        fs::read(graph_dir.path().join("pages/recovery.md")).unwrap(),
        plan.target()
    );
    assert!(store.incomplete_intents().unwrap().is_empty());
    assert!(store
        .load_completion(recovered[0].plan.intent())
        .unwrap()
        .is_some());
}

#[test]
fn stable_completion_bytes_ignore_device_local_attempt_and_evidence_ids() {
    let first_engine_dir = TestDir::new("stable-completion-engine-a");
    let second_engine_dir = TestDir::new("stable-completion-engine-b");
    let base = b"- base\n";
    let first_graph_dir = TestDir::new("stable-completion-graph-a");
    let second_graph_dir = TestDir::new("stable-completion-graph-b");
    for graph_dir in [&first_graph_dir, &second_graph_dir] {
        fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
        fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
        fs::write(graph_dir.path().join("pages/stable-completion.md"), base).unwrap();
    }
    let first_receipts = TestDir::new("stable-completion-receipts-a");
    let second_receipts = TestDir::new("stable-completion-receipts-b");
    let first_graph = Graph::open(first_graph_dir.path());
    let second_graph = Graph::open(second_graph_dir.path());
    let (first_engine, first_page_id, first_store, _) = enrolled_engine_and_store(
        &first_engine_dir,
        &first_receipts,
        &first_graph,
        "pages/stable-completion.md",
        "landed",
        60_070,
    );
    let (second_engine, second_page_id, second_store, _) = enrolled_engine_and_store(
        &second_engine_dir,
        &second_receipts,
        &second_graph,
        "pages/stable-completion.md",
        "landed",
        60_080,
    );

    let first = write_projection_exact(
        &first_graph,
        &first_store,
        &first_engine,
        first_page_id,
        Some(base),
    )
    .unwrap();
    let second = write_projection_exact(
        &second_graph,
        &second_store,
        &second_engine,
        second_page_id,
        Some(base),
    )
    .unwrap();
    assert_eq!(
        first.completion.logical_completion_id(),
        second.completion.logical_completion_id()
    );
    assert_eq!(
        first.completion.encode().unwrap(),
        second.completion.encode().unwrap()
    );
    let first_local = first_store
        .local_forensic_evidence(first.plan.intent())
        .unwrap();
    let second_local = second_store
        .local_forensic_evidence(second.plan.intent())
        .unwrap();
    assert_ne!(first_local[0].attempt_id(), second_local[0].attempt_id());
}

#[test]
fn crash_after_retire_resumes_from_exact_reserved_evidence() {
    let engine_dir = TestDir::new("retired-crash-engine");
    let graph_dir = TestDir::new("retired-crash-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let base = b"- base\n";
    let live = graph_dir.path().join("pages/retired-crash.md");
    fs::write(&live, base).unwrap();
    let receipts_dir = TestDir::new("retired-crash-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/retired-crash.md",
        "landed",
        60_090,
    );
    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let plan = plan_projection(workspace(1), authorization.state(), Some(base)).unwrap();
    store.publish_intent(plan.intent(), Some(base)).unwrap();
    let retired_attempt = store.reserve_attempt(plan.intent()).unwrap();
    let retired = graph_dir
        .path()
        .join("pages")
        .join(retired_attempt.recovery_filename());
    fs::rename(&live, &retired).unwrap();

    let recovered = recover_incomplete_projections(&graph, &store, &engine).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(fs::read(&live).unwrap(), plan.target());
    assert_eq!(fs::read(&retired).unwrap(), base);
    assert!(store
        .local_forensic_evidence(plan.intent())
        .unwrap()
        .iter()
        .any(|record| record.attempt_id() == retired_attempt.attempt_id()));
}

#[test]
fn forged_dense_intent_wire_fails_closed_before_persistence_recovery() {
    let engine_dir = TestDir::new("dense-recovery-engine");
    let graph_dir = TestDir::new("dense-recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("dense-recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/dense-recovery.md",
        "landed",
        60_100,
    );
    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let sparse = plan_projection(workspace(1), authorization.state(), None).unwrap();
    let intent_id = sparse.intent().id().unwrap();
    let mut forged: Value = serde_json::from_slice(&sparse.intent().encode().unwrap()).unwrap();
    forged["policy"] = serde_json::json!("dense_logseq_ids");
    let forged = serde_json::to_vec(&forged).unwrap();
    assert!(ProjectionIntent::decode(&forged).is_err());
    fs::write(
        receipts_dir
            .path()
            .join("intents")
            .join(format!("{intent_id}.intent")),
        forged,
    )
    .unwrap();

    assert!(store.incomplete_intents().is_err());
    assert!(recover_incomplete_projections(&graph, &store, &engine).is_err());
    assert!(!graph_dir.path().join("pages/dense-recovery.md").exists());
}

#[test]
fn transport_ready_dense_looking_manifested_target_is_rejected_before_authority() {
    let (dir, graph, receipts, writer, mut engine, binding, _page_id, prior_intent, _) =
        manifested_fixture(
            "crafted-dense-looking-target",
            "pages/crafted-target.md",
            "before",
            60_105,
        );
    let batch_id = BatchId::from_uuid(uuid(60_106));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(60_107)),
                crdt_peer_id: CrdtPeerId::from_u64(60_108),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    let finalized = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/crafted-target.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    let original_intent_object = finalized
        .objects()
        .iter()
        .find(|object| object.kind() == ObjectKind::ProjectionIntent)
        .unwrap();
    let original_intent =
        ManifestedProjectionIntent::decode(original_intent_object.payload()).unwrap();
    assert_eq!(original_intent.target().bytes().unwrap(), b"- after\n");
    assert!(!text(original_intent.target().bytes().unwrap()).contains("id::"));
    assert_eq!(original_intent.target().annotations().len(), 1);

    let synthetic_dense_id = LogseqUuid::from_uuid(BlockId::from_uuid(uuid(703)).as_uuid());
    let mut crafted_target = original_intent.target().bytes().unwrap().to_vec();
    crafted_target.extend_from_slice(format!("  id:: {synthetic_dense_id}\n").as_bytes());
    let original_annotation = &original_intent.target().annotations()[0];
    let crafted_annotation = AnnotatedIdentity::new(
        original_annotation.locator().clone(),
        StructuralSpan::new(
            original_annotation.span().start(),
            crafted_target.len() as u64,
        )
        .unwrap(),
        original_annotation.block_id(),
        original_annotation.logseq_uuid(),
    );
    let crafted_intent = ManifestedProjectionIntent::new(
        original_intent.workspace_id(),
        original_intent.source_batch_id(),
        original_intent.source_author_device_id(),
        original_intent.source_author_session_id(),
        original_intent.source_endpoint_id(),
        original_intent.page_id(),
        original_intent.path().clone(),
        original_intent.portable_path_index_root(),
        original_intent.precondition().clone(),
        original_intent.render_base().cloned(),
        ManifestProjectionTarget::present(crafted_target.clone(), vec![crafted_annotation])
            .unwrap(),
        original_intent.post_frontier().clone(),
        original_intent.claim_evidence().to_vec(),
    )
    .unwrap();
    let crafted_intent_object = OperationObject::new(
        original_intent_object.workspace_id(),
        original_intent_object.document_id(),
        ObjectKind::ProjectionIntent,
        crafted_intent.encode().unwrap(),
    )
    .unwrap();
    let crafted_objects = finalized
        .objects()
        .iter()
        .map(|object| {
            if object.kind() == ObjectKind::ProjectionIntent {
                crafted_intent_object.clone()
            } else {
                object.clone()
            }
        })
        .collect::<Vec<_>>();
    let crafted_descriptors = crafted_objects
        .iter()
        .map(OperationObject::descriptor)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let source_manifest = finalized.manifest();
    let crafted_manifest = OperationBatch::new_with_causality(
        source_manifest.workspace_id(),
        source_manifest.lineage_digest(),
        source_manifest.batch_id(),
        source_manifest.author_device_id(),
        source_manifest.author_session_id(),
        source_manifest.origin(),
        source_manifest.causal_dot(),
        source_manifest.causal_dependency_heads().to_vec(),
        source_manifest.dependency_frontier().clone(),
        source_manifest.semantic_effect_digest(),
        crafted_descriptors,
    )
    .unwrap();
    let crafted = PreparedBatch::new(crafted_manifest, crafted_objects).unwrap();

    let path = ManagedPath::parse("pages/crafted-target.md").unwrap();
    let graph_path = dir.path().join("graph/pages/crafted-target.md");
    let graph_bytes_before = fs::read(&graph_path).unwrap();
    let graph_modified_before = fs::metadata(&graph_path).unwrap().modified().unwrap();
    let mut graph_entries_before = fs::read_dir(dir.path().join("graph/pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    graph_entries_before.sort_unstable();
    let mut receipt_intents_before = fs::read_dir(receipts.root_path().join("intents"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    receipt_intents_before.sort_unstable();
    let incomplete_intents_before = receipts
        .incomplete_intents()
        .unwrap()
        .into_iter()
        .map(|intent| intent.id().unwrap())
        .collect::<Vec<_>>();
    let accepted_before = engine.status().accepted_batch_ids().unwrap();
    assert!(!accepted_before.contains(&batch_id));
    assert!(engine
        .projection_work_index()
        .unwrap()
        .pending_for_path(&path)
        .unwrap()
        .is_empty());
    assert!(matches!(
        writer.inspect_batch(batch_id).unwrap(),
        BatchInspection::Absent
    ));

    for object in crafted.objects() {
        let descriptor = object.descriptor().unwrap();
        assert_eq!(
            writer
                .stage_object_bytes(&object.encode().unwrap())
                .unwrap(),
            descriptor.content_digest()
        );
    }
    assert!(matches!(
        writer.inspect_batch(batch_id).unwrap(),
        BatchInspection::Absent
    ));
    assert_eq!(
        writer
            .stage_manifest_bytes(&crafted.manifest().encode().unwrap())
            .unwrap(),
        batch_id
    );
    let retained = match writer.inspect_batch(batch_id).unwrap() {
        BatchInspection::Ready(retained) => retained,
        other => panic!("crafted batch was not transport-ready: {other:?}"),
    };
    let retained_intent = retained
        .objects()
        .iter()
        .find(|object| object.kind() == ObjectKind::ProjectionIntent)
        .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
        .unwrap();
    assert_eq!(
        retained_intent.target().bytes().unwrap(),
        crafted_target.as_slice()
    );
    assert!(text(retained_intent.target().bytes().unwrap())
        .contains(&format!("id:: {synthetic_dense_id}")));

    let outcome = engine.stage_archive_batch(batch_id).unwrap();
    assert!(matches!(
        outcome.disposition(),
        BatchDisposition::Rejected {
            error: EngineError::ProjectionManifest(reason)
        } if reason
            == "projection target bytes/annotations do not match semantic post-state"
    ));
    assert!(outcome.newly_accepted().is_empty());
    assert_eq!(
        outcome.status().accepted_batch_ids().unwrap(),
        accepted_before
    );
    assert_eq!(
        engine.status().accepted_batch_ids().unwrap(),
        accepted_before
    );
    assert!(engine
        .status()
        .offered_batch_ids()
        .unwrap()
        .contains(&batch_id));
    assert!(engine
        .projection_work_index()
        .unwrap()
        .pending_for_path(&path)
        .unwrap()
        .is_empty());
    assert!(engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .is_none());

    let mut receipt_intents_after = fs::read_dir(receipts.root_path().join("intents"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    receipt_intents_after.sort_unstable();
    assert_eq!(receipt_intents_after, receipt_intents_before);
    assert_eq!(
        receipts
            .incomplete_intents()
            .unwrap()
            .into_iter()
            .map(|intent| intent.id().unwrap())
            .collect::<Vec<_>>(),
        incomplete_intents_before
    );
    assert_eq!(fs::read(&graph_path).unwrap(), graph_bytes_before);
    assert_eq!(
        fs::metadata(&graph_path).unwrap().modified().unwrap(),
        graph_modified_before
    );
    let mut graph_entries_after = fs::read_dir(dir.path().join("graph/pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    graph_entries_after.sort_unstable();
    assert_eq!(graph_entries_after, graph_entries_before);
}

#[test]
fn recovery_replays_intent_frontier_after_engine_and_claim_index_advance() {
    let engine_dir = TestDir::new("advanced-recovery-engine");
    let graph_dir = TestDir::new("advanced-recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("advanced-recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (mut engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/advanced-recovery.md",
        "historical",
        60_110,
    );

    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let historical = plan_projection(workspace(1), authorization.state(), None).unwrap();
    store.publish_intent(historical.intent(), None).unwrap();
    fs::write(
        graph_dir.path().join("pages/advanced-recovery.md"),
        historical.target(),
    )
    .unwrap();

    let later_batch = BatchId::from_uuid(uuid(720));
    let later = engine
        .prepare_bootstrap_transaction(
            AuthorBatch {
                batch_id: later_batch,
                author_device_id: DeviceId::from_uuid(uuid(721)),
                author_session_id: SessionId::from_uuid(uuid(722)),
                crdt_peer_id: CrdtPeerId::from_u64(723),
            },
            &OperationTransaction::new(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                mutation: tine_core::oplog::LogseqIdentityMutation::AssignExternal {
                    logseq_uuid: logseq(724),
                },
            }])
            .unwrap(),
        )
        .unwrap();
    let writer = ObjectStore::open(&engine_dir.path().join("archive"), workspace(1)).unwrap();
    writer.publish_prepared(&later).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(later_batch).unwrap().disposition,
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        engine.materialize_page(page_id).unwrap().blocks[0].logseq_uuid,
        Some(logseq(724))
    );

    let recovered = recover_incomplete_projections(&graph, &store, &engine).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].plan.intent(), historical.intent());
    assert!(store.incomplete_intents().unwrap().is_empty());
}

#[test]
fn recovery_rejects_interleaved_external_edit_without_completion() {
    let engine_dir = TestDir::new("external-recovery-engine");
    let graph_dir = TestDir::new("external-recovery-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("external-recovery-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/external-recovery.md",
        "landed",
        60_120,
    );
    let authorization = engine.authorize_projection_write(page_id).unwrap();
    let plan = plan_projection(workspace(1), authorization.state(), None).unwrap();
    store.publish_intent(plan.intent(), None).unwrap();
    fs::write(
        graph_dir.path().join("pages/external-recovery.md"),
        b"- external edit\n",
    )
    .unwrap();

    assert!(recover_incomplete_projections(&graph, &store, &engine).is_err());
    assert_eq!(store.incomplete_intents().unwrap().len(), 1);
    assert_eq!(
        fs::read(graph_dir.path().join("pages/external-recovery.md")).unwrap(),
        b"- external edit\n"
    );
}

#[cfg(unix)]
#[test]
fn completion_publication_failure_leaves_recoverable_exact_target() {
    use std::os::unix::fs::symlink;

    let engine_dir = TestDir::new("completion-failure-engine");
    let graph_dir = TestDir::new("completion-failure-graph");
    fs::create_dir_all(graph_dir.path().join("pages")).unwrap();
    fs::create_dir_all(graph_dir.path().join("journals")).unwrap();
    let receipts_dir = TestDir::new("completion-failure-receipts");
    let graph = Graph::open(graph_dir.path());
    let (engine, page_id, store, _) = enrolled_engine_and_store(
        &engine_dir,
        &receipts_dir,
        &graph,
        "pages/completion-failure.md",
        "landed",
        60_130,
    );
    let base = b"- pre-crash base\n";
    fs::write(graph_dir.path().join("pages/completion-failure.md"), base).unwrap();

    fs::remove_dir(receipts_dir.path().join("completions")).unwrap();
    symlink(
        receipts_dir.path().join("intents"),
        receipts_dir.path().join("completions"),
    )
    .unwrap();
    assert!(write_projection_exact(&graph, &store, &engine, page_id, Some(base)).is_err());
    let target = fs::read(graph_dir.path().join("pages/completion-failure.md")).unwrap();
    assert_eq!(text(&target), "- landed\n");

    fs::remove_file(receipts_dir.path().join("completions")).unwrap();
    fs::create_dir(receipts_dir.path().join("completions")).unwrap();
    let recovered = recover_incomplete_projections(&graph, &store, &engine).unwrap();
    assert_eq!(recovered.len(), 1);
    let evidence = store
        .local_forensic_evidence(recovered[0].plan.intent())
        .unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].observed(), BlobDescription::of(base));
}

#[cfg(unix)]
#[test]
fn manifested_present_target_recovers_across_completion_and_status_publication_cuts() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, graph, receipts, writer, mut engine, binding, page_id, prior_intent, base) =
        manifested_fixture(
            "manifested-present-publication-cuts",
            "pages/cuts.md",
            "before",
            70_000,
        );
    let batch_id = BatchId::from_uuid(uuid(70_010));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(70_011)),
                crdt_peer_id: CrdtPeerId::from_u64(70_012),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/cuts.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let post = engine.materialize_page_for_projection(page_id).unwrap();
    let expected = plan_projection(workspace(1), &post, Some(&base)).unwrap();

    let completion_dir = receipts.root_path().join("completions");
    let completion_mode = fs::metadata(&completion_dir).unwrap().permissions().mode();
    fs::set_permissions(&completion_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let first = execute_manifested_projection_work(&graph, &receipts, &mut engine, &work);
    fs::set_permissions(&completion_dir, fs::Permissions::from_mode(completion_mode)).unwrap();
    assert!(first.is_err());
    assert_eq!(
        fs::read(dir.path().join("graph/pages/cuts.md")).unwrap(),
        expected.target()
    );
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Ready)
    );

    let work_dir = dir
        .path()
        .join("archive/projection-work-index-v1")
        .join(binding.endpoint_id.to_string());
    let work_mode = fs::metadata(&work_dir).unwrap().permissions().mode();
    fs::set_permissions(&work_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let second = execute_manifested_projection_work(&graph, &receipts, &mut engine, &work);
    fs::set_permissions(&work_dir, fs::Permissions::from_mode(work_mode)).unwrap();
    assert!(second.is_err());
    assert!(receipts
        .load_completion(expected.intent())
        .unwrap()
        .is_some());
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Ready)
    );

    execute_manifested_projection_work(&graph, &receipts, &mut engine, &work).unwrap();
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Completed)
    );
}

#[cfg(unix)]
#[test]
fn manifested_absence_recovers_from_retained_base_across_both_publication_cuts() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, graph, receipts, writer, mut engine, binding, page_id, prior_intent, _) =
        manifested_fixture(
            "manifested-absence-publication-cuts",
            "pages/delete-cuts.md",
            "before",
            71_000,
        );
    let batch_id = BatchId::from_uuid(uuid(71_010));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(71_011)),
                crdt_peer_id: CrdtPeerId::from_u64(71_012),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::DeletePage { page_id }]).unwrap(),
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/delete-cuts.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();

    let completion_dir = receipts.root_path().join("completions");
    let completion_mode = fs::metadata(&completion_dir).unwrap().permissions().mode();
    fs::set_permissions(&completion_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let first = execute_manifested_projection_work(&graph, &receipts, &mut engine, &work);
    fs::set_permissions(&completion_dir, fs::Permissions::from_mode(completion_mode)).unwrap();
    assert!(first.is_err());
    assert!(!dir.path().join("graph/pages/delete-cuts.md").exists());
    assert_eq!(receipts.incomplete_intents().unwrap().len(), 1);

    let work_dir = dir
        .path()
        .join("archive/projection-work-index-v1")
        .join(binding.endpoint_id.to_string());
    let work_mode = fs::metadata(&work_dir).unwrap().permissions().mode();
    fs::set_permissions(&work_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let second = execute_manifested_projection_work(&graph, &receipts, &mut engine, &work);
    fs::set_permissions(&work_dir, fs::Permissions::from_mode(work_mode)).unwrap();
    assert!(second.is_err());
    assert!(receipts.incomplete_intents().unwrap().is_empty());
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Ready)
    );

    execute_manifested_projection_work(&graph, &receipts, &mut engine, &work).unwrap();
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Completed)
    );
    assert!(!dir.path().join("graph/pages/delete-cuts.md").exists());
}

#[test]
fn rolled_back_work_head_cannot_resurrect_deletion_after_causal_identical_path_reuse() {
    let (dir, graph, receipts, writer, mut engine, binding, page_a, prior_intent, bytes) =
        manifested_fixture(
            "stale-delete-identical-reuse",
            "pages/reused.md",
            "same bytes",
            72_000,
        );
    let path = ManagedPath::parse("pages/reused.md").unwrap();
    let delete_batch = BatchId::from_uuid(uuid(72_010));
    let delete = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id: delete_batch,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(72_011)),
                crdt_peer_id: CrdtPeerId::from_u64(72_012),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::DeletePage { page_id: page_a }])
                .unwrap(),
        )
        .unwrap();
    let delete = engine
        .finalize_author_transaction(
            delete,
            binding,
            vec![receipts
                .capture_projection_input(&graph, binding, path.clone(), Some(&prior_intent))
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&delete).unwrap();
    assert!(matches!(
        engine
            .stage_archive_batch(delete_batch)
            .unwrap()
            .disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let delete_work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let work_head = dir
        .path()
        .join("archive/projection-work-index-v1")
        .join(binding.endpoint_id.to_string())
        .join("projection-work.head");
    let rollback_head = fs::read(&work_head).unwrap();
    execute_manifested_projection_work(&graph, &receipts, &mut engine, &delete_work).unwrap();

    let page_b = PageId::from_uuid(uuid(72_020));
    let home_b = DocumentId::from_uuid(uuid(72_021));
    let block_b = BlockId::from_uuid(uuid(72_022));
    let create_batch = BatchId::from_uuid(uuid(72_023));
    let create = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id: create_batch,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(72_024)),
                crdt_peer_id: CrdtPeerId::from_u64(72_025),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![
                SemanticOperation::CreatePage {
                    page_id: page_b,
                    home_document_id: home_b,
                    path: path.clone(),
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: block_b,
                        home_document_id: home_b,
                    },
                    page_id: page_b,
                    parent: None,
                    order: "a".into(),
                    content: "same bytes".into(),
                },
            ])
            .unwrap(),
        )
        .unwrap();
    let create = engine
        .finalize_author_transaction(
            create,
            binding,
            vec![receipts
                .capture_projection_input(&graph, binding, path.clone(), None)
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&create).unwrap();
    assert!(matches!(
        engine
            .stage_archive_batch(create_batch)
            .unwrap()
            .disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let create_work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    execute_manifested_projection_work(&graph, &receipts, &mut engine, &create_work).unwrap();
    assert_eq!(
        fs::read(dir.path().join("graph/pages/reused.md")).unwrap(),
        bytes
    );

    fs::write(&work_head, rollback_head).unwrap();
    let resurrected = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(resurrected.work_id(), delete_work.work_id());
    assert!(
        execute_manifested_projection_work(&graph, &receipts, &mut engine, &resurrected,).is_err()
    );
    assert_eq!(
        fs::read(dir.path().join("graph/pages/reused.md")).unwrap(),
        bytes
    );
}

#[test]
fn rolled_back_work_head_cannot_replay_stale_present_over_newer_projection() {
    let (dir, graph, receipts, writer, mut engine, binding, page_id, mut prior_intent, _) =
        manifested_fixture(
            "stale-present-overwrite",
            "pages/stale-present.md",
            "initial",
            73_000,
        );
    let path = ManagedPath::parse("pages/stale-present.md").unwrap();
    let mut stale_head = Vec::new();
    let mut stale_work = None;
    for (sequence, content) in [(0_u128, "stale"), (1, "newest")] {
        let batch_id = BatchId::from_uuid(uuid(73_010 + sequence));
        let current_bytes = fs::read(dir.path().join("graph/pages/stale-present.md")).unwrap();
        let draft = engine
            .draft_author_transaction(
                AuthorBatch {
                    batch_id,
                    author_device_id: binding.device_id,
                    author_session_id: SessionId::from_uuid(uuid(73_020 + sequence)),
                    crdt_peer_id: CrdtPeerId::from_u64(73_030 + sequence as u64),
                },
                BatchOrigin::LocalMutation,
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(uuid(703)),
                        home_document_id: DocumentId::from_uuid(uuid(702)),
                    },
                    content: content.into(),
                }])
                .unwrap(),
            )
            .unwrap();
        let prepared = engine
            .finalize_author_transaction(
                draft,
                binding,
                vec![receipts
                    .capture_projection_input(&graph, binding, path.clone(), Some(&prior_intent))
                    .unwrap()],
            )
            .unwrap();
        writer.publish_prepared(&prepared).unwrap();
        assert!(matches!(
            engine.stage_archive_batch(batch_id).unwrap().disposition(),
            tine_core::oplog::BatchDisposition::Accepted { .. }
        ));
        let work = engine
            .projection_work_index()
            .unwrap()
            .pending_for_path(&path)
            .unwrap()
            .into_iter()
            .find(|work| work.batch_id() == batch_id)
            .unwrap();
        if sequence == 0 {
            let work_head = dir
                .path()
                .join("archive/projection-work-index-v1")
                .join(binding.endpoint_id.to_string())
                .join("projection-work.head");
            stale_head = fs::read(work_head).unwrap();
            stale_work = Some(work.clone());
        }
        execute_manifested_projection_work(&graph, &receipts, &mut engine, &work).unwrap();
        let state = engine.materialize_page_for_projection(page_id).unwrap();
        prior_intent = plan_projection(workspace(1), &state, Some(&current_bytes))
            .unwrap()
            .intent()
            .clone();
    }
    let newest = fs::read(dir.path().join("graph/pages/stale-present.md")).unwrap();
    let work_head = dir
        .path()
        .join("archive/projection-work-index-v1")
        .join(binding.endpoint_id.to_string())
        .join("projection-work.head");
    fs::write(&work_head, stale_head).unwrap();
    assert!(execute_manifested_projection_work(
        &graph,
        &receipts,
        &mut engine,
        &stale_work.unwrap(),
    )
    .is_err());
    assert_eq!(
        fs::read(dir.path().join("graph/pages/stale-present.md")).unwrap(),
        newest
    );
}

#[test]
fn capability_capture_rejects_forged_cross_scope_and_byte_mismatched_predecessors() {
    let (dir, graph, receipts, _writer, _engine, binding, _page_id, prior_intent, base) =
        manifested_fixture(
            "capture-predecessor-scope",
            "pages/capture.md",
            "base",
            74_000,
        );
    let path = ManagedPath::parse("pages/capture.md").unwrap();
    let other_binding = ProjectionEndpointBinding {
        endpoint_id: ProjectionEndpointId::from_uuid(uuid(74_100)),
        device_id: binding.device_id,
        graph_resource_id: binding.graph_resource_id,
    };
    assert!(matches!(
        receipts
            .capture_projection_input(&graph, other_binding, path.clone(), Some(&prior_intent),),
        Err(ProjectionStoreError::EndpointBindingMismatch)
    ));

    let other_root = dir.path().join("other-graph");
    fs::create_dir_all(other_root.join("pages")).unwrap();
    fs::write(other_root.join("pages/capture.md"), &base).unwrap();
    let other_graph = Graph::open(&other_root);
    assert!(matches!(
        receipts
            .capture_projection_input(&other_graph, binding, path.clone(), Some(&prior_intent),),
        Err(ProjectionStoreError::GraphResourceMismatch)
    ));

    fs::write(
        dir.path().join("graph/pages/capture.md"),
        b"- external mismatch\n",
    )
    .unwrap();
    assert!(matches!(
        receipts.capture_projection_input(&graph, binding, path.clone(), Some(&prior_intent)),
        Err(ProjectionStoreError::CapturedInputMismatch)
    ));
    fs::write(dir.path().join("graph/pages/capture.md"), &base).unwrap();

    let other_path = ManagedPath::parse("pages/other.md").unwrap();
    fs::write(dir.path().join("graph/pages/other.md"), &base).unwrap();
    assert!(matches!(
        receipts.capture_projection_input(&graph, binding, other_path, Some(&prior_intent),),
        Err(ProjectionStoreError::CapturedInputMismatch)
    ));

    let foreign_workspace_store = ProjectionReceiptStore::open_for_endpoint(
        &dir.path().join("foreign-workspace-receipts"),
        workspace(2),
        binding,
    )
    .unwrap();
    assert!(matches!(
        foreign_workspace_store.capture_projection_input(
            &graph,
            binding,
            path.clone(),
            Some(&prior_intent),
        ),
        Err(ProjectionStoreError::CapturedInputMismatch)
    ));

    let completion_path = receipts.root_path().join("completions").join(format!(
        "{}.completion",
        hex(prior_intent.id().unwrap().as_bytes())
    ));
    let mut forged: Value = serde_json::from_slice(&fs::read(&completion_path).unwrap()).unwrap();
    forged["logical_completion_id"] = Value::String("00".repeat(32));
    fs::write(&completion_path, serde_json::to_vec(&forged).unwrap()).unwrap();
    assert!(receipts
        .capture_projection_input(&graph, binding, path, Some(&prior_intent))
        .is_err());
}

#[test]
fn manifested_work_for_one_enrolled_graph_cannot_mutate_same_bytes_on_another_root() {
    let (dir, graph, receipts, writer, mut engine, binding, _page_id, prior_intent, base) =
        manifested_fixture(
            "wrong-graph-execution",
            "pages/root-bound.md",
            "before",
            75_000,
        );
    let batch_id = BatchId::from_uuid(uuid(75_010));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(75_011)),
                crdt_peer_id: CrdtPeerId::from_u64(75_012),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/root-bound.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();

    let wrong_root = dir.path().join("wrong-root");
    fs::create_dir_all(wrong_root.join("pages")).unwrap();
    fs::write(wrong_root.join("pages/root-bound.md"), &base).unwrap();
    let wrong_graph = Graph::open(&wrong_root);
    assert!(matches!(
        execute_manifested_projection_work(&wrong_graph, &receipts, &mut engine, &work,),
        Err(ProjectionError::EndpointBindingMismatch)
    ));
    assert_eq!(
        fs::read(wrong_root.join("pages/root-bound.md")).unwrap(),
        base
    );
}

#[test]
fn manifested_guarded_conflict_is_the_proof_bearing_block_path() {
    let (dir, graph, receipts, writer, mut engine, binding, _page_id, prior_intent, _) =
        manifested_fixture(
            "manifested-guarded-block",
            "pages/blocked.md",
            "before",
            75_100,
        );
    let batch_id = BatchId::from_uuid(uuid(75_110));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(75_111)),
                crdt_peer_id: CrdtPeerId::from_u64(75_112),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/blocked.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let external = b"- external edit\n";
    fs::write(dir.path().join("graph/pages/blocked.md"), external).unwrap();

    assert!(execute_manifested_projection_work(&graph, &receipts, &mut engine, &work,).is_err());
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Blocked)
    );
    assert_eq!(
        fs::read(dir.path().join("graph/pages/blocked.md")).unwrap(),
        external
    );
    assert!(execute_manifested_projection_work(&graph, &receipts, &mut engine, &work,).is_err());
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Blocked)
    );
}

#[cfg(unix)]
#[test]
fn graph_resource_identity_survives_move_but_rejects_symlink_and_path_substitution() {
    use std::os::unix::fs::symlink;

    let (dir, graph, receipts, writer, mut engine, binding, _page_id, prior_intent, base) =
        manifested_fixture("graph-resource-move", "pages/moved.md", "before", 76_000);
    let original_root = dir.path().join("graph");
    let moved_root = dir.path().join("moved-graph");
    fs::rename(&original_root, &moved_root).unwrap();
    fs::create_dir_all(original_root.join("pages")).unwrap();
    fs::write(original_root.join("pages/moved.md"), &base).unwrap();
    let substituted = Graph::open(&original_root);
    assert_ne!(
        substituted.canonical_resource_id().unwrap(),
        binding.graph_resource_id
    );
    assert_eq!(
        graph.canonical_resource_id().unwrap(),
        binding.graph_resource_id
    );
    let reopened = Graph::open(&moved_root);
    assert_eq!(
        reopened.canonical_resource_id().unwrap(),
        binding.graph_resource_id
    );
    ProjectionReceiptStore::open_for_endpoint(receipts.root_path(), workspace(1), binding).unwrap();

    let symlink_root = dir.path().join("graph-link");
    symlink(&moved_root, &symlink_root).unwrap();
    assert!(Graph::open(&symlink_root).canonical_resource_id().is_err());

    let batch_id = BatchId::from_uuid(uuid(76_010));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(76_011)),
                crdt_peer_id: CrdtPeerId::from_u64(76_012),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after move".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/moved.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    writer.publish_prepared(&prepared).unwrap();
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    execute_manifested_projection_work(&graph, &receipts, &mut engine, &work).unwrap();
    assert_ne!(fs::read(moved_root.join("pages/moved.md")).unwrap(), base);
    assert_eq!(
        fs::read(original_root.join("pages/moved.md")).unwrap(),
        base
    );
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[test]
fn fail_before_projection_crash_windows_recover_without_unauthorized_execution() {
    let dir = TestDir::new("manifested-author-work");
    let workspace_id = workspace(1);
    let lineage = LineageDigest::of(b"projection-test-lineage");
    let catalog = DocumentId::from_uuid(uuid(700));
    let archive_path = dir.path().join("archive");
    let graph_root = dir.path().join("manifested-graph");
    fs::create_dir_all(graph_root.join("pages")).unwrap();
    let graph = Graph::open(&graph_root);
    let binding = projection_binding(&graph, 800);
    let source_endpoint = binding.endpoint_id;
    let source_device = binding.device_id;
    let receipts_root = dir.path().join("manifested-receipts");
    let receipts =
        ProjectionReceiptStore::open_for_endpoint(&receipts_root, workspace_id, binding).unwrap();
    let initial_dir = TestDir::new("manifested-author-work-initial-authority");
    let (initial, page_id) = authorized_engine(
        &initial_dir,
        "pages/manifested.md",
        "before",
        Some((&graph, &receipts)),
    );
    let (archive_seed, archive_page_id) =
        authorized_engine(&dir, "pages/manifested.md", "before", None);
    assert_eq!(archive_page_id, page_id);
    drop(archive_seed);
    let initial_write = write_projection_exact(&graph, &receipts, &initial, page_id, None).unwrap();
    let mut prior_intent = initial_write.plan.intent().clone();
    let mut current_projection_bytes = initial_write.plan.target().to_vec();
    drop(initial);
    let reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let mut engine =
        ShardedHotEngine::with_enrolled_projection(reader, lineage, catalog, &graph, &receipts);
    engine
        .stage_archive_batch(BatchId::from_uuid(uuid(704)))
        .unwrap();

    // Accumulate retired prepared files and authenticated roots first. Startup
    // recovery below must still touch only the one current pending activation.
    for sequence in 0_u128..24 {
        let historical_batch = BatchId::from_uuid(uuid(9_000 + sequence));
        let draft = engine
            .draft_author_transaction(
                AuthorBatch {
                    batch_id: historical_batch,
                    author_device_id: source_device,
                    author_session_id: SessionId::from_uuid(uuid(10_000 + sequence)),
                    crdt_peer_id: CrdtPeerId::from_u64(11_000 + sequence as u64),
                },
                BatchOrigin::LocalMutation,
                &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(uuid(703)),
                        home_document_id: DocumentId::from_uuid(uuid(702)),
                    },
                    content: format!("historical-{sequence}"),
                }])
                .unwrap(),
            )
            .unwrap();
        let historical = engine
            .finalize_author_transaction(
                draft,
                binding,
                vec![receipts
                    .capture_projection_input(
                        &graph,
                        binding,
                        ManagedPath::parse("pages/manifested.md").unwrap(),
                        Some(&prior_intent),
                    )
                    .unwrap()],
            )
            .unwrap();
        let writer = ObjectStore::open(&archive_path, workspace_id).unwrap();
        writer.publish_prepared(&historical).unwrap();
        assert!(matches!(
            engine
                .stage_archive_batch(historical_batch)
                .unwrap()
                .disposition(),
            tine_core::oplog::BatchDisposition::Accepted { .. }
        ));
        let historical_work = engine
            .projection_work_index()
            .unwrap()
            .pending_for_path(&ManagedPath::parse("pages/manifested.md").unwrap())
            .unwrap()
            .into_iter()
            .find(|work| work.batch_id() == historical_batch)
            .unwrap();
        execute_manifested_projection_work(&graph, &receipts, &mut engine, &historical_work)
            .unwrap();
        let historical_state = engine.materialize_page_for_projection(page_id).unwrap();
        let historical_plan = plan_projection(
            workspace_id,
            &historical_state,
            Some(&current_projection_bytes),
        )
        .unwrap();
        prior_intent = historical_plan.intent().clone();
        current_projection_bytes = historical_plan.target().to_vec();
    }

    let batch_id = BatchId::from_uuid(uuid(802));
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: source_device,
                author_session_id: SessionId::from_uuid(uuid(803)),
                crdt_peer_id: CrdtPeerId::from_u64(804),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(uuid(703)),
                    home_document_id: DocumentId::from_uuid(uuid(702)),
                },
                content: "after".into(),
            }])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(draft.requirements().len(), 1);
    let prepared = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![receipts
                .capture_projection_input(
                    &graph,
                    binding,
                    ManagedPath::parse("pages/manifested.md").unwrap(),
                    Some(&prior_intent),
                )
                .unwrap()],
        )
        .unwrap();
    assert_eq!(prepared.manifest().origin(), BatchOrigin::LocalMutation);
    assert_eq!(
        prepared
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::ProjectionIntent)
            .count(),
        1
    );
    assert_eq!(
        prepared
            .objects()
            .iter()
            .filter(|object| object.kind() == ObjectKind::AnnotatedBaseBlob)
            .count(),
        1
    );

    let writer = ObjectStore::open(&archive_path, workspace_id).unwrap();
    writer.publish_prepared(&prepared).unwrap();
    let work_head = archive_path
        .join("projection-work-index-v1")
        .join(source_endpoint.to_string())
        .join("projection-work.head");
    let history_head = archive_path
        .join("engine-history")
        .join(source_endpoint.to_string())
        .join("engine-history.head");
    let history_head_before_acceptance = fs::read(&history_head).unwrap();
    let outcome = engine.stage_archive_batch(batch_id).unwrap();
    assert!(matches!(
        outcome.disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let accepted_head = fs::read(&work_head).unwrap();
    let accepted_history_head = fs::read(&history_head).unwrap();
    let pending_head = engine
        .projection_work_index()
        .unwrap()
        .accepted_preparation_root(batch_id)
        .unwrap();
    assert_ne!(accepted_head, pending_head.to_string().as_bytes());
    assert_ne!(accepted_history_head, history_head_before_acceptance);

    let source_intent = prepared
        .objects()
        .iter()
        .find(|object| object.kind() == ObjectKind::ProjectionIntent)
        .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
        .unwrap();
    assert_eq!(
        source_intent.portable_path_key_version(),
        PORTABLE_PATH_KEY_VERSION
    );
    assert_eq!(
        source_intent.portable_path_key_digest(),
        source_intent.path().portable_key().digest()
    );
    let source_target = source_intent.target().bytes().unwrap();

    // A valid rollback of the durable endpoint-history head must revoke write
    // authority immediately on the same engine. Its authenticated run-local
    // scratch still says Accepted, its cached history root is the accepted
    // root, and the durable work row is still Ready: none may substitute for
    // the current durable endpoint witness.
    let same_engine_work = engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        same_engine_work.portable_path_key_version(),
        PORTABLE_PATH_KEY_VERSION
    );
    assert_eq!(
        same_engine_work.portable_path_key_digest(),
        source_intent.portable_path_key_digest()
    );
    assert_eq!(
        same_engine_work.portable_path_index_root(),
        source_intent.portable_path_index_root()
    );
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(same_engine_work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Ready)
    );
    fs::write(&history_head, &history_head_before_acceptance).unwrap();
    assert!(
        execute_manifested_projection_work(&graph, &receipts, &mut engine, &same_engine_work,)
            .is_err()
    );
    assert_eq!(
        fs::read(graph_root.join("pages/manifested.md")).unwrap(),
        current_projection_bytes
    );

    fs::write(&history_head, &accepted_history_head).unwrap();
    execute_manifested_projection_work(&graph, &receipts, &mut engine, &same_engine_work).unwrap();
    assert_eq!(
        fs::read(graph_root.join("pages/manifested.md")).unwrap(),
        source_target
    );
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .status(same_engine_work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Completed)
    );
    fs::write(&work_head, &accepted_head).unwrap();

    // Model a crash after durable engine acceptance but before the singular
    // pending -> accepted/Ready projection-work root publication.
    fs::write(&work_head, pending_head.to_string()).unwrap();
    assert!(engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    assert_eq!(
        engine
            .projection_work_index()
            .unwrap()
            .pending_activation_page(None, 8)
            .unwrap()
            .pending()
            .len(),
        1
    );

    let crlf = String::from_utf8(source_target.to_vec())
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes();
    let receiver = derive_receiver_local_projection(
        &engine,
        &source_intent,
        ProjectionEndpointId::from_uuid(uuid(805)),
        Some(&crlf),
    )
    .unwrap();
    assert_eq!(receiver.target(), crlf);
    assert_ne!(receiver.target(), source_target);
    assert_eq!(
        receiver.intent().precondition(),
        &ProjectionPrecondition::Base(BlobDescription::of(&crlf))
    );

    drop(engine);
    let recovery_reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let recovery_engine = ShardedHotEngine::with_enrolled_projection(
        recovery_reader,
        lineage,
        catalog,
        &graph,
        &receipts,
    );
    let recovery_work = recovery_engine.instrumentation();
    assert_eq!(recovery_work.projection_pending_entries_read, 1);
    assert_eq!(recovery_work.store.history_record_reads, 1);
    assert!(recovery_work.store.history_index_reads <= 33);
    assert_eq!(fs::read(&work_head).unwrap(), accepted_head);
    assert!(recovery_engine
        .projection_work_index()
        .unwrap()
        .pending_activation_page(None, 8)
        .unwrap()
        .pending()
        .is_empty());
    let work = recovery_engine
        .projection_work_index()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(work.batch_id(), batch_id);
    assert!(matches!(work.target(), ProjectionWorkTarget::Present(_)));
    assert_eq!(
        recovery_engine
            .projection_work_index()
            .unwrap()
            .status(work.work_id())
            .unwrap(),
        Some(ProjectionWorkStatus::Ready)
    );
    assert_eq!(
        recovery_engine
            .projection_work_index()
            .unwrap()
            .pending_for_path(&ManagedPath::parse("pages/manifested.md").unwrap())
            .unwrap()
            .len(),
        1
    );

    let work_id = work.work_id();

    // Even a locally Ready work root cannot authorize execution when a fresh
    // engine's authenticated durable history does not contain this acceptance.
    fs::write(&history_head, &history_head_before_acceptance).unwrap();
    let unaccepted_reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let mut unaccepted_engine = ShardedHotEngine::with_enrolled_projection(
        unaccepted_reader,
        lineage,
        catalog,
        &graph,
        &receipts,
    );
    assert!(
        execute_manifested_projection_work(&graph, &receipts, &mut unaccepted_engine, &work,)
            .is_err()
    );
    assert_eq!(
        fs::read(graph_root.join("pages/manifested.md")).unwrap(),
        source_target
    );

    fs::write(&history_head, &accepted_history_head).unwrap();
    let accepted_reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let mut accepted_engine = ShardedHotEngine::with_enrolled_projection(
        accepted_reader,
        lineage,
        catalog,
        &graph,
        &receipts,
    );
    execute_manifested_projection_work(&graph, &receipts, &mut accepted_engine, &work).unwrap();
    assert_eq!(
        fs::read(graph_root.join("pages/manifested.md")).unwrap(),
        source_target
    );
    assert_eq!(
        accepted_engine
            .projection_work_index()
            .unwrap()
            .status(work_id)
            .unwrap(),
        Some(ProjectionWorkStatus::Completed)
    );
}

#[test]
fn manifested_rename_has_old_removal_and_new_target_using_old_render_base() {
    let dir = TestDir::new("manifested-rename");
    let workspace_id = workspace(1);
    let lineage = LineageDigest::of(b"projection-test-lineage");
    let catalog = DocumentId::from_uuid(uuid(700));
    let archive_path = dir.path().join("archive");
    let graph_root = dir.path().join("rename-graph");
    fs::create_dir_all(graph_root.join("pages")).unwrap();
    let graph = Graph::open(&graph_root);
    let binding = projection_binding(&graph, 820);
    let receipts = ProjectionReceiptStore::open_for_endpoint(
        &dir.path().join("rename-receipts"),
        workspace_id,
        binding,
    )
    .unwrap();
    let initial_dir = TestDir::new("manifested-rename-initial-authority");
    let (initial, page_id) = authorized_engine(
        &initial_dir,
        "pages/old-name.md",
        "rename me",
        Some((&graph, &receipts)),
    );
    let (archive_seed, archive_page_id) =
        authorized_engine(&dir, "pages/old-name.md", "rename me", None);
    assert_eq!(archive_page_id, page_id);
    drop(archive_seed);
    let base_write = write_projection_exact(&graph, &receipts, &initial, page_id, None).unwrap();
    drop(initial);
    let reader = ObjectStore::open(&archive_path, workspace_id).unwrap();
    let mut engine =
        ShardedHotEngine::with_enrolled_projection(reader, lineage, catalog, &graph, &receipts);
    engine
        .stage_archive_batch(BatchId::from_uuid(uuid(704)))
        .unwrap();
    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id: BatchId::from_uuid(uuid(822)),
                author_device_id: binding.device_id,
                author_session_id: SessionId::from_uuid(uuid(823)),
                crdt_peer_id: CrdtPeerId::from_u64(824),
            },
            BatchOrigin::LocalMutation,
            &OperationTransaction::new(vec![SemanticOperation::EditPagePath {
                page_id,
                path: ManagedPath::parse("pages/new-name.md").unwrap(),
            }])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(draft.requirements().len(), 2);
    let finalized = engine
        .finalize_author_transaction(
            draft,
            binding,
            vec![
                receipts
                    .capture_projection_input(
                        &graph,
                        binding,
                        ManagedPath::parse("pages/old-name.md").unwrap(),
                        Some(base_write.plan.intent()),
                    )
                    .unwrap(),
                receipts
                    .capture_projection_input(
                        &graph,
                        binding,
                        ManagedPath::parse("pages/new-name.md").unwrap(),
                        None,
                    )
                    .unwrap(),
            ],
        )
        .unwrap();
    let intents = finalized
        .objects()
        .iter()
        .filter(|object| object.kind() == ObjectKind::ProjectionIntent)
        .map(|object| ManifestedProjectionIntent::decode(object.payload()).unwrap())
        .collect::<Vec<_>>();
    let old = intents
        .iter()
        .find(|intent| intent.path().as_str() == "pages/old-name.md")
        .unwrap();
    assert!(matches!(
        old.precondition(),
        ManifestProjectionPrecondition::Present { .. }
    ));
    assert!(matches!(old.target(), ManifestProjectionTarget::Absent));
    let new = intents
        .iter()
        .find(|intent| intent.path().as_str() == "pages/new-name.md")
        .unwrap();
    assert!(matches!(
        new.precondition(),
        ManifestProjectionPrecondition::Absent
    ));
    assert!(matches!(
        new.target(),
        ManifestProjectionTarget::Present { .. }
    ));
    assert_eq!(
        new.render_base().unwrap().document_id(),
        old.precondition().base().unwrap().document_id()
    );
    let writer = ObjectStore::open(&archive_path, workspace_id).unwrap();
    writer.publish_prepared(&finalized).unwrap();
    let outcome = engine
        .stage_archive_batch(BatchId::from_uuid(uuid(822)))
        .unwrap();
    assert!(matches!(
        outcome.disposition(),
        tine_core::oplog::BatchDisposition::Accepted { .. }
    ));
    let old_work = engine
        .projection_work_index()
        .unwrap()
        .pending_for_path(&ManagedPath::parse("pages/old-name.md").unwrap())
        .unwrap();
    let new_work = engine
        .projection_work_index()
        .unwrap()
        .pending_for_path(&ManagedPath::parse("pages/new-name.md").unwrap())
        .unwrap();
    assert_eq!(old_work.len(), 1);
    assert_eq!(new_work.len(), 1);

    execute_manifested_projection_work(&graph, &receipts, &mut engine, &old_work[0]).unwrap();
    assert!(!graph_root.join("pages/old-name.md").exists());
    execute_manifested_projection_work(&graph, &receipts, &mut engine, &new_work[0]).unwrap();
    assert_eq!(
        fs::read(graph_root.join("pages/new-name.md")).unwrap(),
        new.target().bytes().unwrap()
    );
}
