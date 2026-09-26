use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use tine_store::LoadError;
use tine_store::{Area, Change, ChangeKind, OpenOptions, Origin, Store, Subscription, WatchMode};

struct Fixture {
    root: PathBuf,
    store: Store,
    subscription: Subscription,
}

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        Self::with_mode(name, files, WatchMode::Poll)
    }

    fn with_mode(name: &str, files: &[(&str, &str)], mode: WatchMode) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-store-watch-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        for (rel, text) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let store = Store::open(
            &root,
            OpenOptions {
                approved_external_assets: None,
                watch: mode,
            },
        )
        .unwrap()
        .0;
        store.whole_graph().unwrap();
        let subscription = store.subscribe();
        Self {
            root,
            store,
            subscription,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Atomic replace (temp outside the watched areas, then rename): the poller
    /// runs concurrently and would otherwise observe a truncated or
    /// half-stamped file and publish an honest extra change.
    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let temp = self.root.join(format!(".write-{}", rel.replace('/', "_")));
        std::fs::write(&temp, text).unwrap();
        std::fs::rename(temp, path).unwrap();
    }

    fn changes(&self) -> Vec<Change> {
        self.store.scan_refresh().unwrap();
        let mut changes = Vec::new();
        while let Some(change) = self.subscription.try_recv().unwrap() {
            changes.push(change);
        }
        changes
    }

    fn file_changes(&self) -> Vec<(String, ChangeKind)> {
        let changes = self.changes();
        assert!(changes
            .iter()
            .all(|change| change.origin == Origin::External));
        changes
            .into_iter()
            .flat_map(|change| change.files)
            .map(|(id, kind, _)| (id.as_str().to_owned(), kind))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.store.close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn one(fixture: &Fixture, rel: &str, kind: ChangeKind) {
    assert_eq!(fixture.file_changes(), vec![(rel.into(), kind)]);
}

#[test]
fn atomic_page_save_temp_events_stay_incremental() {
    let graph = Fixture::new("atomic", &[]);
    graph.write("pages/.One.md.123.7.tmp", "- new\n");
    std::fs::rename(
        graph.path("pages/.One.md.123.7.tmp"),
        graph.path("pages/One.md"),
    )
    .unwrap();
    one(&graph, "pages/One.md", ChangeKind::Created);
}

#[test]
fn unknown_path_event_requests_full_scan_only_for_its_owner() {
    let graph = Fixture::new("directory", &[]);
    graph.write("pages/new-directory/New.md", "- new\n");
    one(&graph, "pages/new-directory/New.md", ChangeKind::Created);
}

#[test]
fn pending_paths_are_dispatched_only_to_the_owning_graph() {
    let a = Fixture::new("owner-a", &[]);
    let b = Fixture::new("owner-b", &[]);
    a.write("pages/One.md", "- one\n");
    b.write("journals/2026_07_10.md", "- day\n");
    one(&a, "pages/One.md", ChangeKind::Created);
    one(&b, "journals/2026_07_10.md", ChangeKind::Created);
}

#[test]
fn collect_page_files_descends_subdirectories() {
    let graph = Fixture::new("nested-list", &[]);
    graph.write("pages/top.md", "- top\n");
    graph.write("pages/Archive/mid.org", "* mid\n");
    graph.write("pages/Archive/Deep/Deeper/deep.md", "- deep\n");
    graph.write("pages/Archive/notes.txt", "ignored\n");
    graph.write("pages/.hidden/skip.md", "- skip\n");
    let mut changes = graph.file_changes();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        changes,
        vec![
            (
                "pages/Archive/Deep/Deeper/deep.md".into(),
                ChangeKind::Created
            ),
            ("pages/Archive/mid.org".into(), ChangeKind::Created),
            ("pages/top.md".into(), ChangeKind::Created),
        ]
    );
}

#[cfg(unix)]
#[test]
fn collect_page_files_does_not_follow_page_symlinks() {
    use std::os::unix::fs::symlink;
    let graph = Fixture::new("page-link", &[]);
    let outside = graph.root.with_extension("outside.md");
    std::fs::write(&outside, "- secret\n").unwrap();
    symlink(&outside, graph.path("pages/secret.md")).unwrap();
    assert!(graph.file_changes().is_empty());
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn incremental_create_top_level_file_matches_full_diff() {
    let graph = Fixture::new("create-top", &[("pages/Seed.md", "- seed\n")]);
    graph.write("pages/New.md", "- new\n");
    one(&graph, "pages/New.md", ChangeKind::Created);
}

#[test]
fn incremental_create_is_identified_as_inventory_change() {
    let graph = Fixture::new("inventory", &[("pages/Seed.md", "- seed\n")]);
    graph.write("pages/New.md", "- new\n");
    one(&graph, "pages/New.md", ChangeKind::Created);
    assert!(graph
        .store
        .whole_graph()
        .unwrap()
        .inventory()
        .0
        .iter()
        .any(|entry| entry.name == "New"));
}

#[test]
fn incremental_create_nested_file_matches_full_diff() {
    let graph = Fixture::new("create-nested", &[("pages/Seed.md", "- seed\n")]);
    graph.write("pages/sub/New.md", "- nested\n");
    one(&graph, "pages/sub/New.md", ChangeKind::Created);
}

#[test]
fn incremental_modify_len_change_matches_full_diff() {
    let graph = Fixture::new("modify-len", &[("pages/Edit.md", "- one\n")]);
    graph.write("pages/Edit.md", "- one\n- two\n");
    one(&graph, "pages/Edit.md", ChangeKind::Modified);
}

#[test]
fn incremental_modify_same_len_mtime_change_matches_full_diff() {
    let graph = Fixture::new("modify-same-len", &[("pages/Edit.md", "- alpha\n")]);
    // Setting times needs write access on Windows (FILE_WRITE_ATTRIBUTES).
    let file = std::fs::File::options()
        .write(true)
        .open(graph.path("pages/Edit.md"))
        .unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(5))
        .unwrap();
    graph.store.scan_refresh().unwrap();
    while graph.subscription.try_recv().unwrap().is_some() {}
    graph.write("pages/Edit.md", "- bravo\n");
    one(&graph, "pages/Edit.md", ChangeKind::Modified);
}

#[test]
fn explicit_event_reconciles_even_when_snapshot_metadata_is_equal() {
    let graph = Fixture::with_mode(
        "explicit-same-meta",
        &[("pages/Edit.md", "- alpha\n")],
        WatchMode::Notify,
    );
    std::thread::sleep(Duration::from_millis(100));
    let previous = std::fs::metadata(graph.path("pages/Edit.md"))
        .unwrap()
        .modified()
        .unwrap();
    graph.write("pages/Edit.md", "- bravo\n");
    std::fs::File::options()
        .write(true)
        .open(graph.path("pages/Edit.md"))
        .unwrap()
        .set_modified(previous)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let change = loop {
        if let Some(change) = graph.subscription.try_recv().unwrap() {
            break change;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "notify did not reconcile the explicit path"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(change.files.len(), 1);
    assert_eq!(change.files[0].0.as_str(), "pages/Edit.md");
    assert_eq!(change.files[0].1, ChangeKind::Modified);
}

#[test]
fn incremental_remove_top_level_file_matches_full_diff() {
    let graph = Fixture::new("remove-top", &[("pages/Delete.md", "- old\n")]);
    std::fs::remove_file(graph.path("pages/Delete.md")).unwrap();
    one(&graph, "pages/Delete.md", ChangeKind::Removed);
}

#[test]
fn incremental_remove_nested_file_matches_full_diff() {
    let graph = Fixture::new("remove-nested", &[("pages/sub/Delete.md", "- old\n")]);
    std::fs::remove_file(graph.path("pages/sub/Delete.md")).unwrap();
    one(&graph, "pages/sub/Delete.md", ChangeKind::Removed);
}

#[test]
fn incremental_rename_within_pages_matches_full_diff() {
    let graph = Fixture::new("rename", &[("pages/Old.md", "- old\n")]);
    std::fs::rename(graph.path("pages/Old.md"), graph.path("pages/New.md")).unwrap();
    let mut changes = graph.file_changes();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        changes,
        vec![
            ("pages/New.md".into(), ChangeKind::Created),
            ("pages/Old.md".into(), ChangeKind::Removed),
        ]
    );
}

#[test]
fn incremental_rename_across_tree_matches_full_diff() {
    let graph = Fixture::new("rename-area", &[("pages/Old.md", "- old\n")]);
    std::fs::rename(
        graph.path("pages/Old.md"),
        graph.path("journals/2026_07_10.md"),
    )
    .unwrap();
    let mut changes = graph.file_changes();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        changes,
        vec![
            ("journals/2026_07_10.md".into(), ChangeKind::Created),
            ("pages/Old.md".into(), ChangeKind::Removed),
        ]
    );
}

#[test]
fn incremental_burst_union_matches_full_diff() {
    let graph = Fixture::new(
        "burst",
        &[
            ("pages/Edit.md", "- old\n"),
            ("pages/Delete.md", "- gone\n"),
        ],
    );
    graph.write("pages/Edit.md", "- much longer new content\n");
    graph.write("pages/New.md", "- new\n");
    std::fs::remove_file(graph.path("pages/Delete.md")).unwrap();
    let mut changes = graph.file_changes();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        changes,
        vec![
            ("pages/Delete.md".into(), ChangeKind::Removed),
            ("pages/Edit.md".into(), ChangeKind::Modified),
            ("pages/New.md".into(), ChangeKind::Created),
        ]
    );
}

#[test]
fn reconcile_pending_need_full_uses_full_scan_branch() {
    let graph = Fixture::new("full-branch", &[]);
    graph.write("pages/new-folder/Nested.md", "- found\n");
    one(&graph, "pages/new-folder/Nested.md", ChangeKind::Created);
}

#[test]
fn own_commit_publishes_once_without_external_echo() {
    let graph = Fixture::with_mode("own", &[], WatchMode::Notify);
    std::thread::sleep(Duration::from_millis(100));
    let id = graph.store.file_id(Area::Pages, "Own.md").unwrap();
    let mut tx = graph.store.transaction();
    tx.create(&id, tine_store::Content::Bytes(b"- own\n".to_vec()));
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    let first = graph.subscription.recv().unwrap();
    assert_eq!(first.origin, Origin::Own);
    assert_eq!(first.files.len(), 1);
    assert_eq!(first.files[0].1, ChangeKind::Created);
    assert!(graph.file_changes().is_empty());
    std::thread::sleep(Duration::from_millis(700));
    assert!(graph.subscription.try_recv().unwrap().is_none());
}

#[test]
fn own_config_commit_reloads_format_and_sets_config_changed() {
    let graph = Fixture::new("own-config", &[]);
    std::fs::create_dir_all(graph.path("logseq")).unwrap();
    let id = graph.store.file_id(Area::Meta, "config.edn").unwrap();
    let mut tx = graph.store.transaction();
    tx.create(
        &id,
        tine_store::Content::Bytes(b"{:preferred-format :org}\n".to_vec()),
    );
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    let change = graph.subscription.recv().unwrap();
    assert_eq!(change.origin, Origin::Own);
    assert!(change.config_changed);
    assert_eq!(graph.store.config().preferred_format.ext(), "org");
    assert!(graph
        .store
        .journal_id(tine_store::Day(20260925))
        .as_str()
        .ends_with(".org"));
}

#[test]
fn external_touch_publishes_touched() {
    let graph = Fixture::new("touch", &[("pages/Edit.md", "- same\n")]);
    std::fs::File::options()
        .write(true)
        .open(graph.path("pages/Edit.md"))
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    one(&graph, "pages/Edit.md", ChangeKind::Touched);
}

#[test]
fn config_edit_via_scan_refresh_sets_config_changed() {
    let graph = Fixture::new("config", &[]);
    graph.write("logseq/config.edn", "{:preferred-format :org}\n");
    let changes = graph.changes();
    assert_eq!(changes.len(), 1);
    assert!(changes[0].config_changed);
    assert_eq!(changes[0].origin, Origin::External);
    assert_eq!(graph.store.config().preferred_format.ext(), "org");
}

#[cfg(unix)]
#[test]
fn scan_refresh_refuses_an_unsafe_configured_page_directory() {
    use std::os::unix::fs::symlink;
    let graph = Fixture::new("unsafe-config", &[]);
    let outside = graph.root.with_extension("outside");
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, graph.path("linked-pages")).unwrap();
    graph.write("logseq/config.edn", "{:pages-directory \"linked-pages\"}\n");
    assert!(matches!(
        graph.store.scan_refresh(),
        Err(LoadError::Failed { .. })
    ));
    assert_eq!(graph.store.config().pages_dir, "pages");
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn close_ends_blocked_recv_with_closed() {
    let graph = Fixture::new("closed-subscription", &[]);
    let subscription = graph.store.subscribe();
    let waiting = std::thread::spawn(move || subscription.recv());
    graph.store.close();
    assert!(waiting.join().unwrap().is_err());
}

#[test]
fn second_subscribe_ends_first() {
    let graph = Fixture::new("replace-subscription", &[]);
    let first = graph.store.subscribe();
    let second = graph.store.subscribe();
    assert!(first.recv().is_err());
    assert!(second.try_recv().unwrap().is_none());
}

#[test]
fn scan_refresh_keeps_journal_day_index_current() {
    let graph = Fixture::new("day-index", &[]);
    graph.write("journals/2026_09_25.md", "- day\n");
    graph.changes();
    assert_eq!(
        graph.store.journal_id(tine_store::Day(20260925)).as_str(),
        "journals/2026_09_25.md"
    );
}

#[test]
fn external_changes_during_initial_load_publish_after_ready() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-watch-loading-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("journals")).unwrap();
    std::fs::write(root.join("pages/Existing.md"), "- old\n").unwrap();
    std::fs::write(root.join(".tine-test-pause-load"), "").unwrap();
    let store = Store::open(
        &root,
        OpenOptions {
            approved_external_assets: None,
            watch: WatchMode::Notify,
        },
    )
    .unwrap()
    .0;
    let subscription = store.subscribe();
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(root.join("pages/Existing.md"), "- changed content\n").unwrap();
    std::fs::write(root.join("pages/Created.md"), "- new\n").unwrap();
    std::fs::remove_file(root.join(".tine-test-pause-load")).unwrap();
    store.whole_graph().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut found = Vec::new();
    while std::time::Instant::now() < deadline && found.len() < 2 {
        if let Some(change) = subscription.try_recv().unwrap() {
            found.extend(
                change
                    .files
                    .into_iter()
                    .map(|(id, kind, _)| (id.as_str().to_owned(), kind)),
            );
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        found,
        vec![
            ("pages/Created.md".into(), ChangeKind::Created),
            ("pages/Existing.md".into(), ChangeKind::Modified),
        ]
    );
    store.close();
    std::fs::remove_dir_all(root).unwrap();
}
