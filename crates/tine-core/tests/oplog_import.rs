use std::fs;
use std::path::{Path, PathBuf};

use tine_core::oplog::{
    classify_conflict_copy, execute_manifested_projection_work, inventory_affected,
    inventory_initial_shadow, plan_affected_import, write_projection_exact, AuthorBatch, BatchId,
    BatchOrigin, BlobDescription, BlockId, BlockLocation, BlockMatchBasis, CrdtPeerId,
    CurrentPageAtPath, DeviceId, DocumentId, ImportBlockReason, ImportPlan, ImportPlanStatus,
    LineageDigest, LogseqIdentityMutation, LogseqUuid, ManagedPath, ManagedTextKind, ObjectStore,
    OperationTransaction, PageId, PageMatchBasis, ProjectionEndpointBinding, ProjectionEndpointId,
    ProjectionIntent, ProjectionReceiptStore, RawObservation, RejectedRawIdReason,
    SemanticOperation, SessionId, ShardedHotEngine, WorkspaceId,
};
use tine_core::Graph;
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tine-oplog-import-{label}-{}", Uuid::new_v4()));
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

fn workspace() -> WorkspaceId {
    WorkspaceId::from_uuid(uuid(1))
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[derive(Clone)]
struct BlockSpec {
    content: String,
    parent: Option<usize>,
    order: String,
    logseq_uuid: Option<LogseqUuid>,
}

impl BlockSpec {
    fn root(content: impl Into<String>, order: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            parent: None,
            order: order.into(),
            logseq_uuid: None,
        }
    }

    fn child(content: impl Into<String>, parent: usize, order: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            parent: Some(parent),
            order: order.into(),
            logseq_uuid: None,
        }
    }

    fn with_uuid(mut self, logseq_uuid: LogseqUuid) -> Self {
        self.content = format!("{}\nid:: {logseq_uuid}", self.content);
        self.logseq_uuid = Some(logseq_uuid);
        self
    }
}

struct PageSpec {
    path: String,
    blocks: Vec<BlockSpec>,
}

struct PageAuthority {
    path: String,
    page_id: PageId,
    home_document_id: DocumentId,
    block_ids: Vec<BlockId>,
    intent: ProjectionIntent,
    projected: Vec<u8>,
}

struct AuthorityFixture {
    _dir: TestDir,
    graph_root: PathBuf,
    archive_path: PathBuf,
    graph: Graph,
    receipts: ProjectionReceiptStore,
    engine: ShardedHotEngine,
    pages: Vec<PageAuthority>,
}

impl AuthorityFixture {
    fn new(label: &str, pages: Vec<PageSpec>) -> Self {
        let dir = TestDir::new(label);
        let graph_root = dir.path().join("graph");
        fs::create_dir_all(graph_root.join("pages")).unwrap();
        fs::create_dir_all(graph_root.join("journals")).unwrap();
        let graph = Graph::open(&graph_root);
        let endpoint = ProjectionEndpointBinding::enroll_graph(
            &graph,
            ProjectionEndpointId::from_uuid(uuid(100)),
            DeviceId::from_uuid(uuid(101)),
        )
        .unwrap();
        let receipts = ProjectionReceiptStore::open_for_endpoint(
            &dir.path().join("receipts"),
            workspace(),
            endpoint,
        )
        .unwrap();
        let lineage = LineageDigest::of(b"oplog-import-authority");
        let catalog = DocumentId::from_uuid(uuid(200));
        let author = ShardedHotEngine::new(workspace(), lineage, catalog);
        let mut operations = Vec::new();
        let mut authority = Vec::new();
        for (page_index, page) in pages.iter().enumerate() {
            let seed = 1_000 + page_index as u128 * 1_000;
            let page_id = PageId::from_uuid(uuid(seed));
            let home_document_id = DocumentId::from_uuid(uuid(seed + 1));
            let kind = match page.path.split_once('/') {
                Some(("pages", rest)) if !rest.is_empty() => ManagedTextKind::Page,
                Some(("journals", rest)) if !rest.is_empty() => ManagedTextKind::Journal,
                _ => panic!("import fixture path is outside the guarded default layout"),
            };
            let block_ids = (0..page.blocks.len())
                .map(|index| BlockId::from_uuid(uuid(seed + 10 + index as u128)))
                .collect::<Vec<_>>();
            operations.push(SemanticOperation::CreatePage {
                page_id,
                home_document_id,
                path: ManagedPath::parse(page.path.clone()).unwrap(),
                kind,
            });
            for (index, block) in page.blocks.iter().enumerate() {
                operations.push(SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: block_ids[index],
                        home_document_id,
                    },
                    page_id,
                    parent: block.parent.map(|parent| block_ids[parent]),
                    order: block.order.clone(),
                    content: block.content.clone(),
                });
            }
            for (index, block) in page.blocks.iter().enumerate() {
                if let Some(logseq_uuid) = block.logseq_uuid {
                    operations.push(SemanticOperation::MutateBlockLogseqIdentity {
                        block: BlockLocation {
                            block_id: block_ids[index],
                            home_document_id,
                        },
                        mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
                    });
                }
            }
            authority.push((page.path.clone(), page_id, home_document_id, block_ids));
        }
        let transaction = OperationTransaction::new(operations).unwrap();
        let batch_id = BatchId::from_uuid(uuid(300));
        let prepared = author
            .prepare_bootstrap_transaction(
                AuthorBatch {
                    batch_id,
                    author_device_id: DeviceId::from_uuid(uuid(301)),
                    author_session_id: SessionId::from_uuid(uuid(302)),
                    crdt_peer_id: CrdtPeerId::from_u64(303),
                },
                &transaction,
            )
            .unwrap();
        let archive_path = dir.path().join("archive");
        let writer = ObjectStore::open(&archive_path, workspace()).unwrap();
        writer.publish_prepared(&prepared).unwrap();
        drop(writer);
        let reader = ObjectStore::open(&archive_path, workspace()).unwrap();
        let mut engine =
            ShardedHotEngine::with_enrolled_projection(reader, lineage, catalog, &graph, &receipts);
        engine.stage_archive_batch(batch_id).unwrap();

        let mut page_authorities = Vec::new();
        for (path, page_id, home_document_id, block_ids) in authority {
            let projected =
                write_projection_exact(&graph, &receipts, &engine, page_id, None).unwrap();
            page_authorities.push(PageAuthority {
                path,
                page_id,
                home_document_id,
                block_ids,
                intent: projected.plan.intent().clone(),
                projected: projected.plan.target().to_vec(),
            });
        }
        Self {
            _dir: dir,
            graph_root,
            archive_path,
            graph,
            receipts,
            engine,
            pages: page_authorities,
        }
    }

    fn one_page(label: &str, path: &str, blocks: Vec<BlockSpec>) -> Self {
        Self::new(
            label,
            vec![PageSpec {
                path: path.into(),
                blocks,
            }],
        )
    }

    fn plan(&self, paths: &[&str]) -> ImportPlan {
        plan_affected_import(&self.graph, &self.receipts, &self.engine, paths)
    }

    fn overwrite(&self, path: &str, bytes: &[u8]) {
        write(&self.graph_root, path, bytes);
    }

    fn append_local_tail(&mut self, page: usize, block: usize, content: &str, seed: u128) {
        let page = &self.pages[page];
        let transaction = OperationTransaction::new(vec![SemanticOperation::EditBlockContent {
            block: BlockLocation {
                block_id: page.block_ids[block],
                home_document_id: page.home_document_id,
            },
            content: content.into(),
        }])
        .unwrap();
        let batch_id = BatchId::from_uuid(uuid(seed));
        let prepared = self
            .engine
            .prepare_bootstrap_transaction(
                AuthorBatch {
                    batch_id,
                    author_device_id: DeviceId::from_uuid(uuid(seed + 1)),
                    author_session_id: SessionId::from_uuid(uuid(seed + 2)),
                    crdt_peer_id: CrdtPeerId::from_u64(seed as u64 + 3),
                },
                &transaction,
            )
            .unwrap();
        let writer = ObjectStore::open(&self.archive_path, workspace()).unwrap();
        writer.publish_prepared(&prepared).unwrap();
        self.engine.stage_archive_batch(batch_id).unwrap();
    }

    fn delete_and_project(&mut self, page: usize, seed: u128) {
        let authority = &self.pages[page];
        let endpoint = self.receipts.endpoint_binding().unwrap();
        let draft = self
            .engine
            .draft_author_transaction(
                AuthorBatch {
                    batch_id: BatchId::from_uuid(uuid(seed)),
                    author_device_id: endpoint.device_id(),
                    author_session_id: SessionId::from_uuid(uuid(seed + 1)),
                    crdt_peer_id: CrdtPeerId::from_u64(seed as u64 + 2),
                },
                BatchOrigin::LocalMutation,
                &OperationTransaction::new(vec![SemanticOperation::DeletePage {
                    page_id: authority.page_id,
                }])
                .unwrap(),
            )
            .unwrap();
        let input = self
            .receipts
            .capture_projection_input(
                &self.graph,
                endpoint,
                ManagedPath::parse(&authority.path).unwrap(),
                Some(&authority.intent),
            )
            .unwrap();
        let prepared = self
            .engine
            .finalize_author_transaction(draft, endpoint, vec![input])
            .unwrap();
        let writer = ObjectStore::open(&self.archive_path, workspace()).unwrap();
        writer.publish_prepared(&prepared).unwrap();
        self.engine
            .stage_archive_batch(BatchId::from_uuid(uuid(seed)))
            .unwrap();
        let work = self
            .engine
            .projection_work_index()
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        execute_manifested_projection_work(&self.graph, &self.receipts, &mut self.engine, &work)
            .unwrap();
    }
}

fn blocked_reasons(plan: &ImportPlan) -> Vec<ImportBlockReason> {
    assert_eq!(plan.status(), ImportPlanStatus::Blocked, "{plan:?}");
    plan.blocks().iter().map(|block| block.reason).collect()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[test]
fn exact_inventory_preserves_lf_crlf_twins_nested_paths_and_explicit_absence() {
    let dir = TestDir::new("inventory");
    let nested_path = ManagedPath::parse("pages/topic/subtopic/archive/a.md").unwrap();
    let nested_bytes = b"- nested\n";
    fs::create_dir_all(dir.path().join("pages/topic/subtopic/archive")).unwrap();
    fs::create_dir_all(dir.path().join("journals")).unwrap();
    write(dir.path(), "pages/lf.md", b"- same\n");
    write(dir.path(), "pages/lf.org", b"* same\r\n");
    write(dir.path(), nested_path.as_str(), nested_bytes);
    let graph = Graph::open(dir.path());

    let inventory = inventory_affected(
        &graph,
        &[
            "pages/lf.md",
            "pages/lf.org",
            nested_path.as_str(),
            "pages/missing.md",
        ],
    )
    .unwrap();
    assert_eq!(
        inventory.present("pages/lf.md").unwrap().bytes(),
        b"- same\n"
    );
    assert_eq!(
        inventory.present("pages/lf.org").unwrap().bytes(),
        b"* same\r\n"
    );
    assert_ne!(
        inventory.present("pages/lf.md").unwrap().description(),
        inventory.present("pages/lf.org").unwrap().description()
    );
    assert!(matches!(
        inventory.entries().get(&nested_path),
        Some(RawObservation::Present(bytes)) if bytes.bytes() == nested_bytes
    ));
    assert!(matches!(
        inventory
            .entries()
            .get(&ManagedPath::parse("pages/missing.md").unwrap()),
        Some(RawObservation::Absent)
    ));

    let initial = inventory_initial_shadow(&graph).unwrap();
    assert_eq!(initial.entries().len(), 3);
    assert!(matches!(
        initial.entries().get(&nested_path),
        Some(RawObservation::Present(bytes)) if bytes.bytes() == nested_bytes
    ));
}

#[test]
fn genuine_combined_authority_is_required_and_caller_evidence_is_not_an_input() {
    let fixture = AuthorityFixture::one_page(
        "sealed-authority",
        "pages/page.md",
        vec![BlockSpec::root("base", "a")],
    );
    assert_eq!(
        fixture.plan(&["pages/page.md"]).status(),
        ImportPlanStatus::Noop
    );

    let unbound_dir = TestDir::new("unbound-receipts");
    let unbound = ProjectionReceiptStore::open(unbound_dir.path(), workspace()).unwrap();
    let blocked = plan_affected_import(
        &fixture.graph,
        &unbound,
        &fixture.engine,
        &["pages/page.md"],
    );
    assert!(blocked_reasons(&blocked).contains(&ImportBlockReason::AuthorityUnavailable));
}

#[test]
fn copied_endpoint_tuple_in_a_second_receipt_root_cannot_authenticate() {
    let fixture = AuthorityFixture::one_page(
        "second-receipt-root",
        "pages/page.md",
        vec![BlockSpec::root("base", "a")],
    );
    let second_dir = TestDir::new("second-receipt-root-forged");
    let second = ProjectionReceiptStore::open_for_endpoint(
        second_dir.path(),
        workspace(),
        fixture.receipts.endpoint_binding().unwrap(),
    )
    .unwrap();
    assert_ne!(second.store_id(), fixture.receipts.store_id());
    second
        .publish_intent(&fixture.pages[0].intent, None)
        .unwrap();
    let forged_engine = ShardedHotEngine::with_enrolled_projection(
        ObjectStore::open(&fixture.archive_path, workspace()).unwrap(),
        LineageDigest::of(b"oplog-import-authority"),
        DocumentId::from_uuid(uuid(200)),
        &fixture.graph,
        &second,
    );
    assert!(
        forged_engine.accepted_frontier_root().is_err(),
        "durable history/work claims must retain R1's receipt-store identity"
    );

    let blocked =
        plan_affected_import(&fixture.graph, &second, &fixture.engine, &["pages/page.md"]);
    assert!(blocked_reasons(&blocked).contains(&ImportBlockReason::AuthorityUnavailable));
}

#[test]
fn complete_catalog_finds_existing_paths_and_missing_completion_blocks_read_only_import() {
    let fixture = AuthorityFixture::one_page(
        "catalog-replay",
        "pages/page.md",
        vec![BlockSpec::root("base", "a")],
    );
    let intent_id = fixture.pages[0].intent.id().unwrap();
    let completion = fixture
        .receipts
        .root_path()
        .join("completions")
        .join(format!("{}.completion", hex(intent_id.as_bytes())));
    fs::remove_file(&completion).unwrap();

    let plan = fixture.plan(&["pages/page.md"]);
    assert_eq!(plan.status(), ImportPlanStatus::Blocked);
    assert!(blocked_reasons(&plan).contains(&ImportBlockReason::MissingBase));
    assert!(
        !completion.exists(),
        "read-only import recovery must not publish completion"
    );
}

#[test]
fn corrupt_completion_and_conflicting_local_tail_fail_closed() {
    let mut fixture = AuthorityFixture::one_page(
        "corrupt-and-stale",
        "pages/page.md",
        vec![BlockSpec::root("base", "a")],
    );
    let intent_id = fixture.pages[0].intent.id().unwrap();
    let completion = fixture
        .receipts
        .root_path()
        .join("completions")
        .join(format!("{}.completion", hex(intent_id.as_bytes())));
    fs::write(&completion, b"forged downstream bytes").unwrap();
    let corrupt = fixture.plan(&["pages/page.md"]);
    assert!(blocked_reasons(&corrupt).contains(&ImportBlockReason::CorruptBase));

    fs::remove_file(&completion).unwrap();
    fixture.append_local_tail(0, 0, "local tail", 400);
    let stale = fixture.plan(&["pages/page.md"]);
    assert!(blocked_reasons(&stale).contains(&ImportBlockReason::ConflictingLocalTail));
}

#[test]
fn two_id_properties_in_one_block_never_anchor_and_each_occurrence_is_reported() {
    let anchor = LogseqUuid::from_uuid(uuid(500));
    let fixture = AuthorityFixture::one_page(
        "two-ids",
        "pages/page.md",
        vec![BlockSpec::root("base", "a").with_uuid(anchor)],
    );
    fixture.overwrite(
        "pages/page.md",
        format!("- edited\n  id:: {anchor}\n  id:: {anchor}\n").as_bytes(),
    );

    let plan = fixture.plan(&["pages/page.md"]);
    let matches = plan
        .matches()
        .expect("conservative reconciliation remains plannable");
    assert!(matches.blocks().is_empty());
    assert_eq!(matches.rejected_raw_ids().len(), 2);
    assert!(matches
        .rejected_raw_ids()
        .iter()
        .all(|id| id.reason() == RejectedRawIdReason::Duplicate));
}

#[test]
fn invalid_raw_id_bytes_are_preserved_reported_and_never_authorize_identity() {
    let fixture = AuthorityFixture::one_page(
        "invalid-id",
        "pages/page.md",
        vec![BlockSpec::root("base", "a")],
    );
    let external = b"- base\n  id:: definitely-not-a-uuid\n";
    fixture.overwrite("pages/page.md", external);

    let plan = fixture.plan(&["pages/page.md"]);
    let matches = plan
        .matches()
        .expect("invalid identity degrades conservatively");
    assert!(matches.blocks().is_empty());
    assert_eq!(matches.rejected_raw_ids().len(), 1);
    assert_eq!(
        matches.rejected_raw_ids()[0].reason(),
        RejectedRawIdReason::InvalidSyntax
    );
    assert_eq!(
        fs::read(fixture.graph_root.join("pages/page.md")).unwrap(),
        external
    );
}

#[test]
fn unique_uuid_anchor_precedes_structure() {
    let anchor = LogseqUuid::from_uuid(uuid(510));
    let fixture = AuthorityFixture::one_page(
        "uuid-anchor",
        "pages/page.md",
        vec![
            BlockSpec::root("first", "a").with_uuid(anchor),
            BlockSpec::root("second", "b"),
        ],
    );
    fixture.overwrite(
        "pages/page.md",
        format!("- second\n- moved\n  id:: {anchor}\n").as_bytes(),
    );
    let plan = fixture.plan(&["pages/page.md"]);
    let anchored = plan
        .matches()
        .unwrap()
        .blocks()
        .iter()
        .find(|matched| matched.basis() == BlockMatchBasis::UniqueLogseqUuid)
        .unwrap();
    assert_eq!(anchored.block_id(), fixture.pages[0].block_ids[0]);
    assert_eq!(anchored.locator().components(), &[1]);
}

#[test]
fn one_idless_in_place_edit_retains_block_id_by_ordered_tree_alignment() {
    let fixture = AuthorityFixture::one_page(
        "idless-edit",
        "pages/page.md",
        vec![BlockSpec::root("before", "a")],
    );
    fixture.overwrite("pages/page.md", b"- after\n");

    let plan = fixture.plan(&["pages/page.md"]);
    let matched = &plan.matches().unwrap().blocks()[0];
    assert_eq!(matched.block_id(), fixture.pages[0].block_ids[0]);
    assert_eq!(
        matched.basis(),
        BlockMatchBasis::ReceiptOrderedTreeAlignment
    );
}

#[test]
fn copy_inserted_before_exact_anchor_does_not_steal_trailing_identity() {
    let fixture = AuthorityFixture::one_page(
        "ordered-copy",
        "pages/page.md",
        vec![BlockSpec::root("X", "a"), BlockSpec::root("A", "b")],
    );
    fixture.overwrite("pages/page.md", b"- A\n- X\n- A\n");

    let matches = fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    let by_locator = matches
        .blocks()
        .iter()
        .map(|matched| (matched.locator().components().to_vec(), matched.block_id()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(!by_locator.contains_key(&vec![0]));
    assert_eq!(by_locator[&vec![1]], fixture.pages[0].block_ids[0]);
    assert!(!by_locator.contains_key(&vec![2]));
}

#[test]
fn nested_edits_retain_structure_and_unequal_duplicate_gaps_never_guess() {
    let nested = AuthorityFixture::one_page(
        "nested",
        "pages/page.md",
        vec![
            BlockSpec::root("root", "a"),
            BlockSpec::child("child", 0, "a"),
        ],
    );
    nested.overwrite("pages/page.md", b"- root edited\n\t- child edited\n");
    let nested_matches = nested
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert_eq!(nested_matches.blocks().len(), 2);
    assert_eq!(
        nested_matches.blocks()[0].block_id(),
        nested.pages[0].block_ids[0]
    );
    assert_eq!(
        nested_matches.blocks()[1].block_id(),
        nested.pages[0].block_ids[1]
    );

    let duplicates = AuthorityFixture::one_page(
        "duplicate-gap",
        "pages/page.md",
        vec![BlockSpec::root("same", "a"), BlockSpec::root("same", "b")],
    );
    duplicates.overwrite("pages/page.md", b"- same\n");
    assert!(
        duplicates
            .plan(&["pages/page.md"])
            .matches()
            .unwrap()
            .blocks()
            .is_empty(),
        "unequal duplicate gap must conservatively lose continuity"
    );
}

#[test]
fn equal_structures_separated_by_anchor_remain_globally_ambiguous() {
    let anchor = LogseqUuid::from_uuid(uuid(515));
    let fixture = AuthorityFixture::one_page(
        "anchor-separated-duplicates",
        "pages/page.md",
        vec![
            BlockSpec::root("same", "a"),
            BlockSpec::root("anchor", "b").with_uuid(anchor),
            BlockSpec::root("same", "c"),
        ],
    );
    fixture.overwrite(
        "pages/page.md",
        format!("- same\n- anchor\n  id:: {anchor}\n").as_bytes(),
    );
    let matches = fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert_eq!(matches.blocks().len(), 1);
    assert_eq!(
        matches.blocks()[0].basis(),
        BlockMatchBasis::UniqueLogseqUuid
    );

    fixture.overwrite(
        "pages/page.md",
        format!("- same\n- anchor\n  id:: {anchor}\n- same\n").as_bytes(),
    );
    let both_survive = fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert_eq!(both_survive.blocks().len(), 1);
    assert_eq!(
        both_survive.blocks()[0].basis(),
        BlockMatchBasis::UniqueLogseqUuid
    );
}

#[test]
fn nested_reparented_duplicate_class_does_not_gain_identity_from_an_anchor_gap() {
    let anchor = LogseqUuid::from_uuid(uuid(516));
    let fixture = AuthorityFixture::one_page(
        "nested-anchor-ambiguity",
        "pages/page.md",
        vec![
            BlockSpec::root("parent", "a"),
            BlockSpec::child("same child", 0, "a"),
            BlockSpec::root("anchor", "b").with_uuid(anchor),
            BlockSpec::root("parent", "c"),
            BlockSpec::child("same child", 3, "a"),
        ],
    );
    fixture.overwrite(
        "pages/page.md",
        format!("- parent\n\t- same child\n- anchor\n  id:: {anchor}\n").as_bytes(),
    );
    let matches = fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert_eq!(matches.blocks().len(), 1);
    assert_eq!(
        matches.blocks()[0].basis(),
        BlockMatchBasis::UniqueLogseqUuid
    );
}

#[test]
fn global_exact_matching_retains_unambiguous_cross_page_move_but_not_copy() {
    let moved = AuthorityFixture::new(
        "cross-page-exact-move",
        vec![
            PageSpec {
                path: "pages/a.md".into(),
                blocks: vec![BlockSpec::root("moved", "a")],
            },
            PageSpec {
                path: "pages/b.md".into(),
                blocks: vec![BlockSpec::root("resident", "a")],
            },
        ],
    );
    moved.overwrite("pages/a.md", b"");
    moved.overwrite("pages/b.md", b"- resident\n- moved\n");
    let matches = moved
        .plan(&["pages/a.md", "pages/b.md"])
        .matches()
        .unwrap()
        .to_owned();
    let retained = matches
        .blocks()
        .iter()
        .find(|matched| matched.block_id() == moved.pages[0].block_ids[0])
        .unwrap();
    assert_eq!(retained.path().as_str(), "pages/b.md");
    assert_eq!(retained.locator().components(), &[1]);
    assert_eq!(retained.basis(), BlockMatchBasis::ReceiptStructuralExact);

    let copied = AuthorityFixture::new(
        "cross-page-exact-copy",
        vec![
            PageSpec {
                path: "pages/a.md".into(),
                blocks: vec![BlockSpec::root("moved", "a")],
            },
            PageSpec {
                path: "pages/b.md".into(),
                blocks: vec![BlockSpec::root("resident", "a")],
            },
        ],
    );
    copied.overwrite("pages/b.md", b"- resident\n- moved\n");
    let copy_matches = copied
        .plan(&["pages/a.md", "pages/b.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert!(!copy_matches
        .blocks()
        .iter()
        .any(|matched| matched.block_id() == copied.pages[0].block_ids[0]));
}

#[test]
fn unrelated_equal_length_replacements_never_retain_block_identity() {
    let fixture = AuthorityFixture::one_page(
        "equal-replacement-gap",
        "pages/page.md",
        vec![BlockSpec::root("A", "a"), BlockSpec::root("B", "b")],
    );
    fixture.overwrite("pages/page.md", b"- X\n- Y\n");
    assert!(fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .blocks()
        .is_empty());
}

#[test]
fn crossed_uuid_anchors_block_all_unanchored_sibling_guessing() {
    let p = LogseqUuid::from_uuid(uuid(521));
    let q = LogseqUuid::from_uuid(uuid(522));
    let fixture = AuthorityFixture::one_page(
        "crossed-anchors",
        "pages/page.md",
        vec![
            BlockSpec::root("A", "a"),
            BlockSpec::root("P", "b").with_uuid(p),
            BlockSpec::root("B", "c"),
            BlockSpec::root("Q", "d").with_uuid(q),
        ],
    );
    fixture.overwrite(
        "pages/page.md",
        format!("- Q\n  id:: {q}\n- X\n- P\n  id:: {p}\n- Y\n").as_bytes(),
    );
    let matches = fixture
        .plan(&["pages/page.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert_eq!(matches.blocks().len(), 2);
    assert!(matches
        .blocks()
        .iter()
        .all(|matched| matched.basis() == BlockMatchBasis::UniqueLogseqUuid));
    let ids = matches
        .blocks()
        .iter()
        .map(|matched| matched.block_id())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(ids.contains(&fixture.pages[0].block_ids[1]));
    assert!(ids.contains(&fixture.pages[0].block_ids[3]));
}

#[test]
fn exact_rename_and_copy_before_delete_preserve_conservative_page_identity() {
    let rename = AuthorityFixture::one_page(
        "rename",
        "pages/old.md",
        vec![BlockSpec::root("retained", "a")],
    );
    let projected = rename.pages[0].projected.clone();
    fs::rename(
        rename.graph_root.join("pages/old.md"),
        rename.graph_root.join("pages/new.md"),
    )
    .unwrap();
    let plan = rename.plan(&["pages/old.md", "pages/new.md"]);
    let page = &plan.matches().unwrap().pages()[0];
    assert_eq!(page.page_id(), rename.pages[0].page_id);
    assert_eq!(page.basis(), PageMatchBasis::ReceiptBackedExactRename);
    assert_eq!(page.path().as_str(), "pages/new.md");

    let copy = AuthorityFixture::one_page(
        "copy-before-delete",
        "pages/old.md",
        vec![BlockSpec::root("retained", "a")],
    );
    write(&copy.graph_root, "pages/copy.md", &projected);
    let matches = copy
        .plan(&["pages/old.md", "pages/copy.md"])
        .matches()
        .unwrap()
        .to_owned();
    assert!(matches
        .pages()
        .iter()
        .any(|page| page.path().as_str() == "pages/old.md"));
    assert!(!matches
        .pages()
        .iter()
        .any(|page| page.path().as_str() == "pages/copy.md"));
}

#[test]
fn completed_release_allows_exact_recreation_as_new_but_blocks_portable_neighbor() {
    let mut fixture = AuthorityFixture::one_page(
        "released-recreation",
        "pages/released.md",
        vec![BlockSpec::root("historical", "a")],
    );
    let old_page_id = fixture.pages[0].page_id;
    fixture.delete_and_project(0, 58_000);
    assert!(!fixture.graph_root.join("pages/released.md").exists());
    fixture.overwrite("pages/released.md", b"- external recreation\n");

    let recreated = fixture.plan(&["pages/released.md"]);
    assert_eq!(recreated.status(), ImportPlanStatus::Reconcile);
    assert!(recreated
        .matches()
        .unwrap()
        .pages()
        .iter()
        .all(|matched| matched.page_id() != old_page_id));
    assert!(recreated.import_id().is_some());

    let collision = fixture.plan(&["pages/Released.md"]);
    assert!(blocked_reasons(&collision).contains(&ImportBlockReason::PortablePathCollision));
}

#[test]
fn affected_scope_does_not_enumerate_unrelated_entries_and_nondefault_layout_refuses() {
    let dir = TestDir::new("scope");
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("journals")).unwrap();
    write(dir.path(), "pages/affected.md", b"- affected\n");
    fs::create_dir(dir.path().join("pages/unrelated.md")).unwrap();
    let graph = Graph::open(dir.path());
    let inventory = inventory_affected(&graph, &["pages/affected.md"]).unwrap();
    assert_eq!(inventory.entries().len(), 1);

    let custom = TestDir::new("custom-layout");
    fs::create_dir_all(custom.path().join("logseq")).unwrap();
    fs::write(
        custom.path().join("logseq/config.edn"),
        "{:pages-directory \"notes\" :journals-directory \"diary\"}\n",
    )
    .unwrap();
    fs::create_dir_all(custom.path().join("notes")).unwrap();
    let graph = Graph::open(custom.path());
    assert!(inventory_affected(&graph, &["pages/page.md"]).is_err());
    assert!(inventory_initial_shadow(&graph).is_err());
}

#[test]
fn portable_case_and_unicode_collisions_fail_closed_but_exact_owner_remains_valid() {
    let fixture = AuthorityFixture::one_page(
        "portable-collision",
        "pages/Foo.md",
        vec![BlockSpec::root("owned", "a")],
    );
    assert_eq!(
        fixture.plan(&["pages/Foo.md"]).status(),
        ImportPlanStatus::Noop
    );

    let case = fixture.plan(&["pages/foo.md"]);
    assert!(blocked_reasons(&case).contains(&ImportBlockReason::PortablePathCollision));
    let lookup = fixture
        .engine
        .current_page_at_path(&ManagedPath::parse("pages/foo.md").unwrap())
        .unwrap();
    let CurrentPageAtPath::PortableCollision(occupied) = lookup else {
        panic!("engine folded a portable collision into unowned");
    };
    assert_eq!(occupied.exact_path().as_str(), "pages/Foo.md");
    assert_eq!(occupied.page_id(), fixture.pages[0].page_id);

    let requested_case = fixture.plan(&["pages/New.md", "pages/new.md"]);
    assert!(blocked_reasons(&requested_case).contains(&ImportBlockReason::PortablePathCollision));

    let composed = "pages/Caf\u{e9}.md";
    let decomposed = "pages/Cafe\u{301}.md";
    let unicode = fixture.plan(&[composed, decomposed]);
    assert!(blocked_reasons(&unicode).contains(&ImportBlockReason::PortablePathCollision));
}

#[test]
fn deep_input_fails_before_parse_and_inventory_instrumentation_counts_physical_work() {
    let deep = AuthorityFixture::one_page(
        "deep-budget",
        "pages/deep.md",
        vec![BlockSpec::root("base", "a")],
    );
    let mut external = "\t".repeat(tine_core::oplog::MAX_IMPORT_DEPTH);
    external.push_str("- too deep\n");
    deep.overwrite("pages/deep.md", external.as_bytes());
    let blocked = deep.plan(&["pages/deep.md"]);
    assert!(blocked_reasons(&blocked).contains(&ImportBlockReason::ResourceLimit));

    let fixture = AuthorityFixture::new(
        "inventory-peak",
        vec![
            PageSpec {
                path: "pages/a.md".into(),
                blocks: vec![BlockSpec::root("short", "a")],
            },
            PageSpec {
                path: "pages/b.md".into(),
                blocks: vec![BlockSpec::root("a somewhat longer block", "a")],
            },
        ],
    );
    let plan = fixture.plan(&["pages/a.md", "pages/b.md"]);
    assert_eq!(plan.status(), ImportPlanStatus::Noop);
    let total = fixture
        .pages
        .iter()
        .map(|page| page.projected.len() as u64)
        .sum::<u64>();
    let max = fixture
        .pages
        .iter()
        .map(|page| page.projected.len() as u64)
        .max()
        .unwrap();
    let work = plan.instrumentation();
    assert_eq!(work.bytes_read, total * 4);
    assert!(work.bytes_hashed >= total * 4);
    assert_eq!(work.peak_owned_raw_bytes, total + max + 16 * 1024);
}

#[test]
fn conflict_copy_classification_is_read_only_and_non_authoritative_for_deletion() {
    let dir = TestDir::new("conflict");
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::create_dir_all(dir.path().join("journals")).unwrap();
    let path = "pages/a.sync-conflict-20260724-120000-AAAAAAA.md";
    write(dir.path(), path, b"- generated\n");
    let graph = Graph::open(dir.path());
    let inventory = inventory_affected(&graph, &[path]).unwrap();
    let class = classify_conflict_copy(
        ManagedPath::parse(path).unwrap(),
        inventory.present(path).unwrap(),
        BlobDescription::of(b"- generated\n"),
        Some(BlobDescription::of(b"- external\n")),
    )
    .unwrap();
    assert_eq!(format!("{class:?}"), "GeneratedExact");
    assert!(dir.path().join(path).exists());
}

#[test]
fn large_noop_and_many_error_scopes_obey_complete_numeric_work_ceilings() {
    const PAGE_COUNT: usize = 128;
    let pages = (0..PAGE_COUNT)
        .map(|index| PageSpec {
            path: format!("pages/p{index:04}.md"),
            blocks: vec![BlockSpec::root(format!("block {index}"), "a")],
        })
        .collect();
    let fixture = AuthorityFixture::new("large-noop", pages);
    let owned_paths = fixture
        .pages
        .iter()
        .map(|page| page.path.clone())
        .collect::<Vec<_>>();
    let paths = owned_paths.iter().map(String::as_str).collect::<Vec<_>>();
    let noop = fixture.plan(&paths);
    assert_eq!(noop.status(), ImportPlanStatus::Noop);
    let work = noop.instrumentation();
    eprintln!(
        "large-noop instrumentation: {work:?}, total={}",
        work.recorded_work_units()
    );
    assert!(
        work.recorded_work_units() <= PAGE_COUNT * 4096,
        "complete work ceiling exceeded: {work:?}"
    );
    assert!(work.bytes_read <= PAGE_COUNT as u64 * 64);
    assert!(work.catalog_bytes_hashed <= PAGE_COUNT as u64 * 4096);
    assert!(work.locator_components_materialized <= PAGE_COUNT * 2);
    assert!(work.structural_key_comparisons <= PAGE_COUNT * 16);

    let error_names = (0..PAGE_COUNT)
        .map(|index| format!("pages/error-{index:04}.md"))
        .collect::<Vec<_>>();
    for path in &error_names {
        write(&fixture.graph_root, path, &[0xff, b'\n']);
    }
    let error_paths = error_names.iter().map(String::as_str).collect::<Vec<_>>();
    let errors = fixture.plan(&error_paths);
    assert_eq!(errors.status(), ImportPlanStatus::Blocked);
    assert_eq!(errors.blocks().len(), 1);
    assert_eq!(errors.inventory().unwrap().entries().len(), PAGE_COUNT);
    eprintln!(
        "many-error instrumentation: {:?}, total={}",
        errors.instrumentation(),
        errors.instrumentation().recorded_work_units()
    );
    assert!(
        errors.instrumentation().recorded_work_units() <= PAGE_COUNT * 4096,
        "many-error work ceiling exceeded: {:?}",
        errors.instrumentation()
    );
}
