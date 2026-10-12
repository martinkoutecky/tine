//! A feature's planned write as one host operation (host-only-writer P2,
//! DESIGN §3.1), against a real Store.
use super::super::apply::{ApplyRefusal, Effect, Expected, HostOp};
use super::*;

fn bytes(text: &str) -> Arc<[u8]> {
    Arc::from(text.as_bytes())
}

fn op(pages: Vec<(&str, Effect, Expected)>, policy: Input) -> HostOp {
    HostOp {
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

/// A move: the receiver's new bytes took the source's, which the operation
/// deletes; a source planned from absent bytes, a move whose pages are not
/// the operation's, and a page named twice are not operations.
#[test]
fn a_move_applies_and_a_malformed_operation_is_refused() {
    let live = Live::new(&[("pages/a.md", "- a\n"), ("pages/c.md", "- c\n")]);
    let moving = |source: Expected, moved: Option<(&str, &str)>| HostOp {
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
