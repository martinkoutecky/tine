use tine_core::doc::DocBlock;
use tine_core::model::block_dto_estimated_bytes;
use tine_core::projection::{block_to_bounded_dto, subtree_node_count};

#[test]
fn bounded_projection_keeps_preorder_and_shared_budgets() {
    let leaf = |id: &str| {
        let mut block = DocBlock::new("x");
        block.uuid = id.into();
        block
    };
    let mut root = leaf("root");
    let mut child = leaf("a");
    child.children.push(leaf("b"));
    root.children = vec![child, leaf("c")];
    assert_eq!(subtree_node_count(&root), 4);
    let (mut nodes, mut bytes) = (2, usize::MAX);
    let dto = block_to_bounded_dto(&root, &mut nodes, &mut bytes).unwrap();
    assert_eq!(nodes, 0);
    assert_eq!(dto.children.len(), 1);
    assert_eq!(dto.children[0].id, "a");
    assert!(dto.children[0].children.is_empty());
    let root_bytes = block_dto_estimated_bytes(&tine_core::projection::block_to_shallow_dto(&root));
    let (mut nodes, mut bytes) = (4, root_bytes);
    assert!(block_to_bounded_dto(&root, &mut nodes, &mut bytes)
        .unwrap()
        .children
        .is_empty());
    assert_eq!((nodes, bytes), (3, 0));
    let (mut nodes, mut bytes) = (4, 1);
    assert!(block_to_bounded_dto(&root, &mut nodes, &mut bytes).is_none());
    assert_eq!((nodes, bytes), (4, 1));
}

#[test]
fn native_byte_policy_can_admit_an_ancestor_sibling_after_rejecting_a_child() {
    let mut root = DocBlock::new("root");
    root.uuid = "root".into();
    let mut a = DocBlock::new("a");
    a.uuid = "a".into();
    let mut huge = DocBlock::new(&"x".repeat(10000));
    huge.uuid = "huge".into();
    a.children.push(huge);
    let mut c = DocBlock::new("c");
    c.uuid = "c".into();
    root.children = vec![a, c];
    let (mut nodes, mut bytes) = (4, 4096);
    let dto = block_to_bounded_dto(&root, &mut nodes, &mut bytes).unwrap();
    assert_eq!(dto.children.len(), 2);
    assert!(dto.children[0].children.is_empty());
    assert_eq!(dto.children[1].id, "c");
    assert_eq!(nodes, 1);
}
