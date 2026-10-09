//! The graph binding (STEP3 §3): its bookkeeping beside a model host, and
//! its command path against a real Store with the driver thread running.
use super::*;
use crate::page_host::model_fs::ModelFs;
use crate::page_host::tests::{open as open_page, risk, saved, send, text};
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
    assert!(delivery.documents.is_empty());
    assert_eq!(u.book.versions["a.md"].len(), 1);
    u.progress.with_host(|h| saved(h, "a.md"));
    let delivery = u.collect();
    assert_eq!(delivery.documents.len(), 1);
    let (page, published, document) = &delivery.documents[0];
    assert_eq!((page.as_str(), *published), ("a.md", version));
    assert_eq!(
        document.pre_block.as_deref(),
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
    store: Store,
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
        let store = Store::open(&graph, Default::default()).unwrap().0;
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
            .open(&self.store, generation, id, &PageId::from(rel), name)
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
                &self.store,
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
        fs::write(self.root.join(key), body).unwrap();
        self.host.disk_changed(key);
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
        .open(&live.store, generation, 1, &PageId::from("pages/c.md"), "c");
    assert!(
        matches!(&refused, Err(PageRefusal::Twin { existing }) if existing.ends_with("c.org")),
        "{refused:?}"
    );
    let refused = live.host.open(
        &live.store,
        generation,
        2,
        &PageId::from("logseq/config.edn"),
        "config",
    );
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
    let against =
        |bytes: &str| PageHost::serialize(&live.store, &key, &dto, Some(&text(bytes)), &[]);
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
