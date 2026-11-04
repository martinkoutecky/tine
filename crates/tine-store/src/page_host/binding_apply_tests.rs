//! A feature's planned write as one host operation (host-only-writer P2,
//! DESIGN §3.1), against a real Store.
use super::super::apply::{ApplyRefusal, Effect, Expected, HostOp};
use super::*;

/// Replacement for a_request_queued_before_a_spelling_move_applies_to_the_same_page_after_it:
/// the driver refuses unsaved input rather than handing it to an external writer.
#[test]
fn a_spelling_move_with_pending_input_is_refused_and_preserves_input_and_spelling() {
    let live = Live::new(&[("pages/old.md", "- old\n")]);
    let src = PageId::from("pages/old.md");
    let dst = PageId::from("pages/Old.md");
    let (key, page) = live.open(src.as_str());
    let pause = live.host.pause_for_tests(&[src.clone()]);
    let id = live
        .submit(&key, "- pending\n", page.version, None)
        .unwrap();
    assert_eq!(
        live.host
            .move_spelling(src.clone(), dst, key.clone(), bytes("- old\n"), None, None),
        Err(ApplyRefusal::Unsaved(src.clone()))
    );
    assert_eq!(live.host.spelling(&key), src);
    assert_eq!(live.disk(&key), "- old\n");
    assert_eq!(
        fs::read_dir(live.root.join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>(),
        [std::ffi::OsString::from("old.md")]
    );
    drop(pause);
    live.answer(&key, id);
    live.until_disk("pages/old.md", "- pending\n");
    assert_eq!(live.host.spelling(&key).as_str(), "pages/old.md");
    live.host.stop();
}

#[path = "binding_delete_tests.rs"]
mod delete_tests;
#[path = "binding_l8_tests.rs"]
mod l8_tests;

#[test]
fn review_binding_reports_unsaved_for_settled_dirty_input() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, _page) = live.open("pages/a.md");
    live.pause(true);
    live.host.driver.shared.with_state_for_tests(|state| {
        state.progress.with_host(|host| {
            // Freeze the valid post-submit page shape before the automatic save,
            // and reconcile the production progress bookkeeping at its boundary.
            host.version += 1;
            let page = host.pages.get_mut(&key).unwrap();
            page.buf = Some(bytes("- typed\n"));
            page.typed = true;
            page.version = host.version;
            host.mark(&key);
            assert!(!host.pages[&key].clean());
            assert!(matches!(
                host.logical(&key),
                crate::page_host::Logical::Settled(_)
            ));
        });
    });
    let result = live.host.apply(op(
        vec![(
            "pages/a.md",
            Effect::Write(bytes("- planned\n")),
            Expected::Bytes(bytes("- typed\n")),
        )],
        Input::Refuse,
    ));
    assert_eq!(
        result,
        Err(ApplyRefusal::Unsaved(PageId::from("pages/a.md")))
    );
    live.host.stop();
}

fn bytes(text: &str) -> Arc<[u8]> {
    Arc::from(text.as_bytes())
}

/// R6: page failures use the actual host, whose guarantee is retained input
/// and recoverable deletion, rather than transaction-wide rollback. Keep the
/// independent live/fresh oracle before and after retry and a cold relaunch.
#[test]
fn page_operation_failures_retain_input_and_fresh_indexes_through_relaunch() {
    use crate::test_transactions::conformance_f3::fresh;
    for (action, phase) in [
        ("create", Phase::PageTemp),
        ("create", Phase::PageRename),
        ("replace", Phase::PageTemp),
        ("replace", Phase::PageRename),
        ("move", Phase::PageTemp),
        ("move", Phase::PageRename),
        ("move", Phase::TrashMove),
        ("delete", Phase::TrashMove),
    ] {
        let live = Live::new(&[("pages/A.md", "- original [[Shared]]\n")]);
        let source = PageId::from("pages/A.md");
        let destination = PageId::from("pages/B.md");
        let rev = live.store.read(&source.file(), None).unwrap().1;
        let mut plan = live.store.page_plan(Some(EditKind::ReplacePage));
        match action {
            "create" => {
                plan.create(
                    &destination.file(),
                    crate::Content::Bytes(b"- created [[Shared]]\n".to_vec()),
                );
            }
            "replace" => {
                plan.replace_page_bytes(&source, rev, b"- replacement [[Shared]]\n".to_vec());
            }
            "move" => {
                plan.move_file(&source.file(), rev, &destination.file(), None);
            }
            "delete" => {
                plan.trash(&source.file(), rev, crate::TrashIf::Any);
            }
            _ => unreachable!(),
        }
        let planned = plan.plan().unwrap();
        let effects = planned.pages.clone();
        let at = format!("{action}/{phase:?}");
        fresh(&live.store, &live.root, &["Shared"], &at);
        live.faults(phase, 10_000);
        let outcome = live.host.apply(planned);
        assert!(
            matches!(
                outcome,
                Err(ApplyRefusal::Unwritten(_) | ApplyRefusal::Uncertain)
            ),
            "{at}: {outcome:?}"
        );
        // Failed creates and replaces leave the original untouched. The move
        // may already have published its receiver before source trash fails.
        assert_eq!(
            live.disk(source.as_str()),
            "- original [[Shared]]\n",
            "{at}"
        );
        if action != "move" || phase != Phase::TrashMove {
            assert!(!live.root.join(destination.as_str()).exists(), "{at}");
        } else {
            assert_eq!(live.disk(destination.as_str()), "- original [[Shared]]\n");
        }
        for (page, effect, _) in &effects {
            let expected = match effect {
                Effect::Write(bytes) => Some(bytes.clone()),
                Effect::Delete => None,
            };
            assert_eq!(
                live.host.read(page).unwrap().bytes,
                expected,
                "{at}: {page:?}"
            );
        }
        fresh(&live.store, &live.root, &["Shared"], &at);
        let app = live.app();
        let Live {
            host,
            store,
            root,
            _dir,
            ..
        } = live;
        drop(host);
        store.close();
        let cold = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        fresh(&cold, &root, &["Shared"], &format!("cold {at}"));
        let host = PageHost::start_for_tests(&cold, &app).unwrap();
        for (index, (page, _, _)) in effects.iter().enumerate() {
            host.open(host.session(), index as u64 + 1, page, "recovered")
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let complete = effects.iter().all(|(page, effect, _)| match effect {
                Effect::Write(bytes) => {
                    host.read(page).unwrap().bytes.as_deref() == Some(bytes.as_ref())
                }
                Effect::Delete => host.read(page).unwrap().bytes.is_none(),
            });
            let indexed = host.driver.shared.with_state_for_tests(|state| {
                effects
                    .iter()
                    .all(|(page, _, _)| state.book.index.contains_key(page.as_str()))
            });
            if complete && indexed {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "lost admitted input after relaunch: {at}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        fresh(&cold, &root, &["Shared"], &format!("relaunched {at}"));
        if matches!(action, "move" | "delete") && !root.join(source.as_str()).exists() {
            let trash = root.join("logseq/.tine-trash/pages");
            assert!(
                crate::test_transactions::conformance_f3::collect(&trash)
                    .values()
                    .any(|bytes| bytes == b"- original [[Shared]]\n"),
                "{at}"
            );
        }
        drop(host);
        cold.close();
        drop(_dir);
    }
}

#[test]
fn an_uncertain_admitted_apply_records_its_later_publication() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    let sink = Arc::new(Mutex::new(BTreeSet::new()));
    let mut planned = op(
        vec![(
            "pages/a.md",
            Effect::Write(bytes("- pulled\n")),
            Expected::Bytes(bytes("- old\n")),
        )],
        Input::Refuse,
    );
    planned.publications = Some(sink.clone());
    // Pause the real draft worker after admission until settle's production
    // deadline expires. No fabricated receipt or reply stands in for it.
    live.pause(true);
    assert_eq!(live.host.apply(planned), Err(ApplyRefusal::Uncertain));
    assert!(sink.lock().unwrap().is_empty());
    assert_eq!(live.disk("pages/a.md"), "- old\n");
    live.pause(false);
    live.until_disk("pages/a.md", "- pulled\n");
    // The same driver transition records the sink before exposing the file
    // to a later command; synchronize with that driver before checking it.
    live.host.driver.shared.with_state_for_tests(|_| ());
    assert_eq!(
        *sink.lock().unwrap(),
        BTreeSet::from([PageId::from("pages/a.md")])
    );
    live.host.stop();
}

#[test]
fn a_window_save_back_to_h_does_not_erase_the_publication_receipt() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    let sink = Arc::new(Mutex::new(BTreeSet::new()));
    let mut planned = op(
        vec![(
            "pages/a.md",
            Effect::Write(bytes("- pulled\n")),
            Expected::Bytes(bytes("- old\n")),
        )],
        Input::Refuse,
    );
    planned.publications = Some(sink.clone());
    assert_eq!(live.host.apply(planned), Ok(()));
    let (key, page) = live.open("pages/a.md");
    let request = live.submit(&key, "- old\n", page.version, None).unwrap();
    live.answer(&key, request);
    live.until_disk("pages/a.md", "- old\n");
    assert_eq!(
        *sink.lock().unwrap(),
        BTreeSet::from([PageId::from("pages/a.md")])
    );
    live.host.stop();
}

fn op(pages: Vec<(&str, Effect, Expected)>, policy: Input) -> HostOp {
    HostOp {
        separate_case_entries: false,
        inputs: None,
        creation_checks: Vec::new(),
        pages: pages
            .into_iter()
            .map(|(page, effect, expected)| (PageId::from(page), effect, expected))
            .collect(),
        kinds: vec![EditKind::InsertBlocks],
        policy,
        moves: Vec::new(),
        publications: None,
        view: None,
    }
}

#[test]
fn selection_refuses_changed_unsaved_and_new_unindexed_referrers() {
    for newly_held in [false, true] {
        let live = Live::new(&[("pages/a.md", "- old\n")]);
        let empty = live.host.inputs().unwrap();
        let (key, page) = live.open("pages/unindexed.md");
        live.faults(super::super::io::Phase::PageTemp, 10_000);
        let id = live
            .submit(&key, "- already [[New]]\n", page.version, None)
            .unwrap();
        let version = live.answer(&key, id).answer.unwrap().version;
        let inputs = if newly_held {
            empty
        } else {
            live.host.inputs().unwrap()
        };
        let id = live.submit(&key, "- now [[Old]]\n", version, None).unwrap();
        live.answer(&key, id);
        let mut planned = op(
            vec![(
                "pages/a.md",
                Effect::Write(bytes("- renamed\n")),
                Expected::Bytes(bytes("- old\n")),
            )],
            Input::Refuse,
        );
        planned.inputs = Some(inputs);
        assert_eq!(
            live.host.apply(planned),
            Err(ApplyRefusal::Changed),
            "newly held: {newly_held}"
        );
        assert_eq!(live.disk("pages/a.md"), "- old\n");
        assert!(!live.root.join("pages/unindexed.md").exists());
        live.host.stop();
    }
}

#[test]
fn a_flush_that_saves_input_invalidates_its_selection_witness() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    let (key, page) = live.open("pages/a.md");
    live.faults(super::super::io::Phase::PageTemp, 10_000);
    let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
    live.answer(&key, id);
    let before = live.host.inputs().unwrap();
    live.host
        .driver
        .shared
        .with_state_for_tests(|state| state.progress.host.fs.faults.clear());
    live.host
        .flush_input(&[PageId::from("pages/a.md")])
        .unwrap();
    assert_ne!(live.host.inputs().unwrap(), before);
    assert!(!live
        .host
        .driver
        .shared
        .read(move |state| before.current(&state.progress.host))
        .unwrap());
    live.host.stop();
}

// The preflight bound is exercised by rename::a_rename_or_delete_under_a_held_path_lock_is_bounded,
// whose fixture now plans through PagePlan before applying the HostOp.

/// A save's directory sync precedes its index delivery. A merge cannot
/// report success while that delivery is failing (or waiting for the writer).
#[test]
fn a_merge_waits_for_its_deleted_sources_index_publication() {
    let source = "pages/MiXeD1.md";
    let destination = "pages/Next Space1.md";
    let live = Live::new(&[(source, "- source\n"), (destination, "- kept\n")]);
    live.store.watch.stop_background_for_test();
    let mut planned = op(
        vec![
            (source, Effect::Delete, Expected::Bytes(bytes("- source\n"))),
            (
                destination,
                Effect::Write(bytes("- kept\n- source\n")),
                Expected::Bytes(bytes("- kept\n")),
            ),
        ],
        Input::Refuse,
    );
    planned.moves = vec![(PageId::from(destination), PageId::from(source))];
    live.index_faults(10_000);
    std::thread::scope(|scope| {
        let host = &live.host;
        let (sent, received) = mpsc::channel();
        scope.spawn(move || sent.send(host.apply(planned)).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let synced = live
                .host
                .driver
                .shared
                .read(|state| {
                    state
                        .book
                        .retry
                        .get("pages/MiXeD1.md")
                        .is_some_and(|retry| {
                            retry.publication.bytes.is_none() && retry.publication.own.is_some()
                        })
                })
                .unwrap();
            if synced {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the merge never synced its files"
            );
            std::thread::yield_now();
        }
        assert!(!live.root.join(source).exists(), "the bytes already merged");
        assert!(
            live.store
                .whole_graph()
                .unwrap()
                .corpus(None)
                .pages
                .iter()
                .any(|p| p.name == "MiXeD1"),
            "the index delivery is still pending"
        );
        let early = received.recv_timeout(Duration::from_millis(300));
        // Release the fault before asserting, so a failed test can join its caller.
        live.index_faults(0);
        assert!(
            matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
            "merge returned before its index publication: {early:?}"
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            Ok(())
        );
    });
    let view = live.store.whole_graph().unwrap();
    let corpus = view.corpus(None);
    assert!(!corpus.pages.iter().any(|p| p.name == "MiXeD1"));
    assert_eq!(
        corpus
            .pages
            .iter()
            .find(|p| p.name == "Next Space1")
            .unwrap()
            .document
            .roots
            .len(),
        2
    );
    live.host.stop();
}

/// The ordinary writer-contention branch needs no index error: the driver
/// revisits the caller while the completed save's delivery is deferred.
#[test]
fn a_delete_waits_for_its_index_while_the_store_writer_is_busy() {
    let source = "pages/MiXeD1.md";
    let live = Live::new(&[(source, "- source\n")]);
    live.store.watch.stop_background_for_test();
    let (key, page) = live.open(source);
    live.until_published(&[(key, page.version, None)]);
    let (ready, at_delivery) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    *live.host.launch.kinds.lock().unwrap() = Box::new(move |_, _| {
        ready.send(()).unwrap();
        resumed.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    std::thread::scope(|scope| {
        let host = &live.host;
        let (sent, received) = mpsc::channel();
        scope.spawn(move || {
            sent.send(host.delete(None, &PageId::from(source), Some(b"- source\n")))
                .unwrap();
        });
        at_delivery.recv_timeout(Duration::from_secs(5)).unwrap();
        // The kinds sink runs after the disk witness, before taking the
        // store writer: hold it so that this exact publication is deferred.
        let writer = live.store.writer.lock().unwrap();
        resume.send(()).unwrap();
        assert!(live
            .host
            .driver
            .shared
            .read(|state| state.book.delivering)
            .unwrap());
        assert!(!live.root.join(source).exists());
        assert_eq!(live.indexed(source), Some(content_rev("- source\n")));
        let early = received.recv_timeout(Duration::from_millis(300));
        drop(writer);
        assert!(
            matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
            "delete returned before its deferred index publication: {early:?}"
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            PageOperation::Applied
        );
    });
    assert!(live
        .store
        .whole_graph()
        .unwrap()
        .corpus(None)
        .pages
        .is_empty());
    live.host.stop();
}

fn folded_op(old: &str) -> HostOp {
    let mut planned = op(
        vec![
            ("pages/old.md", Effect::Delete, Expected::Bytes(bytes(old))),
            (
                "pages/Old.md",
                Effect::Write(bytes("title:: Old\n- [[Old]]\n")),
                Expected::Absent,
            ),
            (
                "pages/Ref.md",
                Effect::Write(bytes("- [[Old]]\n")),
                Expected::Bytes(bytes("- [[old]]\n")),
            ),
        ],
        Input::Refuse,
    );
    planned.kinds = vec![EditKind::RenamePage];
    planned
        .moves
        .push((PageId::from("pages/Old.md"), PageId::from("pages/old.md")));
    planned
}

/// Enter the folding wrapper explicitly on Linux's sensitive tmpfs. This
/// proves its entry-first/content protocol; native alias discovery remains
/// covered by the folding-volume tests below on macOS/Windows.
#[test]
fn folded_wrapper_records_both_paths_with_and_without_an_edit() {
    for (from, to) in [
        ("pages/old.md", "pages/Old.md"),
        ("pages/é.md", "pages/É.md"),
    ] {
        for edited in [false, true] {
            let live = Live::new(&[(from, "- old\n")]);
            let src = PageId::from(from);
            let dst = PageId::from(to);
            let (key, spelling, ..) = live.host.try_identify(&src).unwrap();
            let sink = Arc::new(Mutex::new(BTreeSet::new()));
            let new = if edited { "- edited\n" } else { "- old\n" };
            let mut planned = op(
                vec![
                    (
                        src.as_str(),
                        Effect::Delete,
                        Expected::Bytes(bytes("- old\n")),
                    ),
                    (dst.as_str(), Effect::Write(bytes(new)), Expected::Absent),
                ],
                Input::Refuse,
            );
            planned.moves.push((dst.clone(), src.clone()));
            planned.publications = Some(sink.clone());
            planned.kinds = vec![EditKind::RenamePage];
            assert_eq!(
                live.host.apply_spellings(
                    planned,
                    vec![(dst.clone(), src.clone(), key.clone(), spelling)]
                ),
                Ok(())
            );
            assert_eq!(
                fs::read(live.root.join(dst.as_str())).unwrap(),
                new.as_bytes()
            );
            assert_eq!(*sink.lock().unwrap(), BTreeSet::from([src, dst]));
            assert!(
                live.kinds
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(page, kinds)| { page == &key && kinds.contains(&EditKind::RenamePage) }),
                "entry-only completion must still report its edit kinds"
            );
            live.host.stop();
        }
    }
}

#[test]
fn a_receipted_spelling_is_not_moved_by_a_stale_retry() {
    for receipted in ["pages/é.md", "pages/É.md"] {
        let live = Live::new(&[("pages/é.md", "- old\n")]);
        let src = PageId::from("pages/é.md");
        let dst = PageId::from("pages/É.md");
        let mut planned = op(
            vec![
                (
                    src.as_str(),
                    Effect::Delete,
                    Expected::Bytes(bytes("- old\n")),
                ),
                (
                    dst.as_str(),
                    Effect::Write(bytes("- old\n")),
                    Expected::Absent,
                ),
            ],
            Input::Refuse,
        );
        planned.moves.push((dst.clone(), src.clone()));
        planned.publications = Some(Arc::new(Mutex::new(BTreeSet::from([PageId::from(
            receipted,
        )]))));
        assert_eq!(
            live.host.apply(planned),
            Err(ApplyRefusal::SpellingIncomplete)
        );
        let listed: BTreeSet<_> = fs::read_dir(live.root.join("pages"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(listed.contains(&std::ffi::OsString::from("é.md")));
        assert!(!listed.contains(&std::ffi::OsString::from("É.md")));
        assert_eq!(fs::read(live.root.join(src.as_str())).unwrap(), b"- old\n");
        live.host.stop();
    }
}

/// Drive the entry-first postcondition on every filesystem: retain the old
/// key, move its real entry, then admit content through the new spelling.
#[test]
fn entry_first_retained_key_crash_worker() {
    let Ok(root) = std::env::var("TINE_S3_RETAINED_KEY_ROOT") else {
        return;
    };
    let app = std::env::var("TINE_S3_RETAINED_KEY_APP").unwrap();
    let phase = match std::env::var("TINE_S3_RETAINED_KEY_PHASE")
        .unwrap()
        .as_str()
    {
        "rename" => Phase::PageRename,
        "sync" => Phase::PageSync,
        _ => unreachable!(),
    };
    super::super::faults::abort_before(phase, 0);
    let store = Arc::new(Store::open(Path::new(&root), Default::default()).unwrap().0);
    let host = PageHost::start_for_tests(&store, Path::new(&app)).unwrap();
    let src = PageId::from("pages/old.md");
    let dst = PageId::from("pages/Old.md");
    let (key, spelling, ..) = host.try_identify(&src).unwrap();
    host.try_register(&key, &spelling).unwrap();
    let graph = store.graph.clone();
    let lock = graph.page_lock(&graph.root.join(dst.as_str()));
    host.driver
        .shared
        .call_writer(&store.writer, move |state| {
            fs::rename(graph.root.join(src.as_str()), graph.root.join(dst.as_str())).unwrap();
            let host = &mut state.progress.host;
            host.fs.spell(&key, dst.as_str());
            host.locks.insert(key, lock);
        })
        .unwrap();
    let result = host.apply(op(
        vec![(
            "pages/Old.md",
            Effect::Write(bytes("title:: Old\n- [[Old]]\n")),
            Expected::Bytes(bytes("title:: old\n- [[old]]\n")),
        )],
        Input::Refuse,
    ));
    panic!("content did not reach {phase:?}: {result:?}");
}

#[cfg(unix)]
#[test]
fn entry_first_retained_key_draft_recovers_through_disk_alias() {
    for phase in ["rename", "sync"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("graph");
        let app = dir.path().join("app");
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::write(root.join("pages/old.md"), "title:: old\n- [[old]]\n").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "page_host::binding::tests::apply_tests::entry_first_retained_key_crash_worker",
                "--nocapture",
            ])
            .env("TINE_S3_RETAINED_KEY_ROOT", &root)
            .env("TINE_S3_RETAINED_KEY_APP", &app)
            .env("TINE_S3_RETAINED_KEY_PHASE", phase)
            .output()
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(6), "{output:?}");
        let records = super::super::drafts::scan(
            fs::read_dir(app.join("drafts-v2/test-graph"))
                .unwrap()
                .filter_map(|entry| {
                    let path = entry.unwrap().path();
                    Some((
                        path.file_name()?.to_str()?.to_owned(),
                        fs::read(&path).ok()?,
                    ))
                })
                .collect(),
        );
        assert_eq!(
            records.logical["pages/old.md"].bytes,
            Some(bytes("title:: Old\n- [[Old]]\n"))
        );
        // As in page_title_identity's alias fixture, a same-directory link
        // supplies Linux with the folding volume's old-path lookup. A native
        // folding volume already provides it, without a link.
        let alias = !root.join("pages/old.md").exists();
        if alias {
            std::os::unix::fs::symlink("Old.md", root.join("pages/old.md")).unwrap();
        }
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        assert_eq!(
            store.disk_spelling(&PageId::from("pages/old.md")),
            Some(PageId::from("pages/Old.md"))
        );
        assert!(store
            .graph
            .ensure_write_target(&store.graph.root.join("pages/old.md"))
            .is_err());
        let host = PageHost::start_for_tests(&store, &app).unwrap();
        assert_eq!(host.spelling("pages/old.md"), PageId::from("pages/Old.md"));
        if alias {
            // All recovery I/O must now use the resolved entry, even when
            // the old-path lookup disappears immediately after launch.
            fs::remove_file(root.join("pages/old.md")).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(root.join("pages/Old.md")).unwrap() != "title:: Old\n- [[Old]]\n" {
            assert!(
                Instant::now() < deadline,
                "{phase}: {:?}",
                host.draft_status()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let listed: Vec<_> = fs::read_dir(root.join("pages"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            // The killed temp is crash debris, never a live graph page.
            .filter(|name| {
                let name = name.to_string_lossy();
                !(name.starts_with('.') && name.ends_with(".tmp"))
            })
            .collect();
        assert_eq!(listed, ["Old.md"]);
        host.stop();
        store.close();
    }
}

fn folding(live: &Live) -> bool {
    let folding = super::rename::folds_case(&live.root);
    if !folding {
        eprintln!("S3 skipped: a folding filesystem is required; Linux green proves nothing here");
    }
    folding
}

#[test]
fn a_folded_move_operation_keeps_its_key_and_rewrites_referrers() {
    let live = Live::new(&[
        ("pages/old.md", "title:: old\n- [[old]]\n"),
        ("pages/Ref.md", "- [[old]]\n"),
    ]);
    if !folding(&live) {
        live.host.stop();
        return;
    }
    let (key, _) = live.open("pages/old.md");
    assert_eq!(
        live.host.apply(folded_op("title:: old\n- [[old]]\n")),
        Ok(())
    );
    let listed: Vec<_> = fs::read_dir(live.root.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(listed.contains(&"Old.md".into()) && !listed.contains(&"old.md".into()));
    assert_eq!(live.disk("pages/Old.md"), "title:: Old\n- [[Old]]\n");
    assert_eq!(live.disk("pages/Ref.md"), "- [[Old]]\n");
    assert_eq!(trashed(&live, "title:: old\n- [[old]]\n"), 1);
    assert_eq!(
        live.host
            .try_identify(&PageId::from("pages/Old.md"))
            .unwrap()
            .0,
        key
    );
    live.until_indexed("pages/Old.md", "title:: Old\n- [[Old]]\n");
    let (opened_key, page) = live.open("pages/Old.md");
    assert_eq!(opened_key, key);
    live.submit(&key, "title:: Old\n- edited\n", page.version, None)
        .unwrap();
    live.until_disk("pages/Old.md", "title:: Old\n- edited\n");
    live.host.stop();
}

#[test]
fn a_folded_move_refuses_unsaved_input_before_any_write() {
    let live = Live::new(&[
        ("pages/old.md", "title:: old\n- [[old]]\n"),
        ("pages/Ref.md", "- [[old]]\n"),
    ]);
    if !folding(&live) {
        live.host.stop();
        return;
    }
    let (key, _) = live.open("pages/Ref.md");
    live.pause(true);
    live.host.driver.shared.with_state_for_tests(|state| {
        state.progress.with_host(|host| {
            host.version += 1;
            let page = host.pages.get_mut(&key).unwrap();
            page.buf = Some(bytes("- [[old]] typed\n"));
            page.typed = true;
            page.version = host.version;
            host.mark(&key);
        });
    });
    assert_eq!(
        live.host.apply(folded_op("title:: old\n- [[old]]\n")),
        Err(ApplyRefusal::Unsaved(PageId::from("pages/Ref.md")))
    );
    assert_eq!(live.disk("pages/old.md"), "title:: old\n- [[old]]\n");
    assert_eq!(live.disk("pages/Ref.md"), "- [[old]]\n");
    live.host.stop();
}

#[test]
fn a_folded_entry_move_publishes_an_unheld_page_before_reply() {
    let live = Live::watching(
        &[("pages/old.md", "title:: Old\n- [[Old]]\n")],
        crate::WatchMode::Poll,
    );
    if !folding(&live) {
        live.host.stop();
        return;
    }
    let src = PageId::from("pages/old.md");
    let dst = PageId::from("pages/Old.md");
    let (key, spelling, ..) = live.host.try_identify(&src).unwrap();
    live.host.try_register(&key, &spelling).unwrap();
    assert!(live.store.graph.disk_sourced(&live.root.join(src.as_str())));
    assert_eq!(
        live.host
            .move_spelling(
                src.clone(),
                dst.clone(),
                key,
                bytes("title:: Old\n- [[Old]]\n"),
                None,
                None
            )
            .map(|_| ()),
        Ok(())
    );
    assert!(live
        .store
        .snapshot_knows(&live.store.graph.root.join(dst.as_str())));
    assert!(!live
        .store
        .snapshot_knows(&live.store.graph.root.join(src.as_str())));
    live.host.stop();
}

#[test]
fn a_failed_folded_entry_move_keeps_old_text_and_can_be_retried() {
    let live = Live::new(&[
        ("pages/old.md", "title:: old\n- [[old]]\n"),
        ("pages/Ref.md", "- [[old]]\n"),
    ]);
    if !folding(&live) {
        live.host.stop();
        return;
    }
    live.store
        .inject_fault(crate::FaultPoint::CaseMoveAliasRefusal);
    live.store
        .inject_fault(crate::FaultPoint::CaseMoveAliasNoop);
    // ... and the move through the temporary entry fails too (GS1).
    live.store
        .inject_fault(crate::FaultPoint::CaseMoveTemporaryIo);
    assert_eq!(
        live.host.apply(folded_op("title:: old\n- [[old]]\n")),
        Err(ApplyRefusal::SpellingIncomplete)
    );
    assert_eq!(live.disk("pages/old.md"), "title:: old\n- [[old]]\n");
    assert_eq!(live.disk("pages/Ref.md"), "- [[old]]\n");
    // A provider accepted the syscall but still lists old.md. The driver
    // must keep the key, path lock and index at that observed spelling.
    let (_, spelling, ..) = live
        .host
        .try_identify(&PageId::from("pages/old.md"))
        .unwrap();
    assert_eq!(spelling.as_str(), "pages/old.md");
    let retry = folded_op("title:: old\n- [[old]]\n");
    assert_eq!(live.host.apply(retry), Ok(()));
    let listed: Vec<_> = fs::read_dir(live.root.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(listed.contains(&"Old.md".into()) && !listed.contains(&"old.md".into()));
    live.host.stop();
}

#[cfg(unix)]
#[test]
fn a_folded_entry_sync_error_keeps_the_key_at_the_new_spelling() {
    let live = Live::new(&[
        ("pages/old.md", "title:: old\n- [[old]]\n"),
        ("pages/Ref.md", "- [[old]]\n"),
    ]);
    if !folding(&live) {
        live.host.stop();
        return;
    }
    let (key, _) = live.open("pages/old.md");
    // This transaction fault affects only the entry command's existing
    // directory-sync recipe, before the ordinary content HostOp starts.
    live.store.inject_fault(crate::FaultPoint::DirectorySyncIo);
    assert_eq!(
        live.host.apply(folded_op("title:: old\n- [[old]]\n")),
        Err(ApplyRefusal::SpellingIncomplete)
    );
    let (same, spelling, ..) = live
        .host
        .try_identify(&PageId::from("pages/Old.md"))
        .unwrap();
    assert_eq!(same, key);
    assert_eq!(spelling.as_str(), "pages/Old.md");
    live.until_indexed("pages/Old.md", "title:: old\n- [[old]]\n");
    assert_eq!(live.disk("pages/Ref.md"), "- [[old]]\n");
    let (_, page) = live.open("pages/Old.md");
    live.submit(&key, "title:: Old\n- after error\n", page.version, None)
        .unwrap();
    live.until_disk("pages/Old.md", "title:: Old\n- after error\n");
    live.host.stop();
}

/// The files under the graph's trash holding `body`.
fn trashed(live: &Live, body: &str) -> usize {
    fn walk(dir: &std::path::Path, body: &str) -> usize {
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        entries
            .map(|entry| entry.unwrap().path())
            .map(|path| match path.is_dir() {
                true => walk(&path, body),
                false => usize::from(fs::read(&path).unwrap() == body.as_bytes()),
            })
            .sum()
    }
    walk(&crate::model::trash_root(&live.root), body)
}

#[test]
fn a_write_and_a_deletion_apply_as_one_operation() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b\n")]);
    let applied = live.host.apply(op(
        vec![
            (
                "pages/a.md",
                Effect::Write(bytes("- one\n- two\n")),
                Expected::Bytes(bytes("- one\n")),
            ),
            (
                "pages/b.md",
                Effect::Delete,
                Expected::Bytes(bytes("- b\n")),
            ),
        ],
        Input::Refuse,
    ));
    assert_eq!(applied, Ok(()));
    assert_eq!(live.disk("pages/a.md"), "- one\n- two\n");
    assert!(!live.root.join("pages/b.md").exists());
    assert_eq!(
        trashed(&live, "- b\n"),
        1,
        "the deleted bytes stay recoverable"
    );
    live.until_indexed("pages/a.md", "- one\n- two\n");
    live.host.stop();
}

/// The operation guard: a page that no longer holds the planned bytes
/// refuses the whole operation, and nothing changes (the feature replans).
#[test]
fn a_page_changed_since_the_plan_refuses_the_operation() {
    let live = Live::new(&[("pages/a.md", "- one\n"), ("pages/b.md", "- b changed\n")]);
    let applied = live.host.apply(op(
        vec![
            (
                "pages/a.md",
                Effect::Write(bytes("- two\n")),
                Expected::Bytes(bytes("- one\n")),
            ),
            (
                "pages/b.md",
                Effect::Delete,
                Expected::Bytes(bytes("- b\n")),
            ),
        ],
        Input::Refuse,
    ));
    assert_eq!(applied, Err(ApplyRefusal::Changed));
    assert_eq!(live.disk("pages/a.md"), "- one\n");
    assert_eq!(live.disk("pages/b.md"), "- b changed\n");
    // A page planned as absent that has a file is changed too.
    let created = live.host.apply(op(
        vec![(
            "pages/b.md",
            Effect::Write(bytes("- new\n")),
            Expected::Absent,
        )],
        Input::Refuse,
    ));
    assert_eq!(created, Err(ApplyRefusal::Changed));
    assert_eq!(live.disk("pages/b.md"), "- b changed\n");
    live.host.stop();
}

/// D4: input the host has not saved refuses a `Refuse` writer, by the
/// page's spelling; a `Flush` writer waits for its save, then applies over
/// the saved bytes.
#[test]
fn unsaved_input_refuses_or_is_saved_first() {
    let live = Live::new(&[("pages/a.md", "- one\n")]);
    let (key, page) = live.open("pages/a.md");
    live.pause(true);
    let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
    let planned = |expected: &str| {
        op(
            vec![(
                "pages/a.md",
                Effect::Write(bytes("- typed\n- planned\n")),
                Expected::Bytes(bytes(expected)),
            )],
            Input::Flush,
        )
    };
    let mut refuse = planned("- one\n");
    refuse.policy = Input::Refuse;
    assert_eq!(
        live.host.apply(refuse),
        Err(ApplyRefusal::Unsaved(PageId::from("pages/a.md")))
    );
    assert_eq!(live.disk("pages/a.md"), "- one\n");
    std::thread::scope(|scope| {
        let host = &live.host;
        let flushed = scope.spawn(move || host.apply(planned("- typed\n")));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!flushed.is_finished(), "the flush waits for the save");
        live.pause(false);
        assert_eq!(flushed.join().unwrap(), Ok(()));
    });
    live.answer(&key, id);
    assert_eq!(live.disk("pages/a.md"), "- typed\n- planned\n");
    live.host.stop();
}

/// REVIEW P2: a feature plans from the store, whose reads answer saved
/// bytes only, so a page that exists only as typed input reads as absent.
/// `flush_input` saves it first (waiting for the save), after which the
/// store reads the typed bytes a plan expects; a clean page returns at once.
#[test]
fn flush_input_saves_a_page_that_exists_only_as_typed_input() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let (key, page) = live.open("pages/new.md");
    live.pause(true);
    let id = live.submit(&key, "- typed\n", page.version, None).unwrap();
    let new = PageId::from("pages/new.md");
    assert!(
        matches!(
            live.store.read(&new.file(), None),
            Err(crate::StoreError::NotFound)
        ),
        "the plan's read sees no file"
    );
    std::thread::scope(|scope| {
        let host = &live.host;
        let new = &new;
        let flushed = scope.spawn(move || host.flush_input(std::slice::from_ref(new)));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!flushed.is_finished(), "the flush waits for the save");
        live.pause(false);
        assert_eq!(flushed.join().unwrap(), Ok(()));
    });
    live.answer(&key, id);
    assert_eq!(live.disk("pages/new.md"), "- typed\n");
    assert_eq!(
        &*live.store.read(&new.file(), None).unwrap().0,
        b"- typed\n"
    );
    assert_eq!(live.host.flush_input(&[PageId::from("pages/a.md")]), Ok(()));
    assert_eq!(
        live.host.flush_input(&[PageId::from("../outside.md")]),
        Err(ApplyRefusal::Refused)
    );
    live.host.stop();
}

/// REVIEW P2 round 2, finding 1: an imported graph whose `pages/link` is a
/// symlink to a folder outside it. A write under it is not a page of this
/// graph: refused, and nothing is created outside the graph.
#[cfg(unix)]
#[test]
fn a_write_through_a_symlinked_folder_is_refused() {
    let live = Live::new(&[("pages/a.md", "- a\n")]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), live.root.join("pages/link")).unwrap();
    let applied = live.host.apply(op(
        vec![(
            "pages/link/new.md",
            Effect::Write(bytes("- escaped\n")),
            Expected::Absent,
        )],
        Input::Refuse,
    ));
    assert_eq!(applied, Err(ApplyRefusal::Refused));
    assert_eq!(
        live.host.flush_input(&[PageId::from("pages/link/new.md")]),
        Err(ApplyRefusal::Refused)
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    live.host.stop();
}

/// A move: the receiver's new bytes took the source's, which the operation
/// deletes; a source planned from absent bytes, a move whose pages are not
/// the operation's, and a page named twice are not operations.
#[test]
fn a_move_applies_and_a_malformed_operation_is_refused() {
    let live = Live::new(&[("pages/a.md", "- a\n"), ("pages/c.md", "- c\n")]);
    let moving = |source: Expected, moved: Option<(&str, &str)>| HostOp {
        inputs: None,
        creation_checks: Vec::new(),
        moves: moved
            .map(|(dst, src)| (PageId::from(dst), PageId::from(src)))
            .into_iter()
            .collect(),
        ..op(
            vec![
                (
                    "pages/b.md",
                    Effect::Write(bytes("- a\n")),
                    Expected::Absent,
                ),
                ("pages/a.md", Effect::Delete, source),
            ],
            Input::Refuse,
        )
    };
    let refused = Err(ApplyRefusal::Refused);
    assert_eq!(
        live.host
            .apply(moving(Expected::Absent, Some(("pages/b.md", "pages/a.md")))),
        refused
    );
    assert_eq!(
        live.host.apply(moving(
            Expected::Bytes(bytes("- a\n")),
            Some(("pages/b.md", "pages/c.md"))
        )),
        refused
    );
    let twice = op(
        vec![
            (
                "pages/c.md",
                Effect::Delete,
                Expected::Bytes(bytes("- c\n")),
            ),
            (
                "pages/c.md",
                Effect::Delete,
                Expected::Bytes(bytes("- c\n")),
            ),
        ],
        Input::Refuse,
    );
    assert_eq!(live.host.apply(twice), refused);
    assert_eq!(live.host.apply(op(vec![], Input::Refuse)), refused);
    let mut unkinded = op(
        vec![(
            "pages/c.md",
            Effect::Write(bytes("- c2\n")),
            Expected::Bytes(bytes("- c\n")),
        )],
        Input::Refuse,
    );
    unkinded.kinds.clear();
    assert_eq!(
        live.host.apply(unkinded),
        refused,
        "OG-RULES Rule 8: no kind"
    );
    assert_eq!(live.disk("pages/a.md"), "- a\n");
    assert_eq!(live.disk("pages/c.md"), "- c\n");
    let applied = live.host.apply(moving(
        Expected::Bytes(bytes("- a\n")),
        Some(("pages/b.md", "pages/a.md")),
    ));
    assert_eq!(applied, Ok(()));
    assert_eq!(live.disk("pages/b.md"), "- a\n");
    assert!(!live.root.join("pages/a.md").exists());
    live.host.stop();
}

/// REG-RENAME-READ-AFTER-WRITE: keep the source open to force the receiver's
/// Close/cleanup to finish first. All ownership transitions run through the
/// real host and bridge; the test never releases a watcher hold itself.
#[test]
fn a_completed_move_stays_visible_when_the_receiver_closes_first() {
    for watch in [crate::WatchMode::Notify, crate::WatchMode::Poll] {
        for (source, target) in [
            ("a", "Renamed"),
            ("ns___child", "Moved___child"),
            ("a", "A"),
            ("ns___child", "NS___child"),
        ] {
            let from = format!("pages/{source}.md");
            let to = format!("pages/{target}.md");
            let live = Live::watching(&[(&from, "- body\n")], watch);
            let src = live.root.join(&from);
            let dst = live.root.join(&to);
            if dst.exists() {
                // Folding volumes use one key and the spelling wrapper;
                // their native spelling tests cover that separate path.
                live.host.stop();
                continue;
            }
            let (source_key, _) = live.open(&from);
            let receiver_key = to.clone();
            let mut moving = op(
                vec![
                    (&to, Effect::Write(bytes("- body\n")), Expected::Absent),
                    (&from, Effect::Delete, Expected::Bytes(bytes("- body\n"))),
                ],
                Input::Refuse,
            );
            moving.separate_case_entries = true;
            moving
                .moves
                .push((PageId::from(to.clone()), PageId::from(from.clone())));
            moving.creation_checks.push((
                PageId::from(to.clone()),
                None,
                Some(crate::FileId::from(from.clone())),
            ));
            assert_eq!(live.host.apply(moving), Ok(()), "{watch:?}: {from} -> {to}");
            let target_name = live.store.graph.entry_for_path(&dst).unwrap().name;
            // Wait for real cleanup, then explicitly deliver the receiver's
            // Close. The source's subscription fixes the adverse schedule.
            let drained = || {
                live.host
                    .driver
                    .shared
                    .read(|state| {
                        let host = &state.progress.host;
                        host.job.is_none()
                            && host.worker.is_none()
                            && host.drafts.is_empty()
                            && host.queue.is_empty()
                            && host.applying.is_none()
                    })
                    .unwrap()
            };
            let wait = || {
                let deadline = Instant::now() + Duration::from_secs(20);
                while !drained() {
                    assert!(Instant::now() < deadline, "{from} -> {to}: cleanup stalled");
                    std::thread::sleep(Duration::from_millis(5));
                }
            };
            wait();
            live.close(&receiver_key);
            wait();
            // A later driver step can run only after the Close's delivery
            // has reached the bridge, including a deferred writer delivery.
            let polls = live.host.driver.shared.read(|state| state.polls).unwrap();
            live.host.driver.shared.call(|_| ()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while live.host.driver.shared.read(|state| state.polls).unwrap() == polls {
                assert!(Instant::now() < deadline, "Close delivery did not finish");
                std::thread::sleep(Duration::from_millis(5));
            }
            let visible = || {
                matches!(live.store.whole_graph().unwrap().resolve(&target_name, false),
                crate::Resolved::Existing { id, .. } if id.as_str() == to)
            };
            assert!(
                visible(),
                "{watch:?}: {from} -> {to}: completed receiver hidden"
            );
            assert!(dst.exists() && !src.exists());
            {
                let _writer = live.store.writer.lock().unwrap();
                // A late removal notification is reconciled against the
                // completed operation, never allowed to erase its receiver.
                live.store
                    .watch
                    .reconcile_raced(&std::collections::HashSet::from([src]));
            }
            assert!(visible(), "a late source event hid the receiver");
            // A retained receiver must still be an observed page, rather
            // than an orphan index row: real external edits and deletion
            // reach the host through either watcher mode while source stays open.
            fs::write(&dst, "- external edit\n").unwrap();
            live.until_indexed(&to, "- external edit\n");
            fs::remove_file(&dst).unwrap();
            assert!(live
                .store
                .watcher_drained_for_tests(Duration::from_secs(20)));
            let deadline = Instant::now() + Duration::from_secs(20);
            while visible() {
                assert!(Instant::now() < deadline, "external deletion was hidden");
                std::thread::sleep(Duration::from_millis(5));
            }
            fs::write(&dst, "- restored\n").unwrap();
            live.until_indexed(&to, "- restored\n");
            let deadline = Instant::now() + Duration::from_secs(20);
            while !visible() {
                assert!(
                    Instant::now() < deadline,
                    "{from} -> {to}: external restoration hidden"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            {
                // Eviction collection alone is not the handoff: delay its
                // bridge delivery with the real writer, keeping the new
                // name resolvable until that delivery can finish.
                let _writer = live.store.writer.lock().unwrap();
                live.close(&source_key);
                wait();
                assert!(visible(), "a deferred source handoff hid the receiver");
            }
            assert!(
                live.host.quiesce_for_tests(Duration::from_secs(20)),
                "paired keys leaked"
            );
            assert!(visible(), "source handoff hid the receiver");
            fs::remove_file(&dst).unwrap();
            assert!(live
                .store
                .watcher_drained_for_tests(Duration::from_secs(20)));
            assert!(!visible(), "external deletion after handoff was hidden");
            live.host.stop();
        }
    }
}

/// S2: one operation carries several moves (a namespace rename); a page in
/// two moves is not an operation.
#[test]
fn several_moves_apply_as_one_operation_and_a_shared_page_is_refused() {
    let live = Live::new(&[("pages/a.md", "- a\n"), ("pages/c.md", "- c\n")]);
    let pages = || {
        vec![
            (
                "pages/b.md",
                Effect::Write(bytes("- a\n")),
                Expected::Absent,
            ),
            (
                "pages/a.md",
                Effect::Delete,
                Expected::Bytes(bytes("- a\n")),
            ),
            (
                "pages/d.md",
                Effect::Write(bytes("- c\n")),
                Expected::Absent,
            ),
            (
                "pages/c.md",
                Effect::Delete,
                Expected::Bytes(bytes("- c\n")),
            ),
        ]
    };
    let moving = |moves: &[(&str, &str)]| HostOp {
        inputs: None,
        moves: moves
            .iter()
            .map(|(dst, src)| (PageId::from(*dst), PageId::from(*src)))
            .collect(),
        ..op(pages(), Input::Refuse)
    };
    assert_eq!(
        live.host.apply(moving(&[
            ("pages/b.md", "pages/a.md"),
            ("pages/d.md", "pages/a.md")
        ])),
        Err(ApplyRefusal::Refused),
        "a source in two moves"
    );
    assert_eq!(live.disk("pages/a.md"), "- a\n");
    assert_eq!(live.disk("pages/c.md"), "- c\n");
    assert_eq!(
        live.host.apply(moving(&[
            ("pages/b.md", "pages/a.md"),
            ("pages/d.md", "pages/c.md")
        ])),
        Ok(())
    );
    assert_eq!(live.disk("pages/b.md"), "- a\n");
    assert_eq!(live.disk("pages/d.md"), "- c\n");
    assert!(!live.root.join("pages/a.md").exists());
    assert!(!live.root.join("pages/c.md").exists());
    live.host.stop();
}
