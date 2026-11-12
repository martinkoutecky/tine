use super::*;
use crate::test_host_writer::{save_on, HostWriter, SaveRefused};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs as disk, sync::Arc};
use tine_core::model::PageDto;
use tine_store::{FileRev, Refusal, SaveBase, Why};

/// The stale-plan regression probe. Preflight is read-only; a stale context
/// is `Ok(None)`, so no feature test needs a direct page commit.
fn preflight_rename(
    store: &Store,
    old: &str,
    plan: RenamePlan,
    unsaved_paths: &[String],
) -> io::Result<Option<()>> {
    preflight_rename_after(store, old, plan, unsaved_paths, || {})
}

/// [`preflight_rename`] with `between` run after the transaction is built
/// from the planning reads and before its preflight.
fn preflight_rename_after(
    store: &Store,
    old: &str,
    plan: RenamePlan,
    unsaved_paths: &[String],
    between: impl FnOnce(),
) -> io::Result<Option<()>> {
    let tx = match rename_transaction(store, old, plan, unsaved_paths)? {
        RenameTransaction::Unchanged(_) => return Ok(Some(())),
        RenameTransaction::Writes { tx, .. } => tx,
    };
    between();
    match tx.plan() {
        Ok(_) => Ok(Some(())),
        Err(tine_store::Why::Conflict { .. }) => Ok(None),
        Err(why) => Err(crate::why_error(why)),
    }
}

/// Keep mine: a save guarded on the bytes on disk now, not the editor's base.
fn keep_mine(store: &Arc<Store>, id: &PageId, page: &PageDto) -> Result<FileRev, SaveRefused> {
    let (_, current) = store.read(&id.file(), None).unwrap();
    HostWriter::for_store(store).save(
        tine_store::EditKind::ReplacePage,
        id,
        SaveBase::Existing(current),
        page,
    )
}

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(store: &Arc<Store>, run: impl FnOnce(&PageHost) -> T) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn text_at(root: &std::path::Path, file: &str) -> String {
    disk::read_to_string(root.join(file)).unwrap()
}

#[test]
fn single_rename_rewrites_title_and_refs_and_reports_marked_referrers() {
    let marked = "- [[Old]]\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n";
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", "title:: Old\n\n- [[Old]] self\n"),
        ("pages/r.md", "- see [[Old]]\n"),
        ("pages/m.md", marked),
    ]);
    let report = hosted(&store, |host| {
        rename_or_merge_page(&store, host, "Old", "New", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(
        report
            .touched
            .iter()
            .filter(|page| page.path != "pages/Old.md" && page.path != "pages/New.md")
            .map(|page| page.path.as_str())
            .collect::<Vec<_>>(),
        vec!["pages/r.md"]
    );
    assert_eq!(report.skipped_conflicted_referrers, vec!["pages/m.md"]);
    assert_eq!(
        text_at(&root, "pages/New.md"),
        "title:: New\n\n- [[New]] self\n"
    );
    assert!(!root.join("pages/Old.md").exists());
    assert_eq!(text_at(&root, "pages/r.md"), "- see [[New]]\n");
    assert_eq!(text_at(&root, "pages/m.md"), marked);
}

#[test]
fn single_rename_title_read_panic_refuses_untouched_and_next_rename_works() {
    // The store's test-faults sentinel fires inside rewrite_move, under the
    // real preflight panic boundary; this test performs no rewrite itself.
    let bad = "title:: Old\n\n- __TINE_TEST_TITLE_READ_PANIC__\n";
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", bad),
        ("pages/r.md", "- see [[Old]] and [[Other]]\n"),
        ("pages/Other.md", "- other\n"),
    ]);
    hosted(&store, |host| {
        let error = rename_or_merge_page(&store, host, "Old", "New", None, None, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            "page could not be parsed; Tine will not rewrite it"
        );
        assert_eq!(text_at(&root, "pages/Old.md"), bad);
        assert!(!root.join("pages/New.md").exists());
        assert_eq!(
            text_at(&root, "pages/r.md"),
            "- see [[Old]] and [[Other]]\n"
        );
        let report = rename_or_merge_page(&store, host, "Other", "Else", None, None, &[]).unwrap();
        assert_eq!(report.outcome, RenameOutcome::Renamed);
        assert_eq!(
            report
                .touched
                .iter()
                .filter(|page| page.path != "pages/Other.md" && page.path != "pages/Else.md")
                .map(|page| page.path.as_str())
                .collect::<Vec<_>>(),
            vec!["pages/r.md"]
        );
        assert!(report.skipped_conflicted_referrers.is_empty());
        assert_eq!(text_at(&root, "pages/r.md"), "- see [[Old]] and [[Else]]\n");
        assert_eq!(text_at(&root, "pages/Else.md"), "- other\n");
        assert!(!root.join("pages/Other.md").exists());
    });
}

#[test]
fn single_rename_preserves_complete_tag_targets_in_markdown_and_org() {
    let (_temp, root, store) = b2_graph(&[
        ("pages/foo.md", "- source\n"),
        ("pages/r.md", "- [[foo]] and #foo+bar #foo🎉 #foo.bar #foo. #foo... #foo, #foo\n- `#foo.`\n```\n#foo.\n```\n"),
        ("pages/r.org", "* [[foo]] and #foo+bar #foo🎉 #foo.bar #foo. #foo... #foo, #foo\n* =#foo.=\n#+BEGIN_SRC\n#foo.\n#+END_SRC\n"),
    ]);
    let report = hosted(&store, |host| {
        rename_or_merge_page(&store, host, "foo", "New", None, None, &[])
    })
    .unwrap();
    assert_eq!(report.outcome, RenameOutcome::Renamed);
    assert_eq!(text_at(&root, "pages/New.md"), "- source\n");
    assert!(!root.join("pages/foo.md").exists());
    assert_eq!(text_at(&root, "pages/r.md"), "- [[New]] and #foo+bar #foo🎉 #foo.bar #New. #New... #New, #New\n- `#foo.`\n```\n#foo.\n```\n");
    assert_eq!(text_at(&root, "pages/r.org"), "* [[New]] and #foo+bar #foo🎉 #foo.bar #New. #New... #New, #New\n* =#foo.=\n#+BEGIN_SRC\n#foo.\n#+END_SRC\n");
}

fn hold_rename_input(host: &PageHost, store: &Store, id: &PageId, raw: &str) {
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let next_id = reloaded["nextId"].as_u64().unwrap();
    let opened = host.open(session, next_id, id, "r").unwrap();
    let key = serde_json::to_value(opened).unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let version = loop {
        if let Some((_, version)) = host.read(id).unwrap().revision {
            break version;
        }
        assert!(std::time::Instant::now() < deadline, "open completes");
        std::thread::yield_now();
    };
    let mut doc = store.page(id).unwrap().doc;
    doc.blocks[0].raw = raw.into();
    host.submit(
        session,
        next_id + 1,
        &key,
        &doc,
        version,
        None,
        &[tine_store::EditKind::SaveBlock],
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let expected = format!("- {raw}\n");
    while host.read(id).unwrap().bytes.as_deref() != Some(expected.as_bytes()) {
        assert!(
            std::time::Instant::now() < deadline,
            "typed input reaches the host"
        );
        std::thread::yield_now();
    }
}

#[test]
fn single_rename_refuses_unwritable_and_unsaved_referrers_untouched() {
    let org = "* see [[old]]\n*** skipped level\n";
    let (_temp, root, store) = b2_graph(&[
        ("pages/old.md", "- old\n"),
        ("pages/o.org", org),
        ("pages/r.md", "- see [[old]]\n"),
    ]);
    hosted(&store, |host| {
        let error = rename_or_merge_page(&store, host, "old", "new", None, None, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            error.to_string(),
            format!(
                "cannot rename: {} is a read-only .org file (does not round-trip)",
                shown_path(&root.join("pages/o.org"))
            )
        );
        assert!(!error.to_string().contains(r"\\?\"), "{error}");
        assert_eq!(text_at(&root, "pages/o.org"), org);
        assert_eq!(text_at(&root, "pages/old.md"), "- old\n");
        assert_eq!(text_at(&root, "pages/r.md"), "- see [[old]]\n");
        assert!(!root.join("pages/new.md").exists());
    });
    // A separate fixture reaches unsaved-input policy without an earlier Org refusal.
    let (_temp, root, store) = b2_graph(&[
        ("pages/old.md", "- old\n"),
        ("pages/r.md", "- see [[old]]\n"),
    ]);
    tine_store::host_faults::fail(
        tine_store::host_faults::Phase::PageTemp,
        io::ErrorKind::Other,
        1000,
    );
    hosted(&store, |host| {
        let id = PageId::from("pages/r.md");
        hold_rename_input(host, &store, &id, "typed [[old]]");
        let before = host.read(&id).unwrap();
        let error = rename_or_merge_page(&store, host, "old", "new", None, None, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(error.to_string(), "“r” has changes Tine could not save, and this rename would rewrite it. Save or discard those changes, then rename again.");
        assert_eq!(host.read(&id).unwrap(), before);
        assert_eq!(text_at(&root, "pages/old.md"), "- old\n");
        assert_eq!(text_at(&root, "pages/r.md"), "- see [[old]]\n");
        assert!(!root.join("pages/new.md").exists());
    });
}

#[test]
fn single_rename_refusal_names_a_fresh_key_referrer_by_its_spelling() {
    let (_temp, root, store) = b2_graph(&[("pages/a.md", "- old\n"), ("pages/t.md", "- t\n")]);
    if root.join("pages/A.md").exists() {
        return;
    }
    tine_store::host_faults::fail(
        tine_store::host_faults::Phase::PageTemp,
        io::ErrorKind::Other,
        1000,
    );
    hosted(&store, |host| {
        let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
        let session = reloaded["session"].as_u64().unwrap();
        let request_id = std::cell::Cell::new(reloaded["nextId"].as_u64().unwrap());
        let open = |path: &str| {
            let id = request_id.get();
            request_id.set(id + 1);
            let page = PageId::from(path);
            let opened = host.open(session, id, &page, "r").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while host.read(&page).unwrap().revision.is_none() {
                assert!(std::time::Instant::now() < deadline, "open completes");
                std::thread::yield_now();
            }
            serde_json::to_value(opened).unwrap()
        };
        let old = open("pages/a.md");
        disk::rename(root.join("pages/a.md"), root.join("pages/A.md")).unwrap();
        host.respell_for_tests(
            old["key"].as_str().unwrap(),
            &PageId::from("pages/a.md"),
            &PageId::from("pages/A.md"),
        );
        assert_eq!(open("pages/A.md")["key"], old["key"]);
        disk::write(root.join("pages/a.md"), "- new distinct entry\n").unwrap();
        let fresh = open("pages/a.md");
        assert_ne!(fresh["key"], old["key"]);
        assert!(
            fresh["key"].as_str().unwrap().contains('\0'),
            "the replacement entry has a fresh opaque key"
        );
        disk::rename(root.join("pages/a.md"), root.join("pages/b.md")).unwrap();
        host.respell_for_tests(
            fresh["key"].as_str().unwrap(),
            &PageId::from("pages/a.md"),
            &PageId::from("pages/b.md"),
        );
        assert_eq!(open("pages/b.md")["key"], fresh["key"]);
        let id = PageId::from("pages/b.md");
        hold_rename_input(host, &store, &id, "see [[t]]");
        let before = host.read(&id).unwrap();
        assert_eq!(before.page, id);
        let error = rename_or_merge_page(&store, host, "t", "u", None, None, &[]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(error.to_string(), "“b” has changes Tine could not save, and this rename would rewrite it. Save or discard those changes, then rename again.");
        assert_eq!(host.read(&id).unwrap(), before);
        assert_eq!(text_at(&root, "pages/t.md"), "- t\n");
        assert!(!root.join("pages/u.md").exists());
    });
}

#[test]
fn prepared_referrer_rewrite_still_refuses_an_external_edit() {
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", "- source\n"),
        ("pages/r.md", "- [[Old]]\n"),
    ]);
    let mut first = plan_rename(&store, "Old", "New", None, RenameChoice::Rename, &|| {}).unwrap();
    first.pages(&store).unwrap();
    disk::write(root.join("pages/r.md"), "- [[Old]] outside edit\n").unwrap();
    assert_eq!(preflight_rename(&store, "Old", first, &[]).unwrap(), None);
    assert_eq!(text_at(&root, "pages/r.md"), "- [[Old]] outside edit\n");
    assert!(!root.join("pages/New.md").exists());
}

/// A moved page's rewrite is proposed once, from the rename's own read, and
/// carried into its move step. An external edit after that read (an editor
/// or sync delivery) still fails the base-revision guard: the rename
/// replans and nothing is written.
#[test]
fn a_carried_move_rewrite_still_refuses_an_external_edit() {
    for (old, new, moved) in [
        ("Old", "New", "pages/Old.md"),
        ("Ns", "Space", "pages/Ns%2Fchild.md"),
    ] {
        let (_temp, root, store) = b2_graph(&[
            ("pages/Old.md", "title:: Old\n\n- source [[Old]]\n"),
            ("pages/Ns.md", "- ns\n"),
            ("pages/Ns%2Fchild.md", "- child [[Ns/child]]\n"),
        ]);
        let mut plan = plan_rename(&store, old, new, None, RenameChoice::Rename, &|| {}).unwrap();
        plan.pages(&store).unwrap();
        let before = b2_tree(&root);
        let edited = "- edited outside, still naming [[Old]] and [[Ns/child]]\n";
        let result = preflight_rename_after(&store, old, plan, &[], || {
            disk::write(root.join(moved), edited).unwrap();
        });
        assert_eq!(result.unwrap(), None, "{moved}: a stale plan replans");
        let mut expected = before;
        expected.insert(
            moved.trim_start_matches("pages/").to_owned(),
            edited.to_owned(),
        );
        assert_eq!(b2_tree(&root), expected, "{moved}: nothing written");
        store.close();
    }
}

#[test]
fn a_single_org_title_rebind_keeps_its_unwritable_refusal() {
    let content = "#+TITLE: Old\n* body\n*** skipped level\n";
    let (_temp, root, store) = b2_graph(&[("pages/Old.org", content)]);
    let error = hosted(&store, |host| {
        rename_or_merge_page(&store, host, "Old", "New", None, None, &[])
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        error.to_string(),
        format!(
            "cannot rename: {} is a read-only .org file (does not round-trip)",
            shown_path(&root.join("pages/Old.org"))
        )
    );
    assert!(!error.to_string().contains(r"\\?\"), "{error}");
    assert_eq!(text_at(&root, "pages/Old.org"), content);
    assert!(!root.join("pages/New.org").exists());
}

#[test]
fn absent_destination_at_preflight_still_refuses_an_arrival_at_admission() {
    let (_temp, root, store) = b2_graph(&[("pages/Old.md", "- source\n")]);
    hosted(&store, |host| {
        let first = plan_rename(&store, "Old", "New", None, RenameChoice::Rename, &|| {}).unwrap();
        let RenameTransaction::Writes { tx, .. } =
            rename_transaction(&store, "Old", first, &[]).unwrap()
        else {
            panic!("move")
        };
        let op = tx.plan().unwrap();
        disk::write(root.join("pages/New.md"), "- external arrival\n").unwrap();
        assert_eq!(host.apply(op), Err(tine_store::ApplyRefusal::Changed));
        assert_eq!(text_at(&root, "pages/New.md"), "- external arrival\n");
        assert_eq!(text_at(&root, "pages/Old.md"), "- source\n");
    });
}

#[test]
fn general_rename_includes_a_held_referrer_missing_from_the_plan() {
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", "- source\n"),
        ("pages/r.md", "- [[Old]]\n"),
    ]);
    hosted(&store, |host| {
        let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
            .as_u64()
            .unwrap();
        host.open(session, 1, &PageId::from("pages/r.md"), "r")
            .unwrap();
        let plan = || {
            let mut first =
                plan_rename(&store, "Old", "New", None, RenameChoice::Rename, &|| {}).unwrap();
            first.candidates.retain(|id| id.as_str() != "pages/r.md");
            Ok(first)
        };
        let report = multi_rename(&store, host, "Old", plan().unwrap(), &plan, &[])
            .unwrap()
            .unwrap();
        assert!(report.touched.iter().any(|page| page.path == "pages/r.md"));
        assert_eq!(text_at(&root, "pages/r.md"), "- [[New]]\n");
    });
}

#[test]
fn clean_namespace_flush_reuses_its_selected_plan() {
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", "- source\n"),
        ("pages/child.md", "title:: Old/child\n\n- child\n"),
        ("pages/r.md", "- [[Old]]\n"),
    ]);
    hosted(&store, |host| {
        assert!(host.quiesce_for_tests(std::time::Duration::from_secs(10)));
        let first = plan_rename(&store, "Old", "New", None, RenameChoice::Rename, &|| {}).unwrap();
        assert!(first.single(&store).unwrap().is_none());
        multi_rename(
            &store,
            host,
            "Old",
            first,
            &|| unreachable!("unchanged flush must reuse selection"),
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(text_at(&root, "pages/r.md"), "- [[New]]\n");
    });
}

#[cfg(unix)]
#[test]
fn rename_preserves_an_unchanged_unsaved_referrer() {
    use std::os::unix::fs::PermissionsExt;
    for (source, outcome) in [
        ("Old", RenameOutcome::Renamed),
        ("Ghost", RenameOutcome::Unchanged),
    ] {
        let (_temp, root, store) = b2_graph(&[
            ("pages/Old.md", "- source\n"),
            ("journals/r.md", &format!("- [[{source}]]\n")),
        ]);
        hosted(&store, |host| {
            let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
                .as_u64()
                .unwrap();
            let id = PageId::from("journals/r.md");
            let opened = host.open(session, 1, &id, "r").unwrap();
            let opened = serde_json::to_value(opened).unwrap();
            let version = host.read(&id).unwrap().revision.unwrap().1;
            disk::set_permissions(root.join("journals"), disk::Permissions::from_mode(0o555))
                .unwrap();
            let mut doc = store.page(&id).unwrap().doc;
            doc.blocks[0].raw = "[[New]] typed".into();
            host.submit(
                session,
                2,
                opened["key"].as_str().unwrap(),
                &doc,
                version,
                None,
                &[tine_store::EditKind::SaveBlock],
            )
            .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while host.read(&id).unwrap().bytes.as_deref() != Some(&b"- [[New]] typed\n"[..]) {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let before = host.read(&id).unwrap();
            let report =
                rename_or_merge_page(&store, host, source, "New", None, None, &[]).unwrap();
            assert!(!report.touched.iter().any(|page| page.path == id.as_str()));
            assert_eq!(host.read(&id).unwrap(), before);
            assert_eq!(text_at(&root, id.as_str()), format!("- [[{source}]]\n"));
            assert_eq!(report.outcome, outcome);
            disk::set_permissions(root.join("journals"), disk::Permissions::from_mode(0o755))
                .unwrap();
        });
    }
}

#[test]
fn rename_plan_keeps_its_view_during_concurrent_referrer_write() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-rename-plan-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- source\n").unwrap();
    disk::write(root.join("pages/Referrer.md"), "- [[Old]] before\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let written = AtomicBool::new(false);
    hosted(&store, |host| {
        rename_page_after_inventory(
            &store,
            host,
            "Old",
            "New",
            None,
            RenameChoice::Rename,
            &[],
            || {
                if written.swap(true, Ordering::AcqRel) {
                    return;
                }
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            let id = PageId::from("pages/Referrer.md");
                            let read = store.page(&id).unwrap();
                            let mut doc = read.doc;
                            doc.blocks[0].raw = "[[Old]] concurrent".into();
                            save_on(
                                host,
                                &store,
                                tine_store::EditKind::ReplacePage,
                                &id,
                                SaveBase::Existing(read.rev),
                                &doc,
                            )
                            .unwrap();
                        })
                        .join()
                        .unwrap();
                });
            },
        )
    })
    .unwrap();
    assert!(root.join("pages/New.md").is_file());
    assert!(!root.join("pages/Old.md").exists());
    assert!(disk::read_to_string(root.join("pages/Referrer.md"))
        .unwrap()
        .contains("[[New]] concurrent"));
    store.close();
    disk::remove_dir_all(root).unwrap();
}

/// The plan-reuse check compares the published view, no second walk
/// (og-perf-rename-walks): a referrer a sync client delivers after the plan
/// read its view, published by the watcher before the commit, still
/// replans the rename and is rewritten.
#[test]
fn a_referrer_delivered_after_the_plan_replans_the_rename() {
    let (_temp, root, store) = b2_graph(&[
        ("pages/Old.md", "- source\n"),
        ("pages/r.md", "- [[Old]]\n"),
    ]);
    let delivered = AtomicBool::new(false);
    hosted(&store, |host| {
        rename_page_after_inventory(
            &store,
            host,
            "Old",
            "New",
            None,
            RenameChoice::Rename,
            &[],
            || {
                if delivered.swap(true, Ordering::AcqRel) {
                    return;
                }
                disk::write(root.join("pages/late.md"), "- late [[Old]]\n").unwrap();
                // The watcher's delivery, published now.
                store.refresh(tine_store::Depth::Stamps).unwrap();
            },
        )
    })
    .unwrap();
    assert!(delivered.load(Ordering::Acquire));
    assert_eq!(text_at(&root, "pages/r.md"), "- [[New]]\n");
    assert_eq!(text_at(&root, "pages/late.md"), "- late [[New]]\n");
    assert!(!root.join("pages/Old.md").exists());
}

#[test]
fn delete_detects_an_external_twin_before_selecting_a_file() {
    let root = std::env::temp_dir().join(format!(
        "tine-d3-delete-external-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::write(root.join("pages/Old.md"), "- markdown\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let _ = store.whole_graph().unwrap();
    disk::write(root.join("pages/Old.org"), "* org\n").unwrap();

    assert!(hosted(&store, |host| {
        let session = serde_json::to_value(host.window_reloaded()).unwrap()["session"]
            .as_u64()
            .unwrap();
        delete_page_expected(
            &store,
            host,
            session,
            "Old",
            tine_core::model::PageKind::Page,
            None,
        )
    })
    .is_err());
    assert!(root.join("pages/Old.md").exists());
    assert!(root.join("pages/Old.org").exists());
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_writes_only_the_named_twin_on_production_path() {
    // A loaded page carries its file path, so keep-mine targets that file.
    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-save-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let md = root.join("pages/Foo.md");
    let org = root.join("pages/Foo.org");
    disk::write(&md, "- md body\n").unwrap();
    disk::write(&org, "* org body\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let id = PageId::from("pages/Foo.md");
    let read = store.page(&id).unwrap();
    let mut page = read.doc;
    page.blocks[0].raw = "edited".into();

    let outcome = keep_mine(&store, &id, &page);
    let md_after = disk::read_to_string(&md).unwrap();
    let org_after = disk::read_to_string(&org).unwrap();
    store.close();
    disk::remove_dir_all(root).unwrap();
    assert!(
        outcome.is_ok(),
        "force-save must write the named twin: {outcome:?}"
    );
    assert_eq!(md_after, "- edited\n");
    assert_eq!(org_after, "* org body\n");
}

#[test]
fn creating_a_named_page_refuses_an_existing_twin() {
    // The old unpinned DTO has no production equivalent. A create by explicit
    // file name still refuses the competing physical claimant.
    let root = std::env::temp_dir().join(format!(
        "tine-b22-create-twin-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let md = root.join("pages/Foo.md");
    let org = root.join("pages/Foo.org");
    disk::write(&org, "* org body\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let id = PageId::from("pages/Foo.md");
    let mut page = store.page(&PageId::from("pages/Foo.org")).unwrap().doc;
    page.format = tine_core::model::Format::Md;
    page.blocks[0].raw = "edited".into();
    let outcome = HostWriter::for_store(&store).save(
        tine_store::EditKind::ReplacePage,
        &id,
        SaveBase::CreateNew,
        &page,
    );
    assert!(
        matches!(
            outcome,
            Err(SaveRefused::Planned(Why::Refused(Refusal::Twin { .. })))
        ),
        "{outcome:?}"
    );
    assert!(!md.exists());
    assert_eq!(disk::read_to_string(&org).unwrap(), "* org body\n");
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_keeps_guide_ephemeral_and_refuses_unknown_bytes() {
    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-basics-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/A.md");
    disk::write(&path, "- original\n").unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let id = PageId::from("pages/A.md");
    let mut page = store.page(&id).unwrap().doc;
    page.guide = true;
    // A Guide page is never saved: the plan refuses it, nothing reaches disk.
    assert!(keep_mine(&store, &id, &page).is_err());
    assert_eq!(disk::read_to_string(&path).unwrap(), "- original\n");
    page.guide = false;
    page.blocks[0].raw = "replacement".into();
    let unknown = b"\xff\xfeunknown on-disk bytes";
    disk::write(&path, unknown).unwrap();
    assert!(matches!(
        keep_mine(&store, &id, &page),
        Err(SaveRefused::Planned(_))
    ));
    assert_eq!(disk::read(&path).unwrap(), unknown);
    store.close();
    disk::remove_dir_all(root).unwrap();
}

#[test]
fn force_save_refuses_non_round_trip_org_and_header_reclassification() {
    use tine_core::model::BlockDto;

    for (label, original) in [
        ("org", "* a\n*** c\n"),
        ("lf", "A:: XX\nB:: XX\nC:: XX\n"),
        ("crlf", "A:: XX\r\nB:: XX\r\nC:: XX\r\n"),
        ("unicode", "A:: XX\nklíč:: hodnota\nC:: XX\n"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-b22-force-header-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        disk::create_dir_all(root.join("pages")).unwrap();
        let rel = if label == "org" {
            "Weird.org"
        } else {
            "Property.md"
        };
        let path = root.join("pages").join(rel);
        disk::write(&path, original).unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let id = PageId::from(format!("pages/{rel}"));
        let mut page = store.page(&id).unwrap().doc;
        if label == "org" {
            assert!(page.read_only);
            assert!(matches!(
                keep_mine(&store, &id, &page),
                Err(SaveRefused::Planned(Why::Refused(Refusal::ReadOnly(_))))
            ));
        } else {
            let normalized = original.replace("\r\n", "\n");
            let normalized = normalized.trim_end_matches('\n');
            assert_eq!(page.pre_block.as_deref(), Some(normalized));
            assert!(page.blocks.is_empty());
            let (kept, moved) = normalized.split_once('\n').unwrap();
            page.pre_block = Some(kept.into());
            page.blocks = vec![BlockDto {
                id: "corrupt-shape".into(),
                raw: moved.into(),
                ..Default::default()
            }];
            assert!(matches!(
                keep_mine(&store, &id, &page),
                Err(SaveRefused::Planned(Why::Failed(error))) if error.kind() == std::io::ErrorKind::InvalidData
            ));
        }
        assert_eq!(disk::read_to_string(&path).unwrap(), original);
        store.close();
        disk::remove_dir_all(root).unwrap();
    }
}

#[test]
fn force_save_refuses_changed_header_properties_and_preamble_loss() {
    use tine_core::model::BlockDto;

    for (shape, original, kept, moved, childful) in [
        (
            "partial-value",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "partial-key",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "Renamed:: old",
            false,
        ),
        (
            "whole-key-value",
            "A:: old\nB:: old\n",
            None,
            "Renamed:: changed\nC:: newer",
            true,
        ),
        (
            "crlf",
            "A:: old\r\nB:: old\r\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "unicode-plugin",
            "A:: old\n插件/键:: old\n",
            Some("A:: old"),
            "插件/新:: changed",
            false,
        ),
    ] {
        let root = std::env::temp_dir().join(format!(
            "tine-b22-force-changed-{shape}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        disk::create_dir_all(root.join("pages")).unwrap();
        let path = root.join("pages/Property.md");
        disk::write(&path, original).unwrap();
        let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
        let id = PageId::from("pages/Property.md");
        let before = store.page(&id).unwrap().doc;
        let mut page = store.page(&id).unwrap().doc;
        page.pre_block = kept.map(str::to_string);
        page.blocks = vec![BlockDto {
            id: "reclassified-header".into(),
            raw: moved.into(),
            children: childful
                .then(|| BlockDto {
                    id: "body".into(),
                    raw: "Body".into(),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }];
        assert!(matches!(
            keep_mine(&store, &id, &page),
            Err(SaveRefused::Planned(Why::Failed(error))) if error.kind() == std::io::ErrorKind::InvalidData
        ));
        assert_eq!(disk::read_to_string(&path).unwrap(), original);
        let after = store.page(&id).unwrap().doc;
        assert_eq!(after.pre_block, before.pre_block);
        assert_eq!(after.blocks.len(), before.blocks.len());
        assert_eq!(after.rev, before.rev);
        store.close();
        disk::remove_dir_all(root).unwrap();
    }

    let root = std::env::temp_dir().join(format!(
        "tine-b22-force-preamble-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    disk::create_dir_all(root.join("pages")).unwrap();
    let path = root.join("pages/Imported.md");
    let original = "Intro before outline\n\n- Body\n";
    disk::write(&path, original).unwrap();
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let id = PageId::from("pages/Imported.md");
    let mut page = store.page(&id).unwrap().doc;
    page.pre_block = None;
    page.blocks.insert(
        0,
        BlockDto {
            id: "candidate".into(),
            raw: "alias:: book".into(),
            ..Default::default()
        },
    );
    let outcome = keep_mine(&store, &id, &page);
    assert!(matches!(
        outcome,
        Err(SaveRefused::Planned(Why::Failed(error))) if error.kind() == std::io::ErrorKind::InvalidData
            && error.to_string().contains("existing page preamble")
    ));
    assert_eq!(disk::read_to_string(&path).unwrap(), original);
    let cached = store.page(&id).unwrap().doc;
    assert_eq!(cached.pre_block.as_deref(), Some("Intro before outline"));
    assert_eq!(cached.blocks.len(), 1);
    store.close();
    disk::remove_dir_all(root).unwrap();
}

/// STEP3 §7, F10, E17: with a host the command routes a single page's
/// rename, with or without a file, to the host's own operation; a namespace
/// rename or a merge to a host operation that flushes unsaved input
/// first; and an interrupted title completion to one that refuses it.
#[test]
fn a_hosted_rename_routes_by_the_plan_shape() {
    let root = std::env::temp_dir().join(format!(
        "tine-rename-routing-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = disk::remove_dir_all(&root);
    disk::create_dir_all(root.join("pages")).unwrap();
    disk::create_dir_all(root.join("logseq")).unwrap();
    disk::write(
        root.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (name, text) in [
        ("Start.md", "- [[Ghost]]\n"),
        ("Ns.md", "- ns\n"),
        ("Ns___child.md", "- child\n"),
        ("Into.md", "- into\n"),
        ("Merged.md", "- merged\n"),
        ("Done.md", "title:: Half\n\n- interrupted\n"),
    ] {
        disk::write(root.join("pages").join(name), text).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let shape = |old: &str, new: &str, into: Option<&str>| {
        let plan = plan_rename(
            &store,
            old,
            new,
            None,
            into.map_or(RenameChoice::Rename, RenameChoice::MergeInto),
            &|| {},
        )
        .unwrap();
        let single = plan.single(&store).unwrap();
        let single = single.map(|source| source.as_str().to_owned());
        (single, plan.input())
    };
    let source = |path: &str| Some(path.to_owned());
    assert_eq!(
        shape("Start", "Begin", None),
        (source("pages/Start.md"), Input::Flush)
    );
    assert_eq!(
        shape("Ghost", "Spirit", None),
        (source("pages/Ghost.md"), Input::Flush)
    );
    assert_eq!(shape("Ns", "Space", None), (None, Input::Flush));
    assert_eq!(
        shape("Merged", "Into", Some("pages/Into.md")),
        (None, Input::Flush)
    );
    assert_eq!(shape("Half", "Done", None), (None, Input::Refuse));
    store.close();
    let _ = disk::remove_dir_all(root);
}

/// A graph under a temp dir: `files` relative to the graph root.
/// A file as a refusal names it: where it really is (the temp dir may be an
/// 8.3 or symlinked spelling), without Windows' verbatim `\\?\` prefix.
fn shown_path(path: &std::path::Path) -> String {
    let real = disk::canonicalize(path).unwrap();
    tine_core::graph_paths::normalize_graph_path(&real.display().to_string())
}

/// A Windows user reads the refusal: the verbatim prefix of a canonical path
/// never reaches the message, on any platform the path string arrives from.
#[test]
fn a_read_only_org_refusal_names_the_file_without_the_verbatim_prefix() {
    for (path, shown) in [
        (
            r"\\?\C:\Users\me\graph\pages\Old.org",
            r"C:\Users\me\graph\pages\Old.org",
        ),
        (
            r"\\?\UNC\server\share\graph\pages\Old.org",
            r"\\server\share\graph\pages\Old.org",
        ),
        (
            "/home/me/graph/pages/Old.org",
            "/home/me/graph/pages/Old.org",
        ),
    ] {
        let error = read_only_org(std::path::Path::new(path));
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            error.to_string(),
            format!("cannot rename: {shown} is a read-only .org file (does not round-trip)")
        );
    }
}

fn b2_graph(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for sub in ["pages", "journals", "logseq"] {
        disk::create_dir_all(root.join(sub)).unwrap();
    }
    for (path, text) in files {
        disk::write(root.join(path), text).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (dir, root, store)
}

/// Deliver a `config.edn` the way the watcher does: write it, then refresh.
fn b2_deliver(store: &Store, root: &std::path::Path, config: &str) {
    disk::write(root.join("logseq/config.edn"), config).unwrap();
    store.refresh(tine_store::Depth::Bytes).unwrap();
}

fn b2_tree(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    disk::read_dir(root.join("pages"))
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            (name, disk::read_to_string(entry.path()).unwrap())
        })
        .collect()
}

const B2_LEGACY: &str = "{}\n";
const B2_LOWBAR: &str = "{:file/name-format :triple-lowbar}\n";

/// H-Q3 (A-W1, B2): every write kind commits only in the context it was
/// planned in. A config change between plan and commit, for a namespace
/// move whose destinations the old format spelled, a title rebind and a
/// merge's survivor alike, commits nothing: the attempt replans.
#[test]
fn a_rename_planned_before_a_config_change_commits_nothing() {
    for (case, old, new, into) in [
        ("move", "Ns", "Space", None),
        ("rebind", "Half", "Done", None),
        ("survivor", "Merged", "Into", Some("pages/Into.md")),
    ] {
        let (_dir, root, store) = b2_graph(&[
            ("logseq/config.edn", B2_LEGACY),
            ("pages/Ns.md", "- ns [[Merged]] [[Half]]\n"),
            ("pages/Ns%2Fchild.md", "- child\n"),
            ("pages/Into.md", "- into\n"),
            ("pages/Merged.md", "- merged\n"),
            ("pages/Done.md", "title:: Half\n\n- interrupted\n"),
        ]);
        let plan = plan_rename(
            &store,
            old,
            new,
            None,
            into.map_or(RenameChoice::Rename, RenameChoice::MergeInto),
            &|| {},
        )
        .unwrap();
        b2_deliver(&store, &root, B2_LOWBAR);
        let before = b2_tree(&root);
        assert!(
            preflight_rename(&store, old, plan, &[]).unwrap().is_none(),
            "{case}: a stale plan replans"
        );
        assert_eq!(b2_tree(&root), before, "{case}: nothing written");
        store.close();
    }
}

/// H-Q3 (A-W1, B2): the pause between building the targets and capturing
/// the write set. The write set comes from the stale plan, but the commit
/// still refuses it; a full rerun spells the destinations in the new format.
#[test]
fn a_config_change_between_targets_and_write_set_replans() {
    let (_dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Ns.md", "- ns\n"),
        ("pages/Ns%2Fchild.md", "- child\n"),
    ]);
    let mut plan = plan_rename(&store, "Ns", "Space", None, RenameChoice::Rename, &|| {}).unwrap();
    b2_deliver(&store, &root, B2_LOWBAR);
    let pages = plan.pages(&store).unwrap();
    assert!(
        pages
            .iter()
            .any(|id| id.as_str() == "pages/Space%2Fchild.md"),
        "the stale plan spells the old format: {pages:?}"
    );
    let before = b2_tree(&root);
    assert!(preflight_rename(&store, "Ns", plan, &[]).unwrap().is_none());
    assert_eq!(b2_tree(&root), before);
    hosted(&store, |host| {
        rename_page_expected(&store, host, "Ns", "Space", None)
    })
    .unwrap();
    let after = b2_tree(&root);
    assert!(after.contains_key("Space___child.md"), "{after:?}");
    assert!(!after.contains_key("Space%2Fchild.md"), "{after:?}");
    store.close();
}

/// H-Q3 (A-W1, B2), the retry bound: a config that changes during every
/// plan exhausts the existing four attempts with no write; once it is
/// stable the rename goes through, in the format it was planned in. Each
/// attempt now plans more than once (the host operation's discovery under
/// its admission, Q-P2b-1), so the bound is counted in attempts. A
/// rename of several pages is one host operation (host-only-writer S2):
/// the rename's one attempt hands it to `retained::operation`, whose four
/// attempts are the bound.
#[test]
fn a_config_changing_during_every_plan_exhausts_the_retries() {
    let (_dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Ns.md", "- ns\n"),
        ("pages/Ns%2Fchild.md", "- child\n"),
        ("pages/Ref.md", "- see [[Ns/child]]\n"),
    ]);
    let before = b2_tree(&root);
    let flips = std::cell::Cell::new(0);
    let toggle = || {
        flips.set(flips.get() + 1);
        let config = if flips.get() % 2 == 1 {
            B2_LOWBAR
        } else {
            B2_LEGACY
        };
        b2_deliver(&store, &root, config);
    };
    crate::ATTEMPTS.with(|n| n.set(0));
    let err = hosted(&store, |host| {
        rename_page_after_inventory(
            &store,
            host,
            "Ns",
            "Space",
            None,
            RenameChoice::Rename,
            &[],
            toggle,
        )
    })
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(err.to_string(), "page changed repeatedly during rename");
    assert_eq!(
        crate::ATTEMPTS.with(|n| n.get()),
        1 + 4,
        "the existing bound: four attempts of the host operation"
    );
    assert!(
        flips.get() >= 4,
        "every attempt planned under a changed config"
    );
    assert_eq!(b2_tree(&root), before, "no write");
    hosted(&store, |host| {
        rename_page_after_inventory(
            &store,
            host,
            "Ns",
            "Space",
            None,
            RenameChoice::Rename,
            &[],
            || {},
        )
    })
    .unwrap();
    let after = b2_tree(&root);
    // The config the last flip left: an odd count is the triple-lowbar one.
    let child = if flips.get() % 2 == 1 {
        "Space___child.md"
    } else {
        "Space%2Fchild.md"
    };
    assert!(after.contains_key(child), "{after:?}");
    assert_eq!(after["Ref.md"], "- see [[Space/child]]\n");
    store.close();
}

/// H-Q3 (A-W1, B2), the host path: a config change during the first plan
/// fails the host's admission check before anything is written; the
/// replan's single-page rename uses the new format.
#[test]
fn a_hosted_rename_planned_before_a_config_change_replans() {
    let (dir, root, store) = b2_graph(&[
        ("logseq/config.edn", B2_LEGACY),
        ("pages/Start.md", "- start\n"),
        ("pages/Ref.md", "- see [[Start]]\n"),
    ]);
    let app = dir.path().join("app");
    disk::create_dir_all(&app).unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, &app).unwrap();
    let delivered = AtomicBool::new(false);
    rename_page_after_inventory(
        &store,
        &host,
        "Start",
        "Begin/x",
        None,
        RenameChoice::Rename,
        &[],
        || {
            if !delivered.swap(true, Ordering::AcqRel) {
                b2_deliver(&store, &root, B2_LOWBAR);
            }
        },
    )
    .unwrap();
    let after = b2_tree(&root);
    assert!(after.contains_key("Begin___x.md"), "{after:?}");
    assert!(!after.contains_key("Begin%2Fx.md"), "{after:?}");
    assert!(!after.contains_key("Start.md"), "{after:?}");
    assert_eq!(after["Ref.md"], "- see [[Begin/x]]\n");
    drop(host);
    store.close();
}
