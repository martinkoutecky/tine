use std::fs;
use std::path::{Path, PathBuf};

use tine_core::crdt::{
    BlockId, BlockSnapshot, CrdtGraph, ManagedSyncStoreState, PageId, PageSnapshot,
};
use tine_core::model::{Graph, PageKind};
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-managed-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("pages")).unwrap();
        fs::create_dir_all(path.join("journals")).unwrap();
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

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn update_chunks(root: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), output);
            } else if entry.path().extension().and_then(|value| value.to_str()) == Some("chunk")
                && entry.path().to_string_lossy().contains("/sessions/")
            {
                output.push(entry.path());
            }
        }
    }
    let mut output = Vec::new();
    visit(&root.join(".tine-sync/v1"), &mut output);
    output
}

fn deliver_update_chunk(source: &Path, target: &Path, chunk_id: &str) {
    let chunk = update_chunks(source)
        .into_iter()
        .find(|path| path.file_stem().and_then(|value| value.to_str()) == Some(chunk_id))
        .expect("published update chunk exists");
    let incoming = target.join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&chunk, incoming.join(chunk.file_name().unwrap())).unwrap();
}

fn write_genesis_claim(root: &Path, device: Uuid) {
    let store = root.join(".tine-sync/v1");
    fs::create_dir_all(store.join("genesis")).unwrap();
    fs::write(
        store.join("genesis.claim"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "workspace_id": Uuid::new_v4(),
            "device_id": device,
            "session_id": Uuid::new_v4(),
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn claimed_activation_resumes_only_for_its_device() {
    let same = TestDir::new("resume-same-device");
    fs::write(same.path().join("pages/Page.md"), "- resume safely\n").unwrap();
    let device = Uuid::new_v4();
    write_genesis_claim(same.path(), device);
    let graph = Graph::open(same.path());
    assert_eq!(
        graph.managed_sync_store_state().unwrap(),
        ManagedSyncStoreState::Claimed
    );
    graph.enable_managed_sync(device, Uuid::new_v4()).unwrap();
    assert_eq!(
        graph.managed_sync_store_state().unwrap(),
        ManagedSyncStoreState::Initialized
    );

    let foreign = TestDir::new("resume-foreign-device");
    let path = foreign.path().join("pages/Page.md");
    fs::write(&path, "- must remain untouched\n").unwrap();
    write_genesis_claim(foreign.path(), Uuid::new_v4());
    let graph = Graph::open(foreign.path());
    assert!(graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .is_err());
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "- must remain untouched\n"
    );
    assert_eq!(
        graph.managed_sync_store_state().unwrap(),
        ManagedSyncStoreState::Claimed
    );
}

#[test]
fn empty_unclaimed_residue_has_no_device_owner_even_after_id_migration() {
    let dir = TestDir::new("unclaimed-residue");
    let durable = Uuid::new_v4();
    fs::write(
        dir.path().join("pages/Page.md"),
        format!("- already migrated\n  id:: {durable}\n"),
    )
    .unwrap();
    fs::create_dir_all(dir.path().join(".tine-sync/v1/genesis")).unwrap();
    assert_eq!(
        Graph::open(dir.path()).managed_sync_store_state().unwrap(),
        ManagedSyncStoreState::Unclaimed
    );

    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    assert_eq!(
        durable_id(
            &graph
                .load_named("Page", PageKind::Page)
                .unwrap()
                .unwrap()
                .blocks[0]
                .raw
        ),
        durable.to_string()
    );
}

#[test]
fn delivered_operation_projects_to_identical_markdown_on_a_second_graph() {
    let left = TestDir::new("replica-left");
    let right = TestDir::new("replica-right");
    let page_path = left.path().join("pages/Page.md");
    fs::write(&page_path, "- original\n").unwrap();

    let left_graph = Graph::open(left.path());
    let enabled = left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    assert_eq!(enabled.migration.pages_changed, 1);
    assert_eq!(enabled.migration.blocks_changed, 1);
    assert!(fs::read_to_string(&page_path).unwrap().contains("id:: "));

    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    assert!(right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap());

    let mut page = left_graph
        .load_named("Page", PageKind::Page)
        .unwrap()
        .unwrap();
    page.blocks[0].raw = page.blocks[0].raw.replacen("original", "edited on left", 1);
    left_graph.save_page(&page, page.rev.as_deref()).unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1);

    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();
    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert_eq!(pull.changes.len(), 1);
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Page.md")).unwrap(),
        fs::read_to_string(&page_path).unwrap()
    );
}

#[test]
fn failed_multi_page_projection_replays_the_entire_import_on_retry() {
    let left = TestDir::new("projection-retry-left");
    let right = TestDir::new("projection-retry-right");
    fs::write(left.path().join("pages/A.md"), "- old a\n").unwrap();
    fs::write(left.path().join("pages/B.md"), "- old b\n").unwrap();
    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();

    let desired_a = fs::read_to_string(left.path().join("pages/A.md"))
        .unwrap()
        .replacen("old a", "new a", 1);
    let desired_b = fs::read_to_string(left.path().join("pages/B.md"))
        .unwrap()
        .replacen("old b", "new b", 1);
    left_graph
        .commit_managed_restore(&[
            ("pages/A.md".into(), desired_a.clone()),
            ("pages/B.md".into(), desired_b.clone()),
        ])
        .unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1);
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();

    let blocked = right.path().join("pages/B.md");
    fs::remove_file(&blocked).unwrap();
    fs::create_dir(&blocked).unwrap();
    assert!(right_graph.pull_managed_sync().is_err());

    fs::remove_dir(&blocked).unwrap();
    let retry = right_graph.pull_managed_sync().unwrap();
    assert_eq!(retry.imported_chunks, 0, "the chunk was already imported");
    assert_eq!(
        fs::read_to_string(right.path().join("pages/A.md")).unwrap(),
        desired_a
    );
    assert_eq!(fs::read_to_string(blocked).unwrap(), desired_b);
}

#[test]
fn watcher_imports_external_markdown_and_persists_ids_for_new_blocks() {
    let dir = TestDir::new("external");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before_updates = update_chunks(dir.path()).len();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{}- added externally\n", original)).unwrap();

    let changed = graph
        .sync_file(&path)
        .expect("external projection is reported");
    assert_eq!(changed.name, "Page");
    let projected = fs::read_to_string(&path).unwrap();
    assert!(projected.contains("added externally"));
    assert_eq!(projected.matches("id:: ").count(), 2);
    assert_eq!(update_chunks(dir.path()).len(), before_updates + 1);
}

fn durable_id(raw: &str) -> &str {
    raw.lines()
        .find_map(|line| line.strip_prefix("id:: "))
        .expect("managed block has a durable id")
}

#[test]
fn external_copy_with_retained_ids_preserves_both_pages() {
    let dir = TestDir::new("external-copy");
    let source_path = dir.path().join("pages/Source.md");
    let copy_path = dir.path().join("pages/Copy.md");
    fs::write(&source_path, "- copied externally\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let source_before = graph.load_named("Source", PageKind::Page).unwrap().unwrap();
    let source_durable = durable_id(&source_before.blocks[0].raw).to_string();

    fs::copy(&source_path, &copy_path).unwrap();
    graph.sync_file_checked(&copy_path).unwrap();
    graph.project_all_managed_sync().unwrap();

    let source = graph.load_named("Source", PageKind::Page).unwrap().unwrap();
    let copy = graph.load_named("Copy", PageKind::Page).unwrap().unwrap();
    assert_eq!(durable_id(&source.blocks[0].raw), source_durable);
    assert_ne!(durable_id(&copy.blocks[0].raw), source_durable);
    assert_ne!(source.blocks[0].id, copy.blocks[0].id);
    assert_eq!(graph.managed_sync_status().unwrap().page_count, 2);
    assert!(source_path.exists());
    assert!(copy_path.exists());
}

fn copy_first_delete_later_promotes_the_survivor(restart_before_delete: bool) {
    let dir = TestDir::new(if restart_before_delete {
        "copy-delete-restart"
    } else {
        "copy-delete"
    });
    let source_path = dir.path().join("pages/Source.md");
    let destination_path = dir.path().join("pages/Renamed.md");
    fs::write(&source_path, "- provider move\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let original_id = durable_id(
        &graph
            .load_named("Source", PageKind::Page)
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
    )
    .to_string();

    fs::copy(&source_path, &destination_path).unwrap();
    graph.sync_file_checked(&destination_path).unwrap();
    let provisional_id = durable_id(
        &graph
            .load_named("Renamed", PageKind::Page)
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
    )
    .to_string();
    assert_ne!(provisional_id, original_id, "a live copy stays distinct");

    let graph = if restart_before_delete {
        drop(graph);
        let reopened = Graph::open(dir.path());
        reopened
            .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
            .unwrap();
        reopened
    } else {
        graph
    };
    fs::remove_file(&source_path).unwrap();
    graph.sync_deleted_file(&source_path).unwrap();

    assert!(!source_path.exists());
    let survivor = graph
        .load_named("Renamed", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(durable_id(&survivor.blocks[0].raw), original_id);
    assert_eq!(graph.managed_sync_status().unwrap().page_count, 1);
}

#[test]
fn provider_copy_then_delete_is_one_identity_preserving_move() {
    copy_first_delete_later_promotes_the_survivor(false);
}

#[test]
fn provider_copy_then_restart_then_delete_still_preserves_identity() {
    copy_first_delete_later_promotes_the_survivor(true);
}

fn delivered_copy_promotion_keeps_live_destination(source_sorts_first: bool) {
    let ordering = if source_sorts_first {
        "source-first"
    } else {
        "copy-first"
    };
    let left = TestDir::new(&format!("promotion-left-{ordering}"));
    let right = TestDir::new(&format!("promotion-right-{ordering}"));
    let source_page_id = PageId::from_uuid(Uuid::from_u128(if source_sorts_first { 1 } else { 2 }));
    let copy_page_id = PageId::from_uuid(Uuid::from_u128(if source_sorts_first { 2 } else { 1 }));
    let source_block_id = BlockId::from_uuid(Uuid::from_u128(11));
    let copy_block_id = BlockId::from_uuid(Uuid::from_u128(12));
    let source = PageSnapshot {
        id: source_page_id,
        path: "pages/Source.md".into(),
        name: "Source".into(),
        kind: "page".into(),
        format: "md".into(),
        pre_block: None,
        blocks: vec![BlockSnapshot {
            id: source_block_id,
            parent: None,
            order: 0,
            raw: format!("provider move\nid:: {source_block_id}"),
        }],
    };
    let copy = PageSnapshot {
        id: copy_page_id,
        path: "pages/Renamed.md".into(),
        name: "Renamed".into(),
        kind: "page".into(),
        format: "md".into(),
        pre_block: None,
        blocks: vec![BlockSnapshot {
            id: copy_block_id,
            parent: None,
            order: 0,
            raw: format!("provider move\nid:: {copy_block_id}"),
        }],
    };
    let mut promoted = source.clone();
    promoted.path = copy.path.clone();
    promoted.name = copy.name.clone();

    let mut producer =
        CrdtGraph::initialize(left.path(), Uuid::new_v4(), Uuid::new_v4(), vec![source]).unwrap();
    copy_tree(left.path(), right.path());
    let receiver = Graph::open(right.path());
    receiver
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    receiver.project_all_managed_sync().unwrap();

    let copied = producer.commit_page(copy).unwrap();
    deliver_update_chunk(left.path(), right.path(), &copied.chunk_id);
    assert_eq!(receiver.pull_managed_sync().unwrap().imported_chunks, 1);
    let provisional = receiver
        .load_named("Renamed", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(
        durable_id(&provisional.blocks[0].raw),
        copy_block_id.to_string()
    );

    let promotion = producer
        .promote_copy(source_page_id, copy_page_id, promoted)
        .unwrap();
    deliver_update_chunk(left.path(), right.path(), &promotion.chunk_id);
    assert_eq!(receiver.pull_managed_sync().unwrap().imported_chunks, 1);

    let destination = right.path().join("pages/Renamed.md");
    assert!(destination.exists(), "destination vanished for {ordering}");
    assert!(!right.path().join("pages/Source.md").exists());
    let survivor = receiver
        .load_named("Renamed", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(
        durable_id(&survivor.blocks[0].raw),
        source_block_id.to_string()
    );
    assert_eq!(receiver.managed_sync_status().unwrap().page_count, 1);

    let retry = receiver.pull_managed_sync().unwrap();
    assert_eq!(retry.imported_chunks, 0);
    assert!(
        destination.exists(),
        "retry removed destination for {ordering}"
    );
    drop(receiver);

    let reopened = Graph::open(right.path());
    reopened
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    reopened.project_all_managed_sync().unwrap();
    assert!(
        destination.exists(),
        "reopen removed destination for {ordering}"
    );
    drop(reopened);

    let replica = CrdtGraph::open(right.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();
    let survivor = replica
        .materialize_page(source_page_id)
        .unwrap()
        .expect("original page identity survives");
    assert_eq!(survivor.path, "pages/Renamed.md");
    assert_eq!(survivor.blocks[0].id, source_block_id);
    assert!(replica.materialize_page(copy_page_id).unwrap().is_none());
}

#[test]
fn delivered_copy_promotion_cannot_delete_another_live_page_projection() {
    delivered_copy_promotion_keeps_live_destination(true);
    delivered_copy_promotion_keeps_live_destination(false);
}

#[test]
fn stale_projection_is_rejected_before_an_operation_chunk_is_written() {
    let dir = TestDir::new("stale");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let mut stale = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    stale.blocks[0].raw = stale.blocks[0].raw.replacen("original", "stale edit", 1);
    fs::write(&path, "- external replacement\n").unwrap();
    let before = update_chunks(dir.path()).len();
    let error = graph.save_page(&stale, stale.rev.as_deref()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(update_chunks(dir.path()).len(), before);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "- external replacement\n"
    );
}

#[test]
fn delivered_delete_removes_only_the_known_projection_on_a_second_graph() {
    let left = TestDir::new("delete-left");
    let right = TestDir::new("delete-right");
    let page_path = left.path().join("pages/Page.md");
    fs::write(&page_path, "- keep until deleted\n").unwrap();

    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();

    left_graph.delete_page("Page", PageKind::Page).unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1);
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();

    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert!(pull.changes.iter().any(|change| change.removed));
    assert!(!right.path().join("pages/Page.md").exists());
}

#[test]
fn delivered_rename_moves_the_page_and_rewrites_referrers_as_one_operation() {
    let left = TestDir::new("rename-left");
    let right = TestDir::new("rename-right");
    fs::write(left.path().join("pages/Page.md"), "- target\n").unwrap();
    fs::write(
        left.path().join("pages/Referrer.md"),
        "- points to [[Page]]\n",
    )
    .unwrap();

    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    copy_tree(left.path(), right.path());
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();

    left_graph.rename_page("Page", "Renamed").unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), 1, "rename must be one graph transaction");
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&updates[0], incoming.join(updates[0].file_name().unwrap())).unwrap();

    let pull = right_graph.pull_managed_sync().unwrap();
    assert_eq!(pull.imported_chunks, 1);
    assert!(!right.path().join("pages/Page.md").exists());
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Renamed.md")).unwrap(),
        fs::read_to_string(left.path().join("pages/Renamed.md")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Referrer.md")).unwrap(),
        fs::read_to_string(left.path().join("pages/Referrer.md")).unwrap()
    );
    assert!(fs::read_to_string(right.path().join("pages/Referrer.md"))
        .unwrap()
        .contains("[[Renamed]]"));
}

#[test]
fn managed_restore_is_durable_before_projection_and_removes_omitted_pages() {
    let dir = TestDir::new("restore");
    let page_path = dir.path().join("pages/Page.md");
    let omitted_path = dir.path().join("pages/Omitted.md");
    fs::write(&page_path, "- backup version\n").unwrap();
    fs::write(&omitted_path, "- omitted from backup\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let backup = fs::read_to_string(&page_path).unwrap();

    let mut page = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = page.blocks[0]
        .raw
        .replacen("backup version", "newer live version", 1);
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    let before = update_chunks(dir.path()).len();

    assert!(graph
        .commit_managed_restore(&[("pages/Page.md".into(), backup.clone())])
        .unwrap());
    assert_eq!(update_chunks(dir.path()).len(), before + 1);
    assert!(fs::read_to_string(&page_path)
        .unwrap()
        .contains("newer live version"));

    // The exact pre-operation projection is the crash-recovery precondition.
    // Startup can therefore finish the durable restore without granting this
    // operation authority over any later bytes.
    graph.project_all_managed_sync().unwrap();
    assert_eq!(fs::read_to_string(&page_path).unwrap(), backup);
    assert!(!omitted_path.exists());
}

#[test]
fn restore_intent_does_not_overwrite_a_later_external_edit() {
    let dir = TestDir::new("restore-bounded-intent");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- backup version\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let backup = fs::read_to_string(&path).unwrap();

    let mut page = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = page.blocks[0]
        .raw
        .replacen("backup version", "live before restore", 1);
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    graph
        .commit_managed_restore(&[("pages/Page.md".into(), backup)])
        .unwrap();

    let later = fs::read_to_string(&path).unwrap().replacen(
        "live before restore",
        "later external edit",
        1,
    );
    fs::write(&path, later).unwrap();
    graph.project_all_managed_sync().unwrap();

    assert!(fs::read_to_string(path)
        .unwrap()
        .contains("later external edit"));
}

#[test]
fn restore_precondition_recovers_operation_first_on_another_replica() {
    let left = TestDir::new("restore-replica-left");
    let right = TestDir::new("restore-replica-right");
    let path = left.path().join("pages/Page.md");
    fs::write(&path, "- backup version\n").unwrap();
    let left_graph = Graph::open(left.path());
    left_graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let backup = fs::read_to_string(&path).unwrap();
    let mut page = left_graph
        .load_named("Page", PageKind::Page)
        .unwrap()
        .unwrap();
    page.blocks[0].raw = page.blocks[0]
        .raw
        .replacen("backup version", "pre-restore live", 1);
    left_graph.save_page(&page, page.rev.as_deref()).unwrap();

    copy_tree(left.path(), right.path());
    fs::remove_dir_all(right.path().join(".tine-sync/v1/projections")).unwrap();
    let right_graph = Graph::open(right.path());
    right_graph
        .start_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before = update_chunks(left.path()).len();
    left_graph
        .commit_managed_restore(&[("pages/Page.md".into(), backup.clone())])
        .unwrap();
    let updates = update_chunks(left.path());
    assert_eq!(updates.len(), before + 1);
    let restore = updates
        .iter()
        .find(|candidate| {
            !right
                .path()
                .join(".tine-sync/v1")
                .join(
                    candidate
                        .strip_prefix(left.path().join(".tine-sync/v1"))
                        .unwrap(),
                )
                .exists()
        })
        .unwrap();
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(restore, incoming.join(restore.file_name().unwrap())).unwrap();

    right_graph.pull_managed_sync().unwrap();
    assert_eq!(
        fs::read_to_string(right.path().join("pages/Page.md")).unwrap(),
        backup
    );
}

#[test]
fn managed_restore_rejects_pre_identity_backups_before_writing_an_operation() {
    let dir = TestDir::new("restore-old-backup");
    fs::write(dir.path().join("pages/Page.md"), "- current\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before = update_chunks(dir.path()).len();

    let error = graph
        .commit_managed_restore(&[("pages/Page.md".into(), "- old backup\n".into())])
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(update_chunks(dir.path()).len(), before);
}

#[test]
fn watcher_turns_an_external_file_deletion_into_an_operation() {
    let dir = TestDir::new("external-delete");
    let path = dir.path().join("pages/Page.md");
    fs::write(&path, "- delete in Logseq\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let before = update_chunks(dir.path()).len();

    fs::remove_file(&path).unwrap();
    graph.sync_deleted_file(&path).unwrap();

    assert_eq!(update_chunks(dir.path()).len(), before + 1);
    graph.project_all_managed_sync().unwrap();
    assert!(!path.exists());
}

#[test]
fn watcher_preserves_block_identity_when_an_external_tool_renames_a_file() {
    let dir = TestDir::new("external-rename");
    let old_path = dir.path().join("pages/Page.md");
    let new_path = dir.path().join("pages/Renamed.md");
    fs::write(&old_path, "- rename in Logseq\n").unwrap();
    let graph = Graph::open(dir.path());
    graph
        .enable_managed_sync(Uuid::new_v4(), Uuid::new_v4())
        .unwrap();
    let original_page = graph.load_named("Page", PageKind::Page).unwrap().unwrap();
    let original_runtime = original_page.blocks[0].id.clone();
    let original_id = durable_id(&original_page.blocks[0].raw).to_string();
    assert_ne!(original_runtime, original_id);

    fs::rename(&old_path, &new_path).unwrap();
    graph.sync_file_checked(&new_path).unwrap();
    graph.sync_deleted_file(&old_path).unwrap();
    graph.project_all_managed_sync().unwrap();

    assert!(!old_path.exists());
    let renamed = graph
        .load_named("Renamed", PageKind::Page)
        .unwrap()
        .unwrap();
    let renamed_id = renamed.blocks[0]
        .raw
        .lines()
        .find_map(|line| line.strip_prefix("id:: "))
        .expect("renamed projection retains its durable block id");
    assert_eq!(renamed_id, original_id);
    assert_ne!(renamed.blocks[0].id, renamed_id);
    assert_ne!(renamed.blocks[0].id, original_runtime);
}
