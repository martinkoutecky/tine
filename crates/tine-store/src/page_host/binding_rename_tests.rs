//! The single-page host rename and the alias respell (STEP3 §2, §7, F10,
//! Q4), against a real Store.
use super::super::retained::RenameRefusal;
use super::*;
use crate::RenameMap;

fn map(from: &str, to: &str) -> RenameMap {
    RenameMap(vec![(from.into(), to.into())])
}

/// Whether the test volume treats `A` and `a` as one entry.
fn folds_case(root: &std::path::Path) -> bool {
    let probe = root.join("Fold-Probe");
    fs::write(&probe, b"").unwrap();
    let folds = root.join("fold-probe").exists();
    fs::remove_file(probe).unwrap();
    folds
}

/// F10: the rename policy is the rename transaction's: the moving page's
/// title is rebound, a referrer is rewritten, a referrer with VCS markers
/// is left byte-identical and reported, and the source goes to trash.
#[test]
fn a_host_rename_rewrites_referrers_and_leaves_marked_ones() {
    let marked = "- [[Old]]\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n";
    let live = Live::new(&[
        ("pages/Old.md", "title:: Old\n\n- [[Old]] self\n"),
        ("pages/r.md", "- see [[Old]]\n"),
        ("pages/m.md", marked),
    ]);
    let renamed = live.host.rename(
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &[PageId::from("pages/r.md"), PageId::from("pages/m.md")],
        &map("Old", "New"),
    );
    assert_eq!(
        renamed,
        Ok((
            vec![PageId::from("pages/r.md")],
            vec![PageId::from("pages/m.md")]
        ))
    );
    assert_eq!(live.disk("pages/New.md"), "title:: New\n\n- [[New]] self\n");
    assert!(!live.root.join("pages/Old.md").exists());
    assert_eq!(live.disk("pages/r.md"), "- see [[New]]\n");
    assert_eq!(live.disk("pages/m.md"), marked);
    live.host.stop();
}

/// F10: a changed Org referrer that does not round-trip refuses the whole
/// rename before any write, naming it; a referrer with unsaved input does
/// too.
#[test]
fn a_host_rename_refuses_an_unwritable_or_unsaved_referrer() {
    let live = Live::new(&[
        ("pages/old.md", "- old\n"),
        ("pages/o.org", "* see [[old]]\n*** skipped level\n"),
        ("pages/r.md", "- see [[old]]\n"),
    ]);
    let source = PageId::from("pages/old.md");
    let target = PageId::from("pages/new.md");
    assert_eq!(
        live.host.rename(
            &source,
            &target,
            &[PageId::from("pages/o.org")],
            &map("old", "new")
        ),
        Err(RenameRefusal::Unwritable(PageId::from("pages/o.org")))
    );
    let (key, page) = live.open("pages/r.md");
    live.faults(super::super::io::Phase::PageTemp, 100);
    let id = live
        .submit(&key, "- typed [[old]]\n", page.version, None)
        .unwrap();
    live.answer(&key, id);
    assert_eq!(
        live.host.rename(
            &source,
            &target,
            &[PageId::from("pages/r.md")],
            &map("old", "new")
        ),
        Err(RenameRefusal::Unsaved(PageId::from("pages/r.md")))
    );
    assert_eq!(live.disk("pages/old.md"), "- old\n");
    assert!(!live.root.join("pages/new.md").exists());
    live.host.stop();
}

/// Q4 (REVIEW-3), the counterexample trace: on a case-sensitive volume
/// `Foo.md` exists, `foo.md` does not, and a durable draft of `Foo.md` is
/// pending retirement. The case-only rename is a two-key host rename,
/// never a spelling move of one key: it waits out the old-spelling draft,
/// and its own records name each spelling's key. A crash then leaves an
/// old-spelling record durable (a deletion draft and custody marker whose
/// trash move failed, or a custody marker whose retirement failed after
/// `Foo.md` was gone); the relaunched host never brings `Foo.md` back.
#[test]
fn q4_a_case_only_rename_to_an_absent_entry_recovers_after_a_crash() {
    use super::super::io::Phase;
    for (fault, renamed) in [
        (
            Phase::TrashMove,
            Err(RenameRefusal::Unwritten(PageId::from("pages/Foo.md"))),
        ),
        (Phase::CustodyRetire, Ok((vec![], vec![]))),
    ] {
        let live = Live::new(&[("pages/Foo.md", "- one\n")]);
        if folds_case(&live.root) {
            // The alias branch: see `an_alias_spelling_is_refused_for_a_respell`.
            live.host.stop();
            return;
        }
        let (key, page) = live.open("pages/Foo.md");
        // The first save fails, so the edit is drafted; the save then
        // lands but the draft's retirement fails for a while.
        live.faults(Phase::DraftUnlink, 3);
        live.faults(Phase::PageTemp, 1);
        let id = live.submit(&key, "- two\n", page.version, None).unwrap();
        live.answer(&key, id);
        live.until_disk(&key, "- two\n");
        assert!(
            !live.drafts().is_empty(),
            "the old-spelling draft is durable"
        );
        live.faults(fault, 1000);
        assert_eq!(
            live.host.rename(
                &PageId::from("pages/Foo.md"),
                &PageId::from("pages/foo.md"),
                &[],
                &map("Foo", "foo")
            ),
            renamed,
            "{fault:?}"
        );
        assert_eq!(live.disk("pages/foo.md"), "- two\n");
        assert_eq!(
            live.root.join("pages/Foo.md").exists(),
            fault == Phase::TrashMove
        );
        // A crash: the driver stops between steps, with every draft and
        // marker it had; a new process binds the graph.
        let app = live.app();
        let Live {
            _dir,
            root,
            store,
            host,
            mail,
            ..
        } = live;
        drop(host);
        let host = PageHost::start(&store, &app, "test-graph", 7, |_| {}).unwrap();
        let live = Live {
            _dir,
            root,
            store,
            host,
            mail,
            id: std::cell::Cell::new(100),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while live.root.join("pages/Foo.md").exists() {
            assert!(
                Instant::now() < deadline,
                "{fault:?}: recovery kept the old spelling"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        settle();
        assert_eq!(live.disk("pages/foo.md"), "- two\n", "{fault:?}");
        assert!(
            !live.root.join("pages/Foo.md").exists(),
            "Q4 {fault:?}: recovery brought the old spelling back"
        );
        live.host.stop();
    }
}

/// Q4: a target that is another spelling of the source's own entry is
/// not a two-key rename; the caller moves it under a reservation and
/// respells the key, whose page, path lock and held index follow. (A
/// symlink stands in for a folding volume's alias here: the store's
/// case-alias resolution canonicalizes both the same way.)
#[test]
fn an_alias_spelling_is_refused_for_a_respell_that_keeps_the_page() {
    let live = Live::new(&[("pages/Foo.md", "- one\n")]);
    let alias = live.root.join("pages/foo.md");
    if !folds_case(&live.root) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(live.root.join("pages/Foo.md"), &alias).unwrap();
        #[cfg(not(unix))]
        {
            live.host.stop();
            return;
        }
    }
    let (key, page) = live.open("pages/Foo.md");
    assert_eq!(
        live.host.rename(
            &PageId::from("pages/Foo.md"),
            &PageId::from("pages/foo.md"),
            &[],
            &map("Foo", "foo")
        ),
        Err(RenameRefusal::Alias)
    );
    let reservation = live
        .host
        .reserve(|| vec![PageId::from("pages/Foo.md")], Input::Refuse)
        .unwrap();
    // The retained writer's move of the entry to the other spelling.
    if alias.is_symlink() {
        fs::remove_file(&alias).unwrap();
    }
    fs::rename(live.root.join("pages/Foo.md"), &alias).unwrap();
    live.host
        .respell(&PageId::from("pages/Foo.md"), &PageId::from("pages/foo.md"));
    assert!(live.held("pages/foo.md") && !live.held("pages/Foo.md"));
    live.host.release(reservation);
    let id = live.id();
    let generation = live.host.generation();
    let dto = live.dto("pages/foo.md", "- typed\n");
    // The release's observation reads the same bytes at the new spelling,
    // so it changes nothing and mails nothing; the page kept its version.
    live.host
        .submit(
            generation,
            id,
            &key,
            &dto,
            page.version,
            None,
            &[EditKind::SaveBlock],
        )
        .unwrap();
    live.until_disk("pages/foo.md", "- typed\n");
    live.host.stop();
}
