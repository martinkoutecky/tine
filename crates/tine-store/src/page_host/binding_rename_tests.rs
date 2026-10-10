//! The single-page host rename and the alias respell (STEP3 §2, §7, F10,
//! Q4), against a real Store.
use super::super::retained::RenameRefusal;
use super::*;
use crate::RenameMap;

fn map(from: &str, to: &str) -> RenameMap {
    RenameMap(vec![(from.into(), to.into())])
}

/// A host rename as `pages.rs` drives it: planned in a fresh view, and
/// replanned in a new one when another publication made it stale (B2).
fn rename(
    live: &Live,
    source: &PageId,
    target: &PageId,
    referrers: &[PageId],
    map: &RenameMap,
) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
    for _ in 0..20 {
        let view = live.store.whole_graph().unwrap();
        match live.host.rename(source, target, referrers, map, &view) {
            Err(RenameRefusal::Refused) => continue,
            done => return done,
        }
    }
    Err(RenameRefusal::Refused)
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
    let renamed = rename(
        &live,
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

/// OG-RULES Rule 8 (A-K1): a host operation declares its edit kind. Every
/// page a host rename writes publishes with `RenamePage`, and a host
/// delete's page with `DeletePage`, through the per-page kinds a submit
/// uses; no publication of either goes without a kind.
#[test]
fn host_rename_and_delete_publish_their_edit_kinds() {
    let live = Live::new(&[
        ("pages/Old.md", "- old\n"),
        ("pages/r.md", "- see [[Old]]\n"),
        ("pages/gone.md", "- gone\n"),
    ]);
    let renamed = rename(
        &live,
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &[PageId::from("pages/r.md")],
        &map("Old", "New"),
    );
    assert!(renamed.is_ok(), "{renamed:?}");
    let deleted = live.host.delete(&PageId::from("pages/gone.md"));
    assert_eq!(deleted, PageOperation::Pending);
    let expected = [
        ("pages/New.md", EditKind::RenamePage),
        ("pages/Old.md", EditKind::RenamePage),
        ("pages/r.md", EditKind::RenamePage),
        ("pages/gone.md", EditKind::DeletePage),
    ];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let kinds = loop {
        let kinds: BTreeMap<PageKey, Vec<EditKind>> = live
            .host
            .published_kinds
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        if expected.iter().all(|(key, _)| kinds.contains_key(*key)) {
            break kinds;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Rule 8: a host operation's write published without its kind: {kinds:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    for (key, kind) in expected {
        assert_eq!(kinds[key], vec![kind], "Rule 8: {key}");
    }
    assert!(!live.root.join("pages/gone.md").exists());
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
        rename(
            &live,
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
        rename(
            &live,
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
            rename(
                &live,
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
        rename(
            &live,
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
    drop(reservation);
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

/// A-R5 (D-10): a reservation's key lookup and a single-page rename cost
/// the same however many keys the binding registered before (keys are
/// never unregistered): no step walks every registered key.
#[test]
fn rename_and_reserve_cost_is_independent_of_registered_keys() {
    let cost = |registered: usize| {
        let live = Live::new(&[
            ("pages/Old.md", "title:: Old\n\n- body\n"),
            ("pages/r.md", "- see [[Old]]\n"),
        ]);
        for n in 0..registered {
            let page = PageId::from(format!("pages/k{n}.md").as_str());
            live.host.register(page.as_str(), &page);
        }
        super::super::production::SPELLING_LOOKUPS.with(|n| n.set(0));
        super::super::KEY_VISITS.with(|n| n.set(0));
        drop(
            live.host
                .reserve(|| vec![PageId::from("pages/r.md")], Input::Refuse)
                .unwrap(),
        );
        let view = live.store.whole_graph().unwrap();
        live.host
            .rename(
                &PageId::from("pages/Old.md"),
                &PageId::from("pages/New.md"),
                &[PageId::from("pages/r.md")],
                &map("Old", "New"),
                &view,
            )
            .unwrap();
        let lookups = super::super::production::SPELLING_LOOKUPS.with(|n| n.get());
        let visits = super::super::KEY_VISITS.with(|n| n.get());
        live.host.stop();
        (lookups, visits)
    };
    let (few, many) = (cost(10), cost(400));
    assert_eq!(
        few, many,
        "A-R5/D-10: a reserve plus a one-page rename cost (spelling lookups, registered-key \
         visits) {few:?} with 10 registered keys and {many:?} with 400; exemplars binding.rs \
         key_spelled and operations.rs rename_with's version allocation"
    );
}

/// B1: the host and the held index decide a spelling's identity by the
/// same rule. Another spelling of an open page's entry (a folding volume's
/// case alias; elsewhere a symlink, the Q4 test's stand-in) opens the same
/// key, is never sourced from disk while the page is held, and the entry
/// keeps one installed row.
#[test]
fn the_host_and_the_held_index_agree_on_an_alias_spelling() {
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
    let (key, _) = live.open("pages/Foo.md");
    let (again, _) = live.open("pages/foo.md");
    assert_eq!(
        again, key,
        "B1: the host gave an alias spelling its own key"
    );
    assert!(
        !matches!(live.store.graph.source(&alias), crate::model::Source::Disk),
        "B1: the held index sourced an alias spelling of a held page from disk"
    );
    let graph = &live.store.graph;
    assert!(graph.rebuild_cache_cancellable(|| false));
    let rows = graph.with_pages(|pages| {
        pages
            .iter()
            .filter(|(entry, _)| entry.path == alias || entry.path == live.root.join(&key))
            .count()
    });
    assert_eq!(rows, 1, "B1: one entry, one installed row");
    live.host.stop();
}

/// The host page `key` as the driver holds it now.
fn held_page(live: &Live, key: &str) -> Option<Page> {
    let state = live.host.driver.shared.state.lock().unwrap();
    state.progress.host.pages.get(key).cloned()
}

/// R3a(2)/R4 (A-W1): a referrer whose unsaved buffer already says the new
/// name (its saves keep failing) is not the rename's to flush or refuse.
/// The rename succeeds without it; the referrer keeps its buffer, version
/// and unsaved input, its file is untouched, and it publishes no rename.
/// With no source file (a references-only rerun) there is then nothing to
/// write: an empty success, never a replan loop.
#[cfg(unix)]
#[test]
fn a_rename_leaves_an_unsaved_referrer_that_already_says_the_new_name() {
    use std::os::unix::fs::PermissionsExt;
    for source in ["pages/Old.md", "pages/Ghost.md"] {
        let name = if source == "pages/Old.md" {
            "Old"
        } else {
            "Ghost"
        };
        let mut files = vec![("journals/m.md", format!("- see [[{name}]]\n"))];
        if source == "pages/Old.md" {
            files.push(("pages/Old.md", "- old\n".to_owned()));
        }
        let files: Vec<(&str, &str)> = files.iter().map(|(p, b)| (*p, b.as_str())).collect();
        let live = Live::new(&files);
        let (key, page) = live.open("journals/m.md");
        // m's saves fail (no temp file in a read-only directory) while the
        // rename's own writes under `pages/` succeed.
        let journals = live.root.join("journals");
        let mode = |bits| fs::set_permissions(&journals, fs::Permissions::from_mode(bits)).unwrap();
        mode(0o555);
        let id = live
            .submit(&key, "- see [[New]] typed\n", page.version, None)
            .unwrap();
        live.answer(&key, id);
        settle();
        let before = held_page(&live, &key).unwrap();
        assert!(!before.clean(), "{source}: the edit stays unsaved");
        assert_eq!(
            rename(
                &live,
                &PageId::from(source),
                &PageId::from("pages/New.md"),
                &[PageId::from("journals/m.md")],
                &map(name, "New"),
            ),
            Ok((vec![], vec![])),
            "{source}"
        );
        assert_eq!(held_page(&live, &key).unwrap(), before, "{source}");
        assert_eq!(live.disk("journals/m.md"), format!("- see [[{name}]]\n"));
        if source == "pages/Old.md" {
            assert_eq!(live.disk("pages/New.md"), "- old\n");
            assert!(!live.root.join(source).exists());
        } else {
            assert!(!live.root.join("pages/New.md").exists());
        }
        let kinds = live.host.published_kinds.lock().unwrap().clone();
        assert!(
            !kinds.iter().any(|(page, _)| page == "journals/m.md"),
            "{source}: {kinds:?}"
        );
        mode(0o755);
        live.host.stop();
    }
}

/// R3 (A-W1), the host's final value: a held page the planner did not list
/// (its referrers come from the index) is found from its buffer, and the
/// operation's write, edit kind and report cover it.
#[test]
fn a_held_referrer_the_planner_missed_is_part_of_the_rename() {
    let live = Live::new(&[("pages/Old.md", "- old\n"), ("pages/h.md", "- x\n")]);
    let (key, page) = live.open("pages/h.md");
    let id = live
        .submit(&key, "- see [[Old]]\n", page.version, None)
        .unwrap();
    live.answer(&key, id);
    live.until_disk(&key, "- see [[Old]]\n");
    let renamed = rename(
        &live,
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &[],
        &map("Old", "New"),
    );
    assert_eq!(renamed, Ok((vec![PageId::from("pages/h.md")], vec![])));
    assert_eq!(live.disk("pages/h.md"), "- see [[New]]\n");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let kinds = live.host.published_kinds.lock().unwrap().clone();
        if kinds
            .iter()
            .any(|(page, kinds)| page == "pages/h.md" && kinds == &[EditKind::RenamePage])
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "h.md published no rename: {kinds:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    live.host.stop();
}

/// B2 (A-W1): the host admits a rename only in the context it was planned
/// in. A config delivered after the plan's view refuses the operation
/// before any write or draft; the caller replans in a fresh view.
#[test]
fn a_rename_planned_before_a_config_change_is_refused_unwritten() {
    let live = Live::new(&[
        ("pages/Old.md", "- old\n"),
        ("pages/r.md", "- see [[Old]]\n"),
    ]);
    let view = live.store.whole_graph().unwrap();
    fs::create_dir_all(live.root.join("logseq")).unwrap();
    fs::write(
        live.root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    live.store.refresh(crate::Depth::Bytes).unwrap();
    let renamed = live.host.rename(
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &[PageId::from("pages/r.md")],
        &map("Old", "New"),
        &view,
    );
    assert_eq!(renamed, Err(RenameRefusal::Refused));
    assert_eq!(live.disk("pages/Old.md"), "- old\n");
    assert_eq!(live.disk("pages/r.md"), "- see [[Old]]\n");
    assert!(!live.root.join("pages/New.md").exists());
    assert!(live.drafts().is_empty());
    assert_eq!(
        rename(
            &live,
            &PageId::from("pages/Old.md"),
            &PageId::from("pages/New.md"),
            &[PageId::from("pages/r.md")],
            &map("Old", "New"),
        ),
        Ok((vec![PageId::from("pages/r.md")], vec![]))
    );
    live.host.stop();
}
