use sha2::{Digest, Sha256};
use tine_core::oplog::simulator::{
    ByteMutation, CoordinatorAction, CoordinatorFault, CoordinatorHandoffState, CoordinatorOracle,
    CoordinatorReadGate, CoordinatorRunOutcome, CoordinatorSqliteMutation, DeterministicSimulator,
    ExpectedWorkspaceState, ExternalFileFixture, IngressExpectation, InvariantAssertion,
    InvariantPredicate, ProviderLocation, ProviderSource, ProviderTree, ReplicaExpectation,
    ScenarioError, ScenarioWorkspace, ScheduledAction, ScheduledActionKind,
    SimulatorBlockedEvidence, SimulatorDeviceState, StageExpectation, WireBatch, WireBytes,
    WireItem, MAX_PROVIDER_RESCAN_BYTES, MAX_PROVIDER_RESCAN_DEPTH,
};
use tine_core::oplog::{
    AuthorBatch, BatchId, BlockId, BlockLocation, CrdtPeerId, DeviceId, DocumentId,
    FrozenCandidateId, LineageDigest, ManagedPath, ManagedTextKind, OperationTransaction, PageId,
    Scenario, ScenarioDevice, SemanticOperation, SessionId, ShardedHotEngine, WorkspaceId,
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

fn frozen_candidate() -> FrozenCandidateId {
    FrozenCandidateId::parse("be54af627a5a9dc70481478f38817c9955b28faa").unwrap()
}

#[test]
fn scenario_device_names_are_portable_display_components() {
    let ids = Ids::new();
    for name in [
        "",
        ".",
        "..",
        "/var/tmp/escape",
        "name/child",
        r"\\server\share",
        r"name\child",
        r"C:\\provider",
        "C:relative",
        "name:stream",
        "NUL",
        "COM1.trace",
        "trailing.",
        "trailing ",
    ] {
        let result = Scenario::from_schedule(
            "portable-device-name",
            1,
            ids.workspace(),
            vec![device(name, 1)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        assert!(
            matches!(result, Err(ScenarioError::InvalidDevice)),
            "accepted non-portable device name {name:?}"
        );
    }
}

#[test]
fn device_runtime_paths_use_internal_ordinals_not_display_names() {
    let ids = Ids::new();
    let scenario = Scenario::from_schedule(
        "internal-device-root",
        1,
        ids.workspace(),
        vec![device("display-alpha", 1), device("display-beta", 2)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let simulator = DeterministicSimulator::new(scenario).unwrap();
    let alpha = simulator
        .provider_tree_path("display-alpha", ProviderTree::Inbox)
        .unwrap();
    let beta = simulator
        .provider_tree_path("display-beta", ProviderTree::Inbox)
        .unwrap();
    let alpha_root = alpha.parent().unwrap().parent().unwrap();
    let beta_root = beta.parent().unwrap().parent().unwrap();
    assert_eq!(alpha_root.file_name().unwrap(), "device-0000");
    assert_eq!(beta_root.file_name().unwrap(), "device-0001");
    assert_eq!(alpha_root.parent(), beta_root.parent());
    assert!(!alpha_root.to_string_lossy().contains("display-alpha"));
    assert!(!beta_root.to_string_lossy().contains("display-beta"));
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

fn provider_location(
    device: &str,
    tree: ProviderTree,
    path: impl Into<String>,
) -> ProviderLocation {
    ProviderLocation {
        device: device.into(),
        tree,
        path: path.into(),
    }
}

fn provider_copy(event_id: u64, item_id: &str, destination: ProviderLocation) -> ScheduledAction {
    event(
        event_id,
        ScheduledActionKind::ProviderCopy {
            source: ProviderSource::Mailbox {
                item_id: item_id.into(),
            },
            destination,
        },
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
    let minimized = scenario.minimize_failure(frozen_candidate()).unwrap();
    match &minimized.capsule.failure {
        tine_core::oplog::FailureIdentity::Invariant { signature, .. } => {
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
fn filesystem_provider_partial_manifest_partition_rescan_and_duplicate_are_deterministic() {
    let ids = Ids::new();
    let batch = create_page_batch(ids, 150, 150, ids.page_a, ids.home_a, "A", "pages/A.md");
    let object_len = batch.objects[0].bytes_b64.0.len();
    let alpha_object = ProviderLocation {
        device: "alpha".into(),
        tree: ProviderTree::Outbox,
        path: "objects/nested/archive/object-0".into(),
    };
    let beta_object = ProviderLocation {
        device: "beta".into(),
        tree: ProviderTree::Inbox,
        path: "objects/incoming/nested/object-0".into(),
    };
    let beta_manifest = ProviderLocation {
        device: "beta".into(),
        tree: ProviderTree::Inbox,
        path: "manifests/incoming/nested/manifest-0".into(),
    };
    let mut actions = vec![
        event(
            1,
            ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: batch.manifest.item_id.clone(),
                },
                destination: beta_manifest.clone(),
            },
        ),
        event(
            2,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
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
            ScheduledActionKind::BeginProviderWrite {
                source: ProviderSource::Mailbox {
                    item_id: batch.objects[0].item_id.clone(),
                },
                destination: beta_object.clone(),
                transfer_id: "partial-object".into(),
            },
        ),
        event(
            5,
            ScheduledActionKind::AppendProviderWrite {
                device: "beta".into(),
                transfer_id: "partial-object".into(),
                len: object_len / 2,
            },
        ),
        event(
            6,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::ProviderResidue {
                    device: "beta".into(),
                    max_entries: 3,
                    max_bytes: object_len + batch.manifest.bytes_b64.0.len() * 2,
                },
            },
        ),
        event(
            7,
            ScheduledActionKind::Crash {
                device: "beta".into(),
            },
        ),
        event(
            8,
            ScheduledActionKind::Restart {
                device: "beta".into(),
            },
        ),
        event(
            9,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        ),
        event(
            10,
            ScheduledActionKind::AssertInvariant {
                assertion: InvariantAssertion::NoVisibleEffect {
                    device: "beta".into(),
                    snapshot: Default::default(),
                },
            },
        ),
        event(
            11,
            ScheduledActionKind::SetProviderPartition {
                device: "beta".into(),
                partitioned: true,
            },
        ),
        event(
            12,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        ),
        event(
            13,
            ScheduledActionKind::SetProviderPartition {
                device: "beta".into(),
                partitioned: false,
            },
        ),
        event(
            14,
            ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: batch.objects[0].item_id.clone(),
                },
                destination: alpha_object.clone(),
            },
        ),
        event(
            15,
            ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Tree {
                    location: alpha_object.clone(),
                },
                destination: beta_object.clone(),
            },
        ),
    ];
    let mut next = 16;
    for (index, object) in batch.objects.iter().enumerate().skip(1) {
        actions.push(event(
            next,
            ScheduledActionKind::ProviderCopy {
                source: ProviderSource::Mailbox {
                    item_id: object.item_id.clone(),
                },
                destination: ProviderLocation {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    path: format!("objects/incoming/nested/object-{index}"),
                },
            },
        ));
        next += 1;
    }
    actions.push(event(
        next,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
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
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::RestartReplay {
                device: "beta".into(),
            },
        },
    ));
    let scenario = Scenario::from_schedule(
        "filesystem-provider-partial-manifest-partition-rescan",
        150,
        ids.workspace(),
        vec![device("alpha", 1), device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let provider = simulator.provider_snapshots().unwrap();
    assert_eq!(
        provider[1]
            .entries
            .iter()
            .filter(|entry| !entry.temporary)
            .count(),
        batch.objects.len() + 1
    );
    assert!(provider[1]
        .entries
        .iter()
        .filter(|entry| !entry.temporary)
        .all(|entry| entry.path.starts_with("objects/incoming/nested/")
            || entry.path.starts_with("manifests/incoming/nested/")));
}

#[test]
fn filesystem_provider_conflicting_same_name_bytes_fail_closed() {
    let ids = Ids::new();
    let first = create_page_batch(ids, 151, 151, ids.page_a, ids.home_a, "A", "pages/A.md");
    let second = create_page_batch(ids, 152, 152, ids.page_b, ids.home_b, "B", "pages/B.md");
    let destination = ProviderLocation {
        device: "beta".into(),
        tree: ProviderTree::Inbox,
        path: "objects/conflict/object".into(),
    };
    let scenario = Scenario::from_schedule(
        "filesystem-provider-conflicting-same-name-bytes",
        151,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![first.clone(), second.clone()],
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: first.objects[0].item_id.clone(),
                    },
                    destination: destination.clone(),
                },
            ),
            event(
                2,
                ScheduledActionKind::ProviderCopy {
                    source: ProviderSource::Mailbox {
                        item_id: second.objects[0].item_id.clone(),
                    },
                    destination,
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    assert!(matches!(
        simulator.run(),
        Err(tine_core::oplog::simulator::ScenarioError::ProviderConflictingBytes(_))
    ));
}

#[test]
fn filesystem_transport_replay_is_byte_identical_for_snapshots_receipts_and_signature() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        151,
        151,
        ids.page_a,
        ids.home_a,
        "Replay",
        "pages/Replay.md",
    );
    let mut actions = Vec::new();
    let mut event_id = 1_u64;
    let source_path = "objects/nested/source-0";
    let destination_path = "objects/nested/final-0";
    actions.push(provider_copy(
        event_id,
        &batch.objects[0].item_id,
        provider_location("beta", ProviderTree::Inbox, source_path),
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::ProviderRename {
            device: "beta".into(),
            tree: ProviderTree::Inbox,
            from_path: source_path.into(),
            to_path: destination_path.into(),
        },
    ));
    for (index, object) in batch.objects.iter().enumerate().skip(1) {
        event_id += 1;
        actions.push(provider_copy(
            event_id,
            &object.item_id,
            provider_location(
                "beta",
                ProviderTree::Inbox,
                format!("objects/nested/object-{index}"),
            ),
        ));
    }
    event_id += 1;
    actions.push(provider_copy(
        event_id,
        &batch.manifest.item_id,
        provider_location("beta", ProviderTree::Inbox, "manifests/nested/manifest"),
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::ProviderRemove {
            location: provider_location("beta", ProviderTree::Inbox, destination_path),
        },
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::ProviderResidue {
                device: "beta".into(),
                max_entries: 0,
                max_bytes: 0,
            },
        },
    ));
    let scenario = Scenario::from_schedule(
        "filesystem-provider-byte-identical-transport-replay",
        151,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let run = |scenario: Scenario| {
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        let signature = match simulator.run() {
            Err(ScenarioError::Invariant { signature, .. }) => signature,
            other => panic!("expected deterministic terminal signature, got {other:?}"),
        };
        assert!(!simulator.provider_ingress_receipts().is_empty());
        let receipts = simulator
            .provider_ingress_receipts()
            .iter()
            .map(|(key, receipt)| (key.clone(), receipt.clone()))
            .collect::<Vec<_>>();
        (
            serde_json::to_vec(&simulator.provider_snapshots().unwrap()).unwrap(),
            serde_json::to_vec(&receipts).unwrap(),
            simulator.states().unwrap(),
            signature,
        )
    };
    let first = run(scenario.clone());
    let second = run(scenario);
    assert_eq!(first, second);
}

#[test]
fn filesystem_provider_failure_minimizes_and_replays_end_to_end() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        1521,
        1521,
        ids.page_a,
        ids.home_a,
        "Reduced",
        "pages/Reduced.md",
    );
    let mut actions = vec![provider_copy(
        1,
        &batch.objects[0].item_id,
        provider_location("beta", ProviderTree::Outbox, "objects/reducer-noise"),
    )];
    let mut event_id = 2;
    for (index, object) in batch.objects.iter().enumerate() {
        actions.push(provider_copy(
            event_id,
            &object.item_id,
            provider_location(
                "beta",
                ProviderTree::Inbox,
                format!("objects/reduced-{index}"),
            ),
        ));
        event_id += 1;
    }
    actions.push(provider_copy(
        event_id,
        &batch.manifest.item_id,
        provider_location("beta", ProviderTree::Inbox, "manifests/reduced"),
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    event_id += 1;
    actions.push(event(
        event_id,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::NoVisibleEffect {
                device: "beta".into(),
                snapshot: Default::default(),
            },
        },
    ));
    let scenario = Scenario::from_schedule(
        "filesystem-provider-minimization",
        1521,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let minimized = scenario.minimize_failure(frozen_candidate()).unwrap();
    assert!(minimized.scenario.actions.len() < scenario.actions.len());
    let mut replay = DeterministicSimulator::new(minimized.scenario).unwrap();
    assert!(matches!(replay.run(), Err(ScenarioError::Invariant { .. })));
}

#[test]
fn filesystem_distinct_copy_rejects_same_bytes_and_leaves_bounded_residue() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        1523,
        1523,
        ids.page_a,
        ids.home_a,
        "Bounded",
        "pages/Bounded.md",
    );
    let destination = provider_location("beta", ProviderTree::Inbox, "objects/same-bytes");
    let actions = vec![
        provider_copy(1, &batch.objects[0].item_id, destination.clone()),
        provider_copy(2, &batch.objects[0].item_id, destination),
    ];
    let scenario = Scenario::from_schedule(
        "filesystem-provider-same-byte-residue",
        1523,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    assert!(matches!(
        simulator.run(),
        Err(ScenarioError::ProviderConflictingBytes(path)) if path == "objects/same-bytes"
    ));
    let snapshot = simulator.provider_snapshots().unwrap();
    assert_eq!(
        snapshot[0]
            .entries
            .iter()
            .filter(|entry| !entry.temporary)
            .count(),
        1
    );
    assert!(snapshot[0].entries.iter().all(|entry| !entry.temporary));
}

#[test]
fn filesystem_rescan_reconstructs_from_disk_after_tine_crash_without_hidden_metadata() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        153,
        153,
        ids.page_a,
        ids.home_a,
        "Disk",
        "pages/Disk.md",
    );
    let scenario = Scenario::from_schedule(
        "filesystem-provider-disk-only-restart",
        153,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Crash {
                    device: "beta".into(),
                },
            ),
            event(
                2,
                ScheduledActionKind::Restart {
                    device: "beta".into(),
                },
            ),
            event(
                3,
                ScheduledActionKind::ReceiverRescan {
                    device: "beta".into(),
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    let inbox = simulator
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    std::fs::create_dir_all(inbox.join("objects/from-disk")).unwrap();
    std::fs::create_dir_all(inbox.join("manifests/from-disk")).unwrap();
    for (index, object) in batch.objects.iter().enumerate() {
        std::fs::write(
            inbox.join(format!("objects/from-disk/object-{index}")),
            &object.bytes_b64.0,
        )
        .unwrap();
    }
    std::fs::write(
        inbox.join("manifests/from-disk/batch"),
        &batch.manifest.bytes_b64.0,
    )
    .unwrap();

    simulator.run().unwrap();
    println!("{:?}", simulator.outcomes());
    let states = simulator.states().unwrap();
    let [SimulatorDeviceState::Operational(snapshot)] = states.as_slice() else {
        panic!("disk-only restart did not reconstruct an operational replica");
    };
    assert_eq!(snapshot.pages.len(), 1);
    assert_eq!(snapshot.pages[0].0, ids.page_a);
    assert_eq!(snapshot.pages[0].1.path(), Some(&path("pages/Disk.md")));
    assert_eq!(
        std::fs::read(inbox.join("manifests/from-disk/batch")).unwrap(),
        batch.manifest.bytes_b64.0
    );
    let expected_manifest_digest = format!("{:x}", Sha256::digest(&batch.manifest.bytes_b64.0));
    let provider = simulator.provider_snapshots().unwrap();
    assert!(provider[0].entries.iter().any(|entry| {
        entry.path == "manifests/from-disk/batch" && entry.digest == expected_manifest_digest
    }));
    assert_eq!(
        simulator.provider_ingress_receipts().len(),
        batch.objects.len() + 1
    );
}

#[test]
fn filesystem_rescan_propagates_malformed_canonical_bytes_with_stable_receipt() {
    let ids = Ids::new();
    let scenario = Scenario::from_schedule(
        "filesystem-provider-malformed-rescan",
        154,
        ids.workspace(),
        vec![device("beta", 2)],
        Vec::new(),
        Vec::new(),
        vec![event(
            1,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        )],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    let inbox = simulator
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    std::fs::write(inbox.join("objects/malformed"), b"not an operation object").unwrap();

    assert!(matches!(simulator.run(), Err(ScenarioError::Store(_))));
    let receipt = simulator
        .provider_ingress_receipts()
        .get(&(1, "objects/malformed".into()))
        .unwrap();
    assert!(!receipt.accepted);
    assert_eq!(receipt.item_id, "provider/inbox/objects/malformed");
}

#[test]
fn filesystem_unknown_top_namespace_is_diagnostic_residue_only() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        1541,
        1541,
        ids.page_a,
        ids.home_a,
        "Residue",
        "pages/Residue.md",
    );
    let scenario = Scenario::from_schedule(
        "filesystem-provider-unknown-residue",
        1541,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        vec![event(
            1,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        )],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    let inbox = simulator
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    std::fs::create_dir(inbox.join("unknown")).unwrap();
    std::fs::write(
        inbox.join("unknown/valid-manifest-bytes"),
        &batch.manifest.bytes_b64.0,
    )
    .unwrap();

    simulator.run().unwrap();
    assert!(simulator.provider_ingress_receipts().is_empty());
    let states = simulator.states().unwrap();
    let [SimulatorDeviceState::Operational(snapshot)] = states.as_slice() else {
        panic!("residue-only scan changed workspace status");
    };
    assert_eq!(snapshot, &Default::default());
    let provider = simulator.provider_snapshots().unwrap();
    assert!(provider[0]
        .entries
        .iter()
        .any(|entry| entry.path == "unknown/valid-manifest-bytes" && entry.item_kind.is_none()));
}

#[test]
fn filesystem_complete_copy_into_internal_provider_namespaces_is_rejected() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        155,
        155,
        ids.page_a,
        ids.home_a,
        "Part",
        "pages/Part.md",
    );
    for path in [
        ".part/complete-copy.part",
        "removed/forged",
        "rename-evidence/forged",
    ] {
        let result = Scenario::from_schedule(
            "filesystem-provider-direct-internal-copy",
            155,
            ids.workspace(),
            vec![device("beta", 2)],
            vec![batch.clone()],
            Vec::new(),
            vec![provider_copy(
                1,
                &batch.objects[0].item_id,
                provider_location("beta", ProviderTree::Inbox, path),
            )],
            Vec::new(),
            Vec::new(),
        );
        assert!(
            matches!(result, Err(ScenarioError::InvalidProviderPath(_))),
            "{path}"
        );
    }
}

#[test]
fn filesystem_provider_temporary_creation_is_exclusive_and_non_truncating() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        1551,
        1551,
        ids.page_a,
        ids.home_a,
        "Temp",
        "pages/Temp.md",
    );
    let scenario = Scenario::from_schedule(
        "filesystem-provider-temp-collision",
        1551,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        vec![event(
            1,
            ScheduledActionKind::BeginProviderWrite {
                source: ProviderSource::Mailbox {
                    item_id: batch.objects[0].item_id.clone(),
                },
                destination: provider_location("beta", ProviderTree::Inbox, "objects/destination"),
                transfer_id: "collision".into(),
            },
        )],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    let temporary = simulator
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap()
        .join(".part/collision.part");
    std::fs::write(&temporary, b"must not be truncated").unwrap();

    assert!(matches!(
        simulator.run(),
        Err(ScenarioError::UnsafeProviderEntry(_))
    ));
    assert_eq!(std::fs::read(temporary).unwrap(), b"must not be truncated");
}

#[test]
fn filesystem_same_bytes_relabel_requires_visible_rename_and_then_fails_validation() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        156,
        156,
        ids.page_a,
        ids.home_a,
        "Relabel",
        "pages/Relabel.md",
    );
    let scenario = Scenario::from_schedule(
        "filesystem-provider-visible-relabel",
        156,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        vec![
            provider_copy(
                1,
                &batch.objects[0].item_id,
                provider_location("beta", ProviderTree::Inbox, "objects/relabel"),
            ),
            event(
                2,
                ScheduledActionKind::ProviderRename {
                    device: "beta".into(),
                    tree: ProviderTree::Inbox,
                    from_path: "objects/relabel".into(),
                    to_path: "manifests/relabel".into(),
                },
            ),
            event(
                3,
                ScheduledActionKind::ReceiverRescan {
                    device: "beta".into(),
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    let inbox = simulator
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    assert!(matches!(simulator.run(), Err(ScenarioError::Store(_))));
    assert!(!inbox.join("objects/relabel").exists());
    assert_eq!(
        std::fs::read(inbox.join("manifests/relabel")).unwrap(),
        batch.objects[0].bytes_b64.0
    );
    std::fs::write(inbox.join("objects/relabel"), b"later source mutation").unwrap();
    assert_eq!(
        std::fs::read(inbox.join("manifests/relabel")).unwrap(),
        batch.objects[0].bytes_b64.0
    );
}

#[cfg(unix)]
#[test]
fn filesystem_provider_rejects_intermediate_and_final_symlinks_and_hardlinks() {
    use std::os::unix::fs::symlink;

    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        157,
        157,
        ids.page_a,
        ids.home_a,
        "Links",
        "pages/Links.md",
    );
    let make_simulator = |path: &str| {
        let scenario = Scenario::from_schedule(
            "filesystem-provider-link-confinement",
            157,
            ids.workspace(),
            vec![device("beta", 2)],
            vec![batch.clone()],
            Vec::new(),
            vec![provider_copy(
                1,
                &batch.objects[0].item_id,
                provider_location("beta", ProviderTree::Inbox, path),
            )],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        DeterministicSimulator::new(scenario).unwrap()
    };

    let outside = std::env::temp_dir().join(format!("tine-provider-link-test-{}", Uuid::new_v4()));
    std::fs::create_dir(&outside).unwrap();

    let mut intermediate = make_simulator("objects/escape/item");
    let intermediate_root = intermediate
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    symlink(&outside, intermediate_root.join("objects/escape")).unwrap();
    assert!(matches!(
        intermediate.run(),
        Err(ScenarioError::UnsafeProviderEntry(_))
    ));
    assert!(!outside.join("item").exists());

    let mut final_link = make_simulator("objects/final-link");
    let final_root = final_link
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    let outside_file = outside.join("outside");
    std::fs::write(&outside_file, b"untouched").unwrap();
    symlink(&outside_file, final_root.join("objects/final-link")).unwrap();
    assert!(matches!(
        final_link.run(),
        Err(ScenarioError::UnsafeProviderEntry(_))
    ));
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"untouched");

    let mut hardlink = make_simulator("objects/hardlink");
    let hardlink_root = hardlink
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    std::fs::hard_link(&outside_file, hardlink_root.join("objects/hardlink")).unwrap();
    assert!(matches!(
        hardlink.run(),
        Err(ScenarioError::UnsafeProviderEntry(_))
    ));
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"untouched");

    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn filesystem_rescan_enforces_depth_and_actual_byte_bounds() {
    let ids = Ids::new();
    let make_simulator = || {
        let scenario = Scenario::from_schedule(
            "filesystem-provider-rescan-bounds",
            158,
            ids.workspace(),
            vec![device("beta", 2)],
            Vec::new(),
            Vec::new(),
            vec![event(
                1,
                ScheduledActionKind::ReceiverRescan {
                    device: "beta".into(),
                },
            )],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        DeterministicSimulator::new(scenario).unwrap()
    };

    let mut deep = make_simulator();
    let deep_root = deep
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    let mut deep_path = deep_root.join("objects");
    for index in 0..=MAX_PROVIDER_RESCAN_DEPTH {
        deep_path.push(format!("d{index}"));
    }
    std::fs::create_dir_all(&deep_path).unwrap();
    std::fs::write(deep_path.join("object"), b"x").unwrap();
    assert!(matches!(
        deep.run(),
        Err(ScenarioError::ProviderRescanLimit)
    ));

    let mut large = make_simulator();
    let large_root = large
        .provider_tree_path("beta", ProviderTree::Inbox)
        .unwrap();
    let large_file = std::fs::File::create(large_root.join("objects/oversized")).unwrap();
    large_file
        .set_len(u64::try_from(MAX_PROVIDER_RESCAN_BYTES).unwrap() + 1)
        .unwrap();
    assert!(matches!(
        large.run(),
        Err(ScenarioError::ProviderRescanLimit)
    ));
}

#[test]
fn filesystem_two_phase_rescan_accepts_manifest_before_or_after_objects() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        159,
        159,
        ids.page_a,
        ids.home_a,
        "Ordering",
        "pages/Ordering.md",
    );
    let run_order = |manifest_first: bool| {
        let mut actions = Vec::new();
        let mut next = 1;
        if manifest_first {
            actions.push(provider_copy(
                next,
                &batch.manifest.item_id,
                provider_location("beta", ProviderTree::Inbox, "manifests/batch"),
            ));
            next += 1;
        }
        for (index, object) in batch.objects.iter().enumerate() {
            actions.push(provider_copy(
                next,
                &object.item_id,
                provider_location(
                    "beta",
                    ProviderTree::Inbox,
                    format!("objects/object-{index}"),
                ),
            ));
            next += 1;
        }
        if !manifest_first {
            actions.push(provider_copy(
                next,
                &batch.manifest.item_id,
                provider_location("beta", ProviderTree::Inbox, "manifests/batch"),
            ));
            next += 1;
        }
        actions.push(event(
            next,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        ));
        let scenario = Scenario::from_schedule(
            "filesystem-provider-two-phase-order",
            159,
            ids.workspace(),
            vec![device("beta", 2)],
            vec![batch.clone()],
            Vec::new(),
            actions,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        simulator.run().unwrap();
        simulator.states().unwrap()
    };

    let manifest_first = run_order(true);
    let object_first = run_order(false);
    assert_eq!(manifest_first, object_first);
    let [SimulatorDeviceState::Operational(snapshot)] = object_first.as_slice() else {
        panic!("two-phase rescan did not accept the complete batch");
    };
    assert_eq!(snapshot.pages[0].1.path(), Some(&path("pages/Ordering.md")));
}

#[test]
fn filesystem_provider_fixture_matches_deterministic_authored_scenario() {
    let ids = Ids::new();
    let batch = create_page_batch(
        ids,
        160,
        160,
        ids.page_a,
        ids.home_a,
        "Fixture",
        "pages/Fixture.md",
    );
    let mut reference_actions = Vec::new();
    let mut reference_next = 1;
    deliver_all(&mut reference_actions, &mut reference_next, "beta", &batch);
    let reference = Scenario::from_schedule(
        "filesystem-provider-fixture-reference",
        160,
        ids.workspace(),
        vec![device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        reference_actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut reference = DeterministicSimulator::new(reference).unwrap();
    reference.run().unwrap();
    let reference_states = reference.states().unwrap();
    let [SimulatorDeviceState::Operational(expected_snapshot)] = reference_states.as_slice() else {
        panic!("reference fixture batch was not operational");
    };

    let mut actions = vec![
        provider_copy(
            1,
            &batch.objects[0].item_id,
            provider_location("beta", ProviderTree::Inbox, "objects/object-0"),
        ),
        event(
            2,
            ScheduledActionKind::BeginProviderWrite {
                source: ProviderSource::Mailbox {
                    item_id: batch.objects[0].item_id.clone(),
                },
                destination: provider_location(
                    "beta",
                    ProviderTree::Inbox,
                    "objects/abandoned-partial",
                ),
                transfer_id: "fixture-partial".into(),
            },
        ),
        event(
            3,
            ScheduledActionKind::AppendProviderWrite {
                device: "beta".into(),
                transfer_id: "fixture-partial".into(),
                len: batch.objects[0].bytes_b64.0.len() / 2,
            },
        ),
        event(
            4,
            ScheduledActionKind::ReceiverRescan {
                device: "beta".into(),
            },
        ),
    ];
    let mut next = 5;
    for (index, object) in batch.objects.iter().enumerate().skip(1) {
        actions.push(provider_copy(
            next,
            &object.item_id,
            provider_location(
                "beta",
                ProviderTree::Inbox,
                format!("objects/object-{index}"),
            ),
        ));
        next += 1;
    }
    actions.push(provider_copy(
        next,
        &batch.manifest.item_id,
        provider_location("beta", ProviderTree::Inbox, "manifests/batch"),
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::SetProviderPartition {
            device: "beta".into(),
            partitioned: true,
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::NoVisibleEffect {
                device: "beta".into(),
                snapshot: Default::default(),
            },
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::SetProviderPartition {
            device: "beta".into(),
            partitioned: false,
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::Replica {
                device: "beta".into(),
                expected: ReplicaExpectation {
                    accepted: vec![batch.batch_id],
                    offered: vec![batch.batch_id],
                    state: ExpectedWorkspaceState::Operational,
                    snapshot: Some(expected_snapshot.clone()),
                },
            },
        },
    ));
    next += 1;
    actions.push(provider_copy(
        next,
        &batch.objects[0].item_id,
        provider_location("beta", ProviderTree::Inbox, "objects/object-0-duplicate"),
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
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
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ReceiverRescan {
            device: "beta".into(),
        },
    ));
    next += 1;
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
            assertion: InvariantAssertion::ProviderResidue {
                device: "beta".into(),
                // Complete publication consumes named or anonymous staging;
                // only explicit provider files and abandoned partials remain.
                max_entries: batch.objects.len() * 2 + 4,
                max_bytes: MAX_PROVIDER_RESCAN_BYTES,
            },
        },
    ));
    let fixture = Scenario::from_schedule(
        "filesystem-provider-transport",
        160,
        ids.workspace(),
        vec![device("alpha", 1), device("beta", 2)],
        vec![batch.clone()],
        Vec::new(),
        actions,
        vec![tine_core::oplog::simulator::InitialReplica {
            device: "beta".into(),
            stored_items: Vec::new(),
            expected: ReplicaExpectation {
                accepted: vec![batch.batch_id],
                offered: vec![batch.batch_id],
                state: ExpectedWorkspaceState::Operational,
                snapshot: Some(expected_snapshot.clone()),
            },
        }],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        fixture.encode().unwrap(),
        include_str!("fixtures/oplog-simulator/filesystem-provider-transport.scenario.json")
            .trim_end()
            .as_bytes()
    );
}

#[test]
fn filesystem_provider_fixture_executes_real_transport_and_terminal_oracles() {
    let fixture =
        include_str!("fixtures/oplog-simulator/filesystem-provider-transport.scenario.json")
            .trim_end();
    let scenario = Scenario::decode(fixture.as_bytes()).unwrap();
    let expected_manifest_digest = format!(
        "{:x}",
        Sha256::digest(&scenario.wire_batches[0].manifest.bytes_b64.0)
    );
    let partition_index = scenario
        .actions
        .iter()
        .position(|action| {
            matches!(
                action.action,
                ScheduledActionKind::SetProviderPartition {
                    partitioned: true,
                    ..
                }
            )
        })
        .expect("fixture omitted the partition");
    let blocked_rescan_event = match &scenario.actions[partition_index + 1] {
        ScheduledAction {
            event_id,
            action: ScheduledActionKind::ReceiverRescan { device },
            ..
        } if device == "beta" => *event_id,
        _ => panic!("fixture omitted the blocked beta rescan"),
    };
    let complete_copies_before_partition = scenario.actions[..partition_index]
        .iter()
        .filter(|action| {
            matches!(
                &action.action,
                ScheduledActionKind::ProviderCopy {
                    destination: ProviderLocation {
                        device,
                        tree: ProviderTree::Inbox,
                        ..
                    },
                    ..
                } if device == "beta"
            )
        })
        .count();
    assert_eq!(
        complete_copies_before_partition,
        scenario.wire_batches[0].objects.len() + 1,
        "the complete batch must already be disk-visible before partitioning"
    );
    let rejoin_index = scenario
        .actions
        .iter()
        .position(|action| {
            matches!(
                action.action,
                ScheduledActionKind::SetProviderPartition {
                    partitioned: false,
                    ..
                }
            )
        })
        .expect("fixture omitted the rejoin");
    let rejoined_rescan_event = match &scenario.actions[rejoin_index + 1] {
        ScheduledAction {
            event_id,
            action: ScheduledActionKind::ReceiverRescan { device },
            ..
        } if device == "beta" => *event_id,
        _ => panic!("fixture omitted the post-rejoin beta rescan"),
    };
    assert!(!scenario.wire_batches.is_empty());
    assert!(scenario.actions.iter().any(|action| matches!(
        action.action,
        ScheduledActionKind::BeginProviderWrite { .. }
    )));
    assert!(scenario
        .actions
        .iter()
        .any(|action| matches!(action.action, ScheduledActionKind::Crash { .. })));
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    assert!(
        !simulator
            .provider_ingress_receipts()
            .keys()
            .any(|(event_id, _)| *event_id == blocked_rescan_event),
        "partitioned rescan produced an ingestion receipt"
    );
    assert!(
        simulator
            .provider_ingress_receipts()
            .keys()
            .any(|(event_id, _)| *event_id == rejoined_rescan_event),
        "post-rejoin rescan did not ingest the same disk-visible bytes"
    );
    let snapshot = simulator.provider_snapshots().unwrap();
    let beta = snapshot
        .iter()
        .find(|snapshot| snapshot.device == "beta")
        .unwrap();
    assert!(beta.entries.iter().any(|entry| entry.temporary));
    assert!(beta
        .entries
        .iter()
        .any(|entry| entry.item_kind
            == Some(tine_core::oplog::simulator::ProviderItemKind::Manifest)));
    assert!(beta.entries.iter().any(|entry| {
        entry.path == "manifests/batch" && entry.digest == expected_manifest_digest
    }));
    let states = simulator.states().unwrap();
    let semantic = states
        .iter()
        .find_map(|state| match state {
            SimulatorDeviceState::Operational(snapshot) if !snapshot.pages.is_empty() => {
                Some(snapshot)
            }
            _ => None,
        })
        .expect("fixture did not finish with the expected operational replica");
    assert_eq!(semantic.pages.len(), 1);
    assert_eq!(semantic.pages[0].1.path(), Some(&path("pages/Fixture.md")));
}

#[test]
fn fixture_seed_corpus_is_canonical_v5_json() {
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
        include_str!("fixtures/oplog-simulator/filesystem-provider-transport.scenario.json"),
        include_str!("fixtures/oplog-simulator/page-name-conflict-restart.scenario.json"),
        include_str!("fixtures/oplog-simulator/coordinator-v5-nested-retry.scenario.json"),
    ];
    for fixture in fixtures {
        let fixture = fixture.trim_end();
        let scenario = Scenario::decode(fixture.as_bytes()).unwrap();
        assert_eq!(scenario.encode().unwrap(), fixture.as_bytes());
    }
}

#[test]
fn coordinator_v5_fixture_replays_real_storage_retry() {
    let scenario = Scenario::decode(
        include_str!("fixtures/oplog-simulator/coordinator-v5-nested-retry.scenario.json")
            .trim_end()
            .as_bytes(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let observed = simulator.coordinator_observations().unwrap();
    let alpha = observed.get("alpha").unwrap();
    assert_eq!(alpha.accepted_sequence, 2);
    assert_eq!(alpha.sqlite_sequence, Some(2));
    assert_eq!(alpha.handoff, CoordinatorHandoffState::Released);
    assert_eq!(alpha.pending_projection_work, 0);
}

fn page_name_conflict_restart_scenario() -> Scenario {
    let ids = Ids::new();
    let base = BatchId::from_uuid(uuid(400));
    let left = BatchId::from_uuid(uuid(401));
    let right = BatchId::from_uuid(uuid(402));
    let mut actions = vec![event(
        1,
        ScheduledActionKind::AuthorLocal {
            device: "alpha".into(),
            batch_id: base,
            session_id: SessionId::from_uuid(uuid(4_400)),
            transaction: tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_c,
                home_document_id: ids.home_c,
                name: tine_core::oplog::LogicalPageName::parse("Seed").unwrap(),
                path: path("pages/seed.md"),
                kind: ManagedTextKind::Page,
            }]),
        },
    )];
    let mut next = 2;
    for index in 0..3 {
        actions.push(event(
            next,
            ScheduledActionKind::DeliverItem {
                device: "beta".into(),
                item_id: format!("auth/{base}/object/{index}"),
                mutation: ByteMutation::Exact,
                expected: None,
            },
        ));
        next += 1;
    }
    actions.push(event(
        next,
        ScheduledActionKind::DeliverItem {
            device: "beta".into(),
            item_id: format!("auth/{base}/manifest/0"),
            mutation: ByteMutation::Exact,
            expected: None,
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ProbeBatch {
            device: "beta".into(),
            batch_id: base,
            expected: Some(StageExpectation::Accepted),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Crash {
            device: "alpha".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Restart {
            device: "alpha".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::RestartReplay {
                device: "alpha".into(),
            },
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AuthorLocal {
            device: "alpha".into(),
            batch_id: left,
            session_id: SessionId::from_uuid(uuid(4_401)),
            transaction: tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_a,
                home_document_id: ids.home_a,
                name: tine_core::oplog::LogicalPageName::parse("Concurrent Shared").unwrap(),
                path: path("pages/left.md"),
                kind: ManagedTextKind::Page,
            }]),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AuthorLocal {
            device: "beta".into(),
            batch_id: right,
            session_id: SessionId::from_uuid(uuid(4_402)),
            transaction: tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_b,
                home_document_id: ids.home_b,
                name: tine_core::oplog::LogicalPageName::parse("concurrent shared").unwrap(),
                path: path("pages/right.md"),
                kind: ManagedTextKind::Page,
            }]),
        },
    ));
    next += 1;
    for index in 0..3 {
        actions.push(event(
            next,
            ScheduledActionKind::DeliverItem {
                device: "alpha".into(),
                item_id: format!("auth/{right}/object/{index}"),
                mutation: ByteMutation::Exact,
                expected: None,
            },
        ));
        next += 1;
    }
    actions.push(event(
        next,
        ScheduledActionKind::DeliverItem {
            device: "alpha".into(),
            item_id: format!("auth/{right}/manifest/0"),
            mutation: ByteMutation::Exact,
            expected: None,
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::ProbeBatch {
            device: "alpha".into(),
            batch_id: right,
            expected: Some(StageExpectation::Quarantined),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Crash {
            device: "alpha".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::Restart {
            device: "alpha".into(),
        },
    ));
    next += 1;
    actions.push(event(
        next,
        ScheduledActionKind::AssertInvariant {
            assertion: InvariantAssertion::RestartReplay {
                device: "alpha".into(),
            },
        },
    ));
    Scenario::from_schedule(
        "page-name-conflict-restart",
        400,
        ids.workspace(),
        vec![device("alpha", 1), device("beta", 2)],
        Vec::new(),
        Vec::new(),
        actions,
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn page_name_conflict_restart_uses_typed_durable_evidence() {
    let scenario = page_name_conflict_restart_scenario();
    let mut simulator = DeterministicSimulator::new(scenario.clone()).unwrap();
    simulator.run().unwrap();
    assert_eq!(
        scenario.encode().unwrap(),
        include_str!("fixtures/oplog-simulator/page-name-conflict-restart.scenario.json")
            .trim_end()
            .as_bytes()
    );
    let states = simulator.states().unwrap();
    assert!(states.into_iter().any(|state| matches!(
        state,
        SimulatorDeviceState::Blocked(SimulatorBlockedEvidence::PageName(evidence))
            if !evidence.is_empty()
    )));
}

#[test]
fn scenario_decode_rejects_all_non_v5_schema_versions() {
    for version in [0, 1, 2, 3, 4, 6, u32::MAX] {
        let bytes = format!(
            "{{\"scenario_schema_version\":{version},\"family\":\"version-gate\",\"seed\":1,\
             \"workspace\":{{\"workspace_id\":\"00000000-0000-0000-0000-000000000001\",\
             \"lineage_digest\":\"594b2ffe782f7984a7a1de511368306d352f22e6b3c0a67f73faf31b7bcb8c33\",\
             \"catalog_document_id\":\"00000000-0000-0000-0000-000000000002\"}},\
             \"devices\":[{{\"name\":\"alpha\",\"device_id\":\"00000000-0000-0000-0000-0000000003e9\",\
             \"crdt_peer_id\":1}}],\"wire_batches\":[],\"initial_replicas\":[],\"actions\":[],\
             \"terminal\":[],\"external_files\":[]}}"
        );
        assert!(
            Scenario::decode(bytes.as_bytes()).is_err(),
            "accepted v{version}"
        );
    }
}

#[test]
fn coordinator_v5_nested_success_and_projection_fault_are_replayable() {
    let ids = Ids::new();
    let scenario = Scenario::from_schedule(
        "coordinator-v5-nested-success-and-projection-fault",
        50_005,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: "content/pages/deep/projects/a.md".into(),
                        kind: ManagedTextKind::Page,
                        config_edn: Some(WireBytes(
                            b"{:pages-directory \"content/pages\"\n:journals-directory \"content/journals\"}\n"
                                .to_vec(),
                        )),
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: "content/pages/deep/projects/a.md".into(),
                        bytes_b64: WireBytes(b"- root edited\r\n\t- child edited\r\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec!["content/pages/deep/projects/a.md".into()],
                        fault: None,
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "before-projection-fault".into(),
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: "content/pages/deep/projects/a.md".into(),
                        bytes_b64: WireBytes(b"- root durable\n\t- child durable\n".to_vec()),
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec!["content/pages/deep/projects/a.md".into()],
                        fault: Some(CoordinatorFault::BeforeProjection),
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::HeldFailedClosed),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::FailedClosed {
                                phase: "ArchiveStage".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            sqlite_sequence: Some(3),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(1),
                            handoff: Some(CoordinatorHandoffState::HeldFailedClosed),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::FailedClosed {
                                phase: "ProjectionDrain".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                11,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                12,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            sqlite_sequence: Some(3),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let encoded = scenario.encode().unwrap();
    assert_eq!(Scenario::decode(&encoded).unwrap(), scenario);
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let observation = simulator
        .coordinator_observations()
        .unwrap()
        .remove("alpha")
        .unwrap();
    assert_eq!(observation.accepted_sequence, 3);
    assert_eq!(observation.sqlite_sequence, Some(3));
    assert!(observation.sqlite_row_digest.is_some());
    assert_eq!(observation.pending_projection_work, 0);
}

#[test]
fn coordinator_v5_blocked_and_noop_imports_preserve_all_durable_evidence() {
    let ids = Ids::new();
    let path = "pages/v5/noop/exact.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-blocked-noop-exact-evidence",
        50_051,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "settled-noop".into(),
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into(), path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(1),
                            sqlite_sequence: Some(1),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Blocked),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "settled-noop".into(),
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertCheckpoint {
                        name: "settled-noop".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_stale_draft_and_changed_capture_block_before_publication() {
    let ids = Ids::new();
    let path = "pages/v5/stale/draft-and-capture.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-stale-draft-and-capture",
        50_204,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- stale draft before capture\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::InterfereAt {
                        point: CoordinatorFault::AfterDraft,
                        path: path.into(),
                        bytes_b64: WireBytes(b"- stale draft replacement\n".to_vec()),
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(1),
                            sqlite_sequence: Some(1),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            managed_files: Some(vec![ExternalFileFixture {
                                path: path.into(),
                                bytes_b64: WireBytes(b"- stale draft replacement\n".to_vec()),
                            }]),
                            last_outcome: Some(CoordinatorRunOutcome::PrepublicationError {
                                phase: "Capture".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- changed receipt capture\n".to_vec()),
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::InterfereReceiptAt {
                        point: CoordinatorFault::AfterCapture,
                        path: path.into(),
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                11,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::PrepublicationError {
                                phase: "Finalize".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                12,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::RestoreInterferedReceipt,
                },
            ),
            event(
                13,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                14,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                15,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            sqlite_sequence: Some(3),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                16,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "stale-draft-capture-recovered".into(),
                    },
                },
            ),
            event(
                17,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                18,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "stale-draft-capture-recovered".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_acceptance_sequence_is_not_batch_id_order() {
    let ids = Ids::new();
    let path = "pages/v5/ordering/accepted.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-acceptance-order-not-batch-id-order",
        50_305,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- acceptance one zulu\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- acceptance two alpha\n".to_vec()),
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            accepted_frontier_digest: Some(
                                "9e7c9b8ff2334ace94a198d3e1c96bd20c4907153c496488fc797352cc316b26"
                                    .into(),
                            ),
                            accepted_batches: Some(vec![
                                BatchId::from_uuid(uuid(9)),
                                "e27bbc37-3fe4-846c-962b-07dea4588cf3".parse().unwrap(),
                                "490f9f16-e6ce-8f5b-8ab0-8e4a364ec8a6".parse().unwrap(),
                            ]),
                            sqlite_sequence: Some(3),
                            sqlite_frontier_digest: Some(
                                "9e7c9b8ff2334ace94a198d3e1c96bd20c4907153c496488fc797352cc316b26"
                                    .into(),
                            ),
                            sqlite_row_digest: Some(
                                "7bd0f37d5d5084a304f71a78e7299707482fa38e4f9a99a35c1a8e4210ef5552"
                                    .into(),
                            ),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "acceptance-order-complete".into(),
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "acceptance-order-complete".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let observed = simulator
        .coordinator_observations()
        .unwrap()
        .remove("alpha")
        .unwrap();
    assert_eq!(observed.accepted_sequence, 3);
    assert_eq!(observed.sqlite_sequence, Some(3));
    assert_eq!(
        observed.sqlite_frontier_digest,
        Some(observed.accepted_frontier_digest.clone())
    );
    assert!(observed.sqlite_row_digest.is_some());
    assert_eq!(
        observed.accepted_frontier_digest,
        "9e7c9b8ff2334ace94a198d3e1c96bd20c4907153c496488fc797352cc316b26"
    );
    assert_eq!(
        observed.sqlite_row_digest.as_deref(),
        Some("7bd0f37d5d5084a304f71a78e7299707482fa38e4f9a99a35c1a8e4210ef5552")
    );
    assert_eq!(observed.accepted_batches[0], BatchId::from_uuid(uuid(9)));
    assert_eq!(
        observed.accepted_batches,
        vec![
            BatchId::from_uuid(uuid(9)),
            "e27bbc37-3fe4-846c-962b-07dea4588cf3".parse().unwrap(),
            "490f9f16-e6ce-8f5b-8ab0-8e4a364ec8a6".parse().unwrap(),
        ]
    );
    assert!(
        observed.accepted_batches[1] > observed.accepted_batches[2],
        "acceptance order must deliberately differ from BatchId order: {:?}",
        observed.accepted_batches
    );
}

#[test]
fn coordinator_v5_sqlite_stale_frontier_delete_truncate_and_corruption_rebuild_exactly() {
    let ids = Ids::new();
    let path = "pages/deep/sqlite/rebuild.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-sqlite-rebuild",
        50_102,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- durable sqlite rebuild\n\t- nested\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "durable-materialization".into(),
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::StaleFrontier,
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            frontiers_match: Some(false),
                            read_gate: Some(CoordinatorReadGate::Closed),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Reopen,
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertMaterializationCheckpoint {
                        name: "durable-materialization".into(),
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Delete,
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            frontiers_match: Some(false),
                            read_gate: Some(CoordinatorReadGate::Closed),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                11,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Reopen,
                    },
                },
            ),
            event(
                12,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertMaterializationCheckpoint {
                        name: "durable-materialization".into(),
                    },
                },
            ),
            event(
                13,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Truncate { len: 0 },
                    },
                },
            ),
            event(
                14,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            frontiers_match: Some(false),
                            read_gate: Some(CoordinatorReadGate::Closed),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                15,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Reopen,
                    },
                },
            ),
            event(
                16,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertMaterializationCheckpoint {
                        name: "durable-materialization".into(),
                    },
                },
            ),
            event(
                17,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Corrupt {
                            offset: 0,
                            mask: 0xff,
                        },
                    },
                },
            ),
            event(
                18,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            frontiers_match: Some(false),
                            read_gate: Some(CoordinatorReadGate::Closed),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                19,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Sqlite {
                        mutation: CoordinatorSqliteMutation::Reopen,
                    },
                },
            ),
            event(
                20,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertMaterializationCheckpoint {
                        name: "durable-materialization".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_stale_plan_forces_recapture_before_publication() {
    let ids = Ids::new();
    let path = "pages/deep/stale/capture.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-stale-plan-recapture",
        50_203,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- stale first\n\t- nested\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::InterfereAt {
                        point: CoordinatorFault::AfterPlan,
                        path: path.into(),
                        bytes_b64: WireBytes(b"- stale replacement\n\t- nested\n".to_vec()),
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(1),
                            sqlite_sequence: Some(1),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            managed_files: Some(vec![ExternalFileFixture {
                                path: path.into(),
                                bytes_b64: WireBytes(b"- stale replacement\n\t- nested\n".to_vec()),
                            }]),
                            last_outcome: Some(CoordinatorRunOutcome::PrepublicationError {
                                phase: "Capture".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "stale-plan-recovered".into(),
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "stale-plan-recovered".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_rename_then_deletion_reconciles_exact_managed_paths() {
    let ids = Ids::new();
    let old = "pages/deep/rename/old.md";
    let new = "pages/deep/rename/new.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-rename-delete",
        50_304,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: old.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalRename {
                        from_path: old.into(),
                        to_path: new.into(),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![old.into(), new.into()],
                        fault: None,
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            managed_files: Some(vec![ExternalFileFixture {
                                path: new.into(),
                                bytes_b64: WireBytes(b"- root\n\t- child\n".to_vec()),
                            }]),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "renamed-complete".into(),
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![new.into()],
                        fault: None,
                    },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "renamed-complete".into(),
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalDelete { path: new.into() },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![new.into()],
                        fault: None,
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::HeldFailedClosed),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::FailedClosed {
                                phase: "ArchiveStage".into(),
                            }),
                            managed_files: Some(Vec::new()),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                11,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                12,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            sqlite_sequence: Some(3),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            managed_files: Some(Vec::new()),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                13,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "deleted-complete".into(),
                    },
                },
            ),
            event(
                14,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![new.into()],
                        fault: None,
                    },
                },
            ),
            event(
                15,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(3),
                            sqlite_sequence: Some(3),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Noop),
                            managed_files: Some(Vec::new()),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                16,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "deleted-complete".into(),
                    },
                },
            ),
            event(
                17,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "deleted-noop".into(),
                    },
                },
            ),
            event(
                18,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![new.into()],
                        fault: None,
                    },
                },
            ),
            event(
                19,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertCheckpoint {
                        name: "deleted-noop".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_projection_failure_after_acceptance_recovers_without_reaccepting() {
    let ids = Ids::new();
    let path = "pages/deep/projection/authoritative.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-projection-after-acceptance",
        50_406,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(
                            b"- accepted before projection\n\t- pending receipt\n".to_vec(),
                        ),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: Some(CoordinatorFault::DuringProjection),
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(1),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::HeldFailedClosed),
                            read_gate: Some(CoordinatorReadGate::Open),
                            durable_boundary: Some(
                                tine_core::oplog::CoordinatorDurableBoundary::DuringProjection,
                            ),
                            last_outcome: Some(CoordinatorRunOutcome::FailedClosed {
                                phase: "ProjectionDrain".into(),
                            }),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                5,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "accepted-pending-projection".into(),
                    },
                },
            ),
            event(
                6,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Retry { fault: None },
                },
            ),
            event(
                7,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertAcceptedArchiveCheckpoint {
                        name: "accepted-pending-projection".into(),
                    },
                },
            ),
            event(
                8,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Complete),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                9,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "projection-recovered".into(),
                    },
                },
            ),
            event(
                10,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                11,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(2),
                            sqlite_sequence: Some(2),
                            frontiers_match: Some(true),
                            pending_projection_work: Some(0),
                            tail_unapplied_batches: Some(0),
                            tail_retained_bytes: Some(0),
                            handoff: Some(CoordinatorHandoffState::Released),
                            read_gate: Some(CoordinatorReadGate::Open),
                            last_outcome: Some(CoordinatorRunOutcome::Noop),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
            event(
                12,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertDurableCheckpoint {
                        name: "projection-recovered".into(),
                    },
                },
            ),
            event(
                13,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Checkpoint {
                        name: "projection-noop".into(),
                    },
                },
            ),
            event(
                14,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: None,
                    },
                },
            ),
            event(
                15,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::AssertCheckpoint {
                        name: "projection-noop".into(),
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
}

#[test]
fn coordinator_v5_crash_reopen_reconstructs_every_durable_boundary_idempotently() {
    let boundaries = std::iter::once((CoordinatorFault::AfterObjects, false))
        .chain(
            [
                CoordinatorFault::AfterManifest,
                CoordinatorFault::AfterStage,
                CoordinatorFault::DuringSqliteApply,
                CoordinatorFault::AfterSqliteApply,
                CoordinatorFault::BeforeProjection,
                CoordinatorFault::DuringProjection,
                CoordinatorFault::AfterProjection,
            ]
            .into_iter()
            .map(|fault| (fault, true)),
        )
        .collect::<Vec<_>>();
    let ids = Ids::new();
    let bytes = WireBytes(b"- durable boundary\n\t- nested durable boundary\n".to_vec());

    for (index, (fault, published)) in boundaries.into_iter().enumerate() {
        let path = format!("pages/deep/fault-boundaries/{index}.md");
        let resume = if published {
            CoordinatorAction::Retry { fault: None }
        } else {
            CoordinatorAction::Execute {
                paths: vec![path.clone()],
                fault: None,
            }
        };
        let scenario = Scenario::from_schedule(
            format!("coordinator-v5-fault-boundary-{index}"),
            50_400 + index as u64,
            ids.workspace(),
            vec![device("alpha", 1)],
            Vec::new(),
            Vec::new(),
            vec![
                event(
                    1,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Setup {
                            managed_path: path.clone(),
                            kind: ManagedTextKind::Page,
                            config_edn: None,
                        },
                    },
                ),
                event(
                    2,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::ExternalWrite {
                            path: path.clone(),
                            bytes_b64: bytes.clone(),
                        },
                    },
                ),
                event(
                    3,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Execute {
                            paths: vec![path.clone()],
                            fault: Some(fault),
                        },
                    },
                ),
                event(
                    4,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Crash,
                    },
                ),
                event(
                    5,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Assert {
                            oracle: CoordinatorOracle {
                                handoff: Some(
                                    CoordinatorHandoffState::EnrollmentPendingUnprotected,
                                ),
                                durable_boundary: Some(match fault {
                                    CoordinatorFault::AfterObjects => {
                                        tine_core::oplog::CoordinatorDurableBoundary::AfterObjects
                                    }
                                    CoordinatorFault::AfterManifest => {
                                        tine_core::oplog::CoordinatorDurableBoundary::AfterManifest
                                    }
                                    CoordinatorFault::AfterStage => {
                                        tine_core::oplog::CoordinatorDurableBoundary::AfterStage
                                    }
                                    CoordinatorFault::DuringSqliteApply => {
                                        tine_core::oplog::CoordinatorDurableBoundary::
                                            DuringSqliteApply
                                    }
                                    CoordinatorFault::AfterSqliteApply => {
                                        tine_core::oplog::CoordinatorDurableBoundary::
                                            AfterSqliteApply
                                    }
                                    CoordinatorFault::BeforeProjection => {
                                        tine_core::oplog::CoordinatorDurableBoundary::
                                            BeforeProjection
                                    }
                                    CoordinatorFault::DuringProjection => {
                                        tine_core::oplog::CoordinatorDurableBoundary::
                                            DuringProjection
                                    }
                                    CoordinatorFault::AfterProjection => {
                                        tine_core::oplog::CoordinatorDurableBoundary::
                                            AfterProjection
                                    }
                                    _ => unreachable!(),
                                }),
                                ..CoordinatorOracle::default()
                            },
                        },
                    },
                ),
                event(
                    6,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Reopen,
                    },
                ),
                event(
                    7,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: resume,
                    },
                ),
                event(
                    8,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Assert {
                            oracle: CoordinatorOracle {
                                accepted_sequence: Some(2),
                                sqlite_sequence: Some(2),
                                frontiers_match: Some(true),
                                pending_projection_work: Some(0),
                                tail_unapplied_batches: Some(0),
                                tail_retained_bytes: Some(0),
                                handoff: Some(
                                    CoordinatorHandoffState::EnrollmentPendingUnprotected,
                                ),
                                read_gate: Some(CoordinatorReadGate::Open),
                                last_outcome: Some(CoordinatorRunOutcome::Complete),
                                managed_files: Some(vec![ExternalFileFixture {
                                    path: path.clone(),
                                    bytes_b64: bytes.clone(),
                                }]),
                                ..CoordinatorOracle::default()
                            },
                        },
                    },
                ),
                event(
                    9,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Checkpoint {
                            name: "recovered".into(),
                        },
                    },
                ),
                event(
                    10,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::Retry { fault: None },
                    },
                ),
                event(
                    11,
                    ScheduledActionKind::Coordinator {
                        device: "alpha".into(),
                        action: CoordinatorAction::AssertCheckpoint {
                            name: "recovered".into(),
                        },
                    },
                ),
            ],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let mut simulator = DeterministicSimulator::new(scenario).unwrap();
        simulator.run().unwrap();
    }
}

#[test]
fn coordinator_v5_failure_capsule_keeps_exact_durable_witness() {
    let ids = Ids::new();
    let path = "pages/capsule/nested.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-failure-capsule",
        50_512,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(999),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let minimized = scenario.minimize_failure(frozen_candidate()).unwrap();
    match &minimized.capsule.failure {
        tine_core::oplog::FailureIdentity::Invariant { signature, .. } => {
            assert_eq!(
                signature.predicate,
                InvariantPredicate::CoordinatorDurableState
            );
            assert_eq!(signature.assertion_or_event_id, 2);
        }
        other => panic!("unexpected failure identity: {other:?}"),
    }
    let witness = minimized.capsule.observed_coordinator.get("alpha").unwrap();
    assert_eq!(witness.accepted_sequence, 1);
    assert_eq!(witness.accepted_batches.len(), 1);
    assert!(!witness.accepted_frontier_digest.is_empty());
    assert_eq!(witness.sqlite_sequence, Some(1));
    assert_eq!(
        witness.sqlite_frontier_digest.as_deref(),
        Some(witness.accepted_frontier_digest.as_str())
    );
    assert!(witness.sqlite_row_digest.is_some());
    assert!(!witness.sqlite_files.is_empty());
    assert!(witness.managed_file_digests.contains_key(path));
    assert!(!witness.managed_files.is_empty());
    assert!(!witness.archive_files.is_empty());
    assert!(!witness.receipt_files.is_empty());
    assert_eq!(witness.pending_projection_work, 0);
    assert_eq!(witness.tail_unapplied_batches, 0);
    assert_eq!(witness.tail_retained_bytes, 0);
    assert_eq!(witness.handoff, CoordinatorHandoffState::Unused);
    assert_eq!(witness.read_gate, CoordinatorReadGate::Open);
    assert_eq!(minimized.capsule.minimized_scenario, minimized.scenario);
    assert!(!minimized.capsule.failure_message.is_empty());
    assert!(minimized.capsule.expected_coordinator.contains_key("alpha"));
    assert_eq!(
        minimized.capsule.durable_boundaries.get("alpha"),
        Some(&tine_core::oplog::CoordinatorDurableBoundary::Setup)
    );
    let encoded = minimized.capsule.encode().unwrap();
    let decoded = tine_core::oplog::simulator::FailureCapsule::decode(&encoded).unwrap();
    assert_eq!(decoded, minimized.capsule);
    assert_eq!(decoded.frozen_candidate, frozen_candidate());
    assert_eq!(
        decoded.replay(&frozen_candidate()).unwrap(),
        decoded.failure
    );
    let other_candidate =
        FrozenCandidateId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    assert!(matches!(
        decoded.replay(&other_candidate),
        Err(ScenarioError::FrozenCandidateMismatch)
    ));

    // V6 is intentionally a format transition: an old v5 capsule does not
    // silently parse until it happens to fail on the newly required field.
    let mut prior: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    prior["schema_version"] = serde_json::json!(5);
    prior.as_object_mut().unwrap().remove("frozen_candidate");
    assert!(matches!(
        tine_core::oplog::simulator::FailureCapsule::decode(&serde_json::to_vec(&prior).unwrap()),
        Err(ScenarioError::UnknownFailureCapsuleVersion(5))
    ));

    let mut future: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    future["schema_version"] = serde_json::json!(7);
    assert!(matches!(
        tine_core::oplog::simulator::FailureCapsule::decode(&serde_json::to_vec(&future).unwrap()),
        Err(ScenarioError::UnknownFailureCapsuleVersion(7))
    ));

    let mut missing: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    missing.as_object_mut().unwrap().remove("frozen_candidate");
    assert!(matches!(
        tine_core::oplog::simulator::FailureCapsule::decode(&serde_json::to_vec(&missing).unwrap()),
        Err(ScenarioError::Decode(_))
    ));
    assert!(matches!(
        FrozenCandidateId::parse("unknown"),
        Err(ScenarioError::InvalidFrozenCandidateId)
    ));
    assert!(matches!(
        FrozenCandidateId::parse("0000000000000000000000000000000000000000"),
        Err(ScenarioError::InvalidFrozenCandidateId)
    ));
    assert_eq!(
        FrozenCandidateId::parse(
            "bebab91b5aa509bf3147569a321b65274f7181e90425c580db3935f9665126a0"
        )
        .unwrap()
        .as_str(),
        "bebab91b5aa509bf3147569a321b65274f7181e90425c580db3935f9665126a0"
    );
    assert!(matches!(
        FrozenCandidateId::parse(
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        Err(ScenarioError::InvalidFrozenCandidateId)
    ));
}

#[test]
fn coordinator_v5_projection_fault_capsule_records_authoritative_pending_work() {
    let ids = Ids::new();
    let path = "pages/capsule/projection-pending.md";
    let scenario = Scenario::from_schedule(
        "coordinator-v5-projection-fault-capsule",
        50_514,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: path.into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::ExternalWrite {
                        path: path.into(),
                        bytes_b64: WireBytes(b"- capsule accepted\n\t- pending\n".to_vec()),
                    },
                },
            ),
            event(
                3,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Execute {
                        paths: vec![path.into()],
                        fault: Some(CoordinatorFault::DuringProjection),
                    },
                },
            ),
            event(
                4,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            pending_projection_work: Some(0),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let minimized = scenario.minimize_failure(frozen_candidate()).unwrap();
    match &minimized.capsule.failure {
        tine_core::oplog::FailureIdentity::Invariant { signature, .. } => {
            assert_eq!(
                signature.predicate,
                InvariantPredicate::CoordinatorDurableState
            );
            assert_eq!(signature.assertion_or_event_id, 4);
        }
        other => panic!("unexpected failure identity: {other:?}"),
    }
    assert!(minimized.scenario.actions.iter().any(|event| {
        matches!(
            event.action,
            ScheduledActionKind::Coordinator {
                action: CoordinatorAction::Execute {
                    fault: Some(CoordinatorFault::DuringProjection),
                    ..
                },
                ..
            }
        )
    }));
    let witness = minimized.capsule.observed_coordinator.get("alpha").unwrap();
    assert_eq!(witness.accepted_sequence, 2);
    assert_eq!(witness.accepted_batches.len(), 2);
    assert!(!witness.accepted_frontier_digest.is_empty());
    assert_eq!(witness.sqlite_sequence, Some(2));
    assert_eq!(
        witness.sqlite_frontier_digest.as_deref(),
        Some(witness.accepted_frontier_digest.as_str())
    );
    assert!(witness.sqlite_row_digest.is_some());
    assert!(!witness.sqlite_files.is_empty());
    assert!(!witness.managed_files.is_empty());
    assert!(!witness.archive_files.is_empty());
    assert!(!witness.receipt_files.is_empty());
    assert_eq!(witness.pending_projection_work, 1);
    assert_eq!(witness.tail_unapplied_batches, 0);
    assert_eq!(witness.tail_retained_bytes, 0);
    assert_eq!(witness.handoff, CoordinatorHandoffState::HeldFailedClosed);
    assert_eq!(witness.read_gate, CoordinatorReadGate::Open);
    assert_eq!(
        witness.durable_boundary,
        tine_core::oplog::CoordinatorDurableBoundary::DuringProjection
    );
    match minimized.capsule.expected_coordinator.get("alpha").unwrap() {
        tine_core::oplog::simulator::CoordinatorExpectedState::Oracle(oracle) => {
            assert_eq!(oracle.pending_projection_work, Some(0));
        }
        other => panic!("unexpected coordinator expectation: {other:?}"),
    }
    assert_eq!(minimized.capsule.minimized_scenario, minimized.scenario);
    let encoded = minimized.capsule.encode().unwrap();
    let decoded = tine_core::oplog::simulator::FailureCapsule::decode(&encoded).unwrap();
    assert_eq!(
        decoded.replay(&frozen_candidate()).unwrap(),
        decoded.failure
    );
}

#[test]
fn coordinator_oracle_capsules_match_semantics_across_host_bound_receipts() {
    let ids = Ids::new();
    let scenario = Scenario::from_schedule(
        "coordinator-oracle-stable-capsule-identity",
        50_513,
        ids.workspace(),
        vec![device("alpha", 1)],
        Vec::new(),
        Vec::new(),
        vec![
            event(
                1,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Setup {
                        managed_path: "pages/capsule/host-bound-receipt.md".into(),
                        kind: ManagedTextKind::Page,
                        config_edn: None,
                    },
                },
            ),
            event(
                2,
                ScheduledActionKind::Coordinator {
                    device: "alpha".into(),
                    action: CoordinatorAction::Assert {
                        oracle: CoordinatorOracle {
                            accepted_sequence: Some(999),
                            // An empty expected set deliberately fails against
                            // the durable enrollment receipt. Its bytes bind
                            // the temporary graph/receipt-root identity.
                            receipt_files: Some(Vec::new()),
                            ..CoordinatorOracle::default()
                        },
                    },
                },
            ),
        ],
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let first = scenario.minimize_failure(frozen_candidate()).unwrap();
    let second = scenario.minimize_failure(frozen_candidate()).unwrap();
    assert_eq!(first.capsule.failure, second.capsule.failure);
    assert_eq!(
        first.capsule.replay(&frozen_candidate()).unwrap(),
        first.capsule.failure
    );

    // Keep both random roots live concurrently so their graph/receipt
    // filesystem identities cannot be reused between observations.
    let mut first_root = DeterministicSimulator::new(scenario.clone()).unwrap();
    let mut second_root = DeterministicSimulator::new(scenario.clone()).unwrap();
    let first_error = first_root.run().unwrap_err();
    let second_error = second_root.run().unwrap_err();
    assert_eq!(
        first_error.failure_identity(),
        second_error.failure_identity()
    );
    let first_observed = first_root
        .coordinator_observations()
        .unwrap()
        .remove("alpha")
        .unwrap();
    let second_observed = second_root
        .coordinator_observations()
        .unwrap()
        .remove("alpha")
        .unwrap();
    assert!(!first_observed.receipt_files.is_empty());
    assert!(!second_observed.receipt_files.is_empty());
    assert_ne!(first_observed.receipt_files, second_observed.receipt_files);
    assert!(!first
        .capsule
        .observed_coordinator
        .get("alpha")
        .unwrap()
        .receipt_files
        .is_empty());
    let directory_identity = b"directory_identity";
    assert!(first
        .capsule
        .observed_coordinator
        .get("alpha")
        .unwrap()
        .receipt_files
        .iter()
        .any(|file| {
            file.bytes_b64
                .0
                .windows(directory_identity.len())
                .any(|bytes| bytes == directory_identity)
        }));
    assert!(matches!(
        first.capsule.expected_coordinator.get("alpha"),
        Some(tine_core::oplog::CoordinatorExpectedState::Oracle(CoordinatorOracle {
            receipt_files: Some(files),
            ..
        })) if files.is_empty()
    ));

    let mut different_oracle = scenario.clone();
    let ScheduledActionKind::Coordinator {
        action: CoordinatorAction::Assert { oracle },
        ..
    } = &mut different_oracle.actions[1].action
    else {
        panic!("stable identity scenario must end with a coordinator oracle");
    };
    oracle.accepted_sequence = Some(998);
    let different = different_oracle
        .minimize_failure(frozen_candidate())
        .unwrap();
    assert_ne!(first.capsule.failure, different.capsule.failure);
}
