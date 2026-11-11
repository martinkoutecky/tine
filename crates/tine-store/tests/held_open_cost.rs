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
use tine_store::{EditKind, PageHost, PageId, Store};

/// The counters are process-global: one case at a time.
static CASE_LOCK: Mutex<()> = Mutex::new(());

/// A held page beside 20 unrelated pages, every file aged past the
/// watcher's racy window (§5.4), with the background watcher stopped.
struct Held {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<Store>,
    host: Option<PageHost>,
    mail: mpsc::Receiver<serde_json::Value>,
    session: u64,
    id: u64,
    key: String,
    version: u64,
    page: PageId,
}

impl Held {
    fn new() -> Held {
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
        let key = serde_json::to_value(host.open(session, id, &page, "Page").unwrap()).unwrap()
            ["key"]
            .as_str()
            .unwrap()
            .to_string();
        let mut held = Held {
            _dir: dir,
            root,
            store,
            host: Some(host),
            mail,
            session,
            id,
            key,
            version: 0,
            page,
        };
        held.version = held.answer();
        // The open's own work settles: the first read answers the indexed page.
        assert_eq!(
            held.store.page(&held.page).unwrap().doc.blocks[0].raw,
            "before"
        );
        held
    }

    /// The version request `self.id` was answered at.
    fn answer(&self) -> u64 {
        loop {
            let value = (self.mail)
                .recv_timeout(Duration::from_secs(10))
                .expect("the host answers every admitted request");
            if value["answer"]["id"] == self.id {
                assert_eq!(value["answer"]["outcome"]["kind"], "applied", "{value}");
                return value["answer"]["version"].as_u64().unwrap();
            }
        }
    }

    /// An own save of the page's first block as `raw`, published.
    fn save(&mut self, raw: &str) {
        let mut doc = self.store.page(&self.page).unwrap().doc;
        doc.blocks[0].raw = raw.into();
        self.id += 1;
        let host = self.host.as_ref().unwrap();
        (host.submit(
            self.session,
            self.id,
            &self.key,
            &doc,
            self.version,
            None,
            &[EditKind::SaveBlock],
        ))
        .unwrap();
        self.version = self.answer();
        let published = host.wait_published(
            self.session,
            &[(self.key.clone(), self.version, None)],
            Duration::from_secs(10),
        );
        assert_eq!(format!("{published:?}"), "Applied", "the save publishes");
    }

    /// The counts of one `store.page` of the held page, and its first block.
    fn open(&self) -> (Counts, String) {
        cost_counters::reset();
        let read = self.store.page(&self.page).unwrap();
        (cost_counters::snapshot(), read.doc.blocks[0].raw.clone())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        drop(self.host.take());
        self.store.close();
    }
}

/// The counts of `store.page` on a held page, and its first block, after
/// `change` ran on the graph root.
fn held_open(change: impl FnOnce(&std::path::Path)) -> (Counts, String) {
    let held = Held::new();
    change(&held.root);
    held.open()
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

/// GH #863 (A-H1): an own save leaves a racy stamp (§5.4) that no full diff
/// settles while notifications run. A held read inside the racy window (a
/// preview refetching on the save's publication) asks the owner, and its
/// read cannot vouch for the bytes yet. The first open past the window asks
/// the owner, whose read confirms the bytes it saved; that read's stamp
/// settles the entry, so the next open asks no one.
#[test]
fn a_held_open_after_an_own_save_asks_its_owner_at_most_once() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut held = Held::new();
    held.save("saved");
    let (racy, raw) = held.open();
    eprintln!("I-25 GH #863 held open inside the racy window of an own save: {racy:?}");
    assert_eq!(raw, "saved");
    // Past the watcher's 2 s `RACY_WINDOW` (§5.4) since the save.
    std::thread::sleep(Duration::from_millis(2100));
    let (first, raw) = held.open();
    eprintln!("I-25 GH #863 first held open after an own save: {first:?}");
    assert_eq!(raw, "saved");
    let (second, raw) = held.open();
    eprintln!("I-25 GH #863 second held open after an own save: {second:?}");
    assert_eq!(raw, "saved");
    assert_eq!(
        io(&second),
        [
            ("full_reads", 0),
            ("hash_reads", 0),
            ("stamps_by_path", 0),
            ("store_reads", 0),
            ("held_read_forwards", 0),
            ("observation_reads", 0),
            ("host_publication_probes", 0),
        ],
        "I-25/GH #863: once its owner confirmed the saved bytes outside the \
         racy window, a held open asks no one; exemplar watch.rs note_confirmed"
    );
}

/// GH #863 (A-H1): a sync client's backdated replacement with the same
/// bytes changes the file's stamp, not its text. The first held open asks
/// the owner, whose read finds the bytes it holds; the next open trusts
/// that read's stamp and reads nothing.
#[test]
fn a_held_open_after_an_identical_backdated_replacement_asks_its_owner_once() {
    let _integration_guard = crate::exclusive_integration_lock();
    let _case = CASE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let held = Held::new();
    let temp = held.root.join("pages/.syncthing.Page.md.tmp");
    fs::write(&temp, "- before\n").unwrap();
    let old = SystemTime::now() - Duration::from_secs(7200);
    (fs::File::options().write(true).open(&temp).unwrap())
        .set_modified(old)
        .unwrap();
    fs::rename(&temp, held.root.join("pages/Page.md")).unwrap();
    let (first, raw) = held.open();
    eprintln!("I-25 GH #863 first held open after an identical replacement: {first:?}");
    assert_eq!(raw, "before");
    assert_eq!(
        first.held_read_forwards, 1,
        "the moved stamp asks the owner"
    );
    let (second, raw) = held.open();
    eprintln!("I-25 GH #863 second held open after an identical replacement: {second:?}");
    assert_eq!(raw, "before");
    assert_eq!(
        io(&second),
        [
            ("full_reads", 0),
            ("hash_reads", 0),
            ("stamps_by_path", 0),
            ("store_reads", 0),
            ("held_read_forwards", 0),
            ("observation_reads", 0),
            ("host_publication_probes", 0),
        ],
        "I-25/GH #863: the owner's confirming read is the file's observation; \
         exemplar watch.rs note_confirmed"
    );
}
