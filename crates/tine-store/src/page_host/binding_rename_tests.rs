//! General-operation rename proofs against a real Store (STEP3 F10, Q4).
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::page_host::binding) enum RenameRefusal {
    Unsaved(PageId),
    Unwritten(PageId),
    Refused,
    DraftFailed,
    Superseded(PageId),
    Uncertain,
    Busy,
}

/// General admission/order adapter; held-input policy is proved through pages.rs.
impl PageHost {
    pub(in crate::page_host::binding::tests) fn rename(
        &self,
        source: &PageId,
        target: &PageId,
        referrers: &[PageId],
        map: &crate::RenameMap,
        view: &crate::WholeGraph,
    ) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
        use crate::transaction::validation::rewrite;
        let inputs = self.inputs().map_err(|_| RenameRefusal::Refused)?;
        let mut refs: BTreeSet<PageId> = referrers.iter().cloned().collect();
        let rewrite_page = |id: &PageId, bytes: &[u8]| {
            let path = self.store.graph.root.join(id.as_str());
            // This adapter supplies valid fixtures for writer/order proofs.
            // Feature-boundary tests own malformed/marked/unsaved policy.
            rewrite(bytes, &path, map, view.config.file_name_format)
                .expect("valid general-writer rename fixture")
        };
        refs.remove(source);
        refs.remove(target);
        let read = |id: &PageId| match self.store.read(&id.file(), None) {
            Ok((bytes, rev)) => Ok(Some((bytes, rev))),
            Err(crate::StoreError::NotFound(_)) => Ok(None),
            Err(_) => Err(RenameRefusal::Refused),
        };
        let mut tx = self.store.page_plan(Some(EditKind::RenamePage));
        tx.expect_view(view);
        let mut writes = false;
        let mut changed = Vec::new();
        for id in refs {
            let Some((old, rev)) = read(&id)? else {
                continue;
            };
            let new = rewrite_page(&id, &old);
            if new != old {
                let keyed = |pairs: &[(String, String)]| {
                    pairs
                        .iter()
                        .map(|(a, b)| (tine_core::refs::page_key(a), b.clone()))
                        .collect()
                };
                let certificate = tine_core::refs::rename_proposal(
                    std::str::from_utf8(&old).unwrap(),
                    id.as_str().ends_with(".org"),
                    &keyed(&map.0),
                    &keyed(&map.1),
                    view.config.file_name_format,
                )
                .unwrap()
                .trace;
                changed.push(id.clone());
                tx.rewrite_refs_prepared(
                    &[EditKind::RenamePage],
                    &id,
                    rev,
                    (
                        String::from_utf8(old).unwrap(),
                        String::from_utf8(new).unwrap(),
                    ),
                    Some(map),
                    view.config.file_name_format,
                    certificate,
                );
                writes = true;
            }
        }
        if let Some((_, rev)) = read(source)? {
            tx.move_file(&source.file(), rev, &target.file(), Some(map));
            writes = true;
        }
        if writes {
            let mut op = tx.plan().map_err(|why| match why {
                crate::Why::Failed(error) if error.kind == std::io::ErrorKind::WouldBlock => {
                    RenameRefusal::Busy
                }
                _ => RenameRefusal::Refused,
            })?;
            op.inputs = Some(inputs);
            op.separate_case_entries = true;
            self.apply(op).map_err(|why| match why {
                ApplyRefusal::Unsaved(id) => RenameRefusal::Unsaved(id),
                ApplyRefusal::Unwritten(id) => RenameRefusal::Unwritten(id),
                ApplyRefusal::Superseded(id) => RenameRefusal::Superseded(id),
                ApplyRefusal::DraftFailed => RenameRefusal::DraftFailed,
                ApplyRefusal::Busy => RenameRefusal::Busy,
                ApplyRefusal::Uncertain | ApplyRefusal::SpellingIncomplete => {
                    RenameRefusal::Uncertain
                }
                _ => RenameRefusal::Refused,
            })?;
        }
        Ok((changed, vec![]))
    }
}

#[test]
fn rename_cost_is_independent_of_registered_keys() {
    let cost = |registered: usize| {
        let live = Live::new(&[
            ("pages/Old.md", "title:: Old\n\n- body\n"),
            ("pages/r.md", "- see [[Old]]\n"),
        ]);
        for n in 0..registered {
            let page = PageId::from(format!("pages/k{n}.md").as_str());
            live.host.try_register(page.as_str(), &page).unwrap();
        }
        let shared = &live.host.driver.shared;
        uncount();
        shared.call(|_| uncount()).unwrap();
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
        let (caller, driver) = (counted(), shared.read(|_| counted()).unwrap());
        live.host.stop();
        (caller.0 + driver.0, caller.1 + driver.1)
    };
    let (few, many) = (cost(10), cost(400));
    let extra = 400 - 10;
    assert!(
        many.0 < few.0 + extra && many.1 < few.1 + extra,
        "A-R5/D-10: a one-page rename cost (spelling lookups, registered-key \
         visits) {few:?} with 10 registered keys and {many:?} with 400; exemplars binding.rs \
         key_spelled and operations.rs rename_with's version allocation"
    );
}
use crate::RenameMap;

fn map(from: &str, to: &str) -> RenameMap {
    RenameMap(vec![(from.into(), to.into())], Vec::new())
}

/// Retry stale planning contexts as the feature does (B2).
pub(super) fn rename(
    host: &PageHost,
    source: &PageId,
    target: &PageId,
    referrers: &[PageId],
    map: &RenameMap,
) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
    for _ in 0..20 {
        let view = host.store.whole_graph().unwrap();
        match host.rename(source, target, referrers, map, &view) {
            Err(RenameRefusal::Refused) => continue,
            done => return done,
        }
    }
    Err(RenameRefusal::Refused)
}

pub(super) fn folds_case(root: &std::path::Path) -> bool {
    let probe = root.join("Fold-Probe");
    fs::write(&probe, b"").unwrap();
    let folds = root.join("fold-probe").exists();
    fs::remove_file(probe).unwrap();
    folds
}

/// OG-RULES Rule 8 (A-K1): every publication carries RenamePage or DeletePage.
#[test]
fn host_rename_and_delete_publish_their_edit_kinds() {
    let live = Live::new(&[
        ("pages/Old.md", "- old\n"),
        ("pages/r.md", "- see [[Old]]\n"),
        ("pages/gone.md", "- gone\n"),
    ]);
    let renamed = rename(
        &live.host,
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &[PageId::from("pages/r.md")],
        &map("Old", "New"),
    );
    assert!(renamed.is_ok(), "{renamed:?}");
    // Waiting means the host is still finishing the rename (D4): the
    // window retries once settled (`wiring.ts`), and so does this test.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let deleted = loop {
        let deleted = live.host.delete(
            live.host.session(),
            &PageId::from("pages/gone.md"),
            Some(&b"- gone\n"[..]),
        );
        if deleted != PageOperation::Waiting || std::time::Instant::now() > deadline {
            break deleted;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(deleted, PageOperation::Applied);
    let expected = [
        ("pages/New.md", EditKind::RenamePage),
        ("pages/Old.md", EditKind::RenamePage),
        ("pages/r.md", EditKind::RenamePage),
        ("pages/gone.md", EditKind::DeletePage),
    ];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let kinds = loop {
        let kinds: BTreeMap<PageKey, Vec<EditKind>> =
            live.kinds.lock().unwrap().iter().cloned().collect();
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

/// Q4: both spelling keys retain their draft/custody obligations across a crash.
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
                &live.host,
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
            kinds,
            ..
        } = live;
        drop(host);
        let host = PageHost::start(&store, &app, "test-graph", recording(&kinds), |_| {}).unwrap();
        let live = Live {
            _dir,
            root,
            store,
            host,
            mail,
            kinds,
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
fn counted() -> (usize, usize) {
    let lookups = super::super::production::SPELLING_LOOKUPS.with(|n| n.get());
    (lookups, super::super::KEY_VISITS.with(|n| n.get()))
}

fn uncount() {
    super::super::production::SPELLING_LOOKUPS.with(|n| n.set(0));
    super::super::KEY_VISITS.with(|n| n.set(0));
}
/// B1: aliases share one held key and index row; the held index never reads their disk.
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

/// P1 baselineEntry: own or proved folded spelling, never a separately listed alias.
#[test]
fn an_open_reply_says_whether_the_path_is_proved_the_entry() {
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
    let open = |rel: &str| {
        let session = live.host.session();
        let opened = live
            .host
            .open(session, live.id(), &PageId::from(rel), "foo");
        opened.unwrap()
    };
    let own = open("pages/Foo.md");
    assert!(own.baseline_entry, "a page's own spelling");
    let aliased = open("pages/foo.md");
    assert_eq!(aliased.key, own.key);
    assert_eq!(
        aliased.baseline_entry,
        folds_case(&live.root),
        "only Q4's alias is proved the same entry"
    );
    live.host.stop();
}
/// REVIEW-3a4: unsaved input on a fresh key subsequently respelled to b.md.
pub(super) fn typed_fresh_entry(live: &Live, body: &str) -> PageKey {
    let (old, _) = live.open("pages/a.md");
    fs::rename(live.root.join("pages/a.md"), live.root.join("pages/A.md")).unwrap();
    test_spelling(live, &old, "pages/a.md", "pages/A.md");
    fs::write(live.root.join("pages/a.md"), "- new distinct entry\n").unwrap();
    let (new, _) = live.open("pages/a.md");
    assert_ne!(new, old);
    fs::rename(live.root.join("pages/a.md"), live.root.join("pages/b.md")).unwrap();
    test_spelling(live, &new, "pages/a.md", "pages/b.md");
    let (again, page) = live.open("pages/b.md");
    assert_eq!(again, new);
    live.faults(super::super::io::Phase::PageTemp, 100);
    let id = live.id();
    let generation = live.host.session();
    let edit = live.dto("pages/b.md", body);
    live.host
        .submit(
            generation,
            id,
            &new,
            &edit,
            page.version,
            None,
            &[EditKind::SaveBlock],
        )
        .unwrap();
    live.answer(&new, id);
    new
}

/// B2 (A-W1): a changed config refuses unwritten, then a fresh plan completes.
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
            &live.host,
            &PageId::from("pages/Old.md"),
            &PageId::from("pages/New.md"),
            &[PageId::from("pages/r.md")],
            &map("Old", "New"),
        ),
        Ok((vec![PageId::from("pages/r.md")], vec![]))
    );
    live.host.stop();
}

/// The caller's bound is on a stall: publishing referrers keep a long rename alive.
#[test]
fn a_long_rename_that_keeps_publishing_is_waited_for() {
    use super::super::retained::OPERATION_WAIT;
    let referrers: Vec<String> = (0..3).map(|i| format!("pages/r{i}.md")).collect();
    let mut files = vec![("pages/Old.md", "- old\n")];
    files.extend(
        referrers
            .iter()
            .map(|key| (key.as_str(), "- see [[Old]]\n")),
    );
    let live = Live::new(&files);
    let ids: Vec<PageId> = referrers
        .iter()
        .map(|key| PageId::from(key.as_str()))
        .collect();
    // Four page saves (the destination, three referrers), each a third of
    // the bound: the whole outlasts it, no gap between saves comes near it.
    live.host.driver.shared.with_state_for_tests(|state| {
        state
            .progress
            .host
            .fs
            .delays
            .insert(super::io::Phase::PageTemp, OPERATION_WAIT / 3);
    });
    let started = Instant::now();
    let renamed = rename(
        &live.host,
        &PageId::from("pages/Old.md"),
        &PageId::from("pages/New.md"),
        &ids,
        &map("Old", "New"),
    );
    assert!(
        started.elapsed() > OPERATION_WAIT,
        "the rename outlasted the bound"
    );
    assert_eq!(
        renamed,
        Ok((ids, vec![])),
        "a rename that keeps publishing is waited for"
    );
    for key in &referrers {
        assert_eq!(live.disk(key), "- see [[New]]\n");
    }
    assert!(!live.root.join("pages/Old.md").exists());
    live.host.stop();
}

/// P1/F4: an ended driver refuses each operation without panicking.
#[test]
fn every_operation_on_an_ended_driver_is_refused() {
    let mut live = Live::new(&[("pages/Old.md", "- body\n")]);
    let session = live.host.session();
    let view = live.store.whole_graph().unwrap();
    live.host.driver.join();
    assert!(live.host.driver.shared.ended());
    let old = PageId::from("pages/Old.md");
    let opened = live.host.open(session, 1, &old, "Old").err();
    assert_eq!(opened, Some(PageRefusal::NotAdmitted), "open");
    let deleted = live.host.delete(session, &old, Some(&b"- body\n"[..]));
    assert_eq!(deleted, PageOperation::Refused, "delete");
    assert_eq!(live.host.owed(session, Some(&[old.clone()])), None, "owed");
    let renamed = live.host.rename(
        &old,
        &PageId::from("pages/New.md"),
        &[],
        &map("Old", "New"),
        &view,
    );
    assert_eq!(renamed.err(), Some(RenameRefusal::Refused), "rename");
}

/// P1: a held path lock bounds rename (Busy) and delete (Waiting), without writes.
#[test]
fn a_rename_or_delete_under_a_held_path_lock_is_bounded() {
    use super::super::retained::OPERATION_WAIT;
    let live = Live::new(&[("pages/Old.md", "- body\n"), ("pages/Gone.md", "- gone\n")]);
    let session = live.host.session();
    let view = live.store.whole_graph().unwrap();
    let graph = &live.store.graph;
    let old_lock = graph.page_lock(&live.root.join("pages/Old.md"));
    let gone_lock = graph.page_lock(&live.root.join("pages/Gone.md"));
    let (old_held, gone_held) = (old_lock.lock().unwrap(), gone_lock.lock().unwrap());
    let started = Instant::now();
    let (sender, received) = mpsc::channel();
    let (renamed, deleted) = std::thread::scope(|scope| {
        let _renamed = scope.spawn(|| {
            let renamed = live.host.rename(
                &PageId::from("pages/Old.md"),
                &PageId::from("pages/New.md"),
                &[],
                &map("Old", "New"),
                &view,
            );
            let _ = sender.send(renamed);
        });
        let gone = PageId::from("pages/Gone.md");
        let host = &live.host;
        let deleted = scope.spawn(move || host.delete(session, &gone, Some(&b"- gone\n"[..])));
        let renamed = received.recv_timeout(OPERATION_WAIT + Duration::from_secs(3));
        drop(old_held);
        (renamed, deleted.join().unwrap())
    });
    let waited = started.elapsed();
    drop(gone_held);
    let renamed = renamed.expect("rename preflight must be bounded");
    assert!(
        waited < OPERATION_WAIT + Duration::from_secs(3),
        "bounded: {waited:?}"
    );
    assert_eq!(renamed.err(), Some(RenameRefusal::Busy), "rename");
    assert_eq!(deleted, PageOperation::Waiting, "delete");
    assert_eq!(live.disk("pages/Old.md"), "- body\n");
    assert_eq!(live.disk("pages/Gone.md"), "- gone\n");
    assert!(!live.root.join("pages/New.md").exists());
    live.host.stop();
}

// Fixture-only identity setup: the physical move already happened.
fn test_spelling(live: &Live, key: &str, from: &str, to: &str) {
    live.host
        .respell_for_tests(key, &PageId::from(from), &PageId::from(to));
}

#[test]
fn a_new_entry_never_takes_a_respelled_key() {
    let live = Live::new(&[("pages/a.md", "- old\n")]);
    if folds_case(&live.root) {
        live.host.stop();
        return;
    }
    let (old, _) = live.open("pages/a.md");
    fs::rename(live.root.join("pages/a.md"), live.root.join("pages/A.md")).unwrap();
    test_spelling(&live, &old, "pages/a.md", "pages/A.md");
    fs::write(live.root.join("pages/a.md"), "- new distinct entry\n").unwrap();
    let (new, page) = live.open("pages/a.md");
    assert_ne!(new, old, "B1: the new entry reused the respelled key");
    let MailText::Page { dto } = &page.text else {
        panic!("an open answer carries the page: {:?}", page.text);
    };
    assert_eq!(dto.blocks[0].raw, "new distinct entry");
    let id = live.id();
    let generation = live.host.session();
    let edit = live.dto("pages/a.md", "- typed for new a\n");
    live.host
        .submit(
            generation,
            id,
            &new,
            &edit,
            page.version,
            None,
            &[EditKind::SaveBlock],
        )
        .unwrap();
    live.until_disk("pages/a.md", "- typed for new a\n");
    assert_eq!(
        live.disk("pages/A.md"),
        "- old\n",
        "B1: the other entry changed"
    );
    live.host.stop();
}
