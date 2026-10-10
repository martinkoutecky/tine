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
    assert_eq!(pages, BTreeSet::from([key.clone()]));
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
