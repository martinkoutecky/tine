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
    fs::write(root.join(".tine-test-pause-load"), "").unwrap();
    let options = OpenOptions {
        watch: WatchMode::Poll,
        ..Default::default()
    };
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
    let mut tx = store.transaction(Some(crate::EditKind::ReplacePage));
    tx.save_page(
        &[crate::EditKind::ReplacePage],
        &PageId::from("pages/New.md"),
        SaveBase::Existing(second),
        &doc("New", "third"),
    );
    assert!(matches!(
        tx.commit(),
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
