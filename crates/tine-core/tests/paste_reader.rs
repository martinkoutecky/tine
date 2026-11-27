//! GH #1051: the paste table `tests/fixtures/paste-reader-blocks.json`, whose
//! blocks the frontend's `readPastedOutline` must produce (the paste of text
//! that opens with a block header; `src/editor/outline.test.ts`), is
//! what the page reader reads from the same text as a file, and those blocks,
//! pasted after an existing block, come back unchanged after save and reopen.

use serde_json::{json, Value};
use tine_core::doc::{self, DocBlock, Document};
use tine_core::org;

fn tree(blocks: &[DocBlock]) -> Value {
    Value::Array(
        blocks
            .iter()
            .map(|b| json!({"raw": b.raw(), "children": tree(&b.children)}))
            .collect(),
    )
}

fn blocks(tree: &Value) -> Vec<DocBlock> {
    let nodes = tree.as_array().unwrap();
    nodes
        .iter()
        .map(|node| {
            let mut block = DocBlock::new(node["raw"].as_str().unwrap());
            block.children = blocks(&node["children"]);
            block
        })
        .collect()
}

/// The reader's document as paste imports it: a non-blank pre-block first.
fn as_pasted(document: Document) -> Value {
    let mut roots = tree(&document.roots);
    if let Some(pre) = document.pre_block.filter(|pre| !pre.trim().is_empty()) {
        let raw = json!({"raw": pre, "children": []});
        roots.as_array_mut().unwrap().insert(0, raw);
    }
    roots
}

#[test]
fn pasted_blocks_are_the_page_readers_through_save_and_reopen() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/paste-reader-blocks.json"
    ))
    .unwrap();
    assert!(cases.len() > 30);
    for case in &cases {
        let (name, text, want) = (
            &case["name"],
            case["text"].as_str().unwrap(),
            &case["blocks"],
        );
        let is_org = case["format"] == "org";
        let read = |text: &str| {
            if is_org {
                org::parse_org(text)
            } else {
                doc::parse(text)
            }
        };
        let pasted = as_pasted(doc::read_pasted(text, is_org));
        assert_eq!(
            pasted,
            as_pasted(read(text)),
            "{name}: paste is the reader's"
        );
        assert_eq!(&pasted, want, "{name}: paste");

        let existing = if is_org {
            "* keep  \nbody\n"
        } else {
            "- keep  \n  body\n"
        };
        let mut page = read(existing);
        page.roots.extend(blocks(want));
        let out = if is_org {
            org::serialize_org(&page)
        } else {
            doc::serialize(&page)
        };
        assert!(
            out.starts_with(existing),
            "{name}: existing block bytes changed: {out:?}"
        );
        let reopened = read(&out);
        assert_eq!(
            &tree(&reopened.roots[1..]),
            want,
            "{name}: save and reopen: {out:?}"
        );
    }
}
