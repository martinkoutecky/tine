//! R12-09 / R13-11 / R14-06: observations belong to the store that publishes them.
use super::*;
use crate::store::TestPause;
use crate::{Depth, Store};
use std::sync::Condvar;

fn fixture(paused: bool) -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("logseq")).unwrap();
    fs::write(dir.path().join("logseq/config.edn"), "{}\n").unwrap();
    for name in ["A", "B"] {
        write_page(dir.path(), name, "- original\n");
    }
    if paused {
        fs::write(dir.path().join(".tine-test-pause-load"), "").unwrap();
    }
    let store = Arc::new(
        Store::open(
            dir.path(),
            crate::OpenOptions {
                watch: WatchMode::Poll,
                ..Default::default()
            },
        )
        .unwrap()
        .0,
    );
    if !paused {
        store.whole_graph_reconciled().unwrap();
    }
    store.watch.stop_background_for_test();
    (dir, store)
}

fn write_page(root: &Path, name: &str, bytes: &str) {
    let path = root.join(format!("pages/{name}.md"));
    fs::write(&path, bytes).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
}

fn pause() -> TestPause {
    Arc::new((Mutex::new((false, false)), Condvar::new()))
}

fn wait(pause: &TestPause) {
    let (state, ready) = &**pause;
    let (state, timeout) = ready
        .wait_timeout_while(state.lock().unwrap(), Duration::from_secs(10), |s| !s.0)
        .unwrap();
    assert!(
        state.0 && !timeout.timed_out(),
        "ordering hook was not reached"
    );
}

fn release(pause: &TestPause) {
    pause.0.lock().unwrap().1 = true;
    pause.1.notify_all();
}

fn assert_indexed(store: &Store, name: &str, text: &str) {
    let graph = store.whole_graph_reconciled().unwrap();
    let corpus = graph.corpus(None, false);
    let page = corpus.pages.iter().find(|page| page.name == name).unwrap();
    assert_eq!(page.document.roots[0].raw(), text);
}

#[test]
fn a_poll_observation_retries_after_refresh() {
    for depth in [Depth::Stamps, Depth::Rebuild] {
        let (dir, store) = fixture(false);
        let core = store.watch.core_for_load();
        let paused = pause();
        *core.full_walk_pause.lock().unwrap() = Some(Arc::clone(&paused));
        let observer = Arc::clone(&core);
        let poll = std::thread::spawn(move || {
            observer.reconcile_batch(
                None,
                true,
                WatchBatch {
                    first_event_at: None,
                    reconcile_started: Instant::now(),
                    poll: true,
                    full_diff: true,
                    event_paths: 0,
                },
            )
        });
        wait(&paused);
        *core.full_walk_pause.lock().unwrap() = None;
        write_page(dir.path(), "A", "- refreshed [[Target]]\n");
        store.refresh(depth).unwrap();
        // This file is absent from both the paused walk and refresh's view.
        // Only retrying the invalidated walk can discover it in this cycle.
        write_page(dir.path(), "Late", "- delivered after refresh [[Target]]\n");
        release(&paused);
        poll.join().unwrap().unwrap();
        assert_indexed(&store, "A", "refreshed [[Target]]");
        assert_indexed(&store, "Late", "delivered after refresh [[Target]]");
        assert_eq!(
            store
                .whole_graph()
                .unwrap()
                .explicit_referrers(&["Target".into()])
                .len(),
            2
        );
        store.close();
    }
}

#[test]
fn a_closed_watch_cannot_publish_into_a_reopened_store() {
    let (dir, store) = fixture(false);
    let core = store.watch.core_for_load();
    let paused = pause();
    *core.full_walk_pause.lock().unwrap() = Some(Arc::clone(&paused));
    let observer = Arc::clone(&core);
    let poll =
        std::thread::spawn(move || observer.reconcile(None, false, false, DiffTrigger::Test));
    wait(&paused);
    let paths = HashSet::from([dir.path().join("pages/A.md")]);
    store.close();
    write_page(dir.path(), "A", "- reopened [[Target]]\n");
    let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
    reopened.whole_graph_reconciled().unwrap();
    reopened.watch.stop_background_for_test();
    let revision = reopened.whole_graph().unwrap().rev();
    release(&paused);
    assert!(matches!(poll.join().unwrap(), Err(LoadError::Closed)));
    for paths in [Some(&paths), None] {
        assert!(matches!(
            core.reconcile(paths, true, false, DiffTrigger::Test),
            Err(LoadError::Closed)
        ));
    }
    assert_eq!(reopened.whole_graph().unwrap().rev(), revision);
    assert_indexed(&reopened, "A", "reopened [[Target]]");
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/A.md")).unwrap(),
        "- reopened [[Target]]\n"
    );
    reopened.close();
}

#[test]
fn launch_observes_an_edit_after_parse_even_with_a_busy_writer() {
    for busy in [false, true] {
        let (dir, store) = fixture(true);
        let paused = pause();
        *store.graph.warm_after_install_pause.lock().unwrap() = Some(Arc::clone(&paused));
        fs::remove_file(dir.path().join(".tine-test-pause-load")).unwrap();
        wait(&paused);
        let writer = busy.then(|| store.writer.lock().unwrap());
        write_page(dir.path(), "A", "- arrived after parse [[Target]]\n");
        release(&paused);
        drop(writer);
        // Read only the published index: Store::page would itself reread disk.
        assert_indexed(&store, "A", "arrived after parse [[Target]]");
        assert_eq!(
            store
                .whole_graph()
                .unwrap()
                .explicit_referrers(&["Target".into()])
                .len(),
            1
        );
        store.close();
    }
}

#[test]
fn a_config_and_page_batch_publishes_one_current_view() {
    let (dir, store) = fixture(false);
    let core = store.watch.core_for_load();
    let config = dir.path().join("logseq/config.edn");
    write_page(dir.path(), "A", "- same batch [[Target]]\n");
    fs::write(&config, "{:hidden [\"pages/B.md\"]}\n").unwrap();
    let mut pending = Pending::default();
    for path in [config, dir.path().join("pages/A.md")] {
        let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
            .add_path(path);
        assert!(pending.add(Ok(event), dir.path(), &store.graph.current_config()));
    }
    let include_config = pending.config;
    let (paths, full, first_event_at) = pending.drain();
    assert!(!full);
    let changes = store.subscribe();
    core.reconcile_batch(
        Some(&paths),
        include_config,
        WatchBatch {
            first_event_at,
            reconcile_started: Instant::now(),
            poll: false,
            full_diff: full,
            event_paths: paths.len(),
        },
    )
    .unwrap();
    let change = changes.try_recv().unwrap().unwrap();
    assert!(change
        .files
        .iter()
        .any(|(id, _, _)| id.as_str() == "logseq/config.edn"));
    assert!(changes.try_recv().unwrap().is_none());
    assert_indexed(&store, "A", "same batch [[Target]]");
    let graph = store.whole_graph().unwrap();
    assert!(matches!(
        graph.resolve("B", false),
        crate::Resolved::Absent { .. }
    ));
    assert_eq!(graph.explicit_referrers(&["Target".into()]).len(), 1);
    core.reconcile(None, true, false, DiffTrigger::Test)
        .unwrap();
    assert!(
        changes.try_recv().unwrap().is_none(),
        "next poll had to repair the batch"
    );
    assert_eq!(store.whole_graph().unwrap().rev(), graph.rev());
    store.close();
}
