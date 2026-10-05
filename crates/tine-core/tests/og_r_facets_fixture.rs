//! Checkpoint-5 packet R (I-12): the frontend's `facetsOf` (src/render/facets.ts)
//! and Rust's block projection answer the same header facts. Both languages read
//! `tests/fixtures/block-facets.json`; `src/render/facetsParity.test.ts` asserts the
//! TS half. `TINE_REGEN_FACETS=1 cargo test -p tine-core --test og_r_facets_fixture`
//! rewrites the expectations from Rust (review the diff by eye: Rust is the owner).
use serde_json::{json, Value};
use tine_core::doc::DocBlock;

const FIXTURE: &str = "tests/fixtures/block-facets.json";

fn facts(entry: &Value) -> Value {
    let mut block = DocBlock::new(entry["raw"].as_str().unwrap());
    block.set_org(entry["format"] == "org");
    json!({
        "marker": block.marker(),
        "priority": block.priority(),
        "heading": block.heading_level(),
        "scheduled": block.scheduled(),
        "deadline": block.deadline(),
        "tags": block.tags(),
        "properties": block.properties(),
    })
}

#[test]
fn rust_projection_matches_the_shared_facet_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let mut cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    if std::env::var_os("TINE_REGEN_FACETS").is_some() {
        for case in &mut cases {
            let expected = facts(case);
            case["expected"] = expected;
        }
        std::fs::write(&path, serde_json::to_string_pretty(&cases).unwrap() + "\n").unwrap();
        return;
    }
    for case in &cases {
        assert_eq!(case["expected"], facts(case), "{}", case["raw"]);
    }
}
