use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

// One row per production acquisition family. Stop is a cancellation or graph
// revocation signal; join-or-cancel describes who reaps or bounds the work.
const OWNERS: &[(&str, &str, usize, &str, &str)] = &[
    (
        "crates/tine-store/src/model.rs",
        ".spawn(move ||",
        1,
        "parse_pages_parallel scope (on-demand build and background warm)",
        "std::thread::scope joins its child before return",
    ),
    (
        "crates/tine-store/src/page_host/driver.rs",
        ".spawn(move ||",
        1,
        "Driver::join and Drop set `stopping` and wake the host condition",
        "Driver::join and Drop join the driver thread",
    ),
    (
        "crates/tine-store/src/store.rs",
        ".spawn(move ||",
        1,
        "Store slot close",
        "load worker checks cancellation per page",
    ),
    (
        "crates/tine-store/src/store/checkpoint.rs",
        ".spawn(move ||",
        1,
        "Store::close stops the checkpoint Signal",
        "detached; a write in flight finishes or dies with the process, and the atomic replace keeps the previous checkpoint",
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
        1,
        "command future",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/backup/restore.rs",
        "spawn_blocking(",
        1,
        "command future",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/host_retirement.rs",
        ".spawn(move ||",
        // Step 3b P1: one thread per retiring slot whose host runs.
        1,
        "the retiring host's stop completing, or `HostRetirement::adopt` by a reopen of its root (the thread then finds its slot unclaimed and returns)",
        "detached; app exit waits on `HostRetirement::wait_idle` (30 s bound), and the thread removes its slot from the map before it ends",
    ),
    (
        "src-tauri/src/graph.rs",
        ".spawn(move ||",
        1,
        "StartupGraph's single launch-result slot",
        "load joins the worker once; unused results drop their Store and cancel its load worker",
    ),
    (
        "src-tauri/src/graph.rs",
        "spawn_blocking(",
        3,
        "load_graph / open_graph_window command future (and load_graph's iOS Simulator Guide-copy probe)",
        "caller awaits the graph open or the probe's copy; a window closed meanwhile releases only that open's binding",
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
        "src-tauri/src/watcher.rs",
        "spawn_blocking(",
        1,
        "rescan_graph_now: one bounded refresh on the slot's Store",
        "detached; the scan returns and its completion is registered on RescanCursor, never waited on",
    ),
    (
        "src-tauri/src/commands.rs",
        "spawn_blocking(",
        // OG-B-FAIL2: trash_asset now awaits one blocking job because the
        // writer-side reference check can await initial graph publication.
        // Its command future owns/reaps the result; Store close revokes writes.
        // +1 GH #623: get_page_by_path parses off the main thread, awaited by
        // its command future like get_page.
        // +2 GH #623: open_asset and edit_asset_external start the OS opener
        // (PATH search, exec) on the blocking pool, awaited by their futures.
        // -1 step 3b P2b: the deleted `delete_page` command (page_delete).
        36,
        "command future",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/settings.rs",
        "spawn_blocking(",
        // GH #623: save_session's two fsyncs run on the blocking pool; the
        // command future awaits and reaps the result before replying.
        // +1 GH #623: reveal_known_graph waits for dbus-send there.
        2,
        "command future",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/state.rs",
        "spawn_blocking(",
        // og-flow3 R3: `off_ui` is the one helper that moves a command's
        // blocking store/fsync work off the UI thread; every caller awaits it.
        1,
        "command future (off_ui caller)",
        "caller awaits blocking result",
    ),
    (
        "src-tauri/src/commands/concord.rs",
        "spawn_blocking(",
        // +2 og-A: duplicate_journal_diff and resolve_duplicate_journal_day,
        // each awaited by its own command future like the six before them.
        // +1 step 3b R6: merge_live_conflict, likewise.
        // -1 step 3b P2b: the deleted `resolve_live_conflict` command.
        8,
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
        for needle in [
            "thread::spawn(",
            ".spawn(move ||",
            "recommended_watcher(",
            "spawn_blocking(",
        ] {
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
        "src-tauri/src/backup/restore.rs",
        "src-tauri/src/commands.rs",
        "src-tauri/src/commands/concord.rs",
        "src-tauri/src/graph.rs",
        "src-tauri/src/host_retirement.rs",
        "src-tauri/src/settings.rs",
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
