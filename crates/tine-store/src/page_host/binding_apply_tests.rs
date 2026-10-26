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
            .move_spelling(src.clone(), dst, key.clone(), bytes("- old\n"), None),
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

fn op(pages: Vec<(&str, Effect, Expected)>, policy: Input) -> HostOp {
    HostOp {
        creation_checks: Vec::new(),
        pages: pages
            .into_iter()
            .map(|(page, effect, expected)| (PageId::from(page), effect, expected))
            .collect(),
        kinds: vec![EditKind::InsertBlocks],
        policy,
        moves: Vec::new(),
        view: None,
    }
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
