//! I-22 (og C3 L03): a property value is reparsed for its references. That
//! reparse called raw `lsdoc::parse_format`, bypassing the depth-bounded
//! block-parse boundary (`crates/lsdoc-block-parse.rs`), and walked/dropped
//! the tree recursively. A synced or imported `foo:: >>>>…` (≥5000 `>`)
//! builds a ~2500-deep quote staircase and aborted the process as soon as the
//! query index was built (first backlinks/search/query) — on every launch.

use std::path::PathBuf;

use tine_store::{OpenOptions, QueryDialect, Store};

fn run_small_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn_scoped(scope, work)
            .unwrap()
            .join()
            .unwrap()
    })
}

#[test]
fn a_deep_property_value_does_not_abort_the_index_build() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("tine-i22-prop-depth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["pages", "journals"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(
        dir.join("pages/x.md"),
        format!(
            "- hello [[Target]]\n  foo:: {}x [[Inner]]\n",
            ">".repeat(20_000)
        ),
    )
    .unwrap();
    // The benign extreme: a shallow property value keeps its references.
    std::fs::write(
        dir.join("pages/y.md"),
        format!("- shallow\n  bar:: {} [[Inner]]\n", "> ".repeat(8)),
    )
    .unwrap();
    let (store, _, _) = Store::open(&dir, OpenOptions::default()).expect("open");
    let view = store.whole_graph().expect("load");
    let (target, inner) = run_small_stack(|| {
        let _ = view.query("(task TODO)", QueryDialect::Simple);
        (
            view.backlinks("Target").map(|groups| groups.len()),
            view.backlinks("Inner").map(|groups| groups.len()),
        )
    });
    assert_eq!(target.unwrap(), 1, "the block's own reference survives");
    assert!(
        inner.unwrap() >= 1,
        "the shallow property value still references"
    );
    store.close();
    let _ = std::fs::remove_dir_all(&dir);
}
