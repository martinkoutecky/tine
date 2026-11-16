//! Recovery's disk write precedes its owner's index delivery. A refresh
//! must settle that delivery before a name-based operation reads the view.
use super::*;
use crate::{PageHost, Resolved, Store};
use std::fs;
use std::sync::Condvar;

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
    let pause: crate::store::TestPause = Arc::new((Mutex::new((false, false)), Condvar::new()));
    struct Release(crate::store::TestPause);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0 .0.lock().unwrap().1 = true;
            self.0 .1.notify_all();
        }
    }
    let _release = Release(pause.clone());
    *store.host_publication_pause.lock().unwrap() = Some(pause.clone());
    let launched = store.clone();
    let launch = std::thread::spawn(move || PageHost::start_for_tests(&launched, &app).unwrap());
    let (state, ready) = &*pause;
    let mut state = state.lock().unwrap();
    while !state.0 {
        let waited = ready
            .wait_timeout(state, std::time::Duration::from_secs(20))
            .unwrap();
        state = waited.0;
        assert!(
            !waited.1.timed_out() || state.0,
            "recovery did not reach index delivery"
        );
    }
    drop(state);
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
    let mut state = pause.0.lock().unwrap();
    state.1 = true;
    ready.notify_all();
    drop(state);
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
        "recovered page missing: {resolved:?}"
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
