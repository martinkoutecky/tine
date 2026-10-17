//! I-25 unit cost of an ordinary edit on the page host (STEP3 §3, step 3b
//! P2b): bytes, files and syncs per `page_submit` on 1- and 60-block pages,
//! counted at the host's `HostIo` seam (`cost_counters`, Q-P2b-2), and its
//! independence of unrelated graph size (REVIEW-3b-P2b R4(a)). Its own test
//! binary: the counters are process-global.
use std::fs;
use std::sync::Arc;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, SystemTime};

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

/// The graph beside the edited page: `unrelated` other pages, and whether
/// every file is aged past the watcher's 2 s racy window (§5.4) or fresh,
/// as just delivered by a sync tool.
#[derive(Clone, Copy)]
struct Graph {
    unrelated: usize,
    aged: bool,
    /// How many of the unrelated pages the window holds open beside it.
    held: usize,
}

const ALONE: Graph = Graph {
    unrelated: 0,
    aged: false,
    held: 0,
};

/// `edits` submits of the page's first block, each answered before the next,
/// then publication of the last. Returns the counts from the first submit to
/// publication, and the page file's final length.
fn edit(blocks: usize, edits: usize, graph: Graph) -> (Counts, u64) {
    let dir = tempfile::Builder::new()
        .prefix("host-edit-cost-")
        .tempdir()
        .unwrap();
    let root = dir.path().join("graph");
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::write(root.join("pages/Page.md"), "- before\n".repeat(blocks)).unwrap();
    for index in 0..graph.unrelated {
        let rel = format!("pages/Other{index:04}.md");
        fs::write(root.join(&rel), format!("- unrelated [[Other{index}]]\n")).unwrap();
    }
    if graph.aged {
        let old = SystemTime::now() - Duration::from_secs(3600);
        for entry in fs::read_dir(root.join("pages")).unwrap() {
            let file = fs::File::options().write(true).open(entry.unwrap().path());
            file.unwrap().set_modified(old).unwrap();
        }
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
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
    let mut id = reloaded["nextId"].as_u64().unwrap();
    let page = PageId::from("pages/Page.md");
    let key = host.open(session, id, &page, "Page").unwrap();
    let key = serde_json::to_value(key).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let mut version = answer_version(&mail, id);
    for index in 0..graph.held {
        id += 1;
        let other = PageId::from(format!("pages/Other{index:04}.md").as_str());
        host.open(session, id, &other, &format!("Other{index:04}"))
            .unwrap();
        answer_version(&mail, id);
    }
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
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for blocks in [1, 60] {
        let (one, len) = edit(blocks, 1, ALONE);
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
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Five submits answered back to back (the window sends at most one per
    // debounce, 400 ms with a 1 s max wait): the host saves at most once per
    // submit and writes no draft.
    let (burst, len) = edit(60, 5, ALONE);
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

#[test]
fn an_edits_publication_does_not_scale_with_the_unrelated_graph() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // REVIEW-3b-P2b R4(a): the path from submit to publication (save, index
    // publication, the waiter) reads and stamps the edited page only, on an
    // aged graph and on one a sync tool just delivered (every file racy, so
    // the watcher rehashes them; that is its own thread's work, §5.4).
    for aged in [true, false] {
        let [small, large] = [60, 600].map(|unrelated| {
            let graph = Graph {
                unrelated,
                aged,
                held: 0,
            };
            let (counts, _) = edit(1, 1, graph);
            eprintln!("R4(a) aged={aged} unrelated={unrelated}: {counts:?}");
            counts
        });
        let path = |c: &Counts| {
            [
                ("full_reads", c.full_reads),
                ("preamble_reads", c.preamble_reads),
                ("hash_reads", c.hash_reads),
                ("stamps_by_path", c.stamps_by_path),
                ("readdir", c.readdir),
                ("parses", c.parses),
            ]
        };
        assert_eq!(
            path(&large),
            path(&small),
            "I-13/R4(a): an edit's publication must not read or stamp unrelated pages \
             (aged={aged}); exemplar page_host/binding_bridge.rs index()"
        );
        assert_eq!(large.files_written, 1, "the edit's one save");
    }
}

#[test]
fn an_edit_among_many_held_pages_visits_what_changed() {
    let _integration_guard = crate::integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // M4 (REVIEW-progress-plan R5): one edit while the window holds many
    // other pages open. Reconcile, draft-index and worker-key work follows
    // what changed; the subscribed pages are walked only by the retirement
    // candidates and the quiet notices, once per poll and per collection.
    let measured = [20u64, 100, 400].map(|held| {
        let graph = Graph {
            unrelated: held as usize,
            aged: true,
            held: held as usize,
        };
        let (counts, _) = edit(1, 1, graph);
        eprintln!("M4 held={held}: {counts:?}");
        (held, counts)
    });
    let changed = |c: &Counts| {
        [
            ("host_reconcile_visits", c.host_reconcile_visits),
            ("host_draft_record_visits", c.host_draft_record_visits),
            ("host_worker_key_visits", c.host_worker_key_visits),
        ]
    };
    let (_, fewest) = &measured[0];
    for (held, c) in &measured {
        assert!(
            c.host_reconcile_visits > 0,
            "reconcile is instrumented: {c:?}"
        );
        assert_eq!(
            changed(c),
            changed(fewest),
            "M4/I-14/D-10: an edit's reconcile, draft-index and worker work must not grow with \
             the {held} pages the window holds; a reconcile visits what changed (exemplar \
             page_host/progress.rs reconcile)"
        );
        // The named remaining dimension S (contract §9): per poll, the clean
        // subscribed pages as retirement candidates; per collection, the
        // subscribed pages' quiet notices. Fixed from the reviewed
        // measurement (6 and 19 per held page), about 2x headroom.
        for (name, count, per_held) in [
            ("host_candidate_visits", c.host_candidate_visits, 12),
            ("host_book_visits", c.host_book_visits, 40),
        ] {
            assert!(
                count <= per_held * held + 100,
                "M4: an edit's {name} is at most {per_held} per held page plus 100; got {count} \
                 with {held} held: {c:?}"
            );
        }
    }
}
