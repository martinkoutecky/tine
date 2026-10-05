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

#[test]
fn deep_dto_is_the_shallow_dto_plus_projected_children() {
    use tine_core::projection::{block_to_dto, block_to_shallow_dto};
    let mut root = DocBlock::new(
        "TODO [#A] Heading text #tagged\nSCHEDULED: <2026-01-02 Fri>\nDEADLINE: <2026-01-03 Sat>\nkey:: value",
    );
    root.uuid = "root-id".into();
    let mut child = DocBlock::new("DOING child\nid:: 6a1b2c3d-0000-4000-8000-000000000001");
    child.uuid = "6a1b2c3d-0000-4000-8000-000000000001".into();
    let mut grandchild = DocBlock::new("grandchild");
    grandchild.uuid = "grandchild-id".into();
    child.children.push(grandchild);
    let mut plain = DocBlock::new("plain");
    plain.uuid = "plain-id".into();
    root.children = vec![child, plain];

    let deep = block_to_dto(&root);
    // The independent oracle: facets are the parser's answers for this raw text.
    assert_eq!(deep.id, "root-id");
    assert_eq!(deep.has_id, Some(false));
    assert_eq!(deep.marker.as_deref(), Some("TODO"));
    assert_eq!(deep.priority.as_deref(), Some("A"));
    assert_eq!(deep.scheduled.as_deref(), Some("2026-01-02 Fri"));
    assert_eq!(deep.deadline.as_deref(), Some("2026-01-03 Sat"));
    assert!(deep.tags.iter().any(|t| t == "tagged"), "{:?}", deep.tags);
    assert!(deep.properties.iter().any(|(k, v)| k == "key" && v == "value"));
    assert_eq!(deep.children.len(), 2);
    assert_eq!(deep.children[0].has_id, Some(true));
    assert_eq!(deep.children[0].marker.as_deref(), Some("DOING"));
    assert_eq!(deep.children[0].children.len(), 1);
    // Structural relation, at every node: deep == shallow with children attached.
    fn check(block: &DocBlock, dto: &tine_core::model::BlockDto) {
        let mut expected = serde_json::to_value(block_to_shallow_dto(block)).unwrap();
        let mut actual = serde_json::to_value(dto).unwrap();
        assert_eq!(dto.children.len(), block.children.len());
        expected["children"] = serde_json::Value::Null;
        actual["children"] = serde_json::Value::Null;
        assert_eq!(actual, expected, "{}", block.raw());
        for (child, child_dto) in block.children.iter().zip(&dto.children) {
            check(child, child_dto);
        }
    }
    check(&root, &deep);
}
