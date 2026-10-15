//! A feature's planned write as one host operation (host-only-writer P2,
//! DESIGN §3.1), against a real Store.
use super::super::apply::{ApplyRefusal, Effect, Expected, HostOp};
use super::*;

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
        moved: None,
    }
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
        moved: moved.map(|(dst, src)| (PageId::from(dst), PageId::from(src))),
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
