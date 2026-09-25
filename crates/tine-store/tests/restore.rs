use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tine_store::{Area, OpenOptions, RestoreFile, Store};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "tine-store-restore-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn graph(root: &Path, external_assets: Option<&Path>) -> Store {
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    if external_assets.is_none() {
        fs::create_dir_all(root.join("assets")).unwrap();
    }
    Store::open(
        root,
        OpenOptions {
            approved_external_assets: external_assets.map(Path::to_path_buf),
        },
    )
    .unwrap()
    .0
}

fn input(path: &Path, area: Area, rel: &str) -> RestoreFile {
    let source = File::open(path).unwrap();
    let len = source.metadata().unwrap().len();
    RestoreFile {
        area,
        rel: rel.into(),
        source,
        len,
    }
}

fn wait_paused(root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join(".tine-restore-test-paused").exists() {
        assert!(
            Instant::now() < deadline,
            "restore did not reach the bound-handle pause"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn recovery_dir(root: &Path) -> PathBuf {
    fs::read_dir(root.join("logseq/.tine-trash"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap()
}

#[test]
fn restore_recovery_roots_live_on_the_filesystems_they_detach_from() {
    let root = scratch("recovery-roots");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/secret.md"), b"live").unwrap();
    fs::write(graph_root.join("assets/doc.edn"), b"live").unwrap();
    let report = store.restore(Vec::new()).unwrap();
    assert_eq!(report.recovery.len(), 2);
    assert!(report.recovery[0].starts_with(graph_root.join("logseq/.tine-trash")));
    assert!(report.recovery[1].starts_with(graph_root.join("assets/.tine-restore-recovery")));
    assert_eq!(
        fs::read(report.recovery[0].join("pages/secret.md")).unwrap(),
        b"live"
    );
    assert_eq!(
        fs::read(report.recovery[1].join("doc.edn")).unwrap(),
        b"live"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_recovery_symlink_cannot_redirect_or_replace_outside() {
    use std::os::unix::fs::symlink;
    let root = scratch("recovery-symlink");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    let live = graph_root.join("pages/secret.md");
    fs::write(&live, b"live graph data").unwrap();
    fs::create_dir_all(outside.join("restore-1/pages")).unwrap();
    let outside_target = outside.join("restore-1/pages/secret.md");
    fs::write(&outside_target, b"outside sentinel").unwrap();
    symlink(&outside, graph_root.join("logseq/.tine-trash")).unwrap();
    assert!(store.restore(Vec::new()).is_err());
    assert_eq!(fs::read(&live).unwrap(), b"live graph data");
    assert_eq!(fs::read(&outside_target).unwrap(), b"outside sentinel");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_recovery_path_swap_stays_on_the_bound_directory() {
    use std::os::unix::fs::symlink;
    let root = scratch("recovery-swap");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    fs::write(graph_root.join("pages/secret.md"), b"live graph data").unwrap();
    fs::create_dir_all(outside.join("pages")).unwrap();
    fs::write(outside.join("pages/secret.md"), b"outside sentinel").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| store.restore(Vec::new()));
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        let displaced = recovery.with_extension("displaced");
        fs::rename(&recovery, &displaced).unwrap();
        symlink(&outside, &recovery).unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        task.join().unwrap().unwrap();
        assert!(!graph_root.join("pages/secret.md").exists());
        assert_eq!(
            fs::read(displaced.join("pages/secret.md")).unwrap(),
            b"live graph data"
        );
        assert_eq!(
            fs::read(outside.join("pages/secret.md")).unwrap(),
            b"outside sentinel"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn restore_live_path_swap_cannot_move_or_publish_outside() {
    use std::os::unix::fs::symlink;
    let root = scratch("live-swap");
    let graph_root = root.join("graph");
    let outside = root.join("outside");
    let store = graph(&graph_root, None);
    let pages = graph_root.join("pages");
    let snapshot = root.join("snapshot.md");
    fs::create_dir_all(&outside).unwrap();
    fs::write(pages.join("secret.md"), b"live graph data").unwrap();
    fs::write(outside.join("secret.md"), b"outside sentinel").unwrap();
    fs::write(&snapshot, b"snapshot data").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| store.restore(vec![input(&snapshot, Area::Pages, "new.md")]));
        wait_paused(&graph_root);
        let displaced = graph_root.join("pages.displaced");
        fs::rename(&pages, &displaced).unwrap();
        symlink(&outside, &pages).unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        assert!(task.join().unwrap().is_err());
        assert_eq!(
            fs::read(displaced.join("secret.md")).unwrap(),
            b"live graph data"
        );
        assert_eq!(
            fs::read(outside.join("secret.md")).unwrap(),
            b"outside sentinel"
        );
        assert!(!outside.join("new.md").exists());
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_recovery_never_replaces_an_existing_entry() {
    let root = scratch("recovery-no-replace");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let live = graph_root.join("pages/secret.md");
    let snapshot = root.join("snapshot.md");
    fs::write(&live, b"live graph data").unwrap();
    fs::write(&snapshot, b"snapshot data").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| store.restore(vec![input(&snapshot, Area::Pages, "secret.md")]));
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        fs::create_dir_all(recovery.join("pages")).unwrap();
        fs::write(recovery.join("pages/secret.md"), b"recovery sentinel").unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        assert!(task.join().unwrap().is_err());
        assert_eq!(fs::read(&live).unwrap(), b"live graph data");
        assert_eq!(
            fs::read(recovery.join("pages/secret.md")).unwrap(),
            b"recovery sentinel"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_restore_reports_completed_files_and_preserves_the_failing_target() {
    let root = scratch("partial-report");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let first = root.join("first.md");
    let second = root.join("second.md");
    fs::write(&first, b"new first").unwrap();
    fs::write(&second, b"new second").unwrap();
    fs::write(graph_root.join("pages/First.md"), b"old first").unwrap();
    fs::write(graph_root.join("pages/Second.md"), b"old second").unwrap();
    fs::write(graph_root.join(".tine-restore-test-pause"), b"pause").unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(|| {
            store.restore(vec![
                input(&first, Area::Pages, "First.md"),
                input(&second, Area::Pages, "Second.md"),
            ])
        });
        wait_paused(&graph_root);
        let recovery = recovery_dir(&graph_root);
        fs::create_dir_all(recovery.join("pages")).unwrap();
        fs::write(recovery.join("pages/Second.md"), b"recovery sentinel").unwrap();
        fs::write(graph_root.join(".tine-restore-test-resume"), b"resume").unwrap();
        let failed = task.join().unwrap().err().expect("second file must fail");
        assert_eq!(failed.phase, "restore pages failed");
        assert_eq!(failed.done.restored, 1);
        assert!(failed.done.kept_external.is_empty());
        assert_eq!(
            fs::read(graph_root.join("pages/First.md")).unwrap(),
            b"new first"
        );
        assert_eq!(
            fs::read(graph_root.join("pages/Second.md")).unwrap(),
            b"old second"
        );
        assert_eq!(
            fs::read(recovery.join("pages/First.md")).unwrap(),
            b"old first"
        );
    });
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_asset_sidecars_dir_restores_sidecars_and_leaves_binary_assets() {
    let root = scratch("sidecars");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let assets = graph_root.join("assets");
    let snapshot = root.join("snapshot");
    fs::create_dir_all(assets.join("nested")).unwrap();
    fs::create_dir_all(snapshot.join("nested")).unwrap();
    fs::write(snapshot.join("doc.edn"), b"new\n").unwrap();
    fs::write(snapshot.join("nested/hl.edn"), b"nested new\n").unwrap();
    fs::write(assets.join("doc.edn"), b"old\n").unwrap();
    fs::write(assets.join("stale.edn"), b"stale\n").unwrap();
    fs::write(assets.join("nested/stale.edn"), b"stale\n").unwrap();
    fs::write(assets.join("image.png"), b"keep").unwrap();
    fs::write(assets.join("nested/image.png"), b"keep").unwrap();
    let report = store
        .restore(vec![
            input(&snapshot.join("doc.edn"), Area::Assets, "doc.edn"),
            input(
                &snapshot.join("nested/hl.edn"),
                Area::Assets,
                "nested/hl.edn",
            ),
        ])
        .unwrap();
    assert_eq!(report.restored, 2);
    assert_eq!(fs::read(assets.join("doc.edn")).unwrap(), b"new\n");
    assert_eq!(
        fs::read(assets.join("nested/hl.edn")).unwrap(),
        b"nested new\n"
    );
    assert!(!assets.join("stale.edn").exists());
    assert!(!assets.join("nested/stale.edn").exists());
    assert_eq!(
        fs::read(report.recovery[1].join("stale.edn")).unwrap(),
        b"stale\n"
    );
    assert_eq!(fs::read(assets.join("image.png")).unwrap(), b"keep");
    assert_eq!(fs::read(assets.join("nested/image.png")).unwrap(), b"keep");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_text_backup_and_restore_include_nested_pages() {
    let root = scratch("nested-pages");
    let graph_root = root.join("graph");
    let store = graph(&graph_root, None);
    let snapshot = root.join("snapshot");
    fs::create_dir_all(snapshot.join("client-a")).unwrap();
    fs::create_dir_all(graph_root.join("pages/client-a")).unwrap();
    fs::write(snapshot.join("Top.md"), b"top\n").unwrap();
    fs::write(snapshot.join("client-a/Deep.md"), b"deep\n").unwrap();
    fs::write(graph_root.join("pages/client-a/Deep.md"), b"corrupt\n").unwrap();
    fs::write(graph_root.join("pages/client-a/Stale.md"), b"stale\n").unwrap();
    fs::write(graph_root.join("pages/client-a/notes.txt"), b"keep\n").unwrap();
    let report = store
        .restore(vec![
            input(&snapshot.join("Top.md"), Area::Pages, "Top.md"),
            input(
                &snapshot.join("client-a/Deep.md"),
                Area::Pages,
                "client-a/Deep.md",
            ),
        ])
        .unwrap();
    assert_eq!(
        fs::read(graph_root.join("pages/client-a/Deep.md")).unwrap(),
        b"deep\n"
    );
    assert!(!graph_root.join("pages/client-a/Stale.md").exists());
    assert_eq!(
        fs::read(report.recovery[0].join("pages/client-a/Stale.md")).unwrap(),
        b"stale\n"
    );
    assert_eq!(
        fs::read(graph_root.join("pages/client-a/notes.txt")).unwrap(),
        b"keep\n"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn complete_restore_crosses_from_app_data_to_a_distinct_live_filesystem() {
    use std::os::unix::fs::MetadataExt;
    let app_data = scratch("cross-device-source");
    let live_root = PathBuf::from("/dev/shm").join(format!(
        "tine-restore-cross-device-live-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&live_root);
    if fs::create_dir_all(&live_root).is_err()
        || fs::metadata(&app_data).unwrap().dev() == fs::metadata(&live_root).unwrap().dev()
    {
        let _ = fs::remove_dir_all(&app_data);
        let _ = fs::remove_dir_all(&live_root);
        return;
    }
    let snapshot = app_data.join("snapshot");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(snapshot.join(dir)).unwrap();
    }
    let store = graph(&live_root, None);
    for (rel, bytes) in [
        ("pages/Kept.md", b"snapshot page\n".as_slice()),
        ("journals/2026_07_15.md", b"snapshot journal\n"),
        ("assets/doc.edn", b"{:snapshot true}\n"),
        ("logseq/config.edn", b"{:snapshot true}\n"),
    ] {
        fs::write(snapshot.join(rel), bytes).unwrap();
    }
    for (rel, bytes) in [
        ("pages/Kept.md", b"live page\n".as_slice()),
        ("pages/Stale.md", b"stale page\n"),
        ("journals/Old.md", b"old journal\n"),
        ("assets/doc.edn", b"{:live true}\n"),
        ("assets/stale.edn", b"{:stale true}\n"),
        ("assets/binary.pdf", b"keep binary"),
        ("logseq/config.edn", b"{:live true}\n"),
    ] {
        fs::write(live_root.join(rel), bytes).unwrap();
    }
    let report = store
        .restore(vec![
            input(
                &snapshot.join("journals/2026_07_15.md"),
                Area::Journals,
                "2026_07_15.md",
            ),
            input(&snapshot.join("pages/Kept.md"), Area::Pages, "Kept.md"),
            input(&snapshot.join("assets/doc.edn"), Area::Assets, "doc.edn"),
            input(
                &snapshot.join("logseq/config.edn"),
                Area::Meta,
                "config.edn",
            ),
        ])
        .unwrap();
    assert_eq!(
        fs::read(live_root.join("pages/Kept.md")).unwrap(),
        b"snapshot page\n"
    );
    assert!(!live_root.join("pages/Stale.md").exists());
    assert_eq!(
        fs::read(live_root.join("journals/2026_07_15.md")).unwrap(),
        b"snapshot journal\n"
    );
    assert!(!live_root.join("journals/Old.md").exists());
    assert_eq!(
        fs::read(live_root.join("assets/doc.edn")).unwrap(),
        b"{:snapshot true}\n"
    );
    assert!(!live_root.join("assets/stale.edn").exists());
    assert_eq!(
        fs::read(live_root.join("assets/binary.pdf")).unwrap(),
        b"keep binary"
    );
    assert_eq!(
        fs::read(live_root.join("logseq/config.edn")).unwrap(),
        b"{:snapshot true}\n"
    );
    assert_eq!(
        fs::read(report.recovery[0].join("pages/Stale.md")).unwrap(),
        b"stale page\n"
    );
    assert_eq!(
        fs::read(report.recovery[0].join("logseq/config.edn")).unwrap(),
        b"{:live true}\n"
    );
    assert_eq!(
        fs::read(report.recovery[1].join("stale.edn")).unwrap(),
        b"{:stale true}\n"
    );
    drop(store);
    fs::remove_dir_all(app_data).unwrap();
    fs::remove_dir_all(live_root).unwrap();
}

#[cfg(unix)]
#[test]
fn approved_external_assets_restore_keeps_recovery_on_target() {
    use std::os::unix::fs::symlink;
    let root = scratch("external-assets");
    let graph_root = root.join("graph");
    let assets = root.join("external-assets");
    fs::create_dir_all(&graph_root).unwrap();
    fs::create_dir_all(&assets).unwrap();
    symlink(&assets, graph_root.join("assets")).unwrap();
    let store = graph(&graph_root, Some(&assets));
    let source = root.join("source.edn");
    fs::write(&source, b"new").unwrap();
    fs::write(assets.join("old.edn"), b"old").unwrap();
    let report = store
        .restore(vec![input(&source, Area::Assets, "new.edn")])
        .unwrap();
    assert_eq!(fs::read(assets.join("new.edn")).unwrap(), b"new");
    assert!(!assets.join("old.edn").exists());
    assert!(report.recovery[1].starts_with(assets.join(".tine-restore-recovery")));
    assert_eq!(
        fs::read(report.recovery[1].join("old.edn")).unwrap(),
        b"old"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
