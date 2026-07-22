use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use tine_core::crdt::{CrdtError, CrdtGraph, PageId, PageSnapshot, ProjectionPrecondition};
use uuid::Uuid;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tine-crdt-{label}-{}", Uuid::new_v4()));
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

fn page(id: PageId, raw_pre_block: &str) -> PageSnapshot {
    PageSnapshot {
        id,
        path: "page.md".into(),
        name: "page".into(),
        kind: "page".into(),
        format: "markdown".into(),
        pre_block: Some(raw_pre_block.into()),
        blocks: vec![],
    }
}

fn precondition(path: &str, expected_content: Option<&str>) -> ProjectionPrecondition {
    ProjectionPrecondition {
        path: path.into(),
        expected_content: expected_content.map(str::to_string),
    }
}

fn chunk_paths(root: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), output);
            } else if entry.path().extension().and_then(|value| value.to_str()) == Some("chunk") {
                output.push(entry.path());
            }
        }
    }
    let mut output = Vec::new();
    visit(root, &mut output);
    output
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

fn files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    fn visit(path: &Path, extension: &str, output: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry.path(), extension, output);
            } else if entry.path().extension().and_then(|value| value.to_str()) == Some(extension) {
                output.push(entry.path());
            }
        }
    }
    let mut output = Vec::new();
    visit(root, extension, &mut output);
    output
}

#[cfg(unix)]
#[test]
fn managed_store_rejects_an_initial_symlink() {
    use std::os::unix::fs::symlink;

    let graph = TestDir::new("initial-store-link");
    let outside = TestDir::new("initial-store-link-outside");
    symlink(outside.path(), graph.path().join(".tine-sync")).unwrap();
    assert!(CrdtGraph::initialize(
        graph.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "blocked")],
    )
    .is_err());
    assert!(chunk_paths(outside.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn managed_store_keeps_writes_on_its_open_capability_after_parent_retargeting() {
    use std::os::unix::fs::symlink;

    let graph_dir = TestDir::new("retargeted-store");
    let outside = TestDir::new("retargeted-store-outside");
    let page_id = PageId::new();
    let mut graph = CrdtGraph::initialize(
        graph_dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(page_id, "durable")],
    )
    .unwrap();
    let original = graph_dir.path().join(".tine-sync-original");
    fs::rename(graph_dir.path().join(".tine-sync"), &original).unwrap();
    symlink(outside.path(), graph_dir.path().join(".tine-sync")).unwrap();

    graph
        .commit_page(page(page_id, "stays on the opened store"))
        .unwrap();
    assert!(chunk_paths(outside.path()).is_empty());
    assert_eq!(
        chunk_paths(&original.join("v1")).len(),
        2,
        "the update lands in the detached directory capability"
    );
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn rewrite_checksum(bytes: &mut [u8]) {
    let body_len = bytes.len() - 32;
    let checksum = Sha256::digest(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(&checksum);
}

#[test]
fn immutable_store_reopens_without_compacting_or_deleting() {
    let dir = TestDir::new("reopen");
    let page_id = PageId::new();
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(page_id, "one")],
    )
    .unwrap();
    graph.commit_page(page(page_id, "two")).unwrap();
    let before = chunk_paths(&dir.path().join(".tine-sync/v1"));
    assert_eq!(before.len(), 2);
    drop(graph);

    let reopened = CrdtGraph::open(dir.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();
    assert_eq!(
        reopened
            .materialize_page(page_id)
            .unwrap()
            .unwrap()
            .pre_block
            .as_deref(),
        Some("two")
    );
    assert_eq!(chunk_paths(&dir.path().join(".tine-sync/v1")).len(), 2);
}

#[test]
fn unchanged_snapshot_does_not_publish_an_empty_update_chunk() {
    let dir = TestDir::new("no-op");
    let initial = page(PageId::new(), "same");
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![initial.clone()],
    )
    .unwrap();
    let before = chunk_paths(&dir.path().join(".tine-sync/v1")).len();
    let report = graph.commit_page(initial).unwrap();
    assert!(!report.changed);
    assert!(report.chunk_id.is_empty());
    assert_eq!(chunk_paths(&dir.path().join(".tine-sync/v1")).len(), before);
}

#[test]
fn projection_receipt_requires_matching_content_path_and_frontier() {
    let left = TestDir::new("receipt-left");
    let right = TestDir::new("receipt-right");
    let page_id = PageId::new();
    let mut left_graph = CrdtGraph::initialize(
        left.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(page_id, "initial")],
    )
    .unwrap();
    copy_tree(
        &left.path().join(".tine-sync"),
        &right.path().join(".tine-sync"),
    );
    let mut right_graph = CrdtGraph::open(right.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();

    left_graph.commit_page(page(page_id, "changed")).unwrap();
    left_graph
        .record_projection("page.md", "- projected\n")
        .unwrap();
    assert!(left_graph
        .is_known_projection("page.md", "- projected\n")
        .unwrap());
    assert!(!left_graph
        .is_known_projection("other.md", "- projected\n")
        .unwrap());
    assert!(!left_graph
        .is_known_projection("page.md", "- external\n")
        .unwrap());

    let receipt = files_with_extension(&left.path().join(".tine-sync/v1"), "receipt")
        .pop()
        .unwrap();
    let relative = receipt.strip_prefix(left.path()).unwrap();
    let delivered_receipt = right.path().join(relative);
    fs::create_dir_all(delivered_receipt.parent().unwrap()).unwrap();
    fs::copy(receipt, delivered_receipt).unwrap();
    assert!(!right_graph
        .is_known_projection("page.md", "- projected\n")
        .unwrap());

    let update = chunk_paths(&left.path().join(".tine-sync/v1"))
        .into_iter()
        .find(|path| path.to_string_lossy().contains("/sessions/"))
        .unwrap();
    let incoming = right.path().join(".tine-sync/v1/incoming");
    fs::create_dir_all(&incoming).unwrap();
    fs::copy(&update, incoming.join(update.file_name().unwrap())).unwrap();
    let first = right_graph.import_pending().unwrap();
    assert_eq!(first.imported_chunks, 1);
    let retry = right_graph.import_pending().unwrap();
    assert_eq!(retry.imported_chunks, 0);
    assert_eq!(retry.affected_pages, first.affected_pages);
    right_graph.acknowledge_pending_projection();
    assert!(right_graph
        .import_pending()
        .unwrap()
        .affected_pages
        .is_empty());
    assert!(right_graph
        .is_known_projection("page.md", "- projected\n")
        .unwrap());
}

#[test]
fn rejects_truncated_and_checksum_invalid_chunks() {
    let dir = TestDir::new("corrupt");
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "")],
    )
    .unwrap();
    let incoming = dir.path().join(".tine-sync/v1/incoming");
    fs::create_dir(&incoming).unwrap();

    let truncated = b"TINESYNC";
    fs::write(
        incoming.join(format!("{}.chunk", digest_hex(truncated))),
        truncated,
    )
    .unwrap();
    assert!(matches!(
        graph.import_pending(),
        Err(CrdtError::InvalidChunk(_))
    ));
    fs::remove_dir_all(&incoming).unwrap();

    fs::create_dir(&incoming).unwrap();
    let source = chunk_paths(&dir.path().join(".tine-sync/v1"))[0].clone();
    let mut corrupt = Vec::new();
    File::open(source)
        .unwrap()
        .read_to_end(&mut corrupt)
        .unwrap();
    *corrupt.last_mut().unwrap() ^= 0x80;
    fs::write(
        incoming.join(format!("{}.chunk", digest_hex(&corrupt))),
        corrupt,
    )
    .unwrap();
    assert!(matches!(
        graph.import_pending(),
        Err(CrdtError::ChecksumMismatch)
    ));
}

#[test]
fn refuses_multiple_genesis_chunks() {
    let left = TestDir::new("genesis-left");
    let right = TestDir::new("genesis-right");
    CrdtGraph::initialize(
        left.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "left")],
    )
    .unwrap();
    CrdtGraph::initialize(
        right.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "right")],
    )
    .unwrap();

    let foreign = chunk_paths(&right.path().join(".tine-sync/v1/genesis"))[0].clone();
    let target = left
        .path()
        .join(".tine-sync/v1/genesis")
        .join(foreign.file_name().unwrap());
    fs::copy(foreign, target).unwrap();
    assert!(matches!(
        CrdtGraph::open(left.path(), Uuid::new_v4(), Uuid::new_v4()),
        Err(CrdtError::MultipleGenesis(2))
    ));
}

#[test]
fn resumes_empty_and_claimed_partial_initialization() {
    let empty = TestDir::new("empty-residue");
    fs::create_dir_all(empty.path().join(".tine-sync/v1/genesis")).unwrap();
    CrdtGraph::initialize(
        empty.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "")],
    )
    .unwrap();

    let claimed = TestDir::new("claimed-residue");
    let device = Uuid::new_v4();
    let session = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    let store = claimed.path().join(".tine-sync/v1");
    fs::create_dir_all(store.join("genesis")).unwrap();
    fs::create_dir_all(
        store
            .join("devices")
            .join(device.to_string())
            .join("sessions")
            .join(session.to_string()),
    )
    .unwrap();
    let mut claim = File::create(store.join("genesis.claim")).unwrap();
    claim
        .write_all(
            serde_json::to_string(&json!({
                "schema_version": 1,
                "workspace_id": workspace,
                "device_id": device,
                "session_id": session,
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    claim.sync_all().unwrap();

    let graph = CrdtGraph::initialize(
        claimed.path(),
        device,
        Uuid::new_v4(),
        vec![page(PageId::new(), "resumed")],
    )
    .unwrap();
    assert_eq!(graph.status().unwrap().workspace_id, workspace);
}

#[test]
fn activation_residue_with_unowned_artifacts_fails_closed() {
    let dir = TestDir::new("ambiguous-residue");
    let store = dir.path().join(".tine-sync/v1");
    fs::create_dir_all(store.join("projection-intents")).unwrap();
    fs::write(store.join("projection-intents/orphan.intent"), b"legacy").unwrap();

    assert!(CrdtGraph::store_state(dir.path()).is_err());
    assert!(CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "must not activate")],
    )
    .is_err());
    assert!(chunk_paths(&store).is_empty());
}

#[test]
fn failed_publish_rebuilds_the_last_durable_state() {
    let dir = TestDir::new("failed-publish");
    let page_id = PageId::new();
    let device = Uuid::new_v4();
    let session = Uuid::new_v4();
    let original = page(page_id, "durable");
    let mut graph =
        CrdtGraph::initialize(dir.path(), device, session, vec![original.clone()]).unwrap();
    let session_dir = dir
        .path()
        .join(".tine-sync/v1/devices")
        .join(device.to_string())
        .join("sessions")
        .join(session.to_string());
    fs::remove_dir(&session_dir).unwrap();

    assert!(graph.commit_page(page(page_id, "not durable")).is_err());
    assert_eq!(graph.materialize_page(page_id).unwrap(), Some(original));

    fs::create_dir(&session_dir).unwrap();
    graph.commit_page(page(page_id, "now durable")).unwrap();
    assert_eq!(
        graph
            .materialize_page(page_id)
            .unwrap()
            .unwrap()
            .pre_block
            .as_deref(),
        Some("now durable")
    );
}

#[test]
fn failed_update_publish_leaves_no_projection_authorization() {
    let dir = TestDir::new("failed-authorized-publish");
    let page_id = PageId::new();
    let device = Uuid::new_v4();
    let session = Uuid::new_v4();
    let mut graph =
        CrdtGraph::initialize(dir.path(), device, session, vec![page(page_id, "durable")]).unwrap();
    let session_dir = dir
        .path()
        .join(".tine-sync/v1/devices")
        .join(device.to_string())
        .join("sessions")
        .join(session.to_string());
    fs::remove_dir(&session_dir).unwrap();

    assert!(graph
        .replace_pages_with_projection_preconditions(
            vec![page(page_id, "not published")],
            vec![precondition("page.md", Some("before"))],
        )
        .is_err());
    assert!(files_with_extension(&dir.path().join(".tine-sync/v1"), "intent").is_empty());
    assert!(!graph
        .is_projection_authorized("page.md", Some("before"))
        .unwrap());

    fs::create_dir(&session_dir).unwrap();
    graph
        .replace_pages_with_projection_preconditions(
            vec![page(page_id, "published on retry")],
            vec![precondition("page.md", Some("before"))],
        )
        .unwrap();
    assert!(graph
        .is_projection_authorized("page.md", Some("before"))
        .unwrap());
    assert!(!graph
        .is_projection_authorized("page.md", Some("later external bytes"))
        .unwrap());
    assert!(!graph.is_projection_authorized("page.md", None).unwrap());
}

#[test]
fn published_update_repairs_an_interrupted_projection_intent() {
    let dir = TestDir::new("intent-recovery");
    let page_id = PageId::new();
    let device = Uuid::new_v4();
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        device,
        Uuid::new_v4(),
        vec![page(page_id, "before")],
    )
    .unwrap();
    let intents = dir.path().join(".tine-sync/v1/projection-intents");
    fs::write(&intents, b"blocks intent publication").unwrap();

    assert!(graph
        .replace_pages_with_projection_preconditions(
            vec![page(page_id, "after")],
            vec![precondition("page.md", Some("before bytes"))],
        )
        .is_err());
    assert_eq!(
        graph
            .materialize_page(page_id)
            .unwrap()
            .unwrap()
            .pre_block
            .as_deref(),
        Some("after"),
        "the operation chunk is durable even when its intent publish is interrupted"
    );

    fs::remove_file(&intents).unwrap();
    let reopened = CrdtGraph::open(dir.path(), device, Uuid::new_v4()).unwrap();
    assert!(reopened
        .is_projection_authorized("page.md", Some("before bytes"))
        .unwrap());
    assert!(!reopened
        .is_projection_authorized("page.md", Some("future edit"))
        .unwrap());
    assert_eq!(
        files_with_extension(&dir.path().join(".tine-sync/v1"), "intent").len(),
        1
    );
}

#[test]
fn no_op_restore_still_publishes_chunk_bound_authorization() {
    let dir = TestDir::new("no-op-restore-authorization");
    let snapshot = page(PageId::new(), "already desired");
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![snapshot.clone()],
    )
    .unwrap();
    let before = chunk_paths(&dir.path().join(".tine-sync/v1")).len();

    let report = graph
        .replace_pages_with_projection_preconditions(
            vec![snapshot],
            vec![precondition("page.md", None)],
        )
        .unwrap();
    assert!(
        report.changed,
        "the durable authorization chunk was published"
    );
    assert_eq!(
        chunk_paths(&dir.path().join(".tine-sync/v1")).len(),
        before + 1
    );
    assert!(graph.is_projection_authorized("page.md", None).unwrap());
    assert!(!graph
        .is_projection_authorized("page.md", Some("present"))
        .unwrap());
}

#[test]
fn rejects_schema_and_workspace_mismatches() {
    let schema_dir = TestDir::new("schema-mismatch");
    let mut schema_graph = CrdtGraph::initialize(
        schema_dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "")],
    )
    .unwrap();
    let source = chunk_paths(&schema_dir.path().join(".tine-sync/v1"))[0].clone();
    let mut future_schema = fs::read(source).unwrap();
    let marker = b"\"schema_version\":1";
    let position = future_schema
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    future_schema[position + marker.len() - 1] = b'2';
    rewrite_checksum(&mut future_schema);
    let incoming = schema_dir.path().join(".tine-sync/v1/incoming");
    fs::create_dir(&incoming).unwrap();
    fs::write(
        incoming.join(format!("{}.chunk", digest_hex(&future_schema))),
        future_schema,
    )
    .unwrap();
    assert!(matches!(
        schema_graph.import_pending(),
        Err(CrdtError::SchemaMismatch {
            expected: 1,
            found: 2
        })
    ));

    let left = TestDir::new("workspace-left");
    let right = TestDir::new("workspace-right");
    let mut left_graph = CrdtGraph::initialize(
        left.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(PageId::new(), "left")],
    )
    .unwrap();
    let right_page = PageId::new();
    let mut right_graph = CrdtGraph::initialize(
        right.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![page(right_page, "right")],
    )
    .unwrap();
    let foreign = right_graph
        .commit_page(page(right_page, "foreign update"))
        .unwrap();
    let foreign_path = chunk_paths(&right.path().join(".tine-sync/v1"))
        .into_iter()
        .find(|path| {
            path.file_stem().and_then(|value| value.to_str()) == Some(foreign.chunk_id.as_str())
        })
        .unwrap();
    let incoming = left.path().join(".tine-sync/v1/incoming");
    fs::create_dir(&incoming).unwrap();
    fs::copy(
        &foreign_path,
        incoming.join(foreign_path.file_name().unwrap()),
    )
    .unwrap();
    assert!(matches!(
        left_graph.import_pending(),
        Err(CrdtError::WorkspaceMismatch { .. })
    ));
}
