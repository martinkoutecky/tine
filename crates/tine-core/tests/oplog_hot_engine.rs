use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tine_core::oplog::{
    AuthorBatch, BatchCausalDot, BatchDisposition, BatchError, BatchId, BatchInspection,
    BatchOrigin, BlockLocation, BlockOwner, CausalPeerId, ContentDigest, CrdtPeerCounter,
    CrdtPeerId, DeterministicSimulator, DeviceId, DocumentCausalDigest, DocumentDependencies,
    DocumentId, EngineError, FailureCapsule, FailureIdentity, FrontierV2, ImmutableHomeClaim,
    ImmutableHomeConflict, ImmutableHomeEvidence, LineageDigest, LogseqIdentityMutation,
    LogseqIdentityOrigin, LogseqIdentityTrigger, LogseqUuid, LogseqUuidResolution, ManagedPath,
    ObjectKind, ObjectStore, OperationBatch, OperationObject, OperationTransaction, PageId,
    PagePreambleDelta, PagePreambleState, PolicyGeneratedAnchorReason, PreparedBatch,
    ProjectionEndpointBinding, ProjectionEndpointId, ProjectionReceiptStore, Scenario,
    ScenarioAction, ScenarioDevice, SemanticEffect, SemanticEffectDigest, SemanticError,
    SemanticOperation, SessionId, ShardedHotEngine, StoreError, ValidatedBatch, WorkspaceId,
    WorkspaceStatus, OPERATION_SCHEMA_VERSION,
};
use tine_core::Graph;
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-oplog-hot-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
    block_a: tine_core::oplog::BlockId,
    block_c: tine_core::oplog::BlockId,
}

impl Ids {
    fn new() -> Self {
        Self {
            workspace: WorkspaceId::from_uuid(uuid(1)),
            lineage: LineageDigest::of(b"lineage"),
            catalog: DocumentId::from_uuid(uuid(2)),
            page_a: PageId::from_uuid(uuid(10)),
            page_b: PageId::from_uuid(uuid(11)),
            page_c: PageId::from_uuid(uuid(12)),
            home_a: DocumentId::from_uuid(uuid(20)),
            home_b: DocumentId::from_uuid(uuid(21)),
            home_c: DocumentId::from_uuid(uuid(22)),
            block_a: tine_core::oplog::BlockId::from_uuid(uuid(30)),
            block_c: tine_core::oplog::BlockId::from_uuid(uuid(31)),
        }
    }

    fn engine(self) -> ShardedHotEngine {
        ShardedHotEngine::new(self.workspace, self.lineage, self.catalog)
    }
}

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn path(value: &str) -> ManagedPath {
    ManagedPath::parse(value).unwrap()
}

fn author(batch: u128, peer: u64) -> AuthorBatch {
    AuthorBatch {
        batch_id: BatchId::from_uuid(uuid(batch)),
        author_device_id: DeviceId::from_uuid(uuid(1_000 + peer as u128)),
        author_session_id: SessionId::from_uuid(uuid(2_000 + peer as u128)),
        crdt_peer_id: CrdtPeerId::from_u64(peer),
    }
}

fn tx(operations: Vec<SemanticOperation>) -> OperationTransaction {
    OperationTransaction::new(operations).unwrap()
}

fn ready(store: &ObjectStore, prepared: &PreparedBatch) -> ValidatedBatch {
    store.publish_prepared(prepared).unwrap();
    match store.inspect_batch(prepared.manifest().batch_id()).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected Ready, found {other:?}"),
    }
}

fn store(dir: &TestDir, ids: Ids) -> ObjectStore {
    ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap()
}

fn paged_fatal_evidence(engine: &ShardedHotEngine) -> Option<ImmutableHomeEvidence> {
    let mut cursor = None;
    let mut conflicts = Vec::new();
    loop {
        let page = engine.fatal_evidence_page(cursor, 1).unwrap()?;
        assert!(page.conflicts().len() <= 1);
        conflicts.extend_from_slice(page.conflicts());
        cursor = page.next();
        if cursor.is_none() {
            return Some(ImmutableHomeEvidence::new(conflicts));
        }
    }
}

fn tamper_active_scratch_pages(archive_path: &Path) {
    let scratch = archive_path.join("engine-scratch-v2");
    let runs: Vec<_> = std::fs::read_dir(scratch)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(runs.len(), 1, "expected one live scratch run");
    std::fs::write(runs[0].join("pages.index"), b"tampered scratch pages").unwrap();
}

#[test]
fn correction11_authenticated_document_dependency_heads_fail_closed() {
    let ids = Ids::new();
    let dir = TestDir::new("correction11-document-head-auth");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let prepared = genesis(ids, &ids.engine());
    writer.publish_prepared(&prepared).unwrap();

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    assert!(matches!(
        engine
            .stage_archive_batch(prepared.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    tamper_active_scratch_pages(&archive_path);

    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(90_001, 90_001),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "must not author with empty scratch heads".into(),
            }]),
        ),
        Err(EngineError::Archive(_))
    ));
}

fn genesis(ids: Ids, engine: &ShardedHotEngine) -> PreparedBatch {
    engine
        .prepare_bootstrap_transaction(
            author(100, 100),
            &tx(vec![
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
                        block_id: ids.block_a,
                        home_document_id: ids.home_a,
                    },
                    page_id: ids.page_a,
                    parent: None,
                    order: "a".into(),
                    content: "home A content".into(),
                },
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: ids.block_c,
                        home_document_id: ids.home_c,
                    },
                    page_id: ids.page_c,
                    parent: None,
                    order: "c".into(),
                    content: "unrelated content".into(),
                },
            ]),
        )
        .unwrap()
}

fn pages_only_genesis(ids: Ids, engine: &ShardedHotEngine, batch: u128) -> PreparedBatch {
    engine
        .prepare_bootstrap_transaction(
            author(batch, batch as u64),
            &tx(vec![
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
            ]),
        )
        .unwrap()
}

fn create_blocks(
    engine: &ShardedHotEngine,
    batch: u128,
    blocks: &[(tine_core::oplog::BlockId, PageId, DocumentId, &str)],
) -> PreparedBatch {
    engine
        .prepare_bootstrap_transaction(
            author(batch, batch as u64),
            &tx(blocks
                .iter()
                .map(|(block_id, page_id, home_document_id, order)| {
                    SemanticOperation::CreateBlock {
                        block: BlockLocation {
                            block_id: *block_id,
                            home_document_id: *home_document_id,
                        },
                        page_id: *page_id,
                        parent: None,
                        order: (*order).into(),
                        content: format!("batch {batch} block {block_id}"),
                    }
                })
                .collect()),
        )
        .unwrap()
}

fn seed_engine(ids: Ids, store: &ObjectStore) -> (ShardedHotEngine, ValidatedBatch) {
    let mut engine = ids.engine();
    let prepared = genesis(ids, &engine);
    let batch = ready(store, &prepared);
    assert!(matches!(
        engine.stage_ready(batch.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    (engine, batch)
}

#[test]
fn pre_p1b1_operation_schema_is_rejected_at_the_manifest_fence() {
    let ids = Ids::new();
    let dir = TestDir::new("old-operation-schema-fence");
    let archive = store(&dir, ids);
    let prepared = genesis(ids, &ids.engine());
    let semantic = prepared
        .objects()
        .iter()
        .find(|object| object.kind() == ObjectKind::SemanticEffect)
        .unwrap();
    SemanticEffect::decode(semantic.payload()).expect("control payload uses the current schema");

    let mut manifest: serde_json::Value =
        serde_json::from_slice(&prepared.manifest().encode().unwrap()).unwrap();
    manifest["operation_schema_version"] = serde_json::json!(OPERATION_SCHEMA_VERSION - 1);
    let old_schema_bytes = serde_json::to_vec(&manifest).unwrap();
    assert!(matches!(
        archive.stage_manifest_bytes(&old_schema_bytes),
        Err(StoreError::Batch(BatchError::UnknownVersion {
            field: "operation_schema_version",
            expected: OPERATION_SCHEMA_VERSION,
            found,
        })) if found == OPERATION_SCHEMA_VERSION - 1
    ));
    assert!(matches!(
        archive
            .inspect_batch(prepared.manifest().batch_id())
            .unwrap(),
        BatchInspection::Absent
    ));
}

#[test]
fn page_preamble_is_authoritative_across_replay_move_and_rename() {
    let ids = Ids::new();
    let dir = TestDir::new("page-preamble-replay");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ids.engine();
    let genesis = genesis(ids, &engine);
    let mut batch_ids = vec![genesis.manifest().batch_id()];
    assert!(matches!(
        engine.stage_ready(ready(&writer, &genesis)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(engine.materialize_page(ids.page_a).unwrap().preamble, None);

    let preamble = "title:: Stable\nfree text before the outline".to_string();
    let set = engine
        .prepare_bootstrap_transaction(
            author(39_001, 39_001),
            &tx(vec![SemanticOperation::SetPagePreamble {
                page_id: ids.page_a,
                preamble: Some(preamble.clone()),
            }]),
        )
        .unwrap();
    let effect = SemanticEffect::decode(
        set.objects()
            .iter()
            .find(|object| object.kind() == ObjectKind::SemanticEffect)
            .unwrap()
            .payload(),
    )
    .unwrap();
    assert_eq!(effect.page_preambles().len(), 1);
    assert_eq!(
        effect.page_preambles()[0].before.as_ref().unwrap().preamble,
        None
    );
    assert_eq!(
        effect.page_preambles()[0]
            .after
            .as_ref()
            .unwrap()
            .preamble
            .as_deref(),
        Some(preamble.as_str())
    );
    batch_ids.push(set.manifest().batch_id());
    assert!(matches!(
        engine.stage_ready(ready(&writer, &set)).disposition,
        BatchDisposition::Accepted { .. }
    ));

    let neighbors = engine
        .prepare_bootstrap_transaction(
            author(39_002, 39_002),
            &tx(vec![
                SemanticOperation::EditPagePath {
                    page_id: ids.page_a,
                    path: path("journals/2026_07_23.md"),
                },
                SemanticOperation::MoveSubtree {
                    root: BlockLocation {
                        block_id: ids.block_a,
                        home_document_id: ids.home_a,
                    },
                    from_page_id: ids.page_a,
                    to_page_id: ids.page_b,
                    parent: None,
                    order: "moved".into(),
                },
            ]),
        )
        .unwrap();
    batch_ids.push(neighbors.manifest().batch_id());
    assert!(matches!(
        engine.stage_ready(ready(&writer, &neighbors)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let page = engine.materialize_page(ids.page_a).unwrap();
    assert_eq!(page.path, path("journals/2026_07_23.md"));
    assert_eq!(page.preamble.as_deref(), Some(preamble.as_str()));
    assert!(page.blocks.is_empty());
    assert_eq!(engine.materialize_page(ids.page_b).unwrap().blocks.len(), 1);

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    for batch_id in batch_ids {
        assert!(matches!(
            replay.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    let replayed = replay.materialize_page(ids.page_a).unwrap();
    assert_eq!(replayed.path, path("journals/2026_07_23.md"));
    assert_eq!(replayed.preamble.as_deref(), Some(preamble.as_str()));
}

#[test]
fn concurrent_page_preamble_mutations_converge_and_validate_semantically() {
    let ids = Ids::new();
    let dir = TestDir::new("page-preamble-convergence");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let (left, right) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(39_010, 39_010),
        tx(vec![SemanticOperation::SetPagePreamble {
            page_id: ids.page_a,
            preamble: Some("left:: value".into()),
        }]),
        author(39_011, 39_011),
        tx(vec![SemanticOperation::SetPagePreamble {
            page_id: ids.page_a,
            preamble: Some("right free text".into()),
        }]),
    );
    let ab = apply_pair(ids, &baseline, left.clone(), right.clone());
    let ba = apply_pair(ids, &baseline, right, left);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    assert_eq!(
        ab.materialize_page(ids.page_a).unwrap().preamble,
        ba.materialize_page(ids.page_a).unwrap().preamble
    );

    let wrong_home = SemanticEffect::new_with_page_preambles(
        Vec::new(),
        vec![PagePreambleDelta {
            page_id: ids.page_a,
            home_document_id: ids.home_a,
            before: None,
            after: Some(PagePreambleState {
                page_id: ids.page_a,
                home_document_id: ids.home_b,
                preamble: Some("invalid".into()),
            }),
        }],
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(wrong_home, Err(SemanticError::HomeShardChanged)));
}

#[test]
fn projection_write_authorization_requires_durable_engine_derived_state() {
    let ids = Ids::new();
    let dir = TestDir::new("projection-authorization");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let prepared = genesis(ids, &ids.engine());
    let batch_id = prepared.manifest().batch_id();
    let validated = ready(&writer, &prepared);

    let mut hand_built_engine = ids.engine();
    assert!(matches!(
        hand_built_engine.stage_ready(validated).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        hand_built_engine.authorize_projection_write(ids.page_a),
        Err(EngineError::ProjectionAuthorizationUnavailable)
    ));

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut durable = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    assert!(matches!(
        durable.stage_archive_batch(batch_id).unwrap().disposition,
        BatchDisposition::Accepted { .. }
    ));
    let authorization = durable.authorize_projection_write(ids.page_a).unwrap();
    assert_eq!(authorization.state().page.page_id, ids.page_a);
    assert!(!authorization.state().frontier.documents().is_empty());
    assert!(authorization
        .state()
        .frontier
        .documents()
        .iter()
        .flat_map(|document| document.direct_dependency_heads())
        .all(|head| *head == batch_id));
}

#[test]
fn logseq_uuid_assignment_is_explicit_idempotent_replaceable_and_removable() {
    let ids = Ids::new();
    let dir = TestDir::new("logseq-uuid-lifecycle");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);
    let block = BlockLocation {
        block_id: ids.block_a,
        home_document_id: ids.home_a,
    };
    let first = LogseqUuid::from_uuid(uuid(40_001));
    let second = LogseqUuid::from_uuid(uuid(40_002));

    let assign = engine
        .prepare_bootstrap_transaction(
            author(40_010, 40_010),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block,
                mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid: first },
            }]),
        )
        .unwrap();
    let effect = SemanticEffect::decode(
        assign
            .objects()
            .iter()
            .find(|object| object.kind() == ObjectKind::SemanticEffect)
            .unwrap()
            .payload(),
    )
    .unwrap();
    assert_eq!(effect.blocks().len(), 1);
    assert_eq!(
        effect.blocks()[0].before.as_ref().unwrap().logseq_uuid,
        None
    );
    assert_eq!(
        effect.blocks()[0].after.as_ref().unwrap().logseq_uuid,
        Some(first)
    );
    assert!(matches!(
        engine.stage_ready(ready(&archive, &assign)).disposition,
        BatchDisposition::Accepted { no_op: false }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        Some(first)
    );
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_identity_origin,
        Some(LogseqIdentityOrigin::ExternalImported)
    );

    let duplicate_assign = engine.prepare_bootstrap_transaction(
        author(40_011, 40_011),
        &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
            block,
            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid: first },
        }]),
    );
    assert!(
        matches!(duplicate_assign, Err(EngineError::InvalidTransaction(_))),
        "assignment and replacement must remain distinct typed actions"
    );

    let replace = engine
        .prepare_bootstrap_transaction(
            author(40_012, 40_012),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block,
                mutation: LogseqIdentityMutation::ReplaceExternal {
                    logseq_uuid: second,
                },
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &replace)).disposition,
        BatchDisposition::Accepted { no_op: false }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        Some(second)
    );

    let remove = engine
        .prepare_bootstrap_transaction(
            author(40_013, 40_013),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block,
                mutation: LogseqIdentityMutation::RemoveExternal,
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &remove)).disposition,
        BatchDisposition::Accepted { no_op: false }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        None
    );

    let content_only = engine
        .prepare_bootstrap_transaction(
            author(40_014, 40_014),
            &tx(vec![SemanticOperation::EditBlockContent {
                block,
                content: format!("id:: {first}"),
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine
            .stage_ready(ready(&archive, &content_only))
            .disposition,
        BatchDisposition::Accepted { no_op: false }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        None,
        "semantic identity must never be inferred from content"
    );
}

#[test]
fn logseq_uuid_concurrent_assignment_converges_and_survives_move_delete() {
    let ids = Ids::new();
    let dir = TestDir::new("logseq-uuid-convergence");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let block = BlockLocation {
        block_id: ids.block_a,
        home_document_id: ids.home_a,
    };
    let left_uuid = LogseqUuid::from_uuid(uuid(41_001));
    let right_uuid = LogseqUuid::from_uuid(uuid(41_002));
    let (left, right) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(41_010, 41_010),
        tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
            block,
            mutation: LogseqIdentityMutation::AssignExternal {
                logseq_uuid: left_uuid,
            },
        }]),
        author(41_011, 41_011),
        tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
            block,
            mutation: LogseqIdentityMutation::AssignExternal {
                logseq_uuid: right_uuid,
            },
        }]),
    );
    let mut ab = apply_pair(ids, &baseline, left.clone(), right.clone());
    let ba = apply_pair(ids, &baseline, right, left);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    let winner = ab.materialize_page(ids.page_a).unwrap().blocks[0]
        .logseq_uuid
        .expect("one concurrent UUID register wins deterministically");
    assert!(winner == left_uuid || winner == right_uuid);

    let moved = ab
        .prepare_bootstrap_transaction(
            author(41_012, 41_012),
            &tx(vec![SemanticOperation::MoveSubtree {
                root: block,
                from_page_id: ids.page_a,
                to_page_id: ids.page_b,
                parent: None,
                order: "moved-with-logseq-uuid".into(),
            }]),
        )
        .unwrap();
    assert!(matches!(
        ab.stage_ready(ready(&archive, &moved)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        ab.materialize_page(ids.page_b).unwrap().blocks[0].logseq_uuid,
        Some(winner)
    );

    let deleted = ab
        .prepare_bootstrap_transaction(
            author(41_013, 41_013),
            &tx(vec![SemanticOperation::DeleteSubtree {
                root_block_id: ids.block_a,
                page_id: ids.page_b,
            }]),
        )
        .unwrap();
    assert!(matches!(
        ab.stage_ready(ready(&archive, &deleted)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        ab.recover_block_state(ids.home_a, ids.block_a)
            .unwrap()
            .unwrap()
            .logseq_uuid,
        Some(winner)
    );
}

#[test]
fn logseq_uuid_restarts_and_replays_from_the_stable_home_shard() {
    let ids = Ids::new();
    let dir = TestDir::new("logseq-uuid-replay");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut author_engine = ids.engine();
    let genesis = genesis(ids, &author_engine);
    let genesis_id = genesis.manifest().batch_id();
    assert!(matches!(
        author_engine
            .stage_ready(ready(&writer, &genesis))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    let assigned_uuid = LogseqUuid::from_uuid(uuid(42_001));
    let assigned = author_engine
        .prepare_bootstrap_transaction(
            author(42_010, 42_010),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                mutation: LogseqIdentityMutation::AssignExternal {
                    logseq_uuid: assigned_uuid,
                },
            }]),
        )
        .unwrap();
    let assigned_id = assigned.manifest().batch_id();
    assert!(matches!(
        author_engine
            .stage_ready(ready(&writer, &assigned))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    drop(author_engine);

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    for batch_id in [genesis_id, assigned_id] {
        assert!(matches!(
            replay.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    assert_eq!(
        replay.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        Some(assigned_uuid)
    );
    assert_eq!(
        replay
            .recover_block_state(ids.home_a, ids.block_a)
            .unwrap()
            .unwrap()
            .logseq_uuid,
        Some(assigned_uuid)
    );
}

#[test]
fn projection_page_frontier_is_exact_and_same_batch_uuid_reference_is_atomic() {
    let ids = Ids::new();
    let dir = TestDir::new("projection-page-frontier");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);
    let assigned_uuid = LogseqUuid::from_uuid(uuid(43_001));
    let anchored = engine
        .prepare_bootstrap_transaction(
            author(43_010, 43_010),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: BlockLocation {
                        block_id: ids.block_a,
                        home_document_id: ids.home_a,
                    },
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: assigned_uuid,
                        trigger: LogseqIdentityTrigger::BlockReference {
                            referrer: BlockLocation {
                                block_id: ids.block_c,
                                home_document_id: ids.home_c,
                            },
                        },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block_c,
                        home_document_id: ids.home_c,
                    },
                    content: format!("same-batch reference (({assigned_uuid}))"),
                },
            ]),
        )
        .unwrap();
    let anchored_id = anchored.manifest().batch_id();
    let updated_documents: Vec<_> = anchored
        .manifest()
        .required_objects()
        .iter()
        .filter(|object| object.kind() == ObjectKind::CrdtUpdate)
        .map(|object| object.document_id())
        .collect();
    assert_eq!(updated_documents, vec![ids.home_a, ids.home_c]);
    assert!(matches!(
        engine.stage_ready(ready(&archive, &anchored)).disposition,
        BatchDisposition::Accepted { no_op: false }
    ));

    let page_a = engine.materialize_page_for_projection(ids.page_a).unwrap();
    assert_eq!(page_a.page.blocks[0].logseq_uuid, Some(assigned_uuid));
    assert_eq!(
        page_a.page.blocks[0].logseq_identity_origin,
        Some(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockReference,
        })
    );
    let page_a_documents: Vec<_> = page_a
        .frontier
        .documents()
        .iter()
        .map(DocumentDependencies::document_id)
        .collect();
    assert_eq!(page_a_documents, vec![ids.catalog, ids.home_a]);
    assert!(page_a
        .frontier
        .documents()
        .iter()
        .find(|document| document.document_id() == ids.home_a)
        .unwrap()
        .direct_dependency_heads()
        .contains(&anchored_id));

    let page_c = engine.materialize_page_for_projection(ids.page_c).unwrap();
    assert_eq!(
        page_c.page.blocks[0].content,
        format!("same-batch reference (({assigned_uuid}))")
    );
    let page_c_documents: Vec<_> = page_c
        .frontier
        .documents()
        .iter()
        .map(DocumentDependencies::document_id)
        .collect();
    assert_eq!(page_c_documents, vec![ids.catalog, ids.home_a, ids.home_c]);
    assert!(page_c
        .frontier
        .documents()
        .iter()
        .find(|document| document.document_id() == ids.home_c)
        .unwrap()
        .direct_dependency_heads()
        .contains(&anchored_id));
    assert!(!page_a_documents.contains(&ids.home_c));
    assert!(page_c_documents.contains(&ids.home_a));
}

#[test]
fn policy_generated_identity_requires_typed_same_batch_content_or_user_action() {
    let ids = Ids::new();
    let dir = TestDir::new("typed-logseq-triggers");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);
    let target = BlockLocation {
        block_id: ids.block_a,
        home_document_id: ids.home_a,
    };
    let referrer = BlockLocation {
        block_id: ids.block_c,
        home_document_id: ids.home_c,
    };
    let embed_uuid = LogseqUuid::from_uuid(uuid(43_100));

    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_101, 43_101),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block: target,
                mutation: LogseqIdentityMutation::Generate {
                    logseq_uuid: embed_uuid,
                    trigger: LogseqIdentityTrigger::BlockEmbed { referrer },
                },
            }]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_102, 43_102),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockEmbed { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: format!("{{{{embed (({}))}}}}", LogseqUuid::from_uuid(uuid(43_999))),
                },
            ]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_102_001, 43_102_001),
            &tx(vec![
                SemanticOperation::EditPagePath {
                    page_id: ids.page_c,
                    path: path("pages/C.org"),
                },
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockReference { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: format!("#+BEGIN_SRC text\n(({embed_uuid}))\n#+END_SRC"),
                },
            ]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_102_002, 43_102_002),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockEmbed { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: format!("{{{{embed (({embed_uuid}))}}}}"),
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: "the final content removed the trigger".into(),
                },
            ]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));

    let preexisting = format!("{{{{embed (({embed_uuid}))}}}}");
    let seed_trigger = engine
        .prepare_bootstrap_transaction(
            author(43_102_010, 43_102_010),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: referrer,
                content: preexisting.clone(),
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine
            .stage_ready(ready(&archive, &seed_trigger))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_102_011, 43_102_011),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockEmbed { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: preexisting,
                },
            ]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));
    let clear_trigger = engine
        .prepare_bootstrap_transaction(
            author(43_102_012, 43_102_012),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: referrer,
                content: "cleared".into(),
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine
            .stage_ready(ready(&archive, &clear_trigger))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    let org_reference = format!("(({embed_uuid}))");
    let seed_org_trigger = engine
        .prepare_bootstrap_transaction(
            author(43_102_013, 43_102_013),
            &tx(vec![
                SemanticOperation::EditPagePath {
                    page_id: ids.page_c,
                    path: path("pages/C.org"),
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: org_reference.clone(),
                },
            ]),
        )
        .unwrap();
    assert!(matches!(
        engine
            .stage_ready(ready(&archive, &seed_org_trigger))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(43_102_014, 43_102_014),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockReference { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: org_reference,
                },
            ]),
        ),
        Err(EngineError::MissingLogseqIdentityTrigger { .. })
    ));
    let restore_markdown = engine
        .prepare_bootstrap_transaction(
            author(43_102_015, 43_102_015),
            &tx(vec![
                SemanticOperation::EditPagePath {
                    page_id: ids.page_c,
                    path: path("pages/C.md"),
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: "cleared again".into(),
                },
            ]),
        )
        .unwrap();
    assert!(matches!(
        engine
            .stage_ready(ready(&archive, &restore_markdown))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));

    let embed = engine
        .prepare_bootstrap_transaction(
            author(43_103, 43_103),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: target,
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: embed_uuid,
                        trigger: LogseqIdentityTrigger::BlockEmbed { referrer },
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: referrer,
                    content: format!("{{{{embed (({embed_uuid}))}}}}"),
                },
            ]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &embed)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().blocks[0].logseq_identity_origin,
        Some(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockEmbed,
        })
    );

    let exported_block = tine_core::oplog::BlockId::from_uuid(uuid(43_104));
    let exported_uuid = LogseqUuid::from_uuid(uuid(43_105));
    let exported = engine
        .prepare_bootstrap_transaction(
            author(43_106, 43_106),
            &tx(vec![
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: exported_block,
                        home_document_id: ids.home_b,
                    },
                    page_id: ids.page_b,
                    parent: None,
                    order: "exported".into(),
                    content: "explicit export target".into(),
                },
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: BlockLocation {
                        block_id: exported_block,
                        home_document_id: ids.home_b,
                    },
                    mutation: LogseqIdentityMutation::Generate {
                        logseq_uuid: exported_uuid,
                        trigger: LogseqIdentityTrigger::ExportUserAction,
                    },
                },
            ]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &exported)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        engine
            .materialize_page(ids.page_b)
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.block_id == exported_block)
            .unwrap()
            .logseq_identity_origin,
        Some(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::Export,
        })
    );
}

#[test]
fn sparse_uuid_claim_index_converges_and_invalidates_reference_frontiers() {
    let ids = Ids::new();
    let dir = TestDir::new("sparse-uuid-claims");
    let archive = store(&dir, ids);
    let (mut seed, genesis_ready) = seed_engine(ids, &archive);
    let block_b = tine_core::oplog::BlockId::from_uuid(uuid(44_001));
    let create_b = seed
        .prepare_bootstrap_transaction(
            author(44_002, 44_002),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: block_b,
                    home_document_id: ids.home_b,
                },
                page_id: ids.page_b,
                parent: None,
                order: "b".into(),
                content: "second claimant".into(),
            }]),
        )
        .unwrap();
    let create_b_ready = ready(&archive, &create_b);
    assert!(matches!(
        seed.stage_ready(create_b_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let duplicate = LogseqUuid::from_uuid(uuid(44_003));
    let (left, right) = concurrent_ready_from(
        ids,
        &archive,
        &[genesis_ready.clone(), create_b_ready.clone()],
        author(44_004, 44_004),
        tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
            block: BlockLocation {
                block_id: ids.block_a,
                home_document_id: ids.home_a,
            },
            mutation: LogseqIdentityMutation::AssignExternal {
                logseq_uuid: duplicate,
            },
        }]),
        author(44_005, 44_005),
        tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
            block: BlockLocation {
                block_id: block_b,
                home_document_id: ids.home_b,
            },
            mutation: LogseqIdentityMutation::AssignExternal {
                logseq_uuid: duplicate,
            },
        }]),
    );
    let durable_batch_ids = [
        genesis_ready.manifest().batch_id(),
        create_b_ready.manifest().batch_id(),
        left.manifest().batch_id(),
        right.manifest().batch_id(),
    ];
    let mut ab = apply_pair_from(
        ids,
        &[genesis_ready.clone(), create_b_ready.clone()],
        left.clone(),
        right.clone(),
    );
    let ba = apply_pair_from(ids, &[genesis_ready, create_b_ready], right, left);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    assert_eq!(
        ab.resolve_logseq_uuid(duplicate),
        LogseqUuidResolution::Ambiguous { claim_count: 2 }
    );
    assert_eq!(
        ba.resolve_logseq_uuid(duplicate),
        LogseqUuidResolution::Ambiguous { claim_count: 2 }
    );
    assert_eq!(
        ab.materialize_page(ids.page_a).unwrap().blocks[0].logseq_uuid,
        Some(duplicate)
    );
    assert_eq!(
        ab.materialize_page(ids.page_b).unwrap().blocks[0].logseq_uuid,
        Some(duplicate)
    );

    let reader = ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap();
    let mut durable = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    for batch_id in durable_batch_ids {
        assert!(matches!(
            durable.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    assert!(matches!(
        durable.authorize_projection_write(ids.page_a),
        Err(EngineError::AmbiguousLogseqUuid {
            logseq_uuid,
            claim_count: 2,
        }) if logseq_uuid == duplicate
    ));

    let reference = ab
        .prepare_bootstrap_transaction(
            author(44_006, 44_006),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_c,
                    home_document_id: ids.home_c,
                },
                content: format!("ambiguous (({duplicate}))"),
            }]),
        )
        .unwrap();
    assert!(matches!(
        ab.stage_ready(ready(&archive, &reference)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        ab.materialize_page_for_projection(ids.page_c),
        Err(EngineError::AmbiguousLogseqUuid {
            logseq_uuid,
            claim_count: 2,
        }) if logseq_uuid == duplicate
    ));

    let remove_b = ab
        .prepare_bootstrap_transaction(
            author(44_007, 44_007),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block: BlockLocation {
                    block_id: block_b,
                    home_document_id: ids.home_b,
                },
                mutation: LogseqIdentityMutation::RemoveExternal,
            }]),
        )
        .unwrap();
    assert!(matches!(
        ab.stage_ready(ready(&archive, &remove_b)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let unique_frontier = ab.materialize_page_for_projection(ids.page_c).unwrap();
    let unique_documents: Vec<_> = unique_frontier
        .frontier
        .documents()
        .iter()
        .map(DocumentDependencies::document_id)
        .collect();
    assert!(unique_documents.contains(&ids.home_a));
    assert!(unique_documents.contains(&ids.home_b));
    assert_eq!(unique_frontier.claim_evidence.len(), 1);
    assert_eq!(unique_frontier.claim_evidence[0].participants().len(), 2);

    let remove_a = ab
        .prepare_bootstrap_transaction(
            author(44_008, 44_008),
            &tx(vec![SemanticOperation::MutateBlockLogseqIdentity {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                mutation: LogseqIdentityMutation::RemoveExternal,
            }]),
        )
        .unwrap();
    assert!(matches!(
        ab.stage_ready(ready(&archive, &remove_a)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        ab.resolve_logseq_uuid(duplicate),
        LogseqUuidResolution::Unclaimed
    );
    let removed_frontier = ab.materialize_page_for_projection(ids.page_c).unwrap();
    let removed_documents: Vec<_> = removed_frontier
        .frontier
        .documents()
        .iter()
        .map(DocumentDependencies::document_id)
        .collect();
    assert!(removed_documents.contains(&ids.home_a));
    assert!(removed_documents.contains(&ids.home_b));
    assert_eq!(removed_frontier.claim_evidence[0].participants().len(), 2);
    assert_ne!(unique_frontier.frontier, removed_frontier.frontier);
}

#[test]
fn deleting_page_invalidates_uuid_claim_but_retains_participant_evidence() {
    let ids = Ids::new();
    let dir = TestDir::new("page-delete-uuid-claim");
    let archive = store(&dir, ids);
    let (mut author_engine, genesis) = seed_engine(ids, &archive);
    let claimed = LogseqUuid::from_uuid(uuid(44_100));
    let assign = author_engine
        .prepare_bootstrap_transaction(
            author(44_101, 44_101),
            &tx(vec![
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: BlockLocation {
                        block_id: ids.block_a,
                        home_document_id: ids.home_a,
                    },
                    mutation: LogseqIdentityMutation::AssignExternal {
                        logseq_uuid: claimed,
                    },
                },
                SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: ids.block_c,
                        home_document_id: ids.home_c,
                    },
                    content: format!("reference (({claimed}))"),
                },
            ]),
        )
        .unwrap();
    let assign_ready = ready(&archive, &assign);
    assert!(matches!(
        author_engine.stage_ready(assign_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let delete = author_engine
        .prepare_bootstrap_transaction(
            author(44_102, 44_102),
            &tx(vec![SemanticOperation::DeletePage {
                page_id: ids.page_a,
            }]),
        )
        .unwrap();
    let delete_ready = ready(&archive, &delete);
    assert!(matches!(
        author_engine.stage_ready(delete_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));

    let reader = ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    for batch_id in [
        genesis.manifest().batch_id(),
        assign_ready.manifest().batch_id(),
        delete_ready.manifest().batch_id(),
    ] {
        let outcome = replay.stage_archive_batch(batch_id).unwrap();
        assert!(
            matches!(outcome.disposition, BatchDisposition::Accepted { .. }),
            "batch {batch_id}: {outcome:?}"
        );
    }
    assert_eq!(
        replay.resolve_logseq_uuid(claimed),
        LogseqUuidResolution::Unclaimed
    );
    assert!(matches!(
        replay.materialize_page(ids.page_a),
        Err(EngineError::PageDeleted(page_id)) if page_id == ids.page_a
    ));
    let reference = replay.materialize_page_for_projection(ids.page_c).unwrap();
    assert_eq!(reference.claim_evidence.len(), 1);
    assert_eq!(
        reference.claim_evidence[0].participants()[0].block_id(),
        ids.block_a
    );
    assert!(reference
        .frontier
        .documents()
        .iter()
        .any(|document| document.document_id() == ids.home_a));
    replay.authorize_projection_write(ids.page_c).unwrap();
}

#[test]
fn store_backed_uuid_claim_lookup_stays_point_local_and_hot_memory_bounded() {
    const CLAIMS: usize = 128;

    let ids = Ids::new();
    let dir = TestDir::new("uuid-claim-scaling");
    let archive = store(&dir, ids);
    let (mut author_engine, genesis) = seed_engine(ids, &archive);
    let mut operations = Vec::with_capacity(CLAIMS * 2);
    let mut target = None;
    for index in 0..CLAIMS {
        let block_id = tine_core::oplog::BlockId::from_uuid(uuid(45_000 + index as u128));
        let logseq_uuid = LogseqUuid::from_uuid(uuid(46_000 + index as u128));
        target = Some((block_id, logseq_uuid));
        operations.push(SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id,
                home_document_id: ids.home_a,
            },
            page_id: ids.page_a,
            parent: None,
            order: format!("scale-{index:04}"),
            content: format!("scaled block {index}"),
        });
        operations.push(SemanticOperation::MutateBlockLogseqIdentity {
            block: BlockLocation {
                block_id,
                home_document_id: ids.home_a,
            },
            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
        });
    }
    let bulk = author_engine
        .prepare_bootstrap_transaction(author(46_500, 46_500), &tx(operations))
        .unwrap();
    let bulk = ready(&archive, &bulk);
    assert!(matches!(
        author_engine.stage_ready(bulk.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));

    let reader = ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    for batch_id in [genesis.manifest().batch_id(), bulk.manifest().batch_id()] {
        assert!(matches!(
            replay.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    let before = replay.instrumentation();
    assert_eq!(before.logseq_claim_hot_entries, 0);
    let (target_block, target_uuid) = target.unwrap();
    assert!(matches!(
        replay.resolve_logseq_uuid(target_uuid),
        LogseqUuidResolution::Unique(claim)
            if claim.block_id == target_block && claim.home_document_id == ids.home_a
    ));
    let after = replay.instrumentation();
    assert_eq!(after.logseq_claim_hot_entries, 0);
    assert!(
        after
            .logseq_claim_index_reads
            .saturating_sub(before.logseq_claim_index_reads)
            <= 32,
        "one UUID lookup read too many authenticated nodes: before={before:?}, after={after:?}"
    );
    assert!(after.logseq_claim_index_writes > 0);
}

#[test]
fn author_cannot_alias_a_page_home_to_the_catalog() {
    let ids = Ids::new();
    let engine = ids.engine();
    let outcome = engine.prepare_bootstrap_transaction(
        author(99, 99),
        &tx(vec![SemanticOperation::CreatePage {
            page_id: ids.page_a,
            home_document_id: ids.catalog,
            path: path("pages/A.md"),
        }]),
    );

    assert!(matches!(outcome, Err(EngineError::InvalidTransaction(_))));
}

fn rebuild(
    manifest: &OperationBatch,
    objects: Vec<OperationObject>,
    frontier: FrontierV2,
) -> PreparedBatch {
    let semantic = objects
        .iter()
        .find(|object| object.kind() == ObjectKind::SemanticEffect)
        .unwrap();
    let descriptors = objects
        .iter()
        .map(OperationObject::descriptor)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let causal_dependency_heads = frontier
        .documents()
        .iter()
        .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
        .collect();
    let manifest = OperationBatch::new_with_causality(
        manifest.workspace_id(),
        manifest.lineage_digest(),
        manifest.batch_id(),
        manifest.author_device_id(),
        manifest.author_session_id(),
        BatchOrigin::BootstrapImport,
        BatchCausalDot::new(CausalPeerId::from_device_id(manifest.author_device_id()), 1).unwrap(),
        causal_dependency_heads,
        frontier,
        SemanticEffectDigest::of(semantic.payload()),
        descriptors,
    )
    .unwrap();
    PreparedBatch::new(manifest, objects).unwrap()
}

fn rebuild_as(
    manifest: &OperationBatch,
    batch_id: BatchId,
    objects: Vec<OperationObject>,
    frontier: FrontierV2,
) -> PreparedBatch {
    let semantic = objects
        .iter()
        .find(|object| object.kind() == ObjectKind::SemanticEffect)
        .unwrap();
    let descriptors = objects
        .iter()
        .map(OperationObject::descriptor)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let causal_dependency_heads = frontier
        .documents()
        .iter()
        .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
        .collect();
    let manifest = OperationBatch::new_with_causality(
        manifest.workspace_id(),
        manifest.lineage_digest(),
        batch_id,
        manifest.author_device_id(),
        manifest.author_session_id(),
        BatchOrigin::BootstrapImport,
        BatchCausalDot::new(CausalPeerId::from_device_id(manifest.author_device_id()), 1).unwrap(),
        causal_dependency_heads,
        frontier,
        SemanticEffectDigest::of(semantic.payload()),
        descriptors,
    )
    .unwrap();
    PreparedBatch::new(manifest, objects).unwrap()
}

#[derive(Serialize, Deserialize)]
struct TestCrdtUpdatePayload {
    schema_version: u32,
    batch_id: BatchId,
    document_id: DocumentId,
    dependency_heads: Vec<BatchId>,
    batch_dependency_heads: Vec<BatchId>,
    causal_state_digest: Option<DocumentCausalDigest>,
    raw_update: Vec<u8>,
}

/// Rebind the private CRDT envelope to a replacement compact frontier while
/// retaining the raw Loro update. This constructs a canonical, internally
/// coherent witness without adding a production mutation API.
fn rebuild_with_compact_witness(prepared: &PreparedBatch, frontier: FrontierV2) -> PreparedBatch {
    let batch_dependency_heads: Vec<_> = frontier
        .documents()
        .iter()
        .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let objects = prepared
        .objects()
        .iter()
        .map(|object| {
            if object.kind() != ObjectKind::CrdtUpdate {
                return object.clone();
            }
            let mut payload: TestCrdtUpdatePayload =
                postcard::from_bytes(object.payload()).unwrap();
            let dependencies = frontier
                .documents()
                .iter()
                .find(|dependencies| dependencies.document_id() == object.document_id());
            payload.dependency_heads = dependencies
                .into_iter()
                .flat_map(|dependencies| dependencies.direct_dependency_heads().iter().copied())
                .collect();
            payload.batch_dependency_heads = batch_dependency_heads.clone();
            payload.causal_state_digest =
                dependencies.map(DocumentDependencies::causal_state_digest);
            OperationObject::new(
                object.workspace_id(),
                object.document_id(),
                object.kind(),
                postcard::to_allocvec(&payload).unwrap(),
            )
            .unwrap()
        })
        .collect();
    rebuild(prepared.manifest(), objects, frontier)
}

#[test]
fn moved_away_block_keeps_stable_home_and_page_read_loads_only_referenced_homes() {
    let ids = Ids::new();
    let dir = TestDir::new("stable-home");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);

    let moved = engine
        .prepare_bootstrap_transaction(
            author(101, 101),
            &tx(vec![SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                from_page_id: ids.page_a,
                to_page_id: ids.page_b,
                parent: None,
                order: "moved".into(),
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &moved)).disposition,
        BatchDisposition::Accepted { .. }
    ));

    assert!(engine
        .materialize_page(ids.page_a)
        .unwrap()
        .blocks
        .is_empty());
    let page = engine.materialize_page(ids.page_b).unwrap();
    assert_eq!(page.blocks.len(), 1);
    assert_eq!(page.blocks[0].home_document_id, ids.home_a);
    assert_eq!(page.blocks[0].content, "home A content");
    assert_eq!(page.stats.catalog_documents_loaded, 1);
    assert_eq!(page.stats.membership_documents_loaded, 1);
    assert_eq!(page.stats.distinct_home_documents, vec![ids.home_a]);
    assert!(!page.stats.distinct_home_documents.contains(&ids.home_c));
}

#[test]
fn malformed_unrelated_shard_rejects_without_poisoning_sparse_page_reads() {
    let ids = Ids::new();
    let dir = TestDir::new("unrelated-malformed");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);
    let edit = engine
        .prepare_bootstrap_transaction(
            author(102, 102),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_c,
                    home_document_id: ids.home_c,
                },
                content: "will be malformed".into(),
            }]),
        )
        .unwrap();
    let objects = edit
        .objects()
        .iter()
        .map(|object| {
            if object.kind() == ObjectKind::CrdtUpdate {
                OperationObject::new(
                    ids.workspace,
                    object.document_id(),
                    ObjectKind::CrdtUpdate,
                    b"not-a-loro-update".to_vec(),
                )
                .unwrap()
            } else {
                object.clone()
            }
        })
        .collect();
    let malformed = rebuild(
        edit.manifest(),
        objects,
        edit.manifest().dependency_frontier().clone(),
    );
    let malformed_batch_id = malformed.manifest().batch_id();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &malformed)).disposition,
        BatchDisposition::Rejected { .. }
    ));
    let page = engine.materialize_page(ids.page_a).unwrap();
    assert_eq!(page.blocks[0].content, "home A content");
    assert_eq!(page.stats.distinct_home_documents, vec![ids.home_a]);

    let dependent = engine
        .prepare_bootstrap_transaction(
            author(108, 108),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "must not publish".into(),
            }]),
        )
        .unwrap();
    let original = &dependent.manifest().dependency_frontier().documents()[0];
    let mut direct_heads = original.direct_dependency_heads().to_vec();
    direct_heads.push(malformed_batch_id);
    direct_heads.sort_unstable();
    direct_heads.dedup();
    let referenced_frontier = FrontierV2::new(vec![DocumentDependencies::new(
        original.document_id(),
        original.peer_counters().to_vec(),
        direct_heads,
    )
    .unwrap()])
    .unwrap();
    let referenced = rebuild_with_compact_witness(&dependent, referenced_frontier);
    assert!(matches!(
        engine.stage_ready(ready(&archive, &referenced)).disposition,
        BatchDisposition::Rejected {
            error: EngineError::RejectedDependency(batch_id),
            ..
        } if batch_id == malformed_batch_id
    ));
}

#[test]
fn accepted_sparse_reload_reads_only_target_object_but_ingress_stays_fail_closed() {
    let ids = Ids::new();
    let dir = TestDir::new("sparse-exact-reload");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let fixture = ids.engine();
    let prepared = genesis(ids, &fixture);
    archive.publish_prepared(&prepared).unwrap();
    let batch_id = prepared.manifest().batch_id();
    let mut engine = ShardedHotEngine::with_archive_store(archive, ids.lineage, ids.catalog);
    assert!(matches!(
        engine.stage_archive_batch(batch_id).unwrap().disposition,
        BatchDisposition::Accepted { .. }
    ));

    let unrelated_descriptor = prepared
        .manifest()
        .required_objects()
        .iter()
        .find(|descriptor| {
            descriptor.kind() == ObjectKind::CrdtUpdate && descriptor.document_id() == ids.home_c
        })
        .unwrap();
    let unrelated_path = archive_path
        .join("objects")
        .join(format!("{}.object", unrelated_descriptor.content_digest()));
    let mut unrelated_bytes = std::fs::read(&unrelated_path).unwrap();
    let unrelated_index = unrelated_bytes.len() / 2;
    unrelated_bytes[unrelated_index] ^= 1;
    std::fs::write(&unrelated_path, unrelated_bytes).unwrap();

    std::fs::write(
        archive_path
            .join("batches")
            .join("unrelated-malformed-entry"),
        b"must never be opened by accepted exact reload",
    )
    .unwrap();
    let page = engine.materialize_page(ids.page_a).unwrap();
    assert_eq!(page.blocks[0].content, "home A content");
    assert_eq!(page.stats.distinct_home_documents, vec![ids.home_a]);
    assert_eq!(page.stats.physical_manifest_reads, 1);
    assert_eq!(page.stats.physical_object_reads, 1);
    assert!(matches!(
        engine.materialize_page(ids.page_c),
        Err(EngineError::Archive(_))
    ));
    assert!(engine.stage_archive_batch(batch_id).is_err());
}

#[test]
fn accepted_sparse_reload_rejects_target_manifest_or_object_mutation_and_missing_bytes() {
    let ids = Ids::new();
    for mutation in ["manifest", "object", "missing"] {
        let dir = TestDir::new(&format!("sparse-target-{mutation}"));
        let archive_path = dir.path().join("archive");
        let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
        let prepared = genesis(ids, &ids.engine());
        archive.publish_prepared(&prepared).unwrap();
        let batch_id = prepared.manifest().batch_id();
        let home_descriptor = prepared
            .manifest()
            .required_objects()
            .iter()
            .find(|descriptor| {
                descriptor.kind() == ObjectKind::CrdtUpdate
                    && descriptor.document_id() == ids.home_a
            })
            .unwrap();
        let object_path = archive_path
            .join("objects")
            .join(format!("{}.object", home_descriptor.content_digest()));
        let mut engine = ShardedHotEngine::with_archive_store(archive, ids.lineage, ids.catalog);
        assert!(matches!(
            engine.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));

        match mutation {
            "manifest" => std::fs::write(
                archive_path
                    .join("batches")
                    .join(format!("{batch_id}.manifest")),
                b"mutated accepted manifest",
            )
            .unwrap(),
            "object" => {
                let mut bytes = std::fs::read(&object_path).unwrap();
                let index = bytes.len() / 2;
                bytes[index] ^= 1;
                std::fs::write(&object_path, bytes).unwrap();
            }
            "missing" => std::fs::remove_file(&object_path).unwrap(),
            _ => unreachable!(),
        }
        assert!(matches!(
            engine.materialize_page(ids.page_a),
            Err(EngineError::Archive(_))
        ));
    }
}

#[test]
fn correction11_cold_aged_page_reopens_replays_and_authors_without_history_range_scan() {
    const PAGES: usize = 70;
    let ids = Ids::new();
    let dir = TestDir::new("cold-aged-page");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut author_engine = ShardedHotEngine::new(ids.workspace, ids.lineage, ids.catalog);
    let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    let mut operations = Vec::with_capacity(PAGES * 2);
    for index in 0..PAGES {
        let page_id = PageId::from_uuid(uuid(80_000 + index as u128));
        let home_document_id = DocumentId::from_uuid(uuid(81_000 + index as u128));
        let block_id = tine_core::oplog::BlockId::from_uuid(uuid(82_000 + index as u128));
        operations.push(SemanticOperation::CreatePage {
            page_id,
            home_document_id,
            path: path(&format!("pages/Aged {index:03}.md")),
        });
        operations.push(SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id,
                home_document_id,
            },
            page_id,
            parent: None,
            order: "a".into(),
            content: format!("initial {index}"),
        });
    }
    let genesis = author_engine
        .prepare_bootstrap_transaction(author(83_000, 83_000), &tx(operations))
        .unwrap();
    let genesis_ready = ready(&writer, &genesis);
    assert!(matches!(
        author_engine.stage_ready(genesis_ready).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        engine
            .stage_archive_batch(genesis.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(engine.instrumentation().document_hot_entries <= 65);

    let cold_page = PageId::from_uuid(uuid(80_000));
    let cold_home = DocumentId::from_uuid(uuid(81_000));
    let cold_block = tine_core::oplog::BlockId::from_uuid(uuid(82_000));
    let edit = author_engine
        .prepare_bootstrap_transaction(
            author(83_001, 83_000),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: cold_block,
                    home_document_id: cold_home,
                },
                content: "edited after eviction".into(),
            }]),
        )
        .unwrap();
    let edit_ready = ready(&writer, &edit);
    assert!(matches!(
        author_engine.stage_ready(edit_ready).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let before_edit = engine.instrumentation();
    let edit_disposition = engine
        .stage_archive_batch(edit.manifest().batch_id())
        .unwrap()
        .disposition;
    assert!(
        matches!(edit_disposition, BatchDisposition::Accepted { .. }),
        "cold edit disposition: {edit_disposition:?}"
    );
    let materialized = engine.materialize_page(cold_page).unwrap();
    assert_eq!(materialized.blocks[0].content, "edited after eviction");
    assert_eq!(materialized.stats.physical_manifest_reads, 1);
    assert_eq!(materialized.stats.physical_object_reads, 1);
    let instrumentation = engine.instrumentation();
    assert_eq!(
        instrumentation.external_flushes - before_edit.external_flushes,
        1,
        "one exact-current shard transition must flush once"
    );
    assert!(
        instrumentation.external_history_page_reads > before_edit.external_history_page_reads,
        "cold reload must report authenticated history-page reads"
    );
    eprintln!(
        "cold_aged_external_work flushes={} points={} scans={} pages={} blobs={}",
        instrumentation.external_flushes - before_edit.external_flushes,
        instrumentation.external_point_reads - before_edit.external_point_reads,
        instrumentation.external_range_scans - before_edit.external_range_scans,
        instrumentation.external_history_page_reads - before_edit.external_history_page_reads,
        instrumentation.external_history_blob_reads - before_edit.external_history_blob_reads,
    );
    assert_eq!(instrumentation.scratch_syncs, 0);
    assert!(instrumentation.document_hot_entries <= 65);

    let genesis_id = genesis.manifest().batch_id();
    let edit_id = edit.manifest().batch_id();
    drop(engine);

    let replay_reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(replay_reader, ids.lineage, ids.catalog);
    for batch_id in [genesis_id, edit_id] {
        assert!(matches!(
            replay.stage_archive_batch(batch_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    assert!(replay.instrumentation().document_hot_entries <= 65);
    assert_eq!(
        replay.materialize_page(cold_page).unwrap().blocks[0].content,
        "edited after eviction"
    );

    let authored = replay
        .prepare_bootstrap_transaction(
            author(83_002, 83_000),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: cold_block,
                    home_document_id: cold_home,
                },
                content: "authored after cold replay".into(),
            }]),
        )
        .unwrap();
    writer.publish_prepared(&authored).unwrap();
    let before_authored_stage = replay.instrumentation();
    assert!(matches!(
        replay
            .stage_archive_batch(authored.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    let after_authored_stage = replay.instrumentation();
    assert_eq!(
        after_authored_stage.external_flushes - before_authored_stage.external_flushes,
        1,
        "store-backed author replay must keep one exact-current flush"
    );
    assert_eq!(
        after_authored_stage.external_range_scans - before_authored_stage.external_range_scans,
        0,
        "the authenticated adapter must not physically scan history for a point update"
    );
    assert!(
        after_authored_stage.external_history_page_reads
            > before_authored_stage.external_history_page_reads,
        "the replayed author path must expose authenticated history-page reads"
    );
    assert_eq!(
        replay.materialize_page(cold_page).unwrap().blocks[0].content,
        "authored after cold replay"
    );
    assert!(replay.instrumentation().document_hot_entries <= 65);
}

#[test]
fn external_cold_replay_concurrent_old_base_map_and_text_edits_converge() {
    let ids = Ids::new();
    let dir = TestDir::new("external-cold-concurrent");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let baseline = genesis(ids, &ids.engine());
    let baseline_ready = ready(&archive, &baseline);

    let concurrent_pair = |left_author, left_tx, right_author, right_tx| {
        let mut left = ids.engine();
        let mut right = ids.engine();
        left.stage_ready(baseline_ready.clone());
        right.stage_ready(baseline_ready.clone());
        let left = left
            .prepare_bootstrap_transaction(left_author, &left_tx)
            .unwrap();
        let right = right
            .prepare_bootstrap_transaction(right_author, &right_tx)
            .unwrap();
        archive.publish_prepared(&left).unwrap();
        archive.publish_prepared(&right).unwrap();
        [left.manifest().batch_id(), right.manifest().batch_id()]
    };

    let map_batches = concurrent_pair(
        author(83_100, 83_100),
        tx(vec![SemanticOperation::EditPagePath {
            page_id: ids.page_a,
            path: path("pages/concurrent-left.md"),
        }]),
        author(83_101, 83_101),
        tx(vec![SemanticOperation::EditPagePath {
            page_id: ids.page_a,
            path: path("pages/concurrent-right.md"),
        }]),
    );
    let text_batches = concurrent_pair(
        author(83_102, 83_102),
        tx(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: ids.block_a,
                home_document_id: ids.home_a,
            },
            content: "concurrent left text".into(),
        }]),
        author(83_103, 83_103),
        tx(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: ids.block_a,
                home_document_id: ids.home_a,
            },
            content: "concurrent right text".into(),
        }]),
    );

    for batches in [map_batches, text_batches] {
        let mut snapshots = Vec::new();
        for order in [batches, [batches[1], batches[0]]] {
            let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
            let mut receiver =
                ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
            assert!(matches!(
                receiver
                    .stage_archive_batch(baseline.manifest().batch_id())
                    .unwrap()
                    .disposition,
                BatchDisposition::Accepted { .. }
            ));
            assert!(matches!(
                receiver.stage_archive_batch(order[0]).unwrap().disposition,
                BatchDisposition::Accepted { .. }
            ));
            let before_divergent = receiver.instrumentation();
            assert!(matches!(
                receiver.stage_archive_batch(order[1]).unwrap().disposition,
                BatchDisposition::Accepted { .. }
            ));
            let after_divergent = receiver.instrumentation();
            assert_eq!(
                after_divergent.external_flushes - before_divergent.external_flushes,
                2,
                "an old-base delivery must authenticate its exact state and divergent current join"
            );
            assert_eq!(
                after_divergent.external_range_scans - before_divergent.external_range_scans,
                0,
                "divergent point updates must not physically scan external history"
            );
            assert!(
                after_divergent.external_history_page_reads
                    > before_divergent.external_history_page_reads
            );
            assert_eq!(receiver.status().accepted_batch_ids().unwrap().len(), 3);
            receiver.materialize_page(ids.page_a).unwrap();
            snapshots.push(receiver.canonical_snapshot().unwrap());
        }
        assert_eq!(snapshots[0], snapshots[1]);
    }
}

#[test]
fn correction11_late_block_creation_after_long_causal_chain_has_zero_ancestry_walks() {
    const CHAIN: usize = 48;
    let ids = Ids::new();
    let dir = TestDir::new("late-block-causal-chain");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    let initial = pages_only_genesis(ids, &engine, 84_000);
    writer.publish_prepared(&initial).unwrap();
    engine
        .stage_archive_batch(initial.manifest().batch_id())
        .unwrap();
    for index in 0..CHAIN {
        let page_id = if index % 2 == 0 {
            ids.page_a
        } else {
            ids.page_b
        };
        let edit = engine
            .prepare_bootstrap_transaction(
                author(84_001 + index as u128, 84_000),
                &tx(vec![SemanticOperation::EditPagePath {
                    page_id,
                    path: path(&format!("pages/chain-{index:03}.md")),
                }]),
            )
            .unwrap();
        writer.publish_prepared(&edit).unwrap();
        assert!(matches!(
            engine
                .stage_archive_batch(edit.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));
    }
    let before = engine.instrumentation();
    let create = engine
        .prepare_bootstrap_transaction(
            author(84_100, 84_000),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                page_id: ids.page_a,
                parent: None,
                order: "late".into(),
                content: "late block".into(),
            }]),
        )
        .unwrap();
    writer.publish_prepared(&create).unwrap();
    assert!(matches!(
        engine
            .stage_archive_batch(create.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    let after = engine.instrumentation();
    assert_eq!(after.ancestry_traversals - before.ancestry_traversals, 0);
    assert_eq!(after.external_flushes - before.external_flushes, 1);
    assert!(after.external_history_page_reads > before.external_history_page_reads);
    assert_eq!(after.scratch_syncs, 0);
    assert_eq!(engine.materialize_page(ids.page_a).unwrap().blocks.len(), 1);
}

#[test]
#[ignore = "sparse archive-open scaling measurement"]
fn sparse_archive_open_cost_is_independent_of_unrelated_batch_count() {
    use std::time::Instant;

    let ids = Ids::new();
    let unrelated = std::env::var("TINE_SPARSE_UNRELATED_BATCHES")
        .ok()
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(250);
    let dir = TestDir::new("sparse-open-measurement");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let baseline = genesis(ids, &ids.engine());
    writer.publish_prepared(&baseline).unwrap();
    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    engine
        .stage_archive_batch(baseline.manifest().batch_id())
        .unwrap();

    for index in 0..unrelated {
        let fixture = ids.engine();
        let prepared = fixture
            .prepare_bootstrap_transaction(
                author(50_000 + index as u128, 50_000 + index as u64),
                &tx(vec![SemanticOperation::CreatePage {
                    page_id: PageId::from_uuid(uuid(60_000 + index as u128)),
                    home_document_id: DocumentId::from_uuid(uuid(70_000 + index as u128)),
                    path: path(&format!("pages/Unrelated {index:08}.md")),
                }]),
            )
            .unwrap();
        writer.publish_prepared(&prepared).unwrap();
    }
    let started = Instant::now();
    let page = engine.materialize_page(ids.page_a).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(page.stats.catalog_documents_loaded, 1);
    assert_eq!(page.stats.membership_documents_loaded, 1);
    assert_eq!(page.stats.home_documents_loaded, 1);
    assert_eq!(page.stats.distinct_home_documents, vec![ids.home_a]);
    assert_eq!(page.stats.physical_manifest_reads, 1);
    assert_eq!(page.stats.physical_object_reads, 1);
    eprintln!(
        "sparse_archive_open unrelated_batches={unrelated} target_batches=1 referenced_homes=1 manifest_reads={} object_reads={} elapsed_us={}",
        page.stats.physical_manifest_reads,
        page.stats.physical_object_reads,
        elapsed.as_micros(),
    );
}

#[test]
fn incomplete_store_batch_becomes_ready_without_early_visibility() {
    let ids = Ids::new();
    let dir = TestDir::new("incomplete");
    let archive = store(&dir, ids);
    let mut engine = ids.engine();
    let prepared = genesis(ids, &engine);
    archive
        .stage_manifest_bytes(&prepared.manifest().encode().unwrap())
        .unwrap();
    assert!(matches!(
        engine
            .stage_from_store(&archive, prepared.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::IncompleteStaged {
            missing_objects,
            ..
        } if missing_objects == prepared.objects().len()
    ));
    assert!(engine.canonical_snapshot().unwrap().pages.is_empty());
    for object in prepared.objects().iter().rev() {
        archive
            .stage_object_bytes(&object.encode().unwrap())
            .unwrap();
    }
    assert!(matches!(
        engine
            .stage_from_store(&archive, prepared.manifest().batch_id())
            .unwrap()
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(engine.canonical_snapshot().unwrap().pages.len(), 3);
}

#[test]
fn workspace_and_lineage_mismatches_reject_without_visible_mutation() {
    let ids = Ids::new();
    let foreign_workspace_ids = Ids {
        workspace: WorkspaceId::from_uuid(uuid(9_001)),
        ..ids
    };
    let workspace_dir = TestDir::new("workspace-mismatch");
    let workspace_store = store(&workspace_dir, foreign_workspace_ids);
    let foreign_workspace_engine = foreign_workspace_ids.engine();
    let foreign_workspace_batch = ready(
        &workspace_store,
        &genesis(foreign_workspace_ids, &foreign_workspace_engine),
    );
    let mut receiver = ids.engine();
    assert!(matches!(
        receiver.stage_ready(foreign_workspace_batch).disposition,
        BatchDisposition::Rejected {
            error: EngineError::WorkspaceMismatch { .. },
            ..
        }
    ));
    assert!(receiver.canonical_snapshot().unwrap().pages.is_empty());

    let foreign_lineage_ids = Ids {
        lineage: LineageDigest::of(b"foreign-lineage"),
        ..ids
    };
    let lineage_dir = TestDir::new("lineage-mismatch");
    let lineage_store = store(&lineage_dir, foreign_lineage_ids);
    let foreign_lineage_engine = foreign_lineage_ids.engine();
    let foreign_lineage_batch = ready(
        &lineage_store,
        &genesis(foreign_lineage_ids, &foreign_lineage_engine),
    );
    assert!(matches!(
        receiver.stage_ready(foreign_lineage_batch).disposition,
        BatchDisposition::Rejected {
            error: EngineError::LineageMismatch { .. },
            ..
        }
    ));
    assert!(receiver.canonical_snapshot().unwrap().pages.is_empty());
}

#[test]
fn conflicting_reuse_of_an_accepted_batch_id_rejects_without_rollback() {
    let ids = Ids::new();
    let first_dir = TestDir::new("batch-id-first");
    let first_store = store(&first_dir, ids);
    let first_author = ids.engine();
    let first = genesis(ids, &first_author);
    let mut receiver = ids.engine();
    assert!(matches!(
        receiver
            .stage_ready(ready(&first_store, &first))
            .disposition,
        BatchDisposition::Accepted { .. }
    ));
    let before = receiver.canonical_snapshot().unwrap();

    let collision_dir = TestDir::new("batch-id-collision");
    let collision_store = store(&collision_dir, ids);
    let collision_author = ids.engine();
    let collision = collision_author
        .prepare_bootstrap_transaction(
            author(100, 100),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_a,
                home_document_id: ids.home_a,
                path: path("pages/Conflicting.md"),
            }]),
        )
        .unwrap();
    assert!(matches!(
        receiver
            .stage_ready(ready(&collision_store, &collision))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::BatchCollision(_),
            ..
        }
    ));
    assert_eq!(receiver.canonical_snapshot().unwrap(), before);
    assert_eq!(
        receiver.status().accepted_batch_ids().unwrap(),
        vec![author(100, 100).batch_id]
    );
}

#[test]
fn crdt_payload_is_bound_to_batch_and_same_batch_replay_is_a_duplicate_noop() {
    let ids = Ids::new();
    let dir = TestDir::new("payload-batch-binding");
    let archive = store(&dir, ids);
    let engine = ids.engine();
    let prepared = genesis(ids, &engine);
    let foreign_batch_id = BatchId::from_uuid(uuid(9_999));
    let rebound = rebuild_as(
        prepared.manifest(),
        foreign_batch_id,
        prepared.objects().to_vec(),
        prepared.manifest().dependency_frontier().clone(),
    );
    let mut receiver = ids.engine();
    assert!(matches!(
        receiver.stage_ready(ready(&archive, &rebound)).disposition,
        BatchDisposition::Rejected {
            error: EngineError::CrdtPayloadIdentityMismatch { .. },
            ..
        }
    ));

    let ready = ready(&archive, &prepared);
    assert!(matches!(
        receiver.stage_ready(ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        receiver.stage_ready(ready).disposition,
        BatchDisposition::DuplicateAccepted { .. }
    ));
}

#[test]
fn concurrent_same_block_id_in_distinct_homes_blocks_canonically_in_every_order() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(32));
    let dir = TestDir::new("concurrent-immutable-home-conflict");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();

    let fixture = ids.engine();
    let genesis = fixture
        .prepare_bootstrap_transaction(
            author(100, 100),
            &tx(vec![
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
            ]),
        )
        .unwrap();
    archive.publish_prepared(&genesis).unwrap();
    let genesis_id = genesis.manifest().batch_id();
    let genesis_ready = match archive.inspect_batch(genesis_id).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected ready genesis, found {other:?}"),
    };

    let mut author_a = ids.engine();
    let mut author_b = ids.engine();
    assert!(matches!(
        author_a.stage_ready(genesis_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        author_b.stage_ready(genesis_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let created_a = author_a
        .prepare_bootstrap_transaction(
            author(103, 103),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_a,
                },
                page_id: ids.page_a,
                parent: None,
                order: "x-a".into(),
                content: "concurrent content A".into(),
            }]),
        )
        .unwrap();
    let created_b = author_b
        .prepare_bootstrap_transaction(
            author(104, 104),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_b,
                },
                page_id: ids.page_b,
                parent: None,
                order: "x-b".into(),
                content: "concurrent content B".into(),
            }]),
        )
        .unwrap();
    archive.publish_prepared(&created_a).unwrap();
    archive.publish_prepared(&created_b).unwrap();
    let batch_a_id = created_a.manifest().batch_id();
    let batch_b_id = created_b.manifest().batch_id();
    let batch_a = match archive.inspect_batch(batch_a_id).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected ready A, found {other:?}"),
    };
    let batch_b = match archive.inspect_batch(batch_b_id).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected ready B, found {other:?}"),
    };

    assert!(matches!(
        author_a.stage_ready(batch_a.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        author_b.stage_ready(batch_b.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let dependent_a = author_a
        .prepare_bootstrap_transaction(
            author(105, 105),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_a,
                },
                content: "later content A".into(),
            }]),
        )
        .unwrap();
    let dependent_b = author_b
        .prepare_bootstrap_transaction(
            author(106, 106),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_b,
                },
                content: "later content B".into(),
            }]),
        )
        .unwrap();
    archive.publish_prepared(&dependent_a).unwrap();
    archive.publish_prepared(&dependent_b).unwrap();
    let dependent_a_id = dependent_a.manifest().batch_id();
    let dependent_b_id = dependent_b.manifest().batch_id();
    let dependent_a = match archive.inspect_batch(dependent_a_id).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected ready dependent A, found {other:?}"),
    };
    let dependent_b = match archive.inspect_batch(dependent_b_id).unwrap() {
        BatchInspection::Ready(batch) => batch,
        other => panic!("expected ready dependent B, found {other:?}"),
    };

    let expected = ImmutableHomeEvidence::new(vec![ImmutableHomeConflict::new(
        block_id,
        ImmutableHomeClaim::new(batch_a_id, ids.home_a),
        ImmutableHomeClaim::new(batch_b_id, ids.home_b),
    )]);
    for (mut engine, staged, conflicting) in [
        (author_a, dependent_b.clone(), batch_b.clone()),
        (author_b, dependent_a.clone(), batch_a.clone()),
    ] {
        assert!(matches!(
            engine.stage_ready(staged.clone()).disposition,
            BatchDisposition::IncompleteStaged { .. }
        ));
        assert!(matches!(
            engine.stage_ready(conflicting.clone()).disposition,
            BatchDisposition::Quarantined
        ));
        assert_eq!(engine.fatal_evidence(), Some(&expected));
        let expected_handle = engine.fatal_evidence_handle().unwrap();
        assert!(matches!(
            engine.stage_ready(staged).disposition,
            BatchDisposition::Quarantined
        ));
        assert!(matches!(
            engine.stage_ready(conflicting).disposition,
            BatchDisposition::Quarantined
        ));
        assert!(matches!(
            engine.prepare_bootstrap_transaction(
                author(107, 107),
                &tx(vec![SemanticOperation::EditPagePath {
                    page_id: ids.page_a,
                    path: path("pages/blocked.md"),
                }]),
            ),
            Err(EngineError::WorkspaceBlocked(found)) if found == expected_handle
        ));
        assert!(matches!(
            engine.materialize_page(ids.page_a),
            Err(EngineError::WorkspaceBlocked(found)) if found == expected_handle
        ));
        assert!(matches!(
            engine.canonical_snapshot(),
            Err(EngineError::WorkspaceBlocked(found)) if found == expected_handle
        ));
        assert!(matches!(
            engine.recover_block_state(ids.home_a, block_id),
            Err(EngineError::WorkspaceBlocked(found)) if found == expected_handle
        ));
        assert_eq!(engine.status().accepted_batch_ids().unwrap().len(), 2);
    }

    for (first, staged, conflicting) in [
        (batch_a_id, dependent_b_id, batch_b_id),
        (batch_b_id, dependent_a_id, batch_a_id),
    ] {
        let replay_store = ObjectStore::open(&archive_path, ids.workspace).unwrap();
        let mut replay =
            ShardedHotEngine::with_archive_store(replay_store, ids.lineage, ids.catalog);
        assert!(matches!(
            replay.stage_archive_batch(genesis_id).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            replay.stage_archive_batch(first).unwrap().disposition,
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            replay.stage_archive_batch(staged).unwrap().disposition,
            BatchDisposition::IncompleteStaged { .. }
        ));
        let conflicting_outcome = replay.stage_archive_batch(conflicting).unwrap();
        assert!(
            matches!(
                conflicting_outcome.disposition,
                BatchDisposition::Quarantined
            ),
            "unexpected replay conflict disposition: {:?}",
            conflicting_outcome.disposition
        );
        assert_eq!(paged_fatal_evidence(&replay), Some(expected.clone()));
        let expected_handle = replay.fatal_evidence_handle().unwrap();
        assert!(matches!(
            archive.inspect_batch(batch_a_id).unwrap(),
            BatchInspection::Ready(_)
        ));
        assert!(matches!(
            archive.inspect_batch(batch_b_id).unwrap(),
            BatchInspection::Ready(_)
        ));
        assert!(matches!(
            archive.inspect_batch(staged).unwrap(),
            BatchInspection::Ready(_)
        ));
        assert!(matches!(
            replay.stage_archive_batch(staged).unwrap().disposition,
            BatchDisposition::Quarantined
        ));
        assert!(matches!(
            replay.canonical_snapshot(),
            Err(EngineError::WorkspaceBlocked(found)) if found == expected_handle
        ));
        assert_eq!(replay.status().accepted_batch_ids().unwrap().len(), 2);
    }
}

#[test]
fn crossed_concurrent_identity_collisions_converge_live_and_from_fresh_store() {
    let ids = Ids::new();
    let block_x = tine_core::oplog::BlockId::from_uuid(uuid(40));
    let block_y = tine_core::oplog::BlockId::from_uuid(uuid(41));
    let dir = TestDir::new("crossed-identity-collisions");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let genesis = pages_only_genesis(ids, &ids.engine(), 200);
    let genesis_ready = ready(&archive, &genesis);

    let mut author_a = ids.engine();
    let mut author_b = ids.engine();
    author_a.stage_ready(genesis_ready.clone());
    author_b.stage_ready(genesis_ready.clone());
    let prepared_a = create_blocks(
        &author_a,
        201,
        &[
            (block_x, ids.page_a, ids.home_a, "x-a"),
            (block_y, ids.page_b, ids.home_b, "y-b"),
        ],
    );
    let prepared_b = create_blocks(
        &author_b,
        202,
        &[
            (block_y, ids.page_a, ids.home_a, "y-a"),
            (block_x, ids.page_b, ids.home_b, "x-b"),
        ],
    );
    let batch_a = ready(&archive, &prepared_a);
    let batch_b = ready(&archive, &prepared_b);
    let expected = ImmutableHomeEvidence::new(vec![
        ImmutableHomeConflict::new(
            block_x,
            ImmutableHomeClaim::new(prepared_a.manifest().batch_id(), ids.home_a),
            ImmutableHomeClaim::new(prepared_b.manifest().batch_id(), ids.home_b),
        ),
        ImmutableHomeConflict::new(
            block_y,
            ImmutableHomeClaim::new(prepared_b.manifest().batch_id(), ids.home_a),
            ImmutableHomeClaim::new(prepared_a.manifest().batch_id(), ids.home_b),
        ),
    ]);

    let mut live_evidence = Vec::new();
    for order in [
        [batch_a.clone(), batch_b.clone()],
        [batch_b.clone(), batch_a.clone()],
    ] {
        let mut receiver = ids.engine();
        receiver.stage_ready(genesis_ready.clone());
        for batch in order {
            receiver.stage_ready(batch);
        }
        live_evidence.push(receiver.fatal_evidence().cloned().unwrap());
    }
    assert_eq!(live_evidence, vec![expected.clone(), expected.clone()]);

    let genesis_id = genesis.manifest().batch_id();
    let batch_a_id = prepared_a.manifest().batch_id();
    let batch_b_id = prepared_b.manifest().batch_id();
    let mut replay_evidence = Vec::new();
    for order in [[batch_a_id, batch_b_id], [batch_b_id, batch_a_id]] {
        let store = ObjectStore::open(&archive_path, ids.workspace).unwrap();
        let mut receiver = ShardedHotEngine::with_archive_store(store, ids.lineage, ids.catalog);
        receiver.stage_archive_batch(genesis_id).unwrap();
        for batch_id in order {
            receiver.stage_archive_batch(batch_id).unwrap();
        }
        let instrumentation = receiver.instrumentation();
        assert_eq!(instrumentation.block_claim_hot_entries, 0);
        assert!(instrumentation.store.block_claim_index_reads > 0);
        assert!(instrumentation.store.block_claim_index_writes > 0);
        assert_eq!(receiver.fatal_evidence(), None);
        let first = receiver.fatal_evidence_page(None, 1).unwrap().unwrap();
        assert_eq!(first.conflicts().len(), 1);
        let second = receiver
            .fatal_evidence_page(first.next(), 1)
            .unwrap()
            .unwrap();
        assert_eq!(second.conflicts().len(), 1);
        assert_eq!(second.next(), None);
        replay_evidence.push(ImmutableHomeEvidence::new(
            first
                .conflicts()
                .iter()
                .chain(second.conflicts())
                .cloned()
                .collect(),
        ));
        assert_eq!(receiver.instrumentation().conflict_hot_entries, 0);
        tamper_active_scratch_pages(&archive_path);
        assert!(matches!(
            receiver.fatal_evidence_page(None, 1),
            Err(EngineError::Archive(_))
        ));
    }
    assert_eq!(replay_evidence, live_evidence);
}

#[test]
fn concurrent_same_home_duplicate_creation_converges_after_fresh_replay() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(56));
    let dir = TestDir::new("same-home-duplicate-replay");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let genesis = pages_only_genesis(ids, &ids.engine(), 300);
    archive.publish_prepared(&genesis).unwrap();
    let genesis_ready = ready(&archive, &genesis);

    let mut author_a = ids.engine();
    let mut author_b = ids.engine();
    author_a.stage_ready(genesis_ready.clone());
    author_b.stage_ready(genesis_ready);
    let claim_a = create_blocks(&author_a, 301, &[(block_id, ids.page_a, ids.home_a, "a")]);
    let claim_b = author_b
        .prepare_bootstrap_transaction(
            author(302, 302),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_a,
                },
                page_id: ids.page_a,
                parent: None,
                order: "b".into(),
                content: "concurrent same-home duplicate".into(),
            }]),
        )
        .unwrap();
    archive.publish_prepared(&claim_a).unwrap();
    archive.publish_prepared(&claim_b).unwrap();

    let mut snapshots = Vec::new();
    for order in [
        [claim_a.manifest().batch_id(), claim_b.manifest().batch_id()],
        [claim_b.manifest().batch_id(), claim_a.manifest().batch_id()],
    ] {
        let store = ObjectStore::open(&archive_path, ids.workspace).unwrap();
        let mut replay = ShardedHotEngine::with_archive_store(store, ids.lineage, ids.catalog);
        assert!(matches!(
            replay
                .stage_archive_batch(genesis.manifest().batch_id())
                .unwrap()
                .disposition,
            BatchDisposition::Accepted { .. }
        ));
        for batch_id in order {
            assert!(matches!(
                replay.stage_archive_batch(batch_id).unwrap().disposition,
                BatchDisposition::Accepted { .. }
            ));
        }
        assert_eq!(replay.fatal_evidence(), None);
        assert_eq!(replay.instrumentation().block_claim_hot_entries, 0);
        assert!(replay.instrumentation().store.block_claim_index_reads > 0);
        snapshots.push(replay.canonical_snapshot().unwrap());
    }
    assert_eq!(snapshots[0], snapshots[1]);
}

#[test]
fn three_concurrent_identity_claims_and_later_blocked_ingress_have_one_evidence_set() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(42));
    let dir = TestDir::new("three-identity-claims");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 210);
    let genesis_ready = ready(&archive, &genesis);
    let mut claims = Vec::new();
    for (batch, page_id, home_document_id) in [
        (211, ids.page_a, ids.home_a),
        (212, ids.page_b, ids.home_b),
        (213, ids.page_c, ids.home_c),
    ] {
        let mut claim_author = ids.engine();
        claim_author.stage_ready(genesis_ready.clone());
        let prepared = create_blocks(
            &claim_author,
            batch,
            &[(block_id, page_id, home_document_id, "claim")],
        );
        claims.push(ready(&archive, &prepared));
    }
    let mut malformed_author = ids.engine();
    malformed_author.stage_ready(genesis_ready.clone());
    let malformed_prepared = create_blocks(
        &malformed_author,
        214,
        &[(block_id, ids.page_a, ids.home_a, "invalid")],
    );
    let malformed_objects = malformed_prepared
        .objects()
        .iter()
        .map(|object| {
            if object.kind() == ObjectKind::CrdtUpdate {
                OperationObject::new(
                    ids.workspace,
                    object.document_id(),
                    ObjectKind::CrdtUpdate,
                    b"invalid-crdt-evidence".to_vec(),
                )
                .unwrap()
            } else {
                object.clone()
            }
        })
        .collect();
    let malformed = rebuild(
        malformed_prepared.manifest(),
        malformed_objects,
        malformed_prepared.manifest().dependency_frontier().clone(),
    );
    let malformed = ready(&archive, &malformed);

    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut evidence = Vec::new();
    for permutation in permutations {
        let mut receiver = ids.engine();
        receiver.stage_ready(genesis_ready.clone());
        receiver.stage_ready(claims[permutation[0]].clone());
        receiver.stage_ready(claims[permutation[1]].clone());
        assert!(receiver.fatal_evidence().is_some());
        receiver.stage_ready(claims[permutation[2]].clone());
        let before_invalid = receiver.fatal_evidence().cloned().unwrap();
        let terminal_before_invalid = receiver
            .status()
            .validated_unpublished_batch_ids()
            .unwrap()
            .to_vec();
        assert!(matches!(
            receiver.stage_ready(malformed.clone()).disposition,
            BatchDisposition::Rejected { .. }
        ));
        assert_eq!(receiver.fatal_evidence(), Some(&before_invalid));
        assert_eq!(
            receiver.status().validated_unpublished_batch_ids().unwrap(),
            terminal_before_invalid
        );
        evidence.push(before_invalid);
    }
    let expected = ImmutableHomeEvidence::new(vec![ImmutableHomeConflict::from_claims(
        block_id,
        [
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(211)), ids.home_a),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(212)), ids.home_b),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(213)), ids.home_c),
        ],
    )]);
    assert_eq!(evidence, vec![expected; permutations.len()]);
}

fn permutations_of_four() -> Vec<[usize; 4]> {
    let mut permutations = Vec::new();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let candidate = [a, b, c, d];
                    if candidate
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == 4
                    {
                        permutations.push(candidate);
                    }
                }
            }
        }
    }
    permutations
}

#[test]
fn correction6_four_independent_claims_retain_complete_evidence_in_all_orders() {
    let ids = Ids::new();
    let block_x = tine_core::oplog::BlockId::from_uuid(uuid(46));
    let block_y = tine_core::oplog::BlockId::from_uuid(uuid(47));
    let dir = TestDir::new("correction6-four-independent-claims");
    let archive_path = dir.path().join("archive");
    let archive = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let genesis = pages_only_genesis(ids, &ids.engine(), 240);
    let genesis_ready = ready(&archive, &genesis);
    let mut batches = Vec::new();
    for (batch, block_id, page_id, home_document_id) in [
        (241, block_x, ids.page_a, ids.home_a),
        (242, block_x, ids.page_b, ids.home_b),
        (243, block_y, ids.page_a, ids.home_a),
        (244, block_y, ids.page_b, ids.home_b),
    ] {
        let mut claim_author = ids.engine();
        claim_author.stage_ready(genesis_ready.clone());
        batches.push(ready(
            &archive,
            &create_blocks(
                &claim_author,
                batch,
                &[(block_id, page_id, home_document_id, "claim")],
            ),
        ));
    }
    let expected = ImmutableHomeEvidence::new(vec![
        ImmutableHomeConflict::new(
            block_x,
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(241)), ids.home_a),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(242)), ids.home_b),
        ),
        ImmutableHomeConflict::new(
            block_y,
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(243)), ids.home_a),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(244)), ids.home_b),
        ),
    ]);

    let permutations = permutations_of_four();
    assert_eq!(permutations.len(), 24);
    for permutation in &permutations {
        let mut receiver = ids.engine();
        receiver.stage_ready(genesis_ready.clone());
        for index in permutation {
            receiver.stage_ready(batches[*index].clone());
        }
        assert_eq!(paged_fatal_evidence(&receiver), Some(expected.clone()));
    }

    let genesis_id = genesis.manifest().batch_id();
    let batch_ids: Vec<_> = batches
        .iter()
        .map(|batch| batch.manifest().batch_id())
        .collect();
    for permutation in permutations {
        let store = ObjectStore::open(&archive_path, ids.workspace).unwrap();
        let mut receiver = ShardedHotEngine::with_archive_store(store, ids.lineage, ids.catalog);
        receiver.stage_archive_batch(genesis_id).unwrap();
        for index in permutation {
            receiver.stage_archive_batch(batch_ids[index]).unwrap();
        }
        let handle = receiver.fatal_evidence_handle().unwrap();
        assert_eq!(handle.conflicting_block_count(), 2);
        assert_eq!(handle.claim_count(), 4);
        let instrumentation = receiver.instrumentation();
        assert_eq!(instrumentation.conflict_hot_entries, 0);
        assert_eq!(instrumentation.batch_status_hot_entries, 0);
        assert_eq!(instrumentation.ready_payload_hot_entries, 0);
        assert!(instrumentation.external_flushes > 0);
        assert!(instrumentation.external_history_page_reads > 0);
        assert_eq!(instrumentation.scratch_syncs, 0);
        assert!(instrumentation.document_hot_entries <= 65);
        assert_eq!(paged_fatal_evidence(&receiver), Some(expected.clone()));
    }
}

#[test]
fn correction6_blocked_frontier_validates_child_before_parent_and_finds_new_conflict() {
    let ids = Ids::new();
    let conflict_x = tine_core::oplog::BlockId::from_uuid(uuid(48));
    let conflict_y = tine_core::oplog::BlockId::from_uuid(uuid(49));
    let parent_block = tine_core::oplog::BlockId::from_uuid(uuid(50));
    let dir = TestDir::new("correction6-blocked-frontier-chain");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 250);
    let genesis_ready = ready(&archive, &genesis);

    let mut left_author = ids.engine();
    left_author.stage_ready(genesis_ready.clone());
    let x_left = ready(
        &archive,
        &create_blocks(
            &left_author,
            251,
            &[(conflict_x, ids.page_a, ids.home_a, "x-left")],
        ),
    );
    let mut right_author = ids.engine();
    right_author.stage_ready(genesis_ready.clone());
    let x_right = ready(
        &archive,
        &create_blocks(
            &right_author,
            252,
            &[(conflict_x, ids.page_b, ids.home_b, "x-right")],
        ),
    );
    let y_right = ready(
        &archive,
        &create_blocks(
            &right_author,
            253,
            &[(conflict_y, ids.page_b, ids.home_b, "y-right")],
        ),
    );

    let mut chain_author = ids.engine();
    chain_author.stage_ready(genesis_ready.clone());
    let parent = create_blocks(
        &chain_author,
        260,
        &[(parent_block, ids.page_a, ids.home_a, "parent")],
    );
    let parent_ready = ready(&archive, &parent);
    chain_author.stage_ready(parent_ready.clone());
    // The child BatchId deliberately sorts before its parent so blocked
    // draining must reach a fixed point instead of relying on BatchId order.
    let child = create_blocks(
        &chain_author,
        259,
        &[(conflict_y, ids.page_a, ids.home_a, "child")],
    );
    let child_ready = ready(&archive, &child);

    let mut receiver = ids.engine();
    receiver.stage_ready(genesis_ready);
    receiver.stage_ready(x_left);
    assert!(matches!(
        receiver.stage_ready(x_right).disposition,
        BatchDisposition::Quarantined
    ));
    assert!(matches!(
        receiver.stage_ready(child_ready.clone()).disposition,
        BatchDisposition::IncompleteStaged { .. }
    ));
    assert!(matches!(
        receiver.stage_ready(y_right).disposition,
        BatchDisposition::Quarantined
    ));
    assert!(matches!(
        receiver.stage_ready(parent_ready).disposition,
        BatchDisposition::Quarantined
    ));
    let child_outcome = receiver.stage_ready(child_ready).disposition;
    assert!(
        matches!(child_outcome, BatchDisposition::Quarantined),
        "unexpected terminal child outcome: {child_outcome:?}"
    );
    let expected = ImmutableHomeEvidence::new(vec![
        ImmutableHomeConflict::new(
            conflict_x,
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(251)), ids.home_a),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(252)), ids.home_b),
        ),
        ImmutableHomeConflict::new(
            conflict_y,
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(259)), ids.home_a),
            ImmutableHomeClaim::new(BatchId::from_uuid(uuid(253)), ids.home_b),
        ),
    ]);
    assert_eq!(receiver.fatal_evidence(), Some(&expected));

    let store = ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(store, ids.lineage, ids.catalog);
    for batch_id in [
        genesis.manifest().batch_id(),
        BatchId::from_uuid(uuid(251)),
        BatchId::from_uuid(uuid(252)),
        child.manifest().batch_id(),
        BatchId::from_uuid(uuid(253)),
        parent.manifest().batch_id(),
    ] {
        replay.stage_archive_batch(batch_id).unwrap();
    }
    assert_eq!(paged_fatal_evidence(&replay), Some(expected.clone()));
    assert_eq!(
        replay.status().validated_unpublished_batch_ids().unwrap(),
        &[
            BatchId::from_uuid(uuid(252)),
            BatchId::from_uuid(uuid(253)),
            BatchId::from_uuid(uuid(259)),
            BatchId::from_uuid(uuid(260)),
        ]
    );
}

#[test]
fn correction6_latching_batch_retains_novel_claim_for_later_conflict() {
    let ids = Ids::new();
    let block_x = tine_core::oplog::BlockId::from_uuid(uuid(51));
    let block_y = tine_core::oplog::BlockId::from_uuid(uuid(52));
    let dir = TestDir::new("correction6-latch-batch-novel-claim");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 270);
    let genesis_ready = ready(&archive, &genesis);

    let mut left = ids.engine();
    left.stage_ready(genesis_ready.clone());
    let x_left = ready(
        &archive,
        &create_blocks(&left, 271, &[(block_x, ids.page_a, ids.home_a, "x-left")]),
    );
    let mut right = ids.engine();
    right.stage_ready(genesis_ready.clone());
    let latch = ready(
        &archive,
        &create_blocks(
            &right,
            272,
            &[
                (block_x, ids.page_b, ids.home_b, "x-right"),
                (block_y, ids.page_a, ids.home_a, "y-left"),
            ],
        ),
    );
    let y_right = ready(
        &archive,
        &create_blocks(&right, 273, &[(block_y, ids.page_b, ids.home_b, "y-right")]),
    );

    let mut receiver = ids.engine();
    for batch in [genesis_ready, x_left, latch, y_right] {
        receiver.stage_ready(batch);
    }
    assert_eq!(receiver.fatal_evidence().unwrap().conflicts().len(), 2);
    assert_eq!(
        receiver
            .fatal_evidence()
            .unwrap()
            .conflicts()
            .iter()
            .map(ImmutableHomeConflict::block_id)
            .collect::<Vec<_>>(),
        vec![block_x, block_y]
    );
}

#[test]
fn correction6_quarantined_parent_makes_causal_duplicate_child_reject() {
    let ids = Ids::new();
    let conflict = tine_core::oplog::BlockId::from_uuid(uuid(54));
    let causal_duplicate = tine_core::oplog::BlockId::from_uuid(uuid(55));
    let dir = TestDir::new("correction6-terminal-causal-duplicate");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 280);
    let genesis_ready = ready(&archive, &genesis);

    let mut left = ids.engine();
    left.stage_ready(genesis_ready.clone());
    let left_claim = ready(
        &archive,
        &create_blocks(&left, 281, &[(conflict, ids.page_a, ids.home_a, "left")]),
    );
    let mut right = ids.engine();
    right.stage_ready(genesis_ready.clone());
    let right_claim = ready(
        &archive,
        &create_blocks(&right, 282, &[(conflict, ids.page_b, ids.home_b, "right")]),
    );

    let mut parent_author = ids.engine();
    parent_author.stage_ready(genesis_ready.clone());
    let parent = create_blocks(
        &parent_author,
        290,
        &[(causal_duplicate, ids.page_a, ids.home_a, "parent")],
    );
    let parent_ready = ready(&archive, &parent);
    parent_author.stage_ready(parent_ready.clone());
    let dependency_template = parent_author
        .prepare_bootstrap_transaction(
            author(291, 291),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: causal_duplicate,
                    home_document_id: ids.home_a,
                },
                content: "dependency template".into(),
            }]),
        )
        .unwrap();
    let parent_home_dependency = dependency_template
        .manifest()
        .dependency_frontier()
        .documents()
        .iter()
        .find(|entry| entry.document_id() == ids.home_a)
        .unwrap()
        .clone();

    let duplicate = create_blocks(
        &right,
        // Sort before the parent to exercise child-before-parent draining.
        289,
        &[(causal_duplicate, ids.page_b, ids.home_b, "duplicate")],
    );
    let mut child_frontier = duplicate
        .manifest()
        .dependency_frontier()
        .documents()
        .to_vec();
    child_frontier.push(parent_home_dependency);
    let child = rebuild_with_compact_witness(&duplicate, FrontierV2::new(child_frontier).unwrap());
    let child_ready = ready(&archive, &child);

    let mut receiver = ids.engine();
    for batch in [genesis_ready, left_claim, right_claim] {
        receiver.stage_ready(batch);
    }
    assert!(matches!(
        receiver.stage_ready(child_ready.clone()).disposition,
        BatchDisposition::IncompleteStaged { .. }
    ));
    receiver.stage_ready(parent_ready);
    let child_outcome = receiver.stage_ready(child_ready).disposition;
    assert!(
        matches!(
            child_outcome,
            BatchDisposition::Rejected {
                error: EngineError::BlockAlreadyExists(found),
            }
            if found == causal_duplicate
        ),
        "unexpected causal terminal-parent outcome: {child_outcome:?}"
    );
    assert_eq!(receiver.fatal_evidence().unwrap().conflicts().len(), 1);

    let store = ObjectStore::open(&dir.path().join("store"), ids.workspace).unwrap();
    let mut replay = ShardedHotEngine::with_archive_store(store, ids.lineage, ids.catalog);
    for batch_id in [
        genesis.manifest().batch_id(),
        BatchId::from_uuid(uuid(281)),
        BatchId::from_uuid(uuid(282)),
        child.manifest().batch_id(),
        parent.manifest().batch_id(),
    ] {
        replay.stage_archive_batch(batch_id).unwrap();
    }
    let replay_child = replay
        .stage_archive_batch(child.manifest().batch_id())
        .unwrap()
        .disposition;
    assert!(
        matches!(
            replay_child,
            BatchDisposition::Rejected {
                error: EngineError::BlockAlreadyExists(found),
            }
            if found == causal_duplicate
        ),
        "unexpected replay child disposition: {replay_child:?}"
    );
    assert_eq!(paged_fatal_evidence(&replay).unwrap().conflicts().len(), 1);
}

#[test]
fn author_refuses_same_batch_cross_home_duplicate_without_retained_claim() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(43));
    let dir = TestDir::new("same-batch-identity-duplicate");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 220);
    let genesis_ready = ready(&archive, &genesis);
    let mut author_engine = ids.engine();
    author_engine.stage_ready(genesis_ready.clone());
    let malformed = author_engine.prepare_bootstrap_transaction(
        author(221, 221),
        &tx(vec![
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_a,
                },
                page_id: ids.page_a,
                parent: None,
                order: "a".into(),
                content: "a".into(),
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id,
                    home_document_id: ids.home_b,
                },
                page_id: ids.page_b,
                parent: None,
                order: "b".into(),
                content: "b".into(),
            },
        ]),
    );
    assert!(matches!(
        malformed,
        Err(EngineError::BlockAlreadyExists(found)) if found == block_id
    ));
    assert_eq!(author_engine.fatal_evidence(), None);
    assert_eq!(
        author_engine.status().accepted_batch_ids().unwrap(),
        vec![genesis.manifest().batch_id()]
    );
}

#[test]
fn mid_drain_acceptance_and_blocked_duplicate_report_truthful_batch_dispositions() {
    let ids = Ids::new();
    let conflict_id = tine_core::oplog::BlockId::from_uuid(uuid(44));
    let dependency_id = tine_core::oplog::BlockId::from_uuid(uuid(45));
    let dir = TestDir::new("mid-drain-blocked-status");
    let archive = store(&dir, ids);
    let genesis = pages_only_genesis(ids, &ids.engine(), 230);
    let genesis_ready = ready(&archive, &genesis);

    let mut author_a = ids.engine();
    author_a.stage_ready(genesis_ready.clone());
    let claim_a = create_blocks(
        &author_a,
        231,
        &[(conflict_id, ids.page_a, ids.home_a, "a")],
    );
    let claim_a_ready = ready(&archive, &claim_a);

    let mut author_b = ids.engine();
    author_b.stage_ready(genesis_ready.clone());
    let dependency = create_blocks(
        &author_b,
        232,
        &[(dependency_id, ids.page_b, ids.home_b, "dependency")],
    );
    let dependency_ready = ready(&archive, &dependency);
    author_b.stage_ready(dependency_ready.clone());
    let claim_b = create_blocks(
        &author_b,
        233,
        &[(conflict_id, ids.page_b, ids.home_b, "b")],
    );
    let claim_b_ready = ready(&archive, &claim_b);

    let mut receiver = ids.engine();
    receiver.stage_ready(genesis_ready);
    receiver.stage_ready(claim_a_ready.clone());
    assert!(matches!(
        receiver.stage_ready(claim_b_ready).disposition,
        BatchDisposition::IncompleteStaged { .. }
    ));
    let dependency_outcome = receiver.stage_ready(dependency_ready);
    assert_eq!(
        dependency_outcome.batch_id(),
        dependency.manifest().batch_id()
    );
    assert_eq!(
        dependency_outcome.disposition,
        BatchDisposition::Accepted { no_op: false }
    );
    assert_eq!(
        dependency_outcome
            .newly_accepted()
            .iter()
            .map(|accepted| accepted.batch_id)
            .collect::<Vec<_>>(),
        vec![dependency.manifest().batch_id()]
    );
    assert_eq!(
        dependency_outcome.status().workspace(),
        &WorkspaceStatus::Blocked(receiver.fatal_evidence_handle().unwrap())
    );
    assert_eq!(
        dependency_outcome.status().accepted_batch_ids().unwrap(),
        vec![
            genesis.manifest().batch_id(),
            claim_a.manifest().batch_id(),
            dependency.manifest().batch_id(),
        ]
    );
    let duplicate_outcome = receiver.stage_ready(claim_a_ready);
    assert_eq!(duplicate_outcome.batch_id(), claim_a.manifest().batch_id());
    assert_eq!(
        duplicate_outcome.disposition,
        BatchDisposition::DuplicateAccepted { no_op: false }
    );
    assert_eq!(
        duplicate_outcome.status().workspace(),
        &WorkspaceStatus::Blocked(receiver.fatal_evidence_handle().unwrap())
    );
    assert_eq!(
        receiver.status().accepted_batch_ids().unwrap(),
        vec![
            genesis.manifest().batch_id(),
            claim_a.manifest().batch_id(),
            dependency.manifest().batch_id(),
        ]
    );
}

#[test]
fn subtree_reorder_and_rename_referrer_transaction_preserve_atomic_semantics() {
    let ids = Ids::new();
    let child = tine_core::oplog::BlockId::from_uuid(uuid(32));
    let dir = TestDir::new("operation-surface");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);
    let created = engine
        .prepare_bootstrap_transaction(
            author(104, 104),
            &tx(vec![SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: child,
                    home_document_id: ids.home_a,
                },
                page_id: ids.page_a,
                parent: Some(ids.block_a),
                order: "child".into(),
                content: "ref [[A]]".into(),
            }]),
        )
        .unwrap();
    engine.stage_ready(ready(&archive, &created));
    let moved_and_reordered = engine
        .prepare_bootstrap_transaction(
            author(105, 105),
            &tx(vec![
                SemanticOperation::MoveSubtree {
                    root: BlockLocation {
                        block_id: ids.block_a,
                        home_document_id: ids.home_a,
                    },
                    from_page_id: ids.page_a,
                    to_page_id: ids.page_b,
                    parent: None,
                    order: "root-moved".into(),
                },
                SemanticOperation::ReorderBlock {
                    block_id: child,
                    page_id: ids.page_b,
                    parent: Some(ids.block_a),
                    order: "child-reordered".into(),
                },
            ]),
        )
        .unwrap();
    engine.stage_ready(ready(&archive, &moved_and_reordered));
    let renamed = engine
        .prepare_bootstrap_transaction(
            author(106, 106),
            &tx(vec![SemanticOperation::RenamePageAndRewriteReferrers {
                page_id: ids.page_a,
                path: path("pages/A Renamed.md"),
                referrers: vec![(
                    BlockLocation {
                        block_id: child,
                        home_document_id: ids.home_a,
                    },
                    "ref [[A Renamed]]".into(),
                )],
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &renamed)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let page_b = engine.materialize_page(ids.page_b).unwrap();
    assert_eq!(page_b.blocks.len(), 2);
    let child = page_b
        .blocks
        .iter()
        .find(|block| block.block_id == child)
        .unwrap();
    assert_eq!(child.parent, Some(ids.block_a));
    assert_eq!(child.order, "child-reordered");
    assert_eq!(child.content, "ref [[A Renamed]]");
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().path,
        path("pages/A Renamed.md")
    );
}

#[test]
fn causal_frontier_and_semantic_effect_tampering_fail_closed_at_ready_boundary() {
    let ids = Ids::new();
    let dir = TestDir::new("tamper");
    let archive = store(&dir, ids);
    let (engine, genesis_ready) = seed_engine(ids, &archive);
    let edit = engine
        .prepare_bootstrap_transaction(
            author(103, 103),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "edited".into(),
            }]),
        )
        .unwrap();

    let original = &edit.manifest().dependency_frontier().documents()[0];
    let tampered_frontier = FrontierV2::new(vec![DocumentDependencies::new(
        original.document_id(),
        original.peer_counters().to_vec(),
        Vec::new(),
    )
    .unwrap()])
    .unwrap();
    let frontier_tampered = rebuild(edit.manifest(), edit.objects().to_vec(), tampered_frontier);
    let frontier_ready = ready(&archive, &frontier_tampered);
    let mut receiver = ids.engine();
    receiver.stage_ready(genesis_ready.clone());
    assert!(matches!(
        receiver.stage_ready(frontier_ready).disposition,
        BatchDisposition::Rejected { .. }
    ));
    assert_eq!(
        receiver.materialize_page(ids.page_a).unwrap().blocks[0].content,
        "home A content"
    );

    let empty_effect = SemanticEffect::new(Vec::new(), Vec::new(), Vec::new())
        .unwrap()
        .encode()
        .unwrap();
    let objects = edit
        .objects()
        .iter()
        .map(|object| {
            if object.kind() == ObjectKind::SemanticEffect {
                OperationObject::new(
                    ids.workspace,
                    object.document_id(),
                    ObjectKind::SemanticEffect,
                    empty_effect.clone(),
                )
                .unwrap()
            } else {
                object.clone()
            }
        })
        .collect();
    let semantic_tampered = rebuild(
        edit.manifest(),
        objects,
        edit.manifest().dependency_frontier().clone(),
    );
    let semantic_dir = TestDir::new("semantic-tamper-store");
    let semantic_archive = store(&semantic_dir, ids);
    let mut receiver = ids.engine();
    receiver.stage_ready(genesis_ready);
    assert!(matches!(
        receiver
            .stage_ready(ready(&semantic_archive, &semantic_tampered))
            .disposition,
        BatchDisposition::Rejected { .. }
    ));
}

fn concurrent_ready(
    ids: Ids,
    archive: &ObjectStore,
    baseline: &ValidatedBatch,
    left_author: AuthorBatch,
    left_tx: OperationTransaction,
    right_author: AuthorBatch,
    right_tx: OperationTransaction,
) -> (ValidatedBatch, ValidatedBatch) {
    let mut left = ids.engine();
    let mut right = ids.engine();
    left.stage_ready(baseline.clone());
    right.stage_ready(baseline.clone());
    let left = left
        .prepare_bootstrap_transaction(left_author, &left_tx)
        .unwrap();
    let right = right
        .prepare_bootstrap_transaction(right_author, &right_tx)
        .unwrap();
    (ready(archive, &left), ready(archive, &right))
}

fn concurrent_ready_from(
    ids: Ids,
    archive: &ObjectStore,
    baselines: &[ValidatedBatch],
    left_author: AuthorBatch,
    left_tx: OperationTransaction,
    right_author: AuthorBatch,
    right_tx: OperationTransaction,
) -> (ValidatedBatch, ValidatedBatch) {
    let mut left = ids.engine();
    let mut right = ids.engine();
    for baseline in baselines {
        left.stage_ready(baseline.clone());
        right.stage_ready(baseline.clone());
    }
    let left = left
        .prepare_bootstrap_transaction(left_author, &left_tx)
        .unwrap();
    let right = right
        .prepare_bootstrap_transaction(right_author, &right_tx)
        .unwrap();
    (ready(archive, &left), ready(archive, &right))
}

fn apply_pair(
    ids: Ids,
    baseline: &ValidatedBatch,
    first: ValidatedBatch,
    second: ValidatedBatch,
) -> ShardedHotEngine {
    let mut engine = ids.engine();
    engine.stage_ready(baseline.clone());
    assert!(!matches!(
        engine.stage_ready(first).disposition,
        BatchDisposition::Rejected { .. }
    ));
    assert!(!matches!(
        engine.stage_ready(second).disposition,
        BatchDisposition::Rejected { .. }
    ));
    engine
}

fn apply_pair_from(
    ids: Ids,
    baselines: &[ValidatedBatch],
    first: ValidatedBatch,
    second: ValidatedBatch,
) -> ShardedHotEngine {
    let mut engine = ids.engine();
    for baseline in baselines {
        engine.stage_ready(baseline.clone());
    }
    assert!(!matches!(
        engine.stage_ready(first).disposition,
        BatchDisposition::Rejected { .. }
    ));
    assert!(!matches!(
        engine.stage_ready(second).disposition,
        BatchDisposition::Rejected { .. }
    ));
    engine
}

#[test]
fn concurrent_move_move_and_move_edit_converge_in_both_delivery_orders() {
    let ids = Ids::new();
    let dir = TestDir::new("move-concurrency");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let move_b = tx(vec![SemanticOperation::MoveSubtree {
        root: BlockLocation {
            block_id: ids.block_a,
            home_document_id: ids.home_a,
        },
        from_page_id: ids.page_a,
        to_page_id: ids.page_b,
        parent: None,
        order: "b".into(),
    }]);
    let move_c = tx(vec![SemanticOperation::MoveSubtree {
        root: BlockLocation {
            block_id: ids.block_a,
            home_document_id: ids.home_a,
        },
        from_page_id: ids.page_a,
        to_page_id: ids.page_c,
        parent: None,
        order: "c".into(),
    }]);
    let (left, right) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(110, 110),
        move_b.clone(),
        author(111, 111),
        move_c,
    );
    let ab = apply_pair(ids, &baseline, left.clone(), right.clone());
    let ba = apply_pair(ids, &baseline, right, left);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    let visible = [ids.page_b, ids.page_c]
        .into_iter()
        .filter(|page| !ab.materialize_page(*page).unwrap().blocks.is_empty())
        .count();
    assert_eq!(visible, 1, "losing membership claim must be filtered");

    let edit = tx(vec![SemanticOperation::EditBlockContent {
        block: BlockLocation {
            block_id: ids.block_a,
            home_document_id: ids.home_a,
        },
        content: "concurrent edit survives move".into(),
    }]);
    let (moved, edited) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(112, 112),
        move_b,
        author(113, 113),
        edit,
    );
    let ab = apply_pair(ids, &baseline, moved.clone(), edited.clone());
    let ba = apply_pair(ids, &baseline, edited, moved);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    assert_eq!(
        ab.materialize_page(ids.page_b).unwrap().blocks[0].content,
        "concurrent edit survives move"
    );
}

fn move_delete_result(move_peer: u64, delete_peer: u64) -> (bool, bool) {
    let ids = Ids::new();
    let dir = TestDir::new("move-delete-direction");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let moved = tx(vec![SemanticOperation::MoveSubtree {
        root: BlockLocation {
            block_id: ids.block_a,
            home_document_id: ids.home_a,
        },
        from_page_id: ids.page_a,
        to_page_id: ids.page_b,
        parent: None,
        order: "m".into(),
    }]);
    let deleted = tx(vec![SemanticOperation::DeleteSubtree {
        root_block_id: ids.block_a,
        page_id: ids.page_a,
    }]);
    let (moved, deleted) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(10_000 + move_peer as u128, move_peer),
        moved,
        author(20_000 + delete_peer as u128, delete_peer),
        deleted,
    );
    let ab = apply_pair(ids, &baseline, moved.clone(), deleted.clone());
    let ba = apply_pair(ids, &baseline, deleted, moved);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );
    let page_won = !ab.materialize_page(ids.page_b).unwrap().blocks.is_empty();
    (page_won, !page_won)
}

#[test]
fn concurrent_move_delete_covers_page_and_tombstone_winner_directions() {
    let low_move = move_delete_result(200, 300);
    let high_move = move_delete_result(400, 300);
    assert_ne!(
        low_move, high_move,
        "peer order must exercise both register winners"
    );
    assert!(
        low_move.0 || high_move.0,
        "one direction must keep the moved page owner"
    );
    assert!(
        low_move.1 || high_move.1,
        "one direction must keep the tombstone owner"
    );
}

fn moved_away_move_delete_result(move_peer: u64, delete_peer: u64) -> bool {
    let ids = Ids::new();
    let dir = TestDir::new("moved-away-move-delete");
    let archive = store(&dir, ids);
    let (mut seed, genesis_ready) = seed_engine(ids, &archive);
    let moved_to_b = seed
        .prepare_bootstrap_transaction(
            author(30_000, 30_000),
            &tx(vec![SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                from_page_id: ids.page_a,
                to_page_id: ids.page_b,
                parent: None,
                order: "accepted-on-b".into(),
            }]),
        )
        .unwrap();
    let moved_to_b = ready(&archive, &moved_to_b);
    assert!(matches!(
        seed.stage_ready(moved_to_b.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));

    let mut move_author = ids.engine();
    let mut delete_author = ids.engine();
    for engine in [&mut move_author, &mut delete_author] {
        engine.stage_ready(genesis_ready.clone());
        engine.stage_ready(moved_to_b.clone());
    }
    let moved_to_c = move_author
        .prepare_bootstrap_transaction(
            author(31_000 + move_peer as u128, move_peer),
            &tx(vec![SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                from_page_id: ids.page_b,
                to_page_id: ids.page_c,
                parent: None,
                order: "raced-on-c".into(),
            }]),
        )
        .unwrap();
    let deleted_from_b = delete_author
        .prepare_bootstrap_transaction(
            author(32_000 + delete_peer as u128, delete_peer),
            &tx(vec![SemanticOperation::DeleteSubtree {
                root_block_id: ids.block_a,
                page_id: ids.page_b,
            }]),
        )
        .unwrap();
    let moved_to_c = ready(&archive, &moved_to_c);
    let deleted_from_b = ready(&archive, &deleted_from_b);

    let apply = |first: ValidatedBatch, second: ValidatedBatch| {
        let mut engine = ids.engine();
        engine.stage_ready(genesis_ready.clone());
        engine.stage_ready(moved_to_b.clone());
        assert!(!matches!(
            engine.stage_ready(first).disposition,
            BatchDisposition::Rejected { .. }
        ));
        assert!(!matches!(
            engine.stage_ready(second).disposition,
            BatchDisposition::Rejected { .. }
        ));
        engine
    };
    let move_then_delete = apply(moved_to_c.clone(), deleted_from_b.clone());
    let delete_then_move = apply(deleted_from_b, moved_to_c);
    assert_eq!(
        move_then_delete.canonical_snapshot().unwrap(),
        delete_then_move.canonical_snapshot().unwrap()
    );
    assert!(move_then_delete
        .materialize_page(ids.page_a)
        .unwrap()
        .blocks
        .is_empty());
    assert!(move_then_delete
        .materialize_page(ids.page_b)
        .unwrap()
        .blocks
        .is_empty());
    let page_c = move_then_delete.materialize_page(ids.page_c).unwrap();
    let moved_block = page_c
        .blocks
        .iter()
        .find(|block| block.block_id == ids.block_a);
    if let Some(block) = moved_block {
        assert_eq!(block.home_document_id, ids.home_a);
        assert_eq!(block.content, "home A content");
    }
    moved_block.is_some()
}

#[test]
fn moved_away_block_races_move_from_b_to_c_with_delete_from_b_both_orders_and_winners() {
    let low_move = moved_away_move_delete_result(500, 600);
    let high_move = moved_away_move_delete_result(700, 600);
    assert_ne!(
        low_move, high_move,
        "peer order must cover both the moved membership and tombstone winners"
    );
}

#[test]
fn delete_edit_retains_recoverable_crdt_content_but_hides_membership() {
    let ids = Ids::new();
    let dir = TestDir::new("delete-edit");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let deleted = tx(vec![SemanticOperation::DeleteSubtree {
        root_block_id: ids.block_a,
        page_id: ids.page_a,
    }]);
    let edited = tx(vec![SemanticOperation::EditBlockContent {
        block: BlockLocation {
            block_id: ids.block_a,
            home_document_id: ids.home_a,
        },
        content: "recoverable concurrent content".into(),
    }]);
    let (deleted, edited) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(130, 130),
        deleted,
        author(131, 131),
        edited,
    );
    let engine = apply_pair(ids, &baseline, deleted, edited);
    assert!(engine
        .materialize_page(ids.page_a)
        .unwrap()
        .blocks
        .is_empty());
    assert!(engine
        .canonical_snapshot()
        .unwrap()
        .blocks
        .iter()
        .all(|block| block.block_id != ids.block_a));
    let recovered = engine
        .recover_block_state(ids.home_a, ids.block_a)
        .unwrap()
        .expect("tombstoned home content remains in immutable CRDT history");
    assert_eq!(recovered.owner, BlockOwner::Tombstone);
    assert_eq!(recovered.content, "recoverable concurrent content");
}

#[test]
fn page_rename_delete_and_path_conflicts_are_deterministic() {
    let ids = Ids::new();
    let dir = TestDir::new("page-conflicts");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let renamed = tx(vec![SemanticOperation::EditPagePath {
        page_id: ids.page_a,
        path: path("pages/Renamed.md"),
    }]);
    let deleted = tx(vec![SemanticOperation::DeletePage {
        page_id: ids.page_a,
    }]);
    let (renamed, deleted) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(140, 140),
        renamed,
        author(141, 141),
        deleted,
    );
    let ab = apply_pair(ids, &baseline, renamed.clone(), deleted.clone());
    let ba = apply_pair(ids, &baseline, deleted, renamed);
    assert_eq!(
        ab.canonical_snapshot().unwrap(),
        ba.canonical_snapshot().unwrap()
    );

    let mut author_a = ids.engine();
    let mut author_b = ids.engine();
    author_a.stage_ready(baseline.clone());
    author_b.stage_ready(baseline.clone());
    let conflict_a = author_a
        .prepare_bootstrap_transaction(
            author(142, 142),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_a,
                path: path("pages/Conflict.md"),
            }]),
        )
        .unwrap();
    let conflict_b = author_b
        .prepare_bootstrap_transaction(
            author(143, 143),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_b,
                path: path("pages/Conflict.md"),
            }]),
        )
        .unwrap();
    let ab = apply_pair(
        ids,
        &baseline,
        ready(&archive, &conflict_a),
        ready(&archive, &conflict_b),
    );
    let ba = apply_pair(
        ids,
        &baseline,
        ready(&archive, &conflict_b),
        ready(&archive, &conflict_a),
    );
    assert!(matches!(
        ab.status().workspace(),
        WorkspaceStatus::Blocked(_)
    ));
    assert!(matches!(
        ba.status().workspace(),
        WorkspaceStatus::Blocked(_)
    ));
    assert_eq!(ab.fatal_evidence_handle(), ba.fatal_evidence_handle());
    assert_eq!(ab.portable_path_conflicts(), ba.portable_path_conflicts());
    let conflicts = ab.portable_path_conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].participants().len(), 2);
    assert_eq!(
        conflicts[0]
            .participants()
            .iter()
            .map(|participant| participant.page_id())
            .collect::<Vec<_>>(),
        vec![ids.page_a, ids.page_b]
    );
    assert!(matches!(
        ab.canonical_snapshot(),
        Err(EngineError::WorkspaceBlocked(_))
    ));
    assert!(matches!(
        ab.materialize_page(ids.page_a),
        Err(EngineError::WorkspaceBlocked(_))
    ));
}

#[test]
fn portable_aliases_quarantine_in_both_orders_but_compatibility_only_names_stay_distinct() {
    let aliases = [
        ("pages/Foo.md", "pages/foo.md"),
        ("pages/Café.md", "pages/Cafe\u{301}.md"),
        ("pages/Straße.md", "pages/STRASSE.md"),
        ("pages/Σίσυφος.md", "pages/σίσυφοσ.md"),
        ("pages/Kelvin.md", "pages/kelvin.md"),
    ];
    for (offset, (left_path, right_path)) in aliases.into_iter().enumerate() {
        let ids = Ids::new();
        let dir = TestDir::new(&format!("portable-alias-{offset}"));
        let archive = store(&dir, ids);
        let (_, baseline) = seed_engine(ids, &archive);
        let (left, right) = concurrent_ready(
            ids,
            &archive,
            &baseline,
            author(40_000 + offset as u128 * 2, 40_000 + offset as u64 * 2),
            tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_a,
                path: path(left_path),
            }]),
            author(40_001 + offset as u128 * 2, 40_001 + offset as u64 * 2),
            tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_b,
                path: path(right_path),
            }]),
        );
        let ab = apply_pair(ids, &baseline, left.clone(), right.clone());
        let ba = apply_pair(ids, &baseline, right, left);
        assert!(matches!(
            ab.status().workspace(),
            WorkspaceStatus::Blocked(_)
        ));
        assert_eq!(ab.fatal_evidence_handle(), ba.fatal_evidence_handle());
        assert_eq!(ab.portable_path_conflicts(), ba.portable_path_conflicts());
        let evidence = ab.portable_path_conflicts();
        assert_eq!(evidence.len(), 1);
        assert_eq!(
            evidence[0].key_digest(),
            path(left_path).portable_key().digest()
        );
        assert_eq!(
            evidence[0]
                .participants()
                .iter()
                .map(|participant| participant.exact_path().as_str())
                .collect::<Vec<_>>(),
            vec![left_path, right_path]
        );
    }

    let ids = Ids::new();
    let dir = TestDir::new("portable-compatibility-distinct");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let (left, right) = concurrent_ready(
        ids,
        &archive,
        &baseline,
        author(40_100, 40_100),
        tx(vec![SemanticOperation::EditPagePath {
            page_id: ids.page_a,
            path: path("pages/①.md"),
        }]),
        author(40_101, 40_101),
        tx(vec![SemanticOperation::EditPagePath {
            page_id: ids.page_b,
            path: path("pages/1.md"),
        }]),
    );
    let engine = apply_pair(ids, &baseline, left, right);
    assert!(matches!(
        engine.status().workspace(),
        WorkspaceStatus::Operational
    ));
    assert!(engine.portable_path_conflicts().is_empty());
    let snapshot = engine.canonical_snapshot().unwrap();
    assert!(snapshot.path_conflicts.is_empty());
}

#[test]
fn concurrent_portable_alias_creates_quarantine_with_order_independent_evidence() {
    let ids = Ids::new();
    let dir = TestDir::new("portable-create-create");
    let archive = store(&dir, ids);
    let left = ids
        .engine()
        .prepare_bootstrap_transaction(
            author(40_150, 40_150),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_a,
                home_document_id: ids.home_a,
                path: path("pages/Foo.md"),
            }]),
        )
        .unwrap();
    let right = ids
        .engine()
        .prepare_bootstrap_transaction(
            author(40_151, 40_151),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_b,
                home_document_id: ids.home_b,
                path: path("pages/foo.md"),
            }]),
        )
        .unwrap();
    let left = ready(&archive, &left);
    let right = ready(&archive, &right);
    let apply = |first: ValidatedBatch, second: ValidatedBatch| {
        let mut engine = ids.engine();
        assert!(matches!(
            engine.stage_ready(first).disposition,
            BatchDisposition::Accepted { .. }
        ));
        assert!(matches!(
            engine.stage_ready(second).disposition,
            BatchDisposition::Quarantined
        ));
        engine
    };
    let ab = apply(left.clone(), right.clone());
    let ba = apply(right, left);
    assert_eq!(ab.fatal_evidence_handle(), ba.fatal_evidence_handle());
    assert_eq!(ab.portable_path_conflicts(), ba.portable_path_conflicts());
    assert_eq!(ab.portable_path_conflicts()[0].participants().len(), 2);
}

#[test]
fn durable_terminal_portable_latch_blocks_projection_state_after_restart() {
    let ids = Ids::new();
    let dir = TestDir::new("portable-terminal-restart");
    let archive_path = dir.path().join("archive");
    let graph_path = dir.path().join("graph");
    std::fs::create_dir(&graph_path).unwrap();
    let graph = Graph::open(&graph_path);
    let binding = ProjectionEndpointBinding {
        endpoint_id: ProjectionEndpointId::from_uuid(uuid(40_160)),
        device_id: DeviceId::from_uuid(uuid(40_161)),
        graph_resource_id: graph.canonical_resource_id().unwrap(),
    };
    let receipts = ProjectionReceiptStore::open_for_endpoint(
        &dir.path().join("receipts"),
        ids.workspace,
        binding,
    )
    .unwrap();
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let baseline = ids
        .engine()
        .prepare_bootstrap_transaction(
            author(40_162, 40_162),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_c,
                home_document_id: ids.home_c,
                path: path("pages/Baseline.md"),
            }]),
        )
        .unwrap();
    let baseline = ready(&writer, &baseline);
    let mut left_author = ids.engine();
    let mut right_author = ids.engine();
    left_author.stage_ready(baseline.clone());
    right_author.stage_ready(baseline.clone());
    let left = left_author
        .prepare_bootstrap_transaction(
            author(40_163, 40_163),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_a,
                home_document_id: ids.home_a,
                path: path("pages/Foo.md"),
            }]),
        )
        .unwrap();
    let right = right_author
        .prepare_bootstrap_transaction(
            author(40_164, 40_164),
            &tx(vec![SemanticOperation::CreatePage {
                page_id: ids.page_b,
                home_document_id: ids.home_b,
                path: path("pages/foo.md"),
            }]),
        )
        .unwrap();
    writer.publish_prepared(&left).unwrap();
    writer.publish_prepared(&right).unwrap();

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ShardedHotEngine::with_enrolled_projection(
        reader,
        ids.lineage,
        ids.catalog,
        &graph,
        &receipts,
    );
    assert!(matches!(
        engine
            .stage_archive_batch(baseline.manifest().batch_id())
            .unwrap()
            .disposition(),
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        engine
            .stage_archive_batch(left.manifest().batch_id())
            .unwrap()
            .disposition(),
        BatchDisposition::Accepted { .. }
    ));
    let disposition = engine
        .stage_archive_batch(right.manifest().batch_id())
        .unwrap()
        .disposition();
    assert!(
        matches!(disposition, BatchDisposition::Quarantined),
        "{disposition:?}"
    );
    let handle = engine.fatal_evidence_handle().unwrap();
    let conflicts = engine.portable_path_conflicts();
    drop(engine);

    let recovery_reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let recovery = ShardedHotEngine::with_enrolled_projection(
        recovery_reader,
        ids.lineage,
        ids.catalog,
        &graph,
        &receipts,
    );
    assert_eq!(recovery.fatal_evidence_handle(), Some(handle));
    assert_eq!(recovery.portable_path_conflicts(), conflicts);
    assert!(matches!(
        recovery.status().workspace(),
        WorkspaceStatus::Blocked(found) if *found == handle
    ));
    let recovered_projection = recovery.materialize_page_for_projection(ids.page_a);
    assert!(
        matches!(
            recovered_projection,
            Err(EngineError::WorkspaceBlocked(found)) if found == handle
        ),
        "{recovered_projection:?}"
    );
}

#[test]
fn sequential_duplicates_reject_before_batch_and_atomic_swap_and_causal_reuse_succeed() {
    let ids = Ids::new();
    let dir = TestDir::new("portable-sequential-swap-reuse");
    let archive = store(&dir, ids);
    let (mut engine, _) = seed_engine(ids, &archive);

    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(40_200, 40_200),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_b,
                path: path("pages/a.md"),
            }]),
        ),
        Err(EngineError::InvalidTransaction(_))
    ));

    let swap = engine
        .prepare_bootstrap_transaction(
            author(40_201, 40_201),
            &tx(vec![
                SemanticOperation::EditPagePath {
                    page_id: ids.page_a,
                    path: path("pages/B.md"),
                },
                SemanticOperation::EditPagePath {
                    page_id: ids.page_b,
                    path: path("pages/A.md"),
                },
            ]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &swap)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_a).unwrap().path.as_str(),
        "pages/B.md"
    );
    assert_eq!(
        engine.materialize_page(ids.page_b).unwrap().path.as_str(),
        "pages/A.md"
    );

    let release = engine
        .prepare_bootstrap_transaction(
            author(40_202, 40_202),
            &tx(vec![SemanticOperation::DeletePage {
                page_id: ids.page_b,
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &release)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let reuse = engine
        .prepare_bootstrap_transaction(
            author(40_203, 40_203),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_c,
                path: path("pages/A.md"),
            }]),
        )
        .unwrap();
    assert!(matches!(
        engine.stage_ready(ready(&archive, &reuse)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        engine.materialize_page(ids.page_c).unwrap().path.as_str(),
        "pages/A.md"
    );
}

#[test]
fn store_backed_portable_index_is_affected_only_and_missing_root_fails_closed() {
    let ids = Ids::new();
    let dir = TestDir::new("portable-index-auth");
    let archive_path = dir.path().join("archive");
    let writer = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let bootstrap = genesis(ids, &ids.engine());
    writer.publish_prepared(&bootstrap).unwrap();

    let reader = ObjectStore::open(&archive_path, ids.workspace).unwrap();
    let mut engine = ShardedHotEngine::with_archive_store(reader, ids.lineage, ids.catalog);
    assert!(matches!(
        engine
            .stage_archive_batch(bootstrap.manifest().batch_id())
            .unwrap()
            .disposition(),
        BatchDisposition::Accepted { .. }
    ));
    let initial = engine.instrumentation();
    let rename = engine
        .prepare_bootstrap_transaction(
            author(40_300, 40_300),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_a,
                path: path("pages/Only Affected.md"),
            }]),
        )
        .unwrap();
    writer.publish_prepared(&rename).unwrap();
    assert!(matches!(
        engine
            .stage_archive_batch(rename.manifest().batch_id())
            .unwrap()
            .disposition(),
        BatchDisposition::Accepted { .. }
    ));
    let after = engine.instrumentation();
    assert!(
        after
            .portable_path_index_reads
            .saturating_sub(initial.portable_path_index_reads)
            <= 32,
        "one rename must use bounded old/new portable-key point reads"
    );
    assert_ne!(
        engine.portable_path_index_root(),
        tine_core::oplog::PortablePathIndexRoot::empty()
    );

    let root = engine.portable_path_index_root().digest();
    std::fs::remove_file(
        archive_path
            .join("portable-path-index-v1")
            .join(format!("{root}.patricia-node")),
    )
    .unwrap();
    assert!(matches!(
        engine.prepare_bootstrap_transaction(
            author(40_301, 40_301),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_a,
                path: path("pages/Must Fail Closed.md"),
            }]),
        ),
        Err(EngineError::Archive(_))
    ));
}

#[test]
fn received_reuse_that_omits_the_release_frontier_is_rejected_before_visibility() {
    let ids = Ids::new();
    let dir = TestDir::new("portable-stale-reuse");
    let archive = store(&dir, ids);
    let (mut author_engine, baseline) = seed_engine(ids, &archive);
    let release = author_engine
        .prepare_bootstrap_transaction(
            author(40_400, 40_400),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_a,
                path: path("pages/Released.md"),
            }]),
        )
        .unwrap();
    let release_ready = ready(&archive, &release);
    assert!(matches!(
        author_engine.stage_ready(release_ready.clone()).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let safe_reuse = author_engine
        .prepare_bootstrap_transaction(
            author(40_401, 40_401),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_b,
                path: path("pages/A.md"),
            }]),
        )
        .unwrap();
    let stale = rebuild_with_compact_witness(
        &safe_reuse,
        release.manifest().dependency_frontier().clone(),
    );
    let stale_ready = ready(&archive, &stale);

    let mut receiver = ids.engine();
    assert!(matches!(
        receiver.stage_ready(baseline).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(matches!(
        receiver.stage_ready(release_ready).disposition,
        BatchDisposition::Accepted { .. }
    ));
    let before_root = receiver.portable_path_index_root();
    assert!(matches!(
        receiver.stage_ready(stale_ready).disposition,
        BatchDisposition::Rejected { .. }
    ));
    assert_eq!(receiver.portable_path_index_root(), before_root);
    assert!(matches!(
        receiver.status().workspace(),
        WorkspaceStatus::Operational
    ));
    assert_eq!(
        receiver.materialize_page(ids.page_b).unwrap().path.as_str(),
        "pages/B.md"
    );
}

#[test]
fn causal_batch_waits_then_validates_at_declared_frontier_not_delivery_current() {
    let ids = Ids::new();
    let dir = TestDir::new("causal-wait");
    let archive = store(&dir, ids);
    let (mut author_engine, baseline) = seed_engine(ids, &archive);
    let moved = author_engine
        .prepare_bootstrap_transaction(
            author(150, 150),
            &tx(vec![SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                from_page_id: ids.page_a,
                to_page_id: ids.page_b,
                parent: None,
                order: "m".into(),
            }]),
        )
        .unwrap();
    let moved_ready = ready(&archive, &moved);
    author_engine.stage_ready(moved_ready.clone());
    let dependent = author_engine
        .prepare_bootstrap_transaction(
            author(151, 150),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "dependent edit".into(),
            }]),
        )
        .unwrap();
    let dependent_ready = ready(&archive, &dependent);

    let mut concurrent_author = ids.engine();
    concurrent_author.stage_ready(baseline.clone());
    let concurrent = concurrent_author
        .prepare_bootstrap_transaction(
            author(152, 152),
            &tx(vec![SemanticOperation::EditPagePath {
                page_id: ids.page_c,
                path: path("pages/Concurrent.md"),
            }]),
        )
        .unwrap();
    let concurrent_ready = ready(&archive, &concurrent);

    let mut receiver = ids.engine();
    receiver.stage_ready(baseline);
    receiver.stage_ready(concurrent_ready);
    assert!(matches!(
        receiver.stage_ready(dependent_ready).disposition,
        BatchDisposition::IncompleteStaged { .. }
    ));
    let outcome = receiver.stage_ready(moved_ready);
    assert!(matches!(
        outcome.disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert_eq!(
        outcome
            .newly_accepted()
            .iter()
            .map(|accepted| accepted.batch_id)
            .collect::<Vec<_>>(),
        vec![BatchId::from_uuid(uuid(150)), BatchId::from_uuid(uuid(151)),]
    );
    assert_eq!(
        receiver.materialize_page(ids.page_b).unwrap().blocks[0].content,
        "dependent edit"
    );
}

#[test]
fn duplicate_of_still_staged_batch_truthfully_repeats_missing_dependencies() {
    let ids = Ids::new();
    let dir = TestDir::new("duplicate-staged");
    let archive = store(&dir, ids);
    let (mut author_engine, baseline) = seed_engine(ids, &archive);
    let dependency = author_engine
        .prepare_bootstrap_transaction(
            author(170, 170),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "dependency".into(),
            }]),
        )
        .unwrap();
    let dependency_ready = ready(&archive, &dependency);
    author_engine.stage_ready(dependency_ready);
    let dependent = author_engine
        .prepare_bootstrap_transaction(
            author(171, 171),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "dependent".into(),
            }]),
        )
        .unwrap();
    let dependent_ready = ready(&archive, &dependent);

    let mut receiver = ids.engine();
    receiver.stage_ready(baseline);
    let expected_missing = vec![BatchId::from_uuid(uuid(170))];
    for _ in 0..2 {
        assert!(matches!(
            receiver.stage_ready(dependent_ready.clone()).disposition,
            BatchDisposition::IncompleteStaged {
                missing_objects: 0,
                ref missing_dependencies,
                ..
            } if *missing_dependencies == expected_missing
        ));
    }
}

#[test]
fn crdt_update_requires_exact_declared_base_but_not_delivery_current() {
    let ids = Ids::new();
    let dir = TestDir::new("exact-causal-base");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);

    let mut advanced = ids.engine();
    advanced.stage_ready(baseline.clone());
    let intermediate = advanced
        .prepare_bootstrap_transaction(
            author(180, 180),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "intermediate".into(),
            }]),
        )
        .unwrap();
    let intermediate_ready = ready(&archive, &intermediate);
    advanced.stage_ready(intermediate_ready.clone());
    let based_on_advanced = advanced
        .prepare_bootstrap_transaction(
            author(181, 181),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "advanced update".into(),
            }]),
        )
        .unwrap();

    let mut baseline_author = ids.engine();
    baseline_author.stage_ready(baseline.clone());
    let baseline_template = baseline_author
        .prepare_bootstrap_transaction(
            author(181, 181),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "baseline template".into(),
            }]),
        )
        .unwrap();
    let under_declared = rebuild_with_compact_witness(
        &based_on_advanced,
        baseline_template.manifest().dependency_frontier().clone(),
    );
    let mut receiver = ids.engine();
    receiver.stage_ready(baseline.clone());
    assert!(matches!(
        receiver
            .stage_ready(ready(&archive, &under_declared))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::CrdtUpdateBaseMismatch(_),
            ..
        }
    ));

    let based_on_baseline = baseline_author
        .prepare_bootstrap_transaction(
            author(182, 182),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "baseline update".into(),
            }]),
        )
        .unwrap();
    let advanced_template = advanced
        .prepare_bootstrap_transaction(
            author(182, 182),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "advanced template".into(),
            }]),
        )
        .unwrap();
    let over_declared = rebuild_with_compact_witness(
        &based_on_baseline,
        advanced_template.manifest().dependency_frontier().clone(),
    );
    let mut receiver = ids.engine();
    receiver.stage_ready(baseline.clone());
    receiver.stage_ready(intermediate_ready.clone());
    assert!(matches!(
        receiver
            .stage_ready(ready(&archive, &over_declared))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::CrdtUpdateBaseMismatch(_),
            ..
        }
    ));

    let concurrent = baseline_author
        .prepare_bootstrap_transaction(
            author(183, 183),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "delivery-current concurrency".into(),
            }]),
        )
        .unwrap();
    let target = baseline_author
        .prepare_bootstrap_transaction(
            author(184, 184),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "target wins".into(),
            }]),
        )
        .unwrap();
    let mut receiver = ids.engine();
    receiver.stage_ready(baseline);
    receiver.stage_ready(ready(&archive, &concurrent));
    assert!(matches!(
        receiver.stage_ready(ready(&archive, &target)).disposition,
        BatchDisposition::Accepted { .. }
    ));
    assert!(!receiver.materialize_page(ids.page_a).unwrap().blocks[0]
        .content
        .is_empty());
}

#[test]
fn compact_frontier_rejects_nonmaximal_heads_and_inexact_peer_counters() {
    let ids = Ids::new();
    let dir = TestDir::new("compact-frontier-exactness");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);

    let mut author_engine = ids.engine();
    author_engine.stage_ready(baseline.clone());
    let intermediate = author_engine
        .prepare_bootstrap_transaction(
            author(185, 185),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "intermediate".into(),
            }]),
        )
        .unwrap();
    let intermediate_ready = ready(&archive, &intermediate);
    author_engine.stage_ready(intermediate_ready.clone());
    let descendant = author_engine
        .prepare_bootstrap_transaction(
            author(186, 186),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "descendant".into(),
            }]),
        )
        .unwrap();
    let exact = &descendant.manifest().dependency_frontier().documents()[0];

    let nonmaximal = DocumentDependencies::new(
        exact.document_id(),
        exact.peer_counters().to_vec(),
        vec![
            baseline.manifest().batch_id(),
            intermediate.manifest().batch_id(),
        ],
    )
    .unwrap();
    let nonmaximal =
        rebuild_with_compact_witness(&descendant, FrontierV2::new(vec![nonmaximal]).unwrap());
    let mut receiver = ids.engine();
    receiver.stage_ready(baseline.clone());
    receiver.stage_ready(intermediate_ready.clone());
    assert!(matches!(
        receiver
            .stage_ready(ready(&archive, &nonmaximal))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::NonMaximalDependencyHead {
                redundant,
                descendant,
            },
        } if redundant == baseline.manifest().batch_id()
            && descendant == intermediate.manifest().batch_id()
    ));

    let mut counters = exact.peer_counters().to_vec();
    let first = counters[0];
    counters[0] = CrdtPeerCounter::new(first.peer_id(), first.max_counter() + 1);
    let inexact = DocumentDependencies::new(
        exact.document_id(),
        counters,
        exact.direct_dependency_heads().to_vec(),
    )
    .unwrap();
    let inexact =
        rebuild_with_compact_witness(&descendant, FrontierV2::new(vec![inexact]).unwrap());
    let inexact_dir = TestDir::new("compact-frontier-inexact-counter");
    let inexact_archive = store(&inexact_dir, ids);
    let mut receiver = ids.engine();
    receiver.stage_ready(baseline);
    receiver.stage_ready(intermediate_ready);
    assert!(matches!(
        receiver
            .stage_ready(ready(&inexact_archive, &inexact))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::FrontierVectorMismatch(document_id),
        } if document_id == ids.home_a
    ));
}

#[test]
fn compact_frontier_rejects_unrelated_maximal_document_head() {
    let ids = Ids::new();
    let dir = TestDir::new("compact-frontier-unrelated-maximal-head");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);

    let mut unrelated_author = ids.engine();
    unrelated_author.stage_ready(baseline.clone());
    let unrelated = unrelated_author
        .prepare_bootstrap_transaction(
            author(187, 187),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_c,
                    home_document_id: ids.home_c,
                },
                content: "unrelated accepted head".into(),
            }]),
        )
        .unwrap();
    let unrelated_ready = ready(&archive, &unrelated);

    let mut target_author = ids.engine();
    target_author.stage_ready(baseline.clone());
    let target = target_author
        .prepare_bootstrap_transaction(
            author(188, 188),
            &tx(vec![SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: ids.block_a,
                    home_document_id: ids.home_a,
                },
                content: "target edit".into(),
            }]),
        )
        .unwrap();
    let exact = &target.manifest().dependency_frontier().documents()[0];
    assert_eq!(exact.document_id(), ids.home_a);
    let smuggled = DocumentDependencies::new(
        ids.home_a,
        exact.peer_counters().to_vec(),
        vec![unrelated.manifest().batch_id()],
    )
    .unwrap();
    let smuggled = rebuild_with_compact_witness(&target, FrontierV2::new(vec![smuggled]).unwrap());

    let mut receiver = ids.engine();
    receiver.stage_ready(baseline);
    receiver.stage_ready(unrelated_ready);
    assert!(matches!(
        receiver
            .stage_ready(ready(&archive, &smuggled))
            .disposition,
        BatchDisposition::Rejected {
            error: EngineError::InexactDocumentDependencyHeads { document_id },
        } if document_id == ids.home_a
    ));
}

#[test]
fn randomized_replica_delivery_orders_converge_and_duplicates_are_noops() {
    let ids = Ids::new();
    let dir = TestDir::new("random-orders");
    let archive = store(&dir, ids);
    let (_, baseline) = seed_engine(ids, &archive);
    let operations = [
        tx(vec![SemanticOperation::EditPagePath {
            page_id: ids.page_b,
            path: path("pages/B2.md"),
        }]),
        tx(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: ids.block_a,
                home_document_id: ids.home_a,
            },
            content: "randomized concurrent edit".into(),
        }]),
        tx(vec![SemanticOperation::MoveSubtree {
            root: BlockLocation {
                block_id: ids.block_a,
                home_document_id: ids.home_a,
            },
            from_page_id: ids.page_a,
            to_page_id: ids.page_c,
            parent: None,
            order: "z".into(),
        }]),
    ];
    let mut batches = Vec::new();
    for (index, operation) in operations.into_iter().enumerate() {
        let mut author_engine = ids.engine();
        author_engine.stage_ready(baseline.clone());
        let prepared = author_engine
            .prepare_bootstrap_transaction(
                author(160 + index as u128, 160 + index as u64),
                &operation,
            )
            .unwrap();
        batches.push(ready(&archive, &prepared));
    }
    let mut expected = None;
    for seed in 1_u64..=64 {
        let mut order = [0_usize, 1, 2];
        let mut state = seed;
        for index in (1..order.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            order.swap(index, state as usize % (index + 1));
        }
        let mut replica = ids.engine();
        replica.stage_ready(baseline.clone());
        for index in order {
            replica.stage_ready(batches[index].clone());
        }
        assert!(matches!(
            replica.stage_ready(batches[0].clone()).disposition,
            BatchDisposition::DuplicateAccepted { .. }
        ));
        let snapshot = replica.canonical_snapshot().unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&snapshot, expected, "seed {seed}");
        } else {
            expected = Some(snapshot);
        }
    }
}

#[test]
fn scenario_encoding_scheduler_and_production_engine_simulation_are_deterministic() {
    let ids = Ids::new();
    let devices = vec![
        ScenarioDevice {
            name: "alpha".into(),
            device_id: DeviceId::from_uuid(uuid(500)),
            crdt_peer_id: CrdtPeerId::from_u64(500),
        },
        ScenarioDevice {
            name: "beta".into(),
            device_id: DeviceId::from_uuid(uuid(501)),
            crdt_peer_id: CrdtPeerId::from_u64(501),
        },
    ];
    assert_ne!(devices[0].device_id, devices[1].device_id);
    assert_ne!(devices[0].crdt_peer_id, devices[1].crdt_peer_id);
    let create = tx(vec![SemanticOperation::CreatePage {
        page_id: ids.page_a,
        home_document_id: ids.home_a,
        path: path("pages/A.md"),
    }]);
    let scenario = Scenario::new(
        "serializable-foundation",
        0x1234_5678,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: BatchId::from_uuid(uuid(600)),
                session_id: SessionId::from_uuid(uuid(601)),
                transaction: create,
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: BatchId::from_uuid(uuid(600)),
            },
            ScenarioAction::DuplicateDelivery {
                device: 1,
                batch_id: BatchId::from_uuid(uuid(600)),
            },
            ScenarioAction::AssertConverged {
                devices: vec![0, 1],
            },
        ],
    )
    .unwrap();
    assert_eq!(
        Scenario::decode(&scenario.encode().unwrap()).unwrap(),
        scenario
    );
    assert_eq!(scenario.permutation(32), scenario.permutation(32));
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let snapshots = simulator.snapshots().unwrap();
    assert_eq!(snapshots[0], snapshots[1]);
    assert!(matches!(
        simulator
            .outcomes()
            .last()
            .map(|outcome| &outcome.disposition),
        Some(BatchDisposition::DuplicateAccepted { .. })
    ));
}

#[test]
fn simulator_assert_converged_checks_terminal_history_not_only_evidence() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(46));
    let devices = vec![
        ScenarioDevice {
            name: "alpha".into(),
            device_id: DeviceId::from_uuid(uuid(810)),
            crdt_peer_id: CrdtPeerId::from_u64(810),
        },
        ScenarioDevice {
            name: "beta".into(),
            device_id: DeviceId::from_uuid(uuid(811)),
            crdt_peer_id: CrdtPeerId::from_u64(811),
        },
    ];
    let genesis_id = BatchId::from_uuid(uuid(810));
    let claim_a_id = BatchId::from_uuid(uuid(811));
    let claim_b_id = BatchId::from_uuid(uuid(812));
    let scenario = Scenario::new(
        "terminal-blocked-is-comparable",
        46,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: genesis_id,
                session_id: SessionId::from_uuid(uuid(820)),
                transaction: tx(vec![
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
                ]),
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: genesis_id,
            },
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: claim_a_id,
                session_id: SessionId::from_uuid(uuid(821)),
                transaction: tx(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: ids.home_a,
                    },
                    page_id: ids.page_a,
                    parent: None,
                    order: "a".into(),
                    content: "A".into(),
                }]),
            },
            ScenarioAction::LocalTransaction {
                device: 1,
                batch_id: claim_b_id,
                session_id: SessionId::from_uuid(uuid(822)),
                transaction: tx(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: ids.home_b,
                    },
                    page_id: ids.page_b,
                    parent: None,
                    order: "b".into(),
                    content: "B".into(),
                }]),
            },
            ScenarioAction::Deliver {
                device: 0,
                batch_id: claim_b_id,
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: claim_a_id,
            },
            ScenarioAction::AssertConverged {
                devices: vec![0, 1],
            },
        ],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    assert!(matches!(
        simulator.run(),
        Err(tine_core::oplog::ScenarioError::Diverged { action_index: 6 })
    ));
}

#[test]
fn simulator_offered_oracle_compares_opposite_pre_latch_histories() {
    let ids = Ids::new();
    let block_id = tine_core::oplog::BlockId::from_uuid(uuid(53));
    let devices = vec![
        ScenarioDevice {
            name: "left".into(),
            device_id: DeviceId::from_uuid(uuid(830)),
            crdt_peer_id: CrdtPeerId::from_u64(830),
        },
        ScenarioDevice {
            name: "right".into(),
            device_id: DeviceId::from_uuid(uuid(831)),
            crdt_peer_id: CrdtPeerId::from_u64(831),
        },
        ScenarioDevice {
            name: "conflict-author".into(),
            device_id: DeviceId::from_uuid(uuid(832)),
            crdt_peer_id: CrdtPeerId::from_u64(832),
        },
    ];
    let genesis_id = BatchId::from_uuid(uuid(830));
    let claim_a_id = BatchId::from_uuid(uuid(831));
    let claim_b_id = BatchId::from_uuid(uuid(832));
    let scenario = Scenario::new(
        "terminal-offered-frontier-oracle",
        53,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: genesis_id,
                session_id: SessionId::from_uuid(uuid(840)),
                transaction: tx(vec![
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
                ]),
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: genesis_id,
            },
            ScenarioAction::Deliver {
                device: 2,
                batch_id: genesis_id,
            },
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: claim_a_id,
                session_id: SessionId::from_uuid(uuid(841)),
                transaction: tx(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: ids.home_a,
                    },
                    page_id: ids.page_a,
                    parent: None,
                    order: "a".into(),
                    content: "A".into(),
                }]),
            },
            ScenarioAction::LocalTransaction {
                device: 2,
                batch_id: claim_b_id,
                session_id: SessionId::from_uuid(uuid(842)),
                transaction: tx(vec![SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id: ids.home_b,
                    },
                    page_id: ids.page_b,
                    parent: None,
                    order: "b".into(),
                    content: "B".into(),
                }]),
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: claim_b_id,
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: claim_a_id,
            },
            ScenarioAction::Deliver {
                device: 0,
                batch_id: claim_b_id,
            },
        ],
    )
    .unwrap();
    let mut simulator = DeterministicSimulator::new(scenario).unwrap();
    simulator.run().unwrap();
    let statuses = simulator.statuses();
    assert_eq!(
        statuses[0].accepted_batch_ids().unwrap(),
        vec![genesis_id, claim_a_id]
    );
    assert_eq!(
        statuses[1].accepted_batch_ids().unwrap(),
        vec![genesis_id, claim_b_id]
    );
    assert_eq!(
        statuses[0].offered_batch_ids().unwrap(),
        statuses[1].offered_batch_ids().unwrap()
    );
    assert_eq!(
        simulator.states().unwrap()[0],
        simulator.states().unwrap()[1]
    );
}

#[test]
fn scenario_reducer_removes_irrelevant_authors_and_orphan_deliveries() {
    let ids = Ids::new();
    let devices = vec![
        ScenarioDevice {
            name: "alpha".into(),
            device_id: DeviceId::from_uuid(uuid(710)),
            crdt_peer_id: CrdtPeerId::from_u64(710),
        },
        ScenarioDevice {
            name: "beta".into(),
            device_id: DeviceId::from_uuid(uuid(711)),
            crdt_peer_id: CrdtPeerId::from_u64(711),
        },
    ];
    let scenario = Scenario::new(
        "automatic-reducer",
        0xfeed_beef,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        devices,
        vec![
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: BatchId::from_uuid(uuid(720)),
                session_id: SessionId::from_uuid(uuid(721)),
                transaction: tx(vec![SemanticOperation::CreatePage {
                    page_id: ids.page_c,
                    home_document_id: ids.home_c,
                    path: path("pages/Irrelevant.md"),
                }]),
            },
            ScenarioAction::Deliver {
                device: 1,
                batch_id: BatchId::from_uuid(uuid(720)),
            },
            ScenarioAction::LocalTransaction {
                device: 0,
                batch_id: BatchId::from_uuid(uuid(722)),
                session_id: SessionId::from_uuid(uuid(723)),
                transaction: tx(vec![SemanticOperation::CreatePage {
                    page_id: ids.page_a,
                    home_document_id: ids.home_a,
                    path: path("pages/Failure.md"),
                }]),
            },
            ScenarioAction::AssertConverged {
                devices: vec![0, 1],
            },
        ],
    )
    .unwrap();

    let minimized = scenario.minimize_failure().unwrap();
    assert_eq!(minimized.scenario.seed, scenario.seed);
    assert_eq!(minimized.scenario.actions.len(), 2);
    assert_eq!(minimized.capsule.original_seed, scenario.seed);
    assert_eq!(minimized.capsule.failure, FailureIdentity::Diverged);
    assert_eq!(
        FailureCapsule::decode(&minimized.capsule.encode().unwrap()).unwrap(),
        minimized.capsule
    );
    let roundtripped = Scenario::decode(&minimized.scenario.encode().unwrap()).unwrap();
    for replay in [roundtripped.clone(), roundtripped] {
        let mut simulator = DeterministicSimulator::new(replay).unwrap();
        let failure = simulator.run().unwrap_err();
        assert_eq!(failure.failure_identity(), Some(FailureIdentity::Diverged));
    }
}

#[test]
fn scenario_devices_require_independent_device_and_peer_identities() {
    let ids = Ids::new();
    let duplicate_device = ScenarioDevice {
        name: "first".into(),
        device_id: DeviceId::from_uuid(uuid(730)),
        crdt_peer_id: CrdtPeerId::from_u64(730),
    };
    let mut duplicate_peer = duplicate_device.clone();
    duplicate_peer.name = "second".into();
    assert!(Scenario::new(
        "duplicate-identities",
        1,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        vec![duplicate_device.clone(), duplicate_peer],
        Vec::new(),
    )
    .is_err());
    let mut duplicate_device_only = duplicate_device.clone();
    duplicate_device_only.name = "third".into();
    duplicate_device_only.crdt_peer_id = CrdtPeerId::from_u64(731);
    assert!(Scenario::new(
        "duplicate-device",
        1,
        ids.workspace,
        ids.lineage,
        ids.catalog,
        vec![duplicate_device, duplicate_device_only],
        Vec::new(),
    )
    .is_err());
}

#[test]
fn semantic_encoding_is_canonical_and_bounded() {
    let effect = SemanticEffect::new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let bytes = effect.encode().unwrap();
    assert_eq!(SemanticEffect::decode(&bytes).unwrap(), effect);
    let mut noncanonical = bytes;
    noncanonical.push(b' ');
    assert!(SemanticEffect::decode(&noncanonical).is_err());
    assert_ne!(ContentDigest::of(b"a"), ContentDigest::of(b"b"));
    let _ = CrdtPeerCounter::new(CrdtPeerId::from_u64(1), 0);
}
