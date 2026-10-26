//! BqbeQ (D-20261019-B6): an own save while the graph loads, and the
//! first complete view after it. The save finishes without the O(P + B)
//! whole-graph build; the view holds the page exactly once with the saved
//! bytes whether the launch parse saw the file, missed it, or read an
//! earlier version; a page hold or an edit during the parse keeps the
//! launch's own install.
use super::*;
use std::time::Duration;
use tine_core::model::{BlockDto, PageDto};

fn old() -> SystemTime {
    SystemTime::now() - Duration::from_secs(3600)
}

/// `pages/A.md` .. `pages/E.md`, outside the racy window.
fn graph() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    fs::write(
        root.path().join("logseq/config.edn"),
        "{:preferred-format :markdown}\n",
    )
    .unwrap();
    for name in ["A", "B", "C", "D", "E"] {
        let path = root.path().join(format!("pages/{name}.md"));
        fs::write(&path, format!("- old {name}\n")).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old())
            .unwrap();
    }
    root
}

/// A store whose load worker waits until [`go`] (the test pause file).
fn held_open(root: &Path) -> Arc<Store> {
    held_open_with(
        root,
        OpenOptions {
            watch: WatchMode::Poll,
            ..Default::default()
        },
    )
}

fn held_open_with(root: &Path, options: OpenOptions) -> Arc<Store> {
    fs::write(root.join(".tine-test-pause-load"), "").unwrap();
    Arc::new(Store::open(root, options).unwrap().0)
}

fn go(root: &Path) {
    fs::remove_file(root.join(".tine-test-pause-load")).unwrap();
}

fn pause() -> TestPause {
    Arc::new((Mutex::new((false, false)), Condvar::new()))
}

fn wait_hook(pause: &TestPause) {
    let (state, ready) = &**pause;
    let mut state = state.lock().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !state.0 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "test pause hook was not reached");
        state = ready.wait_timeout(state, remaining).unwrap().0;
    }
}

fn release_hook(pause: &TestPause) {
    let (state, ready) = &**pause;
    state.lock().unwrap().1 = true;
    ready.notify_all();
}

fn doc(name: &str, text: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            raw: text.into(),
            ..Default::default()
        }],
        rev: None,
        format: Default::default(),
        read_only: false,
        guide: false,
    }
}

fn create(store: &Store, name: &str, text: &str) -> FileRev {
    let id = PageId::from(format!("pages/{name}.md").as_str());
    match store.save(
        crate::EditKind::ReplacePage,
        &id,
        SaveBase::CreateNew,
        &doc(name, text),
    ) {
        SaveOutcome::Saved(rev) => rev,
        other => panic!("create refused: {other:?}"),
    }
}

fn edit(store: &Store, name: &str, base: FileRev, text: &str) -> FileRev {
    let id = PageId::from(format!("pages/{name}.md").as_str());
    match store.save(
        crate::EditKind::ReplacePage,
        &id,
        SaveBase::Existing(base),
        &doc(name, text),
    ) {
        SaveOutcome::Saved(rev) => rev,
        other => panic!("edit refused: {other:?}"),
    }
}

/// No whole-graph build ran outside the launch pass.
fn on_demand_builds(store: &Store) -> u64 {
    store.diagnostics()["onDemandBuilds"]["count"]
        .as_u64()
        .unwrap()
}

/// The launch passes that did not install (empty: the first one did).
fn declined_passes(store: &Store) -> Vec<String> {
    let outcomes = &store.diagnostics()["launch"]["loadPassOutcomes"];
    serde_json::from_value(outcomes.clone()).unwrap()
}

/// The first complete view holds `name` once, with `text`.
fn assert_once(store: &Store, name: &str, text: &str) {
    let view = store.whole_graph().unwrap();
    let corpus = view.corpus();
    let found: Vec<_> = (corpus.pages.iter())
        .filter(|page| page.name == name)
        .collect();
    assert_eq!(found.len(), 1, "{name} is in the view exactly once");
    assert!(
        found[0].document.roots[0].raw().contains(text),
        "{name} holds the saved bytes"
    );
    assert!(matches!(
        view.resolve(name, false),
        Resolved::Existing { .. }
    ));
    assert_eq!(
        view.inventory(crate::InventoryScope::All)
            .0
            .iter()
            .filter(|entry| entry.name == name)
            .count(),
        1,
        "{name} is listed once"
    );
}

#[test]
fn a_page_created_while_loading_saves_without_building_the_graph() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    let rev = create(&store, "New", "fresh text");
    assert!(
        !store.graph.cache_built() && on_demand_builds(&store) == 0,
        "BqbeQ: a create during loading built the whole graph cache on the saving thread \
         (9.2 s on the realistic 10k graph); its publication waits for the first view instead"
    );
    // The page reads at once, before the graph is ready.
    let read = store.page(&PageId::from("pages/New.md")).unwrap();
    assert_eq!(read.rev, rev);
    go(root.path());
    assert_once(&store, "New", "fresh text");
    assert_eq!(declined_passes(&store), Vec::<String>::new());
    assert_eq!(on_demand_builds(&store), 0);
    assert_eq!(store.graph.warm_passes.load(Ordering::Relaxed), 1);
    // The first complete view comes first; the save's own change follows.
    let ready = feed.recv().unwrap();
    assert_eq!(ready.origin, Origin::External);
    assert!(
        !(ready.files.iter()).any(|(id, _, _)| id.as_str() == "pages/New.md"),
        "the own save is not announced as an outside write"
    );
    let own = feed.recv().unwrap();
    assert_eq!(own.origin, Origin::Own);
    assert!(own.graph_rev > ready.graph_rev);
    assert!(own
        .files
        .iter()
        .any(|(id, kind, _)| id.as_str() == "pages/New.md" && *kind == ChangeKind::Created));
    store.close();
}

#[test]
fn a_page_created_after_the_parse_listed_the_files_is_in_the_first_view() {
    let root = graph();
    let store = held_open(root.path());
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    create(&store, "New", "made mid-parse");
    assert_eq!(on_demand_builds(&store), 0, "the save built the graph");
    release_hook(&listed);
    assert_once(&store, "New", "made mid-parse");
    assert_eq!(
        declined_passes(&store),
        Vec::<String>::new(),
        "the launch pass takes the save in instead of a second whole-graph build"
    );
    assert_eq!(on_demand_builds(&store), 0);
    store.close();
}

#[test]
fn a_page_saved_after_the_parse_read_it_holds_the_last_save() {
    let root = graph();
    let store = held_open(root.path());
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    // Every page, including the one the pass already read.
    for name in ["A", "B", "C", "D", "E"] {
        let read = store
            .page(&PageId::from(format!("pages/{name}.md").as_str()))
            .unwrap();
        edit(&store, name, read.rev, &format!("new {name}"));
    }
    assert_eq!(on_demand_builds(&store), 0, "the save built the graph");
    release_hook(&listed);
    for name in ["A", "B", "C", "D", "E"] {
        assert_once(&store, name, &format!("new {name}"));
    }
    assert_eq!(declined_passes(&store), Vec::<String>::new());
    assert_eq!(on_demand_builds(&store), 0);
    store.close();
}

#[test]
fn a_page_created_then_edited_before_loading_ends_holds_the_edit() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    let first = create(&store, "New", "first");
    let second = edit(&store, "New", first, "second");
    assert_eq!(on_demand_builds(&store), 0, "the saves built the graph");
    go(root.path());
    assert_once(&store, "New", "second");
    assert_eq!(on_demand_builds(&store), 0);
    let kinds: Vec<_> = std::iter::from_fn(|| feed.try_recv().unwrap())
        .map(|change| {
            let kind = (change.files.iter())
                .find(|(id, _, _)| id.as_str() == "pages/New.md")
                .map(|(_, kind, _)| *kind);
            (change.origin, kind)
        })
        .collect();
    assert_eq!(kinds[0].0, Origin::External, "the first view comes first");
    assert_eq!(
        kinds[1..],
        [
            (Origin::Own, Some(ChangeKind::Created)),
            (Origin::Own, Some(ChangeKind::Modified))
        ],
        "each deferred save is its own change, in save order"
    );
    // After loading, an edit publishes as for any page.
    let mut tx = store.page_plan(Some(crate::EditKind::ReplacePage));
    tx.save_page(
        &[crate::EditKind::ReplacePage],
        &PageId::from("pages/New.md"),
        SaveBase::Existing(second),
        &doc("New", "third"),
    );
    assert!(matches!(
        tx.commit_internal(),
        crate::TxOutcome::Committed {
            change: Some(_),
            ..
        }
    ));
    assert_once(&store, "New", "third");
    store.close();
}

#[test]
fn a_page_hold_during_the_parse_keeps_the_launch_install() {
    let root = graph();
    let store = held_open(root.path());
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    {
        let _writer = store.writer.lock().unwrap();
        store.graph.hold_unhosted("pages/Later.md");
        store.publish_retired();
    }
    release_hook(&listed);
    store.whole_graph().unwrap();
    assert_eq!(
        declined_passes(&store),
        Vec::<String>::new(),
        "a page open during the launch parse discarded it: the app then built the graph \
         a second time before Ready (16.9 s instead of 9.6 s on the realistic 10k graph)"
    );
    assert_eq!(on_demand_builds(&store), 0);
    store.close();
}

#[test]
fn an_unexplained_cache_move_during_the_parse_still_rebuilds_correctly() {
    let root = graph();
    let store = held_open(root.path());
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    create(&store, "New", "kept");
    // A move the pass cannot account for path by path (a discarded cache).
    store.graph.invalidate_cache();
    release_hook(&listed);
    assert_once(&store, "New", "kept");
    assert_eq!(declined_passes(&store), vec!["install_declined".to_owned()]);
    store.close();
}

#[test]
fn a_save_deferred_before_a_launch_checkpoint_loads_declines_the_checkpoint() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("graph.bin");
    let options = || OpenOptions {
        watch: WatchMode::Poll,
        launch_checkpoint: Some(checkpoint.clone()),
        ..Default::default()
    };
    let store = Store::open(root.path(), options()).unwrap().0;
    store.whole_graph_reconciled().unwrap();
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(super::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    drop(store);
    fs::write(root.path().join(".tine-test-pause-load"), "").unwrap();
    let store = Store::open(root.path(), options()).unwrap().0;
    create(&store, "New", "before the checkpoint");
    go(root.path());
    // The checkpoint lacks the page and the watcher's baseline carries the
    // save's own stamp, so serving it would hide the page until restart.
    assert_once(&store, "New", "before the checkpoint");
    assert_eq!(
        store.diagnostics()["checkpoint"]["load"]["outcome"],
        "deferred",
        "the diagnostics name why the checkpoint was declined"
    );
    store.close();
}

#[test]
fn early_file_answers_stand_down_while_a_launch_checkpoint_is_served() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("graph.bin");
    let options = || OpenOptions {
        watch: WatchMode::Poll,
        launch_checkpoint: Some(checkpoint.clone()),
        ..Default::default()
    };
    let store = Store::open(root.path(), options()).unwrap().0;
    store.whole_graph_reconciled().unwrap();
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(super::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    drop(store);
    let held = root.path().join(".tine-test-pause-launch-diff");
    fs::write(&held, "").unwrap();
    fs::write(root.path().join(".tine-test-pause-load"), "").unwrap();
    let store = Store::open(root.path(), options()).unwrap().0;
    assert!(store.first_view_pending(), "no view before the checkpoint");
    go(root.path());
    // The served checkpoint answers at once, before the launch diff.
    store.whole_graph().unwrap();
    assert!(matches!(
        *store.load.status.lock().unwrap(),
        LoadStatus::Loading
    ));
    assert!(
        !store.first_view_pending(),
        "the journal-day and named-template file lookups must not run beside a served checkpoint"
    );
    let today = tine_core::date::JournalDate::today().title();
    assert!(store
        .page_named(&today, PageKind::Journal)
        .unwrap()
        .is_none());
    fs::remove_file(&held).unwrap();
    store.close();
}

// Review r1 (subagent-tasks/notes/review-og-loading-own-save.md): R1 and the
// F3 scenarios the contract names.

fn feed_files(feed: &crate::Subscription) -> Vec<(Origin, Vec<(String, ChangeKind)>)> {
    std::iter::from_fn(|| feed.try_recv().unwrap())
        .map(|change| {
            let files = (change.files.iter())
                .map(|(id, kind, _)| (id.as_str().to_owned(), *kind))
                .collect();
            (change.origin, files)
        })
        .collect()
}

fn names_c(changes: &[(Origin, Vec<(String, ChangeKind)>)]) -> Vec<Origin> {
    (changes.iter())
        .filter(|(_, files)| files.iter().any(|(id, _)| id == "pages/C.md"))
        .map(|(origin, _)| *origin)
        .collect()
}

/// R1: a config write during loading publishes a view before Ready; the
/// save deferred before it is published first, so C's own changes keep
/// save order, and neither is announced as an outside write.
#[test]
fn a_config_write_during_loading_keeps_own_changes_in_save_order() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    let read = store.page(&PageId::from("pages/C.md")).unwrap();
    let first = edit(&store, "C", read.rev, "own C");
    let cfg = FileId::from("logseq/config.edn".to_owned());
    let (_, config_rev) = store.read(&cfg, None).unwrap();
    let mut tx = store.page_plan(None);
    tx.replace(
        &cfg,
        config_rev,
        b"{:preferred-format :markdown :x 1}\n".to_vec(),
    );
    assert!(matches!(
        tx.commit_internal(),
        crate::TxOutcome::Committed { .. }
    ));
    let second = edit(&store, "C", first.clone(), "own C two");
    release_hook(&listed);
    assert_once(&store, "C", "own C two");
    std::thread::sleep(Duration::from_millis(200));
    let c: Vec<_> = std::iter::from_fn(|| feed.try_recv().unwrap())
        .filter_map(|change| {
            let rev = (change.files.iter())
                .find(|(id, _, _)| id.as_str() == "pages/C.md")
                .map(|(_, _, rev)| rev.clone())?;
            Some((change.origin, rev))
        })
        .collect();
    assert_eq!(
        c,
        [(Origin::Own, Some(first)), (Origin::Own, Some(second))],
        "C's own changes in save order, none announced as outside"
    );
    store.close();
}

/// F3: a trash during the parse, after it read the page, is gone from the
/// first view, and the own delete is not announced as an outside write.
#[test]
fn a_trash_after_the_parse_read_the_page_is_gone_from_the_first_view() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    for name in ["A", "B", "C", "D", "E"] {
        let read = store
            .page(&PageId::from(format!("pages/{name}.md").as_str()))
            .unwrap();
        let mut tx = store.page_plan(Some(crate::EditKind::DeletePage));
        tx.trash(
            &FileId::from(format!("pages/{name}.md")),
            read.rev,
            crate::TrashIf::Any,
        );
        assert!(matches!(
            tx.commit_internal(),
            crate::TxOutcome::Committed { .. }
        ));
    }
    release_hook(&listed);
    let view = store.whole_graph().unwrap();
    let names: Vec<_> = view.corpus().pages.iter().map(|p| p.name.clone()).collect();
    for name in ["A", "B", "C", "D", "E"] {
        assert!(
            !names.iter().any(|n| n.eq_ignore_ascii_case(name)),
            "{name} resurrected: {names:?}"
        );
    }
    let got = feed_files(&feed);
    assert!(
        !got.iter().any(
            |(o, f)| *o == Origin::External && f.iter().any(|(id, _)| id.starts_with("pages/"))
        ),
        "own delete announced as outside: {got:?}"
    );
    store.close();
}

/// F3: a rename during the parse, after it read the page: the first view
/// holds the new name once and not the old one.
#[test]
fn a_rename_after_the_parse_read_the_page_holds_the_new_name_once() {
    let root = graph();
    let store = held_open(root.path());
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    for name in ["A", "B", "C", "D", "E"] {
        let read = store
            .page(&PageId::from(format!("pages/{name}.md").as_str()))
            .unwrap();
        let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
        tx.move_file(
            &FileId::from(format!("pages/{name}.md")),
            read.rev,
            &FileId::from(format!("pages/{name}2.md")),
            None,
        );
        assert!(matches!(
            tx.commit_internal(),
            crate::TxOutcome::Committed { .. }
        ));
    }
    release_hook(&listed);
    let view = store.whole_graph().unwrap();
    let names: Vec<_> = view
        .corpus()
        .pages
        .iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    for name in ["a", "b", "c", "d", "e"] {
        assert!(
            !names.contains(&name.to_string()),
            "{name} resurrected: {names:?}"
        );
        assert_eq!(
            names.iter().filter(|n| **n == format!("{name}2")).count(),
            1,
            "{names:?}"
        );
    }
    store.close();
}

/// F3: an outside writer (an editor, sync delivery) after a deferred own
/// save: the outside bytes win and are announced as outside.
#[test]
fn an_outside_write_after_a_deferred_save_is_kept_and_announced() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    let listed = pause();
    *store.graph.warm_after_first_page_pause.lock().unwrap() = Some(Arc::clone(&listed));
    go(root.path());
    wait_hook(&listed);
    let read = store.page(&PageId::from("pages/C.md")).unwrap();
    edit(&store, "C", read.rev, "own C");
    std::thread::sleep(Duration::from_millis(30));
    fs::write(root.path().join("pages/C.md"), "- outside C longer bytes\n").unwrap();
    release_hook(&listed);
    assert_once(&store, "C", "outside C");
    std::thread::sleep(Duration::from_millis(200));
    let got = feed_files(&feed);
    assert!(
        names_c(&got).contains(&Origin::External),
        "the outside write was swallowed: {got:?}"
    );
    store.close();
}

/// F3: closing with saves still deferred loses nothing: the bytes are on
/// disk and the next open's first view holds them.
#[test]
fn closing_with_deferred_saves_then_reopening_keeps_them() {
    let root = graph();
    let store = held_open(root.path());
    create(&store, "New", "durable");
    store.close();
    drop(store);
    let _ = fs::remove_file(root.path().join(".tine-test-pause-load"));
    let (store, _, _) = Store::open(root.path(), Default::default()).unwrap();
    assert_once(&store, "New", "durable");
    store.close();
}

/// Hold and release `pages/A.md` while the graph loads, as a page host does
/// for a click into a block at first paint and Escape (card hB0p3s): open
/// (`hold_current`) and close (`release_hold`), each in its own writer
/// section followed by the transition's publication.
fn open_and_close_a(store: &Store) {
    store
        .graph
        .held
        .spellings()
        .spell("pages/A.md", "pages/A.md");
    for open in [true, false] {
        let _writer = store.writer.lock().unwrap();
        if open {
            store.watch.hold_current("pages/A.md".into());
        } else {
            store.watch.release_hold("pages/A.md");
        }
        store.publish_retired();
    }
}

#[test]
fn a_page_opened_and_closed_while_loading_builds_no_graph_under_the_writer() {
    let root = graph();
    let store = held_open(root.path());
    let feed = store.subscribe();
    open_and_close_a(&store);
    assert!(
        !store.graph.cache_built() && on_demand_builds(&store) == 0,
        "card hB0p3s: the release's reread published at once, building the whole graph \
         under the writer while the graph loaded (on a 10k graph a click at first paint \
         then Escape made Ready ~1 s later than a cold launch); it waits for the first view"
    );
    go(root.path());
    assert_once(&store, "A", "old A");
    assert_eq!(declined_passes(&store), Vec::<String>::new());
    assert_eq!(on_demand_builds(&store), 0);
    assert_eq!(store.graph.warm_passes.load(Ordering::Relaxed), 1);
    let ready = feed.recv().unwrap();
    assert_eq!(ready.origin, Origin::External, "the first view comes first");
    assert!(
        !store.watch.holds(&root.path().join("pages/A.md")),
        "the page is back with the watcher"
    );
    store.close();
}

#[test]
fn a_page_opened_and_closed_before_a_launch_checkpoint_loads_costs_no_decode() {
    let root = graph();
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = dir.path().join("graph.bin");
    let options = || OpenOptions {
        watch: WatchMode::Poll,
        launch_checkpoint: Some(checkpoint.clone()),
        ..Default::default()
    };
    let store = Store::open(root.path(), options()).unwrap().0;
    store.whole_graph_reconciled().unwrap();
    assert!(matches!(
        store.write_checkpoint_now(),
        Some(super::checkpoint::CheckpointWrite::Written { .. })
    ));
    store.close();
    drop(store);
    let store = held_open_with(root.path(), options());
    open_and_close_a(&store);
    go(root.path());
    assert_once(&store, "A", "old A");
    let load = &store.diagnostics()["checkpoint"]["load"];
    assert_eq!(
        load["outcome"], "deferred",
        "card hB0p3s: the release published the first generation with an on-demand \
         build under the writer, so the checkpoint was declined `published` after it \
         waited for that build"
    );
    assert_eq!(
        load["bytes"], 0,
        "card hB0p3s: a launch whose checkpoint is already declined must not read and \
         decode it (about 1 s on a 10k graph) before the cold build"
    );
    assert_eq!(on_demand_builds(&store), 0);
    assert_eq!(declined_passes(&store), Vec::<String>::new());
    assert_eq!(store.graph.warm_passes.load(Ordering::Relaxed), 1);
    store.close();
}
