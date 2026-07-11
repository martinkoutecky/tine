use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};
use tine_core::crdt::{CrdtError, CrdtGraph, PageId, PageSnapshot};
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

    assert!(matches!(
        graph.commit_page(page(page_id, "not durable")),
        Err(CrdtError::Io(_))
    ));
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
