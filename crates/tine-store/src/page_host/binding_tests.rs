//! The graph binding (STEP3 §3): its bookkeeping beside a model host, and
//! its command path against a real Store with the driver thread running.
use super::*;
use crate::page_host::model_fs::ModelFs;
use crate::page_host::tests::{edit, open as open_page, risk, saved, send, text};
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
    id: std::cell::Cell<u64>,
}

impl Live {
    fn new(files: &[(&str, &str)]) -> Self {
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
        let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
        // A loaded index: what it holds afterwards was published to it.
        store.whole_graph_reconciled().unwrap();
        let root = store.graph.root.clone();
        let (sender, mail) = mpsc::channel();
        let host = PageHost::start(&store, &app, "test-graph", 7, move |mail| {
            let _ = sender.send(mail);
        })
        .unwrap();
        Self {
            _dir: dir,
            root,
            store,
            host,
            mail,
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
            assert_eq!(mail.binding, 7);
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
        let generation = self.host.generation();
        let name = rel.rsplit('/').next().unwrap().split('.').next().unwrap();
        let key = self
            .host
            .open(generation, id, &PageId::from(rel), name)
            .unwrap();
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
        let generation = self.host.generation();
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
        let generation = self.host.generation();
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
    assert!(matches!(mail.page.unwrap().text, MailText::Unchanged));
    live.until_disk(&key, "- one\n- two\n");
    // A page with no file yet is opened, then created by its first save.
    let (key, page) = live.open("pages/new.md");
    assert_eq!(page.disk, Some(DiskToken::NoFile));
    live.submit(&key, "- made\n", page.version, None).unwrap();
    live.until_disk(&key, "- made\n");
    assert!(live.host.pages_recoverable(&[(key, page.version + 1)]));
    live.host.stop();
}

#[test]
fn f9_create_checks_run_at_open_of_a_page_with_no_file() {
    let live = Live::new(&[("pages/c.org", "* c\n")]);
    let generation = live.host.generation();
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

/// Q1 (REVIEW-3): the window was shown disk A; C is observed before its
/// Keep mine is serialized, and A again before anything could apply it.
/// The resolve's comparison source is exactly the bytes its token names,
/// never another observation: the corrupting DTO never reaches disk, and
/// once A is current again the firewall refuses it against A.
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
    let id = live
        .submit(&key, corrupt, current.version, Some(&token(a)))
        .expect("an old token is admitted as stale input, not refused");
    let page = live.answer(&key, id).page.unwrap();
    assert!(page.conflict, "Q1: an old token's request stays conflicted");
    settle();
    assert_eq!(live.disk(&key), c, "Q1: nothing was checked against C");
    let current = live.external(&key, a);
    assert!(current.conflict);
    settle();
    assert_eq!(
        live.disk(&key),
        a,
        "Q1: A returned, the request was never checked against it"
    );
    let again = live.submit(&key, corrupt, current.version, Some(&token(a)));
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
        self.host.driver.shared.with_state(|state| {
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
    let host = PageHost::start(&store, &app, "test-graph", 7, move |mail| {
        let _ = sender.send(mail);
    })
    .unwrap();
    let live = Live {
        _dir,
        root,
        store,
        host,
        mail,
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
    let (key, page) = live.open("pages/r.md");
    let id = live.submit(&key, "- v2\n", page.version, None).unwrap();
    let v2 = live.answer(&key, id).answer.unwrap().version;
    live.until_published(&[(key.clone(), v2, None)]);
    let id = live.submit(&key, "- v3\n", v2, None).unwrap();
    live.answer(&key, id);
    let discover = || vec![PageId::from("pages/r.md")];
    assert_eq!(
        live.host.reserve(discover, Input::Refuse).unwrap_err(),
        BTreeSet::from([key.clone()]),
        "Q3: a typed page that is not at risk still has unsaved input"
    );
    let reservation = live.host.reserve(discover, Input::Flush).unwrap();
    assert_eq!(live.disk(&key), "- v3\n", "flushed before the write");
    live.transaction(&key, "- v3\n", "- rewritten\n");
    live.host.release(reservation);
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
        BTreeSet::from([key.clone()])
    );
    assert!(
        live.host
            .driver
            .shared
            .state
            .lock()
            .unwrap()
            .progress
            .host
            .retained
            .is_empty(),
        "a refused reservation is withdrawn"
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
        live.host.driver.shared.with_state(|_| {});
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- t\n")),
        "Q6: P's retry overwrote the retained transaction's index"
    );
    live.host.release(reservation);
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
    live.host.release(reservation);
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        live.host.driver.shared.with_state(|_| {});
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live.indexed(&key),
        Some(content_rev("- t\n")),
        "V2: P's retry overwrote the retained transaction's index after a failed observation"
    );
    live.host.driver.shared.with_state(|state| {
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
            u.progress.host.fs.twins.insert(page.into(), "c.org".into());
        }
        u.progress.host.advance_save(0);
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
                .twins
                .insert("c.md".into(), "c.org".into());
        }
        let notice = save_through(&mut u, "c.md", after_rename).unwrap();
        assert_eq!(notice.twin.as_deref(), Some("c.org"), "A2: {after_rename}");
        assert_eq!(u.progress.host.pages["c.md"].clean(), after_rename);
        u.progress.host.fs.twins.clear();
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
    live.host.release(reservation);
    live.wait(&key, "the third failed read is reported", |mail| {
        mail.notice.observe_error
    });
    live.host.driver.shared.with_state(|state| {
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
        .with_state(|state| state.observe.contains_key(&a))
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
    let polls = || live.host.driver.shared.with_state(|state| state.polls);
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
