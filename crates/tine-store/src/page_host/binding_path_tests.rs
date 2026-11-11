//! GH #905: a clean held page follows external removal through its owner.
use super::*;

#[test]
fn a_fileless_held_journal_reads_a_late_sync_delivery_without_a_watcher_event() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    let core = live.store.watch.core_for_load();
    core.deaf.store(true, std::sync::atomic::Ordering::Release);
    let (key, _) = live.open("journals/2026_11_10.md");
    assert!(matches!(
        live.store.graph.source(&live.root.join(&key)),
        crate::model::Source::HeldAbsent
    ));
    let temp = live.root.join("journals/.syncthing.2026_11_10.md.tmp");
    let bytes = b"- Journal delivered by sync\n";
    fs::write(&temp, bytes).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&temp).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
    drop(file);
    fs::rename(&temp, live.root.join(&key)).unwrap();
    assert_eq!(
        live.store
            .read(&crate::FileId::from(key.clone()), None)
            .unwrap()
            .0,
        bytes
    );
    let read = live
        .store
        .page(&PageId::from(key))
        .expect("the delivered file must open without a touch or watcher publication");
    assert_eq!(read.doc.blocks[0].raw, "Journal delivered by sync");
}

#[test]
fn a_fileless_held_journal_reads_a_backdated_sync_delivery_with_a_watcher_event() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::build(&[], false, watch);
        let (key, _) = live.open("journals/2026_11_10.md");
        let temp = live.root.join("journals/.syncthing.2026_11_10.md.tmp");
        fs::write(&temp, "- Journal delivered by sync\n").unwrap();
        let file = fs::OpenOptions::new().write(true).open(&temp).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
            .unwrap();
        drop(file);
        fs::rename(&temp, live.root.join(&key)).unwrap();
        live.wait(&key, "the host observes the delivered journal", |mail| {
            matches!(mail.page.as_ref().map(|page| &page.text),
                Some(MailText::Page { dto }) if dto.blocks[0].raw == "Journal delivered by sync")
        });
        let read = live.store.page(&PageId::from(key)).unwrap();
        assert_eq!(read.doc.blocks[0].raw, "Journal delivered by sync");
    }
}

/// The watcher of `live` drops every OS notification (a missed event).
fn deaf(live: &Live) {
    (live.store.watch.core_for_load())
        .deaf
        .store(true, std::sync::atomic::Ordering::Release);
}

/// A sync client's delivery: a temporary file carrying its source's old
/// modification time, renamed onto `rel`.
fn deliver(live: &Live, rel: &str, bytes: &[u8]) {
    let target = live.root.join(rel);
    let leaf = target.file_name().unwrap().to_str().unwrap();
    let temp = target.with_file_name(format!(".syncthing.{leaf}.tmp"));
    fs::write(&temp, bytes).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&temp).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
    drop(file);
    fs::rename(&temp, &target).unwrap();
}

/// The first block of `read`'s page.
fn first(read: Result<crate::PageRead, crate::StoreError>) -> String {
    match read {
        Ok(read) => read.doc.blocks[0].raw.clone(),
        Err(error) => panic!("the page did not open: {error:?}"),
    }
}

/// Whether `mail` shows the page's text as `body` (its buffer).
fn shows(mail: &PageMail, body: &str) -> bool {
    mail.page.as_ref().is_some_and(|page| match &page.text {
        MailText::Page { dto } => dto
            .blocks
            .first()
            .is_some_and(|b| format!("- {}\n", b.raw) == body),
        MailText::Unchanged { rev } => rev.as_ref() == Some(&FileRev::from_bytes(body.as_bytes())),
        _ => false,
    })
}

/// GH #863 (A-H1): a held day with no file on either side is a fresh
/// absence, and repeated reads keep saying where it came from.
#[test]
fn a_fileless_held_journal_reads_as_a_fresh_held_absence_on_repeated_reads() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    let (key, _) = live.open("journals/2026_11_10.md");
    for _ in 0..2 {
        assert!(matches!(
            live.store.page(&PageId::from(key.clone())),
            Err(crate::StoreError::NotFound(
                crate::PageReadSource::HeldAbsent
            ))
        ));
    }
    assert!(!live.root.join(key).exists(), "a read writes nothing");
}

/// GH #863 (A-H1): a held page removed and then restored by a sync client,
/// both unseen by the watcher, reads as each state in turn.
#[test]
fn a_held_page_removed_then_restored_without_a_watcher_event_reads_each_state() {
    let live = Live::build(
        &[("pages/a.md", "- kept\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, _) = live.open("pages/a.md");
    live.until_indexed(&key, "- kept\n");
    let id = PageId::from(key.as_str());
    fs::remove_file(live.root.join(&key)).unwrap();
    assert!(
        matches!(live.store.page(&id), Err(crate::StoreError::NotFound(_))),
        "a removed held page reads as absent"
    );
    live.wait(&key, "the host shows the absence", |mail| {
        matches!(
            mail.page.as_ref().map(|page| &page.text),
            Some(MailText::NoFile)
        )
    });
    deliver(&live, &key, b"- kept\n");
    assert_eq!(first(live.store.page(&id)), "kept");
}

/// GH #863 with GH #875's read table: a missed external write to a held
/// page with unsaved input is read, the input stays in conflict and nothing
/// is written; a later disk equal to the input retires the conflict.
#[test]
fn a_held_page_with_unsaved_input_reads_a_missed_write_and_keeps_the_input() {
    let live = Live::build(
        &[("pages/a.md", "- base\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, page) = live.open("pages/a.md");
    live.until_indexed(&key, "- base\n");
    let id = PageId::from(key.as_str());
    // Its save keeps failing: the input stays unsaved.
    live.faults(Phase::PageTemp, 10_000);
    let request = live.submit(&key, "- typed\n", page.version, None).unwrap();
    live.answer(&key, request);
    live.wait(&key, "the save fails", |mail| mail.notice.save_error);
    deliver(&live, &key, b"- disk\n");
    assert_eq!(
        first(live.store.page(&id)),
        "disk",
        "the read answers the file"
    );
    live.wait(&key, "the input is kept in conflict", |mail| {
        mail.page.as_ref().is_some_and(|page| page.conflict) && shows(mail, "- typed\n")
    });
    assert_eq!(live.disk(&key), "- disk\n", "nothing was written");
    deliver(&live, &key, b"- typed\n");
    assert_eq!(first(live.store.page(&id)), "typed");
    live.wait(
        &key,
        "a disk equal to the input clears the conflict",
        |mail| mail.page.as_ref().is_some_and(|page| !page.conflict),
    );
    assert_eq!(live.disk(&key), "- typed\n");
}

/// GH #863: two surfaces of one held page reading after one missed write
/// share its owner observation: one publication, and the page stays held.
#[test]
fn two_surfaces_reading_a_missed_write_share_one_owner_publication() {
    let live = Live::build(
        &[("pages/a.md", "- one\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, _) = live.open("pages/a.md");
    let (again, _) = live.open("pages/a.md");
    assert_eq!(again, key);
    live.until_indexed(&key, "- one\n");
    deliver(&live, &key, b"- two\n");
    let before = live.store.changes.rev();
    let (store, id) = (&live.store, PageId::from(key.as_str()));
    let reads = std::thread::scope(|scope| {
        let reads: Vec<_> = (0..2).map(|_| scope.spawn(|| store.page(&id))).collect();
        reads
            .into_iter()
            .map(|read| read.join().unwrap())
            .collect::<Vec<_>>()
    });
    for read in reads {
        assert_eq!(first(read), "two");
    }
    assert_eq!(
        live.store.changes.rev().0,
        before.0 + 1,
        "one publication for both reads"
    );
    assert!(live.held(&key), "the read keeps the hold");
    live.wait(&key, "both surfaces get the page's mail", |mail| {
        shows(mail, "- two\n")
    });
}

/// GH #863: reads racing an own save of a held page answer in the owner's
/// order, never older bytes after newer ones, and the index ends at the
/// save's bytes; no read installs a row.
#[test]
fn reads_racing_an_own_save_of_a_held_page_answer_in_host_order() {
    let live = Live::build(&[("pages/a.md", "- a\n")], false, crate::WatchMode::Notify);
    let (key, page) = live.open("pages/a.md");
    live.until_indexed(&key, "- a\n");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let (store, id) = (&live.store, PageId::from(key.as_str()));
    let seen = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut seen = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                seen.push(first(store.page(&id)));
            }
            seen
        });
        let request = live.submit(&key, "- own\n", page.version, None).unwrap();
        live.answer(&key, request);
        live.until_indexed(&key, "- own\n");
        stop.store(true, std::sync::atomic::Ordering::Release);
        reader.join().unwrap()
    });
    let newer = seen
        .iter()
        .position(|raw| raw == "own")
        .unwrap_or(seen.len());
    assert!(seen[..newer].iter().all(|raw| raw == "a"), "{seen:?}");
    assert!(seen[newer..].iter().all(|raw| raw == "own"), "{seen:?}");
    assert_eq!(first(live.store.page(&id)), "own");
    assert_eq!(live.indexed(&key), Some(content_rev("- own\n")));
}

/// GH #863: a pinned day path with no file never answers its same-day twin
/// delivered later under another accepted name; selection by name finds
/// the twin.
#[test]
fn a_pinned_fileless_day_never_answers_its_twin() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    let (key, _) = live.open("journals/2026_11_10.md");
    // The day arrives under its other accepted name after the pin opened.
    deliver(&live, "journals/2026-11-10.md", b"- the ISO twin\n");
    for _ in 0..2 {
        assert!(matches!(
            live.store.page(&PageId::from(key.as_str())),
            Err(crate::StoreError::NotFound(_))
        ));
    }
    let named = (live.store)
        .page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal)
        .unwrap()
        .expect("the day opens by name");
    assert_eq!(named.id.as_str(), "journals/2026-11-10.md");
}

/// GH #863: a held page whose owner's observation fails answers that read
/// error, never an absence, and opens once the owner's retry succeeds.
#[test]
fn a_held_read_whose_owner_read_fails_answers_the_error_not_an_absence() {
    let live = Live::build(
        &[("pages/a.md", "- one\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, _) = live.open("pages/a.md");
    live.until_indexed(&key, "- one\n");
    let id = PageId::from(key.as_str());
    live.faults(Phase::Read, 1);
    deliver(&live, &key, b"- two\n");
    match live.store.page(&id) {
        Err(crate::StoreError::Io(_)) => {}
        Ok(read) => panic!(
            "read {:?} over a failed observation",
            read.doc.blocks[0].raw
        ),
        Err(error) => panic!("not the read error: {error:?}"),
    }
    assert_eq!(
        first(live.store.page(&id)),
        "two",
        "the owner's retry opens it"
    );
}

/// GH #863 (D2): a stamp equal to the owner's last observation vouches for
/// its bytes, unless that observation was racy (spec §5.4): a same-size
/// rewrite in its time granule is then read through the owner.
#[test]
fn a_held_read_trusts_an_equal_stamp_unless_its_observation_was_racy() {
    let live = Live::build(
        &[("pages/a.md", "- aaaa\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, _) = live.open("pages/a.md");
    live.until_indexed(&key, "- aaaa\n");
    let (id, path) = (PageId::from(key.as_str()), live.root.join(&key));
    fs::write(&path, "- bbbb\n").unwrap();
    // What a platform with coarse change times leaves after a rewrite in
    // the granule of the owner's observation: the same metadata.
    live.store
        .watch
        .forge_unchanged_observation_for_tests(&path, false);
    assert_eq!(
        first(live.store.page(&id)),
        "aaaa",
        "an equal stamp is trusted (Rescan graph covers the residual)"
    );
    live.store
        .watch
        .forge_unchanged_observation_for_tests(&path, true);
    assert_eq!(first(live.store.page(&id)), "bbbb", "a racy stamp is not");
}

/// GH #863: a held read whose owner stays busy past the bound answers the
/// owner's indexed page; the owed observation publishes later, and the
/// next read opens the file.
#[test]
fn a_held_read_answers_the_indexed_page_when_its_owner_is_busy_past_the_bound() {
    let live = Live::build(
        &[("pages/a.md", "- one\n")],
        false,
        crate::WatchMode::Notify,
    );
    deaf(&live);
    let (key, _) = live.open("pages/a.md");
    live.until_indexed(&key, "- one\n");
    *live.store.held_read_wait.lock().unwrap() = Some(Duration::from_millis(200));
    let id = PageId::from(key.as_str());
    let busy = live.host.pause_for_tests(std::slice::from_ref(&id));
    deliver(&live, &key, b"- two\n");
    let started = Instant::now();
    assert_eq!(first(live.store.page(&id)), "one");
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "it waited for the owner"
    );
    drop(busy);
    live.until_indexed(&key, "- two\n");
    assert_eq!(first(live.store.page(&id)), "two");
}

/// A held page's read may wait for the page host driver, so it never runs
/// on that driver's thread (A-H1; a debug assertion pins it).
#[cfg(debug_assertions)]
#[test]
fn store_page_never_runs_on_the_page_host_driver_thread() {
    let live = Live::build(
        &[("pages/a.md", "- one\n")],
        false,
        crate::WatchMode::Notify,
    );
    let store = Arc::clone(&live.store);
    let refused = (live.host.driver.shared)
        .call(move |_| {
            let read = || store.page(&PageId::from("pages/a.md")).is_ok();
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).is_err()
        })
        .unwrap();
    assert!(refused, "Store::page ran on the driver thread");
}

/// GH #863 (D3): a day the index has no file for opens a file a sync client
/// delivered unobserved under another accepted name, and a creating save
/// would target that file instead of a duplicate day.
#[test]
fn an_absent_day_opens_a_file_delivered_unobserved_under_another_accepted_name() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    deliver(
        &live,
        "journals/2026-11-10.md",
        b"- delivered under the ISO name\n",
    );
    let read = (live.store)
        .page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal)
        .unwrap()
        .expect("the delivered day opens");
    assert_eq!(read.id.as_str(), "journals/2026-11-10.md");
    assert_eq!(read.doc.blocks[0].raw, "delivered under the ISO name");
    let day = tine_core::date::JournalDate::from_file_stem("2026_11_10").unwrap();
    assert_eq!(
        live.store
            .journal_id(crate::Day(day.ordinal_key()))
            .as_str(),
        "journals/2026-11-10.md",
        "a creating save targets the delivered file"
    );
}

/// The failed observations the driver has counted for `key`.
fn observe_failures(live: &Live, key: &str) -> u32 {
    (live.host.driver.shared)
        .with_state_for_tests(|state| state.observe.get(key).map_or(0, |o| o.failures))
}

/// GH #863 (A-H1): a held read whose owner's reads keep failing, so the
/// driver's own retries have backed off, has its observation made now: it
/// answers that read error within its wait, never an absence.
#[test]
fn a_held_read_after_repeated_owner_read_failures_answers_the_error_within_its_wait() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    let (key, _) = live.open("journals/2026_11_10.md");
    let id = PageId::from(key.as_str());
    live.faults(Phase::Read, 10);
    deliver(&live, &key, b"- Journal delivered by sync\n");
    assert!(matches!(
        live.store.page(&id),
        Err(crate::StoreError::Io(_))
    ));
    // The driver retries on its own after 100 ms and 300 ms; its third
    // failure puts the next retry 1 s away (`progress::backoff`).
    let deadline = Instant::now() + Duration::from_secs(10);
    while observe_failures(&live, &key) < 3 {
        assert!(Instant::now() < deadline, "the owner never retried");
        std::thread::sleep(Duration::from_millis(5));
    }
    *live.store.held_read_wait.lock().unwrap() = Some(Duration::from_millis(300));
    match live.store.page(&id) {
        Err(crate::StoreError::Io(_)) => {}
        Ok(read) => panic!("read {:?} over failed reads", read.doc.blocks[0].raw),
        Err(error) => panic!("not the read error within the wait: {error:?}"),
    }
    assert!(
        observe_failures(&live, &key) >= 4,
        "the read's own observation failed"
    );
}

/// GH #863 (D3, A-H1): a held day whose owner last published its absence
/// opens by name the file a sync client delivered unobserved, also while
/// its owner is busy: the lookup waits for the observation it asked for.
#[test]
fn a_held_absent_day_opens_by_name_a_missed_delivery_its_busy_owner_observes() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    let (key, _) = live.open("journals/2026_11_10.md");
    let id = PageId::from(key.as_str());
    let busy = live.host.pause_for_tests(std::slice::from_ref(&id));
    deliver(&live, &key, b"- Journal delivered by sync\n");
    let store = Arc::clone(&live.store);
    let named = std::thread::spawn(move || {
        store.page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal)
    });
    // The owner stays busy until the lookup waits for it (or answered).
    let deadline = Instant::now() + Duration::from_secs(10);
    let waiting = |live: &Live| {
        (live.host.driver.shared).with_state_for_tests(|state| state.observed.contains_key(&key))
    };
    while !named.is_finished() && !waiting(&live) {
        assert!(
            Instant::now() < deadline,
            "the lookup never asked the owner"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(busy);
    let read = (named.join().unwrap().unwrap()).expect("the delivered day opens by name");
    assert_eq!(read.id.as_str(), key);
    assert_eq!(read.doc.blocks[0].raw, "Journal delivered by sync");
}

/// GH #863 (D3, review r3 finding 1): a day delivered unobserved under
/// another accepted name that cannot be read (here invalid UTF-8) answers
/// its read error, never an empty day the user would then fill as a
/// duplicate.
#[test]
fn an_absent_day_whose_unobserved_delivery_cannot_be_read_answers_the_error() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    deliver(&live, "journals/2026-11-10.md", b"- \xff\xfe not UTF-8\n");
    // The watcher's refusal keeps the file's stamp: a second lookup sees it
    // unchanged since observed and must still answer the error (review r4).
    for lookup in 1..=2 {
        match (live.store).page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal) {
            Err(crate::StoreError::NotFound(_)) | Ok(None) => {
                panic!("lookup {lookup}: an unreadable delivered day answered absent")
            }
            Ok(Some(read)) => panic!(
                "lookup {lookup}: read {:?} from invalid UTF-8",
                read.doc.blocks
            ),
            Err(_) => {}
        }
    }
}

/// GH #863 (D3, review r4 finding 2): a journals folder whose entries
/// cannot be stat'ed (here no search permission) answers that error, never
/// an empty day.
#[cfg(unix)]
#[test]
fn an_absent_day_in_an_unsearchable_journals_folder_answers_the_error() {
    use std::os::unix::fs::PermissionsExt;
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    deliver(&live, "journals/2026-11-10.md", b"- delivered\n");
    let journals = live.root.join("journals");
    fs::set_permissions(&journals, fs::Permissions::from_mode(0o600)).unwrap();
    let probe = fs::symlink_metadata(journals.join("2026-11-10.md"));
    let named = (live.store).page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal);
    fs::set_permissions(&journals, fs::Permissions::from_mode(0o755)).unwrap();
    if probe.is_ok() {
        return; // running with privileges that bypass the permission bits
    }
    match named {
        Err(crate::StoreError::NotFound(_)) | Ok(None) => {
            panic!("an unsearchable journals folder answered absent")
        }
        Ok(Some(read)) => panic!("read {:?} without permission", read.doc.blocks),
        Err(_) => {}
    }
}

/// GH #863 (D3, review r3 finding 2): a day delivered unobserved under
/// another accepted name that a host holds before its owner's first
/// publication (its Open read still waiting) opens with that file's read,
/// which stays unpublished: the owner alone indexes it.
#[test]
fn an_absent_day_held_before_its_first_publication_opens_its_delivered_file() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    let rel = "journals/2026-11-10.md";
    deliver(&live, rel, b"- delivered before the owner read it\n");
    let id = PageId::from(rel);
    let busy = live.host.pause_for_tests(std::slice::from_ref(&id));
    let generation = live.host.session();
    live.host.open(generation, 1, &id, "2026-11-10").unwrap();
    assert!(matches!(
        live.store.graph.source(&live.root.join(rel)),
        crate::model::Source::HeldUnindexed
    ));
    let read = (live.store)
        .page_named("Nov 10th, 2026", tine_core::model::PageKind::Journal)
        .unwrap()
        .expect("the held delivered day opens");
    assert_eq!(read.id.as_str(), rel);
    assert_eq!(read.doc.blocks[0].raw, "delivered before the owner read it");
    assert!(
        matches!(
            live.store.graph.source(&live.root.join(rel)),
            crate::model::Source::HeldUnindexed
        ),
        "the lookup published a held page"
    );
    drop(busy);
}

/// GH #863 (D3): a day delivered unobserved under an unpadded name the
/// parser accepts (by default `2026-11-1` is November 1) opens by name, and
/// a creating save targets it rather than a duplicate day.
#[test]
fn an_absent_day_opens_an_unpadded_delivery_and_proposes_no_duplicate() {
    let live = Live::build(&[], false, crate::WatchMode::Notify);
    deaf(&live);
    deliver(&live, "journals/2026-11-1.md", b"- delivered unpadded\n");
    let read = (live.store)
        .page_named("Nov 1st, 2026", tine_core::model::PageKind::Journal)
        .unwrap()
        .expect("the delivered day opens");
    assert_eq!(read.id.as_str(), "journals/2026-11-1.md");
    assert_eq!(read.doc.blocks[0].raw, "delivered unpadded");
    let day = tine_core::date::JournalDate::from_file_stem("2026_11_01").unwrap();
    assert_eq!(
        live.store
            .journal_id(crate::Day(day.ordinal_key()))
            .as_str(),
        "journals/2026-11-1.md",
        "a creating save targets the delivered file"
    );
}

#[test]
fn a_live_subfolder_move_of_a_clean_held_page_retires_its_old_claimant() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        let live = Live::build(
            &[("pages/shared-elias/Survey test.md", "- real content\n")],
            false,
            watch,
        );
        let (key, _) = live.open("pages/shared-elias/Survey test.md");
        assert!(
            matches!(live.store.whole_graph().unwrap().resolve("Survey test", false), crate::Resolved::Existing { id, others } if id.as_str() == key && others.is_empty())
        );
        fs::create_dir_all(live.root.join("pages/live")).unwrap();
        fs::copy(
            live.root.join(&key),
            live.root.join("pages/live/Survey test.md"),
        )
        .unwrap();
        fs::remove_file(live.root.join(&key)).unwrap();
        live.store.refresh(crate::Depth::Stamps).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if matches!(live.store.whole_graph().unwrap().resolve("Survey test", false), crate::Resolved::Existing { id, others } if id.as_str() == "pages/live/Survey test.md" && others.is_empty())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the old live claimant was not retired"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn closing_a_stale_sidebar_path_does_not_create_a_phantom_file_claimant() {
    let live = Live::build(&[], true, crate::WatchMode::Poll);
    // Session restore can open the stale path before launch has found the
    // real file. Once found, that empty host page must not claim a disk file.
    let (stale, _) = live.open("pages/Survey test.md");
    fs::create_dir_all(live.root.join("pages/shared-elias")).unwrap();
    fs::write(
        live.root.join("pages/shared-elias/Survey test.md"),
        "- real content\n",
    )
    .unwrap();
    live.store.refresh(crate::Depth::Stamps).unwrap();
    let id = live.id();
    live.host.close(live.host.session(), id, &stale).unwrap();
    assert!(live.host.quiesce_for_tests(Duration::from_secs(10)));
    match live
        .store
        .whole_graph()
        .unwrap()
        .resolve("Survey test", false)
    {
        crate::Resolved::Existing { id, others } => {
            assert_eq!(id.as_str(), "pages/shared-elias/Survey test.md");
            assert!(others.is_empty());
        }
        _ => panic!("the real file must remain the unique claimant"),
    }
}
