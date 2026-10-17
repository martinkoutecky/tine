//! Family 11: an outside `logseq/config.edn` edit (Logseq, a text editor, a
//! sync delivery) is taken in by the running store on its own watcher cycle,
//! without `refresh()` and without reopening the graph. Master contract:
//! `docs/contracts/config-live-reload.md`; og's is the same file in this repo.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tine_store::{ChangeKind, OpenOptions, Origin, Store, Subscription, WatchMode};

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-config-live-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::create_dir_all(root.join("logseq")).unwrap();
    root
}

fn open(root: &Path, mode: WatchMode) -> (Store, Subscription) {
    let store = Store::open(
        root,
        OpenOptions {
            approved_external_assets: None,
            watch: mode,
            launch_checkpoint: None,
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    let subscription = store.subscribe();
    // Let the watcher install its OS subscription before the outside write.
    std::thread::sleep(Duration::from_millis(200));
    (store, subscription)
}

/// An outside writer's atomic replace: temp file, then rename over the target.
fn deliver(root: &Path, rel: &str, text: &str) {
    let temp = root.join(format!(".delivery-{}", rel.replace('/', "_")));
    std::fs::write(&temp, text).unwrap();
    std::fs::rename(temp, root.join(rel)).unwrap();
}

/// Wait (no `refresh`) for an External publication naming `rel`.
fn external_change_naming(subscription: &Subscription, rel: &str, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some(change) = subscription.try_recv().unwrap() {
            if change.origin == Origin::External
                && change.files.iter().any(|(id, _, _)| id.as_str() == rel)
            {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn live_reload_in(mode: WatchMode) {
    let root = scratch(if mode == WatchMode::Poll {
        "poll"
    } else {
        "notify"
    });
    std::fs::write(root.join("logseq/config.edn"), "{:preferred-format :md}\n").unwrap();
    let (store, subscription) = open(&root, mode);
    assert_eq!(store.config().preferred_format.ext(), "md");

    deliver(&root, "logseq/config.edn", "{:preferred-format :org}\n");
    assert!(
        external_change_naming(&subscription, "logseq/config.edn", Duration::from_secs(10)),
        "family 11: an outside config.edn write must be published by the watcher cycle ({mode:?})"
    );
    assert_eq!(
        store.config().preferred_format.ext(),
        "org",
        "family 11: the running store serves the delivered configuration ({mode:?})"
    );
    store.close();
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_outside_config_edit_is_taken_in_by_the_notify_watcher() {
    let _integration_guard = crate::integration_lock();
    live_reload_in(WatchMode::Notify);
}

#[test]
fn an_outside_config_edit_is_taken_in_by_the_poll_watcher() {
    let _integration_guard = crate::integration_lock();
    live_reload_in(WatchMode::Poll);
}

/// Byte-identity gate: a config reload discards every parsed page, so a rewrite
/// of identical bytes (Logseq rewrites config.edn on ordinary UI actions,
/// Syncthing redelivers it) must publish no configuration change.
#[test]
fn an_identical_config_rewrite_publishes_nothing() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("identical");
    std::fs::write(root.join("logseq/config.edn"), "{:preferred-format :md}\n").unwrap();
    let (store, subscription) = open(&root, WatchMode::Notify);
    deliver(&root, "logseq/config.edn", "{:preferred-format :md}\n");
    assert!(
        !external_change_naming(&subscription, "logseq/config.edn", Duration::from_secs(4)),
        "an identical config.edn rewrite must not reload the configuration"
    );
    store.close();
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

/// Refusal scenario (sync delivery / external-editor race): a delivered
/// config.edn that names a page directory escaping the graph is not taken in,
/// the served configuration stays the last good one, and page changes keep
/// being observed — a bad config must not blind the watcher.
#[cfg(unix)]
#[test]
fn an_unsafe_delivered_config_keeps_the_served_one_and_pages_stay_observed() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("unsafe");
    std::fs::write(root.join("logseq/config.edn"), "{:preferred-format :md}\n").unwrap();
    let outside = root.with_extension("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("linked-pages")).unwrap();
    let (store, subscription) = open(&root, WatchMode::Notify);
    deliver(
        &root,
        "logseq/config.edn",
        "{:pages-directory \"linked-pages\" :preferred-format :org}\n",
    );
    std::thread::sleep(Duration::from_millis(600));
    deliver(&root, "pages/Seen.md", "- arrived\n");
    assert!(
        external_change_naming(&subscription, "pages/Seen.md", Duration::from_secs(10)),
        "a refused config must not stop page observation"
    );
    assert_eq!(store.config().pages_dir, "pages");
    assert_eq!(store.config().preferred_format.ext(), "md");
    store.close();
    drop(store);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

/// A config file that disappears keeps the config read from it, as Logseq's
/// watcher does (an iCloud offload or a sync tool's replace must not switch the
/// graph to the defaults and their :legacy file names); its return is taken in
/// again.
#[test]
fn a_removed_config_keeps_its_settings_until_it_returns() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("removed");
    std::fs::write(root.join("logseq/config.edn"), "{:preferred-format :org}\n").unwrap();
    let (store, subscription) = open(&root, WatchMode::Poll);
    std::fs::remove_file(root.join("logseq/config.edn")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut removed = false;
    while Instant::now() < deadline && !removed {
        if let Some(change) = subscription.try_recv().unwrap() {
            removed = change.files.iter().any(|(id, kind, _)| {
                id.as_str() == "logseq/config.edn" && *kind == ChangeKind::Removed
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(removed, "a removed config.edn is published");
    assert_eq!(
        store.config().preferred_format.ext(),
        "org",
        "a vanished config.edn keeps the settings read from it"
    );
    assert!(store.config().problem.is_none());
    deliver(&root, "logseq/config.edn", "{:preferred-format :md}\n");
    assert!(external_change_naming(
        &subscription,
        "logseq/config.edn",
        Duration::from_secs(10)
    ));
    assert_eq!(store.config().preferred_format.ext(), "md");
    store.close();
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

/// A graph whose config.edn is only an iCloud placeholder opens read-only: its
/// settings (above all :file/name-format) are unknown, and the defaults would
/// write namespace pages under :legacy file names. A plainly absent config.edn
/// still opens writable with Logseq's defaults.
#[test]
fn an_undownloaded_icloud_config_opens_the_graph_read_only() {
    let _integration_guard = crate::integration_lock();
    let root = scratch("icloud-placeholder");
    std::fs::write(root.join("logseq/.config.edn.icloud"), b"bplist00").unwrap();
    let (store, _subscription) = open(&root, WatchMode::Poll);
    assert!(
        store.config().problem.is_some(),
        "an undownloaded config.edn is a config problem (read-only graph)"
    );
    store.close();
    drop(store);
    std::fs::remove_file(root.join("logseq/.config.edn.icloud")).unwrap();
    let (store, _subscription) = open(&root, WatchMode::Poll);
    assert!(
        store.config().problem.is_none(),
        "no config.edn at all is no problem"
    );
    store.close();
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
