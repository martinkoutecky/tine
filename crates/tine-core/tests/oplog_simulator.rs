use tine_core::oplog::simulator::{
    ByteMutation, DeterministicSimulator, ExternalFileFixture, IngressExpectation,
    InvariantAssertion, InvariantPredicate, ScenarioWorkspace, ScheduledAction,
    ScheduledActionKind, StageExpectation, WireBatch, WireBytes, WireItem,
};
use tine_core::oplog::{
    AuthorBatch, BatchId, BlockId, BlockLocation, CrdtPeerId, DeviceId, DocumentId, LineageDigest,
    ManagedPath, ManagedTextKind, OperationTransaction, PageId, Scenario, ScenarioDevice,
    SemanticOperation, SessionId, ShardedHotEngine, WorkspaceId,
};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct Ids {
    workspace: WorkspaceId,
    lineage: LineageDigest,
    catalog: DocumentId,
    page_a: PageId,
    page_b: PageId,
    page_c: PageId,
    home_a: DocumentId,
    home_b: DocumentId,
    home_c: DocumentId,
    block: BlockId,
}

impl Ids {
    fn new() -> Self {
        Self {
            workspace: WorkspaceId::from_uuid(uuid(1)),
            lineage: LineageDigest::of(b"oplog simulator independent harness"),
            catalog: DocumentId::from_uuid(uuid(2)),
            page_a: PageId::from_uuid(uuid(10)),
            page_b: PageId::from_uuid(uuid(11)),
            page_c: PageId::from_uuid(uuid(12)),
            home_a: DocumentId::from_uuid(uuid(20)),
            home_b: DocumentId::from_uuid(uuid(21)),
            home_c: DocumentId::from_uuid(uuid(22)),
            block: BlockId::from_uuid(uuid(30)),
        }
    }

    fn workspace(self) -> ScenarioWorkspace {
        ScenarioWorkspace {
            workspace_id: self.workspace,
            lineage_digest: self.lineage,
            catalog_document_id: self.catalog,
        }
    }
}

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn device(name: &str, value: u64) -> ScenarioDevice {
    ScenarioDevice {
        name: name.into(),
        device_id: DeviceId::from_uuid(uuid(1_000 + value as u128)),
        crdt_peer_id: CrdtPeerId::from_u64(value),
    }
}

fn path(value: &str) -> ManagedPath {
    ManagedPath::parse(value).unwrap()
}

fn tx(operations: Vec<SemanticOperation>) -> OperationTransaction {
    OperationTransaction::new(operations).unwrap()
}

fn event(event_id: u64, action: ScheduledActionKind) -> ScheduledAction {
    ScheduledAction {
        event_id,
        tick: event_id,
        action,
    }
}

fn wire_batch(
    ids: Ids,
    batch_id: BatchId,
    peer: u64,
    transaction: OperationTransaction,
) -> WireBatch {
    wire_batch_with_lineage(ids, ids.lineage, batch_id, peer, transaction)
}

fn wire_batch_with_lineage(
    ids: Ids,
    lineage: LineageDigest,
    batch_id: BatchId,
    peer: u64,
    transaction: OperationTransaction,
) -> WireBatch {
    let engine = ShardedHotEngine::new(ids.workspace, lineage, ids.catalog);
    let prepared = engine
        .prepare_bootstrap_transaction(
            AuthorBatch {
                batch_id,
                author_device_id: DeviceId::from_uuid(uuid(2_000 + peer as u128)),
                author_session_id: SessionId::from_uuid(uuid(3_000 + peer as u128)),
                crdt_peer_id: CrdtPeerId::from_u64(peer),
            },
            &transaction,
        )
        .unwrap();
    let objects = prepared
        .objects()
        .iter()
        .enumerate()
        .map(|(index, object)| WireItem {
            item_id: format!("wire/{batch_id}/object/{index}"),
            bytes_b64: WireBytes(object.encode().unwrap()),
        })
        .collect();
    WireBatch {
        name: format!("batch-{batch_id}"),
        batch_id,
        manifest: WireItem {
            item_id: format!("wire/{batch_id}/manifest"),
            bytes_b64: WireBytes(prepared.manifest().encode().unwrap()),
        },
        objects,
    }
}

fn create_page_batch(
    ids: Ids,
    batch: u128,
    peer: u64,
    page: PageId,
    home: DocumentId,
    logical_name: &str,
    page_path: &str,
) -> WireBatch {
    wire_batch(
        ids,
        BatchId::from_uuid(uuid(batch)),
        peer,
        tx(vec![SemanticOperation::CreatePage {
            page_id: page,
            home_document_id: home,
            name: tine_core::oplog::LogicalPageName::parse(logical_name).unwrap(),
            path: path(page_path),
            kind: ManagedTextKind::Page,
        }]),
    )
}

fn deliver_all(
    actions: &mut Vec<ScheduledAction>,
    next: &mut u64,
    device: &str,
    batch: &WireBatch,
) {
    for object in &batch.objects {
        actions.push(event(
            *next,
            ScheduledActionKind::DeliverItem {
                device: device.into(),
                item_id: object.item_id.clone(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ));
        *next += 1;
    }
    actions.push(event(
        *next,
        ScheduledActionKind::DeliverItem {
            device: device.into(),
            item_id: batch.manifest.item_id.clone(),
            mutation: ByteMutation::Exact,
            expected: Some(IngressExpectation::Accepted),
        },
    ));
    *next += 1;
    actions.push(event(
        *next,
        ScheduledActionKind::ProbeBatch {
            device: device.into(),
            batch_id: batch.batch_id,
            expected: Some(StageExpectation::Accepted),
        },
    ));
    *next += 1;
}

#[test]
fn raw_ingress_order_tamper_restart_and_external_oracles_are_store_backed() {
    let ids = Ids::new();
    let batch = create_page_batch(ids, 100, 100, ids.page_a, ids.home_a, "A", "pages/A.md");
    let mut actions = vec![
        event(
            1,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: batch.manifest.item_id.clone(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ),
        event(
            2,
            ScheduledActionKind::ProbeBatch {
                device: "beta".into(),
                batch_id: batch.batch_id,
                expected: Some(StageExpectation::Incomplete),
            },
        ),
        event(
            3,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect {
                    device: "beta".into(),
                    snapshot: Default::default(),
                },
            },
        ),
        event(
            4,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: batch.objects[0].item_id.clone(),
                mutation: ByteMutation::XorByte {
                    offset: 0,
                    mask: 0x80,
                },
                expected: None,
            },
        ),
        event(
            5,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect {
                    device: "beta".into(),
                    snapshot: Default::default(),
                },
            },
        ),
        event(
            6,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: batch.manifest.item_id.clone(),
                mutation: ByteMutation::Truncate { len: 1 },
                expected: None,
            },
        ),
        event(
            7,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect {
                    device: "beta".into(),
                    snapshot: Default::default(),
                },
            },
        ),
        event(
            8,
            ScheduledActionKind::Crash {
                device: "beta".into(),
            },
        ),
        event(
            9,
            ScheduledActionKind::Restart {
                device: "beta".into(),
            },
        ),
    ];
    let mut next = 10;
    deliver_all(&mut actions, &mut next, "beta", &batch);
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::RestartReplay {
                device: "beta".into(),
            },
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::UntouchedExternalFiles,
        },
    ));

    let scenario = Scenario::from_schedule(
        "truncated-and-tampered-object-and-manifest",
        100,
        ids.workspace(),
        vec![device("alpha", 1), device("beta", 2)],
        vec![batch],
        Vec::new(),
        actions,
        Vec::new(),
        vec![ExternalFileFixture {
            path: "external/untouched.md".into(),
            bytes_b64: WireBytes(b"do not touch".to_vec()),
        }],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    assert!(!simulator.ingress_receipts().get(&4).unwrap().accepted);
    assert!(!simulator.ingress_receipts().get(&6).unwrap().accepted);
    assert_eq!(
        simulator.ingress_receipts().get(&4).unwrap().item_id,
        "wire/00000000-0000-0000-0000-000000000064/object/0"
    );
}

#[test]
fn independent_replicas_converge_after_object_first_duplicate_reordered_delivery() {
    let ids = Ids::new();
    let batch = create_page_batch(ids, 110, 110, ids.page_a, ids.home_a, "A", "pages/A.md");
    let mut actions = Vec::new();
    let mut next = 1;
    deliver_all(&mut actions, &mut next, "alpha", &batch);
    for object in batch.objects.iter().rev() {
        actions.push(event(
            next,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: object.item_id.clone(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ));
        next += 1;
    }
    actions.push(event(
        next,
        ScheduledActionKind::DeliverItem {
            device: "beta".into(),
            item_id: batch.objects[0].item_id.clone(),
            mutation: ByteMutation::Exact,
            expected: Some(IngressExpectation::Accepted),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::DeliverItem {
            device: "beta".into(),
            item_id: batch.manifest.item_id.clone(),
            mutation: ByteMutation::Exact,
            expected: Some(IngressExpectation::Accepted),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ProbeBatch {
            device: "beta".into(),
            batch_id: batch.batch_id,
            expected: Some(StageExpectation::Accepted),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::Converged {
                devices: vec!["alpha".into(), "beta".into()],
            },
        },
    ));

    let scenario = Scenario::from_schedule(
        "duplicate-reordered-dependent-tail-restart",
        110,
        ids.workspace(),
        vec![device("alpha", 1), device("beta", 2)],
        vec![batch],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let snapshots = simulator.snapshots().unwrap();
    assert_eq!(snapshots[0], snapshots[1]);
}

#[test]
fn transfer_provider_copy_drop_and_wrong_object_substitution_leave_no_effect_before_recovery() {
    let ids = Ids::new();
    let first = create_page_batch(ids, 120, 120, ids.page_a, ids.home_a, "A", "pages/A.md");
    let second = create_page_batch(ids, 121, 121, ids.page_b, ids.home_b, "B", "pages/B.md");
    let object_len = first.objects[0].bytes_b64.0.len();
    let mut actions = vec![
        event(
            1,
            ScheduledActionKind::CopyProviderItem {
                source_item_id: second.objects[0].item_id.clone(),
                copy_item_id: "provider/conflict-copy".into(),
            },
        ),
        event(
            2,
            ScheduledActionKind::DropProviderItem {
                item_id: "provider/conflict-copy".into(),
            },
        ),
        event(
            3,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: first.manifest.item_id.clone(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ),
        event(
            4,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: first.objects[0].item_id.clone(),
                mutation: ByteMutation::Substitute {
                    item_id: second.objects[0].item_id.clone(),
                },
                expected: Some(IngressExpectation::Accepted),
            },
        ),
        event(
            5,
            ScheduledActionKind::ProbeBatch {
                device: "beta".into(),
                batch_id: first.batch_id,
                expected: Some(StageExpectation::Incomplete),
            },
        ),
        event(
            6,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect {
                    device: "beta".into(),
                    snapshot: Default::default(),
                },
            },
        ),
        event(
            7,
            ScheduledActionKind::BeginTransfer {
                device: "beta".into(),
                transfer_id: "first-object".into(),
                item_id: first.objects[0].item_id.clone(),
            },
        ),
        event(
            8,
            ScheduledActionKind::AppendTransfer {
                device: "beta".into(),
                transfer_id: "first-object".into(),
                len: object_len,
            },
        ),
        event(
            9,
            ScheduledActionKind::CommitTransfer {
                device: "beta".into(),
                transfer_id: "first-object".into(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ),
    ];
    let mut next = 10;
    for object in first.objects.iter().skip(1) {
        actions.push(event(
            next,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: object.item_id.clone(),
                mutation: ByteMutation::Exact,
                expected: Some(IngressExpectation::Accepted),
            },
        ));
        next += 1;
    }
    actions.push(event(
        next,
        ScheduledActionKind::ProbeBatch {
            device: "beta".into(),
            batch_id: first.batch_id,
            expected: Some(StageExpectation::Accepted),
        },
    ));

    let scenario = Scenario::from_schedule(
        "provider-conflict-stale-copy-and-transfer",
        120,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![first, second],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn reducer_preserves_the_first_exact_invariant_signature() {
    let ids = Ids::new();
    let batch = create_page_batch(ids, 130, 130, ids.page_a, ids.home_a, "A", "pages/A.md");
    let mut actions = Vec::new();
    let mut next = 1;
    deliver_all(&mut actions, &mut next, "alpha", &batch);
    let first_assertion = next;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::NoVisibleEffect {
                device: "alpha".into(),
                snapshot: Default::default(),
            },
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::NoVisibleEffect {
                device: "alpha".into(),
                snapshot: Default::default(),
            },
        },
    ));
    let scenario = Scenario::from_schedule(
        "same-page-concurrent-text-reducer",
        130,
        ids.workspace(),
        vec![device("alpha", 1)],
        vec![batch],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let minimized = scenario.minimize_failure().unwrap();
    match minimized.capsule.failure {
        tine_core::oplog::FailureIdentity::Invariant(signature) => {
            assert_eq!(signature.predicate, InvariantPredicate::NoVisibleEffect);
            assert_eq!(signature.assertion_or_event_id, first_assertion);
        }
        other => panic!("unexpected failure identity: {other:?}"),
    }
    assert!(minimized
        .scenario
        .actions
        .iter()
        .any(|action| action.event_id == first_assertion));
    assert!(minimized.capsule.accepted_witness.contains_key("alpha"));
    assert!(minimized.capsule.offered_witness.contains_key("alpha"));
    assert!(minimized
        .capsule
        .status_witness
        .get("alpha")
        .is_some_and(|status| status == "operational"));
    assert!(minimized.capsule.expected_snapshot_hash.is_some());
    assert!(minimized.capsule.observed_snapshot_hash.is_some());
    assert!(minimized.capsule.first_canonical_difference.is_some());
}

#[test]
fn delayed_parent_and_lineage_refusal_replay_from_the_receiver_store() {
    let ids = Ids::new();
    let root = create_page_batch(ids, 135, 135, ids.page_a, ids.home_a, "A", "pages/A.md");
    let foreign = wire_batch_with_lineage(
        ids,
        LineageDigest::of(b"foreign independent genesis"),
        BatchId::from_uuid(uuid(136)),
        136,
        tx(vec![SemanticOperation::CreatePage {
            page_id: ids.page_b,
            home_document_id: ids.home_b,
            name: tine_core::oplog::LogicalPageName::parse("Foreign").unwrap(),
            path: path("pages/Foreign.md"),
            kind: ManagedTextKind::Page,
        }]),
    );
    let mut actions = Vec::new();
    let mut next = 1;
    deliver_all(&mut actions, &mut next, "beta", &root);
    actions.push(event(
        next,
        ScheduledActionKind::DeliverItem {
            device: "beta".into(),
            item_id: foreign.manifest.item_id.clone(),
            mutation: ByteMutation::Exact,
            expected: None,
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::LineageIsolation {
                device: "beta".into(),
                accepted: vec![root.batch_id],
            },
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Crash {
            device: "beta".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Restart {
            device: "beta".into(),
        },
    ));

    let scenario = Scenario::from_schedule(
        "independent-genesis-lineage-refusal-delayed-parent",
        135,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![root.clone(), foreign],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    assert!(
        !simulator
            .ingress_receipts()
            .get(&(next - 3))
            .unwrap()
            .accepted
    );

    let legacy = Scenario::new(
        "delayed-parent-child-before-parent",
        137,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        vec![device("alpha", 1), device("beta", 2)],
        vec![
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: root.batch_id,
                session_id: SessionId::from_uuid(uuid(4_135)),
                transaction: tx(vec![SemanticOperation::CreatePage {
                    page_id: ids.page_a,
                    home_document_id: ids.home_a,
                    name: tine_core::oplog::LogicalPageName::parse("A").unwrap(),
                    path: path("pages/A.md"),
                    kind: ManagedTextKind::Page,
                }]),
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: BatchId::from_uuid(uuid(137)),
                session_id: SessionId::from_uuid(uuid(4_137)),
                transaction: tx(vec![SemanticOperation::EditPagePath {
                    page_id: ids.page_a,
                    path: path("pages/A-renamed.md"),
                }]),
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: BatchId::from_uuid(uuid(137)),
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: root.batch_id,
            },
            tine_core::oplog::ScenarioAction::AssertConverged {
                devices: vec![0, 1],
            },
        ],
    )
    .unwrap();
    let mut delayed = DeterministicSimulator::new(legacy).unwrap();
    delayed.run().unwrap();
}

#[test]
fn same_page_concurrent_text_converges_after_raw_whole_batch_exchange() {
    let ids = Ids::new();
    let root = BatchId::from_uuid(uuid(138));
    let left = BatchId::from_uuid(uuid(139));
    let right = BatchId::from_uuid(uuid(140));
    let scenario = Scenario::new(
        "same-page-concurrent-text",
        138,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        vec![device("alpha", 1), device("beta", 2), device("gamma", 3)],
        vec![
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: root,
                session_id: SessionId::from_uuid(uuid(4_138)),
                transaction: tx(vec![
                    SemanticOperation::CreatePage {
                        page_id: ids.page_a,
                        home_document_id: ids.home_a,
                        name: tine_core::oplog::LogicalPageName::parse("A").unwrap(),
                        path: path("pages/A.md"),
                        kind: ManagedTextKind::Page,
                    },
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: ids.block,
                            home_document_id: ids.home_a,
                        },
                        page_id: ids.page_a,
                        parent: None,
                        order: "a".into(),
                        content: "base".into(),
                    },
                ]),
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: root,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: root,
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: left,
                session_id: SessionId::from_uuid(uuid(4_139)),
                transaction: tx(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.home_a,
                    },
                    content: "left".into(),
                }]),
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 1,
                batch_id: right,
                session_id: SessionId::from_uuid(uuid(4_140)),
                transaction: tx(vec![SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block,
                        home_document_id: ids.home_a,
                    },
                    content: "right".into(),
                }]),
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: right,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: left,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 0,
                batch_id: right,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: left,
            },
            tine_core::oplog::ScenarioAction::AssertConverged {
                devices: vec![0, 1, 2],
            },
        ],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn same_page_concurrent_text_and_moved_away_move_delete_converge_in_both_orders() {
    let ids = Ids::new();
    let root_id = BatchId::from_uuid(uuid(140));
    let move_ab = BatchId::from_uuid(uuid(141));
    let move_bc = BatchId::from_uuid(uuid(142));
    let delete_b = BatchId::from_uuid(uuid(143));
    let devices = vec![
        device("alpha", 1),
        device("beta", 2),
        device("gamma", 3),
        device("delta", 4),
    ];
    let root = tx(vec![
        SemanticOperation::CreatePage {
            page_id: ids.page_a,
            home_document_id: ids.home_a,
            name: tine_core::oplog::LogicalPageName::parse("A").unwrap(),
            path: path("pages/A.md"),
            kind: ManagedTextKind::Page,
        },
        SemanticOperation::CreatePage {
            page_id: ids.page_b,
            home_document_id: ids.home_b,
            name: tine_core::oplog::LogicalPageName::parse("B").unwrap(),
            path: path("pages/B.md"),
            kind: ManagedTextKind::Page,
        },
        SemanticOperation::CreatePage {
            page_id: ids.page_c,
            home_document_id: ids.home_c,
            name: tine_core::oplog::LogicalPageName::parse("C").unwrap(),
            path: path("pages/C.md"),
            kind: ManagedTextKind::Page,
        },
        SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id: ids.block,
                home_document_id: ids.home_a,
            },
            page_id: ids.page_a,
            parent: None,
            order: "a".into(),
            content: "original".into(),
        },
    ]);
    let move_from_a_to_b = tx(vec![SemanticOperation::MoveSubtree {
        root: BlockLocation {
            block_id: ids.block,
            home_document_id: ids.home_a,
        },
        from_page_id: ids.page_a,
        to_page_id: ids.page_b,
        parent: None,
        order: "b".into(),
    }]);
    let move_from_b_to_c = tx(vec![SemanticOperation::MoveSubtree {
        root: BlockLocation {
            block_id: ids.block,
            home_document_id: ids.home_a,
        },
        from_page_id: ids.page_b,
        to_page_id: ids.page_c,
        parent: None,
        order: "c".into(),
    }]);
    let delete_from_b = tx(vec![SemanticOperation::DeleteSubtree {
        root_block_id: ids.block,
        page_id: ids.page_b,
    }]);
    let scenario = Scenario::new(
        "moved-away-move-delete-both-orders-owner-and-tombstone-winners",
        140,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: root_id,
                session_id: SessionId::from_uuid(uuid(4_140)),
                transaction: root,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: root_id,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: root_id,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 3,
                batch_id: root_id,
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: move_ab,
                session_id: SessionId::from_uuid(uuid(4_141)),
                transaction: move_from_a_to_b,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: move_ab,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: move_ab,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 3,
                batch_id: move_ab,
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: move_bc,
                session_id: SessionId::from_uuid(uuid(4_142)),
                transaction: move_from_b_to_c,
            },
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 1,
                batch_id: delete_b,
                session_id: SessionId::from_uuid(uuid(4_143)),
                transaction: delete_from_b,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: move_bc,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: delete_b,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 3,
                batch_id: delete_b,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 3,
                batch_id: move_bc,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 0,
                batch_id: delete_b,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: move_bc,
            },
            tine_core::oplog::ScenarioAction::AssertConverged {
                devices: vec![0, 1, 2, 3],
            },
        ],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let snapshot = simulator.snapshots().unwrap().pop().unwrap();
    assert!(snapshot
        .blocks
        .iter()
        .all(|block| block.block_id != ids.block || block.home_document_id == ids.home_a));
    assert!(
        snapshot
            .memberships
            .iter()
            .filter(|membership| membership.block_id == ids.block)
            .count()
            <= 1
    );
}

#[test]
fn corpus_keeps_same_page_cross_page_and_moved_away_family_seeds_visible() {
    let ids = Ids::new();
    let root = create_page_batch(ids, 140, 140, ids.page_a, ids.home_a, "A", "pages/A.md");
    let devices = vec![device("alpha", 1), device("beta", 2), device("gamma", 3)];
    let legacy = Scenario::new(
        "moved-away-move-delete-both-orders-owner-and-tombstone-winners",
        140,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            tine_core::oplog::ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: root.batch_id,
                session_id: SessionId::from_uuid(uuid(4_140)),
                transaction: tx(vec![SemanticOperation::CreatePage {
                    page_id: ids.page_a,
                    home_document_id: ids.home_a,
                    name: tine_core::oplog::LogicalPageName::parse("A").unwrap(),
                    path: path("pages/A.md"),
                    kind: ManagedTextKind::Page,
                }]),
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 1,
                batch_id: root.batch_id,
            },
            tine_core::oplog::ScenarioAction::Deliver {
                device: 2,
                batch_id: root.batch_id,
            },
            tine_core::oplog::ScenarioAction::AssertConverged {
                devices: vec![0, 1, 2],
            },
        ],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(legacy).unwrap();
    simulator.run().unwrap();
}

#[test]
fn fixture_seed_corpus_is_canonical_v3_json() {
    let fixtures = [
        include_str!("fixtures/oplog-simulator/object-before-manifest.scenario.json"),
        include_str!("fixtures/oplog-simulator/manifest-before-objects-and-missing.scenario.json"),
        include_str!(
            "fixtures/oplog-simulator/truncated-and-tampered-object-and-manifest.scenario.json"
        ),
        include_str!(
            "fixtures/oplog-simulator/duplicate-reordered-dependent-tail-restart.scenario.json"
        ),
        include_str!("fixtures/oplog-simulator/independent-genesis-lineage-refusal.scenario.json"),
        include_str!(
            "fixtures/oplog-simulator/local-author-whole-batch-to-two-replicas.scenario.json"
        ),
        include_str!("fixtures/oplog-simulator/moved-away-move-delete.scenario.json"),
    ];
    for fixture in fixtures {
        let fixture = fixture.trim_end();
        let scenario = Scenario::decode(fixture.as_bytes()).unwrap();
        assert_eq!(scenario.encode().unwrap(), fixture.as_bytes());
    }
}
