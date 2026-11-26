//! tine#1041: the wait of a window leaving its graph for another
//! (`wait_departure`), against a real Store. It sets aside a conflicted page
//! no window is subscribed to once the page's durable draft is exact, and is
//! `wait_published` for every other need.
use super::*;

const LONG: Duration = Duration::from_secs(20);

/// `pages/a.md` edited to `- mine`, then written as `- theirs` on disk, then
/// closed by its window: the host keeps the conflicted page, and no window
/// is subscribed to it. Returns its key and version.
fn left_in_conflict(live: &Live) -> (PageKey, u64) {
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- mine\n", page.version, None).unwrap();
    let version = live.answer(&key, id).answer.unwrap().version;
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    assert_eq!(
        live.host
            .wait_published(live.host.session(), &[(key.clone(), version, None)], LONG),
        (
            PageOperation::Refused,
            vec![],
            vec!["pages/a.md".to_string()]
        ),
        "the page is in conflict"
    );
    live.close(&key);
    (key, version)
}

/// Wait until the host's state for `key` passes `test`.
fn until(live: &Live, key: &str, what: &str, test: fn(&Host<ProductionIo>, &str, &Page) -> bool) {
    let deadline = Instant::now() + LONG;
    loop {
        let page = key.to_string();
        let read = move |state: &State<ProductionIo, SystemClock>| {
            let host = &state.progress.host;
            matches!(host.logical(&page), Logical::Settled(held) if test(host, &page, held))
        };
        if live.host.driver.shared.read(read).unwrap() {
            return;
        }
        assert!(Instant::now() < deadline, "never {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The reporter's state: the conflicted page is closed and its draft exact.
fn until_left_to_its_draft(live: &Live, key: &str) {
    until(live, key, "left to its draft", |host, key, page| {
        page.conflict
            && !host.subscriptions.contains(key)
            && !host.busy(key)
            && host.exact_copy(key, page)
    });
}

fn refused(files: &[&str]) -> (PageOperation, Vec<PublishedRevision>, Vec<String>) {
    let files = files.iter().map(|file| file.to_string()).collect();
    (PageOperation::Refused, vec![], files)
}

/// The reporter's case: a conflicted page no window shows, with an exact
/// durable draft, does not hold a departure. The publication wait on the
/// same need still refuses and names it.
#[test]
fn a_departure_leaves_a_closed_conflicted_page_to_its_exact_draft() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, version) = left_in_conflict(&live);
    until_left_to_its_draft(&live, &key);
    let session = live.host.session();
    let needs = [(key.clone(), version, None)];
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG),
        (PageOperation::Applied, vec![], vec![]),
        "set aside, and no revision claims it published"
    );
    assert_eq!(
        live.host.wait_published(session, &needs, LONG),
        refused(&["pages/a.md"]),
        "publication is unchanged"
    );
    assert_eq!(live.disk(&key), "- theirs\n", "nothing was written");
    // A need for a version the page has not reached is not set aside.
    let later = [(key.clone(), version + 1, None)];
    assert_eq!(
        live.host.wait_departure(session, &later, LONG),
        refused(&["pages/a.md"])
    );
    // Nor is a need with a block witness.
    let witnessed = [(key.clone(), version, Some("some-block".to_string()))];
    assert_eq!(
        live.host.wait_departure(session, &witnessed, LONG),
        refused(&["pages/a.md"])
    );
    live.host.stop();
}

/// A page a window is subscribed to is never set aside: before its window
/// closes it, and again once a window opens it after a departure passed.
#[test]
fn a_departure_refuses_for_a_conflicted_page_a_window_shows() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, page) = live.open("pages/a.md");
    let id = live.submit(&key, "- mine\n", page.version, None).unwrap();
    let version = live.answer(&key, id).answer.unwrap().version;
    fs::write(live.root.join(&key), "- theirs\n").unwrap();
    let session = live.host.session();
    let needs = [(key.clone(), version, None)];
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG),
        refused(&["pages/a.md"]),
        "shown in a window"
    );
    live.close(&key);
    until_left_to_its_draft(&live, &key);
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG).0,
        PageOperation::Applied
    );
    live.open("pages/a.md");
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG),
        refused(&["pages/a.md"]),
        "opened again"
    );
    live.host.stop();
}

/// Without an exact durable draft the page holds the departure, as it holds
/// a publication wait: here the draft cannot be written at all.
#[test]
fn a_departure_refuses_for_a_conflicted_page_without_an_exact_draft() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    live.faults(super::super::io::Phase::DraftTemp, 10_000);
    let (key, version) = left_in_conflict(&live);
    until(&live, &key, "closed in conflict", |host, key, page| {
        page.conflict && !host.subscriptions.contains(key)
    });
    let session = live.host.session();
    let needs = [(key.clone(), version, None)];
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG),
        refused(&["pages/a.md"])
    );
    assert_eq!(
        live.host.wait_published(session, &needs, LONG),
        refused(&["pages/a.md"])
    );
    drop_failed(live);
}

/// In one batch with an ordinary page: the page set aside neither refuses
/// the batch nor is waited on, and the other need is waited on as ever.
/// Its revision is the only one returned.
#[test]
fn a_departure_still_waits_for_the_other_pages_of_its_batch() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    live.submits_wait();
    let (a, va) = left_in_conflict(&live);
    until_left_to_its_draft(&live, &a);
    let (b, page) = live.open("pages/b.md");
    let session = live.host.session();
    let ahead = [(a.clone(), va, None), (b.clone(), page.version + 1, None)];
    assert_eq!(
        live.host
            .wait_departure(session, &ahead, Duration::from_millis(300)),
        (PageOperation::Pending, vec![], vec![]),
        "b is unpublished: pending, not a refusal naming a"
    );
    assert_eq!(
        live.host.wait_published(session, &ahead, LONG),
        refused(&["pages/a.md"]),
        "publication refuses the same batch for a"
    );
    let id = live.submit(&b, "- b two\n", page.version, None).unwrap();
    let vb = live.answer(&b, id).answer.unwrap().version;
    let needs = [(a.clone(), va, None), (b.clone(), vb, None)];
    let (outcome, revisions, files) = live.host.wait_departure(session, &needs, LONG);
    assert_eq!((outcome, files), (PageOperation::Applied, vec![]));
    let published: Vec<_> = revisions
        .iter()
        .map(|r| (r.key.as_str(), r.version))
        .collect();
    assert_eq!(published, [(b.as_str(), vb)], "only b published");
    assert_eq!(live.disk(&b), "- b two\n");
    assert_eq!(live.disk(&a), "- theirs\n");
    live.host.stop();
}

/// In one batch with a page that refuses on its own: the refusal names that
/// page only. The publication wait names both.
#[test]
fn a_departure_refusal_does_not_name_the_page_set_aside() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    live.submits_wait();
    let (a, va) = left_in_conflict(&live);
    until_left_to_its_draft(&live, &a);
    let (b, page) = live.open("pages/b.md");
    let id = live.submit(&b, "- b mine\n", page.version, None).unwrap();
    let vb = live.answer(&b, id).answer.unwrap().version;
    fs::write(live.root.join(&b), "- b theirs\n").unwrap();
    let session = live.host.session();
    let needs = [(a.clone(), va, None), (b.clone(), vb, None)];
    assert_eq!(
        live.host.wait_departure(session, &needs, LONG),
        refused(&["pages/b.md"])
    );
    assert_eq!(
        live.host.wait_published(session, &needs, LONG),
        refused(&["pages/a.md", "pages/b.md"])
    );
    live.host.stop();
}

/// Input admitted for the page and not applied yet is not set aside: the
/// page is not settled.
#[test]
fn a_departure_waits_on_input_admitted_for_the_page() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, version) = left_in_conflict(&live);
    until_left_to_its_draft(&live, &key);
    live.pause(true);
    live.submit(&key, "- more\n", version, None).unwrap();
    let session = live.host.session();
    assert_ne!(
        live.host
            .wait_departure(
                session,
                &[(key.clone(), version, None)],
                Duration::from_millis(300)
            )
            .0,
        PageOperation::Applied
    );
    live.pause(false);
    live.host.stop();
}

/// What the departure promised holds through the stop that follows it: the
/// old host stops with the page's draft still on disk and the file as the
/// other writer left it, also when a window opened the page in between.
#[test]
fn the_host_stops_after_a_departure_with_the_draft_kept() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    live.submits_wait();
    let (key, version) = left_in_conflict(&live);
    until_left_to_its_draft(&live, &key);
    let session = live.host.session();
    assert_eq!(
        live.host
            .wait_departure(session, &[(key.clone(), version, None)], LONG)
            .0,
        PageOperation::Applied
    );
    live.open("pages/a.md");
    let deadline = Instant::now() + LONG;
    loop {
        match live.host.orphan_stop() {
            StopState::Waiting => {
                assert!(Instant::now() < deadline, "the stop never settled");
                std::thread::sleep(Duration::from_millis(20));
            }
            state => {
                assert_eq!(state, StopState::Ready);
                break;
            }
        }
    }
    let drafts = live.drafts();
    assert!(!drafts.is_empty(), "the draft is kept for the next launch");
    assert_eq!(live.disk(&key), "- theirs\n");
    let dir = live.app().join("drafts-v2").join("test-graph");
    let Live {
        _dir, root, host, ..
    } = live;
    assert!(host.stop_finish().is_ok());
    for name in drafts {
        assert!(dir.join(name).is_file(), "the stop keeps the draft");
    }
    assert_eq!(
        fs::read_to_string(root.join("pages/a.md")).unwrap(),
        "- theirs\n"
    );
}
