//! GH #863 (I-25): the unit cost of opening a page the page host holds.
//! An unchanged held page opens from the owner's index with no file read,
//! no stamp by path, no owner request and no publication; a held page
//! changed behind a missed watcher event costs one owner request, its one
//! observation and its publication. Counted at the store's seams
//! (`cost_counters`), with the background watcher stopped so its own
//! cycles stay out of the exact counts.
use std::fs;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime};

use tine_store::cost_counters::{self, Counts};
use tine_store::{PageHost, PageId, Store};

/// The counters are process-global: one case at a time.
static CASE_LOCK: Mutex<()> = Mutex::new(());

/// The counts of `store.page` on a held page, and its first block, after
/// `change` ran on the graph root.
fn held_open(change: impl FnOnce(&std::path::Path)) -> (Counts, String) {
    let dir = tempfile::Builder::new()
        .prefix("held-open-cost-")
        .tempdir()
        .unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Page.md"), "- before\n").unwrap();
    for index in 0..20 {
        let rel = format!("pages/Other{index:04}.md");
        fs::write(root.join(rel), format!("- unrelated [[Other{index}]]\n")).unwrap();
    }
    // Aged past the watcher's racy window (§5.4): an aged page's stamp
    // vouches for its bytes.
    let old = SystemTime::now() - Duration::from_secs(3600);
    for entry in fs::read_dir(root.join("pages")).unwrap() {
        let file = fs::File::options().write(true).open(entry.unwrap().path());
        file.unwrap().set_modified(old).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store.stop_watcher_for_tests();
    let (sink, mail) = mpsc::channel();
    let host = PageHost::start(
        &store,
        &dir.path().join("app"),
        "cost",
        |_, _| {},
        move |m| {
            let _ = sink.send(serde_json::to_value(m).unwrap());
        },
    )
    .unwrap();
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    let page = PageId::from("pages/Page.md");
    host.open(session, id, &page, "Page").unwrap();
    loop {
        let value = mail
            .recv_timeout(Duration::from_secs(10))
            .expect("the host answers the open");
        if value["answer"]["id"] == id {
            break;
        }
    }
    // The open's own work settles: the first read answers the indexed page.
    assert_eq!(store.page(&page).unwrap().doc.blocks[0].raw, "before");
    change(&root);
    cost_counters::reset();
    let read = store.page(&page).unwrap();
    let counts = cost_counters::snapshot();
    drop(host);
    store.close();
    (counts, read.doc.blocks[0].raw.clone())
}

fn io(c: &Counts) -> [(&'static str, u64); 7] {
    [
        ("full_reads", c.full_reads),
        ("hash_reads", c.hash_reads),
        ("stamps_by_path", c.stamps_by_path),
        ("store_reads", c.store_reads),
        ("held_read_forwards", c.held_read_forwards),
        ("observation_reads", c.observation_reads),
        ("host_publication_probes", c.host_publication_probes),
    ]
}

#[test]
fn an_unchanged_held_open_reads_no_file_and_asks_no_owner() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (counts, first) = held_open(|_| {});
    eprintln!("I-25 GH #863 unchanged held open: {counts:?}");
    assert_eq!(first, "before");
    assert_eq!(
        counts.parses, 1,
        "the held row's one parse, as before GH #863"
    );
    assert_eq!(
        io(&counts),
        [
            ("full_reads", 0),
            ("hash_reads", 0),
            ("stamps_by_path", 0),
            ("store_reads", 0),
            ("held_read_forwards", 0),
            ("observation_reads", 0),
            ("host_publication_probes", 0),
        ],
        "I-25/GH #863: an unchanged held open costs one stat and one index \
         lookup; exemplar store/page_read.rs held_observation_owed"
    );
}

#[test]
fn a_changed_held_open_costs_one_owner_observation() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (counts, first) = held_open(|root| {
        // A missed external write (the watcher is stopped).
        fs::write(root.join("pages/Page.md"), "- after\n").unwrap();
    });
    eprintln!("I-25 GH #863 changed held open: {counts:?}");
    assert_eq!(first, "after", "the read answers the file");
    assert_eq!(
        io(&counts),
        [
            ("full_reads", 0),
            ("hash_reads", 0),
            ("stamps_by_path", 1),
            ("store_reads", 0),
            ("held_read_forwards", 1),
            ("observation_reads", 1),
            ("host_publication_probes", 1),
        ],
        "I-25/GH #863: a changed held open is one owner request, its one \
         observation (one stamp, one read) and its one publication; the read \
         itself reads no file; exemplar page_host/driver.rs step_unblocked"
    );
}
