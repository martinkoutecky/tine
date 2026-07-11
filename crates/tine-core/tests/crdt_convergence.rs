use std::fs;
use std::path::{Path, PathBuf};

use tine_core::crdt::{BlockId, BlockSnapshot, CrdtGraph, PageId, PageSnapshot};
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

fn block(id: BlockId, parent: Option<BlockId>, order: u32, raw: &str) -> BlockSnapshot {
    BlockSnapshot {
        id,
        parent,
        order,
        raw: raw.into(),
    }
}

fn page(id: PageId, pre_block: &str, blocks: Vec<BlockSnapshot>) -> PageSnapshot {
    PageSnapshot {
        id,
        path: "page.md".into(),
        name: "page".into(),
        kind: "page".into(),
        format: "markdown".into(),
        pre_block: Some(pre_block.into()),
        blocks,
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

fn find_chunk(root: &Path, id: &str) -> PathBuf {
    fn visit(path: &Path, name: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                if let Some(found) = visit(&entry.path(), name) {
                    return Some(found);
                }
            } else if entry.file_name() == name {
                return Some(entry.path());
            }
        }
        None
    }
    visit(root, &format!("{id}.chunk")).unwrap()
}

fn deliver(root: &Path, source: &Path, lane: &str, duplicate: bool) {
    let name = source.file_name().unwrap();
    let first = root.join(".tine-sync/v1/incoming").join(lane);
    fs::create_dir_all(&first).unwrap();
    fs::copy(source, first.join(name)).unwrap();
    if duplicate {
        let second = root.join(".tine-sync/v1/incoming-duplicate").join(lane);
        fs::create_dir_all(&second).unwrap();
        fs::copy(source, second.join(name)).unwrap();
    }
}

#[test]
fn replicas_converge_after_concurrent_text_insert_move_delete_and_duplicate_delivery() {
    let left_dir = TestDir::new("converge-left");
    let right_dir = TestDir::new("converge-right");
    let page_id = PageId::new();
    let one = BlockId::new();
    let two = BlockId::new();
    let three = BlockId::new();
    let inserted_left = BlockId::new();
    let inserted_right = BlockId::new();
    let initial = page(
        page_id,
        "title:: Base\n",
        vec![
            block(one, None, 0, "shared text"),
            block(two, None, 1, "move me"),
            block(three, None, 2, "delete or edit me"),
        ],
    );

    let mut left = CrdtGraph::initialize(
        left_dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![initial],
    )
    .unwrap();
    copy_tree(
        &left_dir.path().join(".tine-sync"),
        &right_dir.path().join(".tine-sync"),
    );
    let mut right = CrdtGraph::open(right_dir.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();

    let left_first = left
        .commit_page(page(
            page_id,
            "title:: Left\n",
            vec![
                block(one, None, 0, "shared left text"),
                block(inserted_left, None, 1, "left insert"),
                block(two, Some(one), 0, "move me"),
            ],
        ))
        .unwrap();
    let left_second = left
        .commit_page(page(
            page_id,
            "title:: Left final\n",
            vec![
                block(one, None, 0, "shared left text!"),
                block(inserted_left, None, 1, "left insert"),
                block(two, Some(one), 0, "move me"),
            ],
        ))
        .unwrap();
    let right_update = right
        .commit_page(page(
            page_id,
            "title:: Right\n",
            vec![
                block(one, None, 0, "shared right text"),
                block(inserted_right, None, 1, "right insert"),
                block(two, None, 2, "move me"),
                block(three, Some(one), 0, "edited instead of deleted"),
            ],
        ))
        .unwrap();

    let left_store = left_dir.path().join(".tine-sync/v1");
    let right_store = right_dir.path().join(".tine-sync/v1");
    deliver(
        left_dir.path(),
        &find_chunk(&right_store, &right_update.chunk_id),
        "right",
        true,
    );
    // Deliver the dependent left updates through reversed lane names and duplicate each.
    deliver(
        right_dir.path(),
        &find_chunk(&left_store, &left_second.chunk_id),
        "00-second",
        true,
    );
    deliver(
        right_dir.path(),
        &find_chunk(&left_store, &left_first.chunk_id),
        "99-first",
        true,
    );

    assert_eq!(left.import_pending().unwrap().imported_chunks, 1);
    assert_eq!(right.import_pending().unwrap().imported_chunks, 2);
    let left_pages = left.materialize_pages().unwrap();
    let right_pages = right.materialize_pages().unwrap();
    assert_eq!(left_pages, right_pages);
    assert!(left_pages[0]
        .blocks
        .iter()
        .any(|block| block.id == inserted_left));
    assert!(left_pages[0]
        .blocks
        .iter()
        .any(|block| block.id == inserted_right));

    drop(left);
    drop(right);
    let reopened_left = CrdtGraph::open(left_dir.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();
    let reopened_right = CrdtGraph::open(right_dir.path(), Uuid::new_v4(), Uuid::new_v4()).unwrap();
    assert_eq!(
        reopened_left.materialize_pages().unwrap(),
        reopened_right.materialize_pages().unwrap()
    );
}
