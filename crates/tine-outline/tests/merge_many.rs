//! GH #764: merging a run of selected blocks into the first one.
use tine_outline::{
    editor_text::HidePolicy,
    outline::{self, KeySource, Notice, OutlinePage, Reason, Verb},
};

const ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const OTHER: &str = "aaaaaaaa-0000-4000-8000-000000000002";

fn page(format: &str, roots: &[&str], blocks: serde_json::Value) -> OutlinePage {
    serde_json::from_value(serde_json::json!({
        "name": "P", "format": format, "roots": roots, "blocks": blocks
    }))
    .unwrap()
}

fn run(
    pages: &[OutlinePage],
    absorbed: &[&str],
    referenced: &[&str],
) -> Result<outline::Change, Reason> {
    outline::apply_with_policy(
        pages,
        &Verb::MergeMany {
            survivor: "a".into(),
            absorbed: absorbed.iter().map(|s| s.to_string()).collect(),
            referenced: referenced.iter().map(|s| s.to_string()).collect(),
        },
        &KeySource::default(),
        &HidePolicy::Builtin,
    )
}

fn upsert<'a>(change: &'a outline::Change, key: &str) -> &'a outline::Upsert {
    change.pages[0]
        .upserts
        .iter()
        .find(|u| u.key == key)
        .unwrap()
}

#[test]
fn joins_texts_with_line_breaks_and_keeps_every_child_in_order() {
    let p = page(
        "md",
        &["a", "b", "c", "z"],
        serde_json::json!({
            "a": { "raw": "one", "children": ["a1"] }, "a1": { "raw": "a-child" },
            "b": { "raw": "two", "children": ["b1"] }, "b1": { "raw": "b-child" },
            "c": { "raw": "three", "children": ["c1"] }, "c1": { "raw": "c-child" },
            "z": { "raw": "after" }
        }),
    );
    let change = run(&[p], &["b", "c"], &[]).unwrap();
    let a = upsert(&change, "a");
    assert_eq!(a.raw, "one\ntwo\nthree");
    assert_eq!(a.children, ["a1", "b1", "c1"]);
    assert_eq!(change.pages[0].roots.as_deref().unwrap(), ["a", "z"]);
    assert_eq!(change.pages[0].deleted, ["b", "c"]);
}

#[test]
fn a_selection_across_levels_merges_a_block_with_its_own_child() {
    // a, then a's child a1 (with its child a11), then a sibling b, then b's child.
    let p = page(
        "md",
        &["a", "b"],
        serde_json::json!({
            "a": { "raw": "A", "children": ["a1", "a2"] },
            "a1": { "raw": "A1", "children": ["a11"] }, "a11": { "raw": "A11" },
            "a2": { "raw": "A2" },
            "b": { "raw": "B", "children": ["b1"] }, "b1": { "raw": "B1" }
        }),
    );
    let change = run(&[p], &["a1", "a11", "b", "b1"], &[]).unwrap();
    let a = upsert(&change, "a");
    assert_eq!(a.raw, "A\nA1\nA11\nB\nB1");
    assert_eq!(a.children, ["a2"]);
    assert_eq!(change.pages[0].roots.as_deref().unwrap(), ["a"]);
    assert_eq!(change.pages[0].deleted.len(), 4);
}

#[test]
fn an_unselected_child_of_a_merged_block_keeps_its_subtree() {
    let p = page(
        "md",
        &["a", "b"],
        serde_json::json!({
            "a": { "raw": "A" },
            "b": { "raw": "B", "children": ["x"] },
            "x": { "raw": "X", "children": ["y"] }, "y": { "raw": "Y" }
        }),
    );
    let change = run(&[p], &["b"], &[]).unwrap();
    assert_eq!(upsert(&change, "a").children, ["x"]);
    assert!(change.pages[0].upserts.iter().all(|u| u.key != "x"));
}

#[test]
fn an_empty_text_adds_no_blank_line() {
    let p = page(
        "md",
        &["a", "b", "c"],
        serde_json::json!({ "a": { "raw": "" }, "b": { "raw": "two" }, "c": { "raw": "" } }),
    );
    let change = run(&[p], &["b", "c"], &[]).unwrap();
    assert_eq!(upsert(&change, "a").raw, "two");
}

#[test]
fn org_blocks_join_with_a_line_break_too() {
    let p = page(
        "org",
        &["a", "b"],
        serde_json::json!({ "a": { "raw": "one" }, "b": { "raw": "two" } }),
    );
    let change = run(&[p], &["b"], &[]).unwrap();
    assert_eq!(upsert(&change, "a").raw, "one\ntwo");
}

#[test]
fn a_referenced_id_moves_to_the_survivor_when_it_has_none() {
    let p = page(
        "md",
        &["a", "b", "c"],
        serde_json::json!({
            "a": { "raw": "one" },
            "b": { "raw": format!("two\nid:: {ID}") },
            "c": { "raw": format!("three\nid:: {OTHER}") }
        }),
    );
    // c's id is not referenced: it goes with its block, as in Backspace.
    let change = run(&[p], &["b", "c"], &["b"]).unwrap();
    let a = upsert(&change, "a");
    assert_eq!(a.raw, format!("one\ntwo\nthree\nid:: {ID}"));
    assert!(matches!(change.notices[..], [Notice::IdTransferred { .. }]));
}

#[test]
fn a_second_referenced_id_refuses_the_whole_merge() {
    let p = page(
        "md",
        &["a", "b", "c"],
        serde_json::json!({
            "a": { "raw": "one" },
            "b": { "raw": format!("two\nid:: {ID}") },
            "c": { "raw": format!("three\nid:: {OTHER}") }
        }),
    );
    assert_eq!(
        run(&[p], &["b", "c"], &["b", "c"]).unwrap_err(),
        Reason::ReferencedIdentity
    );
}

#[test]
fn a_referenced_id_refuses_when_the_survivor_has_one() {
    let p = page(
        "md",
        &["a", "b"],
        serde_json::json!({
            "a": { "raw": format!("one\nid:: {ID}") },
            "b": { "raw": format!("two\nid:: {OTHER}") }
        }),
    );
    assert_eq!(
        run(&[p], &["b"], &["b"]).unwrap_err(),
        Reason::ReferencedIdentity
    );
}

#[test]
fn refuses_across_pages_and_when_the_survivor_sits_in_an_absorbed_block() {
    let mut q = page(
        "md",
        &["q"],
        serde_json::json!({ "q": { "raw": "other page" } }),
    );
    q.name = "Q".into();
    let p = page(
        "md",
        &["a", "b"],
        serde_json::json!({ "a": { "raw": "one" }, "b": { "raw": "two" } }),
    );
    assert_eq!(
        run(&[p, q], &["b", "q"], &[]).unwrap_err(),
        Reason::CrossPageMerge
    );
    // The survivor is a child of an absorbed block.
    let nested = page(
        "md",
        &["p"],
        serde_json::json!({ "p": { "raw": "P", "children": ["a"] }, "a": { "raw": "A" } }),
    );
    assert_eq!(run(&[nested], &["p"], &[]).unwrap_err(), Reason::Cycle);
}

#[test]
fn refuses_past_the_depth_cap_and_on_a_read_only_page() {
    // b (depth 1) holds a chain that ends at the last legal depth; the survivor
    // a sits one level deeper, so moving the chain under it would pass the cap.
    let depth = outline::OUTLINE_MAX_DEPTH;
    let mut blocks = serde_json::Map::new();
    blocks.insert(
        "r".into(),
        serde_json::json!({ "raw": "R", "children": ["x", "b"] }),
    );
    blocks.insert(
        "x".into(),
        serde_json::json!({ "raw": "X", "children": ["a"] }),
    );
    blocks.insert("a".into(), serde_json::json!({ "raw": "A" }));
    blocks.insert(
        "b".into(),
        serde_json::json!({ "raw": "B", "children": ["n0"] }),
    );
    let chain = depth - 2;
    for i in 0..chain {
        let children: Vec<String> = if i + 1 < chain {
            vec![format!("n{}", i + 1)]
        } else {
            vec![]
        };
        blocks.insert(
            format!("n{i}"),
            serde_json::json!({ "raw": "N", "children": children }),
        );
    }
    let p = page("md", &["r"], serde_json::Value::Object(blocks));
    assert_eq!(run(&[p], &["b"], &[]).unwrap_err(), Reason::DepthCap);

    let mut ro = page(
        "md",
        &["a", "b"],
        serde_json::json!({ "a": { "raw": "one" }, "b": { "raw": "two" } }),
    );
    ro.read_only = true;
    assert_eq!(run(&[ro], &["b"], &[]).unwrap_err(), Reason::ReadOnly);
}
