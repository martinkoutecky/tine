//! External asset refresh (og-J2; master d017d1afc, 2f54a8d5e): a file under
//! the graph's assets capability that is replaced, created or deleted outside
//! Tine reaches the subscriber as an `Origin::External` `assets/<rel>` change,
//! for an in-graph directory and for an approved external target (including one
//! reached through a symlinked path). Own writes never echo back; the watch is
//! released with the store.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tine_store::{ChangeKind, OpenOptions, Origin, Store, Subscription, WatchMode};

static NEXT: AtomicUsize = AtomicUsize::new(0);
/// Serialize these tests so a sibling cannot reuse the descriptor being observed.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tine-asset-watch-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // The store canonicalizes its root; so do the tests, or macOS /tmp differs.
    std::fs::canonicalize(dir).unwrap()
}

fn graph_at(root: &Path) {
    for dir in ["pages", "journals", "logseq"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
}

fn open(root: &Path, mode: WatchMode, approved: Option<PathBuf>) -> (Store, Subscription) {
    let store = Store::open(
        root,
        OpenOptions {
            approved_external_assets: approved,
            watch: mode,
            launch_checkpoint: None,
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    let subscription = store.subscribe();
    (store, subscription)
}

/// Replace exactly as a synchronizer does: temp beside the target, then rename.
fn replace(dir: &Path, name: &str, bytes: &[u8]) {
    let temp = dir.join(format!(".{name}.incoming"));
    std::fs::write(&temp, bytes).unwrap();
    std::fs::rename(temp, dir.join(name)).unwrap();
}

/// Collect external asset tuples until `want` holds or 10 s pass.
fn wait_for_asset(
    subscription: &Subscription,
    id: &str,
    kind: ChangeKind,
) -> Vec<(String, ChangeKind, bool)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    loop {
        while let Some(change) = subscription.try_recv().unwrap() {
            for (file, file_kind, rev) in &change.files {
                seen.push((
                    file.as_str().to_owned(),
                    *file_kind,
                    change.origin == Origin::External && rev.is_none(),
                ));
            }
            if change.origin == Origin::External
                && change
                    .files
                    .iter()
                    .any(|(file, file_kind, _)| file.as_str() == id && *file_kind == kind)
            {
                return seen;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no External {kind:?} for {id}; saw {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_replaced_in_graph_asset_is_published_and_a_new_and_deleted_one_too() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = scratch("in-graph");
    graph_at(&root);
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/pic.png"), b"first").unwrap();
    let (store, subscription) = open(&root, WatchMode::Notify, None);

    replace(&root.join("assets"), "pic.png", b"second, longer");
    wait_for_asset(&subscription, "assets/pic.png", ChangeKind::Modified);

    replace(&root.join("assets"), "new.png", b"new");
    wait_for_asset(&subscription, "assets/new.png", ChangeKind::Created);

    std::fs::remove_file(root.join("assets/pic.png")).unwrap();
    wait_for_asset(&subscription, "assets/pic.png", ChangeKind::Removed);
    store.close();
}

#[test]
fn nested_assets_use_the_assets_relative_id_and_temp_names_are_not_assets() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = scratch("nested");
    graph_at(&root);
    std::fs::create_dir_all(root.join("assets/sub")).unwrap();
    let (store, subscription) = open(&root, WatchMode::Notify, None);

    // Tine's own publisher temp shape, left behind then renamed to the target.
    let temp = root.join(format!("assets/sub/.a.png.{}.7.tmp", std::process::id()));
    std::fs::write(&temp, b"x").unwrap();
    std::fs::rename(&temp, root.join("assets/sub/a.png")).unwrap();
    let seen = wait_for_asset(&subscription, "assets/sub/a.png", ChangeKind::Created);
    assert!(
        seen.iter().all(|(id, _, _)| !id.ends_with(".tmp")),
        "a publisher temp file surfaced as an asset: {seen:?}"
    );
    store.close();
}

#[cfg(unix)]
#[test]
fn an_approved_external_assets_root_is_observed_through_a_symlinked_path() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    use std::os::unix::fs::symlink;
    let root = scratch("external");
    graph_at(&root);
    let real = scratch("external-real").join("media");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("pixel.png"), b"one").unwrap();
    // `alias` -> `media`; the graph's `assets` -> `alias`. The approval is the
    // canonical target; the user's edits arrive by the linked spelling.
    let alias = real.with_file_name("alias");
    symlink(&real, &alias).unwrap();
    symlink(&alias, root.join("assets")).unwrap();
    let approved = std::fs::canonicalize(&alias).unwrap();
    assert_eq!(approved, real);
    let (store, subscription) = open(&root, WatchMode::Notify, Some(approved));

    replace(&alias, "pixel.png", b"two, longer");
    wait_for_asset(&subscription, "assets/pixel.png", ChangeKind::Modified);

    std::fs::remove_file(alias.join("pixel.png")).unwrap();
    wait_for_asset(&subscription, "assets/pixel.png", ChangeKind::Removed);
    store.close();
}

#[test]
fn scan_refresh_and_poll_mode_see_an_asset_replaced_outside() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = scratch("poll");
    graph_at(&root);
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/pic.png"), b"first").unwrap();
    let (store, subscription) = open(&root, WatchMode::Poll, None);
    replace(&root.join("assets"), "pic.png", b"second, longer");
    store.refresh(tine_store::Depth::Stamps).unwrap();
    wait_for_asset(&subscription, "assets/pic.png", ChangeKind::Modified);
    store.close();
}

#[test]
fn an_own_asset_write_is_never_echoed_as_external() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = scratch("own");
    graph_at(&root);
    let (store, subscription) = open(&root, WatchMode::Notify, None);
    let name = tine_graph_features::assets::save_asset(&store, "own.png", b"mine").unwrap();
    assert_eq!(name, "own.png");
    // Let the watcher see (and reconcile) the events of that write, and a
    // forced full pass on top, then read the whole feed.
    std::thread::sleep(Duration::from_millis(900));
    store.refresh(tine_store::Depth::Stamps).unwrap();
    let mut origins = Vec::new();
    while let Some(change) = subscription.try_recv().unwrap() {
        if change
            .files
            .iter()
            .any(|(id, _, _)| id.as_str() == "assets/own.png")
        {
            origins.push(change.origin);
        }
    }
    assert_eq!(
        origins,
        vec![Origin::Own],
        "the own write must be published once, as Own"
    );
    store.close();
}

/// Identify the descriptor by a watch on this graph's root, rather than by
/// the count of unrelated notify event-loop threads still shutting down.
#[cfg(target_os = "linux")]
fn inotify_descriptor_for(root: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let inode = format!("ino:{:x}", std::fs::metadata(root).unwrap().ino());
    std::fs::read_dir("/proc/self/fdinfo")
        .unwrap()
        .flatten()
        .find_map(|entry| {
            let info = std::fs::read_to_string(entry.path()).ok()?;
            info.lines()
                .any(|line| {
                    line.starts_with("inotify ")
                        && line.split_whitespace().any(|field| field == inode)
                })
                .then(|| Path::new("/proc/self/fd").join(entry.file_name()))
        })
}

#[cfg(target_os = "linux")]
fn wait_for_watch_release(descriptor: &Path) {
    // notify releases its descriptor on its backend event-loop thread after
    // Drop posts Shutdown. Observe the actual release, with the same deadline.
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_link(descriptor)
        .is_ok_and(|target| target.to_string_lossy().contains("inotify"))
    {
        assert!(
            Instant::now() < deadline,
            "closing the store must release its OS watch: {descriptor:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_watch_is_released_when_the_store_closes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = scratch("lifecycle");
    graph_at(&root);
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let (store, _subscription) = open(&root, WatchMode::Notify, None);
    let deadline = Instant::now() + Duration::from_secs(10);
    let descriptor = loop {
        if let Some(descriptor) = inotify_descriptor_for(&root) {
            break descriptor;
        }
        assert!(
            Instant::now() < deadline,
            "no OS watch was installed for this graph"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    store.close();
    wait_for_watch_release(&descriptor);
    std::fs::remove_dir_all(root).unwrap();
}
