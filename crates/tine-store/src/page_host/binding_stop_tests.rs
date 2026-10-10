//! The binding's whole restore stop and its relaunch (STEP3 §7), against
//! a real Store.
use super::*;

/// §7 steps 3–6: the whole restore stop saves the drained edit, and the
/// stopped binding relaunches one host, for the same binding and mail
/// sink, on the tree as the restore left it.
#[test]
fn a_restore_stop_relaunches_one_host_on_the_restored_tree() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    live.answer(&key, id);
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        ..
    } = live;
    let Ok(stopped) = host.stop_saved(id, StopMode::Restore) else {
        panic!("the restore stop completes");
    };
    assert_eq!(fs::read_to_string(root.join(&key)).unwrap(), "- two\n");
    {
        let _writer = store.writer.lock().unwrap();
        fs::write(root.join(&key), "- restored\n").unwrap();
    }
    let live = Live {
        _dir,
        root,
        store,
        host: stopped.relaunch().unwrap(),
        mail,
        id: std::cell::Cell::new(100),
    };
    // `wait` asserts every mail carries binding 7 on the original sink.
    let (_, page) = live.open("pages/a.md");
    assert_eq!(page.disk, Some(token("- restored\n")));
    live.host.stop();
}

/// §7 step 3: a restore stop that cannot save every edit hands the host
/// back with the page, admission reopened, as today's restore stops when
/// its flush fails.
#[test]
fn a_restore_stop_that_cannot_save_hands_the_host_back() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- mine\n", page.version, None).unwrap();
    live.answer(&key, id);
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        id,
    } = live;
    let Err((host, pages)) = host.stop_saved(id.get(), StopMode::Restore) else {
        panic!("a conflict aborts the restore stop");
    };
    assert_eq!(pages, BTreeSet::from([PageId::from(key.clone())]));
    let live = Live {
        _dir,
        root,
        store,
        host: *host,
        mail,
        id,
    };
    assert!(live.submit(&key, "- again\n", page.version, None).is_ok());
    live.host.stop();
}

impl Live {
    /// The orphan stop's outcome, once it is not Waiting.
    fn until_orphan_stop(&self) -> StopState {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self.host.orphan_stop() {
                StopState::Waiting => {
                    assert!(Instant::now() < deadline, "the orphan stop never settled");
                    std::thread::sleep(Duration::from_millis(20));
                }
                state => return state,
            }
        }
    }
}

/// Plan v3 §3, S2 (`windowCrash`, `storage-s3.qnt:588-590`): a host no
/// window owns ends the window's session once, keeps applying the request
/// it admitted (here queued behind a retained writer), mails no answer to
/// the gone window, and stops only once that input is saved.
#[test]
fn an_orphan_stop_saves_the_admitted_queue_before_it_is_ready() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/a.md")], Input::Refuse)
        .unwrap();
    let window = live.host.session();
    let id = live.submit(&key, "- two\n", page.version, None).unwrap();
    assert_eq!(live.host.orphan_stop(), StopState::Waiting);
    let orphaned = live.host.session();
    assert_ne!(orphaned, window, "S2: the gone window's session ends");
    assert_eq!(live.host.orphan_stop(), StopState::Waiting);
    assert_eq!(
        live.host.session(),
        orphaned,
        "the window crash runs once per owner"
    );
    let dto = live.dto(&key, "- late\n");
    let late = live
        .host
        .submit(window, live.id(), &key, &dto, page.version, None, &[]);
    assert_eq!(
        late,
        Err(PageRefusal::NotAdmitted),
        "a stale session is not admitted"
    );
    settle();
    assert_eq!(live.host.orphan_stop(), StopState::Waiting);
    assert_eq!(
        live.disk(&key),
        "- one\n",
        "the submit is queued behind the reservation"
    );
    drop(reservation);
    assert_eq!(live.until_orphan_stop(), StopState::Ready);
    assert_eq!(
        live.disk(&key),
        "- two\n",
        "the admitted input is saved first"
    );
    let answered = live
        .mail
        .try_iter()
        .any(|mail| mail.session == window && mail.answer.as_ref().is_some_and(|a| a.id == id));
    assert!(!answered, "no answer is owed to the gone window");
    let Ok(_stopped) = live.host.stop_finish() else {
        panic!("a ready orphan stop finishes");
    };
}

/// Plan v3 §3 (B2) and §6: a host no window owns, holding conflicted input
/// whose draft cannot be written, aborts its stop with that page and keeps
/// the input live; a window reload then owns it again.
#[test]
fn an_orphan_stop_over_an_undraftable_conflict_aborts_and_keeps_the_input() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    live.external(&key, "- theirs\n");
    live.faults(super::io::Phase::DraftTemp, 1000);
    live.submit(&key, "- mine\n", page.version, None).unwrap();
    assert_eq!(
        live.until_orphan_stop(),
        StopState::Aborted(BTreeSet::from([key.clone()]))
    );
    live.host.stop_abort();
    assert_eq!(live.disk(&key), "- theirs\n");
    let reloaded = live.host.window_reloaded();
    assert_eq!(reloaded.session, live.host.session());
    assert!(
        live.submit(&key, "- again\n", page.version, None).is_ok(),
        "the input stays live for the next owner"
    );
    live.host.stop();
}

/// Plan v3 §3, S2: page mail follows the window that now owns the binding.
#[test]
fn retargeted_mail_reaches_the_adopting_window() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (sender, adopted) = mpsc::channel();
    live.host.retarget(move |mail| {
        let _ = sender.send(mail);
    });
    let session = live.host.window_reloaded().session;
    let key = live
        .host
        .open(session, live.id(), &PageId::from("pages/a.md"), "a")
        .unwrap()
        .key;
    let mail = adopted.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!((mail.session, mail.key.as_str()), (session, key.as_str()));
    assert!(live.mail.try_recv().is_err(), "the old window gets none");
    live.host.stop();
}

/// REVIEW-3a5 neighbour: a restore stop that cannot save names the page by
/// its current spelling (the restore error shows it), never by its opaque
/// key.
#[test]
fn a_restore_stop_names_an_unsaved_page_by_its_spelling() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    if super::rename::folds_case(&live.root) {
        live.host.stop();
        return;
    }
    super::rename::typed_fresh_entry(&live, "- typed\n");
    let Live { host, id, .. } = live;
    let Err((host, pages)) = host.stop_saved(id.get(), StopMode::Restore) else {
        panic!("an unsaveable edit aborts the restore stop");
    };
    let pages: Vec<String> = pages.iter().map(|page| page.as_str().to_owned()).collect();
    assert_eq!(
        pages,
        ["pages/b.md"],
        "REVIEW-3a5: the restore stop named the page by its opaque key"
    );
    host.stop();
}

/// A binding over `pages/a.md` (`- one`) whose launch recovered a copy
/// (`- recovered`) but could not sync the draft census (M2): draft I/O is
/// down and the copy is known on disk, unsynced.
fn launched_over_an_unsynced_copy() -> Live {
    use crate::page_host::{drafts, io::Phase, production::ATTACH_FAULTS};
    let Live {
        _dir,
        root,
        store,
        host,
        ..
    } = Live::new(&[("pages/a.md", "- one\n")]);
    host.stop();
    let record = Record {
        page: "pages/a.md".into(),
        wseq: 1,
        version: 1,
        base: Base::Known(Some(Arc::from(b"- one\n".as_slice()))),
        bytes: Some(Arc::from(b"- recovered\n".as_slice())),
    };
    let dir = _dir.path().join("app/drafts-v2/test-graph");
    fs::write(
        dir.join(drafts::page_name("pages/a.md")),
        drafts::encode(&[record]),
    )
    .unwrap();
    ATTACH_FAULTS.with(|faults| {
        let mut faults = faults.borrow_mut();
        faults
            .entry(Phase::DraftSync)
            .or_default()
            .push_back(std::io::ErrorKind::Other);
    });
    let (sender, mail) = mpsc::channel();
    let app = _dir.path().join("app");
    let host = PageHost::start(&store, &app, "test-graph", move |mail| {
        let _ = sender.send(mail);
    })
    .unwrap();
    assert!(
        host.draft_status().unavailable.is_some(),
        "M2: draft I/O down"
    );
    Live {
        _dir,
        root,
        store,
        host,
        mail,
        id: std::cell::Cell::new(0),
    }
}

/// REVIEW-3b-P1 R2: a restore stop over a known, unsynced copy aborts at
/// once naming its page (never waiting under the restore lock), so the next
/// host cannot recover the copy over the restored file; once draft I/O is
/// repaired (Retry), the stop saves the page, retires the copy and closes.
#[test]
fn review_p1_m2_a_restore_stop_aborts_over_a_known_unsynced_copy() {
    let live = launched_over_an_unsynced_copy();
    let copies = live.drafts();
    assert_eq!(copies.len(), 1);
    let Live {
        _dir,
        root,
        store,
        host,
        mail,
        ..
    } = live;
    let Err((host, pages)) = host.stop_saved(0, StopMode::Restore) else {
        panic!("R2: the restore stop closed over a known copy");
    };
    assert_eq!(pages, BTreeSet::from([PageId::from("pages/a.md")]));
    let live = Live {
        _dir,
        root,
        store,
        host: *host,
        mail,
        id: std::cell::Cell::new(100),
    };
    assert_eq!(live.drafts(), copies, "nothing retired while down");
    assert_eq!(live.host.drafts_retry(), Ok(()));
    // The draft refresh that failed while I/O was down surfaced an error
    // naming the page; once the driver redoes it, the restore may close.
    let Live { mut host, root, .. } = live;
    let deadline = Instant::now() + Duration::from_secs(20);
    let stopped = loop {
        match host.stop_saved(0, StopMode::Restore) {
            Ok(stopped) => break stopped,
            Err((back, pages)) => {
                assert!(
                    Instant::now() < deadline,
                    "the restore never closed: {pages:?}"
                );
                host = *back;
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(
        fs::read_to_string(root.join("pages/a.md")).unwrap(),
        "- recovered\n"
    );
    drop(stopped);
}

/// R2: an orphaned host (its window gone) over a known, unsynced copy
/// aborts its stop while draft I/O stays down, and re-probes on each pass:
/// once the filesystem is repaired, the stop saves, retires the copy and
/// finishes, with no window to press Retry.
#[test]
fn review_p1_m2_an_orphan_stop_finishes_once_draft_io_is_repaired() {
    let live = launched_over_an_unsynced_copy();
    live.faults(crate::page_host::io::Phase::DraftSync, 1);
    let deadline = Instant::now() + Duration::from_secs(20);
    let pages = loop {
        match live.host.orphan_stop() {
            StopState::Aborted(pages) => break pages,
            StopState::Ready => panic!("R2: the orphan stop finished over a known copy"),
            StopState::Waiting => {
                assert!(Instant::now() < deadline, "the stop never settled");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(pages, BTreeSet::from(["pages/a.md".into()]));
    live.host.stop_abort();
    assert_eq!(live.drafts().len(), 1, "nothing retired while down");
    loop {
        match live.host.orphan_stop() {
            StopState::Ready => break,
            StopState::Aborted(pages) => {
                live.host.stop_abort();
                assert!(Instant::now() < deadline, "still aborted: {pages:?}");
            }
            StopState::Waiting => assert!(Instant::now() < deadline, "never ready"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let Live { host, root, .. } = live;
    let Ok(_stopped) = host.stop_finish() else {
        panic!("the orphan stop finishes");
    };
    assert_eq!(
        fs::read_to_string(root.join("pages/a.md")).unwrap(),
        "- recovered\n"
    );
}
