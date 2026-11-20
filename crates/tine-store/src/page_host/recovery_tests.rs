//! Recovery's disk write precedes its owner's index delivery. A refresh
//! must settle that delivery before a name-based operation reads the view.
use super::*;
use crate::{PageHost, Resolved, Store};
use std::fs;
use std::sync::Condvar;

struct Pause(crate::store::TestPause);

impl Pause {
    fn new() -> Self {
        Self(Arc::new((Mutex::new((false, false)), Condvar::new())))
    }

    fn wait(&self, message: &str) {
        let (state, ready) = &*self.0;
        let (state, _) = ready
            .wait_timeout_while(
                state.lock().unwrap(),
                std::time::Duration::from_secs(20),
                |state| !state.0,
            )
            .unwrap();
        assert!(state.0, "{message}");
    }

    fn resume(&self) {
        self.0 .0.lock().unwrap().1 = true;
        self.0 .1.notify_all();
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        self.resume();
    }
}

#[test]
fn refresh_then_rename_sees_recovered_pages_before_owner_delivery() {
    for depth in [
        crate::Depth::PageStamps,
        crate::Depth::Stamps,
        crate::Depth::Bytes,
        crate::Depth::Rebuild,
    ] {
        recovered_page_at_depth(depth, false);
        recovered_page_at_depth(depth, true);
    }
}

fn recovered_page_at_depth(depth: crate::Depth, cold: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let app = dir.path().join("app");
    let drafts_dir = app.join("drafts-v2/test-graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(&drafts_dir).unwrap();
    let old = b"title:: old\n- [[old]]\n";
    let new = b"title:: Old\n- [[Old]]\n";
    // The crash can leave the destination bytes durable while the draft
    // still awaits retirement. Recovery must publish its first observation.
    fs::write(root.join("pages/Old.md"), new).unwrap();
    let hold_load = root.join(".tine-test-pause-load");
    if cold {
        fs::write(&hold_load, "").unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    if !cold {
        store.whole_graph_reconciled().unwrap();
    }
    store.watch.stop_background_for_test();
    fs::write(
        drafts_dir.join(drafts::page_name("pages/Old.md")),
        drafts::encode(&[Record {
            page: "pages/Old.md".into(),
            wseq: 1,
            version: 1,
            base: Base::Known(Some(Arc::from(old.as_slice()))),
            bytes: Some(Arc::from(new.as_slice())),
        }]),
    )
    .unwrap();
    let pause = Pause::new();
    *store.host_publication_pause.lock().unwrap() = Some(pause.0.clone());
    let launched = store.clone();
    let launch = std::thread::spawn(move || PageHost::start_for_tests(&launched, &app).unwrap());
    pause.wait("recovery did not reach index delivery");
    assert_eq!(fs::read(root.join("pages/Old.md")).unwrap(), new);
    if cold {
        fs::remove_file(hold_load).unwrap();
        store.whole_graph_reconciled().unwrap();
    }
    let before = store.diagnostics();
    enum Event {
        Forwarded(bool),
        Refreshed(crate::WholeGraph),
    }
    let (sent, events) = std::sync::mpsc::channel();
    let forwarded = sent.clone();
    store.watch.observe_forward_for_tests(move |waits| {
        let _ = forwarded.send(Event::Forwarded(waits));
    });
    let reading = store.clone();
    let read = std::thread::spawn(move || {
        reading.refresh(depth).unwrap();
        sent.send(Event::Refreshed(reading.whole_graph().unwrap()))
            .unwrap();
    });
    // A refresh either returns a view or waits for an acknowledged owner
    // read. Its preliminary config check can also forward without waiting.
    let next = || {
        events
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("refresh did not settle")
    };
    let stale = loop {
        match next() {
            Event::Forwarded(true) => break None,
            Event::Forwarded(false) => {}
            Event::Refreshed(view) => break Some(view),
        }
    };
    pause.resume();
    let host = launch.join().unwrap();
    let view = match stale {
        Some(view) => view,
        None => loop {
            if let Event::Refreshed(view) = next() {
                break view;
            }
        },
    };
    read.join().unwrap();
    let resolved = view.resolve("Old", false);
    let after = store.diagnostics();
    // Clean up before asserting, including the fail-before path.
    *store.host_publication_pause.lock().unwrap() = None;
    host.stop();
    store.close();
    assert!(
        matches!(resolved, Resolved::Existing { .. }),
        "{depth:?} cold={cold}: recovered page missing: {resolved:?}"
    );
    assert_eq!(
        after["fullDiffs"]["total"].as_u64().unwrap(),
        before["fullDiffs"]["total"].as_u64().unwrap() + 1,
        "one existing refresh walk, no second scan for the wait"
    );
    if !matches!(depth, crate::Depth::Rebuild) {
        assert_eq!(
            after["launch"]["loadPassesTotal"], before["launch"]["loadPassesTotal"],
            "no extra parse pass"
        );
    }
}

#[test]
fn rebuild_keeps_the_name_of_a_page_released_before_publication() {
    rebuild_release(0, true, false);
    rebuild_release(0, true, true);
}

#[test]
#[ignore = "synthetic Rescan timing probe"]
fn measure_recovery_rescan() {
    for _ in 0..11 {
        rebuild_release(1000, false, false);
    }
}

fn rebuild_release(nested: usize, check: bool, during_certification: bool) {
    use crate::model::{GRAPH_LIST_CALLS, GRAPH_PREAMBLE_READS};
    use crate::{Depth, PageId};

    let dir = tempfile::tempdir().unwrap();
    let page = PageId::from("pages/straße.md");
    fs::create_dir(dir.path().join("pages")).unwrap();
    let path = dir.path().join(page.as_str());
    let write_settled = |path: &std::path::Path, text: &str| {
        fs::write(path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000),
            )
            .unwrap();
    };
    write_settled(&path, "title:: Recovered\n- text\n");
    let collision = path.with_file_name("STRASSE.md");
    if !collision.exists() {
        write_settled(&collision, "- other\n");
    }
    if check {
        let journals = dir.path().join("journals");
        fs::create_dir(&journals).unwrap();
        write_settled(&journals.join("2026_06_26.md"), "- canonical\n");
        write_settled(&journals.join("2026_06_26.org"), "* duplicate\n");
    }
    for i in 0..nested {
        let parent = dir.path().join(format!("pages/nested-{i}"));
        fs::create_dir(&parent).unwrap();
        write_settled(&parent.join(format!("Page-{i}.md")), "- text\n");
    }
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    store.watch.stop_background_for_test();
    store.hold_page(&page).unwrap();
    fn passes(store: &Store) -> u64 {
        store.diagnostics()["launch"]["loadPassesTotal"]
            .as_u64()
            .unwrap()
    }
    let rescan = move |store: &Store| {
        let before = passes(store);
        GRAPH_LIST_CALLS.with(|calls| calls.set(0));
        GRAPH_PREAMBLE_READS.with(|reads| reads.set(0));
        let start = std::time::Instant::now();
        store.refresh(Depth::Rebuild).unwrap();
        let wall = start.elapsed();
        let probes = crate::model::BUILD_LOCK_PROBES.with(|counts| counts.get());
        if check {
            assert_eq!(probes, [0; 4], "filesystem calls under cache write lock");
        } else {
            let cache = crate::model::BUILD_LOCK_TIME.with(|time| time.get());
            let writer = crate::watch::REBUILD_WRITER_TIME.with(|time| time.get());
            let folds = crate::model::BUILD_FOLD_INDEX_BYTES.with(|bytes| bytes.get());
            println!(
                "rescan_us={} cache_us={} writer_us={} probes={probes:?} folded_index_bytes={folds}",
                wall.as_micros(),
                cache.as_micros(),
                writer.as_micros()
            );
        }
        (
            GRAPH_LIST_CALLS.with(|calls| calls.get()),
            GRAPH_PREAMBLE_READS.with(|reads| reads.get()),
            passes(store) - before,
        )
    };
    let ordinary = rescan(&store);

    let pause = Pause::new();
    let hook = if during_certification {
        &store.graph.certify_pause
    } else {
        &store.graph.warm_after_install_pause
    };
    *hook.lock().unwrap() = Some(pause.0.clone());
    let reading = store.clone();
    let read = std::thread::spawn(move || {
        let counts = rescan(&reading);
        (reading.whole_graph().unwrap(), counts)
    });
    // Exercise both sides of the cache install before Rescan publishes.
    pause.wait("rebuild did not reach the controlled release");
    *hook.lock().unwrap() = None;
    {
        let _writer = store.writer.lock().unwrap();
        let generation = store.graph.cache_generation();
        let epoch = store.graph.held.epoch();
        store.watch.release_hold(page.as_str());
        assert_eq!(store.graph.cache_generation(), generation);
        assert_ne!(store.graph.held.epoch(), epoch);
    }
    pause.resume();
    let (view, released) = read.join().unwrap();
    let resolved = view.resolve("Recovered", false);
    if check {
        let format = store.graph.current_journal_format();
        let title = format.title(format.parse("2026_06_26").unwrap());
        assert!(
            matches!(view.resolve(&title, true), Resolved::Existing { id, others }
            if id.as_str() == "journals/2026_06_26.md"
            && others.iter().any(|id| id.as_str() == "journals/2026_06_26.org"))
        );
    }
    store.close();
    assert!(
        matches!(&resolved, Resolved::Existing { id, .. } if *id == page),
        "recovered page missing: {resolved:?}"
    );
    // Rescan lists once to build and once to recheck stamps before swapping.
    assert_eq!(ordinary, (2, 0, 1), "ordinary Rescan work");
    assert_eq!(released, ordinary, "a release adds no listing work");
}

#[test]
fn page_open_completes_while_rebuild_certification_is_paused() {
    let dir = tempfile::tempdir().unwrap();
    let page = crate::PageId::from("pages/A.md");
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join(page.as_str()), "- contents\n").unwrap();
    fs::write(dir.path().join("pages/B.md"), "- open this\n").unwrap();
    let store = Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    store.watch.stop_background_for_test();
    store.hold_page(&page).unwrap();

    let pause = Pause::new();
    *store.graph.certify_pause.lock().unwrap() = Some(pause.0.clone());
    let rebuilding = store.clone();
    let rebuild = std::thread::spawn(move || rebuilding.refresh(crate::Depth::Rebuild));
    pause.wait("rebuild did not reach name certification");
    let opening = store.clone();
    let (sent, opened) = std::sync::mpsc::channel();
    let open = std::thread::spawn(move || {
        sent.send(opening.page(&crate::PageId::from("pages/B.md")))
            .unwrap();
    });
    let read = opened.recv_timeout(std::time::Duration::from_secs(2));
    pause.resume();
    rebuild.join().unwrap().unwrap();
    open.join().unwrap();
    store.close();
    assert_eq!(
        read.expect("page open waited on certification")
            .unwrap()
            .doc
            .name,
        "B"
    );
}

#[test]
fn refresh_publishes_an_external_edit_to_a_held_page_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/Old.md");
    fs::write(&path, "title:: Old\n- original\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    store.watch.stop_background_for_test();
    let (sent, mail) = std::sync::mpsc::channel();
    let host = PageHost::start(
        &store,
        &dir.path().join("app"),
        "test-graph",
        |_, _| {},
        move |m| {
            let _ = sent.send(m);
        },
    )
    .unwrap();
    host.open(
        host.session(),
        1,
        &crate::PageId::from("pages/Old.md"),
        "Old",
    )
    .unwrap();
    while !mail
        .recv()
        .unwrap()
        .answer
        .is_some_and(|answer| answer.id == 1)
    {}
    for (index, depth) in [
        crate::Depth::PageStamps,
        crate::Depth::Stamps,
        crate::Depth::Bytes,
        crate::Depth::Rebuild,
    ]
    .into_iter()
    .enumerate()
    {
        let title = format!("External {index}");
        fs::write(&path, format!("title:: {title}\n- external text\n")).unwrap();
        store.refresh(depth).unwrap();
        assert!(
            matches!(
                store.whole_graph().unwrap().resolve(&title, false),
                Resolved::Existing { .. }
            ),
            "{depth:?}: external title is published before return"
        );
    }
    host.stop();
    store.close();
}
