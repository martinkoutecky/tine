//! Rename unit-cost probe (I-25, fam 16). Ignored: run with
//! `TINE_RENAME_PROBE_ROOT=<copy of a generated 10k graph> TINE_RENAME_PROBE_OLD=..`
//! `cargo test -p tine-graph-features --release --test rename_cost_probe -- --ignored --nocapture`.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

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

/// Bytes the calling thread read through read(2) so far (`rchar`, Linux
/// only). The rename runs on this thread; background threads are excluded.
fn bytes_read() -> u64 {
    fs::read_to_string("/proc/thread-self/io")
        .ok()
        .and_then(|io| {
            io.lines()
                .find_map(|line| line.strip_prefix("rchar: ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}

#[test]
#[ignore]
fn rename_cost_at_scale() {
    let root = std::path::PathBuf::from(std::env::var("TINE_RENAME_PROBE_ROOT").unwrap());
    let old = std::env::var("TINE_RENAME_PROBE_OLD").unwrap();
    let new = format!("{old} renamed");
    let before = snapshot(&root);
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let warm = Instant::now();
    store.whole_graph().unwrap();
    let warm = warm.elapsed();
    let read_before = bytes_read();
    tine_store::cost_counters::reset();
    let started = Instant::now();
    hosted(&store, |host| {
        pages::rename_page_expected(&store, host, &old, &new, None)
    })
    .unwrap();
    let rename = started.elapsed();
    let counts = tine_store::cost_counters::snapshot();
    let read = bytes_read() - read_before;
    let after = snapshot(&root);
    let written: Vec<_> = after
        .iter()
        .filter(|(path, bytes)| before.get(*path) != Some(*bytes))
        .collect();
    let bytes: usize = written.iter().map(|(_, bytes)| bytes.len()).sum();
    println!(
        "PROBE old={old:?} warm_ms={} rename_ms={} files_written={} bytes_written={bytes} \
         dirs_listed={} full_reads={} bytes_read={read}",
        warm.as_millis(),
        rename.as_millis(),
        written.len(),
        counts.readdir,
        counts.full_reads,
    );
}
