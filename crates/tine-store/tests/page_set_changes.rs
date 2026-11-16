//! GH #543 R15-10 / queue card g8baTc: every way the page SET can change under
//! a running or restarted store must surface in the name inventory, and every
//! running-store change must carry `inventoryChanged` (the signal the frontend
//! page index refetches on). Cases: M1 a page deleted while the app was closed,
//! M2 an unreadable page repaired by an outside editor, an outside rename, a
//! page file delivered by sync, and alias-only / namespace-only edits.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tine_store::{Depth, InventoryScope, OpenOptions, Store, Subscription, WatchMode};

fn graph(files: &[(&str, &[u8])]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-page-set-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, bytes) in files {
        fs::write(root.join(rel), bytes).unwrap();
    }
    root
}

fn open(root: &Path) -> Store {
    let store = Store::open(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    store.whole_graph().unwrap();
    store
}

/// Every listed name with its kind: file-claimed `existing`, `alias`, or
/// reference-only `absent`.
fn names(store: &Store) -> Vec<(String, &'static str)> {
    let mut out: Vec<_> = store
        .whole_graph()
        .unwrap()
        .inventory(InventoryScope::All)
        .0
        .iter()
        .map(|entry| {
            let kind = match entry.target {
                tine_store::Resolved::Existing { .. } => "existing",
                tine_store::Resolved::Alias { .. } => "alias",
                tine_store::Resolved::Absent { .. } => "absent",
            };
            (entry.name.clone(), kind)
        })
        .collect();
    out.sort();
    out
}

fn has(store: &Store, name: &str, kind: &str) -> bool {
    names(store).iter().any(|(n, k)| n == name && *k == kind)
}

/// Re-scan until `done` holds (the watcher may publish part of an edit first),
/// then report whether ANY publication on the way carried `inventoryChanged`.
fn inventory_signalled(store: &Store, sub: &Subscription, done: impl Fn(&Store) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut signalled = false;
    let drain = |signalled: &mut bool| {
        while let Some(change) = sub.try_recv().unwrap() {
            let wire: Value = serde_json::to_value(change).unwrap();
            *signalled |= wire["inventoryChanged"] == true;
        }
    };
    while !done(store) {
        assert!(Instant::now() < deadline, "the page set never converged");
        store.refresh(Depth::Stamps).unwrap();
        drain(&mut signalled);
        std::thread::sleep(Duration::from_millis(50));
    }
    store.refresh(Depth::Stamps).unwrap();
    drain(&mut signalled);
    signalled
}

#[test]
fn m1_a_page_deleted_while_the_app_was_closed_is_not_listed_after_reopen() {
    let _guard = crate::integration_lock();
    let root = graph(&[
        ("pages/Keep.md", b"- keep\n"),
        ("pages/Gone.md", b"- gone\n"),
    ]);
    let store = open(&root);
    assert!(has(&store, "Gone", "existing"));
    store.close();
    drop(store);
    fs::remove_file(root.join("pages/Gone.md")).unwrap();
    let store = open(&root);
    assert!(has(&store, "Keep", "existing"));
    assert!(
        !has(&store, "Gone", "existing"),
        "deleted-while-closed page still listed"
    );
    assert!(!names(&store).iter().any(|(n, _)| n == "Gone"));
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn m2_an_unreadable_page_repaired_outside_appears_and_signals() {
    let _guard = crate::integration_lock();
    // Latin-1 byte in the preamble: undecodable, effective name unknown.
    let root = graph(&[(
        "pages/Other.md",
        b"title:: Target\ntags:: caf\xe9\n\n- body\n",
    )]);
    let store = open(&root);
    let sub = store.subscribe();
    assert!(store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == "pages/Other.md"));
    assert!(!has(&store, "Target", "existing"));
    fs::write(
        root.join("pages/Other.md"),
        "title:: Target\ntags:: cafe\n\n- body\n",
    )
    .unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| has(s, "Target", "existing"));
    assert!(
        signalled,
        "repairing an unreadable page published no inventoryChanged"
    );
    assert!(store.whole_graph().unwrap().unreadable_files().is_empty());
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_outside_rename_moves_the_name_and_signals() {
    let _guard = crate::integration_lock();
    let root = graph(&[("pages/Old.md", b"- body\n")]);
    let store = open(&root);
    let sub = store.subscribe();
    fs::rename(root.join("pages/Old.md"), root.join("pages/New.md")).unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| {
        has(s, "New", "existing") && !has(s, "Old", "existing")
    });
    assert!(signalled, "an outside rename published no inventoryChanged");
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_page_delivered_by_sync_while_open_is_listed_and_signals() {
    let _guard = crate::integration_lock();
    let root = graph(&[("pages/A.md", b"- a\n")]);
    let store = open(&root);
    let sub = store.subscribe();
    fs::write(root.join("pages/Synced.md"), "- from another device\n").unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| has(s, "Synced", "existing"));
    assert!(signalled, "a synced-in page published no inventoryChanged");
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_outside_deletion_while_open_unlists_and_signals() {
    let _guard = crate::integration_lock();
    let root = graph(&[("pages/A.md", b"- a\n"), ("pages/B.md", b"- b\n")]);
    let store = open(&root);
    let sub = store.subscribe();
    fs::remove_file(root.join("pages/B.md")).unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| !has(s, "B", "existing"));
    assert!(
        signalled,
        "an outside deletion published no inventoryChanged"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_alias_only_edit_changes_the_inventory_and_signals() {
    let _guard = crate::integration_lock();
    let root = graph(&[("pages/A.md", b"- a\n")]);
    let store = open(&root);
    let sub = store.subscribe();
    fs::write(root.join("pages/A.md"), "alias:: Nick\n\n- a\n").unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| has(s, "Nick", "alias"));
    assert!(
        signalled,
        "an alias-only edit published no inventoryChanged"
    );
    fs::write(root.join("pages/A.md"), "- a\n").unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| !has(s, "Nick", "alias"));
    assert!(signalled, "removing an alias published no inventoryChanged");
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_namespace_only_reference_edit_changes_the_inventory_and_signals() {
    let _guard = crate::integration_lock();
    let root = graph(&[("pages/A.md", b"- a\n")]);
    let store = open(&root);
    let sub = store.subscribe();
    fs::write(root.join("pages/A.md"), "- a [[ns/child]]\n").unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| has(s, "ns/child", "absent"));
    assert!(
        signalled,
        "a namespace-only reference published no inventoryChanged"
    );
    fs::write(root.join("pages/A.md"), "- a\n").unwrap();
    let signalled = inventory_signalled(&store, &sub, |s| !has(s, "ns/child", "absent"));
    assert!(
        signalled,
        "dropping a namespace-only reference published no inventoryChanged"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
mod permissions {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    fn running_as_root() -> bool {
        // A root user reads mode-000 files, so the precondition cannot be built.
        fs::File::open("/proc/self/status")
            .ok()
            .and_then(|_| fs::read_to_string("/proc/self/status").ok())
            .is_some_and(|s| {
                s.lines()
                    .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(2) == Some("0"))
            })
    }

    // GH #543 retries I/O failures even when a repair changes no content stamp.
    #[test]
    fn a_permission_denied_page_made_readable_again_appears_and_signals() {
        if running_as_root() {
            return;
        }
        let _guard = crate::integration_lock();
        let root = graph(&[("pages/Locked.md", b"- body\n"), ("pages/A.md", b"- a\n")]);
        chmod(&root.join("pages/Locked.md"), 0o000);
        let store = open(&root);
        let sub = store.subscribe();
        assert!(store
            .whole_graph()
            .unwrap()
            .unreadable_files()
            .iter()
            .any(|(id, _)| id.as_str() == "pages/Locked.md"));
        chmod(&root.join("pages/Locked.md"), 0o644);
        let signalled = inventory_signalled(&store, &sub, |s| {
            s.whole_graph().unwrap().unreadable_files().is_empty()
        });
        assert!(
            signalled,
            "making a page readable again published no inventoryChanged"
        );
        store.close();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_permission_denied_page_made_readable_again_is_seen_by_the_live_watcher() {
        if running_as_root() {
            return;
        }
        let _guard = crate::integration_lock();
        let root = graph(&[("pages/Locked.md", b"- body\n"), ("pages/A.md", b"- a\n")]);
        chmod(&root.join("pages/Locked.md"), 0o000);
        let store = Store::open(
            &root,
            OpenOptions {
                watch: WatchMode::Notify,
                ..Default::default()
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let sub = store.subscribe();
        std::thread::sleep(Duration::from_millis(300));
        chmod(&root.join("pages/Locked.md"), 0o644);
        // No refresh(): only the watcher may notice.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut signalled = false;
        let cleared = |s: &Store| s.whole_graph().unwrap().unreadable_files().is_empty();
        while Instant::now() < deadline && !cleared(&store) {
            while let Some(change) = sub.try_recv().unwrap() {
                signalled |= serde_json::to_value(change).unwrap()["inventoryChanged"] == true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let repaired = cleared(&store);
        while let Some(change) = sub.try_recv().unwrap() {
            signalled |= serde_json::to_value(change).unwrap()["inventoryChanged"] == true;
        }
        store.close();
        let _ = fs::remove_dir_all(root);
        assert!(
            repaired,
            "the live watcher never noticed a page that became readable (signalled={signalled})"
        );
        assert!(
            signalled,
            "the live watcher repaired the unreadable list without inventoryChanged"
        );
    }
}
