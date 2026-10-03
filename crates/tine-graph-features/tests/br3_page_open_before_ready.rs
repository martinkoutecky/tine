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
