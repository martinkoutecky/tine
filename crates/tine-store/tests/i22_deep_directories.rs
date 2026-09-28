//! I-22: a deep directory tree delivered into the graph (sync service, external
//! copy) must not exhaust the stack of trash stats, trash purge or area scans.
//! Linux caps a path at 4096 bytes (~2000 one-letter levels); Windows long
//! paths reach 32,767 characters (~16,000 levels), so these walks run the
//! Linux-maximal tree on a stack one ninth of a 2 MiB command worker.
use std::fs;
use std::path::{Path, PathBuf};

use tine_store::{Area, Store};

const SCALED_WORKER_STACK: usize = 2 * 1024 * 1024 / 9;

fn deep_tree(base: &Path) -> PathBuf {
    let budget = 4000usize.saturating_sub(base.as_os_str().len() + 16);
    let mut dir = base.to_path_buf();
    for _ in 0..budget / 2 {
        dir.push("d");
    }
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("leaf.png"), b"leaf").unwrap();
    dir
}

fn graph(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("i22-deep-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    root
}

fn on_small_stack(work: impl FnOnce() + Send) {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(SCALED_WORKER_STACK)
            .spawn_scoped(scope, work)
            .unwrap()
            .join()
            .unwrap();
    });
}

#[test]
fn deep_trash_directory_is_counted_and_purged_iteratively() {
    let root = graph("trash");
    let entry = root.join("logseq/.tine-trash/assets/entry");
    deep_tree(&entry);
    let store = Store::open(&root, Default::default()).unwrap().0;
    on_small_stack(|| {
        let stats = store.trash_stats().unwrap();
        assert!(stats
            .iter()
            .any(|(_, count, bytes)| *count == 1 && *bytes == 4));
        assert_eq!(store.purge_asset_trash().unwrap(), (1, 4));
    });
    assert!(!entry.exists(), "the whole deep entry is purged");
    store.close();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn deep_asset_directory_is_scanned_iteratively() {
    let root = graph("scan");
    deep_tree(&root.join("assets"));
    let store = Store::open(&root, Default::default()).unwrap().0;
    on_small_stack(|| {
        let listing = store.scan_area(Area::Assets, None).unwrap();
        assert!(listing
            .files
            .iter()
            .any(|file| file.rel.ends_with("/d/leaf.png")));
    });
    store.close();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn restore_retires_extras_from_a_deep_page_directory_iteratively() {
    let root = graph("restore");
    fs::create_dir_all(root.join("logseq")).unwrap();
    let leaf = deep_tree(&root.join("pages"));
    fs::write(leaf.join("stray.md"), b"- stray\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    on_small_stack(|| {
        let report = store
            .restore(tine_store::EditKind::ReplacePage, Vec::new())
            .unwrap();
        assert!(!report.recovery.is_empty());
    });
    assert!(!leaf.join("stray.md").exists(), "the stray page is retired");
    store.close();
    let _ = fs::remove_dir_all(&root);
}
