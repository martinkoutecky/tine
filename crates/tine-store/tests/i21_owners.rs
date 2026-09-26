use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

// One row per production acquisition family. Stop is a cancellation or graph
// revocation signal; join-or-cancel describes who reaps or bounds the work.
const OWNERS: &[(&str, &str, usize, &str, &str)] = &[
    (
        "crates/tine-store/src/store.rs",
        "thread::spawn(",
        1,
        "Store slot close",
        "load worker checks cancellation per page",
    ),
    (
        "crates/tine-store/src/watch.rs",
        "thread::spawn(",
        1,
        "WatchHandle::stop",
        "WatchHandle joins worker",
    ),
    (
        "crates/tine-store/src/watch.rs",
        "recommended_watcher(",
        1,
        "WatchHandle::stop",
        "watcher dropped after worker stop",
    ),
    (
        "src-tauri/src/backup.rs",
        "thread::spawn(",
        1,
        "GraphSlot background_cancelled",
        "detached snapshot checks cancellation per entry",
    ),
    (
        "src-tauri/src/backup.rs",
        "spawn_blocking(",
        2,
        "command future",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/graph.rs",
        "thread::spawn(",
        1,
        "GraphSlot warm_generation",
        "detached warm checks generation before publication",
    ),
    (
        "src-tauri/src/watcher.rs",
        "thread::spawn(",
        1,
        "GraphSlot background_cancelled",
        "detached bridge exits on subscription close",
    ),
    (
        "src-tauri/src/commands.rs",
        "spawn_blocking(",
        24,
        "command future",
        "caller awaits blocking result",
    ),
];

fn counts(sources: &[(String, String)]) -> BTreeMap<(String, String), usize> {
    let mut found = BTreeMap::new();
    for (file, source) in sources {
        let source = if file == "crates/tine-store/src/store.rs" {
            source.split("mod rev5_tests {").next().unwrap()
        } else {
            source.split("mod tests {").next().unwrap()
        };
        for needle in ["thread::spawn(", "recommended_watcher(", "spawn_blocking("] {
            let count = source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//") && line.contains(needle))
                .count();
            if count > 0 {
                found.insert((file.to_owned(), needle.to_owned()), count);
            }
        }
    }
    found
}

fn rust_sources(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) != Some("bin") {
                rust_sources(&path, root, out);
            }
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs")
            || stem.ends_with("_tests")
            || stem.starts_with("test_")
        {
            continue;
        }
        let file = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        out.push((file, fs::read_to_string(path).unwrap()));
    }
}

fn assert_owned(actual: &BTreeMap<(String, String), usize>) {
    let expected: BTreeMap<_, _> = OWNERS
        .iter()
        .map(|(file, call, count, stop, join)| {
            assert!(!stop.is_empty() && !join.is_empty());
            (((*file).to_owned(), (*call).to_owned()), *count)
        })
        .collect();
    assert_eq!(actual, &expected,
        "I-21: every spawned worker or watcher needs a named owner, stop and join-or-cancel path; exemplar watch.rs WatchHandle::stop");
}

#[test]
fn production_acquisitions_have_owners() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_sources(&root.join("crates/tine-store/src"), &root, &mut files);
    rust_sources(
        &root.join("crates/tine-graph-features/src"),
        &root,
        &mut files,
    );
    for file in [
        "src-tauri/src/backup.rs",
        "src-tauri/src/commands.rs",
        "src-tauri/src/graph.rs",
        "src-tauri/src/state.rs",
        "src-tauri/src/watcher.rs",
        "src-tauri/src/device_io.rs",
    ] {
        files.push((
            file.to_owned(),
            fs::read_to_string(root.join(file)).unwrap(),
        ));
    }
    assert_owned(&counts(&files));
}

#[test]
fn planted_unowned_worker_fails() {
    let source = vec![(
        "src-tauri/src/new_worker.rs".to_owned(),
        "std::thread::spawn(move || {});".to_owned(),
    )];
    assert!(
        std::panic::catch_unwind(|| assert_owned(&counts(&source))).is_err(),
        "I-21: planted worker must fail; exemplar watch.rs WatchHandle::stop"
    );
}
