//! The graph binding (STEP3 §3): its bookkeeping beside a model host, and
//! its command path against a real Store with the driver thread running.
use super::*;
use crate::page_host::io::Phase;
use crate::page_host::model_fs::{Fault, ModelFs};
use crate::page_host::tests::{edit, open as open_page, risk, saved, send, text};
use crate::page_host::ReserveRefusal;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Frozen;
impl Clock for Frozen {
    fn now_ms(&self) -> u64 {
        0
    }
}

/// A model host with the binding's book, stepped by hand.
struct Unit {
    progress: Progress<ModelFs, Frozen>,
    book: Book,
}

impl Unit {
    fn new() -> Self {
        let mut unit = Self {
            progress: Progress::new(super::super::tests::host(), Frozen),
            book: Book::default(),
        };
        unit.progress.with_host(|h| open_page(h, "a.md"));
        unit.collect();
        unit
    }

    fn collect(&mut self) -> Delivery {
        let delivery = self.book.collect(&mut self.progress);
        // The driver always holds a lock set; these hand steps take none.
        self.progress.host.held = None;
        delivery
    }

    /// Admit a submit with its handoff, as `PageHost::admit` does, apply
    /// it, and collect.
    fn submit(&mut self, bytes: &str, version: u64, resolve: Option<Text>, mark: &str) -> Delivery {
        let key = PageKey::from("a.md");
        let id = self.progress.host.last_admitted + 1;
        let handoff = Handoff {
            bytes: text(bytes),
            document: Document {
                pre_block: Some(mark.into()),
                roots: vec![],
            },
            kinds: vec![EditKind::SaveBlock],
        };
        self.book.pending.insert((key.clone(), id), handoff);
        let kind = RequestKind::Submit {
            bytes: text(bytes),
            version,
            resolve,
        };
        self.progress.with_host(|h| send(h, &key, kind));
        self.collect()
    }

    fn version(&self) -> u64 {
        self.progress.host.pages["a.md"].version
    }
}

/// A delivery's own-save publications.
fn own(delivery: &Delivery) -> Vec<&Publication> {
    delivery
        .publications
        .iter()
        .filter(|p| p.own.is_some())
        .collect()
}

#[test]
fn q8_a_conflict_loop_retains_only_live_handoffs_and_a_late_publication_gets_its_document() {
    let mut u = Unit::new();
    // Stale input: a conflict with an unknown base, so no save runs.
    u.submit("stale", STALE, None, "stale");
    assert!(u.progress.host.pages["a.md"].conflict);
    for i in 0..200 {
        let version = u.version();
        u.submit(
            &format!("typing {i}"),
            version,
            None,
            &format!("typing {i}"),
        );
        assert!(u.book.pending.is_empty(), "answered handoffs leave pending");
        assert!(
            u.book.versions.get("a.md").is_none_or(|v| v.len() <= 1),
            "Q8: a conflict loop must hold the live entry, not edit history ({} entries)",
            u.book.versions["a.md"].len()
        );
    }
    // Keep mine against the observed disk, then a failed save: the entry
    // stays live for the save that finally publishes it.
    let obs = u.progress.host.pages["a.md"].obs.clone().unwrap();
    let version = u.version();
    u.submit("final", version, Some(obs), "final");
    assert!(!u.progress.host.pages["a.md"].conflict);
    let version = u.version();
    u.progress.with_host(|h| risk(h, "a.md"));
    let delivery = u.collect();
    assert!(own(&delivery).is_empty());
    assert_eq!(u.book.versions["a.md"].len(), 1);
    u.progress.with_host(|h| saved(h, "a.md"));
    let delivery = u.collect();
    let own = own(&delivery);
    assert_eq!(own.len(), 1);
    let (published, document) = own[0].own.as_ref().unwrap();
    assert_eq!((own[0].key.as_str(), *published), ("a.md", version));
    assert_eq!(
        document.as_ref().unwrap().pre_block.as_deref(),
        Some("final"),
        "the published version's own Document (its runtime ids)"
    );
    assert_eq!(
        delivery.kinds,
        vec![(PageKey::from("a.md"), vec![EditKind::SaveBlock])]
    );
    assert!(u.book.versions.is_empty());
}

#[test]
fn r3_an_answer_carries_text_only_when_the_page_moved_past_the_submit() {
    let mut u = Unit::new();
    let version = u.version();
    let delivery = u.submit("S", version, None, "S");
    let (_, mail, facts) = &delivery.mail[0];
    assert!(mail.answer.as_ref().unwrap().took);
    assert!(!facts.content, "exactly the submitted state: no text again");
    let version = u.version();
    u.progress.with_host(|h| {
        let id = h.last_admitted + 1;
        let request = Request {
            id,
            generation: h.generation,
            page: "a.md".into(),
            kind: RequestKind::Submit {
                bytes: text("T"),
                version,
                resolve: None,
            },
        };
        assert_eq!(h.admit(request), Disposition::Applied);
        assert_eq!(h.dequeue(), Disposition::Pending);
        h.apply_request();
        saved(h, "a.md");
        h.fs.external("a.md", text("Y"), true);
        assert_eq!(h.observe("a.md"), Disposition::Applied);
    });
    let delivery = u.collect();
    let (_, mail, facts) = &delivery.mail[0];
    assert_eq!(mail.page.as_ref().unwrap().buf, text("Y"));
    assert!(
        facts.content,
        "a newer page state coalesced into the answer"
    );
}

#[test]
fn a_refused_answer_is_typed_in_its_mail() {
    let mut u = Unit::new();
    let kind = RequestKind::Submit {
        bytes: text("x"),
        version: 1,
        resolve: None,
    };
    u.progress.with_host(|h| {
        h.subscriptions.insert("b.md".into());
        send(h, "b.md", kind)
    });
    let delivery = u.collect();
    let facts = delivery
        .mail
        .iter()
        .find(|(page, ..)| page == "b.md")
        .map(|(_, _, facts)| facts)
        .expect("the refused answer is mailed");
    assert_eq!(facts.refused, Some(Refusal::NotHeld));
}

/// A real graph and app-data directory with a running page host.
struct Live {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Arc<Store>,
    host: PageHost,
    mail: mpsc::Receiver<PageMail>,
    /// Each published page's edit kinds, as the host's kinds sink took them.
    kinds: Kinds,
    id: std::cell::Cell<u64>,
}

/// Published pages' edit kinds in delivery order (OG-RULES Rule 8).
type Kinds = Arc<Mutex<Vec<(PageKey, Vec<EditKind>)>>>;

/// A kinds sink that records into `kinds`.
fn recording(kinds: &Kinds) -> impl FnMut(&str, &[EditKind]) + Send + 'static {
    let kinds = kinds.clone();
    move |page, taken| kinds.lock().unwrap().push((page.into(), taken.to_vec()))
}

impl Live {
    fn new(files: &[(&str, &str)]) -> Self {
        Self::with(files, false)
    }

    /// `checkpoint`: the store keeps a launch checkpoint in the app dir.
    fn with(files: &[(&str, &str)], checkpoint: bool) -> Self {
        let dir = match std::env::var_os("TINE_HOST_FS_ROOT") {
            Some(base) => tempfile::tempdir_in(base).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let graph = dir.path().join("graph");
        for area in ["pages", "journals"] {
            fs::create_dir_all(graph.join(area)).unwrap();
        }
        for (path, body) in files {
            fs::write(graph.join(path), body).unwrap();
        }
        let app = dir.path().join("app");
        fs::create_dir_all(&app).unwrap();
        // The root in a non-canonical spelling (as Windows temp dirs come):
        // the host's holds, the watcher's events and the build's lookups
        // must all key pages by the store's resolved root (A-V4).
        let given = graph.join("pages").join("..");
        let options = crate::OpenOptions {
            launch_checkpoint: checkpoint.then(|| app.join("checkpoint.bin")),
            ..Default::default()
        };
        let store = Arc::new(Store::open(&given, options).unwrap().0);
        // A loaded index: what it holds afterwards was published to it.
        store.whole_graph_reconciled().unwrap();
        let root = store.graph.root.clone();
        let (sender, mail) = mpsc::channel();
        let kinds = Kinds::default();
        let host = PageHost::start(&store, &app, "test-graph", recording(&kinds), move |mail| {
            let _ = sender.send(mail);
        })
        .unwrap();
        Self {
            _dir: dir,
            root,
            store,
            host,
            mail,
            kinds,
            id: std::cell::Cell::new(0),
        }
    }

    fn id(&self) -> u64 {
        self.id.set(self.id.get() + 1);
        self.id.get()
    }

    fn wait(&self, key: &str, what: &str, test: impl Fn(&PageMail) -> bool) -> PageMail {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let mail = self
                .mail
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no mail: {what}"));
            assert_eq!(
                mail.session,
                self.host.session(),
                "mail of the current host instance"
            );
            if mail.key == key && test(&mail) {
                return mail;
            }
        }
    }

    fn answer(&self, key: &str, id: u64) -> PageMail {
        self.wait(key, "answer", |mail| {
            mail.answer.as_ref().is_some_and(|a| a.id == id)
        })
    }

    fn open(&self, rel: &str) -> (PageKey, MailPage) {
        let id = self.id();
        let generation = self.host.session();
        let name = rel.rsplit('/').next().unwrap().split('.').next().unwrap();
        let key = self
            .host
            .open(generation, id, &PageId::from(rel), name)
            .unwrap()
            .key;
        let mail = self.answer(&key, id);
        (key, mail.page.unwrap())
    }

    fn dto(&self, rel: &str, body: &str) -> PageDto {
        let path = self.root.join(rel);
        self.store
            .graph
            .page_dto_for_bytes(&path, body.as_bytes())
            .unwrap()
            .unwrap()
    }

    fn submit(
        &self,
        key: &str,
        body: &str,
        version: u64,
        resolve: Option<&DiskToken>,
    ) -> Result<u64, PageRefusal> {
        let id = self.id();
        let dto = self.dto(key, body);
        let generation = self.host.session();
        self.host
            .submit(
                generation,
                id,
                key,
                &dto,
                version,
                resolve,
                &[EditKind::SaveBlock],
            )
            .map(|()| id)
    }

    fn external(&self, key: &str, body: &str) -> MailPage {
        // The watcher forwards the read of a held page to the host (§5).
        fs::write(self.root.join(key), body).unwrap();
        let token = token(body);
        self.wait(key, "observation", |mail| {
            mail.page
                .as_ref()
                .is_some_and(|p| p.disk == Some(token.clone()))
        })
        .page
        .unwrap()
    }

    /// Pause or resume request application and the draft worker (Q-P2b-4).
    /// Window Submits keep the edit debounce (`TestKnobs::submits_wait`).
    fn submits_wait(&self) {
        (self.host.driver.shared)
            .with_state_for_tests(|state| state.progress.host.test.submits_wait = true);
    }

    fn pause(&self, paused: bool) {
        self.host
            .driver
            .shared
            .with_state_for_tests(|state| state.progress.host.test.paused = paused);
    }

    fn disk(&self, key: &str) -> String {
        fs::read_to_string(self.root.join(key)).unwrap()
    }

    fn until_disk(&self, key: &str, body: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while fs::read_to_string(self.root.join(key)).ok().as_deref() != Some(body) {
            assert!(Instant::now() < deadline, "{key} never saved {body:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Tear down a host failed by construction (its state poisoned) without
/// aborting the test binary. Its own teardown panics, and so may its
/// store's: a driver that panicked inside a delivery poisoned the store's
/// writer lock. Dropped together, the store's panic would land during the
/// host's unwind, which aborts; apart, each panic is caught.
fn drop_failed(live: Live) {
    let store = Arc::clone(&live.store);
    drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || drop(live),
    )));
    drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || drop(store),
    )));
}

impl Live {
    /// The index's revision of `key`'s page.
    fn indexed(&self, key: &str) -> Option<String> {
        self.store.graph.cached_rev(&self.root.join(key))
    }

    fn until_indexed(&self, key: &str, body: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.indexed(key) != Some(content_rev(body)) {
            assert!(Instant::now() < deadline, "{key} never indexed {body:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn until_published(&self, pages: &[(String, u64, Option<String>)]) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.host.pages_published(pages) {
            assert!(Instant::now() < deadline, "never published: {pages:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn until_recoverable(&self, pages: &[(String, u64)]) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.host.pages_recoverable(pages) {
            assert!(Instant::now() < deadline, "never recoverable: {pages:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The watcher left `key`'s index to the publication consumer.
    fn held(&self, key: &str) -> bool {
        self.store.watch.holds(&self.root.join(key))
    }

    /// Fail the next `n` index publications.
    fn index_faults(&self, n: u32) {
        let faults = &self.host.index_faults;
        faults.store(n, std::sync::atomic::Ordering::Release);
    }

    fn close(&self, key: &str) {
        let generation = self.host.session();
        self.host.close(generation, self.id(), key).unwrap();
    }
}

fn token(body: &str) -> DiskToken {
    DiskToken::of(&text(body))
}

/// Longer than a save's debounce and first retry: a save that could run
/// has run.
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

#[test]
fn open_submit_and_mail_round_trip_through_a_real_store() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    assert_eq!(key, "pages/a.md");
    assert_eq!(page.disk, Some(token("- one\n")));
    let MailText::Page { dto } = &page.text else {
        panic!("an open answer carries the page: {:?}", page.text);
    };
    assert_eq!(dto.blocks[0].raw, "one");
    let id = live
        .submit(&key, "- one\n- two\n", page.version, None)
        .unwrap();
    let mail = live.answer(&key, id);
    let answer = mail.answer.unwrap();
    assert!(answer.took);
    assert_eq!(answer.outcome, AnswerOutcome::Applied);
    // P1: the unchanged text's revision is the DTO revision of the buffer,
    // so the window can match it against the text it installed.
    let MailText::Unchanged { rev } = mail.page.unwrap().text else {
        panic!("an answer that took the submit carries no text");
    };
    let typed = live.dto(&key, "- one\n- two\n").rev;
    assert_eq!(rev.map(String::from), typed);
    live.until_disk(&key, "- one\n- two\n");
    // A page with no file yet is opened, then created by its first save.
    let (key, page) = live.open("pages/new.md");
    assert_eq!(page.disk, Some(DiskToken::NoFile));
    live.submit(&key, "- made\n", page.version, None).unwrap();
    live.until_disk(&key, "- made\n");
    // The file is visible at rename, before directory sync and the host's
    // Published transition. Wait for the recovery query itself as well.
    let needs = [(key, page.version + 1)];
    live.until_recoverable(&needs);
    assert!(live.host.pages_recoverable(&needs));
    live.host.stop();
}

/// Q-P2b-3 (GH #538): the save-failure notice names the failed platform
/// step and its OS error, and a later published save clears both.
#[cfg(unix)]
#[test]
fn q_p2b_3_the_save_failure_notice_names_the_platform_step_and_os_error() {
    use std::os::unix::fs::PermissionsExt;
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let pages = live.root.join("pages");
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o555)).unwrap();
    live.submit(&key, "- two\n", page.version, None).unwrap();
    let failed = live.wait(&key, "save error", |mail| mail.notice.save_error);
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(failed.notice.operation, Some("create temporary file"));
    assert_eq!(failed.notice.os_error, Some(libc::EACCES));
    let saved = live.wait(&key, "saved", |mail| !mail.notice.save_error);
    assert_eq!(
        (saved.notice.operation, saved.notice.os_error),
        (None, None)
    );
    live.until_disk(&key, "- two\n");
    live.host.stop();
}

#[test]
fn f9_create_checks_run_at_open_of_a_page_with_no_file() {
    let live = Live::new(&[("pages/c.org", "* c\n")]);
    let generation = live.host.session();
    let refused = live
        .host
        .open(generation, 1, &PageId::from("pages/c.md"), "c");
    assert!(
        matches!(&refused, Err(PageRefusal::Twin { existing }) if existing.ends_with("c.org")),
        "{refused:?}"
    );
    let refused = live
        .host
        .open(generation, 2, &PageId::from("logseq/config.edn"), "config");
    assert!(
        matches!(refused, Err(PageRefusal::InvalidTarget { .. })),
        "{refused:?}"
    );
    assert!(!live.root.join("pages/c.md").exists());
    live.host.stop();
}

/// Q1 under amendment A-A1, the reviewer's literal ABA cut: the window
/// was shown disk A; C is observed before its Keep mine over A serializes;
/// A returns between serialization and application. The old token is stale
/// input (E27, a declared conservative deviation): the corrupt bytes, which
/// were never checked against A, never reach disk, the page stays
/// conflicted, and the follow-up resolve against A runs the firewall and
/// refuses.
fn q1_trace(
    rel: &str,
    (start, mine): (&str, &str),
    a: &str,
    c: &str,
    corrupt: &str,
    refused: impl Fn(&PageRefusal) -> bool,
) {
    let live = Live::new(&[(rel, start)]);
    let (key, opened) = live.open(rel);
    let shown = live.external(&key, a);
    assert!(!shown.conflict);
    // Input typed before A arrived: a conflict over disk A.
    let id = live.submit(&key, mine, opened.version, None).unwrap();
    let page = live.answer(&key, id).page.unwrap();
    assert!(page.conflict);
    assert_eq!(page.disk, Some(token(a)));
    // The firewall's verdicts the trace depends on: against A it refuses,
    // against C it passes.
    let dto = live.dto(&key, corrupt);
    let against = |bytes: &str| live.host.serialize(&key, &dto, Some(&text(bytes)), &[]);
    assert!(against(a).is_err_and(|refusal| refused(&refusal)));
    assert!(against(c).is_ok());
    let current = live.external(&key, c);
    assert!(current.conflict);
    // Serialized while C is current; applied only after A returned.
    let lock = live.store.graph.page_lock(&live.root.join(&key));
    let held = lock.lock().unwrap();
    let id = live
        .submit(&key, corrupt, current.version, Some(&token(a)))
        .expect("an old token is admitted as stale input, not refused");
    fs::write(live.root.join(&key), a).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let owed = || {
        live.host
            .driver
            .shared
            .with_state_for_tests(|s| s.observe.contains_key(&key))
    };
    while !owed() {
        assert!(Instant::now() < deadline, "the watcher never forwarded A");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(held);
    let page = live.answer(&key, id).page.unwrap();
    assert_eq!(
        page.disk,
        Some(token(a)),
        "the cut: A is current at application"
    );
    assert!(
        page.conflict,
        "Q1/A-A1: an old token's request stays conflicted"
    );
    settle();
    assert_eq!(
        live.disk(&key),
        a,
        "Q1/A-A1: bytes unchecked against A never reach disk"
    );
    let again = live.submit(&key, corrupt, page.version, Some(&token(a)));
    assert!(
        again.as_ref().is_err_and(&refused),
        "Q1: Keep mine over A compares with A: {again:?}"
    );
    settle();
    assert_eq!(live.disk(&key), a);
    live.host.stop();
}

#[test]
fn q1_keep_mine_over_a_changed_disk_compares_only_with_its_token_markdown_header() {
    q1_trace(
        "pages/p.md",
        ("- start\n", "- mine\n"),
        "A:: XX\nB:: XX\n",
        "A:: XX\n\n- B:: XX\n",
        "A:: XX\n\n- B:: XX\n- mine\n",
        |refusal| matches!(refusal, PageRefusal::Failed { message } if message.contains("page-header property")),
    );
}

#[test]
fn q1_keep_mine_over_a_changed_disk_compares_only_with_its_token_org_roundtrip() {
    q1_trace(
        "pages/o.org",
        ("* start\n", "* mine\n"),
        "* Parent\n*** [[x]]\n",
        "* Parent\n** [[x]]\n",
        "* Parent\n** [[x]]\n** mine\n",
        |refusal| matches!(refusal, PageRefusal::ReadOnly { .. }),
    );
}

/// Q1, the conflict-cleared case: the window was shown disk X, then disk
/// went back to the page's base and the conflict cleared before its Keep
/// mine arrived. The token names bytes the host no longer holds, so the
/// request is stale input: a conflict with an unknown base, never an
/// ordinary save serialized against nothing (that would bypass the GH #163
/// firewall over the header page now on disk).
#[cfg(unix)]
#[test]
fn q1_an_old_token_after_the_conflict_cleared_is_stale_input() {
    use std::os::unix::fs::PermissionsExt;
    let header = "A:: XX\nB:: XX\n";
    let live = Live::new(&[("pages/h.md", header)]);
    let (key, opened) = live.open("pages/h.md");
    let pages = live.root.join("pages");
    let mode = |bits| fs::set_permissions(&pages, fs::Permissions::from_mode(bits)).unwrap();
    // Saves fail (no temp file in a read-only directory); the input stays
    // typed over base A/B.
    mode(0o555);
    let id = live
        .submit(&key, "A:: XX\nB:: XX\n\n- typed\n", opened.version, None)
        .unwrap();
    live.answer(&key, id);
    let shown = live.external(&key, "- other\n");
    assert!(shown.conflict);
    let current = live.external(&key, header);
    assert!(
        !current.conflict,
        "disk back at the base clears the conflict"
    );
    let corrupt = "A:: XX\n\n- B:: XX\n- typed\n";
    let id = live
        .submit(&key, corrupt, current.version, Some(&token("- other\n")))
        .unwrap();
    let page = live.answer(&key, id).page.unwrap();
    mode(0o755);
    assert!(page.conflict, "Q1: an old token is stale input");
    settle();
    assert_eq!(
        live.disk(&key),
        header,
        "the header page was never rewritten"
    );
    live.host.stop();
}

/// R13 (§5): an open page's index has one writer, the publication
/// consumer. The watcher forwards an external change of it to the host and
/// leaves its index alone, also while the consumer's publication fails;
/// the consumer retries with the save backoff, reports the third failure,
/// and the retry indexes the change. An unheld page stays the watcher's.
#[test]
fn r13_a_held_page_is_indexed_by_the_consumer_alone_and_a_failed_publication_retries() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    let (key, page) = live.open("pages/a.md");
    live.until_published(&[(key.clone(), page.version, None)]);
    assert!(live.held(&key));
    assert!(!live.held("pages/b.md"));
    live.index_faults(3);
    let page = live.external(&key, "- two\n");
    assert!(!page.conflict, "a clean page adopts the external change");
    // The watcher indexes an unheld page meanwhile: it is running.
    fs::write(live.root.join("pages/b.md"), "- b2\n").unwrap();
    live.until_indexed("pages/b.md", "- b2\n");
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- one\n")),
        "R13: the watcher must not index a held page"
    );
    live.wait(&key, "the third failure is reported", |mail| {
        mail.notice.index_error
    });
    live.until_indexed(&key, "- two\n");
    live.wait(&key, "the report clears", |mail| !mail.notice.index_error);
    let Live {
        host, store, root, ..
    } = live;
    host.stop();
    assert!(
        !store.watch.holds(&root.join(&key)),
        "a stopped host hands every page back"
    );
}

/// Q6 (REVIEW-3): Close is not eviction. A page closed by its window with
/// a submit answered but not yet saved stays held, and the consumer indexes
/// its save; only once the host retires it does its index return to the
/// watcher, which then indexes the next external change.
#[test]
fn q6_a_dirty_page_closed_before_its_save_stays_held_until_retired() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live
        .submit(&key, "- one\n- two\n", page.version, None)
        .unwrap();
    live.answer(&key, id);
    live.close(&key);
    // Inside the save's debounce: the host still holds the dirty page.
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        live.held(&key),
        "Q6: a surface Close of a dirty page must not hand its index over"
    );
    live.until_disk(&key, "- one\n- two\n");
    live.until_indexed(&key, "- one\n- two\n");
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(&key) {
        assert!(
            Instant::now() < deadline,
            "the retired page is never handed back"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(live.root.join(&key), "- three\n").unwrap();
    live.until_indexed(&key, "- three\n");
    live.host.stop();
}

/// Q2 (REVIEW-3): the block-reference barrier needs a completed
/// publication whose bytes hold the target id. Target submit v2 adds the
/// id; v3, admitted before v2's save, removes it; only v3's bytes publish.
/// The watermark passes v2, and a local restoration of the id would still
/// be unsent, yet no publication ever held the id: the barrier fails.
#[test]
fn q2_a_block_reference_barrier_needs_the_id_in_the_published_bytes() {
    const ID: &str = "64d7c5e0-0000-4000-8000-000000000001";
    let live = Live::new(&[("pages/t.md", "- target\n")]);
    let (key, page) = live.open("pages/t.md");
    let stamped = format!("- target\n  id:: {ID}\n");
    let id = live.submit(&key, &stamped, page.version, None).unwrap();
    let v2 = live.answer(&key, id).answer.unwrap().version;
    let id = live.submit(&key, "- target\n- more\n", v2, None).unwrap();
    let v3 = live.answer(&key, id).answer.unwrap().version;
    live.until_published(&[(key.clone(), v3, None)]);
    assert_eq!(live.disk(&key), "- target\n- more\n");
    let barrier = |version| vec![(key.clone(), version, Some(ID.to_string()))];
    assert!(live.host.pages_published(&[(key.clone(), v2, None)]));
    assert!(
        !live.host.pages_published(&barrier(v2)),
        "Q2: no publication of the target held the id"
    );
    let id = live.submit(&key, &stamped, v3, None).unwrap();
    let v4 = live.answer(&key, id).answer.unwrap().version;
    live.until_published(&barrier(v4));
    assert!(live.disk(&key).contains(ID));
    assert!(live.host.pages_published(&barrier(v2)));
    // Typing on: the page is dirty again, and v4's completed publication
    // (the index watermark) still answers the barrier.
    let more = format!("{stamped}- typing\n");
    let id = live.submit(&key, &more, v4, None).unwrap();
    live.answer(&key, id);
    assert!(
        live.host.pages_published(&barrier(v4)),
        "§4.4: a completed publication at v4 satisfies a v4 barrier"
    );
    live.host.stop();
}

/// Q5 (REVIEW-3): an untouched opened page owes no save. Once its Open
/// read is indexed it is published at its version, with no save receipt;
/// a page the host does not hold has nothing owed.
#[test]
fn q5_a_clean_open_page_is_published_without_a_save() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let modified = || {
        fs::metadata(live.root.join("pages/a.md"))
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = modified();
    let (key, page) = live.open("pages/a.md");
    live.until_published(&[(key.clone(), page.version, None)]);
    assert!(live
        .host
        .pages_published(&[("pages/other.md".into(), 9, None)]));
    settle();
    assert_eq!(modified(), before, "no save of an untouched page");
    live.host.stop();
}

// ---- STEP3 §6/§7: switch and restore through the binding ----

impl Live {
    fn app(&self) -> PathBuf {
        self._dir.path().join("app")
    }

    /// Page draft vehicles in the binding's draft directory.
    fn drafts(&self) -> Vec<String> {
        let dir = self.app().join("drafts-v2").join("test-graph");
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().unwrap().is_file())
            .map(|entry| entry.file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Fail the next `n` calls of `phase` in the host's file adapter.
    fn faults(&self, phase: super::io::Phase, n: usize) {
        self.host.driver.shared.with_state_for_tests(|state| {
            let queue = state.progress.host.fs.faults.entry(phase).or_default();
            queue.extend(std::iter::repeat_n(std::io::ErrorKind::Other, n));
        });
    }

    /// The stop's outcome, once it is not Waiting.
    fn until_stop(&self) -> StopState {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self.host.stop_state() {
                StopState::Waiting => {
                    assert!(Instant::now() < deadline, "the stop never settled");
                    std::thread::sleep(Duration::from_millis(20));
                }
                state => return state,
            }
        }
    }
}

/// §6: a switch saves the unsaved edit and stops with no draft; stopping
/// joins the driver and the watcher indexes the page again.
#[test]
fn a_switch_saves_an_unsaved_edit_and_stops_with_no_draft() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live
        .submit(&key, "- one\n- two\n", page.version, None)
        .unwrap();
    live.answer(&key, id);
    assert!(live.host.stop_begin(id, StopMode::Switch));
    assert_eq!(
        live.submit(&key, "- late\n", page.version, None),
        Err(PageRefusal::NotAdmitted)
    );
    assert_eq!(live.until_stop(), StopState::Ready);
    assert_eq!(live.drafts(), Vec::<String>::new());
    assert_eq!(live.disk(&key), "- one\n- two\n");
    let path = live.root.join(&key);
    let Live { host, store, .. } = live;
    assert!(host.stop_finish().is_ok());
    assert!(!store.watch.holds(&path));
    assert_eq!(
        store.graph.cached_rev(&path),
        Some(content_rev("- one\n- two\n"))
    );
    fs::write(&path, "- three\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while store.graph.cached_rev(&path) != Some(content_rev("- three\n")) {
        assert!(Instant::now() < deadline, "the watcher indexes it again");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// §7 restore: the stop saves every drained edit, writes no draft and
/// saves no untouched page (Q5); a fresh host then serves the restored
/// tree.
#[test]
fn a_restore_stop_saves_every_edit_and_a_fresh_host_reads_the_restored_tree() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- bee\n")]);
    let modified = |live: &Live| {
        fs::metadata(live.root.join("pages/b.md"))
            .unwrap()
            .modified()
            .unwrap()
    };
    let before = modified(&live);
    let (a, page) = live.open("pages/a.md");
    live.open("pages/b.md");
    let id = live
        .submit(&a, "- one\n- two\n", page.version, None)
        .unwrap();
    live.answer(&a, id);
    assert!(live.host.stop_begin(id, StopMode::Restore));
    assert_eq!(live.until_stop(), StopState::Ready);
    assert_eq!(live.disk(&a), "- one\n- two\n");
    assert_eq!(live.drafts(), Vec::<String>::new());
    assert_eq!(modified(&live), before, "Q5: no save of an untouched page");
    let app = live.app();
    let Live {
        _dir,
        root,
        store,
        host,
        ..
    } = live;
    assert!(host.stop_finish().is_ok());
    // The restore, under the writer, while no host exists.
    {
        let _writer = store.writer.lock().unwrap();
        fs::write(root.join(&a), "- restored\n").unwrap();
    }
    let (sender, mail) = mpsc::channel();
    let kinds = Kinds::default();
    let host = PageHost::start(&store, &app, "test-graph", recording(&kinds), move |mail| {
        let _ = sender.send(mail);
    })
    .unwrap();
    let live = Live {
        _dir,
        root,
        store,
        host,
        mail,
        kinds,
        id: std::cell::Cell::new(100),
    };
    let (_, page) = live.open("pages/a.md");
    assert_eq!(page.disk, Some(token("- restored\n")));
    live.host.stop();
}

/// §7 restore step 3: a conflict aborts the restore with the affected
/// page; aborting reopens admission and nothing is stopped.
#[test]
fn a_restore_stop_aborts_on_a_conflict_and_reopens_admission() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- mine\n", page.version, None).unwrap();
    live.answer(&key, id);
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    assert!(live.host.stop_begin(id, StopMode::Restore));
    assert_eq!(
        live.until_stop(),
        StopState::Aborted(BTreeSet::from([key.clone()]))
    );
    live.host.stop_abort();
    assert_eq!(live.host.stop_state(), StopState::Waiting);
    assert!(live.submit(&key, "- again\n", page.version, None).is_ok());
    assert_eq!(live.disk(&key), "- theirs\n");
    live.host.stop();
}

/// §6 step 5: when the page cannot be saved and its draft cannot be
/// written, the switch aborts with that page instead of closing over it.
#[test]
fn a_switch_aborts_when_neither_the_save_nor_the_draft_can_be_written() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.faults(super::io::Phase::PageTemp, 100);
    live.faults(super::io::Phase::DraftTemp, 100);
    assert!(live.host.stop_begin(id, StopMode::Switch));
    assert_eq!(
        live.until_stop(),
        StopState::Aborted(BTreeSet::from([key.clone()]))
    );
    live.host.stop_abort();
    assert_eq!(live.host.stop_state(), StopState::Waiting);
    live.host.stop();
}

// ---- STEP3 §7: retained writers ----

impl Live {
    /// A retained transaction's write of `body` over `key`'s `old` bytes.
    fn transaction(&self, key: &str, old: &str, body: &str) {
        let mut tx = self.store.transaction(None);
        let dto = self.dto(key, body);
        let base = crate::SaveBase::Existing(FileRev::from_bytes(old.as_bytes()));
        tx.save_page(&[EditKind::SaveBlock], &PageId::from(key), base, &dto);
        assert!(matches!(tx.commit(), crate::TxOutcome::Committed { .. }));
    }
}

/// Q3 (REVIEW-3), first trace: the referrer is flushed at v2, then a
/// submit applies v3 before the reservation. The input contract is checked
/// under the final reservation: a refusing writer refuses, a flush-first
/// writer gets the page only once v3 is saved.
#[test]
fn q3_input_applied_before_the_reservation_is_found_under_it() {
    let live = Live::new(&[("pages/r.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/r.md");
    let id = live.submit(&key, "- v2\n", page.version, None).unwrap();
    let v2 = live.answer(&key, id).answer.unwrap().version;
    live.until_published(&[(key.clone(), v2, None)]);
    let id = live.submit(&key, "- v3\n", v2, None).unwrap();
    live.answer(&key, id);
    let discover = || vec![PageId::from("pages/r.md")];
    assert_eq!(
        live.host.reserve(discover, Input::Refuse).unwrap_err(),
        ReserveRefusal::Unsaved(BTreeSet::from([PageId::from(key.clone())])),
        "Q3: a typed page that is not at risk still has unsaved input"
    );
    let reservation = live.host.reserve(discover, Input::Flush).unwrap();
    assert_eq!(live.disk(&key), "- v3\n", "flushed before the write");
    live.transaction(&key, "- v3\n", "- rewritten\n");
    drop(reservation);
    let page = live.wait(&key, "observed", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|p| p.disk == Some(token("- rewritten\n")))
    });
    assert!(!page.page.unwrap().conflict);
    live.host.stop();
}

/// Q3 (REVIEW-3), second trace: discovery grows under the reservation to
/// a page with unsaved input that was never in the first set.
#[test]
fn q3_a_page_discovered_under_the_reservation_is_checked_too() {
    let live = Live::new(&[("pages/a.md", "- a\n"), ("pages/r.md", "- r\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/r.md");
    let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
    live.answer(&key, id);
    let mut round = 0;
    let discover = || {
        round += 1;
        let mut pages = vec![PageId::from("pages/a.md")];
        if round > 1 {
            pages.push(PageId::from("pages/r.md"));
        }
        pages
    };
    assert_eq!(
        live.host.reserve(discover, Input::Refuse).unwrap_err(),
        ReserveRefusal::Unsaved(BTreeSet::from([PageId::from(key.clone())]))
    );
    assert!(
        retained(&live.host).is_empty(),
        "a refused reservation is withdrawn"
    );
    live.host.stop();
}

/// The retained keys, read through the driver's queue: after every
/// reservation release sent before it.
fn retained(host: &PageHost) -> BTreeSet<PageKey> {
    let read = host
        .driver
        .shared
        .read(|state| state.progress.host.retained.clone());
    read.unwrap()
}

/// A-R1, REVIEW-3a2 R1 probe 2: discovery names A, then panics when it
/// runs again under the fenced reservation; the unwind withdraws A.
#[test]
fn a_rediscovery_panic_withdraws_the_reservation() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let mut round = 0;
    let discover = || {
        round += 1;
        assert!(round == 1, "the rediscovery panics");
        vec![PageId::from("pages/a.md")]
    };
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        live.host.reserve(discover, Input::Flush)
    }));
    assert!(unwound.is_err());
    let retained = retained(&live.host);
    assert!(
        retained.is_empty(),
        "A-R1: a rediscovery panic leaked the reservation: {retained:?}"
    );
    live.host.stop();
}

/// E90 (REVIEW-3a3): a reservation dropped while its host's state is
/// poisoned under it returns without a second panic, which during an
/// unwind would abort the app. The poison lands just before the drop's
/// lock, past where a separate `is_poisoned` check would have looked.
#[test]
fn a_reservation_dropped_on_a_poisoned_host_does_not_panic() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/a.md")], Input::Refuse)
        .unwrap();
    let shared = Arc::clone(&live.host.driver.shared);
    retained::BEFORE_RELEASE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let poisoner = std::thread::spawn(move || {
                let _state = shared.state_for_tests();
                panic!("E90 test: poison the host state");
            });
            assert!(poisoner.join().is_err());
        }))
    });
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(reservation)));
    // Torn down before the assertion: a failure's unwind would meet the
    // failed host's panicking teardown and abort the binary.
    drop_failed(live);
    assert!(
        dropped.is_ok(),
        "E90: dropping a reservation on a poisoned host panicked; exemplar binding_retained.rs \
         Reservation::drop (a notice)"
    );
}

/// REVIEW-3a4 #2 under (G) as amended 2026-10-11: a failed first
/// publication of an unchanged file leaves its row, derived from the bytes
/// the Open read read; a file changed behind the index is withheld
/// (`an_opens_hold_of_a_changed_file_installs_no_row`).
#[test]
fn a_failed_first_publication_of_an_unchanged_file_keeps_its_row() {
    let body = "title:: Disk\n- a\n";
    let live = Live::new(&[("pages/a.md", body)]);
    let named = || {
        matches!(
            live.store.whole_graph().unwrap().resolve("Disk", false),
            crate::Resolved::Existing { .. }
        )
    };
    assert!(named());
    live.index_faults(u32::MAX);
    let (key, _page) = live.open("pages/a.md");
    let failed =
        || (live.host.driver.shared).with_state_for_tests(|s| s.book.retry.contains_key(&key));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !failed() {
        assert!(
            Instant::now() < deadline,
            "the Open read was never published"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(live.indexed(&key), Some(content_rev(body)));
    assert!(named(), "the Open read's own bytes name the page");
    live.index_faults(0);
    live.host.stop();
}

/// REVIEW-G F1: a file written after an open's hold kept its row and before
/// the host's Open read: the Open read's publication replaces the kept row,
/// so the old name does not outlive it.
#[test]
fn an_external_write_after_the_hold_is_published_by_the_open_read() {
    let (old, new) = ("title:: Disk\n- a\n", "title:: Edited\n- e\n");
    let live = Live::new(&[("pages/a.md", old)]);
    let names = |name: &str| {
        matches!(
            live.store.whole_graph().unwrap().resolve(name, false),
            crate::Resolved::Existing { .. }
        )
    };
    live.pause(true);
    let id = live.id();
    let opened = live
        .host
        .open(live.host.session(), id, &PageId::from("pages/a.md"), "a")
        .unwrap();
    assert_eq!(
        live.indexed(&opened.key),
        Some(content_rev(old)),
        "the hold kept the row"
    );
    fs::write(live.root.join("pages/a.md"), new).unwrap();
    live.pause(false);
    let page = live.answer(&opened.key, id).page.unwrap();
    assert_eq!(
        page.disk,
        Some(token(new)),
        "the Open read read the new bytes"
    );
    live.until_indexed(&opened.key, new);
    assert!(names("Edited"));
    assert!(
        !names("Disk"),
        "the kept row outlived the Open read's publication"
    );
    live.host.stop();
}

/// Q6 (REVIEW-3), second boundary: publication P fails and awaits retry;
/// a retained writer reserves the page and commits T, which publishes its
/// own index; P's retry must never overwrite T, and the release's
/// observation leaves the index at T.
#[test]
fn q6_a_failed_publication_never_overwrites_a_retained_transaction() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    live.index_faults(u32::MAX);
    let id = live.submit(&key, "- p\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.until_disk(&key, "- p\n");
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/a.md")], Input::Flush)
        .unwrap();
    live.index_faults(0);
    live.transaction(&key, "- p\n", "- t\n");
    live.until_indexed(&key, "- t\n");
    // Past P's retry backoff, with the driver woken throughout (any wake
    // source does): the retry waits for the release.
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        live.host.driver.shared.with_state_for_tests(|_| {});
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- t\n")),
        "Q6: P's retry overwrote the retained transaction's index"
    );
    drop(reservation);
    live.wait(&key, "observed", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|p| p.disk == Some(token("- t\n")))
    });
    settle();
    assert_eq!(live.indexed(&key), Some(content_rev("- t\n")));
    live.host.stop();
}

/// V2 (REVIEW-3a): the release's observation of a retained transaction
/// fails; until an observation succeeds, P's old due retry still must not
/// overwrite the transaction's index. Once the read recovers, the page's
/// publications flow again.
#[test]
fn v2_a_failed_release_observation_keeps_the_handover_protection() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    live.index_faults(u32::MAX);
    let id = live.submit(&key, "- p\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.until_disk(&key, "- p\n");
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/a.md")], Input::Flush)
        .unwrap();
    live.index_faults(0);
    live.transaction(&key, "- p\n", "- t\n");
    live.until_indexed(&key, "- t\n");
    settle();
    live.faults(super::io::Phase::Read, 1_000_000);
    drop(reservation);
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        live.host.driver.shared.with_state_for_tests(|_| {});
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- t\n")),
        "V2: P's retry overwrote the retained transaction's index after a failed observation"
    );
    live.host.driver.shared.with_state_for_tests(|state| {
        state
            .progress
            .host
            .fs
            .faults
            .remove(&super::io::Phase::Read);
    });
    let page = live.wait(&key, "observed", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|p| p.disk == Some(token("- t\n")))
    });
    settle();
    assert_eq!(live.indexed(&key), Some(content_rev("- t\n")));
    let id = live
        .submit(&key, "- after\n", page.page.unwrap().version, None)
        .unwrap();
    live.answer(&key, id);
    live.until_indexed(&key, "- after\n");
    live.host.stop();
}

/// V3 (REVIEW-3a): an own save of P publishes P while the last observation
/// is still A; an honest external writer restores A before the host ever
/// observes P. Adopting A must publish A, although it equals the old
/// observation.
#[test]
fn v3_an_external_return_to_the_pre_save_observation_updates_the_index() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let (key, page) = live.open("pages/a.md");
    let writer = live.store.writer.lock().unwrap();
    let id = live.submit(&key, "- p\n", page.version, None).unwrap();
    let saved = live.answer(&key, id).page.unwrap().version;
    live.until_disk(&key, "- p\n");
    fs::write(live.root.join(&key), "- a\n").unwrap();
    drop(writer);
    live.wait(&key, "the adopted return", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|p| p.disk == Some(token("- a\n")) && p.version > saved)
    });
    settle();
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- a\n")),
        "V3: the index stayed at the saved bytes"
    );
    live.host.stop();
}

/// N3 (REVIEW-3b-P2b-2, Codex's plant): a watcher blocked on the store
/// writer with an external change pending is as quiet as an idle one, so
/// quiet counters cannot be a measurement boundary. The acknowledged drain
/// is not declared while the reconcile is blocked, and once declared the
/// pending change is in the index.
#[test]
fn n3_a_watcher_blocked_on_the_writer_is_not_drained() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let key = "pages/a.md";
    let drained = |seconds| live.host.quiesce_for_tests(Duration::from_secs(seconds));
    assert!(drained(20), "the fresh host settles");
    let writer = live.store.writer.lock().unwrap();
    fs::write(live.root.join(key), "- external\n").unwrap();
    assert!(
        !drained(5),
        "N3: drained while the watcher's reconcile waited on the writer"
    );
    assert_eq!(live.indexed(key), Some(content_rev("- a\n")), "plant held");
    drop(writer);
    assert!(drained(20), "the released watcher drains");
    assert_eq!(
        live.indexed(key),
        Some(content_rev("- external\n")),
        "N3: drained before the pending change was reconciled"
    );
    live.host.stop();
}

/// Sentinel files of drain requests now in the graph root.
fn sentinels(live: &Live) -> usize {
    fs::read_dir(&live.root)
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            name.to_string_lossy().starts_with(".tine-watch-drain-")
        })
        .count()
}

/// A drain of `live`'s watcher in its own thread, false after `seconds`.
fn drain<'a>(
    scope: &'a std::thread::Scope<'a, '_>,
    live: &'a Live,
    seconds: u64,
) -> std::thread::ScopedJoinHandle<'a, bool> {
    let watch = &live.store.watch;
    scope.spawn(move || watch.drained_for_tests(Duration::from_secs(seconds)))
}

/// After a stale-acknowledgement plant: the gate opened and the writer
/// released, the watcher drains and the external change is indexed.
fn drains_after_the_plant(live: Live, key: &str) {
    live.store.watch.gate_notifications(None);
    assert!(live.host.quiesce_for_tests(Duration::from_secs(20)));
    assert_eq!(live.indexed(key), Some(content_rev("- external\n")));
    live.host.stop();
}

/// REVIEW-3b-P2b-3 N3 (Codex's first plant): an older drain's sentinel
/// notification, delivered after a newer drain was requested, acknowledges
/// only the older request. The external change written between the two
/// requests is still held behind it, so the newer drain is not declared.
#[test]
fn n3_an_older_sentinel_does_not_acknowledge_a_newer_drain() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let key = "pages/a.md";
    assert!(live.host.quiesce_for_tests(Duration::from_secs(20)));
    live.store.watch.gate_notifications(Some(0));
    let writer = live.store.writer.lock().unwrap();
    std::thread::scope(|scope| {
        let older = drain(scope, &live, 20);
        until_sentinels(&live, 1);
        fs::write(live.root.join(key), "- external\n").unwrap();
        let newer = drain(scope, &live, 3);
        until_sentinels(&live, 2);
        // The older sentinel's notification only; the rest stay held.
        live.store.watch.gate_notifications(Some(1));
        assert!(
            !newer.join().unwrap(),
            "N3: an older sentinel acknowledged a newer drain"
        );
        assert!(older.join().unwrap(), "the older drain is acknowledged");
    });
    assert_eq!(live.indexed(key), Some(content_rev("- a\n")), "plant held");
    drop(writer);
    drains_after_the_plant(live, key);
}

/// REVIEW-3b-P2b-3 N3 (Codex's second plant): one sentinel write raises
/// more than one notification (and its removal another). Those that arrive
/// after a later drain was requested acknowledge their own, completed
/// request, never the later one.
#[test]
fn n3_a_late_notification_of_a_completed_drain_acknowledges_only_it() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let key = "pages/a.md";
    assert!(live.host.quiesce_for_tests(Duration::from_secs(20)));
    live.store.watch.gate_notifications(Some(1));
    assert!(
        live.store.watch.drained_for_tests(Duration::from_secs(20)),
        "the earlier drain is acknowledged by its sentinel's first notification"
    );
    let writer = live.store.writer.lock().unwrap();
    fs::write(live.root.join(key), "- external\n").unwrap();
    std::thread::scope(|scope| {
        let later = drain(scope, &live, 3);
        until_sentinels(&live, 1);
        // One more notification: the earlier sentinel's next, held before
        // the page's.
        live.store.watch.gate_notifications(Some(1));
        assert!(
            !later.join().unwrap(),
            "N3: a late notification of an earlier drain acknowledged a later one"
        );
    });
    assert_eq!(live.indexed(key), Some(content_rev("- a\n")), "plant held");
    drop(writer);
    drains_after_the_plant(live, key);
}

fn until_sentinels(live: &Live, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while sentinels(live) < count {
        assert!(Instant::now() < deadline, "never {count} drain requests");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A3 (REVIEW-3a): page mail's DTO (`page_dto_for_bytes`, its only
/// conversion) names the page from the buffer it carries, not from a fresh
/// read of the file: disk says `title:: Disk` while a conflicted or
/// recovered buffer says `title:: Mine`; and a file that is gone never
/// blanks a valid buffer's name.
#[test]
fn a3_page_mail_names_the_page_from_its_buffer() {
    let live = Live::new(&[("pages/p.md", "title:: Disk\n\n- one\n")]);
    let graph = &live.store.graph;
    let dto = graph
        .page_dto_for_bytes(&live.root.join("pages/p.md"), b"title:: Mine\n\n- mine\n")
        .unwrap()
        .unwrap();
    assert_eq!(
        (dto.name.as_str(), dto.title.as_str()),
        ("Mine", "Mine"),
        "A3"
    );
    let dto = graph
        .page_dto_for_bytes(&live.root.join("pages/gone.md"), b"title:: Fresh\n\n- x\n")
        .unwrap()
        .unwrap();
    assert_eq!(dto.name, "Fresh", "A3");
    live.host.stop();
}

/// The binding notice the last delivery mailed for `page`, if any.
fn mailed(delivery: &Delivery, page: &str) -> Option<BindingNotice> {
    delivery
        .mail
        .iter()
        .rev()
        .find(|(key, ..)| key == page)
        .map(|(.., facts)| facts.binding.clone())
}

/// Run `page`'s save to its end, collecting after each step.
fn save_through(u: &mut Unit, page: &str, twin_at_rename: bool) -> Option<BindingNotice> {
    let mut notice = None;
    assert_eq!(u.progress.host.start_save(page), Disposition::Pending);
    while let Some(job) = &u.progress.host.job {
        if twin_at_rename && job.phase == SavePhase::Rename {
            u.progress
                .host
                .fs
                .external("c.org", Some(Arc::from(&b"twin"[..])), true);
        }
        u.progress.host.advance_save();
        notice = mailed(&u.collect(), page).or(notice);
    }
    notice
}

/// A2 (REVIEW-3a): a twin reaches the window as a typed notice naming the
/// other file, whether it failed the creating save (before Check) or
/// appeared beside it (after the rename, beside its Published, which it
/// neither invents nor cancels); a later save's publication clears it.
#[test]
fn a2_a_twin_is_a_binding_notice_until_a_later_save_publishes() {
    for after_rename in [false, true] {
        let mut u = Unit::new();
        u.progress.with_host(|h| {
            open_page(h, "c.md");
            edit(h, "c.md", "created");
        });
        u.collect();
        if !after_rename {
            u.progress
                .host
                .fs
                .external("c.org", Some(Arc::from(&b"twin"[..])), true);
        }
        let notice = save_through(&mut u, "c.md", after_rename).unwrap();
        assert_eq!(notice.twin.as_deref(), Some("c.org"), "A2: {after_rename}");
        assert_eq!(u.progress.host.pages["c.md"].clean(), after_rename);
        u.progress.host.fs.external("c.org", None, true);
        if after_rename {
            u.progress.with_host(|h| edit(h, "c.md", "again"));
            u.collect();
        }
        let notice = save_through(&mut u, "c.md", false).unwrap();
        assert!(u.progress.host.pages["c.md"].clean());
        assert_eq!(notice.twin, None, "A2: a later publication clears it");
    }
}

/// A2 (REVIEW-3a), schedule (c): a retained writer releases and the
/// release's observation fails three times running: the window gets a
/// typed notice, and it clears once the read succeeds.
#[test]
fn a2_a_failing_release_observation_is_a_notice_that_clears_on_recovery() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, _) = live.open("pages/a.md");
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/a.md")], Input::Flush)
        .unwrap();
    live.transaction(&key, "- one\n", "- t\n");
    live.faults(super::io::Phase::Read, 1_000_000);
    drop(reservation);
    live.wait(&key, "the third failed read is reported", |mail| {
        mail.notice.observe_error
    });
    live.host.driver.shared.with_state_for_tests(|state| {
        state
            .progress
            .host
            .fs
            .faults
            .remove(&super::io::Phase::Read);
    });
    let mail = live.wait(&key, "the report clears", |mail| !mail.notice.observe_error);
    assert!(mail.page.is_some_and(|p| p.disk == Some(token("- t\n"))));
    live.host.stop();
}

/// A4 (REVIEW-3a): another thread holds page A's path lock while the driver
/// owes A an observation; page B's save falls due meanwhile and must run.
/// The driver yields A's step and retries it, without blocking on A's
/// mutex; once A's lock is free, A is observed.
#[test]
fn a4_a_held_path_lock_does_not_stall_another_pages_due_save() {
    let live = Live::new(&[("pages/a.md", "- a\n"), ("pages/b.md", "- b\n")]);
    let (a, _) = live.open("pages/a.md");
    let (b, page) = live.open("pages/b.md");
    let lock = live.store.graph.page_lock(&live.root.join(&a));
    let held = lock.lock().unwrap();
    fs::write(live.root.join(&a), "- a2\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !live
        .host
        .driver
        .shared
        .with_state_for_tests(|state| state.observe.contains_key(&a))
    {
        assert!(Instant::now() < deadline, "the watcher never forwarded A");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    live.submit(&b, "- b2\n", page.version, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while live.disk(&b) != "- b2\n" {
        assert!(
            Instant::now() < deadline,
            "A4: B's due save waited for A's path lock"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Yielding is no spin: the held lock is retried on a backoff timer.
    let polls = || {
        live.host
            .driver
            .shared
            .with_state_for_tests(|state| state.polls)
    };
    let before = polls();
    std::thread::sleep(Duration::from_secs(1));
    let spent = polls() - before;
    assert!(
        spent < 100,
        "A4: {spent} driver steps in 1 s while A is held"
    );
    drop(held);
    live.wait(&a, "A observed once its lock is free", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|p| p.disk == Some(token("- a2\n")))
    });
    live.host.stop();
}

/// V5 (REVIEW-3a): a restore is not ready while a publication the driver
/// collected is still being delivered; once its result is recorded, an
/// index failure keeps the restore waiting and a success lets it close.
#[test]
fn v5_a_restore_waits_for_a_collected_publication_in_flight() {
    for (indexing, ready) in [(Indexing::Failed, false), (Indexing::Indexed, true)] {
        let mut u = Unit::new();
        let version = u.version();
        u.submit("saved", version, None, "saved");
        u.progress.with_host(|h| saved(h, "a.md"));
        let delivery = u.collect();
        assert_eq!(own(&delivery).len(), 1);
        let publication = own(&delivery)[0].clone();
        u.progress
            .with_host(|h| assert_eq!(h.switch_ready(h.last_applied), Disposition::Applied));
        u.progress.stopping = Some(Stopping {
            restore: true,
            failed: BTreeSet::new(),
        });
        assert_eq!(
            stop_state(&u.progress, &u.book),
            StopState::Waiting,
            "V5: the restore was ready with a publication in flight"
        );
        u.book.record(vec![(publication, indexing)], 0);
        let state = stop_state(&u.progress, &u.book);
        assert_eq!(state == StopState::Ready, ready, "{state:?}");
    }
}

#[path = "binding_stop_tests.rs"]
mod stop_saved;

#[path = "binding_rename_tests.rs"]
mod rename;

#[path = "binding_apply_tests.rs"]
mod apply_tests;

#[path = "binding_order_tests.rs"]
mod order;

/// REVIEW-3a V4, amendment A-V4: while a host holds a page, its publication
/// consumer is the page's only index writer. The reviewer's schedule: the
/// host has not observed disk B yet (another transaction holds the page's
/// path lock) when a `Store::page` read and a whole-graph rebuild run. Both
/// answer from the bytes the consumer last indexed, A, and index nothing,
/// so no consumer publication captured before B can regress the index past
/// them; once the host observes B, its consumer indexes B.
/// REVIEW-3a V4 / A-V4: while the host holds a page whose newer disk bytes
/// it has not observed (its read is blocked on the path lock), a read, a
/// forced rebuild answer the bytes the consumer indexed (the on-demand
/// build: `held_index::tests`). Indexing disk there could be overwritten by the host's older
/// pending event.
#[test]
fn v4_a_read_or_rebuild_never_indexes_a_held_page() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let (key, _) = live.open("pages/a.md");
    live.until_indexed(&key, "- a\n");
    let lock = live.store.graph.page_lock(&live.root.join(&key));
    let held = lock.lock().unwrap();
    fs::write(live.root.join(&key), "- b\n").unwrap();
    let read = live.store.page(&PageId::from(key.as_str())).unwrap();
    assert_eq!(
        read.rev,
        FileRev::from_bytes(b"- a\n"),
        "V4: the read answers A"
    );
    assert_eq!(read.doc.blocks[0].raw, "a");
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- a\n")),
        "V4: Store::page indexed a held page from disk"
    );
    assert!(live.store.graph.rebuild_cache_cancellable(|| false));
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- a\n")),
        "V4: the rebuild indexed a held page from disk"
    );
    drop(held);
    live.until_indexed(&key, "- b\n");
    live.host.stop();
}

/// H1 #3 (REVIEW-AH2-AW1-plan R6, (G)): the consumer's external
/// publication names the page from the bytes it publishes, while the file
/// says something else.
#[test]
fn a_consumer_publication_names_the_bytes_it_publishes() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("pages")).unwrap();
    fs::write(temp.path().join("pages/a.md"), "title:: First\n- a\n").unwrap();
    let store = Store::open(temp.path(), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let writer = store.writer.lock().unwrap();
    store.graph.hold_unhosted("pages/a.md");
    fs::write(temp.path().join("pages/a.md"), "title:: Disk\n- d\n").unwrap();
    let publication = Publication {
        key: "pages/a.md".into(),
        spelling: "pages/a.md".into(),
        bytes: Some(Arc::from(&b"title:: Owner\n- o\n"[..])),
        own: None,
        stamp: None,
    };
    assert!(index(&store, &publication));
    drop(writer);
    let view = store.whole_graph().unwrap();
    assert!(
        matches!(
            view.resolve("Owner", false),
            crate::Resolved::Existing { .. }
        ),
        "(G): the publication's own name was not published"
    );
    assert!(
        !matches!(
            view.resolve("Disk", false),
            crate::Resolved::Existing { .. }
        ),
        "(G): the publication named the page from its file"
    );
    store.close();
}

/// E101 (P1): every host instance and every window reload draws a fresh
/// session. A command of an earlier session is never admitted, whatever
/// version it carries; ids continue above the host's admitted watermark.
#[test]
fn e101_every_host_instance_and_reload_draws_a_fresh_session() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let first = live.host.session();
    let (key, page) = live.open("pages/a.md");
    let reloaded = live.host.window_reloaded();
    assert_eq!(reloaded.session, live.host.session());
    assert_ne!(
        reloaded.session, first,
        "E101: a reload draws a new session"
    );
    let dto = live.dto(&key, "- stale\n");
    let submit = |session, id| {
        let kinds = [EditKind::SaveBlock];
        live.host
            .submit(session, id, &key, &dto, page.version, None, &kinds)
    };
    assert_eq!(
        submit(first, reloaded.next_id),
        Err(PageRefusal::NotAdmitted),
        "E101: a version remembered under an earlier session never grants"
    );
    assert_eq!(
        submit(reloaded.session, reloaded.next_id - 1),
        Err(PageRefusal::NotAdmitted),
        "ids continue above the admitted watermark"
    );
    live.id.set(reloaded.next_id - 1);
    // Admitted at `next_id`; `Live::wait` checks the mail's session.
    assert_eq!(live.open("pages/a.md").0, key);
    let app = live.app();
    let Live { store, host, .. } = live;
    host.stop();
    let host = PageHost::start(&store, &app, "test-graph", |_, _| {}, |_| {}).unwrap();
    assert!(
        ![first, reloaded.session].contains(&host.session()),
        "E101: a relaunched host draws a new session"
    );
    let open = host.open(reloaded.session, 1 << 20, &PageId::from("pages/a.md"), "a");
    assert_eq!(open, Err(PageRefusal::NotAdmitted));
    host.stop();
}

/// REVIEW-3b-P1 F1: the publication queries are the window session's. A
/// wait begun under one session ends unsuccessful when the window reloads
/// meanwhile, and the debt list and the hurry of an earlier session are
/// refused (an empty list would vouch for pages it never saw), on the same
/// host and on a relaunched one.
#[test]
fn f1_publication_queries_never_answer_for_another_session() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let first = live.host.session();
    let (key, page) = live.open("pages/a.md");
    let unpublished = [(key.clone(), page.version + 1, None)];
    let started = Instant::now();
    let waited = std::thread::scope(|scope| {
        let wait = scope.spawn(|| {
            live.host
                .wait_published(first, &unpublished, Duration::from_secs(20))
        });
        std::thread::sleep(Duration::from_millis(200));
        live.host.window_reloaded();
        wait.join().unwrap()
    });
    assert_eq!(
        waited,
        PageOperation::Refused,
        "F1: a reload ends the old session's wait"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    let published = [(key.clone(), page.version, None)];
    let now = live.host.session();
    assert_eq!(
        live.host
            .wait_published(now, &published, Duration::from_secs(20)),
        PageOperation::Applied
    );
    assert_eq!(
        live.host
            .wait_published(first, &published, Duration::from_secs(20)),
        PageOperation::Refused,
        "F1: never success for an earlier session"
    );
    assert_eq!(live.host.owed(first, None), None);
    assert_eq!(live.host.owed(now, None), Some(vec![]));
    let app = live.app();
    let Live { store, host, .. } = live;
    host.stop();
    let host = PageHost::start(&store, &app, "test-graph", |_, _| {}, |_| {}).unwrap();
    assert_eq!(
        host.owed(now, None),
        None,
        "F1: a relaunched host refuses it too"
    );
    assert_eq!(
        host.wait_published(now, &published, Duration::from_secs(1)),
        PageOperation::Refused
    );
    host.stop();
}

/// P7: a Discard carries no version. One sent after the page's version moved
/// past the window's view still takes the current disk, and one from an
/// earlier window session is not admitted.
#[test]
fn a_discard_takes_the_current_disk_and_an_old_session_discard_is_not_admitted() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
    let typed = live.answer(&key, id).page.unwrap().version;
    assert!(typed > page.version, "the submit moved the page's version");
    let id = live.id();
    live.host.discard(live.host.session(), id, &key).unwrap();
    let mail = live.answer(&key, id);
    assert_eq!(mail.answer.unwrap().outcome, AnswerOutcome::Applied);
    let MailText::Page { dto } = &mail.page.unwrap().text else {
        panic!("a discard that changed the buffer carries the disk's text");
    };
    assert_eq!(dto.blocks[0].raw, "one");
    let held = (live.host.driver.shared).with_state_for_tests(|state| {
        let page = &state.progress.host.pages[&key];
        (page.buf.as_deref() == Some(&b"- one\n"[..]), page.clean())
    });
    assert_eq!(held, (true, true), "the host holds the disk, clean");
    let old = live.host.session();
    live.host.window_reloaded();
    assert_eq!(
        live.host.discard(old, live.id(), &key),
        Err(PageRefusal::NotAdmitted)
    );
    assert_eq!(live.disk(&key), "- one\n");
    live.host.stop();
}

/// N1 (OG-RULES Rule 8, STEP3 §3.2 `kinds`): a published page's edit kinds
/// reach the sink `PageHost::start` was given, at its publication; the app
/// logs them there (`src-tauri` `page_commands::log_edit_kinds`).
#[test]
fn a_published_pages_edit_kinds_reach_the_start_kinds_sink() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.until_indexed(&key, "- two\n");
    let kinds = live.kinds.lock().unwrap().clone();
    assert_eq!(kinds, vec![(key.clone(), vec![EditKind::SaveBlock])]);
    live.host.stop();
}

/// E101 (P1): a page the host let go and loads again gets a version above
/// every version it had before, so no answer or need of the old load can
/// be mistaken for the new one.
#[test]
fn e101_an_evicted_and_reloaded_page_gets_a_greater_version() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let mut seen = page.version;
    for body in ["- one\n- two\n", "- one\n- three\n"] {
        let id = live.submit(&key, body, seen, None).unwrap();
        seen = live.answer(&key, id).answer.unwrap().version;
    }
    live.until_published(&[(key.clone(), seen, None)]);
    live.close(&key);
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(&key) {
        assert!(Instant::now() < deadline, "the closed page is never let go");
        std::thread::sleep(Duration::from_millis(20));
    }
    let (_, again) = live.open("pages/a.md");
    assert!(
        again.version > seen,
        "E101: reloaded at {} after {seen}",
        again.version
    );
    live.host.stop();
}

/// `page_owed` (R2): a held page owes its unpublished version; a page the
/// host let go owes its index (version 0) until it publishes; a path
/// filter names keys through the shared entry identity.
#[test]
fn owed_lists_unpublished_held_pages_and_released_index_debt() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    // b stays held and published: it owes nothing.
    let (b, opened) = live.open("pages/b.md");
    live.until_published(&[(b, opened.version, None)]);
    let (key, page) = live.open("pages/a.md");
    live.index_faults(3);
    let id = live
        .submit(&key, "- one\n- two\n", page.version, None)
        .unwrap();
    let version = live.answer(&key, id).answer.unwrap().version;
    let session = live.host.session();
    let owed = |paths: Option<&[PageId]>| live.host.owed(session, paths).unwrap();
    assert_eq!(owed(None), vec![(key.clone(), version)]);
    let a = [PageId::from("pages/a.md")];
    assert_eq!(owed(Some(&a)), vec![(key.clone(), version)]);
    assert_eq!(owed(Some(&[PageId::from("pages/b.md")])), vec![]);
    live.close(&key);
    let until = |owed: Vec<(String, u64)>, what: &str| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while live.host.owed(session, None) != Some(owed.clone()) {
            assert!(Instant::now() < deadline, "owed never {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    until(vec![(key.clone(), 0)], "the released index debt");
    until(vec![], "empty");
    live.until_indexed(&key, "- one\n- two\n");
    live.host.stop();
}

/// `page_wait` (§4.4): true once the needs publish; false at once on a
/// conflict of a needed page, and false at the bound, never success.
#[test]
fn wait_published_is_true_on_publication_false_on_conflict_or_bound() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    let version = live.answer(&key, id).answer.unwrap().version;
    let long = Duration::from_secs(20);
    let session = live.host.session();
    let wait = |needs: &[(String, u64, Option<String>)], bound| {
        live.host.wait_published(session, needs, bound)
    };
    assert_eq!(
        wait(&[(key.clone(), version, None)], long),
        PageOperation::Applied
    );
    assert_eq!(live.disk(&key), "- two\n");
    let start = Instant::now();
    let later = [(key.clone(), version + 1, None)];
    assert_eq!(
        wait(&later, Duration::from_millis(300)),
        PageOperation::Pending
    );
    assert!(
        start.elapsed() >= Duration::from_millis(300),
        "S1: no early false"
    );
    let id = live.submit(&key, "- mine\n", version, None).unwrap();
    let mine = live.answer(&key, id).answer.unwrap().version;
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    let start = Instant::now();
    assert_eq!(
        wait(&[(key.clone(), mine, None)], long),
        PageOperation::Refused
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "a conflict ends the wait"
    );
    live.host.stop();
}

/// REVIEW-3b-P2b R3: with real taken debt (a version whose save keeps
/// failing), a publication wait is Refused once nothing will publish it: the
/// driver ended, the host went down, or its state is poisoned (failure, not a
/// panic). Never Pending at every bound, and never Applied for a dropped page.
#[test]
fn wait_published_refuses_once_the_driver_ended_or_the_host_is_down() {
    use crate::page_host::io::Phase;
    let taken = |live: &Live| {
        live.faults(Phase::PageTemp, 10_000);
        let (key, page) = live.open("pages/a.md");
        let id = live.submit(&key, "- two\n", page.version, None).unwrap();
        let version = live.answer(&key, id).answer.unwrap().version;
        vec![(key, version, None)]
    };
    let bound = Duration::from_millis(30);

    let mut ended = Live::new(&[("pages/a.md", "- one\n")]);
    let needs = taken(&ended);
    let session = ended.host.session();
    ended.host.driver.join();
    assert!(ended.host.driver.shared.ended());
    assert_eq!(
        ended.host.wait_published(session, &needs, bound),
        PageOperation::Refused,
        "an ended driver publishes nothing"
    );

    let down = Live::new(&[("pages/a.md", "- one\n")]);
    let needs = taken(&down);
    let session = down.host.session();
    down.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.stop());
    assert_eq!(
        down.host.wait_published(session, &needs, bound),
        PageOperation::Refused,
        "a host that is down publishes nothing"
    );
    down.host.stop();

    let poisoned = Live::new(&[("pages/a.md", "- one\n")]);
    let needs = taken(&poisoned);
    let session = poisoned.host.session();
    let shared = Arc::clone(&poisoned.host.driver.shared);
    let poisoner = std::thread::spawn(move || {
        let _state = shared.state_for_tests();
        panic!("R3 test: poison the host state");
    });
    assert!(poisoner.join().is_err());
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        poisoned.host.wait_published(session, &needs, bound)
    }));
    // Torn down before the assertion: a failure's unwind would meet the
    // failed host's panicking teardown and abort the binary.
    drop_failed(poisoned);
    assert_eq!(
        answer.ok(),
        Some(PageOperation::Refused),
        "a poisoned host is a failed wait, not a panic"
    );
}

/// Q-P2b-4 (STEP3-DESIGN, P2b checkpoint decisions): a barrier reads the
/// logical state. With the draft worker paused before a deletion's
/// application, `owed` names the page at the deletion's version and
/// `page_wait` does not vouch for it until its file is gone from the disk.
#[test]
fn q_p2b_4_a_deletion_in_flight_is_owed_and_unpublished_until_its_file_is_gone() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let session = live.host.session();
    let page = PageId::from("pages/a.md");
    live.pause(true);
    // Finding B: the delete waits for its own publication, so it runs
    // beside the barrier calls.
    std::thread::scope(|scope| {
        let deleted = scope.spawn(|| live.host.delete(session, &page, b"- one\n"));
        let deadline = Instant::now() + Duration::from_secs(5);
        let owed = loop {
            let owed = live
                .host
                .owed(session, Some(std::slice::from_ref(&page)))
                .unwrap();
            if !owed.is_empty() || Instant::now() >= deadline {
                break owed;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(owed.len(), 1, "the in-flight deletion is debt: {owed:?}");
        let needs = [(owed[0].0.clone(), owed[0].1, None)];
        assert_eq!(
            live.host
                .wait_published(session, &needs, Duration::from_millis(300)),
            PageOperation::Pending
        );
        assert!(live.root.join("pages/a.md").exists());
        live.pause(false);
        assert_eq!(deleted.join().unwrap(), PageOperation::Applied);
        assert!(
            !live.root.join("pages/a.md").exists(),
            "the delete's Applied vouches for the disk"
        );
        assert_eq!(
            live.host
                .wait_published(session, &needs, Duration::from_secs(20)),
            PageOperation::Applied
        );
    });
    live.host.stop();
}

/// Q-P2b-5: the deletion is bound to the bytes the identity checks read. An
/// external editor or sync service that changes the file between the checks
/// and the host's operation read makes the deletion refuse, and the file
/// keeps the new bytes (docs/storage-contract.md
/// `tine-store::page_host::delete`).
#[test]
fn q_p2b_5_a_deletion_refuses_when_the_file_changed_after_its_checks() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let session = live.host.session();
    let page = PageId::from("pages/a.md");
    std::fs::write(live.root.join("pages/a.md"), "- external\n").unwrap();
    assert_eq!(
        live.host.delete(session, &page, b"- one\n"),
        PageOperation::Refused
    );
    assert_eq!(
        std::fs::read(live.root.join("pages/a.md")).unwrap(),
        b"- external\n"
    );
    live.host.stop();
}

/// Q-P2b-4: a submit the host admitted but has not applied is debt too. The
/// window's settle loop (ask `owed`, wait, ask again) ends only once the
/// submitted text is on disk.
#[test]
fn q_p2b_4_a_queued_submit_is_owed_until_its_text_is_on_disk() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let session = live.host.session();
    live.pause(true);
    live.submit(&key, "- two\n", page.version, None).unwrap();
    let owed = live.host.owed(session, None).unwrap();
    assert!(
        owed.iter().any(|(k, v)| *k == key && *v >= page.version),
        "the queued submit is debt: {owed:?}"
    );
    let needs: Vec<_> = owed.into_iter().map(|(k, v)| (k, v, None)).collect();
    assert_eq!(
        live.host
            .wait_published(session, &needs, Duration::from_millis(300)),
        PageOperation::Pending
    );
    assert_eq!(live.disk(&key), "- one\n");
    live.pause(false);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let owed = live.host.owed(session, None).unwrap();
        if owed.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "debt never cleared: {owed:?}");
        let needs: Vec<_> = owed.into_iter().map(|(k, v)| (k, v, None)).collect();
        assert_eq!(
            live.host
                .wait_published(session, &needs, Duration::from_secs(20)),
            PageOperation::Applied
        );
    }
    assert_eq!(live.disk(&key), "- two\n");
    live.host.stop();
}

/// B-Q1 (plan v3 §4): draft I/O that fails never stops a launch, at a
/// restore's relaunch as at start. The relaunched host opens and publishes
/// pages, `draft_status` says why crash recovery is unavailable, and Retry
/// brings it up in the running host (S3).
#[test]
fn b_q1_a_restore_relaunches_into_unavailable_drafts_and_retry_restores_them() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    assert_eq!(live.host.draft_status(), DraftStatus::default());
    let (key, _) = live.open("pages/a.md");
    assert!(live.host.stop_begin(live.id.get(), StopMode::Restore));
    assert_eq!(live.until_stop(), StopState::Ready);
    let drafts = live.app().join("drafts-v2/test-graph");
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        kinds,
        id,
    } = live;
    let stopped = host.stop_finish().ok().unwrap();
    fs::remove_dir_all(&drafts).unwrap();
    fs::write(&drafts, "in the way").unwrap();
    let host = stopped.relaunch().expect("B-Q1: the host always starts");
    let live = Live {
        _dir,
        root,
        store,
        host,
        mail,
        kinds,
        id,
    };
    assert!(live.host.draft_status().unavailable.is_some());
    let page = live.open("pages/a.md").1;
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    let version = live.answer(&key, id).answer.unwrap().version;
    live.until_published(&[(key.clone(), version, None)]);
    assert_eq!(live.disk(&key), "- two\n");
    assert!(live.host.drafts_retry().is_err());
    fs::remove_file(&drafts).unwrap();
    assert_eq!(live.host.drafts_retry(), Ok(()));
    assert_eq!(live.host.draft_status(), DraftStatus::default());
}

/// S9, STEP3 §9: an open names the pages whose text is not on disk yet. A
/// page launch recovered from its draft is `recovered` while it stays at
/// the version launch installed; input typed into it since is ordinary
/// unsaved input; a page whose saves keep failing says so; a published page
/// is not named.
#[test]
fn s9_draft_status_names_recovered_and_unsaved_pages() {
    use crate::page_host::io::{Phase, UnsavedPage};
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    live.submits_wait();
    live.open("pages/b.md");
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.faults(Phase::PageTemp, 100);
    assert!(live.host.stop_begin(id, StopMode::Switch));
    assert_eq!(
        live.until_stop(),
        StopState::Ready,
        "the failed save is drafted"
    );
    let app = live.app();
    let Live {
        _dir,
        root,
        store,
        host,
        ..
    } = live;
    assert!(host.stop_finish().is_ok());
    assert_eq!(
        fs::read_to_string(root.join("pages/a.md")).unwrap(),
        "- one\n"
    );
    crate::page_host::faults::fail(Phase::PageTemp, std::io::ErrorKind::Other, 100);
    let (sender, mail) = mpsc::channel();
    let kinds = Kinds::default();
    let host = PageHost::start(&store, &app, "test-graph", recording(&kinds), move |mail| {
        let _ = sender.send(mail);
    })
    .unwrap();
    let live = Live {
        _dir,
        root,
        store,
        host,
        mail,
        kinds,
        id: std::cell::Cell::new(100),
    };
    let unsaved = |recovered, failing| {
        vec![UnsavedPage {
            path: "pages/a.md".into(),
            recovered,
            conflict: false,
            failing,
        }]
    };
    assert_eq!(live.host.draft_status().unsaved, unsaved(true, false));
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- three\n", page.version, None).unwrap();
    live.answer(&key, id);
    assert_eq!(live.host.draft_status().unsaved, unsaved(false, false));
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.host.draft_status().unsaved != unsaved(false, true) {
        assert!(Instant::now() < deadline, "the save error never showed");
        std::thread::sleep(Duration::from_millis(20));
    }
    live.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
    let deadline = Instant::now() + Duration::from_secs(40);
    while !live.host.draft_status().unsaved.is_empty() {
        assert!(Instant::now() < deadline, "the page never published");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(live.disk(&key), "- three\n");
    live.host.stop();
}

/// og I1c (audit F17) on the host: a transient removal of a page with
/// unsaved input (an external editor's temp+rename, a sync pass mid-delivery)
/// conflicts it; the file back with other bytes stays conflicted; back at
/// the page's baseline the conflict lifts, and the held edit saves over it.
#[test]
fn i1c_a_removal_back_at_the_baseline_lifts_the_conflict_and_the_edit_saves() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    live.faults(crate::page_host::io::Phase::PageTemp, 1000);
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    fs::remove_file(live.root.join(&key)).unwrap();
    let removed = live.wait(&key, "removal", |mail| {
        mail.page
            .as_ref()
            .is_some_and(|page| page.disk == Some(DiskToken::NoFile))
    });
    assert!(removed.page.unwrap().conflict);
    assert!(live.external(&key, "- other\n").conflict);
    assert!(!live.external(&key, "- one\n").conflict);
    live.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
    // Saved means on disk and reported saved: the bytes appear at the
    // rename, a few milliseconds before the save's directory sync completes
    // and the page turns clean.
    let deadline = Instant::now() + Duration::from_secs(40);
    while live.disk(&key) != "- two\n" || !live.host.draft_status().unsaved.is_empty() {
        assert!(Instant::now() < deadline, "the held edit never saved");
        std::thread::sleep(Duration::from_millis(20));
    }
    live.host.stop();
}

/// REVIEW-3b-P1 R2: a key named by a known, unsynced draft copy is not
/// evicted (its index stays with the host) until the census syncs.
#[test]
fn review_p1_m2_a_key_with_vehicle_debt_is_not_evicted() {
    let mut host = super::super::tests::host();
    let mut book = Book::default();
    book.own(["b.md".to_string()], &host);
    assert_eq!(book.evictable(&host, &[]), ["b.md"]);
    host.vehicle_debt.insert("b.md".into());
    assert!(book.evictable(&host, &[]).is_empty());
}

/// PERF-FIXES D4: an Open applying (waiting for its page's lock) keeps its
/// key owned: admission owned it so that no collection evicts it before the
/// host holds the page.
#[test]
fn an_applying_open_keeps_its_key_owned() {
    let mut host = super::super::tests::host();
    let mut book = Book::default();
    book.own(["b.md".to_string()], &host);
    host.applying = Some(Request {
        id: 1,
        generation: host.generation,
        page: "b.md".into(),
        kind: RequestKind::Open,
    });
    assert!(book.evictable(&host, &[]).is_empty());
    host.applying = None;
    assert_eq!(book.evictable(&host, &[]), ["b.md"]);
}

/// R2: a restore over a key with vehicle debt aborts naming it at once,
/// even with every page saved and no surfaced draft failure, so it never
/// waits under the restore lock for draft I/O to come back.
#[test]
fn review_p1_m2_a_restore_aborts_on_vehicle_debt_alone() {
    let mut u = Unit::new();
    u.progress.with_host(|h| {
        h.vehicle_debt.insert("a.md".into());
        assert_eq!(h.switch_ready(h.last_applied), Disposition::Applied);
    });
    u.progress.stopping = Some(Stopping {
        restore: true,
        failed: BTreeSet::new(),
    });
    assert_eq!(
        stop_state(&u.progress, &u.book),
        StopState::Aborted(BTreeSet::from(["a.md".into()]))
    );
}

fn evicted(delivery: &Delivery) -> Vec<&str> {
    delivery
        .evicted
        .iter()
        .map(|(key, _)| key.as_str())
        .collect()
}

/// M4 B2: an Open whose read fails leaves its key owned but never held: an
/// orphan from admission, evicted once nothing else holds its index.
#[test]
fn m4_b2_a_failed_open_is_evicted() {
    let mut u = Unit::new();
    u.progress.host.fs.inject(Phase::Read, [Fault::Before]);
    u.progress.with_host(|h| {
        let request = Request {
            id: h.last_admitted + 1,
            generation: h.generation,
            page: "b.md".into(),
            kind: RequestKind::Open,
        };
        assert_eq!(h.admit(request), Disposition::Applied);
    });
    // `PageHost::admit` takes the key's ownership at admission.
    u.book.own(["b.md".to_string()], &u.progress.host);
    assert!(
        evicted(&u.collect()).is_empty(),
        "a queued Open keeps its index"
    );
    u.progress.with_host(|h| {
        assert_eq!(h.dequeue(), Disposition::Pending);
        h.apply_request();
    });
    assert!(!u.progress.host.pages.contains_key("b.md"));
    assert_eq!(evicted(&u.collect()), ["b.md"]);
    assert!(!u.book.owned().contains("b.md"));
}

/// M4 B2: an admitted Open a window reload overtakes is still applied (it
/// keeps custody), then released unsubscribed: its key is held, becomes an
/// orphan and is evicted.
#[test]
fn m4_b2_an_open_overtaken_by_a_reload_is_evicted_after_release() {
    let mut u = Unit::new();
    u.progress.with_host(|h| {
        let request = Request {
            id: h.last_admitted + 1,
            generation: h.generation,
            page: "b.md".into(),
            kind: RequestKind::Open,
        };
        assert_eq!(h.admit(request), Disposition::Applied);
    });
    u.book.own(["b.md".to_string()], &u.progress.host);
    u.progress.with_host(|h| {
        h.window_crash();
        assert_eq!(h.dequeue(), Disposition::Pending);
        h.apply_request();
    });
    assert!(u.progress.host.pages.contains_key("b.md"));
    assert!(
        evicted(&u.collect()).is_empty(),
        "held again: not an orphan"
    );
    for _ in 0..8 {
        u.progress.poll();
        u.progress.host.held = None;
    }
    assert!(!u.progress.host.pages.contains_key("b.md"), "released");
    assert!(evicted(&u.collect()).contains(&"b.md"));
    assert!(!u.book.owned().contains("b.md"));
}

/// M4 B2: an orphan whose eviction is blocked (vehicle debt, an unobserved
/// release) stays a candidate: it is evicted once the obligation ends,
/// though nothing about the key changes on the host.
#[test]
fn m4_b2_a_blocked_orphan_is_evicted_when_its_obligation_ends() {
    let mut u = Unit::new();
    u.book.own(["b.md".to_string()], &u.progress.host);
    u.progress.host.vehicle_debt.insert("b.md".into());
    u.book.handover.insert("b.md".into());
    assert!(evicted(&u.collect()).is_empty());
    u.progress.host.vehicle_debt.clear();
    assert!(evicted(&u.collect()).is_empty(), "still handed over");
    u.book.handover.clear();
    assert_eq!(evicted(&u.collect()), ["b.md"]);
}

/// PERF-FIXES D4: opening a page whose bytes the index already holds changes
/// no derived state, so it leaves a written launch checkpoint clean. Before
/// the fix each open published the page as a change and restarted the
/// checkpoint's quiet window, so an editing session never wrote one and the
/// next launch was cold (ab1: 0 of 7 typing/custody/drafts trials).
#[test]
fn opening_an_unchanged_page_leaves_the_launch_checkpoint_clean() {
    let live = Live::with(
        &[("pages/a.md", "- a\n"), ("pages/b.md", "- b [[a]]\n")],
        true,
    );
    assert!(matches!(
        live.store.write_checkpoint_now(),
        Some(crate::CheckpointWrite::Written { .. })
    ));
    let dirty = || live.store.changes.checkpoint.get().unwrap().dirty();
    assert!(!dirty());
    let rev = live.store.whole_graph().unwrap().rev();
    let (a, _) = live.open("pages/a.md");
    let (b, _) = live.open("pages/b.md");
    // Past the owner's Open read publications.
    let read = || {
        (live.host.driver.shared).with_state_for_tests(|s| {
            s.book.index.contains_key(&a) && s.book.index.contains_key(&b)
        })
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while !read() {
        assert!(
            Instant::now() < deadline,
            "the Open reads were never published"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live.store.whole_graph().unwrap().rev(),
        rev,
        "an open published"
    );
    assert!(
        !dirty(),
        "an open whose bytes the index holds must not mark the launch checkpoint dirty"
    );
}

/// An ownership transition that moves no row leaves the derived cache the
/// published generation's: an open of an unchanged page must not make every
/// later launch-checkpoint capture "unpublished" (it retried every 5 s and
/// never wrote while no other publication came).
#[test]
fn an_open_that_moves_no_row_leaves_the_checkpoint_capturable() {
    let live = Live::with(
        &[("pages/a.md", "- a\n"), ("pages/b.md", "- b [[a]]\n")],
        true,
    );
    assert!(matches!(
        live.store.write_checkpoint_now(),
        Some(crate::CheckpointWrite::Written { .. })
    ));
    let rev = live.store.whole_graph().unwrap().rev();
    let (a, _) = live.open("pages/a.md");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !(live.host.driver.shared).with_state_for_tests(|s| s.book.index.contains_key(&a)) {
        assert!(
            Instant::now() < deadline,
            "the Open read was never published"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let outcome = live.store.write_checkpoint_now();
    assert!(
        matches!(outcome, Some(crate::CheckpointWrite::Written { .. })),
        "a checkpoint after an unchanged open: {outcome:?}"
    );
    // A release that moves nothing (REVIEW-settle-cow F1) neither.
    live.close(&a);
    let deadline = Instant::now() + Duration::from_secs(20);
    while live.held(&a) {
        assert!(Instant::now() < deadline, "the closed page stayed held");
        std::thread::sleep(Duration::from_millis(10));
    }
    let outcome = live.store.write_checkpoint_now();
    assert!(
        matches!(outcome, Some(crate::CheckpointWrite::Written { .. })),
        "a checkpoint after an unchanged release: {outcome:?}"
    );
    assert_eq!(
        live.store.whole_graph().unwrap().rev(),
        rev,
        "an unchanged open and release publish nothing"
    );
}

/// host-only-writer P1 (review: the driver never waits for the store
/// writer while callers wait on it). While another thread's transaction
/// holds the writer, a save's publication waits for it, and the driver
/// keeps answering: a read is answered, and a command that needs the writer
/// (an open) waits alone. Once the writer is free, both complete.
#[test]
fn the_driver_answers_while_another_thread_holds_the_store_writer() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    let (key, page) = live.open("pages/a.md");
    let session = live.host.session();
    let writer = live.store.writer.lock().unwrap();
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.until_disk(&key, "- two\n");
    // The save's publication is due, and needs the writer.
    std::thread::sleep(Duration::from_millis(100));
    let (opened, open_done) = mpsc::channel();
    let (owed, owed_done) = mpsc::channel();
    std::thread::scope(|scope| {
        let host = &live.host;
        scope.spawn(move || {
            let b = PageId::from("pages/b.md");
            let _ = opened.send(host.open(session, 1_000, &b, "b").is_ok());
        });
        std::thread::sleep(Duration::from_millis(50));
        scope.spawn(move || {
            let _ = owed.send(host.owed(session, None));
        });
        let answered = owed_done.recv_timeout(Duration::from_secs(5));
        let open_waited = open_done.try_recv().is_err();
        drop(writer);
        let debt = answered.expect("a read is answered while the writer is held");
        let debt = debt.expect("the current session");
        assert!(
            debt.iter().any(|(owed, _)| *owed == key),
            "the publication waits for the writer: {debt:?}"
        );
        assert!(open_waited, "an open needs the writer");
        let opened = open_done.recv_timeout(Duration::from_secs(5));
        assert_eq!(
            opened,
            Ok(true),
            "the open completes once the writer is free"
        );
    });
    live.answer(&key, id);
    live.until_indexed(&key, "- two\n");
    live.host.stop();
}

/// P1 re-review 1: the host admits a window's request ids in increasing
/// order, so an admission deferred on the store writer (an open) is never
/// overtaken by a later one (a submit on another page), which would refuse
/// it although its caller sent it first and never left. The submit is sent
/// once the driver has deferred the open (observed, not slept for).
#[test]
fn an_admission_deferred_on_the_writer_is_not_overtaken() {
    use std::sync::atomic::Ordering;
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    let (key, page) = live.open("pages/a.md");
    let session = live.host.session();
    let (open_id, submit_id) = (live.id(), live.id());
    let dto = live.dto(&key, "- two\n");
    let writer = live.store.writer.lock().unwrap();
    std::thread::scope(|scope| {
        let host = &live.host;
        let b = PageId::from("pages/b.md");
        let opened = scope.spawn(move || host.open(session, open_id, &b, "b").map(|_| ()));
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.driver.shared.deferred.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the open was never deferred");
            std::thread::sleep(Duration::from_millis(1));
        }
        let key = key.clone();
        let submitted = scope.spawn(move || {
            let kinds = [EditKind::SaveBlock];
            host.submit(session, submit_id, &key, &dto, page.version, None, &kinds)
        });
        // The submit reaches the driver while the open still waits: queued
        // behind it here, or answered by a driver that lets it pass. The
        // writer is released only once one of those was observed, so a submit
        // that is merely slow to arrive cannot pass for an ordered one.
        let queued = Instant::now() + Duration::from_secs(20);
        while host.driver.shared.deferred.load(Ordering::SeqCst) < 2 && !submitted.is_finished() {
            assert!(
                Instant::now() < queued,
                "the submit never reached the driver"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(writer);
        assert_eq!(
            opened.join().unwrap(),
            Ok(()),
            "the earlier open is admitted"
        );
        assert_eq!(submitted.join().unwrap(), Ok(()), "the later submit too");
    });
    live.answer(&key, submit_id);
    live.until_indexed(&key, "- two\n");
    live.host.stop();
}

/// P1 re-review 2: the kinds sink `PageHost::start` was given runs outside
/// the store writer, as before the driver took it: a sink that takes the
/// writer (`Store::page` on a loaded graph) must not deadlock the driver.
/// The sink asks for the writer for 2 s, so a watcher reconcile holding it
/// briefly is not mistaken for the driver.
#[test]
fn the_kinds_sink_runs_outside_the_store_writer() {
    let dir = tempfile::tempdir().unwrap();
    let graph = dir.path().join("graph");
    fs::create_dir_all(graph.join("pages")).unwrap();
    fs::write(graph.join("pages/a.md"), "- one\n").unwrap();
    let app = dir.path().join("app");
    fs::create_dir_all(&app).unwrap();
    let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
    store.whole_graph_reconciled().unwrap();
    let free = Arc::new(Mutex::new(Vec::new()));
    let (sink_store, seen) = (Arc::downgrade(&store), free.clone());
    let kinds = move |_: &str, _: &[EditKind]| {
        let Some(store) = sink_store.upgrade() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let taken = loop {
            if store.writer.try_lock().is_ok() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        seen.lock().unwrap().push(taken);
    };
    let (sender, mail) = mpsc::channel();
    let host = PageHost::start(&store, &app, "test-graph", kinds, move |mail| {
        let _ = sender.send(mail);
    })
    .unwrap();
    let live = Live {
        root: store.graph.root.clone(),
        _dir: dir,
        store,
        host,
        mail,
        kinds: Kinds::default(),
        id: std::cell::Cell::new(0),
    };
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    live.until_indexed(&key, "- two\n");
    assert_eq!(
        *free.lock().unwrap(),
        vec![true],
        "the sink could take the writer"
    );
    live.host.stop();
}

/// CLI PROTOCOL §5: the host read returns a held page's buffer, unsaved
/// input included, while its file still has the old bytes; a page the host
/// does not hold reads from its file with no revision; no file reads absent.
#[cfg(unix)]
#[test]
fn the_host_read_returns_the_held_buffer_not_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- bee\n")]);
    let (key, page) = live.open("pages/a.md");
    let pages = live.root.join("pages");
    // The save cannot publish: the typed text stays unsaved in the buffer.
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o555)).unwrap();
    live.submit(&key, "- two\n", page.version, None).unwrap();
    live.wait(&key, "save error", |mail| mail.notice.save_error);
    let read = live.host.read(&PageId::from("pages/a.md")).unwrap();
    fs::set_permissions(&pages, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(fs::read_to_string(live.root.join(&key)).unwrap(), "- one\n");
    assert_eq!(read.bytes.as_deref(), Some(&b"- two\n"[..]));
    let (incarnation, version) = read.revision.expect("a held page has a revision");
    assert!(
        incarnation >= 1 && version > page.version,
        "{:?}",
        read.revision
    );
    let unheld = live.host.read(&PageId::from("pages/b.md")).unwrap();
    assert_eq!(unheld.bytes.as_deref(), Some(&b"- bee\n"[..]));
    assert_eq!(
        unheld.revision, None,
        "no version is invented for an unheld page"
    );
    let absent = live.host.read(&PageId::from("pages/none.md")).unwrap();
    assert_eq!((absent.bytes, absent.revision), (None, None));
    live.until_disk(&key, "- two\n");
    live.host.stop();
}
