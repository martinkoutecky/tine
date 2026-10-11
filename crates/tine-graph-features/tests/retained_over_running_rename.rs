//! REVIEW-3b-P2b R2 (STEP3-DESIGN D4, no chaining): a retained writer
//! (STEP3 §7) is a second operation like a host rename or delete, so one over
//! a page of a running rename's order waits until that order ends. Here the
//! rename's destination has published (witnessed, clean), but an external
//! edit made a referrer conflict, so the source's deletion still waits for
//! it. A merge of the destination into another page would trash the page
//! that referrer is still to link to: it waits, an unrelated merge does not,
//! and the window taking disk for the referrer ends the order and lets the
//! merge run.

use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tine_graph_features::pages;
use tine_store::host_faults::{fail, Phase};
use tine_store::{PageHost, PageId, Store};

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

fn until(what: &str, test: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !test() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_merge_over_a_running_renames_destination_waits_until_the_order_ends() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for (rel, body) in [
        ("pages/x.md", "- [[x]]\n"),
        ("pages/one.md", "- [[x]]\n"),
        ("pages/two.md", "- [[x]]\n"),
        ("pages/z.md", "- z\n"),
        ("pages/a.md", "- a\n"),
        ("pages/b.md", "- b\n"),
    ] {
        put(&root, rel, body);
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    // A disk error fails the destination's first five writes, so the caller
    // returns before the order completes (as in client.rs's forward test).
    fail(Phase::PageTemp, std::io::ErrorKind::Other, 5);
    let app_data = tempfile::tempdir().unwrap();
    let (sender, mail) = mpsc::channel::<serde_json::Value>();
    let host = PageHost::start(&store, app_data.path(), "graph", move |m| {
        let _ = sender.send(serde_json::to_value(m).unwrap());
    })
    .unwrap();
    assert!(pages::rename_page_expected(&store, &host, "x", "y", None).is_err());
    put(&root, "pages/two.md", "external stage-2");
    until("the destination and the clean referrer publish", || {
        root.join("pages/y.md").exists() && read(&root, "pages/one.md") == "- [[y]]\n"
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(root.join("pages/x.md").exists(), "the order still runs");

    std::thread::scope(|scope| {
        let (done, merged) = mpsc::channel();
        let (store, host) = (&store, &host);
        scope.spawn(move || {
            let result = pages::merge_pages(store, Some(host), "pages/y.md", "pages/z.md");
            let _ = done.send(result.map_err(|error| error.to_string()));
        });
        assert_eq!(
            merged.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "R2: a merge over the running order's destination waits"
        );
        assert!(root.join("pages/y.md").exists());
        assert_eq!(read(&root, "pages/z.md"), "- z\n");
        pages::merge_pages(store, Some(host), "pages/a.md", "pages/b.md")
            .expect("an unrelated writer stays available");
        assert!(!root.join("pages/a.md").exists());

        // The window takes disk for the conflicted referrer: it leaves the
        // order, the source's deletion completes, and the order ends.
        let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
        let session = reloaded["session"].as_u64().unwrap();
        let id = reloaded["nextId"].as_u64().unwrap();
        let opened = host
            .open(session, id, &PageId::from("pages/two.md"), "two")
            .unwrap();
        let key = serde_json::to_value(&opened).unwrap()["key"]
            .as_str()
            .unwrap()
            .to_owned();
        let version = loop {
            let m = mail.recv_timeout(Duration::from_secs(20)).unwrap();
            if m["key"] == key.as_str() && m["answer"]["id"] == id {
                break m["page"]["version"].as_u64().unwrap();
            }
        };
        host.discard(session, id + 1, &key, version).unwrap();
        assert_eq!(
            merged.recv_timeout(Duration::from_secs(30)),
            Ok(Ok(())),
            "the order's end lets the merge run"
        );
    });
    assert!(!root.join("pages/x.md").exists(), "the rename completed");
    assert!(
        !root.join("pages/y.md").exists(),
        "the merge trashed its source"
    );
    assert!(read(&root, "pages/z.md").starts_with("- z\n"));
    assert_eq!(read(&root, "pages/two.md"), "external stage-2");
}
