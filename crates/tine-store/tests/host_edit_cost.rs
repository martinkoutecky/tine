//! I-25 unit cost of an ordinary edit on the page host (STEP3 §3, step 3b
//! P2b): bytes, files and syncs per `page_submit` on 1- and 60-block pages,
//! counted at the host's `HostIo` seam (`cost_counters`, Q-P2b-2). Its own
//! test binary: the counters are process-global.
use std::fs;
use std::sync::Arc;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use tine_store::cost_counters::{self, Counts};
use tine_store::{EditKind, PageHost, PageId, Store};

/// The counters are process-global: one case at a time.
static CASE_LOCK: Mutex<()> = Mutex::new(());

/// Mail as the window receives it (its JSON wire shape).
type Mail = mpsc::Receiver<serde_json::Value>;

fn answer_version(mail: &Mail, id: u64) -> u64 {
    loop {
        let value = mail
            .recv_timeout(Duration::from_secs(10))
            .expect("the host answers every admitted request");
        if value["answer"]["id"] == id {
            assert_eq!(value["answer"]["outcome"]["kind"], "applied", "{value}");
            return value["answer"]["version"].as_u64().unwrap();
        }
    }
}

/// `edits` submits of the page's first block, each answered before the next,
/// then publication of the last. Returns the counts from the first submit to
/// publication, and the page file's final length.
fn edit(blocks: usize, edits: usize) -> (Counts, u64) {
    let dir = tempfile::Builder::new()
        .prefix("host-edit-cost-")
        .tempdir()
        .unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Page.md"), "- before\n".repeat(blocks)).unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let (sink, mail) = mpsc::channel();
    let host = PageHost::start(&store, &dir.path().join("app"), "cost", move |m| {
        let _ = sink.send(serde_json::to_value(m).unwrap());
    })
    .unwrap();
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let mut id = reloaded["nextId"].as_u64().unwrap();
    let page = PageId::from("pages/Page.md");
    let key = host.open(session, id, &page, "Page").unwrap();
    let key = serde_json::to_value(key).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let mut version = answer_version(&mail, id);
    let mut doc = store.page(&page).unwrap().doc;
    cost_counters::reset();
    for n in 0..edits {
        id += 1;
        doc.blocks[0].raw = format!("after {n}");
        host.submit(
            session,
            id,
            &key,
            &doc,
            version,
            None,
            &[EditKind::SaveBlock],
        )
        .unwrap();
        version = answer_version(&mail, id);
    }
    let published = host.wait_published(
        session,
        &[(key.clone(), version, None)],
        Duration::from_secs(10),
    );
    let counts = cost_counters::snapshot();
    assert_eq!(
        format!("{published:?}"),
        "Applied",
        "the edit publishes on the host"
    );
    let len = fs::metadata(root.join("pages/Page.md")).unwrap().len();
    assert!(fs::read_to_string(root.join("pages/Page.md"))
        .unwrap()
        .starts_with(&format!("- after {}\n", edits - 1)));
    drop(host);
    store.close();
    (counts, len)
}

#[test]
fn an_ordinary_edit_is_one_guarded_save_with_no_draft() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for blocks in [1, 60] {
        let (one, len) = edit(blocks, 1);
        eprintln!("I-25 host edit unit cost: blocks={blocks} page bytes={len} one edit={one:?}");
        // One guarded save: the page's temporary file, renamed over the page,
        // then the temp file's and the directory's syncs. No draft record: an
        // ordinary submit is not at risk (STEP3 §3), so no app-data write.
        assert_eq!(
            one.files_written, 1,
            "I-25: an ordinary edit writes one file, the page's temp (no draft); exemplar page_host/save.rs SavePhase::Temp"
        );
        assert_eq!(
            one.bytes_written, len,
            "I-25: an ordinary edit writes exactly the page's bytes; exemplar page_host/save.rs SavePhase::Temp"
        );
        assert!(
            one.fsyncs <= 2,
            "I-25: an ordinary edit syncs the temp file and the directory only: {one:?}"
        );
        assert_eq!(one.readdir, 0, "I-13: a host save walks no directory");
    }
}

#[test]
fn a_burst_of_submits_is_bounded_by_its_submits() {
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Five submits answered back to back (the window sends at most one per
    // debounce, 400 ms with a 1 s max wait): the host saves at most once per
    // submit and writes no draft.
    let (burst, len) = edit(60, 5);
    eprintln!("I-25 host edit unit cost: 60 blocks, 5 submits={burst:?}");
    assert!(
        burst.files_written <= 5,
        "I-25: at most one save per submit: {burst:?}"
    );
    assert!(
        burst.bytes_written <= 5 * len,
        "I-25: page bytes per save only: {burst:?}"
    );
    assert!(
        burst.fsyncs <= 2 * burst.files_written,
        "I-25: two syncs per save: {burst:?}"
    );
}
