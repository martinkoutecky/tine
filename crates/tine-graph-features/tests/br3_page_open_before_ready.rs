//! GH #623 BR3: the app's page open (`get_page`) does not wait for the whole
//! graph when a file is named for the page; a name only the index can
//! resolve (an alias) or that no file claims waits for it and is never
//! reported absent early.
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tine_core::model::PageKind;
use tine_graph_features::pages::get_page;
use tine_store::{OpenOptions, PageRead, Store};

fn ask(store: &Arc<Store>, name: &str) -> mpsc::Receiver<Option<PageRead>> {
    let (send, receive) = mpsc::channel();
    let (store, name) = (Arc::clone(store), name.to_owned());
    std::thread::spawn(move || {
        let _ = send.send(get_page(&store, &name, PageKind::Page).unwrap());
    });
    receive
}

#[test]
fn a_named_page_opens_before_ready_and_an_alias_waits_for_the_index() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::create_dir_all(root.path().join("journals")).unwrap();
    std::fs::write(root.path().join("pages/Plain.md"), "- plain body\n").unwrap();
    std::fs::write(
        root.path().join("pages/Owner.md"),
        "alias:: Nick\n\n- owned\n",
    )
    .unwrap();
    let hold = root.path().join(".tine-test-pause-load");
    std::fs::write(&hold, "").unwrap();
    let store = Arc::new(Store::open(root.path(), OpenOptions::default()).unwrap().0);
    assert!(matches!(store.is_graph_ready(), Ok(false)));

    let plain = ask(&store, "plain").recv_timeout(Duration::from_secs(2));
    let alias = ask(&store, "Nick");
    let absent = ask(&store, "Nobody");
    let early_alias = alias.recv_timeout(Duration::from_millis(300));
    let early_absent = absent.recv_timeout(Duration::from_millis(1));
    std::fs::remove_file(&hold).unwrap();

    let plain = plain
        .expect("GH #623 BR3: a page a file is named for opens before the initial parse")
        .expect("exists");
    assert_eq!(plain.id.as_str(), "pages/Plain.md");
    assert!(
        early_alias.is_err() && early_absent.is_err(),
        "an alias or unclaimed name waits for the index rather than reading as absent"
    );
    let owner = alias
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(owner.id.as_str(), "pages/Owner.md");
    assert!(absent
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .is_none());
    store.close();
}

/// BqbeQ: an empty today (no journal file) reads as absent while the initial
/// parse is held, instead of waiting for it, and an existing day opens; a
/// page whose alias spells today's title does not claim the journal, before
/// or after the graph is ready (no alias resolves a journal).
#[test]
fn an_unclaimed_journal_day_is_absent_before_ready_and_aliases_never_claim_it() {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::create_dir_all(root.path().join("journals")).unwrap();
    std::fs::write(root.path().join("journals/2026_10_18.md"), "- yesterday\n").unwrap();
    std::fs::write(
        root.path().join("pages/Owner.md"),
        "alias:: Oct 19th, 2026\n\n- owns an alias spelled like a journal day\n",
    )
    .unwrap();
    let hold = root.path().join(".tine-test-pause-load");
    std::fs::write(&hold, "").unwrap();
    let store = Arc::new(Store::open(root.path(), OpenOptions::default()).unwrap().0);
    let journal = |name: &str| {
        let (send, receive) = mpsc::channel();
        let (store, name) = (Arc::clone(&store), name.to_owned());
        std::thread::spawn(move || {
            let _ = send.send(get_page(&store, &name, PageKind::Journal).unwrap());
        });
        receive
    };
    let today = journal("Oct 19th, 2026").recv_timeout(Duration::from_secs(5));
    let yesterday = journal("Oct 18th, 2026").recv_timeout(Duration::from_secs(5));
    // The id an unclaimed day is created as, also before Ready (`resolve_page`).
    let absent = |name: &str| {
        let (send, receive) = mpsc::channel();
        let (store, name) = (Arc::clone(&store), name.to_owned());
        std::thread::spawn(move || {
            let _ =
                send.send(tine_graph_features::journals::absent_journal_id(&store, &name).unwrap());
        });
        receive.recv_timeout(Duration::from_secs(5))
    };
    let today_id = absent("Oct 19th, 2026");
    let yesterday_id = absent("Oct 18th, 2026");
    let not_a_day = absent("Owner");
    let held = matches!(store.is_graph_ready(), Ok(false));
    std::fs::remove_file(&hold).unwrap();
    assert!(held);
    assert!(
        today
            .expect("an unclaimed journal day waited for the graph-wide parse")
            .is_none(),
        "the alias owner is not the journal"
    );
    assert_eq!(
        yesterday.unwrap().unwrap().id.as_str(),
        "journals/2026_10_18.md"
    );
    store.whole_graph().unwrap();
    assert!(get_page(&store, "Oct 19th, 2026", PageKind::Journal)
        .unwrap()
        .is_none());
    let tine_store::Resolved::Absent { id: resolved } =
        store.whole_graph().unwrap().resolve("Oct 19th, 2026", true)
    else {
        panic!("an alias claimed a journal day")
    };
    assert_eq!(
        today_id.expect("the absent day's id waited for the graph-wide parse"),
        Some(resolved),
        "the early id is the graph-wide Absent id"
    );
    assert_eq!(
        yesterday_id.unwrap(),
        None,
        "a claimed day is the graph's answer"
    );
    assert_eq!(not_a_day.unwrap(), None, "not a journal day");
    store.close();
}
