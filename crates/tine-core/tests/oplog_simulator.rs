use sha2::{Digest, Sha256};
use tine_core::oplog::simulator::{
    ByteMutation, DeterministicSimulator, ExpectedWorkspaceState, ExternalFileFixture,
    IngressExpectation, InvariantAssertion, InvariantPredicate, ProviderLocation, ProviderSource,
    ProviderTree, ReplicaExpectation, ScenarioError, ScenarioWorkspace, ScheduledAction,
    ScheduledActionKind, SimulatorDeviceState, StageExpectation, WireBatch, WireBytes, WireItem,
    MAX_PROVIDER_RESCAN_BYTES, MAX_PROVIDER_RESCAN_DEPTH,
};
use tine_core::oplog::{
    AuthorBatch, BatchId, BlockId, BlockLocation, CrdtPeerId, DeviceId, DocumentId, LineageDigest,
    ManagedPath, OperationTransaction, PageId, Scenario, ScenarioDevice, SemanticOperation,
    SessionId, ShardedHotEngine, WorkspaceId,
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
    name: &str,
) -> WireBatch {
    wire_batch(
        ids,
        BatchId::from_uuid(uuid(batch)),
        peer,
        tx(vec![SemanticOperation::CreatePage {
            page_id: page,
            home_document_id: home,
            path: path(name),
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
    let batch = create_page_batch(ids, 100, 100, ids.page_a, ids.home_a, "pages/A.md");
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
    let batch = create_page_batch(ids, 110, 110, ids.page_a, ids.home_a, "pages/A.md");
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
    let first = create_page_batch(ids, 120, 120, ids.page_a, ids.home_a, "pages/A.md");
    let second = create_page_batch(ids, 121, 121, ids.page_b, ids.home_b, "pages/B.md");
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
    let batch = create_page_batch(ids, 130, 130, ids.page_a, ids.home_a, "pages/A.md");
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
    let root = create_page_batch(ids, 135, 135, ids.page_a, ids.home_a, "pages/A.md");
    let foreign = wire_batch_with_lineage(
        ids,
        LineageDigest::of(b"foreign independent genesis"),
        BatchId::from_uuid(uuid(136)),
        136,
        tx(vec![SemanticOperation::CreatePage {
            page_id: ids.page_b,
            home_document_id: ids.home_b,
            path: path("pages/Foreign.md"),
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
                    path: path("pages/A.md"),
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
                        path: path("pages/A.md"),
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
            path: path("pages/A.md"),
        },
        SemanticOperation::CreatePage {
            page_id: ids.page_b,
            home_document_id: ids.home_b,
            path: path("pages/B.md"),
        },
        SemanticOperation::CreatePage {
            page_id: ids.page_c,
            home_document_id: ids.home_c,
            path: path("pages/C.md"),
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
    let root = create_page_batch(ids, 140, 140, ids.page_a, ids.home_a, "pages/A.md");
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
                    path: path("pages/A.md"),
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
    let batch = create_page_batch(ids, 150, 150, ids.page_a, ids.home_a, "pages/A.md");
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
    let first = create_page_batch(ids, 151, 151, ids.page_a, ids.home_a, "pages/A.md");
    let second = create_page_batch(ids, 152, 152, ids.page_b, ids.home_b, "pages/B.md");
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
        provider_location(
            "beta",
            ProviderTree::Inbox,
            "manifests/nested/manifest",
        ),
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
            location: provider_location(
                "beta",
                ProviderTree::Inbox,
                destination_path,
            ),
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
    let batch = create_page_batch(ids, 1521, 1521, ids.page_a, ids.home_a, "pages/Reduced.md");
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

    let minimized = scenario.minimize_failure().unwrap();
    assert!(minimized.scenario.actions.len() < scenario.actions.len());
    let mut replay = DeterministicSimulator::new(minimized.scenario).unwrap();
    assert!(matches!(replay.run(), Err(ScenarioError::Invariant { .. })));
}

#[test]
fn filesystem_distinct_copy_rejects_same_bytes_and_leaves_bounded_residue() {
    let ids = Ids::new();
    let batch = create_page_batch(ids, 1523, 1523, ids.page_a, ids.home_a, "pages/Bounded.md");
    let destination = provider_location("beta", ProviderTree::Inbox, "objects/same-bytes");
    let actions = vec![
        provider_copy(
            1,
            &batch.objects[0].item_id,
            destination.clone(),
        ),
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
    let batch = create_page_batch(ids, 153, 153, ids.page_a, ids.home_a, "pages/Disk.md");
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
    let batch = create_page_batch(ids, 1541, 1541, ids.page_a, ids.home_a, "pages/Residue.md");
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
    let batch = create_page_batch(ids, 155, 155, ids.page_a, ids.home_a, "pages/Part.md");
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
    let batch = create_page_batch(ids, 1551, 1551, ids.page_a, ids.home_a, "pages/Temp.md");
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
    let batch = create_page_batch(ids, 156, 156, ids.page_a, ids.home_a, "pages/Relabel.md");
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
    let batch = create_page_batch(ids, 157, 157, ids.page_a, ids.home_a, "pages/Links.md");
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
    let batch = create_page_batch(ids, 159, 159, ids.page_a, ids.home_a, "pages/Ordering.md");
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
    let batch = create_page_batch(ids, 160, 160, ids.page_a, ids.home_a, "pages/Fixture.md");
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
        provider_location(
            "beta",
            ProviderTree::Inbox,
            "objects/object-0-duplicate",
        ),
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
fn fixture_seed_corpus_is_canonical_v2_json() {
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
    ];
    for fixture in fixtures {
        let fixture = fixture.trim_end();
        let scenario = Scenario::decode(fixture.as_bytes()).unwrap();
        assert_eq!(scenario.encode().unwrap(), fixture.as_bytes());
    }
}
