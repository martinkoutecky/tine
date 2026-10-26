//! L2-04: delete a conflict before consuming its input, with both sides recoverable.
use super::*;

#[test]
fn f6_recovery_staging_leaves_driver_available_and_checks_the_staged_draft() {
    for absent in [false, true] {
        for change_draft in [false, true] {
            let live = Live::new(&[("pages/a.md", "- base\n")]);
            let (key, page) = live.open("pages/a.md");
            if absent {
                fs::remove_file(live.root.join(&key)).unwrap();
            } else {
                fs::write(live.root.join(&key), "- external\n").unwrap();
            }
            let id = live.submit(&key, "- draft\n", page.version, None).unwrap();
            live.answer(&key, id);
            live.wait(&key, "conflict", |mail| {
                mail.page.as_ref().is_some_and(|p| p.conflict)
            });
            let pause: crate::store::TestPause = Default::default();
            *live.store.marker_copy_pause.lock().unwrap() = Some(pause.clone());
            std::thread::scope(|scope| {
                let deleting = scope.spawn(|| {
                    live.host.delete(
                        live.host.session(),
                        &PageId::from(key.clone()),
                        (!absent).then_some(b"- external\n".as_slice()),
                    )
                });
                let (lock, ready) = &*pause;
                let mut at = lock.lock().unwrap();
                while !at.0 {
                    let answer = ready.wait_timeout(at, Duration::from_secs(10)).unwrap();
                    assert!(!answer.1.timed_out(), "staging was never reached");
                    at = answer.0;
                }
                drop(at);
                let shared = live.host.driver.shared.clone();
                let (send, receive) = mpsc::channel();
                scope.spawn(move || {
                    let _ = send.send(shared.read(|state| state.book.session));
                });
                let available = receive.recv_timeout(Duration::from_secs(1)).is_ok();
                if available && change_draft {
                    let current = key.clone();
                    let version = live
                        .host
                        .driver
                        .shared
                        .read(move |state| state.progress.host.pages[&current].version)
                        .unwrap();
                    let id = live.submit(&key, "- newer draft\n", version, None).unwrap();
                    live.answer(&key, id);
                }
                lock.lock().unwrap().1 = true;
                ready.notify_all();
                let outcome = deleting.join().unwrap();
                assert!(available, "recovery staging blocked the host driver");
                assert_eq!(
                    outcome,
                    if change_draft {
                        PageOperation::Waiting
                    } else {
                        PageOperation::Applied
                    }
                );
            });
            let copies: Vec<_> = fs::read_dir(live.root.join("logseq/.tine-trash/conflicts"))
                .unwrap()
                .flatten()
                .collect();
            assert!(copies
                .iter()
                .any(|entry| fs::read(entry.path()).unwrap() == b"- draft\n"));
            if change_draft {
                live.host.driver.shared.with_state_for_tests(|state| {
                    assert_eq!(
                        state.progress.host.pages[&key].buf.as_deref(),
                        Some(b"- newer draft\n".as_slice())
                    );
                });
                assert_eq!(
                    fs::read(live.root.join(&key)).ok(),
                    (!absent).then(|| b"- external\n".to_vec())
                );
            }
            live.host.stop();
        }
    }
}

#[test]
fn l9_04_resolving_submit_copies_disk_before_publish_and_refuses_if_recovery_fails() {
    for blocks in [1, 60] {
        let disk = "- external\n".repeat(blocks);
        let local = "- local\n".repeat(blocks);
        let live = Live::new(&[("pages/a.md", &disk)]);
        let (key, page) = live.open("pages/a.md");
        let trash = live.root.join("logseq/.tine-trash/conflicts");
        fs::create_dir_all(trash.parent().unwrap()).unwrap();
        fs::write(&trash, "unavailable recovery").unwrap();
        let token = DiskToken::File(FileRev::from_bytes(disk.as_bytes()));
        assert!(live
            .submit(&key, &local, page.version, Some(&token))
            .is_err());
        assert_eq!(live.disk(&key), disk);
        fs::rename(&trash, trash.with_extension("obstruction")).unwrap();
        let id = live
            .submit(&key, &local, page.version, Some(&token))
            .unwrap();
        let version = live.answer(&key, id).page.unwrap().version;
        live.host.save_now(live.host.session(), &[key.clone()]);
        assert_eq!(
            live.host.wait_published(
                live.host.session(),
                &vec![(key.clone(), version, None)],
                Duration::from_secs(10)
            ),
            PageOperation::Applied
        );
        assert_eq!(live.disk(&key), local);
        let files: Vec<_> = fs::read_dir(trash).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let recovered = fs::read(files[0].path()).unwrap();
        assert_eq!(recovered, disk.as_bytes());
        assert_eq!(recovered.len(), blocks * 11);
        live.host.stop();
    }
}

#[test]
fn l2_04_conflicted_delete_preserves_input_on_refusal_and_deletes_on_success() {
    for blocks in [1, 60] {
        let local = "- local draft\n".repeat(blocks);
        let external = "- external\n".repeat(blocks);
        let live = Live::new(&[("pages/a.md", "- base\n")]);
        let (key, page) = live.open("pages/a.md");
        fs::write(live.root.join(&key), &external).unwrap();
        let id = live.submit(&key, &local, page.version, None).unwrap();
        live.answer(&key, id);
        live.wait(&key, "conflict", |mail| {
            mail.page.as_ref().is_some_and(|p| p.conflict)
        });
        let trash = live.root.join("logseq/.tine-trash/conflicts");
        fs::create_dir_all(trash.parent().unwrap()).unwrap();
        fs::write(&trash, "unavailable trash directory").unwrap();
        assert_eq!(
            live.host.delete(
                live.host.session(),
                &PageId::from(key.clone()),
                Some(external.as_bytes())
            ),
            PageOperation::Refused
        );
        assert_eq!(live.disk(&key), external);
        live.host.driver.shared.with_state_for_tests(|state| {
            let page = &state.progress.host.pages[&key];
            assert_eq!(page.buf.as_deref(), Some(local.as_bytes()));
            assert!(page.conflict);
        });
        fs::rename(&trash, trash.with_extension("obstruction")).unwrap();
        assert_eq!(
            live.host.delete(
                live.host.session(),
                &PageId::from(key.clone()),
                Some(external.as_bytes())
            ),
            PageOperation::Applied
        );
        assert!(!live.root.join(&key).exists());
        fn retained(dir: &Path, bytes: &[u8]) -> bool {
            fs::read_dir(dir).unwrap().flatten().any(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    retained(&path, bytes)
                } else {
                    fs::read(path).unwrap() == bytes
                }
            })
        }
        assert!(retained(
            &live.root.join("logseq/.tine-trash"),
            local.as_bytes()
        ));
        assert!(retained(
            &live.root.join("logseq/.tine-trash"),
            external.as_bytes()
        ));
        let copies: Vec<_> = fs::read_dir(&trash).unwrap().flatten().collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(
            fs::metadata(copies[0].path()).unwrap().len(),
            (blocks * 14) as u64
        );
        live.host.stop();
    }
}

#[test]
fn a_conflicted_delete_of_an_externally_deleted_file_keeps_the_input_recoverable() {
    // L9 review R1: the file vanished while the page held input; deleting
    // the page has nothing to delete but must copy the draft first.
    let local = "- local draft\n";
    let live = Live::new(&[("pages/a.md", "- base\n")]);
    let (key, page) = live.open("pages/a.md");
    fs::remove_file(live.root.join(&key)).unwrap();
    let id = live.submit(&key, local, page.version, None).unwrap();
    live.answer(&key, id);
    live.wait(&key, "conflict", |mail| {
        mail.page.as_ref().is_some_and(|p| p.conflict)
    });
    let page_id = PageId::from(key.clone());
    let trash = live.root.join("logseq/.tine-trash/conflicts");
    fs::create_dir_all(trash.parent().unwrap()).unwrap();
    fs::write(&trash, "unavailable trash directory").unwrap();
    assert_eq!(
        live.host.delete(live.host.session(), &page_id, None),
        PageOperation::Refused
    );
    live.host.driver.shared.with_state_for_tests(|state| {
        let page = &state.progress.host.pages[&key];
        assert_eq!(page.buf.as_deref(), Some(local.as_bytes()));
        assert!(page.conflict);
    });
    fs::rename(&trash, trash.with_extension("obstruction")).unwrap();
    assert_eq!(
        live.host.delete(live.host.session(), &page_id, None),
        PageOperation::Applied
    );
    assert!(!live.root.join(&key).exists());
    let copies: Vec<_> = fs::read_dir(&trash).unwrap().flatten().collect();
    assert_eq!(copies.len(), 1);
    assert_eq!(fs::read(copies[0].path()).unwrap(), local.as_bytes());
    // A reappeared file is not this case: refuse, keep the input.
    fs::write(live.root.join(&key), "- back\n").unwrap();
    assert_eq!(
        live.host.delete(live.host.session(), &page_id, None),
        PageOperation::Refused
    );
    live.host.stop();
}
