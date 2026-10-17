//! REVIEW-3b-P2b R2 (STEP3-DESIGN D4, no chaining): a retained writer
//! (STEP3 §7) is a second operation like a host rename or delete, so one over
//! a page of a running rename's order waits until that order ends. Here the
//! rename's destination has published (witnessed, clean), but an external
//! edit made a referrer conflict, so the source's deletion still waits for
//! it. A merge of the destination into another page would trash the page
//! that referrer is still to link to: it waits, an unrelated merge does not,
//! and the window taking disk for the referrer ends the order and lets the
//! merge run. The wait is bounded (M6): a merge still waiting
//! `OPERATION_WAIT` after its start is told the rename is still finishing,
//! with nothing written or reserved.

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

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<Store>,
    host: PageHost,
    mail: mpsc::Receiver<serde_json::Value>,
}

/// `x` (linked from `one` and `two`) to rename into `y`, a merge target `z`,
/// and an unrelated pair `a`, `b`; the host's `phase` fails `times` times.
fn fixture(phase: Phase, times: usize) -> Fixture {
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
    fail(phase, std::io::ErrorKind::Other, times);
    let (sender, mail) = mpsc::channel::<serde_json::Value>();
    let host = PageHost::start(
        &store,
        &dir.path().join("app"),
        "graph",
        |_, _| {},
        move |m| {
            let _ = sender.send(serde_json::to_value(m).unwrap());
        },
    )
    .unwrap();
    Fixture {
        _dir: dir,
        root,
        store,
        host,
        mail,
    }
}

/// A rename held running by a conflicted referrer: a disk error fails the
/// destination's first five writes, so the caller returns before the order
/// completes (as in client.rs's forward test), and an external edit then
/// conflicts `two`, so the source's deletion waits for it.
fn conflicted_rename() -> Fixture {
    let f = fixture(Phase::PageTemp, 5);
    let (root, store, host) = (&f.root, &f.store, &f.host);
    assert!(pages::rename_page_expected(store, host, "x", "y", None).is_err());
    put(root, "pages/two.md", "external stage-2");
    until("the destination and the clean referrer publish", || {
        root.join("pages/y.md").exists() && read(root, "pages/one.md") == "- [[y]]\n"
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(root.join("pages/x.md").exists(), "the order still runs");
    f
}

/// The window takes disk for the conflicted referrer: it leaves the order,
/// the source's deletion completes, and the order ends.
fn take_disk_for_two(f: &Fixture) {
    let reloaded = serde_json::to_value(f.host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    let opened = f
        .host
        .open(session, id, &PageId::from("pages/two.md"), "two")
        .unwrap();
    let key = serde_json::to_value(&opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    loop {
        let m = f.mail.recv_timeout(Duration::from_secs(20)).unwrap();
        if m["key"] == key.as_str() && m["answer"]["id"] == id {
            break;
        }
    }
    f.host.discard(session, id + 1, &key).unwrap();
}

const BUSY: &str = "Another rename of these pages is still finishing; try again in a moment.";

#[test]
fn a_merge_over_a_running_renames_destination_waits_until_the_order_ends() {
    let _integration_guard = crate::integration_lock();
    let f = conflicted_rename();
    let (root, store, host) = (&f.root, &*f.store, &f.host);
    std::thread::scope(|scope| {
        let (done, merged) = mpsc::channel();
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

        take_disk_for_two(&f);
        assert_eq!(
            merged.recv_timeout(Duration::from_secs(30)),
            Ok(Ok(())),
            "the order's end before the bound lets the merge run"
        );
    });
    assert!(!root.join("pages/x.md").exists(), "the rename completed");
    assert!(
        !root.join("pages/y.md").exists(),
        "the merge trashed its source"
    );
    assert!(read(root, "pages/z.md").starts_with("- z\n"));
    assert_eq!(read(root, "pages/two.md"), "external stage-2");
}

/// M6 (STEP3-DESIGN "Manager decisions on REVIEW-progress-plan", R4
/// interpretation 2): a retained writer over a rename that is still
/// finishing waits at most `OPERATION_WAIT` from its start, then is told
/// so (the host rename's own `Busy` wording) with nothing written and
/// nothing left reserved. In-scope scenario: an external editor or a sync
/// service conflicted a referrer, which holds the rename until the user
/// resolves it. An unrelated writer is not held up, and once the order
/// ends the same merge runs.
#[test]
fn a_merge_over_a_conflicted_running_rename_is_told_busy_after_the_bound() {
    let _integration_guard = crate::integration_lock();
    let f = conflicted_rename();
    let (root, store, host) = (&f.root, &*f.store, &f.host);
    std::thread::scope(|scope| {
        let (done, merged) = mpsc::channel();
        let started = Instant::now();
        scope.spawn(move || {
            let result = pages::merge_pages(store, Some(host), "pages/y.md", "pages/z.md");
            let _ = done.send(result.map_err(|error| (error.kind(), error.to_string())));
        });
        pages::merge_pages(store, Some(host), "pages/a.md", "pages/b.md")
            .expect("an unrelated writer stays available");
        let outcome =
            merged.recv_timeout(tine_store::host_faults::OPERATION_WAIT + Duration::from_secs(5));
        if outcome.is_err() {
            // Unblock a writer that still waits (the pre-M6 fence) before
            // the scope joins it.
            take_disk_for_two(&f);
        }
        // A host operation's Busy names no rename: other holders refuse it
        // too (REVIEW P2 round 2, finding 3).
        let busy = "Tine is still finishing another change to these pages, so the merge did not start. Try again in a moment.";
        assert_eq!(
            outcome,
            Ok(Err((std::io::ErrorKind::WouldBlock, busy.to_string()))),
            "M6: the writer is refused Busy within its bound, not left waiting"
        );
        assert!(started.elapsed() >= tine_store::host_faults::OPERATION_WAIT);
    });
    assert_eq!(
        read(root, "pages/y.md"),
        "- [[y]]\n",
        "the destination is unchanged"
    );
    assert_eq!(
        read(root, "pages/z.md"),
        "- z\n",
        "the merge target is unchanged"
    );
    assert!(root.join("pages/x.md").exists(), "the order still runs");
    take_disk_for_two(&f);
    until("the order ends", || !root.join("pages/x.md").exists());
    pages::merge_pages(store, Some(host), "pages/y.md", "pages/z.md")
        .expect("nothing stayed reserved: the merge runs once the order ends");
    assert!(!root.join("pages/y.md").exists());
}
