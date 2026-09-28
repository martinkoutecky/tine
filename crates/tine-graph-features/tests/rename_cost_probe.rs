//! Rename unit-cost probe (I-25, fam 16). Ignored: run with
//! `TINE_RENAME_PROBE_ROOT=<copy of a generated 10k graph> TINE_RENAME_PROBE_OLD=..`
//! `cargo test -p tine-graph-features --release --test rename_cost_probe -- --ignored --nocapture`.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tine_graph_features::pages;
use tine_store::Store;

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for dir in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            let name = format!("{dir}/{}", entry.file_name().to_string_lossy());
            out.insert(name, fs::read(entry.path()).unwrap());
        }
    }
    out
}

#[test]
#[ignore]
fn rename_cost_at_scale() {
    let root = std::path::PathBuf::from(std::env::var("TINE_RENAME_PROBE_ROOT").unwrap());
    let old = std::env::var("TINE_RENAME_PROBE_OLD").unwrap();
    let new = format!("{old} renamed");
    let before = snapshot(&root);
    let store = Store::open(&root, Default::default()).unwrap().0;
    let warm = Instant::now();
    store.whole_graph().unwrap();
    let warm = warm.elapsed();
    let started = Instant::now();
    pages::rename_page_expected(&store, &old, &new, None).unwrap();
    let rename = started.elapsed();
    let after = snapshot(&root);
    let written: Vec<_> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .collect();
    let bytes: usize = written.iter().map(|(_, bytes)| bytes.len()).sum();
    println!(
        "PROBE old={old:?} warm_ms={} rename_ms={} files_written={} bytes_written={bytes}",
        warm.as_millis(),
        rename.as_millis(),
        written.len()
    );
}
