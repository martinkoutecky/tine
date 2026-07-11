use std::fs;
use std::path::{Path, PathBuf};

use tine_core::crdt::{BlockId, BlockSnapshot, CrdtError, CrdtGraph, PageId, PageSnapshot};
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

fn page(id: PageId, path: &str, pre_block: &str, blocks: Vec<BlockSnapshot>) -> PageSnapshot {
    PageSnapshot {
        id,
        path: path.into(),
        name: path.trim_end_matches(".md").into(),
        kind: "page".into(),
        format: "markdown".into(),
        pre_block: Some(pre_block.into()),
        blocks,
    }
}

#[test]
fn materializes_structure_and_reuses_blocks_for_destination_first_moves() {
    let dir = TestDir::new("structure");
    let page_a = PageId::new();
    let page_b = PageId::new();
    let root = BlockId::new();
    let child = BlockId::new();
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![
            page(
                page_a,
                "a.md",
                "title:: A\n",
                vec![
                    block(root, None, 0, "root"),
                    block(child, Some(root), 0, "child"),
                ],
            ),
            page(page_b, "b.md", "", vec![]),
        ],
    )
    .unwrap();

    let initial = graph.materialize_page(page_a).unwrap().unwrap();
    assert_eq!(initial.pre_block.as_deref(), Some("title:: A\n"));
    assert_eq!(initial.blocks[1].parent, Some(root));

    graph
        .commit_page(page(
            page_b,
            "b.md",
            "alias:: B\n",
            vec![block(child, None, 0, "child")],
        ))
        .unwrap();

    assert_eq!(
        graph.materialize_page(page_a).unwrap().unwrap().blocks,
        vec![block(root, None, 0, "root")]
    );
    assert_eq!(
        graph.materialize_page(page_b).unwrap().unwrap().blocks,
        vec![block(child, None, 0, "child")]
    );

    // A later source-page projection does not delete the globally reused node.
    graph
        .commit_page(page(
            page_a,
            "a.md",
            "title:: A\n",
            vec![block(root, None, 0, "root")],
        ))
        .unwrap();
    assert_eq!(
        graph.materialize_page(page_b).unwrap().unwrap().blocks[0].id,
        child
    );
}

#[test]
fn rejects_duplicate_ids_and_invalid_trees_before_mutation() {
    let duplicate = BlockId::new();
    let dir = TestDir::new("duplicate-initialize");
    let error = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![
            page(
                PageId::new(),
                "a.md",
                "",
                vec![block(duplicate, None, 0, "a")],
            ),
            page(
                PageId::new(),
                "b.md",
                "",
                vec![block(duplicate, None, 0, "b")],
            ),
        ],
    )
    .unwrap_err();
    assert!(matches!(error, CrdtError::DuplicateBlockId(id) if id == duplicate));
    assert!(!dir.path().join(".tine-sync").exists());

    let dir = TestDir::new("invalid-commit");
    let page_id = PageId::new();
    let original = page(page_id, "page.md", "", vec![]);
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![original.clone()],
    )
    .unwrap();
    let left = BlockId::new();
    let right = BlockId::new();
    let cyclic = page(
        page_id,
        "page.md",
        "",
        vec![
            block(left, Some(right), 0, "left"),
            block(right, Some(left), 0, "right"),
        ],
    );
    assert!(matches!(
        graph.commit_page(cyclic),
        Err(CrdtError::InvalidSnapshot(_))
    ));
    assert_eq!(graph.materialize_page(page_id).unwrap(), Some(original));
}

#[test]
fn preserves_absent_and_present_empty_pre_block() {
    let dir = TestDir::new("pre-block-presence");
    let absent_id = PageId::new();
    let empty_id = PageId::new();
    let mut absent = page(absent_id, "absent.md", "stale hidden text", vec![]);
    absent.pre_block = None;
    let empty = page(empty_id, "empty.md", "", vec![]);
    let mut graph = CrdtGraph::initialize(
        dir.path(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![absent, empty],
    )
    .unwrap();

    assert_eq!(
        graph
            .materialize_page(absent_id)
            .unwrap()
            .unwrap()
            .pre_block,
        None
    );
    assert_eq!(
        graph.materialize_page(empty_id).unwrap().unwrap().pre_block,
        Some(String::new())
    );

    let mut now_empty = page(absent_id, "absent.md", "", vec![]);
    now_empty.pre_block = Some(String::new());
    graph.commit_page(now_empty).unwrap();
    assert_eq!(
        graph
            .materialize_page(absent_id)
            .unwrap()
            .unwrap()
            .pre_block,
        Some(String::new())
    );
}
