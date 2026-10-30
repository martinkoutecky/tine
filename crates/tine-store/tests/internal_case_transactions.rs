use crate::Store;
use std::fs;
/// The retained case move on a case-folding volume: a failed step rolls back.
#[test]
fn transaction_case_move_failures() {
    use crate::FaultPoint;
    let mut cases = vec![
        (FaultPoint::MoveAfterTrashCopyIo, false),
        (FaultPoint::MidStepIoAt(0), false),
        (FaultPoint::MoveAfterTrashCopyIo, true),
        (FaultPoint::MidStepIoAt(0), true),
    ];
    // Directory sync exists only on Unix targets (Windows has no directory
    // flush), so the DirectorySyncIo fault is armed only there.
    if cfg!(unix) {
        cases.insert(0, (FaultPoint::DirectorySyncIo, false));
    }
    for (point, alias) in cases {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "- [[old]]\n").unwrap();
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        store.inject_fault(point);
        if alias {
            store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
        }
        // This is the transaction helper's undo contract, independent of
        // the feature's HostOp + non-drafted spelling command (S3).
        let src = store.file_id(crate::Area::Pages, "old.md").unwrap();
        let dst = store.file_id(crate::Area::Pages, "Old.md").unwrap();
        let rev = store.read(&src, None).unwrap().1;
        let mut tx = store.page_plan(Some(crate::EditKind::RenamePage));
        tx.move_file(
            &src,
            rev,
            &dst,
            Some(&crate::RenameMap(vec![("old".into(), "Old".into())])),
        );
        assert!(matches!(
            tx.commit_internal(),
            crate::TxOutcome::NotCommitted { .. }
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
            "- [[old]]\n"
        );
        let names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["old.md"]);
        store.close();
    }
}

#[test]
fn a_competing_destination_before_case_move_is_never_overwritten() {
    let _integration_guard = crate::test_transactions::integration_lock();
    use crate::{Area, EditKind, FaultPoint, TxOutcome};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let src = store.file_id(Area::Pages, "old.md").unwrap();
    let dst = store.file_id(Area::Pages, "Old.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    // On a case-folding filesystem (Windows NTFS, default macOS APFS) `Old.md`
    // names the source itself, so no distinct competing entry can exist: the
    // injected writer's no-replace create must fail, the case move publishes,
    // and the original bytes survive under the one requested spelling.
    let folding = dir.path().join("pages/Old.md").exists();
    store.inject_fault(FaultPoint::NoReplaceCollision);
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    let outcome = tx.commit_internal();
    if folding {
        assert!(
            matches!(outcome, TxOutcome::Committed { .. }),
            "{outcome:?}"
        );
        let names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["Old.md"]);
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            "- original\n"
        );
        store.close();
        return;
    }
    assert!(
        matches!(outcome, TxOutcome::NotCommitted { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
        "external collision"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "- original\n"
    );
    store.close();
}

#[cfg(unix)]
#[test]
fn separately_listed_hardlinked_case_destination_is_not_an_alias_spelling() {
    let _integration_guard = crate::test_transactions::integration_lock();
    use crate::{Area, EditKind, TxOutcome};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
    if fs::hard_link(
        dir.path().join("pages/old.md"),
        dir.path().join("pages/Old.md"),
    )
    .is_err()
    {
        return;
    }
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let src = store.file_id(Area::Pages, "old.md").unwrap();
    let dst = store.file_id(Area::Pages, "Old.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    assert!(matches!(
        tx.commit_internal(),
        TxOutcome::NotCommitted { .. }
    ));
    assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 2);
    store.close();
}

#[test]
fn unchanged_case_move_has_one_live_name_and_creates_no_staging_directory() {
    let _integration_guard = crate::test_transactions::integration_lock();
    use crate::{Area, EditKind, TxOutcome};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let src = store.file_id(Area::Pages, "old.md").unwrap();
    let dst = store.file_id(Area::Pages, "Old.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    assert!(matches!(tx.commit_internal(), TxOutcome::Committed { .. }));
    assert_eq!(
        fs::read(dir.path().join("pages/Old.md")).unwrap(),
        b"- original\n"
    );
    assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 1);
    assert!(
        !dir.path().join("logseq/.tine-trash").exists(),
        "I-2: a spelling move must publish atomically, without withdrawing the live page to Trash"
    );
    store.close();
}

#[test]
fn folded_alias_refusal_uses_atomic_rename_and_checks_the_result() {
    let _integration_guard = crate::test_transactions::integration_lock();
    use crate::{Area, EditKind, FaultPoint, TxOutcome};
    // `noop`: the provider keeps the old case, so the move goes through its
    // temporary entry (GS1); with `temp_fails` that second half fails and
    // the entry moves back.
    for (noop, temp_fails) in [(false, false), (true, false), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        if temp_fails {
            // Also exercise a graph retaining empty recovery directories on
            // Linux, where the destination does not alias the live source.
            fs::create_dir_all(dir.path().join("logseq/.tine-trash/pages")).unwrap();
        }
        fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let src = store.file_id(Area::Pages, "old.md").unwrap();
        let dst = store.file_id(Area::Pages, "Old.md").unwrap();
        let rev = store.read(&src, None).unwrap().1;
        store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
        if noop {
            store.inject_fault(FaultPoint::CaseMoveAliasNoop);
        }
        if temp_fails {
            store.inject_fault(FaultPoint::CaseMoveTemporaryIo);
        }
        let mut tx = store.page_plan(Some(EditKind::RenamePage));
        tx.move_file(&src, rev, &dst, None);
        let outcome = tx.commit_internal();
        assert_eq!(
            matches!(outcome, TxOutcome::Committed { .. }),
            !temp_fails,
            "{outcome:?}"
        );
        let temporary = dir.path().join("logseq/.tine-respell/pages/Old.md");
        assert!(!temporary.exists(), "no entry stays at {temporary:?}");
        let names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [if temp_fails { "old.md" } else { "Old.md" }]);
        assert_eq!(
            fs::read(dir.path().join("pages").join(&names[0])).unwrap(),
            b"- original\n"
        );
        let trash = dir.path().join("logseq/.tine-trash");
        if temp_fails {
            // A real provider reports rename success before its unchanged
            // listing refuses the operation. On folding volumes undo can
            // stage the aliased live bytes, restore the source and remove
            // the stage, leaving empty trash directories. That is valid
            // recovery, not evidence of a successful move staging its page.
            // Any recovery payload left behind must still be original bytes.
            let mut dirs = vec![trash];
            while let Some(dir) = dirs.pop() {
                let entries = match fs::read_dir(dir) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => panic!("cannot inspect recovery: {error}"),
                };
                for entry in entries {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        dirs.push(path);
                    } else {
                        assert_eq!(fs::read(path).unwrap(), b"- original\n");
                    }
                }
            }
        } else {
            assert!(
                !trash.exists(),
                "a successful byte-preserving move creates no trash"
            );
        }
        store.close();
    }
}

#[test]
fn failed_case_move_undo_keeps_live_bytes_and_recovers_the_baseline() {
    let _integration_guard = crate::test_transactions::integration_lock();
    use crate::{Area, EditKind, FaultPoint, TxOutcome};
    for alias in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
        let store = Store::open(dir.path(), Default::default()).unwrap().0;
        let src = store.file_id(Area::Pages, "old.md").unwrap();
        let dst = store.file_id(Area::Pages, "Old.md").unwrap();
        let rev = store.read(&src, None).unwrap().1;
        if alias {
            store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
        }
        store.inject_fault(FaultPoint::MidStepIoAt(0));
        store.inject_fault(FaultPoint::UndoWithdrawalIo);
        let mut tx = store.page_plan(Some(EditKind::RenamePage));
        tx.move_file(&src, rev, &dst, None);
        let TxOutcome::NotCommitted { rollback, .. } = tx.commit_internal() else {
            panic!("must fail");
        };
        assert!(!rollback.undo_failed.is_empty());
        assert_eq!(
            fs::read(dir.path().join("pages/Old.md")).unwrap(),
            b"- original\n"
        );
        assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 1);
        assert!(
            fs::read_dir(dir.path().join("logseq/.tine-trash/conflicts"))
                .unwrap()
                .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"- original\n")
        );
        store.close();
    }
}

#[test]
fn gs1_case_move_kill_worker() {
    use crate::{Area, EditKind, FaultPoint};
    let Ok(root) = std::env::var("TINE_GS1_KILL_ROOT") else {
        return;
    };
    let store = Store::open(std::path::Path::new(&root), Default::default())
        .unwrap()
        .0;
    let src = store.file_id(Area::Pages, "old.md").unwrap();
    let dst = store.file_id(Area::Pages, "Old.md").unwrap();
    let rev = store.read(&src, None).unwrap().1;
    store.inject_fault(FaultPoint::CaseMoveAliasRefusal);
    store.inject_fault(FaultPoint::CaseMoveAliasNoop);
    store.inject_fault(FaultPoint::AbortBetweenCaseMoveSteps);
    let mut tx = store.page_plan(Some(EditKind::RenamePage));
    tx.move_file(&src, rev, &dst, None);
    let _ = tx.commit_internal();
    panic!("the case move did not reach its abort point");
}

/// GS1: a process killed between the two halves of a case move on a
/// provider that keeps the old case leaves the bytes whole at the temporary
/// entry; the next host launch finishes the move.
#[test]
fn gs1_a_case_move_killed_between_its_halves_finishes_at_the_next_launch() {
    let _integration_guard = crate::test_transactions::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- original\n").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "test_transactions::og_gh609_case_rename::gs1_case_move_kill_worker",
        ])
        .env("TINE_GS1_KILL_ROOT", dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
        "must reach the injected abort, not panic before it: {output:?}"
    );
    let pages = || {
        let mut names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };
    let temporary = dir.path().join("logseq/.tine-respell/pages/Old.md");
    assert!(pages().is_empty(), "{:?}", pages());
    assert_eq!(fs::read(&temporary).unwrap(), b"- original\n");
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let app_data = tempfile::tempdir().unwrap();
    let host = crate::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    assert_eq!(pages(), ["Old.md"]);
    assert_eq!(
        fs::read(dir.path().join("pages/Old.md")).unwrap(),
        b"- original\n"
    );
    assert!(!temporary.exists());
    drop(host);
    store.close();
}
